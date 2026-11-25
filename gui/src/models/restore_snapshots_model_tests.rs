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

/// Two orphaned backups of one destination with the same number of snapshots, newest time and
/// host get different group headings, so the list never merges them under one heading.
#[test]
fn m4_s07_two_orphaned_backups_get_their_own_group() {
    use kestic_daemon::dbus_api::OrphanedBackupDto;
    let snap = |id: &str| {
        let mut dtos = fixture_dtos(include_str!(
            "../../../core/tests/fixtures/snapshot_pre_0_17_no_summary.json"
        ));
        dtos[0].id = id.to_string();
        dtos.remove(0)
    };
    let group = |backup_id: &str, snap_id: &str| OrphanedBackupDto {
        backup_id: backup_id.to_string(),
        count: 1,
        latest_time: Some("2026-10-02T18:00:00Z".to_string()),
        latest_hostname: Some("notebook-alt".to_string()),
        snapshots: vec![snap(snap_id)],
    };
    let dto = DestinationSnapshotsDto {
        destination_id: "d1".to_string(),
        error_kind: None,
        error_detail: None,
        backups: Vec::new(),
        unassigned: Vec::new(),
        orphaned: vec![group("x9", "s1"), group("y8", "s2")],
        described: Vec::new(),
    };
    let now = OffsetDateTime::parse("2026-11-15T14:32:00Z", &Rfc3339).expect("time");
    let mut rows = Vec::new();
    push_group_rows(&mut rows, SnapshotGroup::Orphaned, &dto, "NAS", now);
    assert_eq!(rows.len(), 2, "one row per snapshot");
    assert_ne!(
        rows[0].section_name, rows[1].section_name,
        "each orphaned backup has its own group"
    );
    assert!(
        rows.iter().all(|r| r.hint == "Not managed by Kestic"),
        "orphaned snapshots carry the hint"
    );
}

/// Two destinations with the same name get headings of their own: the shared name carries the
/// location, a unique name stays as it is.
#[test]
fn m4_s07_destinations_with_the_same_name_get_their_own_heading() {
    use kestic_core::model::DestinationKind;
    let local = |name: &str, path: &str| {
        Destination::new(
            name,
            DestinationKind::Local {
                path: std::path::PathBuf::from(path),
            },
        )
        .expect("destination")
    };
    let labels = destination_labels(&[
        local("NAS", "/mnt/a"),
        local("NAS", "/mnt/b"),
        local("USB", "/media/usb"),
    ]);
    assert_eq!(labels[0].1, "NAS · /mnt/a", "first of the shared name");
    assert_eq!(labels[1].1, "NAS · /mnt/b", "second of the shared name");
    assert_eq!(labels[2].1, "USB", "a unique name stays");
    let dto = |dest: &str| DestinationSnapshotsDto {
        destination_id: dest.to_string(),
        error_kind: None,
        error_detail: None,
        backups: Vec::new(),
        unassigned: fixture_dtos(include_str!(
            "../../../core/tests/fixtures/snapshot_pre_0_17_no_summary.json"
        )),
        orphaned: Vec::new(),
        described: Vec::new(),
    };
    let now = OffsetDateTime::parse("2026-11-15T14:32:00Z", &Rfc3339).expect("time");
    let mut rows = Vec::new();
    push_group_rows(
        &mut rows,
        SnapshotGroup::Unassigned,
        &dto(&labels[0].0),
        &labels[0].1,
        now,
    );
    push_group_rows(
        &mut rows,
        SnapshotGroup::Unassigned,
        &dto(&labels[1].0),
        &labels[1].1,
        now,
    );
    assert_ne!(
        rows.first().map(|r| &r.section_name),
        rows.last().map(|r| &r.section_name),
        "each destination has its own heading"
    );
}
