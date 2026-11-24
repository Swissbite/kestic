// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S05 tray entry «Restore Files…»: enabled with a destination, disabled with its reason
//! without one, and opening the main window on the tab «Restore».

mod common;

use common::m3::{StdFakes, connect_local_repo, latest_variant};
use common::m4::{RawSnapshot, small_tree, write_snapshot};
use common::m4_restore::RestoreFixture;
use common::{
    CapturedKesticSignal, KesticSignalCollector, TrayMenuEntry, click_dbusmenu_item,
    fetch_dbusmenu_items, restic_test_binaries, run_isolated_test, sandbox_run_config,
    start_fake_sni_watcher, wait_for_file, wait_for_sni_service, wait_for_sni_service_at,
};
use kestic_core::model::{Destination, DestinationKind, ResticProvider};
use kestic_daemon::dbus_api::{
    KESTIC_BUS_NAME, KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH, RestoreOverwrite, RestoreTarget,
};
use kestic_daemon::service::{DaemonRunConfig, run_daemon};
use kestic_test_support::tools::write_executable;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use zbus::{Connection, block_on};

fn restore_entry(items: &[TrayMenuEntry]) -> TrayMenuEntry {
    items
        .iter()
        .find(|i| i.label.starts_with("Restore Files…"))
        .cloned()
        .unwrap_or_else(|| panic!("no «Restore Files…» entry: {items:?}"))
}

/// AC-M4-05-24: the tray entry «Restore Files…» is (a) enabled with a destination that holds
/// only snapshots without a Kestic tag, (b) disabled with its reason without any destination,
/// and (c) a click opens the main window on the tab «Restore» (`ShowWindowRequested`, then
/// `ShowRestoreRequested`; without a GUI it starts one with `--show-restore`).
#[test]
fn ac_m4_05_24_tray_entry_restore_files() {
    run_isolated_test(
        "ac_m4_05_24_tray_entry_restore_files",
        "KESTIC_AC_M4_05_24_INNER",
        |root| {
            let watcher_conn = start_fake_sni_watcher();
            let client_conn = block_on(Connection::session()).expect("session conn");
            let (_, restic) = restic_test_binaries().pop().expect("restic binary");

            let handle = thread::spawn({
                let cfg = DaemonRunConfig {
                    assume_sni_available: false,
                    ..sandbox_run_config(root)
                };
                move || run_daemon(cfg)
            });
            wait_for_file(&root.join("daemon.ready"), common::DAEMON_READY_TIMEOUT);
            let sni = wait_for_sni_service(&watcher_conn, Duration::from_secs(5));
            let client = common::connect_default_client().expect("connect client");
            client
                .set_restic_provider(&ResticProvider::Custom {
                    path: restic.clone(),
                })
                .expect("set provider");

            let entry = restore_entry(&fetch_dbusmenu_items(&client_conn, &sni));
            assert!(!entry.enabled, "(b) disabled without destination");
            assert_eq!(
                entry.label, "Restore Files… (no destination connected)",
                "(b) with its reason"
            );

            let repo = root.join("repo-foreign");
            let dest = Destination::new("NAS", DestinationKind::Local { path: repo.clone() })
                .expect("dest");
            let password = connect_local_repo(&client, &restic, &dest);
            write_snapshot(
                &restic,
                &repo,
                &password,
                root,
                &RawSnapshot {
                    tag: "daily",
                    time: "2026-11-15 12:00:00",
                    host: "elsewhere",
                },
            );
            let entry = restore_entry(&fetch_dbusmenu_items(&client_conn, &sni));
            assert!(entry.enabled, "(a) enabled with a destination: {entry:?}");
            assert_eq!(entry.label, "Restore Files…", "(a) plain label");

            let signals = KesticSignalCollector::start(
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );
            click_dbusmenu_item(&client_conn, &sni, entry.id);
            let sigs = signals.wait_until(Duration::from_secs(3), |s| {
                s.iter()
                    .any(|x| matches!(x, CapturedKesticSignal::ShowRestoreRequested))
            });
            let window = sigs
                .iter()
                .position(|s| matches!(s, CapturedKesticSignal::ShowWindowRequested));
            let restore = sigs
                .iter()
                .position(|s| matches!(s, CapturedKesticSignal::ShowRestoreRequested));
            assert!(restore.is_some(), "(c) ShowRestoreRequested: {sigs:?}");
            assert!(
                window.is_some() && window < restore,
                "(c) window first: {sigs:?}"
            );
            common::stop_daemon(&client, handle);

            // Without a GUI the daemon starts one on the tab «Restore».
            let log = root.join("gui-spawns.log");
            let fake_gui = root.join("fake-kestic-gui");
            write_executable(
                &fake_gui,
                &format!(
                    "#!/bin/sh\nprintf 'ARG:%s\\n' \"$*\" >> \"{}\"\n",
                    log.display()
                ),
            );
            fs::remove_file(root.join("daemon.ready")).expect("remove ready file");
            let handle = thread::spawn({
                let cfg = DaemonRunConfig {
                    assume_sni_available: false,
                    kestic_binary: Some(fake_gui),
                    suppress_ui_spawn: false,
                    ..sandbox_run_config(root)
                };
                move || run_daemon(cfg)
            });
            wait_for_file(&root.join("daemon.ready"), common::DAEMON_READY_TIMEOUT);
            let sni = wait_for_sni_service_at(&watcher_conn, 1, Duration::from_secs(5));
            let client = common::connect_default_client().expect("connect client");
            let entry = restore_entry(&fetch_dbusmenu_items(&client_conn, &sni));
            click_dbusmenu_item(&client_conn, &sni, entry.id);
            assert_eq!(
                gui_starts(&log),
                vec!["ARG:--show-restore"],
                "(c) GUI start"
            );
            common::stop_daemon(&client, handle);
        },
    );
}

fn cancel_entry(conn: &Connection, sni: &str) -> TrayMenuEntry {
    fetch_dbusmenu_items(conn, sni)
        .into_iter()
        .find(|i| i.label == "Cancel Running Backup")
        .expect("no «Cancel Running Backup» entry")
}

/// Waits until the tray menu shows a line containing `text` (the tray follows the daemon's
/// state asynchronously).
fn wait_for_menu_line(conn: &Connection, sni: &str, text: &str) -> Vec<TrayMenuEntry> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let items = fetch_dbusmenu_items(conn, sni);
        if items.iter().any(|i| i.label.contains(text)) {
            return items;
        }
        assert!(
            Instant::now() < deadline,
            "no line with {text:?}: {items:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// AC-M4-05-24: the tray entry «Cancel Running Backup» is (d) disabled while only a restore
/// runs and (e) enabled while a backup runs beside it.
#[test]
fn ac_m4_05_24_cancel_backup_entry_ignores_a_running_restore() {
    run_isolated_test(
        "ac_m4_05_24_cancel_backup_entry_ignores_a_running_restore",
        "KESTIC_AC_M4_05_24_CANCEL_INNER",
        |root| {
            let fakes = StdFakes::start();
            let client_conn = block_on(Connection::session()).expect("session conn");
            let variant = latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4TC", small_tree, |_| {});
            let sni = wait_for_sni_service(&fakes.sni_conn, Duration::from_secs(5));

            fx.rig.wrapper.hold("restore");
            fx.start_restore(
                &["a.txt"],
                RestoreTarget::NewFolder {
                    dir: fx.new_dir("tray").to_string_lossy().into_owned(),
                },
                RestoreOverwrite::Never,
                None,
            )
            .expect("StartRestore");
            fx.rig
                .wrapper
                .wait_for('S', "restore", 1, Duration::from_secs(30));
            wait_for_menu_line(&client_conn, &sni, "restoring");
            let entry = cancel_entry(&client_conn, &sni);
            assert!(!entry.enabled, "(d) disabled while only a restore runs");

            fx.rig.wrapper.hold("backup");
            fx.rig
                .daemon
                .client
                .start_backup(fx.backup.id.as_str())
                .expect("StartBackup");
            fx.rig
                .wrapper
                .wait_for('S', "backup", 1, Duration::from_secs(30));
            wait_for_menu_line(&client_conn, &sni, "backing up");
            let entry = cancel_entry(&client_conn, &sni);
            assert!(entry.enabled, "(e) enabled while a backup runs");

            fx.rig.wrapper.release("backup");
            fx.rig.wrapper.release("restore");
            fx.rig.daemon.stop();
        },
    );
}

fn gui_starts(log: &Path) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let lines: Vec<String> = fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        if !lines.is_empty() || Instant::now() > deadline {
            return lines;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// A running restore shows its progress in its Plasma job, not only at the end: (a) an update
/// with restic's percentage and (b) the restore's text with that percentage.
#[test]
fn m4_s05_restore_shows_its_progress_in_plasma() {
    run_isolated_test(
        "m4_s05_restore_shows_its_progress_in_plasma",
        "KESTIC_M4_S05_JOB_PROGRESS_INNER",
        |root| {
            let job_view = common::fakes::FakeJobViewHandle::start("org.kde.JobViewServer");
            let _fakes = StdFakes::start();
            let rec = &job_view.recorder;
            let variant = latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RJ", small_tree, |o| {
                o.enable_job_view = true;
            });
            fx.rig.wrapper.hold_with_output(
                "restore",
                "{\"message_type\":\"status\",\"percent_done\":0.25,\"files_done\":1,\
                 \"bytes_done\":10,\"total_bytes\":40,\"seconds_elapsed\":1}\n",
            );
            let target = RestoreTarget::NewFolder {
                dir: fx.new_dir("progress").to_string_lossy().into_owned(),
            };
            let run_id = fx
                .start_restore(&["a.txt"], target, RestoreOverwrite::Never, None)
                .expect("StartRestore");
            common::m3::wait_until("progress update", || {
                rec.updates
                    .lock()
                    .expect("lock")
                    .iter()
                    .any(|(pct, _)| *pct == 25)
            });
            let updates = rec.updates.lock().expect("lock").clone();
            assert!(updates.iter().any(|(pct, _)| *pct == 25), "(a) {updates:?}");
            assert!(
                updates.iter().any(|(pct, msg)| *pct == 25
                    && msg.contains("restoring")
                    && msg.contains("25%")),
                "(b) {updates:?}"
            );
            fx.rig.wrapper.release("restore");
            fx.wait_run(&run_id);
            fx.rig.daemon.stop();
        },
    );
}

/// AC-M4-05-30 (daemon part): the Plasma job of a restore shows the speed from the growth of
/// the bytes restored over the daemon's clock: (a) none from status lines at one moment, and
/// (b) 4 MiB/s from a line 1 s later with 4 MiB more.
#[test]
fn ac_m4_05_30_restore_job_shows_the_speed_of_the_bytes_restored() {
    run_isolated_test(
        "ac_m4_05_30_restore_job_shows_the_speed_of_the_bytes_restored",
        "KESTIC_AC_M4_05_30_INNER",
        |root| {
            const MIB: u64 = 1024 * 1024;
            let job_view = common::fakes::FakeJobViewHandle::start("org.kde.JobViewServer");
            let _fakes = StdFakes::start();
            let rec = &job_view.recorder;
            let variant = latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RS", small_tree, |o| {
                o.enable_job_view = true;
            });
            let line = |bytes: u64| {
                format!(
                    "{{\"message_type\":\"status\",\"percent_done\":0.25,\"files_restored\":1,\
                     \"bytes_restored\":{bytes},\"total_bytes\":{}}}\n",
                    40 * MIB
                )
            };
            fx.rig
                .wrapper
                .hold_with_repeated_output("restore", &line(MIB));
            let target = RestoreTarget::NewFolder {
                dir: fx.new_dir("speed").to_string_lossy().into_owned(),
            };
            let run_id = fx
                .start_restore(&["a.txt"], target, RestoreOverwrite::Never, None)
                .expect("StartRestore");
            common::m3::wait_until("first progress update", || {
                !rec.sizes.lock().expect("lock").is_empty()
            });
            assert!(
                rec.sizes
                    .lock()
                    .expect("lock")
                    .iter()
                    .all(|(_, _, speed)| *speed == 0),
                "(a) no speed from lines at one moment"
            );
            fx.rig.clock.advance(Duration::from_secs(1));
            std::fs::write(
                format!("{}.restore.repeat", fx.rig.wrapper.hold.display()),
                line(5 * MIB),
            )
            .expect("write the next status line");
            common::m3::wait_until("progress with a speed", || {
                rec.sizes
                    .lock()
                    .expect("lock")
                    .iter()
                    .any(|(_, _, speed)| *speed > 0)
            });
            let speeds: Vec<u64> = rec
                .sizes
                .lock()
                .expect("lock")
                .iter()
                .map(|(_, _, speed)| *speed)
                .filter(|s| *s > 0)
                .collect();
            assert!(
                speeds.iter().all(|s| *s == 4 * MIB),
                "(b) 4 MiB more after 1 s: {speeds:?}"
            );
            fx.rig.wrapper.release("restore");
            fx.wait_run(&run_id);
            fx.rig.daemon.stop();
        },
    );
}
