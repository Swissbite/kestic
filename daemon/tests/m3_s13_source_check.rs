// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S13 scheduled source check for drive sources (mountinfo fake + UDisks2 fake)
//! (`AC-M3-13-1` … `AC-M3-13-4`). Folder sources (`AC-M3-13-5`) are unit-tested in `core`.

mod common;

use common::fakes::FakeUDisks2Handle;
use common::m3::{
    LocalBackupFixture, M3Rig, ResticVariant, StdFakes, backup_summary, latest_variant,
    notifications_for, restic_variants, settle, wait_for_notifications, wait_for_state,
};
use common::run_isolated_test;
use kestic_core::model::{RunStatus, Source, SourceIdentity};
use kestic_core::mount::{FakeMountInspector, FakeMountState, MountInspector};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use time::macros::datetime;

const BEFORE_SLOT: OffsetDateTime = datetime!(2026-12-01 10:59:30 UTC);
const SLOT_1: OffsetDateTime = datetime!(2026-12-01 11:00:05 UTC);
const SLOT_2: OffsetDateTime = datetime!(2026-12-02 11:00:05 UTC);

/// The drive-source backup of [`start_rig`] with its mountinfo fake and mount point.
struct UsbDrive {
    mounts: Arc<FakeMountInspector>,
    mount_dir: PathBuf,
    fixture: LocalBackupFixture,
}

/// Daemon with a drive-source backup (UUID `1111-2222`) whose drive is not mounted.
fn start_rig(root: &Path, variant: &ResticVariant, base: &str) -> (M3Rig, UsbDrive) {
    let mounts = Arc::new(FakeMountInspector::new(FakeMountState::default()));
    let rig = M3Rig::start(root, variant, base, BEFORE_SLOT, |opts| {
        opts.mount_inspector = Some(Arc::clone(&mounts) as Arc<dyn MountInspector>);
    });
    let mount_dir = root.join(&rig.sub).join("mnt").join("usb");
    let source_path = mount_dir.join("photos");
    let fixture = rig.add_backup("UsbBk", |b| {
        b.source = Source::drive_with_identity(
            source_path.clone(),
            Some(SourceIdentity::FsUuid {
                fs_uuid: "1111-2222".to_string(),
            }),
            Some("USB".to_string()),
        );
    });
    (
        rig,
        UsbDrive {
            mounts,
            mount_dir,
            fixture,
        },
    )
}

impl UsbDrive {
    fn wait_pending_source(&self, rig: &M3Rig) {
        wait_for_state(
            &rig.daemon.client,
            Duration::from_secs(10),
            "source_unavailable",
            |s| {
                backup_summary(s, &self.fixture.backup.id)
                    .pending_primary_code
                    .map(|c| c.as_str())
                    == Some("source_unavailable")
            },
        );
    }

    fn executed_runs(&self, rig: &M3Rig) -> usize {
        rig.daemon
            .client
            .get_state()
            .expect("GetState")
            .recent_runs
            .iter()
            .filter(|r| r.backup_id() == Some(&self.fixture.backup.id))
            .filter(|r| r.status != RunStatus::Pending)
            .count()
    }

    /// Makes the drive appear in the mountinfo fake and announces it via UDisks2.
    fn mount_drive(&self, udisks: &FakeUDisks2Handle, uuid: &str, device: &str) {
        let mount_point = self.mount_dir.to_string_lossy().into_owned();
        fs::create_dir_all(self.mount_dir.join("photos")).expect("mkdir photos");
        fs::write(self.mount_dir.join("photos").join("pic.jpg"), b"photo").expect("write photo");
        self.mounts.update(|st| {
            st.mountinfo =
                format!("42 1 8:33 / {mount_point} rw,relatime shared:1 - ext4 /dev/{device} rw\n");
            st.device_uuids.insert(
                format!("/dev/{device}"),
                (uuid.to_string(), Some("USB".to_string())),
            );
        });
        udisks.emit_mount_added(device, uuid, &mount_point);
    }
}

/// AC-M3-13-1 (FR-29): drive source not mounted at 12:00 -> (a) no restic process,
/// (b) `ausstehend` (source unavailable), (c) no notification.
#[test]
fn ac_m3_13_1_unmounted_drive_keeps_backup_pending_without_restic_or_notification() {
    run_isolated_test(
        "ac_m3_13_1_unmounted_drive_keeps_backup_pending_without_restic_or_notification",
        "KESTIC_M3_13_1_INNER",
        |root| {
            let _udisks = FakeUDisks2Handle::start();
            let std_fakes = StdFakes::start();
            let (rig, usb) = start_rig(root, &latest_variant(), "M3D1");
            let repo_before = rig.wrapper.started_repo_count();

            rig.clock.set(SLOT_1);
            usb.wait_pending_source(&rig);
            settle();
            assert_eq!(
                rig.wrapper.started_repo_count(),
                repo_before,
                "(a) no restic process: {:?}",
                rig.wrapper.entries()
            );
            let state = rig.daemon.client.get_state().expect("GetState");
            assert_eq!(
                backup_summary(&state, &usb.fixture.backup.id)
                    .pending_primary_code
                    .map(|c| c.as_str()),
                Some("source_unavailable"),
                "(b)"
            );
            assert_eq!(
                std_fakes.notif.snapshot().len(),
                0,
                "(c) no notification: {:?}",
                std_fakes.notif.snapshot()
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M3-13-2 (FR-29): when UDisks2 reports the mount with the matching UUID, the run starts
/// without waiting for the next slot.
#[test]
fn ac_m3_13_2_matching_uuid_mount_starts_pending_backup_immediately() {
    run_isolated_test(
        "ac_m3_13_2_matching_uuid_mount_starts_pending_backup_immediately",
        "KESTIC_M3_13_2_INNER",
        |root| {
            let udisks = FakeUDisks2Handle::start();
            let _std_fakes = StdFakes::start();
            for variant in restic_variants() {
                let (rig, usb) = start_rig(root, &variant, "M3D2");
                rig.clock.set(SLOT_1);
                usb.wait_pending_source(&rig);

                usb.mount_drive(&udisks, "1111-2222", "sdc1");
                let done = wait_for_state(
                    &rig.daemon.client,
                    Duration::from_secs(30),
                    "run after mount",
                    |s| {
                        backup_summary(s, &usb.fixture.backup.id)
                            .last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                    },
                );
                assert_eq!(
                    backup_summary(&done, &usb.fixture.backup.id)
                        .last_run
                        .as_ref()
                        .map(|r| r.started_at),
                    Some(SLOT_1),
                    "{}: started in the same slot, without waiting for the next one",
                    variant.label
                );
                assert_eq!(rig.wrapper.of('S', "backup").len(), 1, "{}", variant.label);
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-13-3 (FR-29, negative): a different drive (other UUID) being mounted leaves the backup
/// `ausstehend`.
#[test]
fn ac_m3_13_3_mount_of_other_drive_leaves_backup_pending() {
    run_isolated_test(
        "ac_m3_13_3_mount_of_other_drive_leaves_backup_pending",
        "KESTIC_M3_13_3_INNER",
        |root| {
            let udisks = FakeUDisks2Handle::start();
            let _std_fakes = StdFakes::start();
            let (rig, usb) = start_rig(root, &latest_variant(), "M3D3");
            rig.clock.set(SLOT_1);
            usb.wait_pending_source(&rig);

            usb.mount_drive(&udisks, "9999-8888", "sdb1");
            settle();
            let state = rig.daemon.client.get_state().expect("GetState");
            assert_eq!(
                backup_summary(&state, &usb.fixture.backup.id)
                    .pending_primary_code
                    .map(|c| c.as_str()),
                Some("source_unavailable"),
                "still pending after mounting a different drive"
            );
            assert_eq!(usb.executed_runs(&rig), 0, "no run for a different drive");
            assert_eq!(
                rig.wrapper.of('S', "backup").len(),
                0,
                "no restic backup process"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M3-13-4 (FR-29, glossary "überfällig"): source missing over two slots -> (b) no
/// notification before, (a) exactly one "overdue" notification with the second missed slot.
#[test]
fn ac_m3_13_4_source_missing_over_two_slots_notifies_overdue_exactly_once() {
    run_isolated_test(
        "ac_m3_13_4_source_missing_over_two_slots_notifies_overdue_exactly_once",
        "KESTIC_M3_13_4_INNER",
        |root| {
            let _udisks = FakeUDisks2Handle::start();
            let std_fakes = StdFakes::start();
            let (rig, usb) = start_rig(root, &latest_variant(), "M3D4");

            rig.clock.set(SLOT_1);
            usb.wait_pending_source(&rig);
            settle();
            let first = rig.daemon.client.get_state().expect("GetState");
            assert!(
                !backup_summary(&first, &usb.fixture.backup.id).is_overdue,
                "(b) one missed slot is not overdue"
            );
            assert_eq!(
                notifications_for(&std_fakes.notif, "BackupOverdue").len(),
                0,
                "(b) no notification before the second missed slot"
            );

            rig.clock.set(SLOT_2);
            wait_for_state(
                &rig.daemon.client,
                Duration::from_secs(10),
                "overdue",
                |s| backup_summary(s, &usb.fixture.backup.id).is_overdue,
            );
            wait_for_notifications(&std_fakes.notif, "BackupOverdue", 1, Duration::from_secs(5));
            settle();
            assert_eq!(
                notifications_for(&std_fakes.notif, "BackupOverdue").len(),
                1,
                "(a) exactly one overdue notification: {:?}",
                std_fakes.notif.snapshot()
            );
            rig.daemon.stop();
        },
    );
}
