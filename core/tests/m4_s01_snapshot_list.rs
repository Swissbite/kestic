// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M4-S01 snapshot lists from fixtures: snapshots without summary and the sorting of a
//! destination's snapshots.

use kestic_core::backup::{
    SnapshotListAccess, list_destination_snapshots, list_snapshots_for_backup,
};
use kestic_core::clock::FakeClock;
use kestic_core::model::{BackupId, Destination, DestinationKind};
use kestic_core::provider::ResolvedRestic;
use kestic_core::restic::SECRET_KIND_REPO;
use kestic_core::secrets::{InMemorySecretStore, SecretStore, SecretValue};
use kestic_core::snapshot::{ResticSnapshot, SnapshotSize};
use kestic_core::snapshot_list::sort_destination_snapshots;
use kestic_test_support::tools::write_executable;
use semver::Version;
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use time::macros::datetime;

/// A restic stand-in that logs its arguments to `calls.log` and prints `fixture` on one line,
/// like restic, for `snapshots`.
fn spy_restic(dir: &Path, fixture: &Path) -> ResolvedRestic {
    let bin = dir.join("spy-restic");
    let log = dir.join("calls.log");
    write_executable(
        &bin,
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in *snapshots*) tr -d '\\n' < '{}'; echo ;; esac\nexit 0\n",
            log.display(),
            fixture.display()
        ),
    );
    ResolvedRestic {
        binary: bin,
        version: Version::new(0, 18, 0),
    }
}

fn logged_calls(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// AC-M4-01-5: a snapshot JSON without summary (restic 0.16, fixture) lists with (a) unknown
/// size, (b) no `stats` call and (c) no error.
#[test]
fn ac_m4_01_5_snapshot_without_summary_lists_without_stats_and_without_error() {
    let tmp = TempDir::new().expect("tempdir");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/snapshot_pre_0_17_no_summary.json");
    let resolved = spy_restic(tmp.path(), &fixture);
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).expect("mkdir repo");
    let dest = Destination::new("NAS", DestinationKind::Local { path: repo }).expect("dest");
    let store = InMemorySecretStore::default();
    let clock = FakeClock::new(datetime!(2026-11-15 14:32:00 UTC));
    let kestic_bin = tmp.path().join("kestic");
    let backup_id = BackupId::generate().expect("backup id");

    let result = list_snapshots_for_backup(
        &backup_id,
        &dest,
        &resolved,
        &kestic_bin,
        &store,
        &clock,
        &[],
    );
    let snaps = result.expect("(c) listing a snapshot without summary is no error");
    assert_eq!(snaps.len(), 1, "(c) the snapshot is listed");
    assert_eq!(snaps[0].size(), SnapshotSize::Unknown, "(a) size unknown");

    let access = SnapshotListAccess {
        destination: &dest,
        resolved: &resolved,
        kestic_binary: &kestic_bin,
        secret_store: &store,
        clock: &clock,
        extra_env: &[],
    };
    let all = list_destination_snapshots(&access).expect("(c) no error for the destination");
    assert_eq!(all.len(), 1, "(c) destination list has the snapshot");

    let calls = logged_calls(tmp.path());
    assert!(
        calls
            .iter()
            .all(|c| !c.split_whitespace().any(|w| w == "stats")),
        "(b) no stats call: {calls:?}"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.split_whitespace().any(|w| w == "snapshots"))
            .count(),
        2,
        "(b) one snapshots call per list: {calls:?}"
    );
}

fn snap(id: &str, time: &str, tags: &[&str]) -> ResticSnapshot {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "time": time,
        "tags": tags,
        "paths": ["/home/user"],
        "hostname": "laptop",
    }))
    .expect("snapshot JSON")
}

/// AC-M4-07-7: the snapshots of a destination are sorted as metadata snapshot (left out),
/// backup, orphaned, unassigned; own tags do not change that.
#[test]
fn ac_m4_07_7_destination_snapshots_are_sorted_meta_backup_orphaned_unassigned() {
    let snapshots = vec![
        snap("meta", "2026-11-15T12:05:00Z", &["kestic-meta:b1"]),
        snap(
            "meta-tagged",
            "2026-11-15T12:06:00Z",
            &["kestic:b1", "kestic-meta:b1"],
        ),
        snap("b1-old", "2026-11-14T12:00:00Z", &["kestic:b1"]),
        snap("b1-new", "2026-11-15T12:00:00Z", &["kestic:b1"]),
        snap("x9", "2026-11-10T12:00:00Z", &["kestic:x9"]),
        snap("foreign", "2026-11-09T12:00:00Z", &[]),
        snap("own-tags", "2026-11-08T12:00:00Z", &["daily"]),
    ];
    let sorted = sort_destination_snapshots(snapshots, &["b1"]);
    let ids = |list: &[ResticSnapshot]| list.iter().map(|s| s.id.clone()).collect::<Vec<_>>();

    let all_ids: Vec<String> = sorted
        .backups
        .iter()
        .chain(&sorted.orphaned)
        .chain(&sorted.described)
        .flat_map(|(_, s)| ids(s))
        .chain(ids(&sorted.unassigned))
        .collect();
    assert!(
        !all_ids.contains(&"meta".to_string()),
        "(a) metadata snapshot left out"
    );
    assert!(
        !all_ids.contains(&"meta-tagged".to_string()),
        "(a) a metadata snapshot with the backup tag too is left out"
    );
    assert_eq!(sorted.backups.len(), 1);
    assert_eq!(sorted.backups[0].0, "b1");
    assert_eq!(
        ids(&sorted.backups[0].1),
        ["b1-new", "b1-old"],
        "(b) snapshots of b1, newest first"
    );
    assert_eq!(sorted.orphaned.len(), 1, "(c) one orphaned backup");
    assert_eq!(sorted.orphaned[0].0, "x9", "(c) x9 is orphaned");
    assert_eq!(ids(&sorted.orphaned[0].1), ["x9"], "(c)");
    assert!(
        ids(&sorted.unassigned).contains(&"foreign".to_string()),
        "(d) without Kestic tag unassigned"
    );
    assert!(
        ids(&sorted.unassigned).contains(&"own-tags".to_string()),
        "(e) own tags stay unassigned"
    );
}

/// A tag `kestic:` or `kestic-meta:` without an ID names no backup: such a snapshot is shown as
/// unassigned, never hidden as a metadata snapshot or listed as an orphaned backup without ID.
#[test]
fn m4_s07_kestic_tags_without_id_leave_the_snapshot_unassigned() {
    let snapshots = vec![
        snap("empty-meta", "2026-11-15T12:00:00Z", &["kestic-meta:"]),
        snap("empty-backup", "2026-11-14T12:00:00Z", &["kestic:"]),
    ];
    let sorted = sort_destination_snapshots(snapshots, &["b1"]);
    let unassigned: Vec<&str> = sorted.unassigned.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        unassigned,
        ["empty-meta", "empty-backup"],
        "both shown as unassigned"
    );
    assert!(sorted.orphaned.is_empty(), "no orphaned backup without ID");
    assert!(
        sorted.described.is_empty(),
        "no described backup without ID"
    );
}

/// A snapshot tagged for two backups, neither configured here, has no unique Kestic tag: it is
/// unassigned, not sorted under the backup of its first tag; one tag repeated stays unique.
#[test]
fn m4_s07_two_unconfigured_backup_tags_leave_the_snapshot_unassigned() {
    let snapshots = vec![
        snap("two", "2026-11-15T12:00:00Z", &["kestic:x9", "kestic:y8"]),
        snap("twice", "2026-11-14T12:00:00Z", &["kestic:x9", "kestic:x9"]),
    ];
    let sorted = sort_destination_snapshots(snapshots, &["b1"]);
    let unassigned: Vec<&str> = sorted.unassigned.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(unassigned, ["two"], "two backup tags: unassigned");
    let orphaned: Vec<(&str, Vec<&str>)> = sorted
        .orphaned
        .iter()
        .map(|(id, s)| (id.as_str(), s.iter().map(|s| s.id.as_str()).collect()))
        .collect();
    assert_eq!(
        orphaned,
        [("x9", vec!["twice"])],
        "one tag repeated: orphaned under it"
    );
}

/// AC-M4-01-1: a repository password that also occurs inside the snapshot JSON (here the
/// backup ID in every `kestic:<id>` tag, generated at runtime) does not change the listed
/// snapshots: (a) both snapshots of the backup are listed, still under their backup.
#[test]
fn ac_m4_01_1_password_inside_the_snapshot_json_does_not_change_the_list() {
    let tmp = TempDir::new().expect("tempdir");
    let backup_id = BackupId::generate().expect("backup id");
    let tag = format!("kestic:{backup_id}");
    let snapshots = serde_json::json!([
        { "id": "a1", "time": "2026-11-14T12:00:00Z", "tags": [tag], "hostname": "laptop" },
        { "id": "a2", "time": "2026-11-15T12:00:00Z", "tags": [tag], "hostname": "laptop" },
    ]);
    let fixture = tmp.path().join("snapshots.json");
    fs::write(&fixture, snapshots.to_string()).expect("write fixture");
    let resolved = spy_restic(tmp.path(), &fixture);
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).expect("mkdir repo");
    let dest = Destination::new("NAS", DestinationKind::Local { path: repo }).expect("dest");
    let store = InMemorySecretStore::default();
    store
        .set_secret(
            dest.id.as_str(),
            SECRET_KIND_REPO,
            &SecretValue::from_str_secret(backup_id.as_str()),
        )
        .expect("store password");
    let clock = FakeClock::new(datetime!(2026-11-15 14:32:00 UTC));
    let kestic_bin = tmp.path().join("kestic");
    let access = SnapshotListAccess {
        destination: &dest,
        resolved: &resolved,
        kestic_binary: &kestic_bin,
        secret_store: &store,
        clock: &clock,
        extra_env: &[],
    };

    let all = list_destination_snapshots(&access).expect("listing succeeds");
    let sorted = sort_destination_snapshots(all, &[backup_id.as_str()]);
    assert_eq!(
        sorted.backups[0].1.len(),
        2,
        "(a) both snapshots stay with their backup"
    );
    assert!(sorted.unassigned.is_empty(), "(a) none becomes unassigned");
    assert!(sorted.orphaned.is_empty(), "(a) none becomes orphaned");
}
