// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of the snapshot texts in the restore area.

use super::*;
use kestic_core::snapshot::ResticSnapshot;

fn fixture_dtos(raw: &str) -> Vec<SnapshotSummaryDto> {
    let snaps: Vec<ResticSnapshot> = serde_json::from_str(raw).expect("fixture JSON parses");
    snaps.into_iter().map(SnapshotSummaryDto::from).collect()
}

/// AC-M4-01-5 (a): a snapshot without summary (restic 0.16 fixture) shows the size as unknown.
#[test]
fn ac_m4_01_5_snapshot_without_summary_shows_size_unknown() {
    let dtos = fixture_dtos(include_str!(
        "../../../core/tests/fixtures/snapshot_pre_0_17_no_summary.json"
    ));
    assert_eq!(dtos.len(), 1);
    assert_eq!(
        snapshot_size_text(&dtos[0]),
        "Size unknown",
        "(a) size unknown"
    );
    assert_eq!(
        snapshot_subtitle(&dtos[0]),
        "Size unknown · tuxedo",
        "(a) host stays next to the unknown size"
    );
}

/// AC-M4-01-1 (d): the size is the newly written data («312 MB new»); with a summary but no
/// `data_added` it is `total_bytes_processed`.
#[test]
fn ac_m4_01_1_size_is_data_added_or_total_bytes_processed() {
    let dtos = fixture_dtos(include_str!(
        "../../../core/tests/fixtures/m4/snapshots_summary_with_and_without_data_added.json"
    ));
    assert_eq!(
        snapshot_size_text(&dtos[1]),
        "312.0 MB new",
        "(d) data_added as new data"
    );
    assert_eq!(
        snapshot_size_text(&dtos[0]),
        format_bytes(2_147_483_648),
        "(d) total_bytes_processed without data_added"
    );
    assert_eq!(
        snapshot_subtitle(&dtos[1]),
        "312.0 MB new · tuxedo",
        "(c) size and host"
    );
}
