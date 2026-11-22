// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M5-S05 destination «External Drive / USB»: removable drives from the UDisks2 fake
//! (`AC-M5-05-1`), the CIFS/SMB note of the connection test with the mountinfo fake
//! (`AC-M5-05-2`) and network addresses as paths (`AC-M5-05-3`), both over D-Bus against an in-process daemon on an isolated session bus.

mod common;

use common::fakes::{FakeUDisks2Filesystem, FakeUDisks2Handle};
use common::m3::{M3Rig, latest_variant};
use common::run_isolated_test;
use kestic_core::model::{Destination, DestinationKind};
use kestic_core::mount::{FakeMountInspector, FakeMountState, MountInspector};
use kestic_daemon::dbus_api::BackendSecrets;
use kestic_daemon::dbus_api::RemovableDriveDto;
use kestic_i18n::codes::{ConnectionNoteCode, ConnectionTestCode};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use time::macros::datetime;

fn filesystem(
    device: &str,
    label: &str,
    mount_points: &[&str],
    removable: bool,
    connection_bus: &str,
) -> FakeUDisks2Filesystem {
    FakeUDisks2Filesystem {
        device_name: device.to_string(),
        label: label.to_string(),
        uuid: format!("uuid-{device}"),
        mount_points: mount_points.iter().map(|m| (*m).to_string()).collect(),
        drive_name: format!("drive_{device}"),
        size_bytes: 64_000_000_000,
        removable,
        connection_bus: connection_bus.to_string(),
    }
}

fn drive(label: &str, mount_point: &str, device: &str) -> RemovableDriveDto {
    RemovableDriveDto {
        label: label.to_string(),
        mount_point: mount_point.to_string(),
        fs_uuid: format!("uuid-{device}"),
        size_bytes: 64_000_000_000,
    }
}

/// AC-M5-05-1 (§9.1): the UDisks2 fake reports a mounted USB stick and a mounted USB hard disk
/// (which UDisks2 does not mark `Removable`, only its bus is `usb`), next to an internal disk,
/// an internal data partition and an unmounted stick; `ListRemovableDrives`, which the type
/// «External Drive / USB» shows, (a) offers both USB drives with name (label) and mount point,
/// and only them.
#[test]
fn ac_m5_05_1_mounted_removable_drive_is_offered_with_name_and_mount_point() {
    run_isolated_test(
        "ac_m5_05_1_mounted_removable_drive_is_offered_with_name_and_mount_point",
        "KESTIC_M5_05_1_INNER",
        |root| {
            let udisks = FakeUDisks2Handle::start();
            for fs in [
                filesystem("sdb1", "USB-Disk", &["/media/user/USB-Disk"], false, "usb"),
                filesystem("sdc1", "Stick", &["/media/user/Stick"], true, "usb"),
                filesystem("nvme0n1p2", "System", &["/"], false, ""),
                filesystem("sda1", "Data", &["/data"], false, ""),
                filesystem("sdd1", "Unplugged", &[], true, "usb"),
            ] {
                udisks.add_filesystem(fs);
            }
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M505D",
                datetime!(2026-12-01 10:00:00 UTC),
                |_| {},
            );

            let drives = rig
                .daemon
                .client
                .list_removable_drives()
                .expect("ListRemovableDrives");
            // (a)
            assert_eq!(
                drives,
                vec![
                    drive("Stick", "/media/user/Stick", "sdc1"),
                    drive("USB-Disk", "/media/user/USB-Disk", "sdb1"),
                ],
                "(a) the mounted USB drives with name and mount point"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M5-05-3 (D-Bus part of the unit test in `core`): `TestDestinationConnection` with a
/// network address as the path of a local destination answers `not_a_local_path` with the
/// address (without the password) as detail, instead of a plain input error.
#[test]
fn m5_05_3_connection_test_reports_network_address() {
    run_isolated_test(
        "m5_05_3_connection_test_reports_network_address",
        "KESTIC_M5_05_3_INNER",
        |root| {
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M505K",
                datetime!(2026-12-01 10:00:00 UTC),
                |_| {},
            );
            let mut dest = Destination::new(
                "NAS",
                DestinationKind::Local {
                    path: root.to_path_buf(),
                },
            )
            .expect("destination");
            dest.kind = DestinationKind::Local {
                path: PathBuf::from("smb://david:secret@nas/backups"),
            };
            let tested = rig
                .daemon
                .client
                .test_destination_connection(&dest, "init", "", &BackendSecrets::default())
                .expect("TestDestinationConnection");
            assert!(!tested.ok, "network address accepted: {tested:?}");
            assert_eq!(
                tested.error_kind,
                Some(ConnectionTestCode::NotALocalPath),
                "code"
            );
            assert_eq!(
                tested.detail, "smb://nas/backups",
                "address without password"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M5-05-2 (§9.1, D-Bus part): a folder on a CIFS mount (mountinfo fake); (a) the
/// connection test succeeds with the note `cifs_share`, which the GUI shows as a warning, and
/// (b) `AddDestination` still creates the destination. The confirmation in the wizard is
/// checked by the GUI smoke test of this AC.
#[test]
fn ac_m5_05_2_cifs_folder_gets_note_and_can_be_added() {
    run_isolated_test(
        "ac_m5_05_2_cifs_folder_gets_note_and_can_be_added",
        "KESTIC_M5_05_2_INNER",
        |root| {
            let share = root.join("mnt").join("nas");
            let repo = share.join("backups");
            fs::create_dir_all(&repo).expect("mkdir repo");
            let mounts = Arc::new(FakeMountInspector::new(FakeMountState {
                mountinfo: format!(
                    "110 25 0:45 / {} rw,relatime - cifs //nas/backups rw\n",
                    share.display()
                ),
                ..FakeMountState::default()
            }));
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M505C",
                datetime!(2026-12-01 10:00:00 UTC),
                |opts| opts.mount_inspector = Some(Arc::clone(&mounts) as Arc<dyn MountInspector>),
            );
            let client = &rig.daemon.client;
            let dest = Destination::new("NAS share", DestinationKind::Local { path: repo })
                .expect("destination");

            let tested = client
                .test_destination_connection(&dest, "init", "", &BackendSecrets::default())
                .expect("TestDestinationConnection");
            assert!(tested.ok, "connection test failed: {tested:?}");
            // (a)
            assert_eq!(
                tested.notes,
                vec![ConnectionNoteCode::CifsShare],
                "(a) note for the CIFS share"
            );

            // (b)
            let id = client
                .add_destination(
                    &dest,
                    "init",
                    "correct horse battery",
                    &BackendSecrets::default(),
                )
                .expect("(b) AddDestination despite the note");
            let state = client.get_state().expect("GetState");
            assert!(
                state
                    .destinations
                    .iter()
                    .any(|d| d.destination.id.as_str() == id),
                "(b) destination saved: {:?}",
                state.destinations
            );
            rig.daemon.stop();
        },
    );
}
