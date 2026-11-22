// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `SnapshotDiffModel` implementation: the changes from one snapshot to another
//! (`diff` query), sorted by path. The daemon compares once and holds the changes; the model
//! fetches them page by page while the view scrolls and filters them in the daemon by text and
//! by kind of change.

use super::qobject;
use super::rows::{role_names, row_count, row_data};
use super::snapshot_query::{
    QueryRun, StartPoll, count_text, fetch_error_text, format_count, to_i32, too_large_text,
};
use crate::state_cache::{i18n, i18np, tr};
use crate::ui_texts::GuiText;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QVariant};
use kestic_core::query_list::{DEFAULT_PAGE_LIMIT, PageFilter};
use kestic_daemon::dbus_api::{
    DiffChangeDto, DiffResultDto, KesticDbusError, RestoreOutcome, SnapshotChange,
    SnapshotQueryPage, SnapshotQueryRequest, SnapshotQueryResultDto,
};
use std::pin::Pin;

model_row!(DiffRow {
    path: String => "path",
    // Code of the kind of change (`SnapshotChange`).
    change: String => "change",
    change_text: String => "changeText",
    icon_name: String => "iconName",
    is_dir: bool => "isDir",
});

#[derive(Default)]
pub struct SnapshotDiffModelRust {
    rows: Vec<DiffRow>,
    pub(crate) state: QString,
    pub(crate) error_text: QString,
    pub(crate) identical: bool,
    pub(crate) too_large: bool,
    pub(crate) total: i32,
    pub(crate) matched: i32,
    pub(crate) count_text: QString,
    pub(crate) too_large_text: QString,
    pub(crate) kind_codes: QStringList,
    pub(crate) kind_texts: QStringList,

    destination_id: String,
    from_snapshot_id: String,
    to_snapshot_id: String,
    filter: PageFilter,
    query: QueryRun,
}

/// Text of a kind of change, as the «Change» column and the filter chips show it.
pub(crate) fn change_text(change: SnapshotChange) -> String {
    match change {
        // The same word as for a file a restore adds.
        SnapshotChange::Added => tr(&RestoreOutcome::New.message()),
        SnapshotChange::Removed => i18n("Removed").subs(&[]),
        // restic reports metadata changes only with `--metadata`, which Kestic does not pass;
        // one that comes anyway reads as changed.
        SnapshotChange::Modified | SnapshotChange::Metadata => i18n("Changed").subs(&[]),
        SnapshotChange::TypeChanged => i18n("Type changed").subs(&[]),
        SnapshotChange::Bitrot => i18n("Content changed, metadata unchanged").subs(&[]),
    }
}

/// Icon of a kind of change, shown next to its text.
pub(crate) fn change_icon(change: SnapshotChange) -> &'static str {
    match change {
        SnapshotChange::Added => "list-add",
        SnapshotChange::Removed => "list-remove",
        SnapshotChange::Modified | SnapshotChange::Metadata => "document-edit",
        SnapshotChange::TypeChanged => "document-swap",
        SnapshotChange::Bitrot => "dialog-warning",
    }
}

fn row_for(change: &DiffChangeDto) -> DiffRow {
    DiffRow {
        path: change.path.clone(),
        change: change.change.as_str().to_string(),
        change_text: change_text(change.change),
        icon_name: change_icon(change.change).to_string(),
        // restic writes a folder with a trailing `/`.
        is_dir: change.path.ends_with('/'),
    }
}

/// Number of changes line: «9 changes», with a filter «3 of 9».
pub(crate) fn changes_count_text(total: u64, matched: u64, filtered: bool) -> String {
    if filtered {
        count_text(total, matched, true)
    } else {
        i18np("%1 change", "%1 changes", total, &[])
    }
}

/// Text of a filter chip: the kind of change and how many changes are of that kind.
pub(crate) fn chip_text(change: SnapshotChange, count: u64) -> String {
    GuiText::TwoParts.with_args(&[&change_text(change), &format_count(count)])
}

impl SnapshotDiffModelRust {
    /// Fetches the page at `offset` of the changes matching `filter`: `Ok(None)` while the
    /// comparison runs.
    fn fetch(
        &mut self,
        offset: u64,
        limit: u32,
        filter: PageFilter,
    ) -> Result<Option<DiffResultDto>, KesticDbusError> {
        let page = SnapshotQueryPage {
            offset,
            limit,
            filter,
        };
        match self.query.fetch_page(&page)? {
            Some(SnapshotQueryResultDto::Diff(diff)) => Ok(Some(diff)),
            Some(_) => Err(KesticDbusError::Internal(
                "unexpected snapshot query result".to_string(),
            )),
            None => Ok(None),
        }
    }

    /// The kinds of change present in the comparison with their number, in the order of
    /// [`SnapshotChange::ALL`].
    fn kind_counts(&mut self) -> Result<Vec<(SnapshotChange, u64)>, KesticDbusError> {
        let mut out = Vec::new();
        for change in SnapshotChange::ALL {
            let filter = PageFilter {
                text: String::new(),
                kinds: vec![change.as_str().to_string()],
            };
            if let Some(diff) = self.fetch(0, 1, filter)?
                && diff.changes.matched > 0
            {
                out.push((change, diff.changes.matched));
            }
        }
        Ok(out)
    }
}

impl qobject::SnapshotDiffModel {
    /// Compares snapshot `from_snapshot_id` with `to_snapshot_id` of `destination_id`.
    pub fn compare(
        mut self: Pin<&mut Self>,
        destination_id: &QString,
        from_snapshot_id: &QString,
        to_snapshot_id: &QString,
    ) {
        self.as_mut().abandon();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.destination_id = destination_id.to_string();
            rust.from_snapshot_id = from_snapshot_id.to_string();
            rust.to_snapshot_id = to_snapshot_id.to_string();
            rust.filter = PageFilter::default();
        }
        self.as_mut().begin_reset_diff();
        self.as_mut().rust_mut().rows.clear();
        self.as_mut().end_reset_diff();
        self.as_mut().set_identical(false);
        self.as_mut().set_too_large(false);
        self.as_mut().set_total(0);
        self.as_mut().set_matched(0);
        self.as_mut().set_count_text(QString::from(""));
        self.as_mut().set_error_text(QString::from(""));
        self.as_mut().set_kind_codes(QStringList::default());
        self.as_mut().set_kind_texts(QStringList::default());
        let request = SnapshotQueryRequest::diff(
            &destination_id.to_string(),
            &from_snapshot_id.to_string(),
            &to_snapshot_id.to_string(),
        );
        self.as_mut().rust_mut().query.start(request);
        self.as_mut().set_state(QString::from("running"));
    }

    /// Checks the running comparison (called by a timer while `state` is `running`).
    pub fn poll(mut self: Pin<&mut Self>) {
        match self.as_mut().rust_mut().query.poll_start() {
            StartPoll::Waiting => {}
            StartPoll::Failed(text) => self.as_mut().fail(&text),
            StartPoll::Started => self.as_mut().load_first_page(),
        }
    }

    /// Filters the changes by `text` (any case, in the paths); the daemon filters.
    pub fn set_filter(mut self: Pin<&mut Self>, text: &QString) {
        let text = text.to_string();
        if text == self.filter.text {
            return;
        }
        self.as_mut().rust_mut().filter.text = text;
        self.as_mut().reload_if_done();
    }

    /// Shows only the changes of the kinds `codes` (`SnapshotChange` codes); none shows all.
    pub fn set_kinds(mut self: Pin<&mut Self>, codes: &QStringList) {
        let kinds: Vec<String> = codes
            .iter()
            .map(ToString::to_string)
            .filter(|code| SnapshotChange::parse(code).is_some())
            .collect();
        if kinds == self.filter.kinds {
            return;
        }
        self.as_mut().rust_mut().filter.kinds = kinds;
        self.as_mut().reload_if_done();
    }

    /// Stops the running comparison (the user cancels it or the page closes).
    pub fn cancel(self: Pin<&mut Self>) {
        self.abandon();
    }

    fn reload_if_done(mut self: Pin<&mut Self>) {
        if self.state.to_string() == "done" {
            self.as_mut().load_first_page();
        }
    }

    fn abandon(mut self: Pin<&mut Self>) {
        if self.state.to_string() != "running" {
            return;
        }
        self.as_mut().rust_mut().query.abandon();
        self.as_mut().set_state(QString::from("cancelled"));
    }

    fn fail(mut self: Pin<&mut Self>, text: &str) {
        self.as_mut().set_error_text(QString::from(text));
        self.as_mut().set_state(QString::from("error"));
    }

    fn load_first_page(mut self: Pin<&mut Self>) {
        let first_load = self.state.to_string() != "done";
        let filter = self.filter.clone();
        let res = self
            .as_mut()
            .rust_mut()
            .fetch(0, DEFAULT_PAGE_LIMIT, filter);
        let diff = match res {
            Ok(None) => return,
            Ok(Some(diff)) => diff,
            Err(KesticDbusError::NotFound(_)) => return self.as_mut().compare_again(),
            Err(err) => return self.as_mut().fail(&fetch_error_text(err)),
        };
        if first_load && !diff.changes.too_large && diff.changes.total > 0 {
            match self.as_mut().rust_mut().kind_counts() {
                Ok(counts) => self.as_mut().apply_kind_counts(&counts),
                Err(KesticDbusError::NotFound(_)) => return self.as_mut().compare_again(),
                Err(err) => return self.as_mut().fail(&fetch_error_text(err)),
            }
        }
        self.as_mut().apply_first_page(&diff);
    }

    /// Compares again (the daemon freed the old result); the filter stays.
    fn compare_again(mut self: Pin<&mut Self>) {
        let dest = QString::from(&self.destination_id);
        let from = QString::from(&self.from_snapshot_id);
        let to = QString::from(&self.to_snapshot_id);
        let filter = self.filter.clone();
        self.as_mut().compare(&dest, &from, &to);
        self.as_mut().rust_mut().filter = filter;
    }

    fn apply_kind_counts(mut self: Pin<&mut Self>, counts: &[(SnapshotChange, u64)]) {
        let mut codes = QStringList::default();
        let mut texts = QStringList::default();
        for (change, count) in counts {
            codes.append(QString::from(change.as_str()));
            texts.append(QString::from(&chip_text(*change, *count)));
        }
        self.as_mut().set_kind_codes(codes);
        self.as_mut().set_kind_texts(texts);
    }

    fn apply_first_page(mut self: Pin<&mut Self>, diff: &DiffResultDto) {
        let changes = &diff.changes;
        let rows: Vec<DiffRow> = changes.items.iter().map(row_for).collect();
        self.as_mut().begin_reset_diff();
        self.as_mut().rust_mut().rows = rows;
        self.as_mut().end_reset_diff();
        self.as_mut().set_identical(diff.identical);
        self.as_mut().set_too_large(changes.too_large);
        self.as_mut().set_total(to_i32(changes.total));
        self.as_mut().set_matched(to_i32(changes.matched));
        // A list too large to hold cannot be filtered: its count is the total.
        let filtered = !self.filter.is_empty() && !changes.too_large;
        let count = changes_count_text(changes.total, changes.matched, filtered);
        self.as_mut().set_count_text(QString::from(&count));
        self.as_mut()
            .set_too_large_text(QString::from(&too_large_text(changes.total)));
        self.as_mut().set_error_text(QString::from(""));
        self.as_mut().set_state(QString::from("done"));
    }

    pub fn can_fetch_more(&self, parent: &QModelIndex) -> bool {
        !parent.is_valid()
            && self.state.to_string() == "done"
            && !self.too_large
            && i32::try_from(self.rows.len()).unwrap_or(i32::MAX) < self.matched
    }

    /// Appends the next page (at most 200 changes) at the end of the list.
    pub fn fetch_more(mut self: Pin<&mut Self>, parent: &QModelIndex) {
        if !self.can_fetch_more(parent) {
            return;
        }
        let offset = self.rows.len() as u64;
        let filter = self.filter.clone();
        let res = self
            .as_mut()
            .rust_mut()
            .fetch(offset, DEFAULT_PAGE_LIMIT, filter);
        match res {
            Ok(Some(diff)) if !diff.changes.items.is_empty() => {
                let rows: Vec<DiffRow> = diff.changes.items.iter().map(row_for).collect();
                let first = row_count(&self.rows);
                let last = first + row_count(&rows) - 1;
                self.as_mut().begin_insert_rows_diff(parent, first, last);
                self.as_mut().rust_mut().rows.extend(rows);
                self.as_mut().end_insert_rows_diff();
            }
            Ok(_) => {}
            Err(KesticDbusError::NotFound(_)) => self.as_mut().compare_again(),
            Err(err) => self.as_mut().fail(&fetch_error_text(err)),
        }
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        row_count(&self.rows)
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        role_names::<DiffRow>()
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        row_data(&self.rows, index, role)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_name_the_kind_of_change_and_mark_folders() {
        let file = row_for(&DiffChangeDto {
            path: "/home/u/docs/a.txt".to_string(),
            change: SnapshotChange::Modified,
        });
        assert_eq!(file.change, "modified");
        assert_eq!(file.change_text, "Changed");
        assert_eq!(file.icon_name, "document-edit");
        assert!(!file.is_dir);
        let dir = row_for(&DiffChangeDto {
            path: "/home/u/new/".to_string(),
            change: SnapshotChange::Added,
        });
        assert_eq!(dir.change_text, "New");
        assert!(dir.is_dir);
        let metadata = row_for(&DiffChangeDto {
            path: "/home/u/docs/b.txt".to_string(),
            change: SnapshotChange::Metadata,
        });
        assert_eq!(metadata.change, "metadata");
        assert_eq!(metadata.change_text, "Changed");
    }

    #[test]
    fn count_texts_name_changes_and_the_filter() {
        assert_eq!(changes_count_text(9, 9, false), "9 changes");
        assert_eq!(changes_count_text(1, 1, false), "1 change");
        assert_eq!(changes_count_text(9, 3, true), "3 of 9");
        assert_eq!(chip_text(SnapshotChange::Removed, 2), "Removed · 2");
    }
}
