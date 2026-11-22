// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M5-S05 destination «External Drive / USB»: removable drives from the UDisks2 fake
//! (`AC-M5-05-1`) and the CIFS/SMB note of the connection test with the mountinfo fake
//! (`AC-M5-05-2`), both over D-Bus against an in-process daemon on an isolated session bus.

mod common;

use common::fakes::{FakeUDisks2Filesystem, FakeUDisks2Handle};
use common::m3::{M3Rig, latest_variant};
use common::run_isolated_test;
use kestic_core::model::{Destination, DestinationKind};
use kestic_core::mount::{FakeMountInspector, FakeMountState, MountInspector};
use kestic_daemon::dbus_api::RemovableDriveDto;
use kestic_i18n::codes::ConnectionNoteCode;
use std::fs;
use std::sync::Arc;
use time::macros::datetime;

fn filesystem(
    device: &str,
    label: &str,
    mount_points: &[&str],
    removable: bool,
) -> FakeUDisks2Filesystem {
    FakeUDisks2Filesystem {
        device_name: device.to_string(),
        label: label.to_string(),
        uuid: format!("uuid-{device}"),
        mount_points: mount_points.iter().map(|m| (*m).to_string()).collect(),
        drive_name: format!("drive_{device}"),
        vendor: "Vendor".to_string(),
        model: "Model".to_string(),
        removable,
    }
}

/// AC-M5-05-1 (§9.1): the UDisks2 fake reports a mounted removable drive (next to an internal
/// disk and an unmounted stick); `ListRemovableDrives`, which the type «External Drive / USB»
/// shows, (a) offers it with name and mount point, and only it.
#[test]
fn ac_m5_05_1_mounted_removable_drive_is_offered_with_name_and_mount_point() {
    run_isolated_test(
        "ac_m5_05_1_mounted_removable_drive_is_offered_with_name_and_mount_point",
        "KESTIC_M5_05_1_INNER",
        |root| {
            let udisks = FakeUDisks2Handle::start();
            udisks.add_filesystem(filesystem(
                "sdb1",
                "USB-Disk",
                &["/media/user/USB-Disk"],
                true,
            ));
            udisks.add_filesystem(filesystem("nvme0n1p2", "System", &["/"], false));
            udisks.add_filesystem(filesystem("sdc1", "Stick", &[], true));
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
                vec![RemovableDriveDto {
                    name: "USB-Disk".to_string(),
                    mount_point: "/media/user/USB-Disk".to_string(),
                }],
                "(a) the mounted removable drive with name and mount point"
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
                .test_destination_connection(&dest, "init", "", "")
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
                .add_destination(&dest, "init", "correct horse battery", "")
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
