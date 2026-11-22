// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Tolerant restic snapshot JSON parser.

use serde::{Deserialize, Deserializer};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Tag prefix of the snapshots of a backup (`kestic:<backup-id>`).
pub const BACKUP_TAG_PREFIX: &str = "kestic:";
/// Tag prefix of metadata snapshots (`kestic-meta:<backup-id>`).
pub const META_TAG_PREFIX: &str = "kestic-meta:";

pub(crate) fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Optional snapshot summary introduced in restic 0.17+.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SnapshotSummary {
    pub total_bytes_processed: Option<u64>,
    pub total_files_processed: Option<u64>,
    /// Data newly written to the repository by this snapshot.
    #[serde(default)]
    pub data_added: Option<u64>,
}

/// Tolerant representation of a restic snapshot JSON entry.
/// Unknown fields are ignored and `summary` is optional.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ResticSnapshot {
    pub id: String,
    pub short_id: Option<String>,
    pub time: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub paths: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub tags: Vec<String>,
    pub hostname: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub uid: Option<u32>,
    pub program_version: Option<String>,
    pub original: Option<String>,
    pub summary: Option<SnapshotSummary>,
}

/// Size shown for a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotSize {
    /// Data newly written to the repository (`data_added`).
    NewData(u64),
    /// Data read for the snapshot (`total_bytes_processed`), when the summary lacks
    /// `data_added`.
    Processed(u64),
    /// No summary (older restic versions, foreign snapshots); never computed with `stats`.
    Unknown,
}

impl SnapshotSize {
    /// The size from the summary fields `data_added` and `total_bytes_processed`.
    pub fn from_summary(data_added: Option<u64>, total_bytes_processed: Option<u64>) -> Self {
        match (data_added, total_bytes_processed) {
            (Some(bytes), _) => Self::NewData(bytes),
            (None, Some(bytes)) => Self::Processed(bytes),
            (None, None) => Self::Unknown,
        }
    }
}

impl ResticSnapshot {
    /// Size shown for the snapshot.
    pub fn size(&self) -> SnapshotSize {
        let summary = self.summary.as_ref();
        SnapshotSize::from_summary(
            summary.and_then(|s| s.data_added),
            summary.and_then(|s| s.total_bytes_processed),
        )
    }

    /// Backup IDs named by `kestic:<id>` tags, in tag order.
    pub fn backup_ids(&self) -> impl Iterator<Item = &str> {
        self.tags
            .iter()
            .filter_map(|t| t.strip_prefix(BACKUP_TAG_PREFIX))
    }

    /// Backup IDs named by `kestic-meta:<id>` tags.
    pub fn meta_backup_ids(&self) -> impl Iterator<Item = &str> {
        self.tags
            .iter()
            .filter_map(|t| t.strip_prefix(META_TAG_PREFIX))
    }

    /// The configured backup this snapshot belongs to: the only configured ID among its
    /// `kestic:<id>` tags. A metadata snapshot, and one tagged for several configured backups,
    /// belongs to none, so it never shows in the list of a backup whose tag it does not carry
    /// alone.
    pub fn assigned_backup(&self, configured: &[&str]) -> Option<&str> {
        if self.is_meta() {
            return None;
        }
        let mut ids = self.backup_ids().filter(|id| configured.contains(id));
        let first = ids.next()?;
        ids.all(|id| id == first).then_some(first)
    }

    /// Whether the snapshot carries the tags of more than one configured backup.
    pub fn has_several_backups(&self, configured: &[&str]) -> bool {
        let mut ids = self.backup_ids().filter(|id| configured.contains(id));
        ids.next().is_some_and(|first| ids.any(|id| id != first))
    }

    /// Whether this is a metadata snapshot (`kestic-meta:<id>`).
    pub fn is_meta(&self) -> bool {
        self.meta_backup_ids().next().is_some()
    }

    fn parsed_time(&self) -> Option<OffsetDateTime> {
        OffsetDateTime::parse(&self.time, &Rfc3339).ok()
    }
}

/// Sorts snapshots newest first. restic lists them oldest first, so snapshots whose time does
/// not parse keep that order reversed and go to the end.
pub fn sort_newest_first(snapshots: &mut [ResticSnapshot]) {
    snapshots.reverse();
    snapshots.sort_by(|a, b| match (a.parsed_time(), b.parsed_time()) {
        (Some(ta), Some(tb)) => tb.cmp(&ta),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pre-0.17 snapshot JSON without `summary` fixture parses cleanly.
    #[test]
    fn ac_m0_02_5_parses_snapshot_fixture_without_summary() {
        let raw = include_str!("../tests/fixtures/snapshot_pre_0_17_no_summary.json");
        let snapshots: Vec<ResticSnapshot> =
            serde_json::from_str(raw).expect("fixture JSON should parse");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].short_id.as_deref(), Some("c3d4e5f6"));
        assert_eq!(
            snapshots[0].program_version.as_deref(),
            Some("restic 0.16.4")
        );
        assert!(
            snapshots[0].summary.is_none(),
            "pre-0.17 fixture must not contain a summary field"
        );
    }
}
