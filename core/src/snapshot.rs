// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Tolerant restic snapshot JSON parser (NFR-10, FR-06, P17).

use serde::{Deserialize, Deserializer};

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
}

/// Tolerant representation of a restic snapshot JSON entry.
/// Unknown fields are ignored (NFR-10) and `summary` is optional (P17).
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
    pub program_version: Option<String>,
    pub original: Option<String>,
    pub summary: Option<SnapshotSummary>,
}

/// Parses a `restic snapshots --json` payload tolerantly.
pub fn parse_snapshots_json(input: &str) -> Result<Vec<ResticSnapshot>, serde_json::Error> {
    serde_json::from_str(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-M0-02-5 (P17): Pre-0.17 snapshot JSON without `summary` fixture parses cleanly.
    #[test]
    fn ac_m0_02_5_parses_snapshot_fixture_without_summary() {
        let raw = include_str!("../tests/fixtures/snapshot_pre_0_17_no_summary.json");
        let snapshots = parse_snapshots_json(raw).expect("fixture JSON should parse");
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
