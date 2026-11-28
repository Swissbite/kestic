// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::m3::{M3DaemonOpts, connect_local_repo, spawn_m3_daemon, write_incompressible_dataset};
use common::{
    CapturedKesticSignal, KesticSignalCollector, restic_test_binaries, run_isolated_test,
    sandbox_run_config, wait_for_file,
};
use kestic_core::clock::SystemClock;
use kestic_core::history::RunHistory;
use kestic_core::model::{Backup, Destination, DestinationKind, Run, RunStatus, Source};
use kestic_daemon::dbus_api::{KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH};
use kestic_daemon::service::{DaemonCore, DaemonRunConfig, run_daemon};
use kestic_test_support::bus::test_bus_name;
use kestic_test_support::wait::try_poll_every;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

fn run_cancel_check_for_restic(root: &Path, label: &str, restic_bin: &Path) {
    let sub = root.join(label);
    let repo_dir = sub.join("repo");
    let src_dir = sub.join("large-src");

    fs::create_dir_all(&repo_dir).expect("mkdir repo");
    write_incompressible_dataset(&src_dir, 96, 4 * 1024 * 1024);

    let bus_name = test_bus_name("CancelTest_", label);
    let daemon = spawn_m3_daemon(root, M3DaemonOpts::on_request(label, &bus_name, restic_bin));
    let client = &daemon.client;

    let dest = Destination::new(
        format!("Cancel Repo ({label})"),
        DestinationKind::Local {
            path: repo_dir.clone(),
        },
    )
    .expect("valid dest");
    let repo_password = connect_local_repo(client, restic_bin, &dest);

    let backup = Backup::new(
        format!("Large Dataset ({label})"),
        Source::folder(&src_dir),
        dest.id,
    )
    .expect("valid backup");
    let backup_id = client.add_backup(&backup).expect("add backup");

    let signals = KesticSignalCollector::start(&bus_name, KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1);

    let run_id = client.start_backup(&backup_id).expect("StartBackup");

    // Waits briefly for restic progress (at most 250ms, so the cancel catches it mid-flight on
    // fast NVMe); the cancel follows either way.
    try_poll_every(
        Duration::from_millis(250),
        Duration::from_millis(15),
        || {
            signals
                .snapshot()
                .iter()
                .any(|s| matches!(s, CapturedKesticSignal::RunProgress(p) if p.run_id == run_id))
                .then_some(())
        },
    );

    // AC-M1-10-1: CancelRun sends SIGTERM and run finishes as Cancelled.
    client.cancel_run(&run_id).expect("CancelRun");

    let captured = signals.wait_until(Duration::from_secs(15), |sigs| {
        sigs.iter().any(|s| {
            if let CapturedKesticSignal::RunUpdated(json) = s
                && let Ok(run) = serde_json::from_str::<Run>(json)
            {
                return run.id.as_str() == run_id && run.status != RunStatus::Running;
            }
            false
        })
    });

    let final_run = captured
        .iter()
        .rev()
        .find_map(|s| {
            if let CapturedKesticSignal::RunUpdated(json) = s
                && let Ok(run) = serde_json::from_str::<Run>(json)
                && run.id.as_str() == run_id
                && run.status != RunStatus::Running
            {
                return Some(run);
            }
            None
        })
        .expect("final RunUpdated");
    assert_eq!(
        final_run.status,
        RunStatus::Cancelled,
        "[{label}] expected Cancelled status after CancelRun, got {:?}",
        final_run.status
    );

    let detail = client.get_run_detail(&run_id).expect("GetRunDetail");
    assert_eq!(detail.run.status, RunStatus::Cancelled);

    // AC-M1-10-3: SIGTERM causes restic to remove its lock cleanly (0 locks left).
    let locks_out = Command::new(restic_bin)
        .args([
            "-r",
            &repo_dir.to_string_lossy(),
            "list",
            "locks",
            "--no-lock",
        ])
        .env("RESTIC_PASSWORD", &repo_password)
        .env("RESTIC_CACHE_DIR", sub.join("restic-cache"))
        .output()
        .expect("restic list locks");
    assert!(
        locks_out.status.success(),
        "[{label}] restic list locks failed: {}",
        String::from_utf8_lossy(&locks_out.stderr)
    );
    let lock_lines: Vec<&str> = std::str::from_utf8(&locks_out.stdout)
        .expect("utf8 locks")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert!(
        lock_lines.is_empty(),
        "[{label}] expected 0 locks after SIGTERM cancel, found: {lock_lines:?}"
    );

    daemon.stop();

    // Clean up heavy dataset inside sub dir right away.
    let _ = fs::remove_dir_all(&sub);
}

#[test]
fn ac_m1_10_1_and_10_3_cancel_run_sends_sigterm_and_leaves_zero_locks_dual_restic() {
    run_isolated_test(
        "ac_m1_10_1_and_10_3_cancel_run_sends_sigterm_and_leaves_zero_locks_dual_restic",
        "KESTIC_M1_S10_CANCEL",
        |root| {
            for (label, restic_bin) in restic_test_binaries() {
                run_cancel_check_for_restic(root, label, &restic_bin);
            }
        },
    );
}

#[test]
fn ac_m1_10_2_preseeded_running_run_transitions_to_interrupted_on_daemon_startup() {
    run_isolated_test(
        "ac_m1_10_2_preseeded_running_run_transitions_to_interrupted_on_daemon_startup",
        "KESTIC_M1_S10_RECOVER",
        |root| {
            let state_dir = root.join("state").join("kestic");
            let _cfg_path = root.join("cfg").join("kestic").join("config.toml");

            let dest = Destination::new(
                "Local Repo",
                DestinationKind::Local {
                    path: root.join("repo"),
                },
            )
            .expect("dest");
            let backup = Backup::new("Crash Backup", Source::folder(root.join("src")), dest.id)
                .expect("backup");
            let clock = SystemClock;
            let stale_run = Run::new_backup(&clock, &backup).expect("new running run");
            assert_eq!(stale_run.status, RunStatus::Running);

            {
                let mut hist = RunHistory::open(&state_dir).expect("open RunHistory");
                hist.upsert_run(&stale_run, &[]).expect("seed running run");
            }

            let run_cfg = DaemonRunConfig {
                ready_file: None,
                ..sandbox_run_config(root)
            };

            // Verify via DaemonCore::new directly.
            let core = DaemonCore::new(&run_cfg).expect("DaemonCore::new");
            let recovered = core
                .history()
                .get_run(&stale_run.id)
                .expect("get recovered run")
                .expect("run must exist");
            assert_eq!(recovered.status, RunStatus::Interrupted);
            assert!(recovered.finished_at.is_some());
            drop(core);

            // Also verify full run_daemon startup path exposes Interrupted via D-Bus GetRunDetail.
            let mut stale_run_2 = Run::new_backup(&clock, &backup).expect("second stale run");
            stale_run_2.status = RunStatus::Running;
            {
                let mut hist = RunHistory::open(&state_dir).expect("open RunHistory");
                hist.upsert_run(&stale_run_2, &[])
                    .expect("seed second running run");
            }

            let ready_file = root.join("daemon.ready");
            let mut daemon_cfg = run_cfg;
            daemon_cfg.ready_file = Some(ready_file.clone());
            let handle = thread::spawn(move || run_daemon(&daemon_cfg));
            wait_for_file(&ready_file, common::DAEMON_READY_TIMEOUT);

            let client = common::connect_default_client().expect("connect client");
            let detail = client
                .get_run_detail(stale_run_2.id.as_str())
                .expect("GetRunDetail");
            assert_eq!(detail.run.status, RunStatus::Interrupted);
            assert!(detail.run.finished_at.is_some());

            common::stop_daemon(&client, handle);
        },
    );
}
