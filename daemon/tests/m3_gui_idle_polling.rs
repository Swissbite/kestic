// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! An idle main window asks the daemon for its state only when something changed: it must not
//! poll `GetState` on its 150 ms event timer.

mod common;

use common::m3::{M3Rig, StdFakes, latest_variant};
use common::{find_kestic_binary, run_isolated_test};
use kestic_daemon::dbus_api::KESTIC_INTERFACE_V1;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use time::macros::datetime;
use zbus::export::ordered_stream::OrderedStreamExt;

/// Bus name the GUI under test connects to (passed with `--bus-name`).
const BUS: &str = "net.daester.KesticIdlePolling";
/// Quiet window in which the daemon state does not change.
const IDLE_WINDOW: Duration = Duration::from_secs(3);
/// `GetState` calls tolerated in the quiet window (window activation and the like).
const MAX_CALLS_WHILE_IDLE: usize = 3;

/// Kills and reaps the GUI process when the test ends.
struct GuiProcess(Child);

impl Drop for GuiProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Counts `GetState` method calls on the session bus with a monitor connection.
fn count_get_state_calls() -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let ready = Arc::new(Mutex::new(false));
    {
        let calls = Arc::clone(&calls);
        let ready = Arc::clone(&ready);
        thread::spawn(move || {
            zbus::block_on(async move {
                let conn = zbus::Connection::session()
                    .await
                    .expect("monitor connection");
                let rule = format!(
                    "type='method_call',interface='{KESTIC_INTERFACE_V1}',member='GetState'"
                );
                conn.call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus.Monitoring"),
                    "BecomeMonitor",
                    &(vec![rule.as_str()], 0u32),
                )
                .await
                .expect("BecomeMonitor");
                *ready.lock().expect("ready flag") = true;
                let mut stream = zbus::MessageStream::from(&conn);
                while let Some(Ok(msg)) = stream.next().await {
                    if msg
                        .header()
                        .member()
                        .is_some_and(|m| m.as_str() == "GetState")
                    {
                        calls.fetch_add(1, Ordering::SeqCst);
                    }
                }
            });
        });
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !*ready.lock().expect("ready flag") {
        assert!(Instant::now() < deadline, "monitor connection ready");
        thread::sleep(Duration::from_millis(20));
    }
    calls
}

/// An idle GUI: (a) loads the daemon state at start, then (b) does not call `GetState` on every
/// event-timer tick while nothing changes.
#[test]
fn idle_gui_does_not_poll_daemon_state() {
    run_isolated_test(
        "idle_gui_does_not_poll_daemon_state",
        "KESTIC_GUI_IDLE_POLLING_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let variant = latest_variant();
            let rig = M3Rig::start_on(
                root,
                &variant,
                &variant.sub("IdlePolling"),
                BUS,
                datetime!(2026-12-01 09:00:00 UTC),
                |opts| opts.enable_scheduler = false,
            );
            let calls = count_get_state_calls();

            let _gui = GuiProcess(
                Command::new(find_kestic_binary())
                    .args(["--bus-name", BUS, "--no-autostart"])
                    .env("QT_QPA_PLATFORM", "offscreen")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("start GUI"),
            );

            // (a) the GUI loads the state once it is up.
            let deadline = Instant::now() + Duration::from_secs(30);
            while calls.load(Ordering::SeqCst) == 0 {
                assert!(
                    Instant::now() < deadline,
                    "(a) GUI called GetState after start"
                );
                thread::sleep(Duration::from_millis(50));
            }
            thread::sleep(Duration::from_secs(2));

            // (b) quiet window: no state change, so (almost) no further GetState calls.
            let before = calls.load(Ordering::SeqCst);
            thread::sleep(IDLE_WINDOW);
            let during = calls.load(Ordering::SeqCst) - before;
            assert!(
                during <= MAX_CALLS_WHILE_IDLE,
                "(b) idle GUI called GetState {during} times in {IDLE_WINDOW:?}"
            );
            drop(_gui);
            rig.daemon.stop();
        },
    );
}
