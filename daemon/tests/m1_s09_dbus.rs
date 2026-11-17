// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::{
    CapturedKesticSignal, KesticSignalCollector, find_kestic_binary, free_loopback_port,
    restic_test_binaries, run_isolated_test, start_fake_notifications, wait_for_file,
};
use kestic_core::config::load_config;
use kestic_core::model::{
    Backup, Destination, DestinationKind, ResticProvider, Run, RunStatus, Source,
};
use kestic_core::secrets::{FileSecretStore, SecretStore};
use kestic_daemon::dbus_api::{
    KESTIC_BUS_NAME, KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH, KesticClient, OverallStatusDto,
    REASON_SOURCE_UNAVAILABLE,
};
use kestic_daemon::notifications::KESTIC_DESKTOP_ENTRY;
use kestic_daemon::service::{DaemonOutcome, DaemonRunConfig, run_daemon};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn assert_dbus_error_name(err: &zbus::Error, expected_name: &str) {
    match err {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(
                name.as_str(),
                expected_name,
                "unexpected D-Bus error name: {err:?}"
            );
        }
        other => panic!("expected D-Bus MethodError `{expected_name}`, got {other:?}"),
    }
}

fn spawn_daemon_thread(
    root: &Path,
    ready_file: PathBuf,
    restic_path: PathBuf,
) -> (
    KesticClient,
    thread::JoinHandle<Result<DaemonOutcome, String>>,
) {
    let cfg_path = root.join("cfg").join("kestic").join("config.toml");
    let state_dir = root.join("state").join("kestic");
    let secret_file = root.join("secrets.hex");
    let kestic_bin = find_kestic_binary();

    let run_cfg = DaemonRunConfig {
        bus_name: KESTIC_BUS_NAME.to_string(),
        assume_sni_available: true,
        ready_file: Some(ready_file.clone()),
        config_path: Some(cfg_path),
        state_dir: Some(state_dir),
        kestic_binary: Some(kestic_bin),
        secret_store: Some(Arc::new(FileSecretStore::new(secret_file))),
        suppress_ui_spawn: true,
        enable_job_view: false,
        enable_portal_inhibit: false,
        ..Default::default()
    };

    let handle = thread::spawn(move || run_daemon(run_cfg));
    wait_for_file(&ready_file, Duration::from_secs(5));

    let client = KesticClient::connect_default().expect("connect KesticClient");
    let status = client
        .set_restic_provider(&ResticProvider::Custom { path: restic_path })
        .expect("configure test restic provider");
    assert!(status.valid, "restic provider invalid: {status:?}");
    (client, handle)
}

#[test]
fn ac_m1_09_1_single_instance_lock_exits_cleanly_as_already_running() {
    run_isolated_test(
        "ac_m1_09_1_single_instance_lock_exits_cleanly_as_already_running",
        "KESTIC_M1_S09_SINGLE",
        |root| {
            let ready = root.join("daemon1.ready");
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let (client, handle) = spawn_daemon_thread(root, ready, restic_bin);

            assert!(client.is_bus_owned().expect("is_bus_owned"));

            let second_cfg = DaemonRunConfig {
                bus_name: KESTIC_BUS_NAME.to_string(),
                assume_sni_available: true,
                config_path: Some(root.join("cfg").join("kestic").join("config.toml")),
                state_dir: Some(root.join("state").join("kestic")),
                kestic_binary: Some(find_kestic_binary()),
                secret_store: Some(Arc::new(FileSecretStore::new(root.join("secrets.hex")))),
                suppress_ui_spawn: true,
                enable_job_view: false,
                enable_portal_inhibit: false,
                ..Default::default()
            };
            let second_outcome = run_daemon(second_cfg).expect("second daemon run");
            assert_eq!(second_outcome, DaemonOutcome::AlreadyRunning);

            // Also verify CLI `kestic daemon` exits 0 cleanly while first daemon holds the name.
            let cli_out = Command::new(find_kestic_binary())
                .args(["daemon", "--duration-ms", "2000"])
                .output()
                .expect("run second kestic daemon CLI");
            assert!(cli_out.status.success());

            client.confirm_quit().expect("confirm quit");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn ac_m1_09_2_09_3_09_4_crud_validation_config_persistence_and_signals() {
    run_isolated_test(
        "ac_m1_09_2_09_3_09_4_crud_validation_config_persistence_and_signals",
        "KESTIC_M1_S09_CRUD",
        |root| {
            let ready = root.join("daemon.ready");
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let (client, handle) = spawn_daemon_thread(root, ready, restic_bin);
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
            let dest_id = client
                .add_destination(&dest, "init", "repo-pw-123", "")
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
            assert!(
                !raw_toml.contains("repo-pw-123"),
                "secrets must never be written into config.toml"
            );

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
                .update_backup(&invalid_upd)
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
                .update_backup(&updated_backup)
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

            client.confirm_quit().expect("confirm quit");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
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
            let ready = root.join("daemon.ready");
            let (_, real_restic) = restic_test_binaries().pop().expect("restic binary");

            // Spy wrapper that records if any non-`version` restic invocation happens.
            let spy_marker = root.join("restic-invoked.marker");
            let spy_script = root.join("restic-spy");
            let script_body = format!(
                "#!/bin/sh\nif [ \"$1\" != \"version\" ]; then\n  touch \"{}\"\nfi\nexec \"{}\" \"$@\"\n",
                spy_marker.display(),
                real_restic.display()
            );
            fs::write(&spy_script, script_body).expect("write restic spy");
            fs::set_permissions(&spy_script, fs::Permissions::from_mode(0o755))
                .expect("chmod restic spy");

            let (client, handle) = spawn_daemon_thread(root, ready, real_restic);
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
            client
                .add_destination(&dest, "init", "pw-blocked", "")
                .expect("add dest");

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
                        } if bid == &backup_id && reason == REASON_SOURCE_UNAVAILABLE
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
                        } if bid == &backup_id && reason == REASON_SOURCE_UNAVAILABLE
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

            client.confirm_quit().expect("confirm quit");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn ac_m1_19_1_19_4_19_6_and_20_1_20_2_20_3_rename_in_use_verify_update_and_password() {
    run_isolated_test(
        "ac_m1_19_1_19_4_19_6_and_20_1_20_2_20_3_rename_in_use_verify_update_and_password",
        "KESTIC_M1_S09_MUTATIONS",
        |root| {
            let ready = root.join("daemon.ready");
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let (client, handle) = spawn_daemon_thread(root, ready, restic_bin.clone());

            let cfg_path = root.join("cfg").join("kestic").join("config.toml");
            let secret_file = root.join("secrets.hex");
            let secret_store = FileSecretStore::new(&secret_file);

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
            let dest_id = client
                .add_destination(&dest, "init", "initial-pw", "")
                .expect("AddDestination");

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
                .update_destination(&bad_dest, "connect", "wrong-or-unused", "", true)
                .expect_err("UpdateDestination with unreachable target must fail");
            assert_dbus_error_name(
                &upd_dest_err,
                "net.daester.Kestic.Error.DestinationUnreachable",
            );
            assert_eq!(fs::read(&cfg_path).expect("read config"), cfg_before_in_use);
            assert_eq!(
                secret_store
                    .get_secret(&dest_id, "repo")
                    .expect("get secret")
                    .unwrap()
                    .as_str()
                    .unwrap(),
                "initial-pw"
            );

            // AC-M1-20-2: UpdateDestinationPassword with wrong password fails & leaves stored secret intact.
            let keys_before = Command::new(&restic_bin)
                .args(["-r", &repo_dir.to_string_lossy(), "key", "list", "--json"])
                .env("RESTIC_PASSWORD", "initial-pw")
                .output()
                .expect("restic key list before");
            assert!(keys_before.status.success());

            let bad_pw_err = client
                .update_destination_password(&dest_id, "definitely-wrong-pw")
                .expect_err("UpdateDestinationPassword with wrong pw must fail");
            assert_dbus_error_name(&bad_pw_err, "net.daester.Kestic.Error.WrongPassword");
            assert_eq!(
                secret_store
                    .get_secret(&dest_id, "repo")
                    .expect("get secret")
                    .unwrap()
                    .as_str()
                    .unwrap(),
                "initial-pw"
            );

            // Rotate repository password externally using `restic key passwd`.
            let new_pw_file = root.join("new-pw.txt");
            fs::write(&new_pw_file, "rotated-pw-2026\n").expect("write new pw file");
            let passwd_out = Command::new(&restic_bin)
                .args([
                    "-r",
                    &repo_dir.to_string_lossy(),
                    "key",
                    "passwd",
                    "--new-password-file",
                ])
                .arg(&new_pw_file)
                .env("RESTIC_PASSWORD", "initial-pw")
                .output()
                .expect("restic key passwd");
            assert!(
                passwd_out.status.success(),
                "restic key passwd failed: {}",
                String::from_utf8_lossy(&passwd_out.stderr)
            );

            let keys_after_external = Command::new(&restic_bin)
                .args(["-r", &repo_dir.to_string_lossy(), "key", "list", "--json"])
                .env("RESTIC_PASSWORD", "rotated-pw-2026")
                .output()
                .expect("restic key list after external change");
            assert!(keys_after_external.status.success());

            // AC-M1-20-1 & AC-M1-20-3: UpdateDestinationPassword verifies new password without running `restic key`.
            client
                .update_destination_password(&dest_id, "rotated-pw-2026")
                .expect("UpdateDestinationPassword with valid new pw must succeed");
            assert_eq!(
                secret_store
                    .get_secret(&dest_id, "repo")
                    .expect("get secret")
                    .unwrap()
                    .as_str()
                    .unwrap(),
                "rotated-pw-2026"
            );

            let keys_after_update = Command::new(&restic_bin)
                .args(["-r", &repo_dir.to_string_lossy(), "key", "list", "--json"])
                .env("RESTIC_PASSWORD", "rotated-pw-2026")
                .output()
                .expect("restic key list after UpdateDestinationPassword");
            assert!(keys_after_update.status.success());
            assert_eq!(
                keys_after_external.stdout, keys_after_update.stdout,
                "UpdateDestinationPassword must not modify repository keys"
            );

            client.confirm_quit().expect("confirm quit");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn duplicate_destination_location_rejected_and_edit_reinit_empty_folder() {
    run_isolated_test(
        "duplicate_destination_location_rejected_and_edit_reinit_empty_folder",
        "KESTIC_M1_S09_DUP_REINIT",
        |root| {
            let ready = root.join("daemon.ready");
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let (client, handle) = spawn_daemon_thread(root, ready, restic_bin);
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
            let dest1_id = client
                .add_destination(&dest1, "init", "repo-pw-1", "")
                .expect("AddDestination dest1");

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
                .test_destination_connection(&dup_dest, "connect", "repo-pw-1", "")
                .expect("TestDestinationConnection dup");
            assert!(!test_dup.ok, "duplicate location test must fail");
            assert_eq!(test_dup.error_kind.as_deref(), Some("duplicate_location"));

            let add_dup_err = client
                .add_destination(&dup_dest, "connect", "repo-pw-1", "")
                .expect_err("AddDestination with duplicate location must fail");
            assert_dbus_error_name(&add_dup_err, "net.daester.Kestic.Error.InvalidInput");
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
            let _dest2_id = client
                .add_destination(&dest2, "init", "repo-pw-2", "")
                .expect("AddDestination dest2");

            let mut dest2_collision = dest2.clone();
            dest2_collision.kind = DestinationKind::Local {
                path: PathBuf::from(format!("{}/", repo_dir.display())),
            };
            let upd_dup_err = client
                .update_destination(&dest2_collision, "connect", "", "", true)
                .expect_err("UpdateDestination into dest1 location must fail");
            assert_dbus_error_name(&upd_dup_err, "net.daester.Kestic.Error.InvalidInput");

            // Delete dest1's folder from disk and recreate it empty (simulating Dolphin / rm + mkdir).
            fs::remove_dir_all(&repo_dir).expect("delete repo_dir");
            fs::create_dir_all(&repo_dir).expect("recreate empty repo_dir");

            // Testing existing dest1 in `connect` mode must detect missing repo (`repository_missing`),
            // while testing in `init` mode with empty password uses the stored secret and succeeds.
            let check_missing = client
                .test_destination_connection(&dest1, "connect", "", "")
                .expect("TestDestinationConnection connect on empty dir");
            assert!(!check_missing.ok);
            assert_eq!(
                check_missing.error_kind.as_deref(),
                Some("repository_missing")
            );

            let check_reinit = client
                .test_destination_connection(&dest1, "init", "", "")
                .expect("TestDestinationConnection init on empty dir");
            assert!(
                check_reinit.ok,
                "TestDestinationConnection init should pass on empty folder: {check_reinit:?}"
            );

            // UpdateDestination with mode="init" initializes the repo in the empty folder while
            // keeping DestinationId, stored secrets, and attached backup intact.
            client
                .update_destination(&dest1, "init", "", "", true)
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

            client.confirm_quit().expect("confirm quit");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn unlock_secret_store_over_dbus_clears_blocking_and_emits_status_changed() {
    struct ToggleLockedSecretStore {
        locked: std::sync::atomic::AtomicBool,
        inner: kestic_core::secrets::InMemorySecretStore,
    }

    impl SecretStore for ToggleLockedSecretStore {
        fn set_secret(
            &self,
            target_id: &str,
            kind: &str,
            value: &kestic_core::secrets::SecretValue,
        ) -> Result<(), kestic_core::secrets::SecretError> {
            if self.locked.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(kestic_core::secrets::SecretError::LockedOrPromptRequired(
                    "locked".to_string(),
                ));
            }
            self.inner.set_secret(target_id, kind, value)
        }

        fn get_secret(
            &self,
            target_id: &str,
            kind: &str,
        ) -> Result<Option<kestic_core::secrets::SecretValue>, kestic_core::secrets::SecretError>
        {
            if self.locked.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(kestic_core::secrets::SecretError::LockedOrPromptRequired(
                    "locked".to_string(),
                ));
            }
            self.inner.get_secret(target_id, kind)
        }

        fn delete_secret(
            &self,
            target_id: &str,
            kind: &str,
        ) -> Result<(), kestic_core::secrets::SecretError> {
            self.inner.delete_secret(target_id, kind)
        }

        fn check_availability(&self) -> Result<(), kestic_core::secrets::SecretError> {
            if self.locked.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(kestic_core::secrets::SecretError::LockedOrPromptRequired(
                    "collection locked".to_string(),
                ));
            }
            Ok(())
        }

        fn unlock_interactive(&self) -> Result<(), kestic_core::secrets::SecretError> {
            self.locked
                .store(false, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    run_isolated_test(
        "unlock_secret_store_over_dbus_clears_blocking_and_emits_status_changed",
        "KESTIC_M1_S09_UNLOCK_INNER",
        |root| {
            let (_, restic_bin) = restic_test_binaries()
                .into_iter()
                .next()
                .expect("restic binary");
            let store: Arc<dyn SecretStore + Send + Sync> = Arc::new(ToggleLockedSecretStore {
                locked: std::sync::atomic::AtomicBool::new(true),
                inner: kestic_core::secrets::InMemorySecretStore::new(),
            });
            let ready = root.join("daemon.ready");
            let cfg = DaemonRunConfig {
                bus_name: KESTIC_BUS_NAME.to_string(),
                assume_sni_available: true,
                config_path: Some(root.join("config.toml")),
                state_dir: Some(root.join("state")),
                kestic_binary: Some(find_kestic_binary()),
                secret_store: Some(store),
                ready_file: Some(ready.clone()),
                enable_job_view: false,
                enable_portal_inhibit: false,
                suppress_ui_spawn: true,
                ..Default::default()
            };
            let handle = thread::spawn(move || run_daemon(cfg));
            wait_for_file(&ready, Duration::from_secs(5));

            let signals = KesticSignalCollector::start(
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );
            let client =
                KesticClient::connect(KESTIC_BUS_NAME, KESTIC_OBJECT_PATH).expect("client");
            let status = client
                .set_restic_provider(&ResticProvider::Custom { path: restic_bin })
                .expect("configure restic provider");
            assert!(status.valid, "restic provider invalid: {status:?}");

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

            client.confirm_quit().expect("confirm quit");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}
