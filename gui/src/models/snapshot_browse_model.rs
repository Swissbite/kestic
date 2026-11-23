// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `SnapshotBrowseModel` implementation: the content of one folder of a snapshot. The
//! daemon lists the folder once (`ls` query) and holds it; the model fetches it page by page
//! while the view scrolls (`canFetchMore`/`fetchMore`) and filters it in the daemon. The
//! selection of entries to restore stays when the folder changes.

use super::qobject;
use super::snapshot_query::{
    QueryList, QueryListModel, QueryListView, count_text, to_i32, too_large_text,
};
use crate::state_cache::{format_bytes, i18np};
use crate::time_texts::to_local;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QModelIndex, QString, QStringList};
use kestic_core::query_list::QueryState;
use kestic_core::snapshot_find::parent_dir;
use kestic_core::snapshot_ls::path_segments;
use kestic_daemon::dbus_api::{
    LsEntryDto, LsResultDto, SnapshotQueryRequest, SnapshotQueryResultDto,
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

    destination_id: String,
    snapshot_id: String,
    /// Absolute snapshot paths marked for restore.
    selection: BTreeSet<String>,
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
        .map_or_default(|t| {
            let d = to_local(t).date();
            format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day())
        })
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
        self.set_query_state(Some(QueryState::Done));
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
            rust.destination_id = request.destination_id.clone();
            rust.snapshot_id = snapshot_id.to_string();
        }
        self.as_mut().clear_list();
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

    fn update_selection_text(mut self: Pin<&mut Self>) {
        let count = i32::try_from(self.selection.len()).unwrap_or(i32::MAX);
        let text = selection_text(&self.selection, &self.dir.to_string());
        self.as_mut().set_selection_count(count);
        self.as_mut().set_selection_text(QString::from(&text));
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
