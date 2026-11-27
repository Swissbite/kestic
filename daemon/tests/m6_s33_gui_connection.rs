// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M6-S33: the GUI talks to the daemon over one session-bus connection (`AC-M6-33-2` (a)).

mod common;

use common::m3::{M3Rig, StdFakes, latest_variant};
use common::{find_kestic_binary, run_isolated_test};
use kestic_daemon::dbus_api::KESTIC_INTERFACE_V1;
use std::collections::BTreeSet;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use time::macros::datetime;
use zbus::export::ordered_stream::OrderedStreamExt;

/// Bus name the GUI under test connects to (passed with `--bus-name`).
const BUS: &str = "net.daester.KesticOneConnection";
/// Daemon calls the GUI makes before the connections are counted.
const MIN_GUI_CALLS: usize = 10;
/// Interface of the desktop portal settings that Qt reads over its own connection.
const QT_PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";

/// Kills and reaps the GUI process when the test ends.
struct GuiProcess(Child);

impl Drop for GuiProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One message seen on the bus.
#[derive(Clone, Debug)]
struct Seen {
    sender: String,
    interface: String,
    member: String,
    /// First string argument (the rule of an `AddMatch`), else empty.
    arg0: String,
}

/// Records every method call on the session bus with a monitor connection.
fn monitor_method_calls() -> Arc<Mutex<Vec<Seen>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let ready = Arc::new(Mutex::new(false));
    {
        let seen = Arc::clone(&seen);
        let ready = Arc::clone(&ready);
        thread::spawn(move || {
            zbus::block_on(async move {
                let conn = zbus::Connection::session()
                    .await
                    .expect("monitor connection");
                conn.call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus.Monitoring"),
                    "BecomeMonitor",
                    &(vec!["type='method_call'"], 0u32),
                )
                .await
                .expect("BecomeMonitor");
                *ready.lock().expect("ready flag") = true;
                let mut stream = zbus::MessageStream::from(&conn);
                while let Some(Ok(msg)) = stream.next().await {
                    let header = msg.header();
                    let text = |v: Option<&str>| v.unwrap_or_default().to_string();
                    let entry = Seen {
                        sender: text(header.sender().map(|s| s.as_str())),
                        interface: text(header.interface().map(|i| i.as_str())),
                        member: text(header.member().map(|m| m.as_str())),
                        arg0: msg.body().deserialize::<String>().unwrap_or_default(),
                    };
                    seen.lock().expect("seen calls").push(entry);
                }
            });
        });
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !*ready.lock().expect("ready flag") {
        assert!(Instant::now() < deadline, "monitor connection ready");
        thread::sleep(Duration::from_millis(20));
    }
    seen
}

/// The unique names on the bus that belong to process `pid`.
fn connections_of(conn: &zbus::Connection, pid: u32) -> BTreeSet<String> {
    zbus::block_on(async {
        let dbus = zbus::fdo::DBusProxy::new(conn).await.expect("DBus proxy");
        let mut out = BTreeSet::new();
        for name in dbus.list_names().await.expect("ListNames") {
            if !name.starts_with(':') {
                continue;
            }
            if dbus
                .get_connection_unix_process_id(name.as_ref())
                .await
                .is_ok_and(|p| p == pid)
            {
                out.insert(name.to_string());
            }
        }
        out
    })
}

/// AC-M6-33-2 (a): over a GUI session with at least ten daemon calls, every `Kestic1` method
/// call and the match for the `Kestic1` signals come from one unique bus name, and the GUI
/// holds no further connection of its own.
#[test]
fn ac_m6_33_2a_gui_uses_one_session_bus_connection() {
    run_isolated_test(
        "ac_m6_33_2a_gui_uses_one_session_bus_connection",
        "KESTIC_M6_S33_ONE_CONNECTION_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let variant = latest_variant();
            let rig = M3Rig::start_on(
                root,
                &variant,
                &variant.sub("OneConnection"),
                BUS,
                datetime!(2026-12-01 09:00:00 UTC),
                |opts| opts.enable_scheduler = false,
            );
            let client = rig.daemon.client.clone();
            let test_name = client
                .connection()
                .unique_name()
                .expect("test client name")
                .to_string();
            let seen = monitor_method_calls();

            let gui = GuiProcess(
                Command::new(find_kestic_binary())
                    .args(["--bus-name", BUS, "--no-autostart"])
                    .env("QT_QPA_PLATFORM", "offscreen")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("start GUI"),
            );
            let gui_calls = || -> Vec<Seen> {
                seen.lock()
                    .expect("seen calls")
                    .iter()
                    .filter(|s| s.interface == KESTIC_INTERFACE_V1 && s.sender != test_name)
                    .cloned()
                    .collect()
            };

            // Every state change makes the GUI fetch the state again.
            let deadline = Instant::now() + Duration::from_secs(40);
            let mut expert = false;
            while gui_calls().len() < MIN_GUI_CALLS {
                assert!(
                    Instant::now() < deadline,
                    "precondition: the GUI made {MIN_GUI_CALLS} daemon calls, saw {:?}",
                    gui_calls()
                );
                if !gui_calls().is_empty() {
                    expert = !expert;
                    client.set_expert_mode(expert).expect("SetExpertMode");
                }
                thread::sleep(Duration::from_millis(300));
            }

            let calls = gui_calls();
            let senders: BTreeSet<String> = calls.iter().map(|s| s.sender.clone()).collect();
            assert_eq!(
                senders.len(),
                1,
                "(a) all Kestic1 calls of the GUI come from one connection: {calls:?}"
            );
            let kestic = senders.first().cloned().unwrap_or_default();

            let all = seen.lock().expect("seen calls").clone();
            let match_senders: BTreeSet<String> = all
                .iter()
                .filter(|s| s.member == "AddMatch" && s.arg0.contains(KESTIC_INTERFACE_V1))
                .filter(|s| s.sender != test_name)
                .map(|s| s.sender.clone())
                .collect();
            assert_eq!(
                match_senders,
                BTreeSet::from([kestic.clone()]),
                "(a) the match for the Kestic1 signals is on the same connection"
            );

            // Qt opens a session-bus connection of its own in every Qt application (it reads
            // the desktop portal settings for the platform theme); Kestic cannot remove it, so
            // it is not counted.
            let qt_toolkit: BTreeSet<String> = all
                .iter()
                .filter(|s| s.interface == QT_PORTAL_SETTINGS)
                .map(|s| s.sender.clone())
                .collect();
            let live = connections_of(client.connection(), gui.0.id());
            let others: BTreeSet<String> = live
                .iter()
                .filter(|n| **n != kestic && !qt_toolkit.contains(*n))
                .cloned()
                .collect();
            assert!(
                live.contains(&kestic),
                "(a) the GUI still holds its connection {kestic}: {live:?}"
            );
            assert!(
                others.is_empty(),
                "(a) the GUI holds no further connection besides Qt's {qt_toolkit:?}: {others:?}"
            );

            drop(gui);
            rig.daemon.stop();
        },
    );
}
