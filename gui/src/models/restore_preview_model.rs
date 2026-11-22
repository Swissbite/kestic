// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `RestorePreviewModel`: the dialog «Restore n items». It runs the preview
//! (`restore_preview` query) for the chosen target and mode, lists the files the preview
//! classified (page by page, filtered by outcome in the daemon), starts the restore with the
//! preview's ID and follows the run until it ends.

use super::qobject;
use super::rows::{role_names, row_count, row_data};
use super::snapshot_query::{
    QueryRun, StartPoll, count_text, fetch_error_text, format_count, to_i32, too_large_text,
};
use crate::state_cache::{
    dbus_error_text, format_bytes, i18n, i18np, make_client, shared_state, tr,
};
use crate::ui_texts::GuiText;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QVariant};
use kestic_core::model::RunStatus;
use kestic_core::query_list::{DEFAULT_PAGE_LIMIT, PageFilter};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{
    KesticDbusError, RestoreOutcome, RestoreOverwrite, RestorePreviewEntryDto,
    RestorePreviewResultDto, RestoreRequest, RestoreTarget, RestoreTargetCode, RunDetailDto,
    SnapshotQueryPage, SnapshotQueryRequest, SnapshotQueryResultDto,
};
use kestic_i18n::codes::ErrorKind;
use kestic_i18n::messages::{active_run_message, nothing_restored_message};
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
    rows: Vec<PreviewRow>,
    pub(crate) state: QString,
    pub(crate) error_text: QString,
    pub(crate) title: QString,
    pub(crate) new_folder: QString,
    pub(crate) original_dir: QString,
    pub(crate) original_available: bool,
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

    destination_id: String,
    snapshot_id: String,
    selection: Vec<String>,
    choice: Choice,
    filter: PageFilter,
    result: Option<RestorePreviewResultDto>,
    query: QueryRun,
    run_id: Option<String>,
}

/// The overwrite modes in the order of the dialog's list (index 0 = «Never»).
const MODES: [RestoreOverwrite; 4] = [
    RestoreOverwrite::Never,
    RestoreOverwrite::IfNewer,
    RestoreOverwrite::IfChanged,
    RestoreOverwrite::Always,
];

fn mode_of(index: i32) -> RestoreOverwrite {
    usize::try_from(index)
        .ok()
        .and_then(|i| MODES.get(i).copied())
        .unwrap_or_default()
}

/// `path` with the home folder shown as `~`.
fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&format!("{home}/")) => {
            format!("~{}", &path[home.len()..])
        }
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

/// The counts of a preview in one line, e.g. «117 files new · 3 existing files will not be
/// overwritten»; the wording of the kept files follows the mode.
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
        RunStatus::Cancelled => tr(&ErrorKind::Cancelled.message(None, false)),
        _ => tr(&detail
            .error_kind
            .unwrap_or(ErrorKind::Unknown)
            .message(detail.error_exit_code, false)),
    }
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

    fn fetch(&mut self, offset: u64) -> Result<Option<RestorePreviewResultDto>, KesticDbusError> {
        let page = SnapshotQueryPage {
            offset,
            limit: DEFAULT_PAGE_LIMIT,
            filter: self.filter.clone(),
        };
        match self.query.fetch_page(&page)? {
            Some(SnapshotQueryResultDto::RestorePreview(p)) => Ok(Some(*p)),
            Some(_) => Err(KesticDbusError::Internal(
                "unexpected snapshot query result".to_string(),
            )),
            None => Ok(None),
        }
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
        // The message of a running restore stays; an ended one gives way to the new dialog.
        if self.run_state.to_string() != "running" {
            self.as_mut().rust_mut().run_id = None;
            self.as_mut().set_run_state(QString::from(""));
            self.as_mut().set_run_text(QString::from(""));
        }
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
    pub fn set_kinds(mut self: Pin<&mut Self>, codes: &QStringList) {
        let kinds: Vec<String> = codes
            .iter()
            .map(ToString::to_string)
            .filter(|c| RestoreOutcome::parse(c).is_some())
            .collect();
        if kinds == self.filter.kinds {
            return;
        }
        self.as_mut().rust_mut().filter.kinds = kinds;
        if self.state.to_string() == "done" {
            self.as_mut().load_first_page();
        }
    }

    /// Filters the listed files by `text` (any case, in the path); the daemon filters.
    pub fn set_filter(mut self: Pin<&mut Self>, text: &QString) {
        let text = text.to_string();
        if text == self.filter.text {
            return;
        }
        self.as_mut().rust_mut().filter.text = text;
        if self.state.to_string() == "done" {
            self.as_mut().load_first_page();
        }
    }

    fn run_preview(mut self: Pin<&mut Self>) {
        if self.state.to_string() == "running" {
            self.as_mut().rust_mut().query.abandon();
        }
        let original = self.choice == Choice::Original;
        let target = self.target();
        let request = SnapshotQueryRequest::restore_preview(
            &self.destination_id,
            &self.snapshot_id,
            &self.selection,
            target,
            mode_of(self.overwrite),
        );
        self.as_mut().rust_mut().filter = PageFilter::default();
        self.as_mut().begin_reset_preview();
        self.as_mut().rust_mut().rows.clear();
        self.as_mut().end_reset_preview();
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
        self.as_mut().rust_mut().query.start(request);
        self.as_mut().set_state(QString::from("running"));
    }

    /// Checks the running preview (called by a timer while `state` is `running`).
    pub fn poll(mut self: Pin<&mut Self>) {
        match self.as_mut().rust_mut().query.poll_start() {
            StartPoll::Waiting => {}
            StartPoll::Failed(text) => self.as_mut().fail(&text),
            StartPoll::Started => self.as_mut().load_first_page(),
        }
    }

    /// Stops a running preview; nothing is written (the dialog was closed).
    pub fn cancel(mut self: Pin<&mut Self>) {
        if self.state.to_string() == "running" {
            self.as_mut().rust_mut().query.abandon();
            self.as_mut().set_state(QString::from("cancelled"));
        }
    }

    fn fail(mut self: Pin<&mut Self>, text: &str) {
        self.as_mut().set_error_text(QString::from(text));
        self.as_mut().set_state(QString::from("error"));
    }

    fn load_first_page(mut self: Pin<&mut Self>) {
        let first_load = self.state.to_string() != "done";
        let p = match self.as_mut().rust_mut().fetch(0) {
            Ok(None) => return,
            Ok(Some(p)) => p,
            // The daemon freed the preview: run it again.
            Err(KesticDbusError::NotFound(_)) => return self.as_mut().run_preview(),
            Err(err) => return self.as_mut().fail(&fetch_error_text(err)),
        };
        let rows: Vec<PreviewRow> = p.entries.items.iter().map(row_for).collect();
        self.as_mut().begin_reset_preview();
        self.as_mut().rust_mut().rows = rows;
        self.as_mut().end_reset_preview();
        self.as_mut().set_matched(to_i32(p.entries.matched));
        self.as_mut().set_too_large(p.entries.too_large);
        self.as_mut()
            .set_too_large_text(QString::from(&too_large_text(p.entries.total)));
        let filtered = !self.filter.is_empty() && !p.entries.too_large;
        self.as_mut().set_count_text(QString::from(&count_text(
            p.entries.total,
            p.entries.matched,
            filtered,
        )));
        if first_load {
            self.as_mut().apply_result(&p);
        }
        self.as_mut().rust_mut().result = Some(p);
        self.as_mut().set_state(QString::from("done"));
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
            .set_target_dir(QString::from(&tilde(&p.target.check.detail)));
        let check = p
            .target
            .check
            .code
            .message(&p.target.check.detail)
            .filter(|_| p.target.check.code != RestoreTargetCode::ExistsNotEmpty)
            .map_or_else(String::new, |m| tr(&m));
        self.as_mut().set_check_text(QString::from(&check));
        let usable = matches!(
            p.target.check.code,
            RestoreTargetCode::Ok | RestoreTargetCode::ExistsNotEmpty
        );
        let required = p.target.target.is_original()
            || p.target.check.code == RestoreTargetCode::ExistsNotEmpty;
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
        self.as_mut().set_can_restore(usable);
    }

    /// Starts the restore with the finished preview; `run_state` follows the run.
    pub fn start_restore(mut self: Pin<&mut Self>) {
        let Some(result) = self.result.clone() else {
            return;
        };
        if !self.can_restore || self.state.to_string() != "done" {
            return;
        }
        let request = RestoreRequest {
            destination_id: self.destination_id.clone(),
            snapshot_id: self.snapshot_id.clone(),
            selection: self.selection.clone(),
            target: result.target.target.clone(),
            overwrite: mode_of(self.overwrite),
            preview_id: self.query.id().map(str::to_string),
        };
        let started = make_client().and_then(|client| {
            client
                .start_restore(&request)
                .map_err(|e| dbus_error_text(&e))
        });
        match started {
            Ok(run_id) => {
                self.as_mut().rust_mut().run_id = Some(run_id);
                self.as_mut()
                    .set_run_target(QString::from(&result.target.check.detail));
                self.as_mut()
                    .set_run_text(QString::from(&i18n("Restoring files…").to_string()));
                self.as_mut().set_run_state(QString::from("running"));
            }
            Err(text) => {
                self.as_mut().set_run_text(QString::from(&text));
                self.as_mut().set_run_state(QString::from("error"));
            }
        }
    }

    /// Checks the running restore (called by a timer while `run_state` is `running`).
    pub fn poll_run(mut self: Pin<&mut Self>) {
        let Some(run_id) = self.run_id.clone() else {
            return;
        };
        let Ok(detail) =
            make_client().and_then(|c| c.get_run_detail(&run_id).map_err(|e| dbus_error_text(&e)))
        else {
            return;
        };
        let state = match detail.run.status {
            RunStatus::Running | RunStatus::Pending => {
                let progress = shared_state()
                    .lock_unpoisoned()
                    .active_progress
                    .get(&run_id)
                    .map(|p| tr(&active_run_message(&p.info())));
                if let Some(text) = progress {
                    self.as_mut().set_run_text(QString::from(&text));
                }
                return;
            }
            RunStatus::Ok => "ok",
            RunStatus::Warning => "warning",
            _ => "error",
        };
        self.as_mut()
            .set_run_text(QString::from(&run_end_text(&detail)));
        self.as_mut().set_run_state(QString::from(state));
    }

    /// Cancels the running restore; files written so far stay.
    pub fn cancel_run(self: Pin<&mut Self>) {
        if let (Some(run_id), Ok(client)) = (self.run_id.as_deref(), make_client()) {
            // A run that ended meanwhile needs no cancel.
            let _ = client.cancel_run(run_id);
        }
    }

    /// Hides the message of an ended restore.
    pub fn dismiss_run(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().run_id = None;
        self.as_mut().set_run_state(QString::from(""));
    }

    pub fn can_fetch_more(&self, parent: &QModelIndex) -> bool {
        !parent.is_valid()
            && self.state.to_string() == "done"
            && !self.too_large
            && i32::try_from(self.rows.len()).unwrap_or(i32::MAX) < self.matched
    }

    /// Appends the next page (at most 200 files).
    pub fn fetch_more(mut self: Pin<&mut Self>, parent: &QModelIndex) {
        if !self.can_fetch_more(parent) {
            return;
        }
        let offset = self.rows.len() as u64;
        match self.as_mut().rust_mut().fetch(offset) {
            Ok(Some(p)) if !p.entries.items.is_empty() => {
                let rows: Vec<PreviewRow> = p.entries.items.iter().map(row_for).collect();
                let first = row_count(&self.rows);
                let last = first + row_count(&rows) - 1;
                self.as_mut().begin_insert_rows_preview(parent, first, last);
                self.as_mut().rust_mut().rows.extend(rows);
                self.as_mut().end_insert_rows_preview();
            }
            Ok(_) => {}
            Err(KesticDbusError::NotFound(_)) => self.as_mut().run_preview(),
            Err(err) => self.as_mut().fail(&fetch_error_text(err)),
        }
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        row_count(&self.rows)
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        role_names::<PreviewRow>()
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        row_data(&self.rows, index, role)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestic_daemon::dbus_api::{
        ListPageDto, RestoreOriginalDto, RestorePreviewTargetDto, RestoreSuggestionDto,
        RestoreTargetCheckDto,
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

    #[test]
    fn modes_follow_the_list_order_and_default_to_never() {
        assert_eq!(mode_of(0), RestoreOverwrite::Never);
        assert_eq!(mode_of(3), RestoreOverwrite::Always);
        assert_eq!(mode_of(9), RestoreOverwrite::Never);
    }
}
