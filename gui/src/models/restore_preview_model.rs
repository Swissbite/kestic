// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `RestorePreviewModel`: the dialog «Restore n items». It runs the preview
//! (`restore_preview` query) for the chosen target and mode, lists the files the preview
//! classified (page by page, filtered by outcome in the daemon), starts the restore with the
//! preview's ID and follows the run until it ends.

use super::qobject;
use super::snapshot_browse_model::last_name;
use super::snapshot_query::{
    QueryFailure, QueryList, QueryListModel, QueryListView, count_text, format_count, to_i32,
    too_large_text,
};
use crate::backend::{Lane, call_daemon_on, send_to_daemon};
use crate::state_cache::{
    daemon_unavailable_text, dbus_error_text, format_bytes, i18n, i18np, shared_state, tr,
};
use crate::ui_texts::GuiText;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use kestic_core::model::RunStatus;
use kestic_core::mount::current_home_dir;
use kestic_core::query_list::QueryState;
use kestic_core::restore::{
    folder_instead_of_large, folder_instead_of_unmatchable, selection_with_folders_instead,
};
use kestic_core::restore_target::{code_usable, preview_needed};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{
    RestoreOriginalDto, RestoreOutcome, RestoreOverwrite, RestorePreviewEntryDto,
    RestorePreviewResultDto, RestoreRequest, RestoreTarget, RestoreTargetCode, RunDetailDto,
    SnapshotQueryRequest, SnapshotQueryResultDto,
};
use kestic_i18n::codes::ErrorKind;
use kestic_i18n::messages::{active_run_message, nothing_restored_message};
use std::path::Path;
use std::pin::Pin;

model_row!(PreviewRow {
    path: String => "path",
    // Code of the outcome (`RestoreOutcome`).
    outcome: String => "outcome",
    outcome_text: String => "outcomeText",
    size_text: String => "sizeText",
});

/// The target the user chose in the dialog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum Choice {
    /// The suggested new folder (the default).
    #[default]
    Suggested,
    /// A folder from the folder dialog.
    Folder(String),
    Original,
}

#[derive(Default)]
pub struct RestorePreviewModelRust {
    /// The files of the preview and its query.
    list: QueryList<PreviewRow>,
    pub(crate) state: QString,
    pub(crate) error_text: QString,
    pub(crate) title: QString,
    pub(crate) new_folder: QString,
    pub(crate) original_dir: QString,
    pub(crate) original_available: bool,
    pub(crate) mapping_text: QString,
    pub(crate) original_note: QString,
    pub(crate) target_original: bool,
    pub(crate) target_dir: QString,
    pub(crate) overwrite: i32,
    pub(crate) check_text: QString,
    pub(crate) preview_required: bool,
    pub(crate) summary_text: QString,
    pub(crate) consequence_text: QString,
    pub(crate) overwrites: bool,
    pub(crate) notes_text: QString,
    pub(crate) can_restore: bool,
    pub(crate) matched: i32,
    pub(crate) too_large: bool,
    pub(crate) too_large_text: QString,
    pub(crate) count_text: QString,
    pub(crate) kind_codes: QStringList,
    pub(crate) kind_texts: QStringList,
    pub(crate) run_state: QString,
    pub(crate) run_text: QString,
    pub(crate) run_target: QString,
    pub(crate) folder_instead: QString,
    /// The preview failed because the daemon refuses the target: trying again does not help.
    pub(crate) target_refused: bool,

    destination_id: String,
    snapshot_id: String,
    selection: Vec<String>,
    choice: Choice,
    result: Option<RestorePreviewResultDto>,
    run_id: Option<String>,
    /// A check of the run was asked for and has not arrived yet.
    run_polling: bool,
}

/// The overwrite mode at `index` of the dialog's list, which lists the modes in the order of
/// [`RestoreOverwrite::ALL`] (index 0 = «Never»).
fn mode_of(index: i32) -> RestoreOverwrite {
    usize::try_from(index)
        .ok()
        .and_then(|i| RestoreOverwrite::ALL.get(i).copied())
        .unwrap_or_default()
}

/// `path` with the home folder shown as `~`.
fn tilde(path: &str) -> String {
    let home = current_home_dir();
    match home.as_deref().and_then(Path::to_str) {
        Some(home) if path.starts_with(&format!("{home}/")) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}

fn row_for(entry: &RestorePreviewEntryDto) -> PreviewRow {
    PreviewRow {
        path: entry.path.clone(),
        outcome: entry.outcome.as_str().to_string(),
        outcome_text: tr(&entry.outcome.message()),
        size_text: format_bytes(entry.size),
    }
}

/// Title of the dialog: «Restore 3 Items», the whole snapshot «Restore Snapshot».
pub(crate) fn title_text(selected: usize) -> String {
    if selected == 0 {
        i18n("Restore Snapshot").to_string()
    } else {
        i18np("Restore %1 Item", "Restore %1 Items", selected, &[])
    }
}

/// Why the selection is too large for one restore, and which folder can be restored instead.
pub(crate) fn too_many_text(folder: &str) -> String {
    i18n("Too many entries are selected for one restore. Restore their folder '%1' instead.")
        .subs(&[folder])
}

/// Why a selected name cannot be restored on its own, and which folder can.
pub(crate) fn unmatchable_text(path: &str, folder: &str) -> String {
    i18n("'%1' cannot be restored on its own: its name contains characters that are not valid UTF-8. Restore its folder '%2' instead.")
        .subs(&[path, folder])
}

/// «Original: /home/alt/Dokumente (alt → neu)» when the original location was mapped to the
/// user's home; empty otherwise.
pub(crate) fn mapping_text(original: &RestoreOriginalDto) -> String {
    original
        .mapping
        .as_ref()
        .map_or_else(String::new, |(from, to)| {
            i18n("Original: %1 (%2 → %3)").subs(&[&original.original_dir, from, to])
        })
}

/// Why the original location cannot be chosen: its folder does not exist here, or the
/// selection has no usable original location at all.
pub(crate) fn original_note(original: &RestoreOriginalDto) -> String {
    match original.mapped_dir.as_deref() {
        Some(dir) if original.needs_user_choice => RestoreTargetCode::MissingMappedDir
            .message(&tilde(dir))
            .map_or_else(String::new, |m| tr(&m)),
        _ => i18n("The original location cannot be used for this selection; choose a folder.")
            .to_string(),
    }
}

/// Whether the preview stopped at the memory limit instead of classifying the files: its list
/// is too large and nothing is counted. Where a preview is required, the daemon refuses a
/// restore with it.
pub(crate) fn preview_stopped(p: &RestorePreviewResultDto) -> bool {
    p.entries.too_large && p.new_files + p.overwritten_files + p.kept_files + p.identical_files == 0
}

/// The counts of a preview in one line, e.g. «117 files new · 3 existing files will not be
/// overwritten».
pub(crate) fn summary_text(p: &RestorePreviewResultDto) -> String {
    let mut parts = Vec::new();
    if p.new_files > 0 {
        parts.push(i18np("%1 file new", "%1 files new", p.new_files, &[]));
    }
    if p.overwritten_files > 0 {
        parts.push(i18np(
            "%1 existing file will be overwritten",
            "%1 existing files will be overwritten",
            p.overwritten_files,
            &[],
        ));
    }
    if p.kept_files > 0 {
        parts.push(i18np(
            "%1 existing file will not be overwritten",
            "%1 existing files will not be overwritten",
            p.kept_files,
            &[],
        ));
    }
    if p.identical_files > 0 {
        parts.push(i18np(
            "%1 file unchanged",
            "%1 files unchanged",
            p.identical_files,
            &[],
        ));
    }
    parts.join(" · ")
}

/// The consequence named before overwriting: «3 files will be overwritten. This cannot be
/// undone.»
pub(crate) fn consequence_text(overwritten: u64) -> String {
    i18np(
        "%1 file will be overwritten. This cannot be undone.",
        "%1 files will be overwritten. This cannot be undone.",
        overwritten,
        &[],
    )
}

/// Text of a filter chip: the outcome and how many files have it.
fn chip_text(outcome: RestoreOutcome, count: u64) -> String {
    GuiText::TwoParts.with_args(&[&tr(&outcome.message()), &format_count(count)])
}

/// The line of a running restore: its progress, and the speed once it is known.
pub(crate) fn restore_progress_text(progress: &str, speed_bps: u64) -> String {
    if speed_bps == 0 {
        return progress.to_string();
    }
    let speed = i18n("%1/s").subs(&[&format_bytes(speed_bps)]);
    GuiText::TwoParts.with_args(&[progress, &speed])
}

/// The state the restore message shows for an ended run. A cancelled run (the user's choice)
/// and an interrupted one (sleep, daemon start) are no failures: their message stays neutral.
pub(crate) fn run_state_of(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Ok => "ok",
        RunStatus::Warning => "warning",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Interrupted => "interrupted",
        _ => "error",
    }
}

/// The line shown when a restore run ended.
pub(crate) fn run_end_text(detail: &RunDetailDto) -> String {
    match detail.run.status {
        RunStatus::Ok => {
            let files = detail
                .run
                .restore
                .as_ref()
                .map_or(0, |r| r.files_new + r.files_overwritten);
            i18np("%1 file restored", "%1 files restored", files, &[])
        }
        RunStatus::Warning => tr(&nothing_restored_message()),
        RunStatus::Cancelled => i18n("Restore cancelled").to_string(),
        // Stopped by sleep (the run says so) or ended at the next daemon start (no message).
        RunStatus::Interrupted => detail
            .run
            .summary_message
            .clone()
            .unwrap_or_else(|| GuiText::Interrupted.text()),
        // The generic text of an unrecognised error names a backup.
        _ => match detail.error_kind {
            Some(kind) if kind != ErrorKind::Unknown => {
                tr(&kind.message(detail.error_exit_code, false))
            }
            _ => i18n("Restore failed").to_string(),
        },
    }
}

/// Whether a preview failed because the daemon refuses its target; trying again does not help.
pub(crate) fn refuses_target(kind: Option<ErrorKind>) -> bool {
    kind == Some(ErrorKind::InvalidTarget)
}

impl RestorePreviewModelRust {
    fn target(&self) -> Option<RestoreTarget> {
        match &self.choice {
            Choice::Suggested => None,
            Choice::Folder(dir) => Some(RestoreTarget::NewFolder { dir: dir.clone() }),
            Choice::Original => self
                .result
                .as_ref()
                .and_then(|r| r.suggestion.original.mapped_dir.clone())
                .map(|dir| RestoreTarget::Original { dir }),
        }
    }
}

query_list_model!(
    RestorePreviewModel,
    PreviewRow,
    begin_reset: begin_reset_preview,
    end_reset: end_reset_preview,
    begin_insert: begin_insert_rows_preview,
    end_insert: end_insert_rows_preview,
);

impl QueryListModel for qobject::RestorePreviewModel {
    type Page = RestorePreviewResultDto;

    fn page_of(result: SnapshotQueryResultDto) -> Option<RestorePreviewResultDto> {
        match result {
            SnapshotQueryResultDto::RestorePreview(p) => Some(*p),
            _ => None,
        }
    }

    fn page_rows(&self, p: &RestorePreviewResultDto) -> Vec<PreviewRow> {
        p.entries.items.iter().map(row_for).collect()
    }

    /// Shows the first page; when the preview just ended, also its target, counts and notes.
    fn show_first_page(mut self: Pin<&mut Self>, p: RestorePreviewResultDto) {
        let first_load = self.query_state() != Some(QueryState::Done);
        let rows = self.page_rows(&p);
        self.as_mut().reset_rows(rows);
        self.as_mut().set_matched(to_i32(p.entries.matched));
        self.as_mut().set_too_large(p.entries.too_large);
        self.as_mut()
            .set_too_large_text(QString::from(&too_large_text(p.entries.total)));
        let filtered = !self.list.filter.is_empty() && !p.entries.too_large;
        self.as_mut().set_count_text(QString::from(&count_text(
            p.entries.total,
            p.entries.matched,
            filtered,
        )));
        if first_load {
            self.as_mut().apply_result(&p);
        }
        self.as_mut().rust_mut().result = Some(p);
        self.set_query_state(Some(QueryState::Done));
    }

    /// Runs the preview again; the filter starts empty.
    fn restart(self: Pin<&mut Self>) {
        self.run_preview();
    }

    fn note_failure(self: Pin<&mut Self>, kind: Option<ErrorKind>) {
        self.set_target_refused(refuses_target(kind));
    }
}

impl qobject::RestorePreviewModel {
    /// Opens the dialog for `selection` (absolute snapshot paths, empty = everything) of
    /// `snapshot_id` in `destination_id`: suggested new folder, mode «Never».
    pub fn open(
        mut self: Pin<&mut Self>,
        destination_id: &QString,
        snapshot_id: &QString,
        selection: &QStringList,
    ) {
        let selection: Vec<String> = selection.iter().map(ToString::to_string).collect();
        self.as_mut()
            .set_title(QString::from(&title_text(selection.len())));
        {
            let mut rust = self.as_mut().rust_mut();
            rust.destination_id = destination_id.to_string();
            rust.snapshot_id = snapshot_id.to_string();
            rust.selection = selection;
            rust.choice = Choice::Suggested;
            rust.result = None;
        }
        self.as_mut().set_overwrite(0);
        self.as_mut().set_new_folder(QString::from(""));
        self.as_mut().set_original_dir(QString::from(""));
        self.as_mut().set_original_available(false);
        self.as_mut().set_mapping_text(QString::from(""));
        self.as_mut().set_original_note(QString::from(""));
        // The message of a running restore stays; an ended one gives way to the new dialog.
        if self.run_state.to_string() != "running" {
            self.as_mut().rust_mut().run_id = None;
            self.as_mut().set_run_state(QString::from(""));
            self.as_mut().set_run_text(QString::from(""));
        }
        self.as_mut().run_preview();
    }

    /// restic cannot match a name that is not UTF-8: the dialog says so and offers the folder
    /// above it instead of running a preview the daemon would refuse.
    fn refuse_unmatchable(mut self: Pin<&mut Self>, path: &str, folder: &str) {
        self.as_mut().set_can_restore(false);
        self.as_mut()
            .set_folder_instead(QString::from(&tilde(folder)));
        let text = unmatchable_text(last_name(path), last_name(folder));
        self.as_mut().fail_query(&QueryFailure::from_text(text));
    }

    /// The selection is too large for one restore: the dialog says so and offers the folder
    /// holding it.
    fn refuse_too_many(mut self: Pin<&mut Self>, folder: &str) {
        self.as_mut().set_can_restore(false);
        self.as_mut()
            .set_folder_instead(QString::from(&tilde(folder)));
        let text = too_many_text(&tilde(folder));
        self.as_mut().fail_query(&QueryFailure::from_text(text));
    }

    /// Restores the folder instead: of a selection too large for one restore, or of every
    /// selected name restic cannot match; then runs the preview.
    pub fn restore_folder_instead(mut self: Pin<&mut Self>) {
        let selection = folder_instead_of_large(&self.selection).map_or_else(
            || selection_with_folders_instead(&self.selection),
            |folder| vec![folder],
        );
        self.as_mut()
            .set_title(QString::from(&title_text(selection.len())));
        self.as_mut().rust_mut().selection = selection;
        self.as_mut().run_preview();
    }

    /// Restores into the suggested new folder, or into `dir` when it is not empty.
    pub fn choose_folder(mut self: Pin<&mut Self>, dir: &QString) {
        let dir = dir.to_string();
        self.as_mut().rust_mut().choice = if dir.is_empty() {
            Choice::Suggested
        } else {
            Choice::Folder(dir)
        };
        // Overwriting is chosen for one target only; another target starts with «Never».
        self.as_mut().set_overwrite(0);
        self.as_mut().run_preview();
    }

    /// Restores to the original location.
    pub fn choose_original(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().choice = Choice::Original;
        self.as_mut().set_overwrite(0);
        self.as_mut().run_preview();
    }

    /// Sets what happens to existing files (index into «Never · If older · If changed ·
    /// Always»); a new preview runs.
    pub fn choose_overwrite(mut self: Pin<&mut Self>, index: i32) {
        if index == self.overwrite {
            return;
        }
        self.as_mut().set_overwrite(index);
        self.as_mut().run_preview();
    }

    /// Shows only the files with the outcomes `codes`; none shows all.
    pub fn set_kinds(self: Pin<&mut Self>, codes: &QStringList) {
        let kinds: Vec<String> = codes
            .iter()
            .map(ToString::to_string)
            .filter(|c| RestoreOutcome::parse(c).is_some())
            .collect();
        self.set_filter_kinds(kinds);
    }

    fn run_preview(mut self: Pin<&mut Self>) {
        if self.query_state() == Some(QueryState::Running) {
            self.as_mut().list_mut().run.abandon();
        }
        self.as_mut().set_target_refused(false);
        if let Some(folder) = folder_instead_of_large(&self.selection) {
            return self.as_mut().refuse_too_many(&folder);
        }
        let unmatchable = self
            .selection
            .iter()
            .find_map(|p| folder_instead_of_unmatchable(p).map(|f| (p.clone(), f)));
        if let Some((path, folder)) = unmatchable {
            return self.as_mut().refuse_unmatchable(&path, &folder);
        }
        self.as_mut().set_folder_instead(QString::from(""));
        let original = self.choice == Choice::Original;
        let target = self.target();
        let request = SnapshotQueryRequest::restore_preview(
            &self.destination_id,
            &self.snapshot_id,
            &self.selection,
            target,
            mode_of(self.overwrite),
        );
        self.as_mut().clear_list();
        self.as_mut().set_target_original(original);
        self.as_mut().set_can_restore(false);
        self.as_mut().set_summary_text(QString::from(""));
        self.as_mut().set_consequence_text(QString::from(""));
        self.as_mut().set_overwrites(false);
        self.as_mut().set_notes_text(QString::from(""));
        self.as_mut().set_check_text(QString::from(""));
        self.as_mut().set_error_text(QString::from(""));
        self.as_mut().set_kind_codes(QStringList::default());
        self.as_mut().set_kind_texts(QStringList::default());
        self.as_mut().set_matched(0);
        self.as_mut().start_query(request);
    }

    /// The texts of every outcome the preview list can show, to size its outcome column.
    pub fn outcome_texts(&self) -> QStringList {
        let mut list = QStringList::default();
        for outcome in RestoreOutcome::ALL {
            list.append(QString::from(&tr(&outcome.message())));
        }
        list
    }

    /// Tries again after an error (the preview failed, or a page or a filter could not be
    /// fetched): runs the preview again, with the same filter.
    pub fn retry(mut self: Pin<&mut Self>) {
        if self.query_state() != Some(QueryState::Error) || !self.folder_instead.is_empty() {
            return;
        }
        let filter = self.list.filter.clone();
        self.as_mut().run_preview();
        self.as_mut().list_mut().filter = filter;
    }

    fn apply_result(mut self: Pin<&mut Self>, p: &RestorePreviewResultDto) {
        let original = &p.suggestion.original;
        self.as_mut()
            .set_new_folder(QString::from(&tilde(&p.suggestion.new_folder)));
        self.as_mut().set_original_dir(QString::from(&tilde(
            original
                .mapped_dir
                .as_deref()
                .unwrap_or(&original.original_dir),
        )));
        self.as_mut()
            .set_original_available(!original.needs_user_choice && original.mapped_dir.is_some());
        self.as_mut()
            .set_mapping_text(QString::from(&mapping_text(original)));
        self.as_mut()
            .set_original_note(QString::from(&original_note(original)));
        self.as_mut()
            .set_target_dir(QString::from(&tilde(&p.target.check.detail)));
        let check = p
            .target
            .check
            .code
            .message(&p.target.check.detail)
            .filter(|_| p.target.check.code != RestoreTargetCode::ExistsNotEmpty)
            .map_or_else(String::new, |m| tr(&m));
        self.as_mut().set_check_text(QString::from(&check));
        let usable = code_usable(p.target.check.code);
        let required = preview_needed(&p.target.target, p.target.check.code);
        self.as_mut().set_preview_required(required);
        self.as_mut()
            .set_summary_text(QString::from(&summary_text(p)));
        self.as_mut().set_overwrites(p.overwritten_files > 0);
        let consequence = if p.overwritten_files > 0 {
            consequence_text(p.overwritten_files)
        } else {
            String::new()
        };
        self.as_mut()
            .set_consequence_text(QString::from(&consequence));
        let notes: Vec<String> = p.notes.iter().map(|n| tr(&n.message())).collect();
        self.as_mut()
            .set_notes_text(QString::from(&notes.join("\n")));
        let mut codes = QStringList::default();
        let mut texts = QStringList::default();
        let counts = [
            (RestoreOutcome::New, p.new_files),
            (RestoreOutcome::Overwritten, p.overwritten_files),
            (RestoreOutcome::Kept, p.kept_files),
            (RestoreOutcome::Identical, p.identical_files),
        ];
        for (outcome, count) in counts.into_iter().filter(|(_, n)| *n > 0) {
            codes.append(QString::from(outcome.as_str()));
            texts.append(QString::from(&chip_text(outcome, count)));
        }
        self.as_mut().set_kind_codes(codes);
        self.as_mut().set_kind_texts(texts);
        self.as_mut()
            .set_can_restore(usable && !(required && preview_stopped(p)));
    }

    /// Starts the restore with the finished preview; `run_state` follows the run.
    pub fn start_restore(mut self: Pin<&mut Self>) {
        let Some(result) = self.result.clone() else {
            return;
        };
        if !self.can_restore || self.query_state() != Some(QueryState::Done) {
            return;
        }
        let request = RestoreRequest {
            destination_id: self.destination_id.clone(),
            snapshot_id: self.snapshot_id.clone(),
            selection: self.selection.clone(),
            target: result.target.target.clone(),
            overwrite: mode_of(self.overwrite),
            preview_id: self.list.run.id().map(str::to_string),
        };
        // One run per preview: a second click must not start another restore.
        self.as_mut().set_can_restore(false);
        let target = result.target.check.detail;
        call_daemon_on(
            &self.qt_thread(),
            Lane::slow("restore"),
            move |client| {
                client
                    .start_restore(&request)
                    .map_err(|e| dbus_error_text(&e))
            },
            |_| false,
            move |mut model: Pin<&mut Self>, started| match started
                .unwrap_or_else(|_| Err(daemon_unavailable_text()))
            {
                Ok(run_id) => {
                    model.as_mut().rust_mut().run_id = Some(run_id);
                    model.as_mut().set_run_target(QString::from(&target));
                    model
                        .as_mut()
                        .set_run_text(QString::from(&i18n("Restoring files…").to_string()));
                    model.as_mut().set_run_state(QString::from("running"));
                }
                Err(text) => {
                    model.as_mut().set_run_text(QString::from(&text));
                    model.as_mut().set_run_state(QString::from("error"));
                }
            },
        );
    }

    /// Checks the running restore (called by a timer while `run_state` is `running`), unless
    /// the answer to the last check is still on its way.
    pub fn poll_run(mut self: Pin<&mut Self>) {
        let Some(run_id) = self.run_id.clone() else {
            return;
        };
        if self.run_polling {
            return;
        }
        self.as_mut().rust_mut().run_polling = true;
        call_daemon_on(
            &self.qt_thread(),
            Lane::Alone,
            move |client| client.get_run_detail(&run_id),
            |_| false,
            |mut model: Pin<&mut Self>, detail| {
                model.as_mut().rust_mut().run_polling = false;
                if let Ok(Ok(detail)) = detail {
                    model.show_run(&detail);
                }
            },
        );
    }

    /// Shows the state of the restore as `detail` has it.
    fn show_run(mut self: Pin<&mut Self>, detail: &RunDetailDto) {
        // A dismissed restore shows nothing any more.
        if self.run_id.as_deref() != Some(detail.run.id.as_str()) {
            return;
        }
        let run_id = detail.run.id.as_str().to_string();
        let state = match detail.run.status {
            RunStatus::Pending => {
                let waiting = tr(&kestic_i18n::codes::PendingReasonCode::Queue.waiting_message(""));
                self.as_mut().set_run_text(QString::from(&waiting));
                return;
            }
            RunStatus::Running => {
                let progress = shared_state()
                    .lock_unpoisoned()
                    .active_progress
                    .get(&run_id)
                    .map(|p| {
                        restore_progress_text(&tr(&active_run_message(&p.info())), p.speed_bps)
                    });
                if let Some(text) = progress {
                    self.as_mut().set_run_text(QString::from(&text));
                }
                return;
            }
            status => run_state_of(status),
        };
        self.as_mut()
            .set_run_text(QString::from(&run_end_text(detail)));
        self.as_mut().set_run_state(QString::from(state));
    }

    /// Cancels the running restore; files written so far stay.
    pub fn cancel_run(self: Pin<&mut Self>) {
        if let Some(run_id) = self.run_id.clone() {
            send_to_daemon(Lane::Alone, move |client| {
                // A run that ended meanwhile needs no cancel.
                let _ = client.cancel_run(&run_id);
            });
        }
    }

    /// Hides the message of an ended restore.
    pub fn dismiss_run(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().run_id = None;
        self.as_mut().set_run_state(QString::from(""));
    }
}

#[cfg(test)]
mod tests {
    use super::super::snapshot_query::{Fetched, status_outcome};
    use super::*;
    use kestic_daemon::dbus_api::{
        ListPageDto, RestoreOriginalDto, RestorePreviewTargetDto, RestoreSuggestionDto,
        RestoreTargetCheckDto, SnapshotQueryStatusDto,
    };

    fn preview(counts: (u64, u64, u64, u64)) -> RestorePreviewResultDto {
        RestorePreviewResultDto {
            suggestion: RestoreSuggestionDto {
                new_folder: "/home/u/Restored/x".into(),
                original: RestoreOriginalDto {
                    original_dir: "/home/u".into(),
                    mapped_dir: Some("/home/u".into()),
                    mapping: None,
                    needs_user_choice: false,
                },
            },
            target: RestorePreviewTargetDto {
                target: RestoreTarget::Original {
                    dir: "/home/u".into(),
                },
                check: RestoreTargetCheckDto {
                    code: RestoreTargetCode::ExistsNotEmpty,
                    detail: "/home/u".into(),
                },
            },
            new_files: counts.0,
            overwritten_files: counts.1,
            kept_files: counts.2,
            identical_files: counts.3,
            bytes_to_write: 0,
            entries: ListPageDto {
                items: Vec::new(),
                offset: 0,
                total: 0,
                matched: 0,
                too_large: false,
            },
            notes: Vec::new(),
        }
    }

    /// A preview that stopped at the memory limit counts nothing; one whose list is only too
    /// large to show keeps its counts and allows the restore.
    #[test]
    fn preview_stopped_at_the_memory_limit_is_told_apart() {
        let mut stopped = preview((0, 0, 0, 0));
        stopped.entries.too_large = true;
        assert!(preview_stopped(&stopped), "too large, nothing counted");
        let mut long = preview((900, 100, 0, 0));
        long.entries.too_large = true;
        assert!(!preview_stopped(&long), "too large to show, counted");
        assert!(!preview_stopped(&preview((0, 0, 0, 0))), "an empty preview");
    }

    #[test]
    fn summary_names_the_outcomes_that_occur() {
        assert_eq!(
            summary_text(&preview((117, 0, 3, 0))),
            "117 files new · 3 existing files will not be overwritten"
        );
        assert_eq!(
            summary_text(&preview((0, 1, 0, 2))),
            "1 existing file will be overwritten · 2 files unchanged"
        );
        assert_eq!(
            consequence_text(3),
            "3 files will be overwritten. This cannot be undone."
        );
        assert_eq!(title_text(3), "Restore 3 Items");
        assert_eq!(title_text(0), "Restore Snapshot");
    }

    fn mapped_preview() -> RestorePreviewResultDto {
        let mut p = preview((0, 0, 0, 0));
        p.suggestion.original = RestoreOriginalDto {
            original_dir: "/home/alt/Dokumente".into(),
            mapped_dir: Some("/home/neu/Dokumente".into()),
            mapping: Some(("alt".into(), "neu".into())),
            needs_user_choice: false,
        };
        p
    }

    /// AC-M4-06-1 (b), GUI part: the dialog names the original folder and the mapping
    /// `alt → neu`; without a mapping it says nothing.
    #[test]
    fn ac_m4_06_1_dialog_names_the_mapping() {
        let p = mapped_preview();
        assert_eq!(
            mapping_text(&p.suggestion.original),
            "Original: /home/alt/Dokumente (alt → neu)",
            "(b)"
        );
        assert_eq!(
            mapping_text(&preview((0, 0, 0, 0)).suggestion.original),
            "",
            "(b) nothing mapped, nothing shown"
        );
    }

    /// AC-M4-06-2: when the user changes the suggested original location to a folder of their
    /// own, (a) the chosen folder is the target.
    #[test]
    fn ac_m4_06_2_chosen_folder_replaces_the_suggestion() {
        let mut model = RestorePreviewModelRust {
            result: Some(mapped_preview()),
            choice: Choice::Original,
            ..RestorePreviewModelRust::default()
        };
        assert_eq!(
            model.target(),
            Some(RestoreTarget::Original {
                dir: "/home/neu/Dokumente".into()
            }),
            "the suggestion before the change"
        );
        model.choice = Choice::Folder("/home/neu/Elsewhere".into());
        assert_eq!(
            model.target(),
            Some(RestoreTarget::NewFolder {
                dir: "/home/neu/Elsewhere".into()
            }),
            "(a) the chosen folder"
        );
    }

    /// AC-M4-06-4 (a), GUI part: a missing mapped folder is named with the request to choose a
    /// folder.
    #[test]
    fn ac_m4_06_4_missing_mapped_folder_asks_for_a_folder() {
        let mut original = mapped_preview().suggestion.original;
        original.mapped_dir = Some("/nonexistent/neu/Projekte".into());
        original.needs_user_choice = true;
        assert_eq!(
            original_note(&original),
            "/nonexistent/neu/Projekte does not exist; choose a folder",
            "(a)"
        );
    }

    /// A preview the daemon refuses for its target cannot succeed on «Retry»: (a) a failed
    /// query with `invalid_target` is a refusal, (b) one with another error code is not, and
    /// (c) neither is one without an error code.
    #[test]
    fn failed_preview_tells_a_refused_target_from_other_errors() {
        let failed_kind = |kind| {
            let status = SnapshotQueryStatusDto {
                state: "error".to_string(),
                error_kind: kind,
                error_detail: Some("/home/u".to_string()),
                result: None,
            };
            match status_outcome(status) {
                Fetched::Failed(failure) => failure.kind,
                _ => panic!("a failed preview is a failure"),
            }
        };
        assert!(
            refuses_target(failed_kind(Some(ErrorKind::InvalidTarget))),
            "(a)"
        );
        assert!(
            !refuses_target(failed_kind(Some(ErrorKind::Unknown))),
            "(b)"
        );
        assert!(!refuses_target(failed_kind(None)), "(c)");
    }

    #[test]
    fn modes_follow_the_list_order_and_default_to_never() {
        assert_eq!(mode_of(0), RestoreOverwrite::Never);
        assert_eq!(mode_of(3), RestoreOverwrite::Always);
        assert_eq!(mode_of(9), RestoreOverwrite::Never);
    }

    fn ended_run(status: RunStatus, summary: Option<&str>) -> RunDetailDto {
        let clock =
            kestic_core::clock::FakeClock::new(time::macros::datetime!(2026-11-15 12:00 UTC));
        let dest = kestic_core::model::DestinationId::generate().expect("destination id");
        let mut run = kestic_core::model::Run::new_restore(&clock, dest, None).expect("run");
        run.status = status;
        run.summary_message = summary.map(str::to_string);
        RunDetailDto {
            run,
            backup_name: None,
            destination_name: "NAS".into(),
            log_lines: Vec::new(),
            transparent_commands: Vec::new(),
            error_action: None,
            error_raw_detail: None,
            unreadable_files: Vec::new(),
            error_kind: None,
            error_exit_code: Some(1),
        }
    }

    #[test]
    fn ended_restore_never_names_a_backup() {
        assert_eq!(
            run_end_text(&ended_run(
                RunStatus::Interrupted,
                Some("Interrupted by system sleep")
            )),
            "Interrupted by system sleep"
        );
        assert_eq!(
            run_end_text(&ended_run(RunStatus::Interrupted, None)),
            "Interrupted",
            "ended at daemon start: no cause is named"
        );
        assert_eq!(
            run_end_text(&ended_run(RunStatus::Error, Some("restic failed"))),
            "Restore failed"
        );
    }

    /// AC-M4-05-30 (banner part): the line of a running restore names (a) no speed before one
    /// is known and (b) the speed once it is.
    #[test]
    fn ac_m4_05_30_running_restore_line_names_the_speed() {
        assert_eq!(
            restore_progress_text("Home: restoring · 1%", 0),
            "Home: restoring · 1%",
            "(a) no speed yet"
        );
        assert_eq!(
            restore_progress_text("Home: restoring · 1%", 4_000_000),
            format!("Home: restoring · 1% · {}/s", format_bytes(4_000_000)),
            "(b) with the speed"
        );
    }

    /// AC-M4-05-29 (b, unit part): a cancelled and an interrupted restore end in neutral states
    /// of their own, a failed one in «error».
    #[test]
    fn ac_m4_05_29_cancelled_and_interrupted_restores_are_no_errors() {
        assert_eq!(
            run_state_of(RunStatus::Cancelled),
            "cancelled",
            "(b) cancelled"
        );
        assert_eq!(
            run_state_of(RunStatus::Interrupted),
            "interrupted",
            "(b) interrupted"
        );
        assert_eq!(
            run_state_of(RunStatus::Error),
            "error",
            "a failure stays an error"
        );
        assert_eq!(run_state_of(RunStatus::Ok), "ok", "ok");
    }
}
