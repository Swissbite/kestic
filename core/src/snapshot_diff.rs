// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Comparing two snapshots with `restic diff --json <from> <to>`: the changed paths with the
//! kind of their change, and the counts of the statistics line.

use crate::backup::{BackupRunError, SnapshotListAccess, check_command_succeeded, run_streaming};
use crate::destination::{DEFAULT_CONNECT_TIMEOUT, preflight_check_destination};
use crate::query_list::{HeldResult, ListBuilder, ListEntry, ListPage, PageRequest, QueryList};
use crate::restic::CancelHandle;
use kestic_i18n::codes::SnapshotChange;
use serde::{Deserialize, Serialize};

/// One changed path between two snapshots (`DiffChangeDto`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffChange {
    /// Absolute path as restic reports it; a folder ends with `/`.
    pub path: String,
    pub change: SnapshotChange,
}

impl ListEntry for DiffChange {
    fn path(&self) -> &str {
        &self.path
    }

    fn kind_code(&self) -> Option<&str> {
        Some(self.change.as_str())
    }
}

/// The counts of the statistics line of `restic diff --json`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffCounts {
    pub added_files: u64,
    pub removed_files: u64,
    pub changed_files: u64,
    pub bytes_added: u64,
    pub bytes_removed: u64,
}

/// The changes from one snapshot to another, sorted by path.
pub struct DiffResult {
    changes: QueryList<DiffChange>,
    /// Counts of the whole comparison, independent of any filter.
    pub counts: DiffCounts,
}

impl DiffResult {
    /// The page `request` asks for; the `kinds` filter compares [`SnapshotChange`] codes.
    pub fn page(&mut self, request: &PageRequest) -> ListPage<DiffChange> {
        self.changes.page(request)
    }

    /// All changes, held or not.
    pub fn total(&self) -> u64 {
        self.changes.total()
    }

    /// Whether restic reported no change at all («no differences»): no change line and no
    /// added, removed or changed file in the statistics.
    pub fn identical(&self) -> bool {
        self.changes.total() == 0
            && self.counts.added_files == 0
            && self.counts.removed_files == 0
            && self.counts.changed_files == 0
    }
}

impl HeldResult for DiffResult {
    fn held_bytes(&self) -> u64 {
        self.changes.held_bytes()
    }
}

/// The kind of change of a `restic diff` modifier; `None` for a modifier without a letter
/// Kestic knows. restic combines letters for one path (`M?` is content changed with the same
/// metadata, `TU` a type change with metadata); the most telling letter decides: `?` before
/// `T` before `M`. `U` alone (only the metadata changed) appears only with `--metadata`, which
/// Kestic does not pass, and is ignored.
fn change_of_modifier(modifier: &str) -> Option<SnapshotChange> {
    match modifier {
        "+" => return Some(SnapshotChange::Added),
        "-" => return Some(SnapshotChange::Removed),
        _ => {}
    }
    [
        ('?', SnapshotChange::Bitrot),
        ('T', SnapshotChange::TypeChanged),
        ('M', SnapshotChange::Modified),
    ]
    .into_iter()
    .find(|(letter, _)| modifier.contains(*letter))
    .map(|(_, change)| change)
}

/// Arguments of `restic diff` from snapshot `from` to snapshot `to` (full IDs; `--json` comes
/// from the command spec).
pub fn diff_args(from: &str, to: &str) -> Vec<String> {
    vec![
        "diff".to_string(),
        "--no-lock".to_string(),
        from.to_string(),
        to.to_string(),
    ]
}

#[derive(Deserialize, Default)]
struct RawStats {
    #[serde(default)]
    files: u64,
    #[serde(default)]
    bytes: u64,
}

#[derive(Deserialize)]
struct RawLine {
    #[serde(default)]
    message_type: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    modifier: String,
    #[serde(default)]
    changed_files: u64,
    #[serde(default)]
    added: Option<RawStats>,
    #[serde(default)]
    removed: Option<RawStats>,
}

/// Turns the output lines of `restic diff --json` into the changes and the counts. Unknown
/// lines, message types, modifiers and fields are ignored.
pub struct DiffCollector {
    builder: ListBuilder<DiffChange>,
    counts: DiffCounts,
}

impl DiffCollector {
    /// Collects the changes; the list is dropped once it exceeds `limit_bytes`.
    pub fn new(limit_bytes: u64) -> Self {
        Self {
            builder: ListBuilder::new(limit_bytes),
            counts: DiffCounts::default(),
        }
    }

    pub fn push_line(&mut self, line: &str) {
        let Ok(raw) = serde_json::from_str::<RawLine>(line.trim()) else {
            return;
        };
        match raw.message_type.as_str() {
            "change" => {
                if let Some(change) = change_of_modifier(&raw.modifier)
                    && !raw.path.is_empty()
                {
                    self.builder.push(DiffChange {
                        path: raw.path,
                        change,
                    });
                }
            }
            "statistics" => {
                let added = raw.added.unwrap_or_default();
                let removed = raw.removed.unwrap_or_default();
                self.counts = DiffCounts {
                    added_files: added.files,
                    removed_files: removed.files,
                    changed_files: raw.changed_files,
                    bytes_added: added.bytes,
                    bytes_removed: removed.bytes,
                };
            }
            _ => {}
        }
    }

    pub fn finish(self) -> DiffResult {
        DiffResult {
            changes: self.builder.finish(),
            counts: self.counts,
        }
    }
}

/// Compares snapshot `from` with snapshot `to` (`restic diff --no-lock`), after the
/// reachability check of the destination. Both IDs must be full.
pub fn execute_diff(
    access: &SnapshotListAccess<'_>,
    from: &str,
    to: &str,
    cancel: Option<&CancelHandle>,
    limit_bytes: u64,
) -> Result<DiffResult, BackupRunError> {
    preflight_check_destination(access.destination, DEFAULT_CONNECT_TIMEOUT)?;
    let mut collector = DiffCollector::new(limit_bytes);
    let res = run_streaming(access, diff_args(from, to), cancel, &mut |line| {
        collector.push_line(line)
    })?;
    check_command_succeeded(&res)?;
    Ok(collector.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_list::PageFilter;

    #[test]
    fn modifiers_map_to_change_kinds() {
        assert_eq!(change_of_modifier("+"), Some(SnapshotChange::Added));
        assert_eq!(change_of_modifier("-"), Some(SnapshotChange::Removed));
        assert_eq!(change_of_modifier("M"), Some(SnapshotChange::Modified));
        assert_eq!(change_of_modifier("U"), None);
        assert_eq!(change_of_modifier("T"), Some(SnapshotChange::TypeChanged));
        assert_eq!(change_of_modifier("?"), Some(SnapshotChange::Bitrot));
        assert_eq!(change_of_modifier("M?"), Some(SnapshotChange::Bitrot));
        assert_eq!(change_of_modifier("TU"), Some(SnapshotChange::TypeChanged));
        assert_eq!(change_of_modifier("MU"), Some(SnapshotChange::Modified));
        assert_eq!(change_of_modifier(""), None);
        assert_eq!(change_of_modifier("X"), None);
    }

    #[test]
    fn diff_args_compare_the_two_snapshots_without_lock() {
        assert_eq!(diff_args("aaa", "bbb"), ["diff", "--no-lock", "aaa", "bbb"]);
    }

    const STATS: &str = r#"{"message_type":"statistics","source_snapshot":"a","target_snapshot":"b","changed_files":1,"added":{"files":1,"dirs":1,"others":0,"data_blobs":1,"tree_blobs":11,"bytes":4770},"removed":{"files":1,"dirs":0,"others":0,"data_blobs":2,"tree_blobs":10,"bytes":4358}}"#;

    /// AC-M4-02-2: `diff` JSON with an unknown field, an unknown message type and an unknown
    /// modifier (a) gives the changes without error, and (b) ignores what it does not know.
    #[test]
    fn ac_m4_02_2_unknown_fields_and_message_types_are_ignored() {
        let mut c = DiffCollector::new(u64::MAX);
        c.push_line(
            r#"{"message_type":"change","path":"/s/docs/c.txt","modifier":"-","future":{"x":1}}"#,
        );
        c.push_line(r#"{"message_type":"change","path":"/s/docs/a.txt","modifier":"M"}"#);
        c.push_line(r#"{"message_type":"progress","percent_done":0.5}"#);
        c.push_line(r#"{"message_type":"change","path":"/s/docs/z.txt","modifier":"Z"}"#);
        c.push_line(r#"{"message_type":"change","path":"/s/new/","modifier":"+","size":3}"#);
        c.push_line("not json at all");
        c.push_line(STATS);
        let mut result = c.finish();
        let page = result.page(&PageRequest::default());
        let changes: Vec<(&str, SnapshotChange)> = page
            .items
            .iter()
            .map(|c| (c.path.as_str(), c.change))
            .collect();
        assert_eq!(
            changes,
            [
                ("/s/docs/a.txt", SnapshotChange::Modified),
                ("/s/docs/c.txt", SnapshotChange::Removed),
                ("/s/new/", SnapshotChange::Added),
            ],
            "(a) the known changes, sorted by path"
        );
        assert_eq!(
            page.total, 3,
            "(b) the unknown modifier and lines are left out"
        );
        assert_eq!(
            result.counts,
            DiffCounts {
                added_files: 1,
                removed_files: 1,
                changed_files: 1,
                bytes_added: 4770,
                bytes_removed: 4358,
            },
            "(a) the counts of the statistics line, its unknown fields ignored"
        );
        assert!(!result.identical());
    }

    /// restic reports a file whose content changed while its metadata stayed the same as
    /// `M?`; it is listed, so the snapshots are not identical.
    #[test]
    fn bitrot_is_listed_and_not_identical() {
        let mut c = DiffCollector::new(u64::MAX);
        c.push_line(r#"{"message_type":"change","path":"/f","modifier":"M?"}"#);
        c.push_line(
            r#"{"message_type":"statistics","changed_files":1,"added":{"files":0,"bytes":0},"removed":{"files":0,"bytes":0}}"#,
        );
        let mut result = c.finish();
        let page = result.page(&PageRequest::default());
        assert_eq!(
            page.items,
            [DiffChange {
                path: "/f".to_string(),
                change: SnapshotChange::Bitrot,
            }]
        );
        assert!(!result.identical());
    }

    /// A change restic counts but Kestic cannot list never reads as «no differences».
    #[test]
    fn counted_changes_without_lines_are_not_identical() {
        let mut c = DiffCollector::new(u64::MAX);
        c.push_line(r#"{"message_type":"change","path":"/f","modifier":"Z"}"#);
        c.push_line(
            r#"{"message_type":"statistics","changed_files":1,"added":{"files":0,"bytes":0},"removed":{"files":0,"bytes":0}}"#,
        );
        let result = c.finish();
        assert_eq!(result.total(), 0);
        assert!(!result.identical());
    }

    #[test]
    fn only_statistics_means_identical() {
        let mut c = DiffCollector::new(u64::MAX);
        c.push_line(
            r#"{"message_type":"statistics","changed_files":0,"added":{"files":0,"bytes":0},"removed":{"files":0,"bytes":0}}"#,
        );
        let result = c.finish();
        assert!(result.identical());
        assert_eq!(result.counts, DiffCounts::default());
    }

    #[test]
    fn kinds_filter_picks_the_changes_of_one_kind() {
        let mut c = DiffCollector::new(u64::MAX);
        for (path, m) in [("/a", "+"), ("/b", "-"), ("/c", "+"), ("/d", "M")] {
            c.push_line(&format!(
                r#"{{"message_type":"change","path":"{path}","modifier":"{m}"}}"#
            ));
        }
        let mut result = c.finish();
        let page = result.page(&PageRequest {
            filter: PageFilter {
                text: String::new(),
                kinds: vec!["added".to_string()],
            },
            ..PageRequest::default()
        });
        let paths: Vec<&str> = page.items.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["/a", "/c"]);
        assert_eq!(page.matched, 2);
        assert_eq!(page.total, 4);
    }
}
