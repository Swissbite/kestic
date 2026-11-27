// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M6-S33: the GUI starts the daemon, receives its changes and builds the shown command without
//! waiting in the Qt main thread (`AC-M6-33-3`, `AC-M6-33-4`, `AC-M6-33-5`).

mod common;

use common::m3::{M3Rig, StdFakes, latest_variant, wait_until_within};
use common::{find_kestic_binary, run_isolated_test};
use kestic_core::model::Backup;
use kestic_daemon::dbus_api::{KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH, KesticClient};
use kestic_test_support::bus::PrivateDbusGuard;
use kestic_test_support::delayed_daemon::DelayedDaemon;
use kestic_test_support::gui::{IsolatedEnv, SmokeContext};
use kestic_test_support::workspace::workspace_root;
use std::collections::BTreeSet;
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
    /// The first argument of `WindowActive`.
    active: Option<bool>,
}

/// The address of the test's own session bus.
fn session_address() -> String {
    std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("isolated session bus")
}

/// A new connection to the bus at `address`.
fn connect(address: &str) -> zbus::Connection {
    zbus::block_on(zbus::connection::Builder::address(address).build()).expect("connect to the bus")
}

/// Records every `Kestic1` method call on the bus at `address` with a monitor connection.
fn monitor_kestic_calls(address: &str) -> Arc<Mutex<Vec<Call>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let ready = Arc::new(Mutex::new(false));
    {
        let seen = Arc::clone(&seen);
        let ready = Arc::clone(&ready);
        let conn = connect(address);
        thread::spawn(move || {
            zbus::block_on(async move {
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
                    let member = header.member().map_or_else(String::new, |m| m.to_string());
                    let active = (member == "WindowActive")
                        .then(|| msg.body().deserialize::<(bool, String)>().ok())
                        .flatten()
                        .map(|(active, _)| active);
                    let call = Call {
                        sender: header.sender().map_or_else(String::new, |s| s.to_string()),
                        member,
                        at: Instant::now(),
                        active,
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

/// Starts the GUI offscreen on `bus` of the session bus at `address`, without starting a daemon
/// of its own.
fn start_gui(bus: &str, address: &str) -> GuiProcess {
    GuiProcess(
        Command::new(find_kestic_binary())
            .args(["--bus-name", bus, "--no-autostart"])
            .env("QT_QPA_PLATFORM", "offscreen")
            .env("DBUS_SESSION_BUS_ADDRESS", address)
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

/// How late the daemon stand-in answers every call (`AC-M6-33-1`).
const SLOW_REPLY: Duration = Duration::from_secs(5);

/// AC-M6-33-1: a daemon stand-in answers every call 5 s late. (a) The Qt main thread never
/// blocks longer than 100 ms: the window is up at once, each action returns at once, a 20 ms
/// timer never pauses longer than 100 ms; (b) the window stays usable: it shows that it loads,
/// switches tabs while calls are pending, and the results arrive. The smoke scene
/// `m6-slow-daemon` checks this inside the GUI.
#[test]
fn ac_m6_33_1_window_does_not_wait_for_a_slow_daemon() {
    run_isolated_test(
        "ac_m6_33_1_window_does_not_wait_for_a_slow_daemon",
        "KESTIC_M6_S33_SLOW_DAEMON_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let variant = latest_variant();
            let back = "net.daester.KesticSlowBack";
            let front = "net.daester.KesticSlowFront";
            let rig = M3Rig::start_on(
                root,
                &variant,
                &variant.sub("SlowDaemon"),
                back,
                datetime!(2026-12-01 09:00:00 UTC),
                |opts| opts.enable_scheduler = false,
            );
            let dest = rig.add_destination("Slow Vault", |_| {});
            rig.add_backup_to(&dest, "Documents", |_| {});
            let _stand_in = DelayedDaemon::start(&session_address(), front, back, SLOW_REPLY);

            for lang in ["de", "en"] {
                // Each language starts from a running, not paused daemon in plain mode.
                rig.daemon.client.resume_global().expect("ResumeGlobal");
                rig.daemon
                    .client
                    .set_expert_mode(false)
                    .expect("SetExpertMode");
                SmokeContext {
                    kestic_bin: &find_kestic_binary(),
                    iso: &IsolatedEnv::adopt(root),
                    dbus_addr: &session_address(),
                }
                .run_with_env(
                    "m6-slow-daemon",
                    "wide",
                    lang,
                    &format!("target/smoke-artifacts/m6-slow-daemon-wide-{lang}.png"),
                    &["--bus-name", front, "--no-autostart"],
                    &[],
                );
            }
            rig.daemon.stop();
        },
    );
}

/// A daemon with the backup «Documents» on bus `back`, behind a stand-in on `front` that
/// delays each call as `delay_of` says; returns the rig and the backup.
fn slow_rig(
    root: &Path,
    base: &str,
    back: &str,
    front: &str,
    delay_of: impl Fn(&str, &str) -> Duration + Send + Sync + 'static,
) -> (M3Rig, Backup) {
    let variant = latest_variant();
    let rig = M3Rig::start_on(
        root,
        &variant,
        &variant.sub(base),
        back,
        datetime!(2026-12-01 09:00:00 UTC),
        |opts| opts.enable_scheduler = false,
    );
    let dest = rig.add_destination("Slow Vault", |_| {});
    let backup = rig.add_backup_to(&dest, "Documents", |_| {});
    DelayedDaemon::start_with(&session_address(), front, back, delay_of);
    (rig, backup)
}

/// Runs the smoke `scenario` in English against the daemon stand-in `front`.
fn run_slow_scene(
    root: &Path,
    scenario: &str,
    front: &str,
    extra_args: &[&str],
    env: &[(&str, &str)],
) {
    let mut args = vec!["--bus-name", front, "--no-autostart"];
    args.extend_from_slice(extra_args);
    SmokeContext {
        kestic_bin: &find_kestic_binary(),
        iso: &IsolatedEnv::adopt(root),
        dbus_addr: &session_address(),
    }
    .run_with_env(
        scenario,
        "wide",
        "en",
        &format!("target/smoke-artifacts/{scenario}-wide-en.png"),
        &args,
        env,
    );
}

/// AC-M6-33-1: every reply comes 5 s late. (a) The edits and lists whose result the window waits
/// for (rename, settings, run detail, destination types, known networks, snapshot list) return at
/// once and the main thread never pauses longer than 100 ms; (b) their results arrive in the
/// window.
#[test]
fn ac_m6_33_1_edits_and_lists_do_not_wait_for_a_slow_daemon() {
    run_isolated_test(
        "ac_m6_33_1_edits_and_lists_do_not_wait_for_a_slow_daemon",
        "KESTIC_M6_S33_SLOW_EDITS_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let front = "net.daester.KesticSlowEditsFront";
            let (rig, backup) = slow_rig(
                root,
                "SlowEdits",
                "net.daester.KesticSlowEditsBack",
                front,
                |_, _| SLOW_REPLY,
            );
            let client = rig.daemon.client.clone();
            client
                .start_backup(backup.id.as_str())
                .expect("start the backup");
            wait_until_within(
                Duration::from_secs(60),
                "precondition: one finished run",
                || {
                    client.get_state().is_ok_and(|s| {
                        s.recent_runs
                            .iter()
                            .any(|r| r.status != kestic_core::model::RunStatus::Running)
                    })
                },
            );
            run_slow_scene(root, "m6-slow-edits", front, &[], &[]);
            rig.daemon.stop();
        },
    );
}

/// AC-M6-33-1: every reply comes 5 s late. (a) The previews, exclude helpers, source checks,
/// connection test, restic source and the snapshot query of a folder list return at once and the
/// main thread never pauses longer than 100 ms; (b) their answers arrive (the dry run's id, the
/// source check, the folder list of the snapshot).
#[test]
fn ac_m6_33_1_queries_and_previews_do_not_wait_for_a_slow_daemon() {
    run_isolated_test(
        "ac_m6_33_1_queries_and_previews_do_not_wait_for_a_slow_daemon",
        "KESTIC_M6_S33_SLOW_QUERIES_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let front = "net.daester.KesticSlowQueriesFront";
            let (rig, backup) = slow_rig(
                root,
                "SlowQueries",
                "net.daester.KesticSlowQueriesBack",
                front,
                |_, _| SLOW_REPLY,
            );
            let client = rig.daemon.client.clone();
            client
                .start_backup(backup.id.as_str())
                .expect("start the backup");
            next_finished_run(&client, &BTreeSet::new());
            let snapshot = client
                .list_backup_snapshots(backup.id.as_str())
                .expect("list the snapshots")
                .current_snapshots
                .first()
                .map(|s| s.id.clone())
                .expect("precondition: one snapshot");
            let probe = root.join("probe-repo");
            let source = backup.source.path.to_string_lossy().to_string();
            run_slow_scene(
                root,
                "m6-slow-queries",
                front,
                &[],
                &[
                    ("KESTIC_TEST_BACKUP", backup.id.as_str()),
                    ("KESTIC_TEST_DEST", backup.destination_id.as_str()),
                    ("KESTIC_TEST_SNAPSHOT", &snapshot),
                    ("KESTIC_TEST_SOURCE", &source),
                    ("KESTIC_TEST_PROBE", probe.to_str().expect("utf-8 path")),
                ],
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M6-33-1 (b): every reply comes 5 s late; answers that arrive after the window moved on do
/// not count. (a) A connection test answered after the location changed leaves the form untested;
/// (b) a dry run whose page closed before it started is cancelled; (c) a further page answered
/// after the filter changed does not stop the folder list from loading more.
#[test]
fn ac_m6_33_1_late_answers_do_not_count_after_the_window_moved_on() {
    run_isolated_test(
        "ac_m6_33_1_late_answers_do_not_count_after_the_window_moved_on",
        "KESTIC_M6_S33_SLOW_STALE_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let front = "net.daester.KesticSlowStaleFront";
            let (rig, backup) = slow_rig(
                root,
                "SlowStale",
                "net.daester.KesticSlowStaleBack",
                front,
                |_, _| SLOW_REPLY,
            );
            for n in 0..250 {
                fs::write(backup.source.path.join(format!("file-{n}")), b"x").expect("source file");
            }
            let client = rig.daemon.client.clone();
            client
                .start_backup(backup.id.as_str())
                .expect("start the backup");
            next_finished_run(&client, &BTreeSet::new());
            let snapshot = client
                .list_backup_snapshots(backup.id.as_str())
                .expect("list the snapshots")
                .current_snapshots
                .first()
                .map(|s| s.id.clone())
                .expect("precondition: one snapshot");
            let me = client
                .connection()
                .unique_name()
                .expect("test client name")
                .to_string();
            let seen = monitor_kestic_calls(&session_address());
            let probe = root.join("probe-repo");
            // Two empty folders a new repository can go into: a test of either succeeds.
            fs::create_dir_all(probe.join("a")).expect("probe folder a");
            fs::create_dir_all(probe.join("b")).expect("probe folder b");
            let source = backup.source.path.to_string_lossy().to_string();
            run_slow_scene(
                root,
                "m6-slow-stale",
                front,
                &[],
                &[
                    ("KESTIC_TEST_BACKUP", backup.id.as_str()),
                    ("KESTIC_TEST_DEST", backup.destination_id.as_str()),
                    ("KESTIC_TEST_SNAPSHOT", &snapshot),
                    ("KESTIC_TEST_SOURCE", &source),
                    ("KESTIC_TEST_PROBE", probe.to_str().expect("utf-8 path")),
                ],
            );
            wait_until_within(
                Duration::from_secs(15),
                "(b) the dry run nobody follows was cancelled",
                || !calls_of(&seen, "CancelBackupDryRun", &me).is_empty(),
            );
            rig.daemon.stop();
        },
    );
}

/// The id of a run that has finished and is not in `known`, once there is one.
fn next_finished_run(client: &KesticClient, known: &BTreeSet<String>) -> String {
    let mut found = String::new();
    wait_until_within(Duration::from_secs(60), "a run finished", || {
        let finished = client.get_state().ok().and_then(|s| {
            s.recent_runs
                .iter()
                .find(|r| {
                    r.status != kestic_core::model::RunStatus::Running
                        && !known.contains(r.id.as_str())
                })
                .map(|r| r.id.as_str().to_string())
        });
        finished.map(|id| found = id).is_some()
    });
    found
}

/// AC-M6-33-1 (b): replies that arrive late or out of order: the window shows the run and the
/// snapshot list it asked for last, and an edit saved while an earlier one (a rename) is still on
/// its way does not undo it.
#[test]
fn ac_m6_33_1_late_replies_do_not_show_stale_data_or_undo_edits() {
    run_isolated_test(
        "ac_m6_33_1_late_replies_do_not_show_stale_data_or_undo_edits",
        "KESTIC_M6_S33_SLOW_ORDER_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let front = "net.daester.KesticSlowOrderFront";
            let held: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let held_for_delay = Arc::clone(&held);
            let (rig, backup_a) = slow_rig(
                root,
                "SlowOrder",
                "net.daester.KesticSlowOrderBack",
                front,
                move |member, first| {
                    let late = held_for_delay
                        .lock()
                        .expect("held ids")
                        .iter()
                        .any(|id| id == first);
                    match member {
                        "RenameBackup" => Duration::from_secs(3),
                        "GetRunDetail" | "ListBackupSnapshots" if late => Duration::from_secs(4),
                        _ => Duration::ZERO,
                    }
                },
            );
            let dest = rig.add_destination("Order Vault B", |_| {});
            let backup_b = rig.add_backup_to(&dest, "Pictures", |_| {});
            let client = rig.daemon.client.clone();
            client.start_backup(backup_a.id.as_str()).expect("start A");
            let run_a = next_finished_run(&client, &BTreeSet::new());
            client.start_backup(backup_b.id.as_str()).expect("start B");
            let run_b = next_finished_run(&client, &BTreeSet::from([run_a.clone()]));
            held.lock()
                .expect("held ids")
                .extend([run_a.clone(), backup_a.id.as_str().to_string()]);
            run_slow_scene(
                root,
                "m6-slow-order",
                front,
                &[],
                &[
                    ("KESTIC_TEST_BACKUP_A", backup_a.id.as_str()),
                    ("KESTIC_TEST_BACKUP_B", backup_b.id.as_str()),
                    ("KESTIC_TEST_RUN_A", &run_a),
                    ("KESTIC_TEST_RUN_B", &run_b),
                ],
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M6-33-1 (b): the state comes 2 s late and saving a backup 3 s late. (a) A backup added in
/// the answer of adding its local destination (as the wizard does) gets that destination's run
/// conditions (no network needed); (b) while exclude rules are saved, «Save» is off and a second
/// save starts nothing, so a change made after the answer stays (checked in the smoke scene).
#[test]
fn ac_m6_33_1_saves_wait_for_their_answer() {
    run_isolated_test(
        "ac_m6_33_1_saves_wait_for_their_answer",
        "KESTIC_M6_S33_SLOW_SAVE_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let front = "net.daester.KesticSlowSaveFront";
            let (rig, _) = slow_rig(
                root,
                "SlowSave",
                "net.daester.KesticSlowSaveBack",
                front,
                |member, _| match member {
                    "GetState" => Duration::from_secs(2),
                    "UpdateBackup" => Duration::from_secs(3),
                    _ => Duration::ZERO,
                },
            );
            let repo = root.join("added-repo");
            let source = root.join("added-source");
            std::fs::create_dir_all(&source).expect("source folder");
            std::fs::create_dir_all(&repo).expect("empty repository folder");
            run_slow_scene(
                root,
                "m6-slow-save",
                front,
                &[],
                &[
                    ("KESTIC_TEST_REPO", repo.to_str().expect("utf-8 path")),
                    ("KESTIC_TEST_SOURCE", source.to_str().expect("utf-8 path")),
                ],
            );
            let state = rig.daemon.client.get_state().expect("state");
            let added = state
                .backups
                .iter()
                .find(|b| b.backup.name == "Added")
                .expect("the added backup");
            assert!(
                !added.backup.run_conditions.require_network,
                "(a) the backup added right after its local destination needs no network"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M6-33-1 (b): the quit dialog of the tray opens before the first state arrived; the state
/// that arrives later names the backup that runs.
#[test]
fn ac_m6_33_1_quit_dialog_names_the_running_backup_once_the_state_arrives() {
    run_isolated_test(
        "ac_m6_33_1_quit_dialog_names_the_running_backup_once_the_state_arrives",
        "KESTIC_M6_S33_QUIT_RUNNING_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let front = "net.daester.KesticQuitFront";
            let (rig, backup) = slow_rig(
                root,
                "QuitRunning",
                "net.daester.KesticQuitBack",
                front,
                |_, _| Duration::from_secs(2),
            );
            rig.wrapper.hold("backup");
            rig.daemon
                .client
                .start_backup(backup.id.as_str())
                .expect("start the backup");
            wait_until_within(
                Duration::from_secs(20),
                "precondition: the backup runs",
                || {
                    rig.daemon
                        .client
                        .get_state()
                        .is_ok_and(|s| s.overall.active_run_progress.is_some())
                },
            );
            run_slow_scene(root, "m6-quit-running", front, &["--confirm-quit"], &[]);
            rig.wrapper.release("backup");
            rig.daemon.stop();
        },
    );
}

/// AC-M6-33-1 (b): the daemon's password prompt (`UnlockSecretStore`, answered after 30 s)
/// does not hold back the user's other actions.
#[test]
fn ac_m6_33_1_password_prompt_does_not_hold_back_other_actions() {
    run_isolated_test(
        "ac_m6_33_1_password_prompt_does_not_hold_back_other_actions",
        "KESTIC_M6_S33_PROMPT_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let front = "net.daester.KesticPromptFront";
            let (rig, _) = slow_rig(
                root,
                "Prompt",
                "net.daester.KesticPromptBack",
                front,
                |member, _| {
                    if member == "UnlockSecretStore" {
                        Duration::from_secs(30)
                    } else {
                        Duration::ZERO
                    }
                },
            );
            run_slow_scene(root, "m6-prompt-alone", front, &[], &[]);
            rig.daemon.stop();
        },
    );
}

/// AC-M6-33-1 (b): the window closes while calls are still on their way; the daemon still learns
/// that the window closed (`WindowActive(false)`, not held back by an earlier `WindowActive`), and
/// a call queued behind a slow one is still sent before the process ends.
#[test]
fn ac_m6_33_1_closing_window_still_reaches_the_daemon() {
    run_isolated_test(
        "ac_m6_33_1_closing_window_still_reaches_the_daemon",
        "KESTIC_M6_S33_CLOSE_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let front = "net.daester.KesticCloseFront";
            let (rig, _) = slow_rig(
                root,
                "Close",
                "net.daester.KesticCloseBack",
                front,
                |member, _| match member {
                    "WindowActive" => Duration::from_secs(30),
                    "PauseGlobal" => Duration::from_secs(2),
                    _ => Duration::ZERO,
                },
            );
            let me = rig
                .daemon
                .client
                .connection()
                .unique_name()
                .expect("test client name")
                .to_string();
            let seen = monitor_kestic_calls(&session_address());
            run_slow_scene(root, "m6-close-window", front, &[], &[]);
            // The monitor reads the last calls of the ended GUI on its own thread.
            wait_until_within(
                Duration::from_secs(5),
                "(b) the daemon learned that the window closed",
                || {
                    calls_of(&seen, "WindowActive", &me)
                        .iter()
                        .any(|c| c.active == Some(false))
                },
            );
            wait_until_within(
                Duration::from_secs(5),
                "(b) a call queued when the window closed was still sent",
                || !calls_of(&seen, "ResumeGlobal", &me).is_empty(),
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M6-33-1 (b): «Back Up Now» and «Back Up All Now» answered late (`StartBackup` and
/// `StartAllBackups` held back 30 s) do not hold back pausing that backup or all backups.
#[test]
fn ac_m6_33_1_slow_start_does_not_hold_back_pausing() {
    run_isolated_test(
        "ac_m6_33_1_slow_start_does_not_hold_back_pausing",
        "KESTIC_M6_S33_SLOW_START_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let front = "net.daester.KesticSlowStartFront";
            let (rig, _) = slow_rig(
                root,
                "SlowStart",
                "net.daester.KesticSlowStartBack",
                front,
                |member, _| {
                    if member == "StartBackup" || member == "StartAllBackups" {
                        Duration::from_secs(30)
                    } else {
                        Duration::ZERO
                    }
                },
            );
            run_slow_scene(root, "m6-slow-start", front, &[], &[]);
            rig.daemon.stop();
        },
    );
}

/// AC-M6-33-1 (b): «Back Up Now» before the first state arrived, for a backup whose source is
/// gone: the refusal names the backup and the reason the daemon gave.
#[test]
fn ac_m6_33_1_refused_start_names_the_backup_and_the_reason() {
    run_isolated_test(
        "ac_m6_33_1_refused_start_names_the_backup_and_the_reason",
        "KESTIC_M6_S33_REFUSED_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let front = "net.daester.KesticRefusedFront";
            let (rig, backup) = slow_rig(
                root,
                "Refused",
                "net.daester.KesticRefusedBack",
                front,
                |_, _| Duration::from_secs(3),
            );
            fs::remove_dir_all(&backup.source.path).expect("remove the source");
            run_slow_scene(
                root,
                "m6-refused-start",
                front,
                &[],
                &[("KESTIC_TEST_BACKUP_ID", backup.id.as_str())],
            );
            rig.daemon.stop();
        },
    );
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
            let dbus_addr = session_address();
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
            let seen = monitor_kestic_calls(&session_address());
            let _gui = start_gui(bus, &session_address());
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

            // (a) in the window: the scene `m6-state-change` sees the change within 1 s.
            let marker = root.join("state-change");
            let scene = {
                let root = root.to_path_buf();
                let marker = marker.display().to_string();
                let bus = bus.to_string();
                thread::spawn(move || {
                    SmokeContext {
                        kestic_bin: &find_kestic_binary(),
                        iso: &IsolatedEnv::adopt(&root),
                        dbus_addr: &session_address(),
                    }
                    .run_with_env(
                        "m6-state-change",
                        "wide",
                        "en",
                        "target/smoke-artifacts/m6-state-change-wide-en.png",
                        &["--bus-name", &bus, "--no-autostart"],
                        &[("KESTIC_TEST_STATE_CHANGE_MARKER", &marker)],
                    );
                })
            };
            let ready = marker.with_extension("ready");
            wait_until_within(Duration::from_secs(60), "the scene is up", || {
                ready.exists()
            });
            let expert = client.get_state().expect("GetState").config.expert_mode;
            fs::write(marker.with_extension("go"), b"1").expect("write go marker");
            client.set_expert_mode(!expert).expect("SetExpertMode");
            scene
                .join()
                .expect("(a) the window showed the change within 1 s");
            rig.daemon.stop();
        },
    );
}

/// Starts `kestic daemon` as a process of its own on `bus` of the session bus that `conn` is
/// connected to (at `address`) and waits until it owns the name.
fn start_daemon_process(conn: &zbus::Connection, bus: &str, address: &str) -> GuiProcess {
    let daemon = GuiProcess(
        Command::new(find_kestic_binary())
            .args(["daemon", "--bus-name", bus])
            .env("DBUS_SESSION_BUS_ADDRESS", address)
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
            let address = session_address();
            let seen = monitor_kestic_calls(&address);
            let mut first = start_daemon_process(&fakes.sni_conn, bus, &address);
            let _gui = start_gui(bus, &address);
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
            let _second = start_daemon_process(&fakes.sni_conn, bus, &address);
            wait_until_within(
                Duration::from_secs(10),
                "the GUI registers with the new daemon",
                || calls_of(&seen, "RegisterGui", "").len() > registered_before,
            );

            let client =
                KesticClient::connect(bus, KESTIC_OBJECT_PATH).expect("client of the new daemon");
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

/// Whether process `pid` has ended. A process that ended but was not reaped (a zombie: in a
/// container nobody may reap the forked bus) counts as ended.
fn process_gone(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |stat| {
        stat.rsplit_once(") ")
            .and_then(|(_, rest)| rest.chars().next())
            .is_some_and(|state| state == 'Z' || state == 'X')
    })
}

/// The session bus goes away while the GUI is idle, and a new one starts at the same address:
/// the GUI opens a new connection, registers with the new daemon and receives its changes.
#[test]
fn idle_gui_reconnects_after_the_session_bus_restarts() {
    run_isolated_test(
        "idle_gui_reconnects_after_the_session_bus_restarts",
        "KESTIC_M6_S33_BUS_RESTART_INNER",
        |root| {
            let address = format!("unix:path={}", root.join("restartable-bus").display());
            let bus = "net.daester.KesticBusRestart";
            let first_bus = PrivateDbusGuard::start_at(&address);
            let first_conn = connect(&address);
            let seen = monitor_kestic_calls(&address);
            let first = start_daemon_process(&first_conn, bus, &address);
            let _gui = start_gui(bus, &address);
            wait_until_within(
                Duration::from_secs(40),
                "precondition: the GUI registered with the daemon",
                || !calls_of(&seen, "RegisterGui", "").is_empty(),
            );

            let old_bus = first_bus.pid;
            drop(first_bus);
            wait_until_within(Duration::from_secs(10), "the first bus is gone", || {
                process_gone(old_bus)
            });
            drop(first);
            let _second_bus = PrivateDbusGuard::start_at(&address);
            let conn = connect(&address);
            let seen = monitor_kestic_calls(&address);
            let _second = start_daemon_process(&conn, bus, &address);
            wait_until_within(
                Duration::from_secs(15),
                "the GUI registers on the new bus",
                || !calls_of(&seen, "RegisterGui", "").is_empty(),
            );

            let client = KesticClient::from_connection(conn, bus, KESTIC_OBJECT_PATH);
            let me = client
                .connection()
                .unique_name()
                .expect("test client name")
                .to_string();
            let before = calls_of(&seen, "GetState", &me).len();
            client.set_expert_mode(true).expect("SetExpertMode");
            wait_until_within(
                Duration::from_secs(5),
                "the GUI fetches the state on the new bus after a change",
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

            // Each language gets its own hold, released 8 s after its GUI started.
            let iso = IsolatedEnv::adopt(root);
            for lang in ["de", "en"] {
                rig.wrapper.hold("version");
                let releaser = {
                    let marker = format!("{}.version", rig.wrapper.hold.display());
                    thread::spawn(move || {
                        thread::sleep(VERSION_HOLD);
                        let _ = fs::remove_file(marker);
                    })
                };
                SmokeContext {
                    kestic_bin: &find_kestic_binary(),
                    iso: &iso,
                    dbus_addr: &session_address(),
                }
                .run_with_env(
                    "m6-transparent-command",
                    "wide",
                    lang,
                    &format!("target/smoke-artifacts/m6-transparent-command-wide-{lang}.png"),
                    &["--bus-name", bus, "--no-autostart"],
                    &[],
                );
                releaser.join().expect("release thread");
            }
            rig.daemon.stop();
        },
    );
}
