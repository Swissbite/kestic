// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M5-S05 local destination folders: the CIFS/SMB note from mountinfo (`AC-M5-05-2`) and
//! network addresses of the file dialog (`AC-M5-05-3`).

use kestic_core::local_path::{NotALocalPath, connection_notes, local_path_from_dialog_url};
use kestic_core::model::{Destination, DestinationKind, ModelError};
use kestic_core::mount::{FakeMountInspector, FakeMountState};
use kestic_i18n::codes::ConnectionNoteCode;
use kestic_i18n::codes::ConnectionTestCode;
use kestic_i18n::messages::UiText;
use std::path::PathBuf;

const MOUNTINFO: &str = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
110 22 0:45 / /mnt/nas rw,relatime - cifs //nas/backups rw,vers=3.1.1
111 22 0:46 / /mnt/nas3 rw,relatime - smb3 //nas/other rw
120 22 0:50 / /run/user/1000/kio-fuse-AbCdEf rw,nosuid - fuse.kio-fuse kio-fuse rw
121 22 0:51 / /run/user/1000/gvfs rw,nosuid - fuse.gvfsd-fuse gvfsd-fuse rw
";

fn local(path: &str) -> Destination {
    Destination::new(
        "Destination",
        DestinationKind::Local {
            path: PathBuf::from(path),
        },
    )
    .expect("local destination")
}

/// AC-M5-05-2 (a), unit part (mountinfo fake): a folder below a `cifs` or `smb3` mount, or on an
/// SMB share shown by the FUSE mount of KIO or GVfs, gets the note `cifs_share`; a folder on the ext4 root and a REST destination do not. The D-Bus and
/// GUI tests of this AC show that adding stays possible after the confirmation (b).
#[test]
fn ac_m5_05_2_folder_on_cifs_mount_gets_cifs_note() {
    let mounts = FakeMountInspector::new(FakeMountState {
        mountinfo: MOUNTINFO.to_string(),
        ..FakeMountState::default()
    });

    assert_eq!(
        connection_notes(&local("/mnt/nas/kestic"), &mounts),
        vec![ConnectionNoteCode::CifsShare],
        "(a) cifs mount"
    );
    assert_eq!(
        connection_notes(&local("/mnt/nas3"), &mounts),
        vec![ConnectionNoteCode::CifsShare],
        "(a) smb3 mount"
    );
    assert_eq!(
        connection_notes(
            &local("/run/user/1000/kio-fuse-AbCdEf/smb/nas/share/kestic"),
            &mounts
        ),
        vec![ConnectionNoteCode::CifsShare],
        "(a) SMB share through the FUSE mount of KIO"
    );
    assert_eq!(
        connection_notes(
            &local("/run/user/1000/gvfs/smb-share:server=nas,share=backups/kestic"),
            &mounts
        ),
        vec![ConnectionNoteCode::CifsShare],
        "(a) SMB share through the FUSE mount of GVfs"
    );
    assert_eq!(
        connection_notes(
            &local("/run/user/1000/kio-fuse-AbCdEf/sftp/nas/kestic"),
            &mounts
        ),
        Vec::new(),
        "an SFTP folder through KIO is no SMB share"
    );
    assert_eq!(
        connection_notes(&local("/mnt/nasa/kestic"), &mounts),
        Vec::new(),
        "a sibling path of the share is on ext4"
    );
    let rest = Destination::new(
        "NAS",
        DestinationKind::Rest {
            url: "https://nas:8000/".to_string(),
            username: None,
        },
    )
    .expect("rest destination");
    assert_eq!(connection_notes(&rest, &mounts), Vec::new(), "REST");
}

/// AC-M5-05-3 (§9.1): the folder dialog returns `smb://nas/backups` instead of a local path;
/// (a) Kestic rejects it with a hint: the dialog result is no local folder, a destination with
/// it does not validate, and the hint names the address (without the password in it).
#[test]
fn ac_m5_05_3_kio_address_is_rejected_with_hint() {
    // (a) the dialog result
    assert_eq!(
        local_path_from_dialog_url("smb://david:geheim@nas/backups"),
        Err(NotALocalPath("smb://nas/backups".to_string())),
        "(a) smb:// is not a local folder"
    );
    assert_eq!(
        local_path_from_dialog_url("file:///media/user/USB-Disk/Kestic%20Backup"),
        Ok(PathBuf::from("/media/user/USB-Disk/Kestic Backup")),
        "a file:// URL becomes its path"
    );

    // (a) the destination itself
    let err = Destination::new(
        "NAS",
        DestinationKind::Local {
            path: PathBuf::from("sftp://nas/backups"),
        },
    )
    .expect_err("(a) sftp:// as a local path");
    assert!(
        matches!(&err, ModelError::NotALocalPath(NotALocalPath(a)) if a == "sftp://nas/backups"),
        "(a) rejected as not a local path"
    );

    // (a) the hint
    let UiText::Message(hint) = ConnectionTestCode::NotALocalPath.text("smb://nas/backups") else {
        panic!("(a) the hint is a translated message");
    };
    assert_eq!(
        hint.msgid,
        "'%1' is a network address, not a folder on this computer. restic cannot use it; choose a local or mounted folder.",
        "(a) hint text"
    );
    assert_eq!(
        hint.args,
        vec!["smb://nas/backups".to_string()],
        "(a) address"
    );
}
