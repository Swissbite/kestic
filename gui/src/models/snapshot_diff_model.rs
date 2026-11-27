// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `SnapshotDiffModel` implementation: the changes from one snapshot to another
//! (`diff` query), sorted by path. The daemon compares once and holds the changes; the model
//! fetches them page by page while the view scrolls and filters them in the daemon by text and
//! by kind of change.

use super::qobject;
use super::snapshot_query::{
    Fetched, QueryList, QueryListModel, QueryListView, count_text, format_count, to_i32,
    too_large_text,
};
use crate::state_cache::{i18n, i18np, tr};
use crate::ui_texts::GuiText;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};
use kestic_core::query_list::{PageFilter, QueryState};
use kestic_daemon::dbus_api::{
    DiffChangeDto, DiffResultDto, RestoreOutcome, SnapshotChange, SnapshotQueryPage,
    SnapshotQueryRequest, SnapshotQueryResultDto,
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
    list: QueryList<DiffRow>,
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
}

/// Text of a kind of change, as the «Change» column and the filter chips show it.
pub(crate) fn change_text(change: SnapshotChange) -> String {
    match change {
        // The same word as for a file a restore adds.
        SnapshotChange::Added => tr(&RestoreOutcome::New.message()),
        SnapshotChange::Removed => i18n("Removed").subs(&[]),
        // restic reports metadata changes only with `--metadata`, which Kestic does not pass;
        // one that comes anyway reads as changed.
        SnapshotChange::Modified => i18n("Changed").subs(&[]),
        SnapshotChange::TypeChanged => i18n("Type changed").subs(&[]),
        SnapshotChange::Bitrot => i18n("Content changed, metadata unchanged").subs(&[]),
    }
}

/// Icon of a kind of change, shown next to its text.
pub(crate) fn change_icon(change: SnapshotChange) -> &'static str {
    match change {
        SnapshotChange::Added => "list-add",
        SnapshotChange::Removed => "list-remove",
        SnapshotChange::Modified => "document-edit",
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

query_list_model!(
    SnapshotDiffModel,
    DiffRow,
    begin_reset: begin_reset_diff,
    end_reset: end_reset_diff,
    begin_insert: begin_insert_rows_diff,
    end_insert: end_insert_rows_diff,
);

impl QueryListModel for qobject::SnapshotDiffModel {
    type Page = DiffResultDto;

    fn page_of(result: SnapshotQueryResultDto) -> Option<DiffResultDto> {
        match result {
            SnapshotQueryResultDto::Diff(diff) => Some(diff),
            _ => None,
        }
    }

    fn page_rows(&self, diff: &DiffResultDto) -> Vec<DiffRow> {
        diff.changes.items.iter().map(row_for).collect()
    }

    /// Shows the first page; when the comparison just ended, also the kinds of change present.
    fn show_first_page(mut self: Pin<&mut Self>, diff: DiffResultDto) {
        let first_load = self.query_state() != Some(QueryState::Done);
        let changes = &diff.changes;
        if first_load && !changes.too_large && changes.total > 0 {
            self.as_mut().load_kind_counts();
        }
        let rows = self.page_rows(&diff);
        self.as_mut().reset_rows(rows);
        self.as_mut().set_identical(diff.identical);
        self.as_mut().set_too_large(changes.too_large);
        self.as_mut().set_total(to_i32(changes.total));
        self.as_mut().set_matched(to_i32(changes.matched));
        // A list too large to hold cannot be filtered: its count is the total.
        let filtered = !self.list.filter.is_empty() && !changes.too_large;
        let count = changes_count_text(changes.total, changes.matched, filtered);
        self.as_mut().set_count_text(QString::from(&count));
        self.as_mut()
            .set_too_large_text(QString::from(&too_large_text(changes.total)));
        self.as_mut().set_error_text(QString::from(""));
        self.set_query_state(Some(QueryState::Done));
    }

    /// Compares again; the filter stays.
    fn restart(mut self: Pin<&mut Self>) {
        let dest = QString::from(&self.destination_id);
        let from = QString::from(&self.from_snapshot_id);
        let to = QString::from(&self.to_snapshot_id);
        let filter = self.list.filter.clone();
        self.as_mut().compare(&dest, &from, &to);
        self.as_mut().list_mut().filter = filter;
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
        self.as_mut().abandon_query();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.destination_id = destination_id.to_string();
            rust.from_snapshot_id = from_snapshot_id.to_string();
            rust.to_snapshot_id = to_snapshot_id.to_string();
        }
        self.as_mut().clear_list();
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
        self.as_mut().start_query(request);
    }

    /// Shows only the changes of the kinds `codes` (`SnapshotChange` codes); none shows all.
    pub fn set_kinds(self: Pin<&mut Self>, codes: &QStringList) {
        let kinds: Vec<String> = codes
            .iter()
            .map(ToString::to_string)
            .filter(|code| SnapshotChange::parse(code).is_some())
            .collect();
        self.set_filter_kinds(kinds);
    }

    /// Asks for the kinds of change present in the comparison with their number, in the order of
    /// [`SnapshotChange::ALL`]; the chips show once they arrived.
    fn load_kind_counts(self: Pin<&mut Self>) {
        let pages = SnapshotChange::ALL
            .iter()
            .map(|change| SnapshotQueryPage {
                offset: 0,
                limit: 1,
                filter: PageFilter {
                    text: String::new(),
                    kinds: vec![change.as_str().to_string()],
                },
            })
            .collect();
        self.fetch_pages(pages, |mut model, fetched| match fetched {
            Fetched::Page(diffs) => {
                let counts: Vec<(SnapshotChange, u64)> = SnapshotChange::ALL
                    .into_iter()
                    .zip(diffs)
                    .filter(|(_, diff)| diff.changes.matched > 0)
                    .map(|(change, diff)| (change, diff.changes.matched))
                    .collect();
                model.as_mut().apply_kind_counts(&counts);
            }
            Fetched::Running => {}
            Fetched::Gone => model.restart(),
            Fetched::Failed(failure) => model.fail_query(&failure),
        });
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
    }

    #[test]
    fn count_texts_name_changes_and_the_filter() {
        assert_eq!(changes_count_text(9, 9, false), "9 changes");
        assert_eq!(changes_count_text(1, 1, false), "1 change");
        assert_eq!(changes_count_text(9, 3, true), "3 of 9");
        assert_eq!(chip_text(SnapshotChange::Removed, 2), "Removed · 2");
    }
}
