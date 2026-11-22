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

/// AC-M4-01-1 (c): every entry carries its time; the list groups by day as «Today»,
/// «Yesterday», «This Week» (with weekday and date) and «Earlier» (with the date).
#[test]
fn ac_m4_01_1_snapshots_are_grouped_by_day_with_their_time() {
    use time::macros::datetime;
    // Times in the local zone of the test machine, so the days never shift.
    let offset = crate::time_texts::to_local(datetime!(2026-11-15 12:00:00 UTC)).offset();
    let local = |t: time::PrimitiveDateTime| t.assume_offset(offset);
    // Sunday, 15.11.2026: the week started on Monday, 09.11.
    let now = local(datetime!(2026-11-15 14:32:00));
    let today = day_section(local(datetime!(2026-11-15 12:00:00)), now);
    let yesterday = day_section(local(datetime!(2026-11-14 12:00:00)), now);
    let monday = day_section(local(datetime!(2026-11-09 12:00:00)), now);
    let earlier = day_section(local(datetime!(2026-11-08 12:00:00)), now);
    assert_eq!(today.1, "Today", "(c) today's group");
    assert_eq!(today.0, "12:00", "(c) time of today's snapshot");
    assert_eq!(yesterday.1, "Yesterday", "(c) yesterday's group");
    assert_eq!(monday.1, "This Week", "(c) this week's group");
    assert_eq!(
        monday.0, "Mon 2026-11-09 12:00",
        "(c) weekday and date in this week"
    );
    assert_eq!(earlier.1, "Earlier", "(c) older snapshots");
    assert_eq!(
        earlier.0, "2026-11-08 12:00",
        "(c) date of an older snapshot"
    );
}
