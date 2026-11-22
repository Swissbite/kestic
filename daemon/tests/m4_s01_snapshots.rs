// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S01 snapshot list per backup against real restic (0.18.0 and latest) with temp repos:
//! `ListBackupSnapshots` and `ListDestinationSnapshots` over D-Bus.

mod common;

use common::m3::{M3Rig, ResticVariant, restic_variants};
use common::m4::{RawSnapshot, write_snapshot};
use common::run_isolated_test;
use kestic_core::model::{Backup, Destination, DestinationKind};
use kestic_daemon::dbus_api::{BackupSnapshotsDto, DestinationSnapshotsDto};
use std::path::Path;
use time::macros::datetime;

/// A daemon with one local destination and the backups `b1` and `b2` on it.
struct Fixture {
    rig: M3Rig,
    dest: Destination,
    password: String,
    b1: Backup,
    b2: Backup,
}

impl Fixture {
    fn start(root: &Path, variant: &ResticVariant) -> Self {
        let rig = M3Rig::start(
            root,
            variant,
            "M4S01",
            datetime!(2026-11-15 14:32:00 UTC),
            |o| o.enable_scheduler = false,
        );
        let repo_dir = root.join(rig.sub.clone()).join("repo");
        let (dest, password) = common::m3::add_local_destination(
            &rig.daemon.client,
            &variant.bin,
            &repo_dir,
            "NAS",
            |_| {},
        );
        let b1 = rig.add_backup_to(&dest, "Home", |_| {});
        let b2 = rig.add_backup_to(&dest, "Photos", |_| {});
        Self {
            rig,
            dest,
            password,
            b1,
            b2,
        }
    }

    fn repo(&self) -> &Path {
        let DestinationKind::Local { path } = &self.dest.kind else {
            panic!("local destination expected");
        };
        path
    }

    /// Writes a snapshot directly with restic.
    fn snapshot(&self, tag: &str, time: &str, host: &str) {
        let work = self.rig.root.join(&self.rig.sub);
        write_snapshot(
            &self.rig.variant.bin,
            self.repo(),
            &self.password,
            &work,
            &RawSnapshot { tag, time, host },
        );
    }

    fn b1_tag(&self) -> String {
        format!("kestic:{}", self.b1.id)
    }

    /// Three snapshots of `b1`, written out of time order.
    fn three_b1_snapshots(&self) {
        let tag = self.b1_tag();
        self.snapshot(&tag, "2026-11-14 12:00:00", "laptop");
        self.snapshot(&tag, "2026-11-13 12:00:00", "desktop");
        self.snapshot(&tag, "2026-11-15 12:00:00", "laptop");
    }

    fn list_b1(&self) -> BackupSnapshotsDto {
        self.rig
            .daemon
            .client
            .list_backup_snapshots(self.b1.id.as_str())
            .expect("ListBackupSnapshots")
    }

    fn list_destination(&self) -> DestinationSnapshotsDto {
        self.rig
            .daemon
            .client
            .list_destination_snapshots(self.dest.id.as_str())
            .expect("ListDestinationSnapshots")
    }
}

fn for_each_variant(root: &Path, body: impl Fn(&Fixture, &str)) {
    for variant in restic_variants() {
        let fixture = Fixture::start(root, &variant);
        body(&fixture, variant.label);
        fixture.rig.daemon.stop();
    }
}

/// AC-M4-01-1: three snapshots of `b1` are listed newest first, each with time, size (new data)
/// and host.
#[test]
fn ac_m4_01_1_lists_three_snapshots_newest_first_with_time_size_and_host() {
    run_isolated_test(
        "ac_m4_01_1_lists_three_snapshots_newest_first_with_time_size_and_host",
        "KESTIC_AC_M4_01_1_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                fx.three_b1_snapshots();
                let list = fx.list_b1();
                let snaps = &list.current_snapshots;
                assert_eq!(snaps.len(), 3, "[{label}] (a) exactly the three snapshots");
                assert!(
                    snaps.iter().all(|s| s.tags.contains(&fx.b1_tag())),
                    "[{label}] (a) only snapshots of b1: {snaps:?}"
                );
                let days: Vec<&str> = snaps.iter().map(|s| &s.time[..10]).collect();
                assert_eq!(
                    days,
                    ["2026-11-15", "2026-11-14", "2026-11-13"],
                    "[{label}] (b) newest first"
                );
                let hosts: Vec<&str> = snaps.iter().map(|s| s.hostname.as_str()).collect();
                assert_eq!(
                    hosts,
                    ["laptop", "laptop", "desktop"],
                    "[{label}] (c) host per entry"
                );
                for s in snaps {
                    assert!(
                        s.data_added.is_some_and(|b| b > 0),
                        "[{label}] (c)(d) size is the new data (data_added): {s:?}"
                    );
                }
                assert!(
                    list.current_error.is_none(),
                    "[{label}] no error: {:?}",
                    list.current_error
                );
            });
        },
    );
}

/// AC-M4-01-2: metadata snapshots `kestic-meta:b1` are neither listed nor counted.
#[test]
fn ac_m4_01_2_metadata_snapshots_are_hidden_and_not_counted() {
    run_isolated_test(
        "ac_m4_01_2_metadata_snapshots_are_hidden_and_not_counted",
        "KESTIC_AC_M4_01_2_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                fx.three_b1_snapshots();
                let meta_tag = format!("kestic-meta:{}", fx.b1.id);
                fx.snapshot(&meta_tag, "2026-11-15 12:05:00", "laptop");
                fx.snapshot(&meta_tag, "2026-11-15 12:06:00", "laptop");
                // restic takes comma-separated tags: a metadata snapshot with the backup tag too.
                let both = format!("{},{meta_tag}", fx.b1_tag());
                fx.snapshot(&both, "2026-11-15 12:07:00", "laptop");

                let list = fx.list_b1();
                assert!(
                    list.current_snapshots
                        .iter()
                        .all(|s| !s.tags.contains(&meta_tag)),
                    "[{label}] (a) no metadata snapshot in the list"
                );
                let dest = fx.list_destination();
                assert!(dest.error_kind.is_none(), "[{label}] {dest:?}");
                let count = dest
                    .backups
                    .iter()
                    .find(|b| b.backup_id == fx.b1.id.as_str())
                    .map(|b| b.count);
                assert_eq!(count, Some(3), "[{label}] (b) count without metadata");
                assert!(
                    dest.unassigned
                        .iter()
                        .chain(dest.orphaned.iter().flat_map(|o| o.snapshots.iter()))
                        .all(|s| !s.tags.contains(&meta_tag)),
                    "[{label}] (a) metadata snapshots nowhere in the destination list"
                );
            });
        },
    );
}

/// AC-M4-01-3: the list of `b1` holds no snapshot of `b2` in the same repository.
#[test]
fn ac_m4_01_3_list_of_b1_has_no_snapshot_of_b2() {
    run_isolated_test(
        "ac_m4_01_3_list_of_b1_has_no_snapshot_of_b2",
        "KESTIC_AC_M4_01_3_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                fx.three_b1_snapshots();
                let b2_tag = format!("kestic:{}", fx.b2.id);
                fx.snapshot(&b2_tag, "2026-11-15 13:00:00", "laptop");
                fx.snapshot(&b2_tag, "2026-11-12 13:00:00", "laptop");

                let list = fx.list_b1();
                assert!(
                    list.current_snapshots
                        .iter()
                        .all(|s| !s.tags.contains(&b2_tag)),
                    "[{label}] (a) no snapshot of b2 in the list of b1: {:?}",
                    list.current_snapshots
                );
                assert_eq!(list.current_snapshots.len(), 3, "[{label}] (a)");
            });
        },
    );
}
