// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Lock order between the tray (`ksni`) and the daemon state: opening the tray menu while a
//! backup reports progress must never block the daemon.

mod common;

use common::m3::{M3Rig, StdFakes, backup_summary, latest_variant, wait_for_state};
use common::{run_isolated_test, wait_for_sni_service};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use time::macros::datetime;

/// How long the tray menu is opened over and over while the backup reports progress.
const HAMMER_FOR: Duration = Duration::from_secs(6);
/// A `GetState` slower than this means the daemon is stuck.
const RESPONSIVE_WITHIN: Duration = Duration::from_secs(5);

/// restic `--json` status lines of a running backup (one progress report per line); the held
/// backup prints them over and over while the tray menu is opened.
fn status_chunk(count: usize) -> String {
    let mut out = String::with_capacity(count * 170);
    for i in 0..count {
        let percent = (i as f64) / (count as f64);
        out.push_str(&format!(
            "{{\"message_type\":\"status\",\"percent_done\":{percent:.6},\"total_files\":1000,\"files_done\":{i},\"total_bytes\":104857600,\"bytes_done\":{i},\"seconds_elapsed\":1,\"seconds_remaining\":30}}\n"
        ));
    }
    out
}

/// Calls `GetState` on a helper thread; `false` when it fails or does not answer in time.
fn get_state_within(client: &kestic_daemon::dbus_api::KesticClient, timeout: Duration) -> bool {
    let client = client.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(client.get_state().is_ok());
    });
    rx.recv_timeout(timeout).unwrap_or(false)
}

/// Opening the tray menu (`AboutToShow`) while a backup reports progress: (a) the daemon keeps
/// answering `GetState`, (b) every `AboutToShow` call returns, and (c) the run then finishes.
#[test]
fn tray_menu_during_backup_progress_does_not_block_daemon() {
    run_isolated_test(
        "tray_menu_during_backup_progress_does_not_block_daemon",
        "KESTIC_TRAY_LOCK_ORDER_INNER",
        |root| {
            let fakes = StdFakes::start();
            let variant = latest_variant();
            let rig = M3Rig::start(
                root,
                &variant,
                "TrayLock",
                datetime!(2026-12-01 09:00:00 UTC),
                |opts| opts.enable_scheduler = false,
            );
            let client = rig.daemon.client.clone();
            let sni_service = wait_for_sni_service(&fakes.sni_conn, Duration::from_secs(5));
            let fx = rig.add_backup("Docs", |_| {});
            let bid = fx.backup.id.clone();

            rig.wrapper
                .hold_with_repeated_output("backup", &status_chunk(1_000));
            client.start_backup(&fx.backup_id_str).expect("StartBackup");
            wait_for_state(&client, Duration::from_secs(10), "run active", |s| {
                backup_summary(s, &bid).active_run.is_some()
            });

            let stop = Arc::new(AtomicBool::new(false));
            let menu_calls = Arc::new(AtomicUsize::new(0));
            let hammer = {
                let stop = Arc::clone(&stop);
                let menu_calls = Arc::clone(&menu_calls);
                thread::spawn(move || {
                    let conn = zbus::block_on(zbus::Connection::session()).expect("session bus");
                    while !stop.load(Ordering::SeqCst) {
                        zbus::block_on(conn.call_method(
                            Some(sni_service.as_str()),
                            "/MenuBar",
                            Some("com.canonical.dbusmenu"),
                            "AboutToShow",
                            &(0i32,),
                        ))
                        .expect("AboutToShow");
                        menu_calls.fetch_add(1, Ordering::SeqCst);
                    }
                })
            };

            // (a) the daemon answers throughout.
            let deadline = Instant::now() + HAMMER_FOR;
            while Instant::now() < deadline {
                assert!(
                    get_state_within(&client, RESPONSIVE_WITHIN),
                    "(a) daemon did not answer GetState within {RESPONSIVE_WITHIN:?} while the \
                     tray menu was opened {} times",
                    menu_calls.load(Ordering::SeqCst)
                );
                thread::sleep(Duration::from_millis(200));
            }

            // (b) the menu calls return.
            stop.store(true, Ordering::SeqCst);
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = tx.send(hammer.join().is_ok());
            });
            assert_eq!(
                rx.recv_timeout(RESPONSIVE_WITHIN),
                Ok(true),
                "(b) AboutToShow calls returned"
            );
            assert!(
                menu_calls.load(Ordering::SeqCst) > 0,
                "(b) the tray menu was opened at least once"
            );

            // (c) the run finishes once restic is released.
            rig.wrapper.release("backup");
            wait_for_state(&client, Duration::from_secs(60), "run finished", |s| {
                backup_summary(s, &bid).active_run.is_none()
            });
            rig.daemon.stop();
        },
    );
}
