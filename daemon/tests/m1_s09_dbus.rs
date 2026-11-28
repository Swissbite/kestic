// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::revision::UpdateNow;

use common::m3::{
    LockableSecretStore, M3DaemonOpts, assert_dbus_error_name, connect_local_repo, spawn_m3_daemon,
};
use common::{
    CapturedKesticSignal, KesticSignalCollector, connect_default_client, find_kestic_binary,
    isolated_secret_store, restic_test_binaries, run_isolated_test, sandbox_run_config,
    start_fake_notifications,
};
use kestic_core::model::{
    Backup, Destination, DestinationKind, ResticProvider, Run, RunStatus, Source,
};
use kestic_core::secrets::{SecretError, SecretStore, SecretValue};
use kestic_daemon::dbus_api::BackendSecrets;
use kestic_daemon::dbus_api::{
    KESTIC_BUS_NAME, KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH, OverallStatusDto,
};
use kestic_daemon::notifications::KESTIC_DESKTOP_ENTRY;
use kestic_daemon::secrets::SecretServiceStore;
use kestic_daemon::service::{DaemonOutcome, DaemonRunConfig, run_daemon};
use kestic_i18n::codes::BlockedReason;
use kestic_test_support::config::load_config;
use kestic_test_support::password::{assert_no_secret, strong_password};
use kestic_test_support::tools::{free_loopback_port, install_marker_spy};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn ac_m1_09_1_single_instance_lock_exits_cleanly_as_already_running() {
    run_isolated_test(
        "ac_m1_09_1_single_instance_lock_exits_cleanly_as_already_running",
        "KESTIC_M1_S09_SINGLE",
        |root| {
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts::on_request("", KESTIC_BUS_NAME, &restic_bin),
            );
            let client = &daemon.client;

            assert!(client.is_bus_owned().expect("is_bus_owned"));

            let second_cfg = DaemonRunConfig {
                ready_file: None,
                ..sandbox_run_config(root)
            };
            let second_outcome = run_daemon(second_cfg).expect("second daemon run");
            assert_eq!(second_outcome, DaemonOutcome::AlreadyRunning);

            // Also verify CLI `kestic daemon` exits 0 cleanly while first daemon holds the name.
            let cli_out = Command::new(find_kestic_binary())
                .args(["daemon", "--duration-ms", "2000"])
                .output()
                .expect("run second kestic daemon CLI");
            assert!(
                cli_out.status.success(),
                "second CLI daemon must exit 0, got {:?}",
                cli_out.status.code()
            );
            let cli_stdout = String::from_utf8_lossy(&cli_out.stdout);
            assert!(
                cli_stdout.contains("already running"),
                "second CLI daemon must report the running instance on stdout, got: {cli_stdout:?}"
            );

            daemon.stop();
        },
    );
}

#[test]
fn ac_m1_09_2_09_3_09_4_crud_validation_config_persistence_and_signals() {
    run_isolated_test(
        "ac_m1_09_2_09_3_09_4_crud_validation_config_persistence_and_signals",
        "KESTIC_M1_S09_CRUD",
        |root| {
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts::on_request("", KESTIC_BUS_NAME, &restic_bin),
            );
            let client = &daemon.client;
            let signals = KesticSignalCollector::start(
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );

            let cfg_path = root.join("cfg").join("kestic").join("config.toml");
            let repo_dir = root.join("repo-local");
            let source_dir = root.join("src-docs");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&source_dir).expect("mkdir src");
            fs::write(source_dir.join("hello.txt"), b"kestic backup data\n").expect("write file");

            // AC-M1-09-3: Add destination + backup purely over D-Bus; daemon writes config.toml.
            let dest = Destination::new(
                "Local Test Repo",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("valid dest");
            let repo_password = strong_password();
            let dest_id = client
                .add_destination(&dest, "init", &repo_password, &BackendSecrets::default())
                .expect("AddDestination");
            assert_eq!(dest_id, dest.id.as_str());

            let backup = Backup::new("Documents", Source::folder(&source_dir), dest.id.clone())
                .expect("valid backup");
            let backup_id = client.add_backup(&backup).expect("AddBackup");
            assert_eq!(backup_id, backup.id.as_str());

            assert!(cfg_path.is_file(), "daemon must persist config.toml");
            let persisted = load_config(&cfg_path).expect("load persisted config.toml");
            assert_eq!(persisted.destinations.len(), 1);
            assert_eq!(persisted.backups.len(), 1);
            assert_eq!(persisted.backups[0].id, backup.id);

            let raw_toml = fs::read_to_string(&cfg_path).expect("read raw config.toml");
            assert_no_secret(&raw_toml, &repo_password, "config.toml");

            // AC-M1-09-2: Invalid AddBackup & UpdateBackup return InvalidInput, config.toml unchanged.
            let before_bytes = fs::read(&cfg_path).expect("read config bytes before invalid call");

            let mut invalid_add = backup.clone();
            invalid_add.source = Source::folder("");
            let add_err = client
                .add_backup(&invalid_add)
                .expect_err("empty source AddBackup must fail");
            assert_dbus_error_name(&add_err, "net.daester.Kestic.Error.InvalidInput");
            assert_eq!(fs::read(&cfg_path).expect("read config"), before_bytes);

            let mut invalid_upd = backup.clone();
            invalid_upd.source = Source::folder("");
            let upd_err = client
                .update_backup_now(&invalid_upd)
                .expect_err("empty source UpdateBackup must fail");
            assert_dbus_error_name(&upd_err, "net.daester.Kestic.Error.InvalidInput");
            assert_eq!(fs::read(&cfg_path).expect("read config"), before_bytes);

            // AC-M1-09-4: StartBackup emits RunUpdated + StatusChanged signals.
            signals.clear();
            let run_id = client.start_backup(&backup_id).expect("StartBackup");

            let captured = signals.wait_until(Duration::from_secs(20), |sigs| {
                let has_status = sigs
                    .iter()
                    .any(|s| matches!(s, CapturedKesticSignal::StatusChanged(_)));
                let has_finished_run = sigs.iter().any(|s| {
                    if let CapturedKesticSignal::RunUpdated(json) = s
                        && let Ok(run) = serde_json::from_str::<Run>(json)
                    {
                        return run.id.as_str() == run_id && run.status == RunStatus::Ok;
                    }
                    false
                });
                has_status && has_finished_run
            });

            assert!(captured.iter().any(|s| {
                if let CapturedKesticSignal::StatusChanged(json) = s {
                    serde_json::from_str::<OverallStatusDto>(json).is_ok()
                } else {
                    false
                }
            }));

            // Verify backup CRUD update + remove + destination remove.
            let mut updated_backup = backup.clone();
            updated_backup.excludes.lines = vec!["*.tmp".to_string()];
            client
                .update_backup_now(&updated_backup)
                .expect("UpdateBackup valid");
            let state = client.get_state().expect("GetState");
            assert_eq!(state.backups[0].backup.excludes.lines, vec!["*.tmp"]);

            client.remove_backup(&backup_id).expect("RemoveBackup");
            client
                .remove_destination(&dest_id)
                .expect("RemoveDestination after backup removed");
            let reloaded = load_config(&cfg_path).expect("reload config");
            assert!(reloaded.backups.is_empty());
            assert!(reloaded.destinations.is_empty());

            daemon.stop();
        },
    );
}

#[test]
fn ac_m1_09_6_and_09_7_missing_source_blocks_without_restic_and_respects_window_active() {
    run_isolated_test(
        "ac_m1_09_6_and_09_7_missing_source_blocks_without_restic_and_respects_window_active",
        "KESTIC_M1_S09_BLOCKED",
        |root| {
            let (_notif_conn, notif_capture) = start_fake_notifications();
            let (_, real_restic) = restic_test_binaries().pop().expect("restic binary");

            // Spy wrapper that records if any non-`version` restic invocation happens.
            let (spy_script, spy_marker) = install_marker_spy(root, "restic-spy", &real_restic);

            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts::on_request("", KESTIC_BUS_NAME, &real_restic),
            );
            let client = &daemon.client;
            let signals = KesticSignalCollector::start(
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );

            let repo_dir = root.join("repo-blocked");
            let source_dir = root.join("src-disappears");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&source_dir).expect("mkdir src");

            let dest = Destination::new(
                "Local Repo",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            connect_local_repo(client, &real_restic, &dest);

            let backup = Backup::new("Photos", Source::folder(&source_dir), dest.id.clone())
                .expect("backup");
            let backup_id = client.add_backup(&backup).expect("add backup");

            // Install spy provider AFTER init so only StartBackup would hit it.
            client
                .set_restic_provider(&ResticProvider::Custom { path: spy_script })
                .expect("install restic spy");

            // Remove source directory so preflight fails before launching restic.
            fs::remove_dir_all(&source_dir).expect("remove source dir");

            // 1. When window_active = true -> ManualRunBlocked error + signal, NO notification.
            client
                .window_active(true, "overview")
                .expect("WindowActive(true)");
            signals.clear();
            notif_capture.clear();

            let err1 = client
                .start_backup(&backup_id)
                .expect_err("StartBackup with missing source must fail");
            assert_dbus_error_name(&err1, "net.daester.Kestic.Error.ManualRunBlocked");
            assert!(
                !spy_marker.exists(),
                "zero restic process must be spawned when source is unavailable"
            );

            let sigs1 = signals.wait_until(Duration::from_secs(3), |sigs| {
                sigs.iter().any(|s| {
                    matches!(
                        s,
                        CapturedKesticSignal::ManualRunBlocked {
                            backup_id: bid,
                            reason,
                        } if bid == &backup_id && reason == BlockedReason::SourceUnavailable.as_str()
                    )
                })
            });
            assert!(!sigs1.is_empty());
            assert!(
                notif_capture.snapshot().is_empty(),
                "notification must be suppressed while window_active = true"
            );

            // 2. When window_active = false -> emits ManualRunBlocked AND sends notification.
            client
                .window_active(false, "")
                .expect("WindowActive(false)");
            signals.clear();
            notif_capture.clear();

            let err2 = client
                .start_backup(&backup_id)
                .expect_err("StartBackup with missing source must still fail");
            assert_dbus_error_name(&err2, "net.daester.Kestic.Error.ManualRunBlocked");
            assert!(
                !spy_marker.exists(),
                "zero restic process must be spawned when source is unavailable"
            );

            signals.wait_until(Duration::from_secs(3), |sigs| {
                sigs.iter().any(|s| {
                    matches!(
                        s,
                        CapturedKesticSignal::ManualRunBlocked {
                            backup_id: bid,
                            reason,
                        } if bid == &backup_id && reason == BlockedReason::SourceUnavailable.as_str()
                    )
                })
            });

            let notifs = notif_capture.snapshot();
            assert_eq!(notifs.len(), 1, "expected 1 ManualRunBlocked notification");
            assert_eq!(
                notifs[0].desktop_entry.as_deref(),
                Some(KESTIC_DESKTOP_ENTRY)
            );
            assert_eq!(notifs[0].event_id.as_deref(), Some("ManualRunBlocked"));

            daemon.stop();
        },
    );
}

#[test]
fn ac_m1_19_1_19_4_19_6_rename_in_use_and_verify_connection() {
    run_isolated_test(
        "ac_m1_19_1_19_4_19_6_rename_in_use_and_verify_connection",
        "KESTIC_M1_S09_RENAME",
        |root| {
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts::on_request("", KESTIC_BUS_NAME, &restic_bin),
            );
            let client = &daemon.client;

            let cfg_path = root.join("cfg").join("kestic").join("config.toml");
            let secret_store = common::isolated_secret_store();

            let repo_dir = root.join("repo-mut");
            let src_dir = root.join("src-mut");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");

            let dest = Destination::new(
                "Primary Repo",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            let initial_password = connect_local_repo(client, &restic_bin, &dest);
            let dest_id = dest.id.as_str().to_string();

            let backup = Backup::new("Original Name", Source::folder(&src_dir), dest.id.clone())
                .expect("backup");
            let backup_id = client.add_backup(&backup).expect("AddBackup");

            // AC-M1-19-1: RenameBackup changes name while keeping BackupId immutable.
            client
                .rename_backup(&backup_id, "Renamed Documents")
                .expect("RenameBackup");
            let state = client.get_state().expect("GetState");
            assert_eq!(state.backups[0].backup.id.as_str(), backup_id);
            assert_eq!(state.backups[0].backup.name, "Renamed Documents");
            let cfg_after_rename = load_config(&cfg_path).expect("load config");
            assert_eq!(cfg_after_rename.backups[0].name, "Renamed Documents");

            // AC-M1-19-4: RemoveDestination rejected with Error.InUse while backup references it.
            let cfg_before_in_use = fs::read(&cfg_path).expect("read config");
            let in_use_err = client
                .remove_destination(&dest_id)
                .expect_err("RemoveDestination must fail while referenced");
            assert_dbus_error_name(&in_use_err, "net.daester.Kestic.Error.InUse");
            assert_eq!(fs::read(&cfg_path).expect("read config"), cfg_before_in_use);

            // AC-M1-19-6: UpdateDestination with verify_connection = true fails on unreachable endpoint,
            // leaving config.toml and stored password untouched.
            let mut bad_dest = dest.clone();
            bad_dest.name = "Broken Endpoint".to_string();
            bad_dest.kind = DestinationKind::Rest {
                url: format!("http://127.0.0.1:{}/alice", free_loopback_port()),
                username: Some("alice".to_string()),
            };
            let upd_dest_err = client
                .update_destination_now(
                    &bad_dest,
                    "connect",
                    &strong_password(),
                    &BackendSecrets::default(),
                    true,
                )
                .expect_err("UpdateDestination with unreachable target must fail");
            assert_dbus_error_name(
                &upd_dest_err,
                "net.daester.Kestic.Error.DestinationUnreachable",
            );
            assert_eq!(fs::read(&cfg_path).expect("read config"), cfg_before_in_use);
            assert_eq!(
                secret_store
                    .get_secret(&dest_id, "repo")
                    .expect("get secret"),
                Some(SecretValue::from_str_secret(&initial_password)),
                "stored repo password"
            );

            daemon.stop();
        },
    );
}

/// AC-M1-20-1, AC-M1-20-2, AC-M1-20-3 (FR-15): a wrong password is rejected and leaves the stored
/// secret alone; a password rotated outside Kestic is accepted without touching the repository keys.
#[test]
fn ac_m1_20_1_20_2_20_3_update_password_verifies_without_changing_keys() {
    run_isolated_test(
        "ac_m1_20_1_20_2_20_3_update_password_verifies_without_changing_keys",
        "KESTIC_M1_S09_PASSWORD",
        |root| {
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts::on_request("", KESTIC_BUS_NAME, &restic_bin),
            );
            let client = &daemon.client;
            let secret_store = common::isolated_secret_store();

            let repo_dir = root.join("repo-pw");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            let dest = Destination::new(
                "Primary Repo",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            let initial_password = connect_local_repo(client, &restic_bin, &dest);
            let dest_id = dest.id.as_str().to_string();
            let rotated_password = strong_password();

            // AC-M1-20-2: UpdateDestinationPassword with wrong password fails & leaves stored secret intact.
            let restic_cache = root.join("restic-cache");
            let keys_before = Command::new(&restic_bin)
                .args(["-r", &repo_dir.to_string_lossy(), "key", "list", "--json"])
                .env("RESTIC_PASSWORD", &initial_password)
                .env("RESTIC_CACHE_DIR", &restic_cache)
                .output()
                .expect("restic key list before");
            assert!(keys_before.status.success());

            let bad_pw_err = client
                .update_destination_password(&dest_id, &strong_password())
                .expect_err("UpdateDestinationPassword with wrong pw must fail");
            assert_dbus_error_name(&bad_pw_err, "net.daester.Kestic.Error.WrongPassword");
            assert_eq!(
                secret_store
                    .get_secret(&dest_id, "repo")
                    .expect("get secret"),
                Some(SecretValue::from_str_secret(&initial_password)),
                "stored repo password"
            );

            // Rotate repository password externally using `restic key passwd`.
            let new_pw_file = root.join("new-pw.txt");
            fs::write(&new_pw_file, format!("{rotated_password}\n")).expect("write new pw file");
            let passwd_out = Command::new(&restic_bin)
                .args([
                    "-r",
                    &repo_dir.to_string_lossy(),
                    "key",
                    "passwd",
                    "--new-password-file",
                ])
                .arg(&new_pw_file)
                .env("RESTIC_PASSWORD", &initial_password)
                .env("RESTIC_CACHE_DIR", &restic_cache)
                .output()
                .expect("restic key passwd");
            assert!(
                passwd_out.status.success(),
                "restic key passwd failed: {}",
                String::from_utf8_lossy(&passwd_out.stderr)
            );

            let keys_after_external = Command::new(&restic_bin)
                .args(["-r", &repo_dir.to_string_lossy(), "key", "list", "--json"])
                .env("RESTIC_PASSWORD", &rotated_password)
                .env("RESTIC_CACHE_DIR", &restic_cache)
                .output()
                .expect("restic key list after external change");
            assert!(keys_after_external.status.success());

            // AC-M1-20-1 & AC-M1-20-3: UpdateDestinationPassword verifies new password without running `restic key`.
            client
                .update_destination_password(&dest_id, &rotated_password)
                .expect("UpdateDestinationPassword with valid new pw must succeed");
            assert_eq!(
                secret_store
                    .get_secret(&dest_id, "repo")
                    .expect("get secret"),
                Some(SecretValue::from_str_secret(&rotated_password)),
                "stored repo password"
            );

            let keys_after_update = Command::new(&restic_bin)
                .args(["-r", &repo_dir.to_string_lossy(), "key", "list", "--json"])
                .env("RESTIC_PASSWORD", &rotated_password)
                .env("RESTIC_CACHE_DIR", &restic_cache)
                .output()
                .expect("restic key list after UpdateDestinationPassword");
            assert!(keys_after_update.status.success());
            assert_eq!(
                keys_after_external.stdout, keys_after_update.stdout,
                "UpdateDestinationPassword must not modify repository keys"
            );

            daemon.stop();
        },
    );
}

#[test]
fn duplicate_destination_location_rejected_and_edit_reinit_empty_folder() {
    run_isolated_test(
        "duplicate_destination_location_rejected_and_edit_reinit_empty_folder",
        "KESTIC_M1_S09_DUP_REINIT",
        |root| {
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts::on_request("", KESTIC_BUS_NAME, &restic_bin),
            );
            let client = &daemon.client;
            let signals = KesticSignalCollector::start(
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );

            let cfg_path = root.join("cfg").join("kestic").join("config.toml");
            let repo_dir = root.join("repo-shared");
            let other_repo_dir = root.join("repo-other");
            let src_dir = root.join("src-docs");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&other_repo_dir).expect("mkdir other repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("note.txt"), b"hello kestic\n").expect("write source file");

            let dest1 = Destination::new(
                "Primary Local",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest1");
            let repo_password = connect_local_repo(client, &restic_bin, &dest1);
            let dest1_id = dest1.id.as_str().to_string();

            let backup = Backup::new("Docs", Source::folder(&src_dir), dest1.id.clone())
                .expect("valid backup");
            let backup_id = client.add_backup(&backup).expect("AddBackup");

            let cfg_before = fs::read(&cfg_path).expect("read config before duplicate checks");

            // Variant of the same directory (`repo-shared/.`) must be rejected by Test & Add.
            let dup_path_variant = PathBuf::from(format!("{}/.", repo_dir.display()));
            let dup_dest = Destination::new(
                "Duplicate Name",
                DestinationKind::Local {
                    path: dup_path_variant,
                },
            )
            .expect("dup_dest");

            let test_dup = client
                .test_destination_connection(
                    &dup_dest,
                    "connect",
                    &repo_password,
                    &BackendSecrets::default(),
                )
                .expect("TestDestinationConnection dup");
            assert!(!test_dup.ok, "duplicate location test must fail");
            assert_eq!(
                test_dup.error_kind.map(|c| c.as_str()),
                Some("duplicate_location")
            );

            let add_dup_err = client
                .add_destination(
                    &dup_dest,
                    "connect",
                    &repo_password,
                    &BackendSecrets::default(),
                )
                .expect_err("AddDestination with duplicate location must fail");
            assert_dbus_error_name(&add_dup_err, "net.daester.Kestic.Error.DuplicateLocation");
            assert_eq!(
                fs::read(&cfg_path).expect("read config after dup add"),
                cfg_before
            );

            // Second distinct destination cannot be edited to point to dest1's location.
            let dest2 = Destination::new(
                "Second Local",
                DestinationKind::Local {
                    path: other_repo_dir,
                },
            )
            .expect("dest2");
            connect_local_repo(client, &restic_bin, &dest2);

            let mut dest2_collision = dest2.clone();
            dest2_collision.kind = DestinationKind::Local {
                path: PathBuf::from(format!("{}/", repo_dir.display())),
            };
            let upd_dup_err = client
                .update_destination_now(
                    &dest2_collision,
                    "connect",
                    "",
                    &BackendSecrets::default(),
                    true,
                )
                .expect_err("UpdateDestination into dest1 location must fail");
            assert_dbus_error_name(&upd_dup_err, "net.daester.Kestic.Error.DuplicateLocation");

            // Delete dest1's folder from disk and recreate it empty (simulating Dolphin / rm + mkdir).
            fs::remove_dir_all(&repo_dir).expect("delete repo_dir");
            fs::create_dir_all(&repo_dir).expect("recreate empty repo_dir");

            // Testing existing dest1 in `connect` mode must detect missing repo (`repository_missing`),
            // while testing in `init` mode with empty password uses the stored secret and succeeds.
            let check_missing = client
                .test_destination_connection(&dest1, "connect", "", &BackendSecrets::default())
                .expect("TestDestinationConnection connect on empty dir");
            assert!(!check_missing.ok);
            assert_eq!(
                check_missing.error_kind.map(|c| c.as_str()),
                Some("repository_missing")
            );

            let check_reinit = client
                .test_destination_connection(&dest1, "init", "", &BackendSecrets::default())
                .expect("TestDestinationConnection init on empty dir");
            assert!(
                check_reinit.ok,
                "TestDestinationConnection init should pass on empty folder: {check_reinit:?}"
            );

            // UpdateDestination with mode="init" initializes the repo in the empty folder while
            // keeping DestinationId, stored secrets, and attached backup intact.
            client
                .update_destination_now(&dest1, "init", "", &BackendSecrets::default(), true)
                .expect("UpdateDestination init on empty folder");
            assert!(
                repo_dir.join("config").is_file(),
                "restic repo config must be created by UpdateDestination(init)"
            );

            let state = client.get_state().expect("GetState");
            assert_eq!(state.destinations[0].destination.id.as_str(), dest1_id);
            assert_eq!(state.backups[0].backup.destination_id, dest1.id);

            signals.clear();
            let run_id = client
                .start_backup(&backup_id)
                .expect("StartBackup after reinit");
            signals.wait_until(Duration::from_secs(20), |sigs| {
                sigs.iter().any(|s| {
                    if let CapturedKesticSignal::RunUpdated(json) = s
                        && let Ok(run) = serde_json::from_str::<Run>(json)
                    {
                        return run.id.as_str() == run_id && run.status == RunStatus::Ok;
                    }
                    false
                })
            });

            daemon.stop();
        },
    );
}

#[test]
fn unlock_secret_store_over_dbus_clears_blocking_and_emits_status_changed() {
    run_isolated_test(
        "unlock_secret_store_over_dbus_clears_blocking_and_emits_status_changed",
        "KESTIC_M1_S09_UNLOCK_INNER",
        |root| {
            let (_, restic_bin) = restic_test_binaries()
                .into_iter()
                .next()
                .expect("restic binary");
            let store = Arc::new(LockableSecretStore::new(true));
            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts {
                    secret_store: Some(store),
                    ..M3DaemonOpts::on_request("", KESTIC_BUS_NAME, &restic_bin)
                },
            );
            let client = &daemon.client;
            let signals = KesticSignalCollector::start(
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );

            let initial = client.get_state().expect("GetState locked");
            assert_eq!(
                initial.blocking.as_ref().map(|b| b.kind.as_str()),
                Some("secret_store_locked")
            );

            signals.clear();
            let unlocked = client
                .unlock_secret_store()
                .expect("UnlockSecretStore D-Bus call");
            assert!(unlocked);

            let after = client.get_state().expect("GetState unlocked");
            assert!(
                after.blocking.is_none(),
                "blocking banner must clear after UnlockSecretStore: {:?}",
                after.blocking
            );
            let captured = signals.wait_until(Duration::from_secs(5), |sigs| {
                sigs.iter()
                    .any(|s| matches!(s, CapturedKesticSignal::StatusChanged(_)))
            });
            assert!(
                captured
                    .iter()
                    .any(|s| matches!(s, CapturedKesticSignal::StatusChanged(_))),
                "StatusChanged must be emitted after UnlockSecretStore: {captured:?}"
            );

            daemon.stop();
        },
    );
}

/// The test keyring, except that an interactive unlock waits until the test opens the gate, so
/// an `UnlockSecretStore` call is still under way while the daemon stops.
struct GatedUnlockStore {
    inner: SecretServiceStore,
    entered: Mutex<mpsc::Sender<()>>,
    gate: Mutex<mpsc::Receiver<()>>,
}

impl SecretStore for GatedUnlockStore {
    fn set_secret(
        &self,
        target_id: &str,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError> {
        self.inner.set_secret(target_id, kind, value)
    }

    fn get_secret(&self, target_id: &str, kind: &str) -> Result<Option<SecretValue>, SecretError> {
        self.inner.get_secret(target_id, kind)
    }

    fn delete_secret(&self, target_id: &str, kind: &str) -> Result<(), SecretError> {
        self.inner.delete_secret(target_id, kind)
    }

    fn unlock_interactive(&self) -> Result<(), SecretError> {
        self.entered
            .lock()
            .expect("entered sender")
            .send(())
            .expect("the test waits for the unlock");
        self.gate
            .lock()
            .expect("gate receiver")
            .recv_timeout(Duration::from_secs(20))
            .map_err(|_| SecretError::LockedOrPromptRequired("gate never opened".to_string()))
    }
}

/// A method call still under way when `ConfirmQuit` stops the daemon gets its reply instead of
/// `NoReply`: the daemon waits with closing its D-Bus connection until the replies under way,
/// such as the one to `ConfirmQuit` itself, have been written, and no longer.
#[test]
fn reply_under_way_at_shutdown_reaches_its_caller() {
    run_isolated_test(
        "reply_under_way_at_shutdown_reaches_its_caller",
        "KESTIC_M1_S09_SHUTDOWN_REPLY",
        |root| {
            let (_, restic_bin) = restic_test_binaries()
                .into_iter()
                .next()
                .expect("restic binary");
            let (entered_tx, entered_rx) = mpsc::channel();
            let (gate_tx, gate_rx) = mpsc::channel();
            let store = Arc::new(GatedUnlockStore {
                inner: isolated_secret_store(),
                entered: Mutex::new(entered_tx),
                gate: Mutex::new(gate_rx),
            });
            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts {
                    secret_store: Some(store),
                    ..M3DaemonOpts::on_request("", KESTIC_BUS_NAME, &restic_bin)
                },
            );

            let unlocking = thread::spawn(|| {
                connect_default_client()
                    .expect("second client")
                    .unlock_secret_store()
            });
            entered_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("UnlockSecretStore is under way");

            let gate = thread::spawn(move || {
                // quiet window: the unlock stays held while the daemon is asked to stop
                thread::sleep(Duration::from_millis(300));
                let opened = Instant::now();
                gate_tx.send(()).expect("open the unlock gate");
                opened
            });
            daemon.stop();
            let stopped = Instant::now();
            let gate_opened = gate.join().expect("gate thread");

            let reply = unlocking.join().expect("unlocking thread");
            assert!(
                matches!(reply, Ok(true)),
                "(a) the call under way at shutdown must get its reply, not an error: {reply:?}"
            );
            assert!(
                stopped > gate_opened,
                "(b) the daemon must stop only after the call under way could answer"
            );
            let waited = stopped.duration_since(gate_opened);
            assert!(
                waited < Duration::from_secs(1),
                "(c) the daemon must stop once the reply is out, not when the 2 s limit ends: \
                 stopped {waited:?} after the gate opened"
            );
        },
    );
}
