// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `SnapshotBrowseModel` implementation: the content of one folder of a snapshot. The
//! daemon lists the folder once (`ls` query) and holds it; the model fetches it page by page
//! while the view scrolls (`canFetchMore`/`fetchMore`) and filters it in the daemon. The
//! selection of entries to restore stays when the folder changes.

use super::qobject;
use super::snapshot_query::{
    Fetched, QueryList, QueryListModel, QueryListView, count_text, to_i32, too_large_text,
};
use crate::state_cache::{format_bytes, i18np};
use crate::time_texts::{iso_date, to_local};
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QModelIndex, QString, QStringList};
use kestic_core::query_list::{MAX_PAGE_LIMIT, QueryState};
use kestic_core::restore::selection_is_whole_folder;
use kestic_core::snapshot_ls::{parent_dir, path_segments};
use kestic_daemon::dbus_api::{
    LsEntryDto, LsResultDto, SnapshotQueryPage, SnapshotQueryRequest, SnapshotQueryResultDto,
};
use std::collections::BTreeSet;
use std::pin::Pin;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Pages [`qobject::SnapshotBrowseModel::row_of_path`] fetches at most; each page is one call to
/// the daemon while the window waits.
const FIND_ROW_PAGES: usize = 10;

model_row!(BrowseRow {
    path: String => "path",
    name: String => "name",
    is_dir: bool => "isDir",
    icon_name: String => "iconName",
    size_text: String => "sizeText",
    modified_text: String => "modifiedText",
    selected: bool => "selected",
});

#[derive(Default)]
pub struct SnapshotBrowseModelRust {
    /// The entries of the current folder and the query that lists it.
    list: QueryList<BrowseRow>,
    pub(crate) state: QString,
    pub(crate) error_text: QString,
    pub(crate) dir: QString,
    pub(crate) dir_exists: bool,
    pub(crate) too_large: bool,
    pub(crate) total: i32,
    pub(crate) matched: i32,
    pub(crate) count_text: QString,
    pub(crate) too_large_text: QString,
    pub(crate) selection_count: i32,
    pub(crate) selection_text: QString,
    /// The folder a restore of the whole snapshot runs from (set by the view).
    pub(crate) top_dir: QString,
    /// Whether the selection restores the same files as the whole snapshot.
    pub(crate) whole_snapshot: bool,
    /// The header check box: 0 none, 1 some, 2 all entries of the list shown selected.
    pub(crate) header_check_state: i32,

    destination_id: String,
    snapshot_id: String,
    /// Absolute snapshot paths marked for restore.
    selection: BTreeSet<String>,
    /// Number of entries of `top_dir`, once it was listed.
    top_entries: Option<u64>,
    /// Every entry the list shows with the filter, once fetched, with that filter text.
    shown: Option<(String, Vec<String>)>,
}

/// Size column: «–» for folders and links, the size for files.
fn size_text(e: &LsEntryDto) -> String {
    match (e.is_dir(), e.size) {
        (false, Some(bytes)) if e.entry_type == "file" => format_bytes(bytes),
        _ => "–".to_string(),
    }
}

/// «Modified» column: the local date of the modification time.
fn modified_text(e: &LsEntryDto) -> String {
    e.mtime
        .as_deref()
        .and_then(|t| OffsetDateTime::parse(t, &Rfc3339).ok())
        .map_or_default(|t| iso_date(to_local(t).date()))
}

fn icon_name(e: &LsEntryDto) -> &'static str {
    match e.entry_type.as_str() {
        "dir" => "folder",
        "symlink" => "inode-symlink",
        _ => "text-x-generic",
    }
}

/// The last segment of a snapshot path; `/` for the root.
pub(super) fn last_name(path: &str) -> &str {
    match path.rsplit('/').next() {
        Some(name) if !name.is_empty() => name,
        _ => "/",
    }
}

/// The state of the header check box for `selected` of the `shown` entries of the list:
/// `Qt.Unchecked` (0) for none, `Qt.PartiallyChecked` (1) for some, `Qt.Checked` (2) for all.
/// An empty list, or one not listed yet, is unchecked.
pub(crate) fn header_check_state(selected: u64, shown: u64) -> i32 {
    match selected {
        _ if shown == 0 => 0,
        0 => 0,
        n if n >= shown => 2,
        _ => 1,
    }
}

/// Selection line: the number of selected entries, how many are in `dir`, and where the
/// others are («3 items selected (2 here, 1 in ‹Pictures›)»).
pub(crate) fn selection_text(selection: &BTreeSet<String>, dir: &str) -> String {
    let n = selection.len();
    let here = selection.iter().filter(|p| parent_dir(p) == dir).count();
    let elsewhere: BTreeSet<&str> = selection
        .iter()
        .map(|p| parent_dir(p))
        .filter(|parent| *parent != dir)
        .collect();
    let others = (n - here).to_string();
    let here = here.to_string();
    match elsewhere.len() {
        0 => i18np("%1 item selected", "%1 items selected", n, &[]),
        1 => {
            let folder = elsewhere.first().map_or("", |p| last_name(p));
            i18np(
                "%1 item selected (%2 here, %3 in ‹%4›)",
                "%1 items selected (%2 here, %3 in ‹%4›)",
                n,
                &[&here, &others, folder],
            )
        }
        _ => i18np(
            "%1 item selected (%2 here, %3 in other folders)",
            "%1 items selected (%2 here, %3 in other folders)",
            n,
            &[&here, &others],
        ),
    }
}

impl SnapshotBrowseModelRust {
    fn row_for(&self, e: &LsEntryDto) -> BrowseRow {
        BrowseRow {
            path: e.path.clone(),
            name: e.name.clone(),
            is_dir: e.is_dir(),
            icon_name: icon_name(e).to_string(),
            size_text: size_text(e),
            modified_text: modified_text(e),
            selected: self.selection.contains(&e.path),
        }
    }
}

query_list_model!(
    SnapshotBrowseModel,
    BrowseRow,
    begin_reset: begin_reset_browse,
    end_reset: end_reset_browse,
    begin_insert: begin_insert_rows_browse,
    end_insert: end_insert_rows_browse,
);

impl QueryListModel for qobject::SnapshotBrowseModel {
    type Page = LsResultDto;

    fn page_of(result: SnapshotQueryResultDto) -> Option<LsResultDto> {
        match result {
            SnapshotQueryResultDto::Ls(ls) => Some(ls),
            _ => None,
        }
    }

    fn page_rows(&self, ls: &LsResultDto) -> Vec<BrowseRow> {
        ls.entries.items.iter().map(|e| self.row_for(e)).collect()
    }

    fn show_first_page(mut self: Pin<&mut Self>, ls: LsResultDto) {
        let rows = self.page_rows(&ls);
        self.as_mut().reset_rows(rows);
        self.as_mut().set_dir_exists(ls.dir_exists);
        self.as_mut().set_too_large(ls.entries.too_large);
        self.as_mut().set_total(to_i32(ls.entries.total));
        self.as_mut().set_matched(to_i32(ls.entries.matched));
        // A list too large to hold cannot be filtered: its count is the total.
        let filtered = !self.list.filter.is_empty() && !ls.entries.too_large;
        let count = count_text(ls.entries.total, ls.entries.matched, filtered);
        self.as_mut().set_count_text(QString::from(&count));
        let too_large = too_large_text(ls.entries.total);
        self.as_mut().set_too_large_text(QString::from(&too_large));
        self.as_mut().set_error_text(QString::from(""));
        if ls.dir_exists && ls.dir == self.top_dir.to_string() {
            self.as_mut().rust_mut().top_entries = Some(ls.entries.total);
        }
        self.as_mut().set_query_state(Some(QueryState::Done));
        self.update_selection_text();
    }

    /// Lists the current folder again; the filter stays.
    fn restart(mut self: Pin<&mut Self>) {
        let dest = QString::from(&self.destination_id);
        let snap = QString::from(&self.snapshot_id);
        let dir = self.dir.clone();
        let filter = self.list.filter.clone();
        self.as_mut().open(&dest, &snap, &dir);
        self.as_mut().list_mut().filter = filter;
    }
}

impl qobject::SnapshotBrowseModel {
    /// Lists `dir` of `snapshot_id` in `destination_id`; the selection stays. The query of the
    /// folder being left is cancelled while it still runs, so restic stops.
    pub fn open(
        mut self: Pin<&mut Self>,
        destination_id: &QString,
        snapshot_id: &QString,
        dir: &QString,
    ) {
        self.as_mut().abandon_query();
        let request = SnapshotQueryRequest::ls(
            &destination_id.to_string(),
            &snapshot_id.to_string(),
            &dir.to_string(),
        );
        {
            let mut rust = self.as_mut().rust_mut();
            if rust.snapshot_id != snapshot_id.to_string() {
                rust.top_entries = None;
            }
            rust.destination_id = request.destination_id.clone();
            rust.snapshot_id = snapshot_id.to_string();
        }
        self.as_mut().clear_list();
        self.as_mut().rust_mut().shown = None;
        self.as_mut().set_dir(dir.clone());
        self.as_mut().set_dir_exists(true);
        self.as_mut().set_too_large(false);
        self.as_mut().set_total(0);
        self.as_mut().set_matched(0);
        self.as_mut().set_count_text(QString::from(""));
        self.as_mut().set_error_text(QString::from(""));
        self.as_mut().update_selection_text();
        self.as_mut().start_query(request);
    }

    /// The row of the entry at `path`, fetching at most [`FIND_ROW_PAGES`] further pages while it
    /// is not loaded yet; -1 when the folder does not list it there.
    pub fn row_of_path(mut self: Pin<&mut Self>, path: &QString) -> i32 {
        let path = path.to_string();
        let parent = QModelIndex::default();
        for _ in 0..FIND_ROW_PAGES {
            if self.list.rows.iter().any(|r| r.path == path) || !self.can_fetch_more(&parent) {
                break;
            }
            let before = self.list.rows.len();
            self.as_mut().fetch_more(&parent);
            if self.list.rows.len() == before {
                break;
            }
        }
        self.list
            .rows
            .iter()
            .position(|r| r.path == path)
            .map_or(-1, |row| to_i32(row as u64))
    }

    /// Marks or unmarks the entry at `row` for restore.
    pub fn toggle_selected(mut self: Pin<&mut Self>, row: i32) {
        let Some(index) = usize::try_from(row)
            .ok()
            .filter(|&i| i < self.list.rows.len())
        else {
            return;
        };
        {
            let mut rust = self.as_mut().rust_mut();
            let path = rust.list.rows[index].path.clone();
            let selected = !rust.selection.remove(&path);
            if selected {
                rust.selection.insert(path);
            }
            rust.list.rows[index].selected = selected;
        }
        let model_index = self.as_ref().index_browse(row, 0, &QModelIndex::default());
        self.as_mut()
            .data_changed_browse(&model_index, &model_index);
        self.as_mut().update_selection_text();
    }

    /// Unmarks every entry, in this folder and elsewhere.
    pub fn clear_selection(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().selection.clear();
        let changed: Vec<usize> = self
            .list
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selected)
            .map(|(i, _)| i)
            .collect();
        for i in changed {
            self.as_mut().rust_mut().list.rows[i].selected = false;
            let row = i32::try_from(i).unwrap_or(i32::MAX);
            let model_index = self.as_ref().index_browse(row, 0, &QModelIndex::default());
            self.as_mut()
                .data_changed_browse(&model_index, &model_index);
        }
        self.as_mut().update_selection_text();
    }

    /// Marks every entry of the list shown, also those not loaded yet (all pages, with the
    /// filter).
    pub fn select_all(mut self: Pin<&mut Self>) {
        // Everything shown is selected already: nothing to fetch (Ctrl+A held down).
        if self.header_check_state == 2 {
            return;
        }
        let Some(paths) = self.as_mut().shown_paths() else {
            return;
        };
        self.as_mut().rust_mut().selection.extend(paths);
        self.as_mut().show_selection_in_rows();
    }

    /// The header check box: unmarks the entries of the list shown when all are marked,
    /// otherwise marks them all.
    pub fn toggle_all(mut self: Pin<&mut Self>) {
        if self.header_check_state != 2 {
            self.select_all();
            return;
        }
        let Some(paths) = self.as_mut().shown_paths() else {
            return;
        };
        {
            let mut rust = self.as_mut().rust_mut();
            for path in &paths {
                rust.selection.remove(path);
            }
        }
        self.as_mut().show_selection_in_rows();
    }

    /// The paths of every entry of the list shown, fetched from the daemon in pages without
    /// adding them to the view; `None` while the list is not listed, when it is too large to
    /// hold, or when a page cannot be fetched.
    fn shown_paths(mut self: Pin<&mut Self>) -> Option<Vec<String>> {
        if self.list.state != Some(QueryState::Done) || self.too_large {
            return None;
        }
        let matched = usize::try_from(self.matched).unwrap_or(0);
        let mut paths = Vec::with_capacity(matched);
        while paths.len() < matched {
            let page = SnapshotQueryPage {
                offset: paths.len() as u64,
                limit: MAX_PAGE_LIMIT,
                filter: self.list.filter.clone(),
            };
            match self.as_mut().fetch(&page) {
                Fetched::Page(ls) if !ls.entries.items.is_empty() => {
                    paths.extend(ls.entries.items.into_iter().map(|e| e.path));
                }
                Fetched::Page(_) | Fetched::Running => return None,
                Fetched::Gone => {
                    self.restart();
                    return None;
                }
                Fetched::Failed(failure) => {
                    self.fail_query(&failure);
                    return None;
                }
            }
        }
        let filter = self.list.filter.text.clone();
        self.as_mut().rust_mut().shown = Some((filter, paths.clone()));
        Some(paths)
    }

    /// Shows the selection in the check boxes of the loaded rows and in the selection line.
    fn show_selection_in_rows(mut self: Pin<&mut Self>) {
        let changed: Vec<usize> = self
            .list
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selected != self.selection.contains(&r.path))
            .map(|(i, _)| i)
            .collect();
        for i in changed {
            let selected = !self.list.rows[i].selected;
            self.as_mut().rust_mut().list.rows[i].selected = selected;
            let row = i32::try_from(i).unwrap_or(i32::MAX);
            let model_index = self.as_ref().index_browse(row, 0, &QModelIndex::default());
            self.as_mut()
                .data_changed_browse(&model_index, &model_index);
        }
        self.as_mut().update_selection_text();
    }

    /// How many entries of the list shown are selected, of how many: without a filter the
    /// selected entries of the folder of all its entries, with a filter the selected loaded rows
    /// of all matching entries.
    fn shown_selected(&self) -> (u64, u64) {
        if self.list.filter.is_empty() {
            let dir = self.dir.to_string();
            let here = self
                .selection
                .iter()
                .filter(|p| parent_dir(p) == dir)
                .count();
            (here as u64, u64::try_from(self.total).unwrap_or(0))
        } else if let Some((_, paths)) = self
            .shown
            .as_ref()
            .filter(|(filter, _)| *filter == self.list.filter.text)
        {
            // Every matching entry is known (fetched by «Select All»), also those not loaded.
            let marked = paths.iter().filter(|p| self.selection.contains(*p)).count();
            (marked as u64, paths.len() as u64)
        } else {
            let marked = self.list.rows.iter().filter(|r| r.selected).count();
            (marked as u64, u64::try_from(self.matched).unwrap_or(0))
        }
    }

    fn update_selection_text(mut self: Pin<&mut Self>) {
        let count = i32::try_from(self.selection.len()).unwrap_or(i32::MAX);
        let text = selection_text(&self.selection, &self.dir.to_string());
        let (selected, shown) = self.shown_selected();
        let whole = self.top_entries.is_some_and(|entries| {
            selection_is_whole_folder(&self.selection, &self.top_dir.to_string(), entries)
        });
        self.as_mut().set_selection_count(count);
        self.as_mut().set_selection_text(QString::from(&text));
        self.as_mut()
            .set_header_check_state(header_check_state(selected, shown));
        self.as_mut().set_whole_snapshot(whole);
    }

    /// The widest text the size column shows (a file below 10 TB), to size the column.
    pub fn size_sample(&self) -> QString {
        QString::from(&format_bytes(9_999_900_000_000))
    }

    /// The selection to restore: none (the whole snapshot) when it covers every entry of the
    /// top folder, otherwise the marked entries.
    pub fn restore_selection(&self) -> QStringList {
        if self.whole_snapshot {
            QStringList::default()
        } else {
            self.selected_paths()
        }
    }

    /// The marked entries (absolute snapshot paths), sorted.
    pub fn selected_paths(&self) -> QStringList {
        let mut list = QStringList::default();
        for p in &self.selection {
            list.append(QString::from(p));
        }
        list
    }

    /// Folder names from the top of the snapshot down to `dir`; the top has an empty name.
    pub fn segment_names(&self, dir: &QString) -> QStringList {
        let mut list = QStringList::default();
        for (name, _) in path_segments(&dir.to_string()) {
            list.append(QString::from(&name));
        }
        list
    }

    /// Folder paths from the top of the snapshot down to `dir`.
    pub fn segment_paths(&self, dir: &QString) -> QStringList {
        let mut list = QStringList::default();
        for (_, path) in path_segments(&dir.to_string()) {
            list.append(QString::from(&path));
        }
        list
    }
}

#[cfg(test)]
#[path = "snapshot_browse_model_tests.rs"]
mod tests;
