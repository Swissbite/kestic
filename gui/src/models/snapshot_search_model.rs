// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `SnapshotSearchModel` implementation: the hits of a search in the snapshots of one
//! backup (`find` query), newest snapshot first. The daemon searches once and holds the hits;
//! the model fetches them page by page while the view scrolls and filters them in the daemon.

use super::qobject;
use super::rows::{role_names, row_count, row_data};
use super::snapshot_query::{
    QueryRun, StartPoll, count_text, fetch_error_text, to_i32, too_large_text,
};
use crate::state_cache::{format_bytes, i18n, i18np};
use crate::time_texts::format_datetime_display;
use crate::ui_texts::GuiText;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_daemon::dbus_api::{
    FindHitDto, FindResultDto, FindScope, KesticDbusError, SnapshotQueryRequest,
    SnapshotQueryResultDto,
};
use std::pin::Pin;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

model_row!(SearchRow {
    path: String => "path",
    snapshot_id: String => "snapshotId",
    // Date and time of the snapshot, also for the breadcrumb of the snapshot browser.
    snapshot_text: String => "snapshotText",
    // The folder that shows the hit in the snapshot browser.
    folder: String => "folder",
    is_dir: bool => "isDir",
    icon_name: String => "iconName",
    subtitle: String => "subtitle",
});

#[derive(Default)]
pub struct SnapshotSearchModelRust {
    rows: Vec<SearchRow>,
    pub(crate) state: QString,
    pub(crate) error_text: QString,
    pub(crate) search_text: QString,
    pub(crate) too_large: bool,
    pub(crate) total: i32,
    pub(crate) matched: i32,
    pub(crate) count_text: QString,
    pub(crate) too_large_text: QString,
    pub(crate) no_hits_text: QString,

    destination_id: String,
    backup_id: String,
    filter: String,
    query: QueryRun,
}

/// Date and time of the snapshot of `hit`, as restic reports it when it does not parse.
fn snapshot_text(hit: &FindHitDto) -> String {
    OffsetDateTime::parse(&hit.snapshot_time, &Rfc3339)
        .map_or_else(|_| hit.snapshot_time.clone(), format_datetime_display)
}

/// Second line of a hit: the snapshot, and the size of a file (restic reports none for an empty
/// one).
fn subtitle(hit: &FindHitDto, snapshot: &str) -> String {
    if hit.entry_type == "file" {
        let size = format_bytes(hit.size.unwrap_or(0));
        GuiText::TwoParts.with_args(&[snapshot, &size])
    } else {
        snapshot.to_string()
    }
}

fn icon_name(hit: &FindHitDto) -> &'static str {
    match hit.entry_type.as_str() {
        "dir" => "folder",
        "symlink" => "inode-symlink",
        _ => "text-x-generic",
    }
}

fn row_for(hit: &FindHitDto) -> SearchRow {
    let snapshot = snapshot_text(hit);
    SearchRow {
        path: hit.path.clone(),
        snapshot_id: hit.snapshot_id.clone(),
        subtitle: subtitle(hit, &snapshot),
        snapshot_text: snapshot,
        folder: hit.folder().to_string(),
        is_dir: hit.entry_type == "dir",
        icon_name: icon_name(hit).to_string(),
    }
}

/// Line above the hits: «2 hits for ‹bericht.pdf› in 2 snapshots».
pub(crate) fn hits_text(total: u64, snapshots: u64, text: &str) -> String {
    let hits = i18np("%1 hit for ‹%2›", "%1 hits for ‹%2›", total, &[text]);
    let within = i18np("in %1 snapshot", "in %1 snapshots", snapshots, &[]);
    format!("{hits} {within}")
}

impl SnapshotSearchModelRust {
    /// Fetches the page at `offset` of the hits: `Ok(None)` while the search runs.
    fn fetch(&mut self, offset: u64) -> Result<Option<FindResultDto>, KesticDbusError> {
        let filter = self.filter.clone();
        match self.query.fetch(offset, &filter)? {
            Some(SnapshotQueryResultDto::Find(found)) => Ok(Some(found)),
            Some(_) => Err(KesticDbusError::Internal(
                "unexpected snapshot query result".to_string(),
            )),
            None => Ok(None),
        }
    }
}

impl qobject::SnapshotSearchModel {
    /// Searches the snapshots of `backup_id` in `destination_id` for paths that contain
    /// `text` (taken literally, any case). An empty text clears the hits.
    pub fn search(
        mut self: Pin<&mut Self>,
        destination_id: &QString,
        backup_id: &QString,
        text: &QString,
    ) {
        self.as_mut().abandon();
        let text = text.to_string().trim().to_string();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.destination_id = destination_id.to_string();
            rust.backup_id = backup_id.to_string();
            rust.filter.clear();
        }
        self.as_mut().begin_reset_search();
        self.as_mut().rust_mut().rows.clear();
        self.as_mut().end_reset_search();
        self.as_mut().set_search_text(QString::from(&text));
        self.as_mut().set_too_large(false);
        self.as_mut().set_total(0);
        self.as_mut().set_matched(0);
        self.as_mut().set_count_text(QString::from(""));
        self.as_mut().set_error_text(QString::from(""));
        let no_hits = i18n("No hits for ‹%1›").subs(&[&text]);
        self.as_mut().set_no_hits_text(QString::from(&no_hits));
        if text.is_empty() {
            self.as_mut().set_state(QString::from(""));
            return;
        }
        let scope = FindScope::Backup {
            backup_id: backup_id.to_string(),
        };
        let request = SnapshotQueryRequest::find(&destination_id.to_string(), scope, &text);
        self.as_mut().rust_mut().query.start(request);
        self.as_mut().set_state(QString::from("running"));
    }

    /// Checks the running search (called by a timer while `state` is `running`).
    pub fn poll(mut self: Pin<&mut Self>) {
        match self.as_mut().rust_mut().query.poll_start() {
            StartPoll::Waiting => {}
            StartPoll::Failed(text) => self.as_mut().fail(&text),
            StartPoll::Started => self.as_mut().load_first_page(),
        }
    }

    /// Filters the hits by `text` (any case, in the paths); the daemon filters.
    pub fn set_filter(mut self: Pin<&mut Self>, text: &QString) {
        let text = text.to_string();
        if text == self.filter {
            return;
        }
        self.as_mut().rust_mut().filter = text;
        if self.state.to_string() == "done" {
            self.as_mut().load_first_page();
        }
    }

    /// Stops the running search (the user cancels it or the page closes).
    pub fn cancel(self: Pin<&mut Self>) {
        self.abandon();
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
        match self.as_mut().rust_mut().fetch(0) {
            Ok(None) => {}
            Ok(Some(found)) => self.as_mut().apply_first_page(&found),
            Err(KesticDbusError::NotFound(_)) => self.as_mut().search_again(),
            Err(err) => self.as_mut().fail(&fetch_error_text(err)),
        }
    }

    /// Starts the search again (the daemon freed the old result).
    fn search_again(mut self: Pin<&mut Self>) {
        let dest = QString::from(&self.destination_id);
        let backup = QString::from(&self.backup_id);
        let text = self.search_text.clone();
        let filter = self.filter.clone();
        self.as_mut().search(&dest, &backup, &text);
        self.as_mut().rust_mut().filter = filter;
    }

    fn apply_first_page(mut self: Pin<&mut Self>, found: &FindResultDto) {
        let hits = &found.hits;
        let rows: Vec<SearchRow> = hits.items.iter().map(row_for).collect();
        self.as_mut().begin_reset_search();
        self.as_mut().rust_mut().rows = rows;
        self.as_mut().end_reset_search();
        self.as_mut().set_too_large(hits.too_large);
        self.as_mut().set_total(to_i32(hits.total));
        self.as_mut().set_matched(to_i32(hits.matched));
        // A list too large to hold cannot be filtered: its count is the total.
        let count = if !self.filter.is_empty() && !hits.too_large {
            count_text(hits.total, hits.matched, true)
        } else {
            hits_text(
                hits.total,
                found.snapshots_matched,
                &self.search_text.to_string(),
            )
        };
        self.as_mut().set_count_text(QString::from(&count));
        self.as_mut()
            .set_too_large_text(QString::from(&too_large_text(hits.total)));
        self.as_mut().set_error_text(QString::from(""));
        self.as_mut().set_state(QString::from("done"));
    }

    pub fn can_fetch_more(&self, parent: &QModelIndex) -> bool {
        !parent.is_valid()
            && self.state.to_string() == "done"
            && !self.too_large
            && i32::try_from(self.rows.len()).unwrap_or(i32::MAX) < self.matched
    }

    /// Appends the next page (at most 200 hits) at the end of the list.
    pub fn fetch_more(mut self: Pin<&mut Self>, parent: &QModelIndex) {
        if !self.can_fetch_more(parent) {
            return;
        }
        let offset = self.rows.len() as u64;
        match self.as_mut().rust_mut().fetch(offset) {
            Ok(Some(found)) if !found.hits.items.is_empty() => {
                let rows: Vec<SearchRow> = found.hits.items.iter().map(row_for).collect();
                let first = row_count(&self.rows);
                let last = first + row_count(&rows) - 1;
                self.as_mut().begin_insert_rows_search(parent, first, last);
                self.as_mut().rust_mut().rows.extend(rows);
                self.as_mut().end_insert_rows_search();
            }
            Ok(_) => {}
            Err(KesticDbusError::NotFound(_)) => self.as_mut().search_again(),
            Err(err) => self.as_mut().fail(&fetch_error_text(err)),
        }
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        row_count(&self.rows)
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        role_names::<SearchRow>()
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        row_data(&self.rows, index, role)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(entry_type: &str, size: Option<u64>) -> FindHitDto {
        FindHitDto {
            snapshot_id: "a".repeat(64),
            snapshot_time: "2026-11-14T12:00:00Z".to_string(),
            path: "/home/u/Archiv/bericht.pdf".to_string(),
            entry_type: entry_type.to_string(),
            size,
            backup_id: Some("b1".to_string()),
        }
    }

    #[test]
    fn hit_rows_name_snapshot_folder_and_size_of_files() {
        let file = row_for(&hit("file", Some(48_000)));
        assert_eq!(file.folder, "/home/u/Archiv");
        assert_eq!(file.subtitle, format!("{} · 48 KB", file.snapshot_text));
        assert!(!file.is_dir);
        let empty = row_for(&hit("file", None));
        assert_eq!(empty.subtitle, format!("{} · 0 B", empty.snapshot_text));
        let dir = row_for(&hit("dir", None));
        assert_eq!(dir.subtitle, dir.snapshot_text);
        assert!(dir.is_dir);
        let mut odd = hit("file", None);
        odd.snapshot_time = "not a time".to_string();
        assert_eq!(snapshot_text(&odd), "not a time");
    }

    #[test]
    fn hits_text_names_hits_text_and_snapshots() {
        assert_eq!(
            hits_text(2, 2, "bericht.pdf"),
            "2 hits for ‹bericht.pdf› in 2 snapshots"
        );
        assert_eq!(hits_text(1, 1, "x"), "1 hit for ‹x› in 1 snapshot");
    }
}
