// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::{
    CapturedKesticSignal, KesticSignalCollector, click_dbusmenu_item, fetch_dbusmenu_items,
    find_kestic_binary, read_sni_string_prop, read_sni_tooltip, restic_test_binaries,
    run_isolated_test, start_fake_notifications, start_fake_sni_watcher,
    wait_for_bus_name_released, wait_for_file, wait_for_sni_service, wait_for_sni_service_at,
};
use kestic_core::clock::SystemClock;
use kestic_core::config::{AppConfig, save_config};
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, Destination, DestinationKind, ResticProvider, Run, RunStatus, Schedule, Source,
};
use kestic_core::secrets::FileSecretStore;
use kestic_daemon::dbus_api::{
    KESTIC_BUS_NAME, KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH, KesticClient,
};
use kestic_daemon::notifications::KESTIC_DESKTOP_ENTRY;
use kestic_daemon::service::{DaemonOutcome, DaemonRunConfig, run_daemon};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use zbus::{Connection, block_on};

fn write_large_dataset(dir: &Path, num_files: usize, bytes_per_file: usize) {
    fs::create_dir_all(dir).expect("mkdir dataset");
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut buf = vec![0u8; bytes_per_file];
    for idx in 0..num_files {
        for chunk in buf.chunks_exact_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            chunk.copy_from_slice(&state.to_le_bytes());
        }
        let mut f = fs::File::create(dir.join(format!("data_{idx:03}.bin"))).expect("create file");
        f.write_all(&buf).expect("write file");
    }
}

fn assert_sni_matches(
    client_conn: &Connection,
    sni_service: &str,
    expected_status: &str,
    expected_overlay: &str,
    label: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let status = read_sni_string_prop(client_conn, sni_service, "Status");
        let overlay = read_sni_string_prop(client_conn, sni_service, "OverlayIconName");
        if status == expected_status && overlay == expected_overlay {
            let (title, desc) = read_sni_tooltip(client_conn, sni_service);
            assert_eq!(title, "Kestic", "[{label}] ToolTip title");
            assert!(!desc.is_empty(), "[{label}] ToolTip description empty");
            return;
        }
        if Instant::now() >= deadline {
            panic!(
                "[{label}] SNI mismatch: expected (status={expected_status:?}, overlay={expected_overlay:?}), \
                 got (status={status:?}, overlay={overlay:?})"
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
}

struct SniTestCtx<'a> {
    root: &'a Path,
    watcher_conn: &'a Connection,
    client_conn: &'a Connection,
    restic_bin: &'a Path,
    kestic_bin: &'a Path,
    sni_idx: usize,
}

fn assert_real_daemon_sni_state(
    ctx: &mut SniTestCtx<'_>,
    sub_name: &str,
    cfg: AppConfig,
    seeded_status: Option<(RunStatus, Option<i32>)>,
    expected_status: &str,
    expected_overlay: &str,
) {
    let sub = ctx.root.join(sub_name);
    let cfg_path = sub.join("config.toml");
    let state_dir = sub.join("state");
    let secret_file = sub.join("secrets.hex");
    let ready_file = sub.join("daemon.ready");
    fs::create_dir_all(&state_dir).expect("mkdir state");

    save_config(&cfg_path, &cfg).expect("save config");
    if let Some((st, exit_code)) = seeded_status {
        let clock = SystemClock;
        let mut run = Run::new_backup(&clock, &cfg.backups[0]).expect("new run");
        run.finish(&clock, st, exit_code);
        let mut hist = RunHistory::open(&state_dir).expect("open history");
        hist.upsert_run(&run, &[]).expect("seed run");
    }

    let run_cfg = DaemonRunConfig {
        bus_name: KESTIC_BUS_NAME.to_string(),
        assume_sni_available: false,
        ready_file: Some(ready_file.clone()),
        config_path: Some(cfg_path),
        state_dir: Some(state_dir),
        kestic_binary: Some(ctx.kestic_bin.to_path_buf()),
        secret_store: Some(Arc::new(FileSecretStore::new(secret_file))),
        suppress_ui_spawn: true,
        enable_job_view: false,
        enable_portal_inhibit: false,
        ..Default::default()
    };

    let handle = thread::spawn(move || run_daemon(run_cfg));
    wait_for_file(&ready_file, common::DAEMON_READY_TIMEOUT);
    let sni_svc = wait_for_sni_service_at(ctx.watcher_conn, ctx.sni_idx, Duration::from_secs(5));
    ctx.sni_idx += 1;

    assert_sni_matches(
        ctx.client_conn,
        &sni_svc,
        expected_status,
        expected_overlay,
        sub_name,
    );

    let client = KesticClient::connect_default().expect("connect client");
    client.confirm_quit().expect("confirm quit");
    assert_eq!(
        handle.join().expect("join daemon"),
        Ok(DaemonOutcome::RanAndStopped)
    );
    wait_for_bus_name_released(ctx.client_conn, KESTIC_BUS_NAME, Duration::from_secs(3));
}

fn assert_running_backup_sni_state(ctx: &mut SniTestCtx<'_>) {
    let sub = ctx.root.join("state_running");
    let cfg_path = sub.join("config.toml");
    let state_dir = sub.join("state");
    let secret_file = ctx.root.join("secrets.hex");
    let ready_file = sub.join("daemon.ready");
    let run_repo = sub.join("repo");
    let run_src = sub.join("large-src");
    fs::create_dir_all(&run_repo).expect("mkdir run repo");
    write_large_dataset(&run_src, 36, 4 * 1024 * 1024);

    let run_cfg = DaemonRunConfig {
        bus_name: KESTIC_BUS_NAME.to_string(),
        assume_sni_available: false,
        ready_file: Some(ready_file.clone()),
        config_path: Some(cfg_path),
        state_dir: Some(state_dir),
        kestic_binary: Some(ctx.kestic_bin.to_path_buf()),
        secret_store: Some(Arc::new(FileSecretStore::new(secret_file))),
        suppress_ui_spawn: true,
        enable_job_view: false,
        enable_portal_inhibit: false,
        ..Default::default()
    };
    let handle = thread::spawn(move || run_daemon(run_cfg));
    wait_for_file(&ready_file, common::DAEMON_READY_TIMEOUT);
    let sni_svc = wait_for_sni_service_at(ctx.watcher_conn, ctx.sni_idx, Duration::from_secs(5));
    ctx.sni_idx += 1;

    let client = KesticClient::connect_default().expect("connect client");
    client
        .set_restic_provider(&ResticProvider::Custom {
            path: ctx.restic_bin.to_path_buf(),
        })
        .expect("set provider");
    let r_dest =
        Destination::new("Run Repo", DestinationKind::Local { path: run_repo }).expect("dest");
    client
        .add_destination(&r_dest, "init", "run-pw-1", "")
        .expect("add dest");
    let r_backup = Backup::new("Live Run", Source::folder(&run_src), r_dest.id).expect("backup");
    let b_id = client.add_backup(&r_backup).expect("add backup");

    let signals =
        KesticSignalCollector::start(KESTIC_BUS_NAME, KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1);
    let run_id = client.start_backup(&b_id).expect("StartBackup");
    assert_sni_matches(ctx.client_conn, &sni_svc, "Active", "state-sync", "running");

    client.cancel_run(&run_id).expect("CancelRun");
    signals.wait_until(Duration::from_secs(15), |sigs| {
        sigs.iter().any(|s| {
            if let CapturedKesticSignal::RunUpdated(json) = s
                && let Ok(run) = serde_json::from_str::<Run>(json)
            {
                return run.id.as_str() == run_id && run.status != RunStatus::Running;
            }
            false
        })
    });
    client.confirm_quit().expect("confirm quit");
    assert_eq!(
        handle.join().expect("join daemon"),
        Ok(DaemonOutcome::RanAndStopped)
    );
    wait_for_bus_name_released(ctx.client_conn, KESTIC_BUS_NAME, Duration::from_secs(3));
    let _ = fs::remove_dir_all(&sub);
}

fn assert_simulated_sni_matrix(ctx: &mut SniTestCtx<'_>) {
    let sim_cases = [
        ("overdue", "NeedsAttention", "alarm-symbolic"),
        ("paused", "Active", "state-pause"),
        ("waiting", "Passive", ""),
        ("paused_waiting", "Active", "state-pause"),
        ("maintenance", "Active", "state-sync"),
        ("restore", "Active", "state-sync"),
    ];

    for (sim_arg, exp_status, exp_overlay) in sim_cases {
        let sub = ctx.root.join(format!("sim_{sim_arg}"));
        let cfg_path = sub.join("config.toml");
        let state_dir = sub.join("state");
        let ready_file = sub.join("daemon.ready");
        fs::create_dir_all(&state_dir).expect("mkdir state");
        save_config(
            &cfg_path,
            &AppConfig {
                restic_provider: ResticProvider::Custom {
                    path: ctx.restic_bin.to_path_buf(),
                },
                ..Default::default()
            },
        )
        .expect("save sim config");

        let child = Command::new(ctx.kestic_bin)
            .args([
                "daemon",
                "--require-sni",
                "--no-job-view",
                "--no-portal-inhibit",
                "--suppress-ui-spawn",
                "--simulate-status",
                sim_arg,
                "--config-path",
            ])
            .arg(&cfg_path)
            .args(["--state-dir"])
            .arg(&state_dir)
            .args(["--ready-file"])
            .arg(&ready_file)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn simulated kestic daemon");

        wait_for_file(&ready_file, common::DAEMON_READY_TIMEOUT);
        let sni_svc =
            wait_for_sni_service_at(ctx.watcher_conn, ctx.sni_idx, Duration::from_secs(5));
        ctx.sni_idx += 1;

        assert_sni_matches(
            ctx.client_conn,
            &sni_svc,
            exp_status,
            exp_overlay,
            &format!("simulate-status={sim_arg}"),
        );

        let client = KesticClient::connect_default().expect("connect client");
        client.confirm_quit().expect("confirm quit");
        let out = child.wait_with_output().expect("wait simulated daemon");
        assert!(
            out.status.success(),
            "simulated daemon ({sim_arg}) failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        wait_for_bus_name_released(ctx.client_conn, KESTIC_BUS_NAME, Duration::from_secs(3));
    }
}

#[test]
fn ac_m1_11_1_full_section_10_1_sni_matrix_including_simulated_statuses() {
    run_isolated_test(
        "ac_m1_11_1_full_section_10_1_sni_matrix_including_simulated_statuses",
        "KESTIC_M1_S11_MATRIX",
        |root| {
            let watcher_conn = start_fake_sni_watcher();
            let client_conn = block_on(Connection::session()).expect("session conn");
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let kestic_bin = find_kestic_binary();

            let mut ctx = SniTestCtx {
                root,
                watcher_conn: &watcher_conn,
                client_conn: &client_conn,
                restic_bin: &restic_bin,
                kestic_bin: &kestic_bin,
                sni_idx: 0,
            };

            let repo_dir = root.join("repo-matrix");
            let src_dir = root.join("src-matrix");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");

            let dest = Destination::new(
                "Local Matrix",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");

            let mut hourly_backup =
                Backup::new("Hourly Docs", Source::folder(&src_dir), dest.id.clone())
                    .expect("backup");
            hourly_backup.schedule = Schedule::Daily { hour: 2, minute: 0 };

            let mut manual_backup =
                Backup::new("Manual Docs", Source::folder(&src_dir), dest.id.clone())
                    .expect("manual backup");
            manual_backup.schedule = Schedule::ManualOnly;

            let valid_provider = ResticProvider::Custom {
                path: restic_bin.clone(),
            };

            let real_cases = [
                (
                    "state_ok",
                    valid_provider.clone(),
                    hourly_backup.clone(),
                    Some((RunStatus::Ok, Some(0))),
                    "Passive",
                    "",
                ),
                (
                    "state_never_run",
                    valid_provider.clone(),
                    hourly_backup.clone(),
                    None,
                    "Passive",
                    "",
                ),
                (
                    "state_manual_only",
                    valid_provider.clone(),
                    manual_backup,
                    Some((RunStatus::Ok, Some(0))),
                    "Passive",
                    "",
                ),
                (
                    "state_warning",
                    valid_provider.clone(),
                    hourly_backup.clone(),
                    Some((RunStatus::Warning, Some(3))),
                    "Active",
                    "state-warning",
                ),
                (
                    "state_error",
                    valid_provider,
                    hourly_backup.clone(),
                    Some((RunStatus::Error, Some(1))),
                    "NeedsAttention",
                    "state-error",
                ),
                (
                    "state_blocked",
                    ResticProvider::Custom {
                        path: root.join("nonexistent-restic-bin"),
                    },
                    hourly_backup,
                    None,
                    "NeedsAttention",
                    "state-error",
                ),
            ];

            for (name, provider, backup, seeded, exp_status, exp_overlay) in real_cases {
                assert_real_daemon_sni_state(
                    &mut ctx,
                    name,
                    AppConfig {
                        restic_provider: provider,
                        destinations: vec![dest.clone()],
                        backups: vec![backup],
                        ..Default::default()
                    },
                    seeded,
                    exp_status,
                    exp_overlay,
                );
            }

            // 7. running (real live backup in progress)
            assert_running_backup_sni_state(&mut ctx);

            // 8-13. CLI `--simulate-status` matrix (overdue, paused, waiting, paused_waiting, maintenance, restore).
            assert_simulated_sni_matrix(&mut ctx);
        },
    );
}

#[test]
fn ac_m1_11_2_11_3_11_5_dbusmenu_items_quit_confirmation_and_active_progress() {
    run_isolated_test(
        "ac_m1_11_2_11_3_11_5_dbusmenu_items_quit_confirmation_and_active_progress",
        "KESTIC_M1_S11_MENU_QUIT",
        |root| {
            let watcher_conn = start_fake_sni_watcher();
            let client_conn = block_on(Connection::session()).expect("session conn");
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");

            let repo_dir = root.join("repo-menu");
            let src_dir = root.join("src-menu");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            write_large_dataset(&src_dir, 40, 4 * 1024 * 1024);

            let ready_file = root.join("daemon.ready");
            let run_cfg = DaemonRunConfig {
                bus_name: KESTIC_BUS_NAME.to_string(),
                assume_sni_available: false,
                ready_file: Some(ready_file.clone()),
                config_path: Some(root.join("cfg").join("kestic").join("config.toml")),
                state_dir: Some(root.join("state").join("kestic")),
                kestic_binary: Some(find_kestic_binary()),
                secret_store: Some(Arc::new(FileSecretStore::new(root.join("secrets.hex")))),
                suppress_ui_spawn: true,
                enable_job_view: false,
                enable_portal_inhibit: false,
                ..Default::default()
            };

            let handle = thread::spawn(move || run_daemon(run_cfg));
            wait_for_file(&ready_file, common::DAEMON_READY_TIMEOUT);
            let sni_svc = wait_for_sni_service(&watcher_conn, Duration::from_secs(5));

            let client = KesticClient::connect_default().expect("connect client");
            client
                .set_restic_provider(&ResticProvider::Custom { path: restic_bin })
                .expect("set provider");

            let dest = Destination::new("Menu Repo", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            client
                .add_destination(&dest, "init", "menu-pw-1", "")
                .expect("add dest");
            let backup =
                Backup::new("Menu Backup", Source::folder(&src_dir), dest.id).expect("backup");
            let backup_id = client.add_backup(&backup).expect("add backup");

            // AC-M1-11-2: Idle dbusmenu layout checks.
            let idle_items = fetch_dbusmenu_items(&client_conn, &sni_svc);
            let labels: Vec<&str> = idle_items.iter().map(|i| i.label.as_str()).collect();
            assert!(
                labels.contains(&"Back Up Now"),
                "missing 'Back Up Now' in {labels:?}"
            );
            let cancel_item = idle_items
                .iter()
                .find(|i| i.label == "Cancel Running Backup")
                .expect("missing 'Cancel Running Backup'");
            assert!(!cancel_item.enabled, "Cancel must be disabled when idle");
            assert!(
                labels.contains(&"Open Kestic"),
                "missing 'Open Kestic' in {labels:?}"
            );
            assert!(
                labels.contains(&"Quit Kestic…"),
                "missing 'Quit Kestic…' in {labels:?}"
            );

            assert!(
                labels.contains(&"Pause Schedules"),
                "missing 'Pause Schedules' in {labels:?}"
            );

            for forbidden in ["Restore Files…", "Create Recovery Kit…", "Update restic…"] {
                assert!(
                    !labels.iter().any(|l| l.contains(forbidden)),
                    "M4-M6 item `{forbidden}` must not exist yet in M3 tray menu: {labels:?}"
                );
            }

            // AC-M1-11-5: Active backup shows active progress line + enabled Cancel in tray menu.
            let signals = KesticSignalCollector::start(
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );
            let run_id = client.start_backup(&backup_id).expect("StartBackup");

            let active_items = fetch_dbusmenu_items(&client_conn, &sni_svc);
            assert!(
                active_items
                    .iter()
                    .any(|i| i.label.contains("Menu Backup: backing up")),
                "expected active progress line in tray menu, got: {active_items:?}"
            );
            let active_cancel = active_items
                .iter()
                .find(|i| i.label == "Cancel Running Backup")
                .expect("cancel item");
            assert!(
                active_cancel.enabled,
                "Cancel Running Backup must be enabled during active backup"
            );

            // AC-M1-11-3: Clicking Quit triggers QuitConfirmationRequested WITHOUT terminating daemon.
            let quit_item = active_items
                .iter()
                .find(|i| i.label == "Quit Kestic…")
                .expect("quit item");
            signals.clear();
            click_dbusmenu_item(&client_conn, &sni_svc, quit_item.id);

            signals.wait_until(Duration::from_secs(3), |sigs| {
                sigs.iter().any(|s| {
                    matches!(
                        s,
                        CapturedKesticSignal::QuitConfirmationRequested {
                            has_running_run: true,
                            running_backup_name,
                        } if running_backup_name == "Menu Backup"
                    )
                })
            });

            thread::sleep(Duration::from_millis(150));
            assert!(
                client.is_bus_owned().expect("is_bus_owned"),
                "daemon must stay alive until ConfirmQuit is called"
            );
            assert!(
                client.daemon_version().is_ok(),
                "daemon D-Bus API must remain responsive before ConfirmQuit"
            );

            client.cancel_run(&run_id).expect("CancelRun");
            signals.wait_until(Duration::from_secs(15), |sigs| {
                sigs.iter().any(|s| {
                    if let CapturedKesticSignal::RunUpdated(json) = s
                        && let Ok(run) = serde_json::from_str::<Run>(json)
                    {
                        return run.id.as_str() == run_id && run.status != RunStatus::Running;
                    }
                    false
                })
            });

            client.confirm_quit().expect("ConfirmQuit");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn ac_m1_11_4_notifications_only_on_error_and_warning_with_desktop_entry() {
    run_isolated_test(
        "ac_m1_11_4_notifications_only_on_error_and_warning_with_desktop_entry",
        "KESTIC_M1_S11_NOTIFY",
        |root| {
            let (_notif_conn, notif_capture) = start_fake_notifications();
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");

            let ready_file = root.join("daemon.ready");
            let run_cfg = DaemonRunConfig {
                bus_name: KESTIC_BUS_NAME.to_string(),
                assume_sni_available: true,
                ready_file: Some(ready_file.clone()),
                config_path: Some(root.join("cfg").join("kestic").join("config.toml")),
                state_dir: Some(root.join("state").join("kestic")),
                kestic_binary: Some(find_kestic_binary()),
                secret_store: Some(Arc::new(FileSecretStore::new(root.join("secrets.hex")))),
                suppress_ui_spawn: true,
                enable_job_view: false,
                enable_portal_inhibit: false,
                ..Default::default()
            };

            let handle = thread::spawn(move || run_daemon(run_cfg));
            wait_for_file(&ready_file, common::DAEMON_READY_TIMEOUT);

            let client = KesticClient::connect_default().expect("connect client");
            client
                .set_restic_provider(&ResticProvider::Custom { path: restic_bin })
                .expect("set provider");

            let signals = KesticSignalCollector::start(
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );

            let wait_run_finish = |run_id: &str| -> Run {
                let sigs = signals.wait_until(Duration::from_secs(20), |sigs| {
                    sigs.iter().any(|s| {
                        if let CapturedKesticSignal::RunUpdated(json) = s
                            && let Ok(run) = serde_json::from_str::<Run>(json)
                        {
                            return run.id.as_str() == run_id && run.status != RunStatus::Running;
                        }
                        false
                    })
                });
                sigs.into_iter()
                    .rev()
                    .find_map(|s| {
                        if let CapturedKesticSignal::RunUpdated(json) = s
                            && let Ok(run) = serde_json::from_str::<Run>(&json)
                            && run.id.as_str() == run_id
                            && run.status != RunStatus::Running
                        {
                            return Some(run);
                        }
                        None
                    })
                    .expect("final run")
            };

            // 1. RunStatus::Ok -> ZERO notifications.
            let ok_repo = root.join("repo-ok");
            let ok_src = root.join("src-ok");
            fs::create_dir_all(&ok_repo).expect("mkdir ok repo");
            fs::create_dir_all(&ok_src).expect("mkdir ok src");
            fs::write(ok_src.join("clean.txt"), b"clean\n").expect("write clean");

            let ok_dest = Destination::new("OK Repo", DestinationKind::Local { path: ok_repo })
                .expect("dest");
            client
                .add_destination(&ok_dest, "init", "ok-pw", "")
                .expect("add ok dest");
            let ok_backup =
                Backup::new("OK Backup", Source::folder(&ok_src), ok_dest.id).expect("backup");
            let ok_bid = client.add_backup(&ok_backup).expect("add ok backup");

            notif_capture.clear();
            signals.clear();
            let ok_rid = client.start_backup(&ok_bid).expect("start ok backup");
            let ok_run = wait_run_finish(&ok_rid);
            assert_eq!(ok_run.status, RunStatus::Ok);
            thread::sleep(Duration::from_millis(80));
            assert!(
                notif_capture.snapshot().is_empty(),
                "RunStatus::Ok must emit NO notification"
            );

            // 2. RunStatus::Warning (Exit 3 via unreadable file) -> BackupWarning notification.
            let warn_src = root.join("src-warn");
            fs::create_dir_all(&warn_src).expect("mkdir warn src");
            fs::write(warn_src.join("readable.txt"), b"ok\n").expect("write readable");
            let secret_unreadable = warn_src.join("unreadable.txt");
            fs::write(&secret_unreadable, b"locked\n").expect("write unreadable");
            fs::set_permissions(&secret_unreadable, fs::Permissions::from_mode(0o000))
                .expect("chmod 000");

            let warn_backup = Backup::new(
                "Warn Backup",
                Source::folder(&warn_src),
                ok_backup.destination_id,
            )
            .expect("warn backup");
            let warn_bid = client.add_backup(&warn_backup).expect("add warn backup");

            notif_capture.clear();
            signals.clear();
            let warn_rid = client.start_backup(&warn_bid).expect("start warn backup");
            let warn_run = wait_run_finish(&warn_rid);
            assert_eq!(warn_run.status, RunStatus::Warning);

            let warn_notifs = notif_capture.snapshot();
            assert_eq!(
                warn_notifs.len(),
                1,
                "expected 1 BackupWarning notification"
            );
            assert_eq!(
                warn_notifs[0].desktop_entry.as_deref(),
                Some(KESTIC_DESKTOP_ENTRY)
            );
            assert_eq!(warn_notifs[0].event_id.as_deref(), Some("BackupWarning"));
            let _ = fs::set_permissions(&secret_unreadable, fs::Permissions::from_mode(0o600));

            // 3. RunStatus::Error (corrupted/missing restic config inside existing repo dir) -> BackupFailed notification.
            let err_repo = root.join("repo-err");
            fs::create_dir_all(&err_repo).expect("mkdir err repo");
            let err_dest = Destination::new(
                "Broken Repo",
                DestinationKind::Local {
                    path: err_repo.clone(),
                },
            )
            .expect("err dest");
            client
                .add_destination(&err_dest, "init", "err-pw", "")
                .expect("add err dest");
            // Delete repo `config` file after init so directory exists (preflight passes)
            // while `restic backup` fails with exit 10 (`RepositoryMissing`).
            fs::remove_file(err_repo.join("config")).expect("remove repo config");

            let err_backup = Backup::new("Error Backup", Source::folder(&ok_src), err_dest.id)
                .expect("err backup");
            let err_bid = client.add_backup(&err_backup).expect("add err backup");

            notif_capture.clear();
            signals.clear();
            let err_rid = client.start_backup(&err_bid).expect("start err backup");
            let err_run = wait_run_finish(&err_rid);
            assert_eq!(err_run.status, RunStatus::Error);

            let err_notifs = notif_capture.snapshot();
            assert_eq!(err_notifs.len(), 1, "expected 1 BackupFailed notification");
            assert_eq!(
                err_notifs[0].desktop_entry.as_deref(),
                Some(KESTIC_DESKTOP_ENTRY)
            );
            assert_eq!(err_notifs[0].event_id.as_deref(), Some("BackupFailed"));

            client.confirm_quit().expect("confirm quit");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn tray_open_and_quit_respect_live_gui_registration_and_recover_from_stale_window_active() {
    run_isolated_test(
        "tray_open_and_quit_respect_live_gui_registration_and_recover_from_stale_window_active",
        "KESTIC_M1_S11_TRAY_SPAWN",
        |root| {
            let watcher_conn = start_fake_sni_watcher();
            let client_conn = block_on(Connection::session()).expect("session conn");

            let spawn_log = root.join("gui-spawns.log");
            let fake_gui_bin = root.join("fake-kestic-gui");
            let script = format!(
                "#!/bin/sh\nprintf 'ARG:%s\\n' \"$*\" >> \"{}\"\n",
                spawn_log.display()
            );
            fs::write(&fake_gui_bin, script).expect("write fake gui binary");
            fs::set_permissions(&fake_gui_bin, fs::Permissions::from_mode(0o755))
                .expect("chmod fake gui binary");

            let wait_for_log_lines = |expected_lines: usize| -> Vec<String> {
                let deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    let content = fs::read_to_string(&spawn_log).unwrap_or_default();
                    let lines: Vec<String> = content.lines().map(ToOwned::to_owned).collect();
                    if lines.len() >= expected_lines {
                        return lines;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "timed out waiting for {expected_lines} fake GUI spawn(s), saw {lines:?}"
                    );
                    thread::sleep(Duration::from_millis(20));
                }
            };

            let ready_file = root.join("daemon.ready");
            let run_cfg = DaemonRunConfig {
                bus_name: KESTIC_BUS_NAME.to_string(),
                assume_sni_available: false,
                ready_file: Some(ready_file.clone()),
                config_path: Some(root.join("cfg").join("kestic").join("config.toml")),
                state_dir: Some(root.join("state").join("kestic")),
                kestic_binary: Some(fake_gui_bin),
                secret_store: Some(Arc::new(FileSecretStore::new(root.join("secrets.hex")))),
                suppress_ui_spawn: false,
                enable_job_view: false,
                enable_portal_inhibit: false,
                ..Default::default()
            };

            let handle = thread::spawn(move || run_daemon(run_cfg));
            wait_for_file(&ready_file, common::DAEMON_READY_TIMEOUT);
            let sni_svc = wait_for_sni_service(&watcher_conn, Duration::from_secs(5));

            let client = KesticClient::connect_default().expect("connect client");
            let signals = KesticSignalCollector::start(
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );

            let items = fetch_dbusmenu_items(&client_conn, &sni_svc);
            let open_id = items
                .iter()
                .find(|i| i.label == "Open Kestic")
                .expect("Open Kestic tray entry")
                .id;
            let quit_id = items
                .iter()
                .find(|i| i.label == "Quit Kestic…")
                .expect("Quit Kestic… tray entry")
                .id;

            // Case 1: Stale `window_active(true)` without an active `RegisterGui` connection must
            // still spawn GUI for both Open and Quit (--confirm-quit).
            client
                .window_active(true, "overview")
                .expect("stale window_active(true)");

            click_dbusmenu_item(&client_conn, &sni_svc, open_id);
            let lines1 = wait_for_log_lines(1);
            assert_eq!(lines1[0], "ARG:");

            click_dbusmenu_item(&client_conn, &sni_svc, quit_id);
            let lines2 = wait_for_log_lines(2);
            assert_eq!(lines2[1], "ARG:--confirm-quit");

            // Case 2: Persistent registered GUI connection suppresses duplicate spawns even when
            // `window_active` is false (e.g. unfocused window) while emitting D-Bus signals.
            let gui_conn = block_on(Connection::session()).expect("persistent gui session conn");
            let gui_unique_name = gui_conn
                .unique_name()
                .expect("gui unique name")
                .as_str()
                .to_string();
            let gui_client = KesticClient::from_connection(
                gui_conn.clone(),
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
            );
            gui_client.register_gui().expect("RegisterGui");
            gui_client
                .window_active(false, "")
                .expect("window_active(false)");

            fs::write(&spawn_log, "").expect("truncate spawn log");
            signals.clear();

            click_dbusmenu_item(&client_conn, &sni_svc, open_id);
            signals.wait_until(Duration::from_secs(3), |sigs| {
                sigs.iter()
                    .any(|s| matches!(s, CapturedKesticSignal::ShowWindowRequested))
            });

            signals.clear();
            click_dbusmenu_item(&client_conn, &sni_svc, quit_id);
            signals.wait_until(Duration::from_secs(3), |sigs| {
                sigs.iter()
                    .any(|s| matches!(s, CapturedKesticSignal::QuitConfirmationRequested { .. }))
            });

            thread::sleep(Duration::from_millis(120));
            let suppressed_log = fs::read_to_string(&spawn_log).unwrap_or_default();
            assert!(
                suppressed_log.trim().is_empty(),
                "must not spawn duplicate GUI while registered GUI connection is alive; saw: {suppressed_log:?}"
            );

            // Case 3: Dropping the registered GUI connection makes `NameHasOwner` false so the
            // next tray Open spawns a fresh GUI instance cleanly.
            drop(gui_client);
            drop(gui_conn);
            wait_for_bus_name_released(&client_conn, &gui_unique_name, Duration::from_secs(3));

            click_dbusmenu_item(&client_conn, &sni_svc, open_id);
            let lines3 = wait_for_log_lines(1);
            assert_eq!(lines3[0], "ARG:");

            client.confirm_quit().expect("confirm quit");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}
