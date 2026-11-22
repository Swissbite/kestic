// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S05 tray entry «Restore Files…»: enabled with a destination, disabled with its reason
//! without one, and opening the main window on the tab «Restore».

mod common;

use common::m3::connect_local_repo;
use common::m4::{RawSnapshot, write_snapshot};
use common::{
    CapturedKesticSignal, KesticSignalCollector, TrayMenuEntry, click_dbusmenu_item,
    fetch_dbusmenu_items, restic_test_binaries, run_isolated_test, sandbox_run_config,
    start_fake_sni_watcher, wait_for_file, wait_for_sni_service, wait_for_sni_service_at,
};
use kestic_core::model::{Destination, DestinationKind, ResticProvider};
use kestic_daemon::dbus_api::{KESTIC_BUS_NAME, KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH};
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
