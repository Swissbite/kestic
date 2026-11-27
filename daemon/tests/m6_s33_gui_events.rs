// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M6-S33: the GUI starts the daemon, receives its changes and builds the shown command without
//! waiting in the Qt main thread (`AC-M6-33-3`, `AC-M6-33-4`, `AC-M6-33-5`).

mod common;

use common::m3::{M3Rig, StdFakes, latest_variant, wait_until_within};
use common::{find_kestic_binary, run_isolated_test};
use kestic_daemon::dbus_api::KESTIC_INTERFACE_V1;
use kestic_test_support::gui::{IsolatedEnv, SmokeContext};
use kestic_test_support::workspace::workspace_root;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use time::macros::datetime;
use zbus::export::ordered_stream::OrderedStreamExt;

/// How long the daemon started by the GUI takes before it takes its bus name (`AC-M6-33-3`).
/// It is longer than the 4.5 s a GUI that waited in the main thread would give it.
const DAEMON_START_DELAY: &str = "6000";
/// Longest time a state change may take from the daemon into the GUI (`AC-M6-33-4` (a)).
const CHANGE_WITHIN: Duration = Duration::from_secs(1);
/// Idle window and the `GetState` calls allowed in it (`AC-M6-33-4` (b)).
const IDLE_WINDOW: Duration = Duration::from_secs(10);
const MAX_GET_STATE_WHILE_IDLE: usize = 2;
/// How long a held `restic version` blocks before the test releases it (`AC-M6-33-5`).
const VERSION_HOLD: Duration = Duration::from_secs(8);

/// Kills and reaps the GUI process when the test ends.
struct GuiProcess(Child);

impl Drop for GuiProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One `Kestic1` method call seen on the bus.
#[derive(Clone, Debug)]
struct Call {
    sender: String,
    member: String,
    at: Instant,
}

/// Records every `Kestic1` method call on the session bus with a monitor connection.
fn monitor_kestic_calls() -> Arc<Mutex<Vec<Call>>> {
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
                let rule = format!("type='method_call',interface='{KESTIC_INTERFACE_V1}'");
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
                    let header = msg.header();
                    let call = Call {
                        sender: header.sender().map_or_else(String::new, |s| s.to_string()),
                        member: header.member().map_or_else(String::new, |m| m.to_string()),
                        at: Instant::now(),
                    };
                    seen.lock().expect("seen calls").push(call);
                }
            });
        });
    }
    wait_until_within(Duration::from_secs(5), "monitor connection ready", || {
        *ready.lock().expect("ready flag")
    });
    seen
}

/// The calls named `member` that did not come from `except` (the test's own client).
fn calls_of(seen: &Mutex<Vec<Call>>, member: &str, except: &str) -> Vec<Call> {
    seen.lock()
        .expect("seen calls")
        .iter()
        .filter(|c| c.member == member && c.sender != except)
        .cloned()
        .collect()
}

/// Starts the GUI offscreen on `bus`, without starting a daemon of its own.
fn start_gui(bus: &str) -> GuiProcess {
    GuiProcess(
        Command::new(find_kestic_binary())
            .args(["--bus-name", bus, "--no-autostart"])
            .env("QT_QPA_PLATFORM", "offscreen")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start GUI"),
    )
}

/// The process that owns `bus`, if any.
fn owner_pid(conn: &zbus::Connection, bus: &str) -> Option<u32> {
    zbus::block_on(async {
        let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
        let name = zbus::names::BusName::try_from(bus).ok()?;
        dbus.get_connection_unix_process_id(name).await.ok()
    })
}

/// Runs the offscreen smoke `scenario` of this story in German and English.
fn run_smoke(root: &Path, scenario: &str, extra_args: &[&str], extra_env: &[(&str, &str)]) {
    let iso = IsolatedEnv::adopt(root);
    let dbus_addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("isolated session bus");
    for lang in ["de", "en"] {
        SmokeContext {
            kestic_bin: &find_kestic_binary(),
            iso: &iso,
            dbus_addr: &dbus_addr,
        }
        .run_with_env(
            scenario,
            "wide",
            lang,
            &format!("target/smoke-artifacts/{scenario}-wide-{lang}.png"),
            extra_args,
            extra_env,
        );
    }
}

/// AC-M6-33-3: the daemon is not running when the GUI starts and takes 6 s to come up; the GUI
/// starts it and (a) does not wait for it in the main thread, (b) shows «Hintergrunddienst
/// startet…» meanwhile. The smoke scene `m6-daemon-starting` checks both inside the GUI.
#[test]
fn ac_m6_33_3_gui_starts_daemon_without_waiting() {
    run_isolated_test(
        "ac_m6_33_3_gui_starts_daemon_without_waiting",
        "KESTIC_M6_S33_DAEMON_START_INNER",
        |root| {
            let fakes = StdFakes::start();
            let bus = "net.daester.KesticDaemonStarting";
            let iso = IsolatedEnv::adopt(root);
            let dbus_addr =
                std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("isolated session bus");
            for lang in ["de", "en"] {
                SmokeContext {
                    kestic_bin: &find_kestic_binary(),
                    iso: &iso,
                    dbus_addr: &dbus_addr,
                }
                .run_starting_daemon(
                    "m6-daemon-starting",
                    "wide",
                    lang,
                    &format!("target/smoke-artifacts/m6-daemon-starting-wide-{lang}.png"),
                    &["--bus-name", bus],
                    &[("KESTIC_TEST_DAEMON_START_DELAY_MS", DAEMON_START_DELAY)],
                );
                // The daemon the GUI started keeps running; the next language starts it again.
                let pid = owner_pid(&fakes.sni_conn, bus).expect("the GUI started the daemon");
                common::m3::send_signal(pid, "TERM");
                wait_until_within(Duration::from_secs(10), "daemon gone", || {
                    owner_pid(&fakes.sni_conn, bus).is_none()
                });
            }
        },
    );
}

/// AC-M6-33-4: with an idle GUI, (a) a change in the daemon reaches the GUI over
/// `StatusChanged` within 1 s (it fetches the new state at once), and (b) the GUI calls
/// `GetState` at most twice in 10 s of idle time, because no periodic timer drives it.
#[test]
fn ac_m6_33_4_changes_arrive_by_signal_without_polling() {
    run_isolated_test(
        "ac_m6_33_4_changes_arrive_by_signal_without_polling",
        "KESTIC_M6_S33_SIGNAL_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let variant = latest_variant();
            let bus = "net.daester.KesticSignalOnly";
            let rig = M3Rig::start_on(
                root,
                &variant,
                &variant.sub("SignalOnly"),
                bus,
                datetime!(2026-12-01 09:00:00 UTC),
                |opts| opts.enable_scheduler = false,
            );
            let client = rig.daemon.client.clone();
            let me = client
                .connection()
                .unique_name()
                .expect("test client name")
                .to_string();
            let seen = monitor_kestic_calls();
            let _gui = start_gui(bus);
            wait_until_within(
                Duration::from_secs(40),
                "precondition: the GUI registered and loaded the state",
                || {
                    !calls_of(&seen, "RegisterGui", &me).is_empty()
                        && !calls_of(&seen, "GetState", &me).is_empty()
                },
            );
            thread::sleep(Duration::from_secs(2));

            let before = calls_of(&seen, "GetState", &me).len();
            let changed_at = Instant::now();
            client.set_expert_mode(true).expect("SetExpertMode");
            wait_until_within(
                Duration::from_secs(10),
                "the GUI fetches the state after the change",
                || calls_of(&seen, "GetState", &me).len() > before,
            );
            let fetched_at = calls_of(&seen, "GetState", &me)[before].at;
            let took = fetched_at.saturating_duration_since(changed_at);
            assert!(
                took <= CHANGE_WITHIN,
                "(a) the change reached the GUI after {took:?}, not within {CHANGE_WITHIN:?}"
            );

            thread::sleep(Duration::from_secs(2));
            let idle_start = calls_of(&seen, "GetState", &me).len();
            thread::sleep(IDLE_WINDOW);
            let idle_calls = calls_of(&seen, "GetState", &me).len() - idle_start;
            assert!(
                idle_calls <= MAX_GET_STATE_WHILE_IDLE,
                "(b) the idle GUI called GetState {idle_calls} times in {IDLE_WINDOW:?}"
            );
            let qml_dir = workspace_root().join("gui/src/qml");
            let polling: Vec<String> = fs::read_dir(&qml_dir)
                .expect("read QML dir")
                .map(|e| e.expect("dir entry").path())
                .filter(|p| fs::read_to_string(p).is_ok_and(|s| s.contains("pollEvents")))
                .map(|p| p.display().to_string())
                .collect();
            assert!(
                polling.is_empty(),
                "(b) no periodic timer in QML calls pollEvents: {polling:?}"
            );
            rig.daemon.stop();
        },
    );
}

/// Starts `kestic daemon` as a process of its own on `bus` and waits until it owns the name.
fn start_daemon_process(conn: &zbus::Connection, bus: &str) -> GuiProcess {
    let daemon = GuiProcess(
        Command::new(find_kestic_binary())
            .args(["daemon", "--bus-name", bus])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start daemon"),
    );
    wait_until_within(Duration::from_secs(20), "daemon owns its bus name", || {
        owner_pid(conn, bus) == Some(daemon.0.id())
    });
    daemon
}

/// A daemon that dies without a word (SIGKILL) while the GUI is idle, and a new one in its
/// place: the GUI registers with the new daemon on its own and receives its changes again.
#[test]
fn idle_gui_follows_a_replaced_daemon() {
    run_isolated_test(
        "idle_gui_follows_a_replaced_daemon",
        "KESTIC_M6_S33_REPLACED_INNER",
        |_root| {
            let fakes = StdFakes::start();
            let bus = "net.daester.KesticReplaced";
            let seen = monitor_kestic_calls();
            let mut first = start_daemon_process(&fakes.sni_conn, bus);
            let _gui = start_gui(bus);
            wait_until_within(
                Duration::from_secs(40),
                "precondition: the GUI registered with the first daemon",
                || !calls_of(&seen, "RegisterGui", "").is_empty(),
            );
            thread::sleep(Duration::from_secs(1));

            first.0.kill().expect("SIGKILL the first daemon");
            first.0.wait().expect("reap the first daemon");
            wait_until_within(Duration::from_secs(10), "bus name released", || {
                owner_pid(&fakes.sni_conn, bus).is_none()
            });
            let registered_before = calls_of(&seen, "RegisterGui", "").len();
            let _second = start_daemon_process(&fakes.sni_conn, bus);
            wait_until_within(
                Duration::from_secs(10),
                "the GUI registers with the new daemon",
                || calls_of(&seen, "RegisterGui", "").len() > registered_before,
            );

            let client = kestic_daemon::dbus_api::KesticClient::connect(
                bus,
                kestic_daemon::dbus_api::KESTIC_OBJECT_PATH,
            )
            .expect("client of the new daemon");
            let me = client
                .connection()
                .unique_name()
                .expect("test client name")
                .to_string();
            thread::sleep(Duration::from_secs(1));
            let before = calls_of(&seen, "GetState", &me).len();
            client.set_expert_mode(true).expect("SetExpertMode");
            wait_until_within(
                Duration::from_secs(5),
                "the GUI fetches the new daemon's state after a change",
                || calls_of(&seen, "GetState", &me).len() > before,
            );
        },
    );
}

/// AC-M6-33-5: `getBackupTransparentCommand` needs the restic version; (a) it does not run
/// `restic version` in the main thread. Every `restic version` is held for 8 s; the smoke scene
/// `m6-transparent-command` times the call inside the GUI.
#[test]
fn ac_m6_33_5_shown_command_does_not_run_restic_in_main_thread() {
    run_isolated_test(
        "ac_m6_33_5_shown_command_does_not_run_restic_in_main_thread",
        "KESTIC_M6_S33_SHOWN_COMMAND_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let variant = latest_variant();
            let bus = "net.daester.KesticShownCommand";
            let rig = M3Rig::start_on(
                root,
                &variant,
                &variant.sub("ShownCommand"),
                bus,
                datetime!(2026-12-01 09:00:00 UTC),
                |opts| opts.enable_scheduler = false,
            );
            let dest = rig.add_destination("Command Vault", |_| {});
            rig.add_backup_to(&dest, "Documents", |_| {});

            rig.wrapper.hold("version");
            let releaser = {
                let wrapper_hold = rig.wrapper.hold.clone();
                thread::spawn(move || {
                    thread::sleep(VERSION_HOLD);
                    let _ = fs::remove_file(format!("{}.version", wrapper_hold.display()));
                })
            };
            run_smoke(
                root,
                "m6-transparent-command",
                &["--bus-name", bus, "--no-autostart"],
                &[],
            );
            releaser.join().expect("release thread");
            rig.daemon.stop();
        },
    );
}
