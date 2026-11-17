// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

mod common;

use kestic_core::backup::{
    BackupRunError, check_source_available_with, execute_backup_with_inspector,
    list_snapshots_for_backup,
};
use kestic_core::clock::FakeClock;
use kestic_core::config::{AppConfig, load_config, save_config};
use kestic_core::destination::{DestinationSetupMode, setup_destination};
use kestic_core::model::{
    Backup, Destination, DestinationKind, NestedRelationship, RunStatus, Source, SourceIdentity,
    SourceKind, check_nested_sources_for_candidate, find_nested_source_warnings,
};
use kestic_core::mount::{
    FakeMountInspector, FakeMountState, SourceCheckResult, check_source_status_with,
    detect_source_with,
};
use kestic_core::provider::{ResolvedRestic, check_binary_version};
use kestic_core::restic::ResticRuntime;
use kestic_core::secrets::{FileSecretStore, SecretValue};
use semver::Version;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use time::macros::datetime;

fn write_spy_restic(dir: &Path, log_path: &Path, target_binary: Option<&Path>) -> PathBuf {
    let script_path = dir.join("restic-spy");
    let body = match target_binary {
        Some(real_bin) => format!(
            "#!/bin/sh\necho \"INVOKED:$*\" >> {log:?}\nexec {bin:?} \"$@\"\n",
            log = log_path.to_string_lossy(),
            bin = real_bin.to_string_lossy(),
        ),
        None => format!(
            "#!/bin/sh\necho \"INVOKED:$*\" >> {log:?}\nexit 99\n",
            log = log_path.to_string_lossy(),
        ),
    };
    fs::write(&script_path, body).expect("write spy restic");
    let mut perms = fs::metadata(&script_path).expect("meta").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).expect("chmod spy restic");
    script_path
}

fn base_mountinfo(home_mount: &Path) -> String {
    format!(
        "25 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw\n\
         26 25 259:3 / {} rw,relatime shared:2 - ext4 /dev/nvme0n1p3 rw\n",
        home_mount.display()
    )
}

#[test]
fn ac_m2_09_3_detects_block_drive_and_subfolder_with_fs_uuid() {
    let home = PathBuf::from("/home/david");
    let mut mountinfo = base_mountinfo(&home);
    mountinfo.push_str(
        "101 25 8:17 / /media/david/BackupDrive rw,nosuid,nodev - ext4 /dev/sdb1 rw\n\
         102 26 8:33 / /home/david/Extern rw,nosuid,nodev - btrfs /dev/sdc1 rw\n",
    );

    let mut device_uuids = HashMap::new();
    device_uuids.insert(
        "/dev/sdb1".to_string(),
        (
            "1111-2222-3333-4444".to_string(),
            Some("MyPassport".to_string()),
        ),
    );
    device_uuids.insert(
        "/dev/sdc1".to_string(),
        ("aaaa-bbbb-cccc-dddd".to_string(), None),
    );

    let inspector = FakeMountInspector::new(FakeMountState {
        mountinfo,
        device_uuids,
        empty_dirs: HashMap::new(),
    });

    // Mountpoint itself -> Drive with FS-UUID + label.
    let drive_root = detect_source_with(
        Path::new("/media/david/BackupDrive"),
        Some(&home),
        &inspector,
    );
    assert_eq!(drive_root.kind, SourceKind::Drive);
    assert_eq!(
        drive_root.identity,
        Some(SourceIdentity::FsUuid {
            fs_uuid: "1111-2222-3333-4444".to_string(),
        })
    );
    assert_eq!(drive_root.drive_label.as_deref(), Some("MyPassport"));

    // Subfolder inside the mounted drive -> still detected as Drive with the device's FS-UUID.
    let drive_sub = detect_source_with(
        Path::new("/media/david/BackupDrive/Fotos/2026"),
        Some(&home),
        &inspector,
    );
    assert_eq!(drive_sub.kind, SourceKind::Drive);
    assert_eq!(
        drive_sub.identity,
        Some(SourceIdentity::FsUuid {
            fs_uuid: "1111-2222-3333-4444".to_string(),
        })
    );

    // Nested mountpoint inside $HOME -> Drive, not Folder.
    let nested_drive = detect_source_with(
        Path::new("/home/david/Extern/Archiv"),
        Some(&home),
        &inspector,
    );
    assert_eq!(nested_drive.kind, SourceKind::Drive);
    assert_eq!(
        nested_drive.identity,
        Some(SourceIdentity::FsUuid {
            fs_uuid: "aaaa-bbbb-cccc-dddd".to_string(),
        })
    );

    // Regular folder on $HOME or / -> Folder with no drive identity.
    let home_folder =
        detect_source_with(Path::new("/home/david/Dokumente"), Some(&home), &inspector);
    assert_eq!(home_folder.kind, SourceKind::Folder);
    assert_eq!(home_folder.identity, None);

    let root_folder = detect_source_with(Path::new("/etc/kestic"), Some(&home), &inspector);
    assert_eq!(root_folder.kind, SourceKind::Folder);
    assert_eq!(root_folder.identity, None);
}

#[test]
fn ac_m2_09_7_detects_uuidless_filesystems_using_mount_source_and_fs_type() {
    let home = PathBuf::from("/home/david");
    let mut mountinfo = base_mountinfo(&home);
    mountinfo.push_str(
        "110 25 0:45 / /mnt/nas-fotos rw,relatime - nfs4 nas:/fotos rw,vers=4.2\n\
         111 25 0:46 / /mnt/remote-ssh rw,nosuid,nodev - fuse.sshfs alice@server:/data rw\n\
         112 25 0:47 / /mnt/ramdisk rw,nosuid,nodev - tmpfs tmpfs rw,size=512M\n",
    );

    // No UUID entries for non-block filesystems.
    let inspector = FakeMountInspector::new(FakeMountState {
        mountinfo,
        device_uuids: HashMap::new(),
        empty_dirs: HashMap::new(),
    });

    let nfs = detect_source_with(Path::new("/mnt/nas-fotos"), Some(&home), &inspector);
    assert_eq!(nfs.kind, SourceKind::Drive);
    assert_eq!(
        nfs.identity,
        Some(SourceIdentity::MountSource {
            mount_source: "nas:/fotos".to_string(),
            fs_type: "nfs4".to_string(),
        })
    );
    assert_eq!(nfs.drive_label, None);

    let nfs_sub = detect_source_with(
        Path::new("/mnt/nas-fotos/2026/raw"),
        Some(&home),
        &inspector,
    );
    assert_eq!(nfs_sub.kind, SourceKind::Drive);
    assert_eq!(
        nfs_sub.identity,
        Some(SourceIdentity::MountSource {
            mount_source: "nas:/fotos".to_string(),
            fs_type: "nfs4".to_string(),
        })
    );

    let sshfs = detect_source_with(Path::new("/mnt/remote-ssh"), Some(&home), &inspector);
    assert_eq!(sshfs.kind, SourceKind::Drive);
    assert_eq!(
        sshfs.identity,
        Some(SourceIdentity::MountSource {
            mount_source: "alice@server:/data".to_string(),
            fs_type: "fuse.sshfs".to_string(),
        })
    );

    let ramdisk = detect_source_with(Path::new("/mnt/ramdisk"), Some(&home), &inspector);
    assert_eq!(ramdisk.kind, SourceKind::Drive);
    assert_eq!(
        ramdisk.identity,
        Some(SourceIdentity::MountSource {
            mount_source: "tmpfs".to_string(),
            fs_type: "tmpfs".to_string(),
        })
    );
}

#[test]
fn ac_m2_09_4_drive_preflight_blocks_manual_run_with_zero_restic_invocations() {
    let dir = TempDir::new().expect("tempdir");
    let home_dir = dir.path().join("home");
    let mount_uuid = dir.path().join("mnt-uuid");
    let mount_nfs = dir.path().join("mnt-nfs");
    let repo_dir = dir.path().join("repo");
    let state_dir = dir.path().join("state");
    fs::create_dir_all(&home_dir).expect("mkdir home");
    fs::create_dir_all(&mount_uuid).expect("mkdir mount_uuid");
    fs::create_dir_all(&mount_nfs).expect("mkdir mount_nfs");
    fs::create_dir_all(&repo_dir).expect("mkdir repo");

    // Put a file inside both mountpoints initially so case (a) and (b) test mount/identity, not emptiness.
    fs::write(mount_uuid.join("data.txt"), b"payload").expect("write data");
    fs::write(mount_nfs.join("photo.jpg"), b"jpg").expect("write photo");

    let secrets_hex = dir.path().join("secrets.hex");
    let store = FileSecretStore::new(&secrets_hex);
    let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
    let spy_log = dir.path().join("restic-invocations.log");
    let spy_bin = write_spy_restic(dir.path(), &spy_log, None);
    let resolved = ResolvedRestic {
        binary: spy_bin,
        version: Version::new(0, 18, 0),
    };
    let clock = FakeClock::new(datetime!(2026-12-01 12:00:00 UTC));
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
    };

    let dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: repo_dir.clone(),
        },
    )
    .expect("dest");

    let backup_uuid = Backup::new(
        "USB Drive",
        Source::drive_with_identity(
            &mount_uuid,
            Some(SourceIdentity::FsUuid {
                fs_uuid: "expected-uuid-1111".to_string(),
            }),
            Some("BackupUSB".to_string()),
        ),
        dest.id.clone(),
    )
    .expect("backup uuid");

    let backup_nfs = Backup::new(
        "NAS Photos",
        Source::drive_with_identity(
            &mount_nfs,
            Some(SourceIdentity::MountSource {
                mount_source: "nas:/fotos".to_string(),
                fs_type: "nfs4".to_string(),
            }),
            None,
        ),
        dest.id.clone(),
    )
    .expect("backup nfs");

    // Case (a): Drive not mounted (mountpoint falls back to `/` in mountinfo).
    let unmounted_inspector = FakeMountInspector::new(FakeMountState {
        mountinfo: format!(
            "25 1 259:2 / / rw,relatime - ext4 /dev/nvme0n1p2 rw\n\
             26 25 259:3 / {} rw,relatime - ext4 /dev/nvme0n1p3 rw\n",
            home_dir.display()
        ),
        device_uuids: HashMap::new(),
        empty_dirs: HashMap::new(),
    });
    assert_eq!(
        check_source_status_with(&backup_uuid, Some(&home_dir), &unmounted_inspector),
        SourceCheckResult::DriveNotMounted
    );
    let err_a = check_source_available_with(&backup_uuid, Some(&home_dir), &unmounted_inspector)
        .expect_err("unmounted drive must fail check");
    assert!(
        err_a.to_string().contains("Quelle nicht verfügbar")
            && err_a.to_string().contains("Laufwerk nicht eingehängt"),
        "unexpected error message: {err_a}"
    );
    let exec_err_a = execute_backup_with_inspector(
        &backup_uuid,
        &dest,
        &state_dir,
        &runtime,
        Some(&home_dir),
        &unmounted_inspector,
        None,
        &[],
        |_| {},
    )
    .expect_err("execute must abort when unmounted");
    assert!(matches!(exec_err_a, BackupRunError::SourceUnavailable(_)));
    assert!(
        !spy_log.exists(),
        "restic must not be invoked when drive is unmounted"
    );

    // Case (b1): Different FS-UUID mounted at the same mountpoint.
    let mut wrong_uuid_devices = HashMap::new();
    wrong_uuid_devices.insert(
        "/dev/sdb1".to_string(),
        (
            "different-uuid-9999".to_string(),
            Some("OtherUSB".to_string()),
        ),
    );
    let wrong_uuid_inspector = FakeMountInspector::new(FakeMountState {
        mountinfo: format!(
            "{}101 25 8:17 / {} rw,nosuid,nodev - ext4 /dev/sdb1 rw\n",
            base_mountinfo(&home_dir),
            mount_uuid.display()
        ),
        device_uuids: wrong_uuid_devices,
        empty_dirs: HashMap::new(),
    });
    assert_eq!(
        check_source_status_with(&backup_uuid, Some(&home_dir), &wrong_uuid_inspector),
        SourceCheckResult::DriveMismatch
    );
    let err_b1 = execute_backup_with_inspector(
        &backup_uuid,
        &dest,
        &state_dir,
        &runtime,
        Some(&home_dir),
        &wrong_uuid_inspector,
        None,
        &[],
        |_| {},
    )
    .expect_err("wrong FS-UUID must fail");
    assert!(
        err_b1.to_string().contains("Quelle nicht verfügbar")
            && err_b1.to_string().contains("anderes Laufwerk"),
        "unexpected error: {err_b1}"
    );
    assert!(
        !spy_log.exists(),
        "restic must not be invoked on UUID mismatch"
    );

    // Case (b2): Different mount_source or different fs_type on UUID-less mount.
    let wrong_source_inspector = FakeMountInspector::new(FakeMountState {
        mountinfo: format!(
            "{}110 25 0:45 / {} rw,relatime - nfs4 nas:/musik rw\n",
            base_mountinfo(&home_dir),
            mount_nfs.display()
        ),
        device_uuids: HashMap::new(),
        empty_dirs: HashMap::new(),
    });
    assert_eq!(
        check_source_status_with(&backup_nfs, Some(&home_dir), &wrong_source_inspector),
        SourceCheckResult::DriveMismatch
    );
    let err_b2 = execute_backup_with_inspector(
        &backup_nfs,
        &dest,
        &state_dir,
        &runtime,
        Some(&home_dir),
        &wrong_source_inspector,
        None,
        &[],
        |_| {},
    )
    .expect_err("wrong mount_source must fail");
    assert!(err_b2.to_string().contains("anderes Laufwerk"));
    assert!(!spy_log.exists());

    let wrong_fstype_inspector = FakeMountInspector::new(FakeMountState {
        mountinfo: format!(
            "{}110 25 0:45 / {} rw,relatime - cifs nas:/fotos rw\n",
            base_mountinfo(&home_dir),
            mount_nfs.display()
        ),
        device_uuids: HashMap::new(),
        empty_dirs: HashMap::new(),
    });
    assert_eq!(
        check_source_status_with(&backup_nfs, Some(&home_dir), &wrong_fstype_inspector),
        SourceCheckResult::DriveMismatch
    );
    let err_b3 = execute_backup_with_inspector(
        &backup_nfs,
        &dest,
        &state_dir,
        &runtime,
        Some(&home_dir),
        &wrong_fstype_inspector,
        None,
        &[],
        |_| {},
    )
    .expect_err("wrong fs_type must fail");
    assert!(err_b3.to_string().contains("anderes Laufwerk"));
    assert!(!spy_log.exists());

    // Case (c): Drive mounted with matching FS-UUID, but mountpoint directory is empty.
    fs::remove_file(mount_uuid.join("data.txt")).expect("remove file to make mountpoint empty");
    let mut matching_uuid_devices = HashMap::new();
    matching_uuid_devices.insert(
        "/dev/sdb1".to_string(),
        (
            "expected-uuid-1111".to_string(),
            Some("BackupUSB".to_string()),
        ),
    );
    let empty_drive_inspector = FakeMountInspector::new(FakeMountState {
        mountinfo: format!(
            "{}101 25 8:17 / {} rw,nosuid,nodev - ext4 /dev/sdb1 rw\n",
            base_mountinfo(&home_dir),
            mount_uuid.display()
        ),
        device_uuids: matching_uuid_devices,
        empty_dirs: HashMap::new(),
    });
    assert_eq!(
        check_source_status_with(&backup_uuid, Some(&home_dir), &empty_drive_inspector),
        SourceCheckResult::DriveEmpty
    );
    let err_c = execute_backup_with_inspector(
        &backup_uuid,
        &dest,
        &state_dir,
        &runtime,
        Some(&home_dir),
        &empty_drive_inspector,
        None,
        &[],
        |_| {},
    )
    .expect_err("empty drive mountpoint must fail");
    assert!(
        err_c.to_string().contains("Quelle nicht verfügbar")
            && err_c.to_string().contains("Quelle leer"),
        "unexpected error: {err_c}"
    );
    assert!(
        !spy_log.exists(),
        "restic must not be invoked when drive is empty"
    );
}

#[test]
fn ac_m2_09_5_empty_unmounted_drive_mountpoint_creates_zero_snapshots_and_runs_no_backup() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        let home_dir = dir.path().join("home");
        let empty_mnt = dir.path().join("mnt").join("external");
        fs::create_dir_all(&repo_dir).expect("mkdir repo");
        fs::create_dir_all(&home_dir).expect("mkdir home");
        fs::create_dir_all(&empty_mnt).expect("mkdir empty_mnt");

        let secrets_hex = dir.path().join("secrets.hex");
        let store = FileSecretStore::new(&secrets_hex);
        let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
        let resolved = check_binary_version(&restic_bin).expect("resolved restic");
        let clock = FakeClock::new(datetime!(2026-12-01 12:30:00 UTC));
        let init_runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
        };

        let dest = Destination::new(
            "Local",
            DestinationKind::Local {
                path: repo_dir.clone(),
            },
        )
        .expect("dest");
        let pw = SecretValue::from_str_secret("drive-neg-test-pw");
        setup_destination(
            &dest,
            DestinationSetupMode::InitNew,
            &pw,
            None,
            &init_runtime,
        )
        .unwrap_or_else(|e| panic!("{label}: init repo failed: {e}"));

        let spy_log = dir.path().join("spy-backup.log");
        let spy_bin = write_spy_restic(dir.path(), &spy_log, Some(&resolved.binary));
        let spy_resolved = ResolvedRestic {
            binary: spy_bin,
            version: resolved.version.clone(),
        };
        let spy_runtime = ResticRuntime {
            resolved: &spy_resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
        };

        let backup = Backup::new(
            "External Drive",
            Source::drive_with_identity(
                &empty_mnt,
                Some(SourceIdentity::FsUuid {
                    fs_uuid: "ext-uuid-5555".to_string(),
                }),
                None,
            ),
            dest.id.clone(),
        )
        .expect("backup");

        // Empty, unmounted mountpoint (mountinfo only lists `/` and `$HOME`).
        let inspector = FakeMountInspector::new(FakeMountState {
            mountinfo: format!(
                "25 1 259:2 / / rw,relatime - ext4 /dev/nvme0n1p2 rw\n\
                 26 25 259:3 / {} rw,relatime - ext4 /dev/nvme0n1p3 rw\n",
                home_dir.display()
            ),
            device_uuids: HashMap::new(),
            empty_dirs: HashMap::new(),
        });

        let err = execute_backup_with_inspector(
            &backup,
            &dest,
            &state_dir,
            &spy_runtime,
            Some(&home_dir),
            &inspector,
            None,
            &[],
            |_| {},
        )
        .expect_err("empty unmounted drive must not execute backup");
        assert!(
            matches!(err, BackupRunError::SourceUnavailable(_)),
            "{label}: unexpected error: {err}"
        );
        assert!(
            !spy_log.exists(),
            "{label}: restic backup must not be invoked on unmounted empty mountpoint"
        );

        let snaps =
            list_snapshots_for_backup(&backup.id, &dest, &resolved, &helper, &store, &clock, &[])
                .unwrap_or_else(|e| panic!("{label}: list snapshots failed: {e}"));
        assert!(
            snaps.is_empty(),
            "{label}: expected 0 snapshots, got {}",
            snaps.len()
        );
    }
}

#[test]
fn ac_m2_09_6_empty_folder_on_home_filesystem_runs_backup_normally() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        let home_dir = dir.path().join("home");
        let empty_folder = home_dir.join("EmptyFolder");
        fs::create_dir_all(&repo_dir).expect("mkdir repo");
        fs::create_dir_all(&empty_folder).expect("mkdir empty_folder");

        let inspector = FakeMountInspector::new(FakeMountState {
            mountinfo: format!(
                "25 1 259:2 / / rw,relatime - ext4 /dev/nvme0n1p2 rw\n\
                 26 25 259:3 / {} rw,relatime - ext4 /dev/nvme0n1p3 rw\n",
                home_dir.display()
            ),
            device_uuids: HashMap::new(),
            empty_dirs: HashMap::new(),
        });

        let detected = detect_source_with(&empty_folder, Some(&home_dir), &inspector);
        assert_eq!(detected.kind, SourceKind::Folder, "{label}");
        assert_eq!(detected.identity, None, "{label}");

        let secrets_hex = dir.path().join("secrets.hex");
        let store = FileSecretStore::new(&secrets_hex);
        let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
        let resolved = check_binary_version(&restic_bin).expect("resolved restic");
        let clock = FakeClock::new(datetime!(2026-12-01 13:00:00 UTC));
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
        };

        let dest = Destination::new(
            "Local",
            DestinationKind::Local {
                path: repo_dir.clone(),
            },
        )
        .expect("dest");
        let pw = SecretValue::from_str_secret("empty-folder-pw");
        setup_destination(&dest, DestinationSetupMode::InitNew, &pw, None, &runtime)
            .unwrap_or_else(|e| panic!("{label}: init repo failed: {e}"));

        let backup = Backup::new("Empty Home Folder", detected, dest.id.clone()).expect("backup");
        let res = execute_backup_with_inspector(
            &backup,
            &dest,
            &state_dir,
            &runtime,
            Some(&home_dir),
            &inspector,
            None,
            &[],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: empty folder backup should succeed: {e}"));

        assert_eq!(res.exit_code, 0, "{label}");
        assert_eq!(res.presentation.status, RunStatus::Ok, "{label}");

        let snaps =
            list_snapshots_for_backup(&backup.id, &dest, &resolved, &helper, &store, &clock, &[])
                .unwrap_or_else(|e| panic!("{label}: list snapshots failed: {e}"));
        assert_eq!(
            snaps.len(),
            1,
            "{label}: expected 1 snapshot for empty SourceKind::Folder"
        );
    }
}

#[test]
fn ac_m2_10_1_nested_sources_on_same_destination_warn_and_allow_saving() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = dir.path().join("config.toml");

    let dest_z = Destination::new(
        "Ziel Z",
        DestinationKind::Local {
            path: dir.path().join("repo-z"),
        },
    )
    .expect("dest z");

    let backup_home =
        Backup::new("Home", Source::folder("/home/david"), dest_z.id.clone()).expect("home");

    // Pre-save candidate check when creating `/home/david/Bilder` on Ziel Z.
    let candidate_conflicts = check_nested_sources_for_candidate(
        None,
        Path::new("/home/david/Bilder"),
        &dest_z.id,
        std::slice::from_ref(&backup_home),
    );
    assert_eq!(candidate_conflicts.len(), 1);
    assert_eq!(candidate_conflicts[0].other_backup_id, backup_home.id);
    assert_eq!(candidate_conflicts[0].other_backup_name, "Home");
    assert_eq!(
        candidate_conflicts[0].other_source_path,
        PathBuf::from("/home/david")
    );
    assert_eq!(
        candidate_conflicts[0].relationship,
        NestedRelationship::ContainedIn
    );

    let backup_pictures = Backup::new(
        "Bilder",
        Source::folder("/home/david/Bilder"),
        dest_z.id.clone(),
    )
    .expect("pictures");

    // Config validation and saving must remain allowed (`AC-M2-10-1`).
    let config = AppConfig {
        destinations: vec![dest_z],
        backups: vec![backup_home.clone(), backup_pictures.clone()],
        ..AppConfig::default()
    };
    config
        .validate()
        .expect("nested sources config must validate");
    save_config(&config_path, &config).expect("nested sources config must save");
    let loaded = load_config(&config_path).expect("load saved config");
    assert_eq!(loaded.backups.len(), 2);

    // Bidirectional warnings mention both backups and paths.
    let warnings = find_nested_source_warnings(&loaded.backups);
    assert_eq!(warnings.len(), 2);

    let pic_warns = warnings
        .get(&backup_pictures.id)
        .expect("Bilder backup should have nested warning");
    assert_eq!(pic_warns.len(), 1);
    assert_eq!(pic_warns[0].other_backup_id, backup_home.id);
    assert_eq!(pic_warns[0].other_source_path, PathBuf::from("/home/david"));
    assert_eq!(pic_warns[0].relationship, NestedRelationship::ContainedIn);

    let home_warns = warnings
        .get(&backup_home.id)
        .expect("Home backup should have nested warning");
    assert_eq!(home_warns.len(), 1);
    assert_eq!(home_warns[0].other_backup_id, backup_pictures.id);
    assert_eq!(
        home_warns[0].other_source_path,
        PathBuf::from("/home/david/Bilder")
    );
    assert_eq!(home_warns[0].relationship, NestedRelationship::Contains);
}

#[test]
fn ac_m2_10_2_nested_sources_on_different_destinations_produce_no_warning() {
    let dest_a = Destination::new(
        "Ziel A",
        DestinationKind::Local {
            path: PathBuf::from("/tmp/repo-a"),
        },
    )
    .expect("dest a");
    let dest_b = Destination::new(
        "Ziel B",
        DestinationKind::Local {
            path: PathBuf::from("/tmp/repo-b"),
        },
    )
    .expect("dest b");

    let backup_home =
        Backup::new("Home", Source::folder("/home/david"), dest_a.id.clone()).expect("home");
    let backup_pictures = Backup::new(
        "Bilder",
        Source::folder("/home/david/Bilder"),
        dest_b.id.clone(),
    )
    .expect("pictures");

    let candidate_conflicts = check_nested_sources_for_candidate(
        None,
        Path::new("/home/david/Bilder"),
        &dest_b.id,
        std::slice::from_ref(&backup_home),
    );
    assert!(candidate_conflicts.is_empty());

    let warnings = find_nested_source_warnings(&[backup_home, backup_pictures]);
    assert!(warnings.is_empty());
}

#[test]
fn ac_m2_10_3_component_wise_comparison_does_not_warn_on_prefix_siblings() {
    let dest_z = Destination::new(
        "Ziel Z",
        DestinationKind::Local {
            path: PathBuf::from("/tmp/repo-z"),
        },
    )
    .expect("dest z");

    let backup_foto =
        Backup::new("Foto", Source::folder("/data/foto"), dest_z.id.clone()).expect("foto");
    let backup_fotos =
        Backup::new("Fotos", Source::folder("/data/fotos"), dest_z.id.clone()).expect("fotos");

    let candidate_conflicts = check_nested_sources_for_candidate(
        None,
        Path::new("/data/fotos"),
        &dest_z.id,
        std::slice::from_ref(&backup_foto),
    );
    assert!(
        candidate_conflicts.is_empty(),
        "/data/foto and /data/fotos must not conflict"
    );

    let warnings = find_nested_source_warnings(&[backup_foto, backup_fotos]);
    assert!(
        warnings.is_empty(),
        "component-wise matching must not emit warning for /data/foto vs /data/fotos"
    );
}
