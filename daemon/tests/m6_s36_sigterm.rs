// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M6-S36 `SIGTERM`: the daemon stops every restic it started, gives each the chance to release
//! its lock, records its runs as interrupted and ends (AC-M6-36-1 to AC-M6-36-4).
//!
//! The binary tests send the signal to a `kestic daemon` process. The in-process tests install
//! the same handler in the (isolated) test process and send the signal to it, so the wrapper
//! around restic can record what every restic process received.

mod common;

use common::revision::UpdateNow;

use common::m3::{
    LocalBackupFixture, M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, StdFakes,
    add_folder_backup, add_local_backup, daemon_paths, latest_variant, notifications_for,
    restic_cmd, restic_variants, runtime_state, seed_local_destination, send_signal,
    spawn_m3_daemon, wait_until, wait_until_within, write_incompressible_dataset,
};
use common::m4::{RawSnapshot, backup_dir};
use common::release::serve;
use common::{
    find_kestic_binary, run_isolated_test, start_fake_notifications, start_fake_sni_watcher,
    wait_for_file,
};
use kestic_core::config::{AppConfig, save_config};
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationKind, ResticProvider, Run, RunKind, RunStatus,
    Schedule, Source,
};
use kestic_core::restic_install::{app_managed_binary, kestic_data_dir};
use kestic_core::restore::RestoreTarget;
use kestic_daemon::dbus_api::{
    BackendSecrets, KESTIC_OBJECT_PATH, KesticClient, RestoreRequest, SnapshotQueryRequest,
};
use kestic_daemon::service::{DaemonOutcome, install_stop_signal};
use kestic_test_support::release_mock::{ReleaseFiles, Route, TestSigningKey, asset_path};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use time::macros::datetime;

/// 09:00 in Zurich: the daily 12:00 slots of the backups are three hours away.
const NOW: time::OffsetDateTime = datetime!(2026-12-01 08:00:00 UTC);

/// Wall-clock milliseconds, as the restic wrapper logs them.
fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after the epoch")
        .as_millis()
}

fn alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            let rest = &stat[stat.rfind(')')? + 1..];
            rest.split_whitespace().next().map(|s| s != "Z")
        })
        .unwrap_or(false)
}

/// Processes whose command line mentions `needle` (restic working on a repository).
fn processes_mentioning(needle: &str) -> Vec<u32> {
    fs::read_dir("/proc")
        .expect("read /proc")
        .map_while(Result::ok)
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| *pid != std::process::id())
        .filter(|pid| {
            fs::read(format!("/proc/{pid}/cmdline"))
                .is_ok_and(|cmd| String::from_utf8_lossy(&cmd).contains(needle))
        })
        .filter(|pid| alive(*pid))
        .collect()
}

fn history_runs(state_dir: &Path) -> Vec<Run> {
    RunHistory::open(state_dir)
        .expect("open history")
        .list_runs()
        .expect("list runs")
}

fn runs_of(runs: &[Run], pred: impl Fn(&Run) -> bool) -> Vec<Run> {
    runs.iter().filter(|r| pred(r)).cloned().collect()
}

/// Waits at most `limit` for `child` to end and returns its status.
fn wait_exit(child: &mut Child, limit: Duration) -> (ExitStatus, Duration) {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return (status, start.elapsed());
        }
        assert!(
            start.elapsed() < limit,
            "the daemon did not end within {limit:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// A `kestic daemon` process with its own bus name, config and state.
struct BinaryDaemon {
    child: Child,
    ready: PathBuf,
    stderr: PathBuf,
}

impl BinaryDaemon {
    fn spawn(
        binary: &Path,
        root: &Path,
        sub: &str,
        bus: &str,
        extra_args: &[&str],
        env: &[(&str, &str)],
    ) -> Self {
        let (cfg_path, state_dir) = daemon_paths(root, sub);
        let dir = root.join(sub);
        fs::create_dir_all(&dir).expect("mkdir daemon dir");
        let ready = dir.join("daemon.ready");
        let _ = fs::remove_file(&ready);
        let stderr = dir.join(format!("daemon-{}.stderr", now_ms()));
        let mut cmd = Command::new(binary);
        cmd.args([
            "daemon",
            "--no-monitors",
            "--no-job-view",
            "--no-portal-inhibit",
            "--suppress-ui-spawn",
            "--bus-name",
            bus,
            "--config-path",
            &cfg_path.to_string_lossy(),
            "--state-dir",
            &state_dir.to_string_lossy(),
            "--ready-file",
            &ready.to_string_lossy(),
        ])
        .args(extra_args)
        .envs(env.iter().copied())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&stderr).expect("daemon stderr file"));
        let child = cmd.spawn().expect("spawn kestic daemon");
        Self {
            child,
            ready,
            stderr,
        }
    }

    fn wait_ready(&self) {
        wait_for_file(&self.ready, Duration::from_secs(20));
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.stderr).unwrap_or_default()
    }
}

impl Drop for BinaryDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Seeds an overdue daily backup of a large source on a local destination (`restic` as the
/// custom restic), so a daemon that starts with its scheduler runs it at once.
fn seed_overdue_large_backup(
    root: &Path,
    variant: &ResticVariant,
    sub: &str,
) -> (Backup, PathBuf, String) {
    let (cfg_path, state_dir) = daemon_paths(root, sub);
    let repo_dir = root.join(sub).join("repo");
    let src_dir = root.join(sub).join("src");
    write_incompressible_dataset(&src_dir, 192, 4 * 1024 * 1024);
    let (dest, password) = seed_local_destination(root, &variant.bin, "Large", &repo_dir);
    let mut backup =
        Backup::new("Large", Source::folder(&src_dir), dest.id.clone()).expect("backup");
    backup.schedule = Schedule::Daily {
        hour: 12,
        minute: 0,
    };
    let mut cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: variant.bin.clone(),
        },
        ..Default::default()
    };
    cfg.destinations.push(dest);
    cfg.backups.push(backup.clone());
    save_config(&cfg_path, &cfg).expect("save config");
    let history = RunHistory::open(&state_dir).expect("open history");
    let mut state = history
        .get_backup_runtime_state(&backup.id)
        .expect("runtime state");
    state.baseline_at = Some(time::OffsetDateTime::now_utc() - time::Duration::days(2));
    history
        .upsert_backup_runtime_state(&backup.id, &state)
        .expect("seed baseline");
    (backup, repo_dir, password)
}

fn lock_files(repo_dir: &Path) -> usize {
    fs::read_dir(repo_dir.join("locks")).map_or(0, |d| d.count())
}

/// AC-M6-36-1 (a)–(e): a scheduled backup ends cleanly when the daemon process gets `SIGTERM`.
#[test]
fn ac_m6_36_1_sigterm_ends_a_backup_cleanly() {
    run_isolated_test(
        "ac_m6_36_1_sigterm_ends_a_backup_cleanly",
        "KESTIC_M6_36_1_INNER",
        |root| {
            let _sni = start_fake_sni_watcher();
            let (_notif_conn, notifications) = start_fake_notifications();
            let binary = find_kestic_binary();
            for variant in restic_variants() {
                let sub = variant.sub("S36A");
                let bus = variant.bus_name("S36A");
                let (backup, repo_dir, password) = seed_overdue_large_backup(root, &variant, &sub);
                let (_, state_dir) = daemon_paths(root, &sub);
                let repo_text = repo_dir.to_string_lossy().into_owned();

                let mut daemon = BinaryDaemon::spawn(&binary, root, &sub, &bus, &[], &[]);
                daemon.wait_ready();
                wait_until_within(
                    Duration::from_secs(20),
                    "the catch-up backup holds a lock",
                    || lock_files(&repo_dir) > 0,
                );
                thread::sleep(Duration::from_millis(300));
                assert!(
                    !processes_mentioning(&repo_text).is_empty(),
                    "precondition: restic runs"
                );

                send_signal(daemon.pid(), "TERM");
                let (status, _) = wait_exit(&mut daemon.child, Duration::from_secs(15));

                // (a) restic has ended, and it saw the signal.
                assert!(
                    processes_mentioning(&repo_text).is_empty(),
                    "AC-M6-36-1 (a): no restic left on {repo_text} ({}):\n{}",
                    variant.label,
                    daemon.log()
                );
                let runs = history_runs(&state_dir);
                let run = runs_of(&runs, |r| r.backup_id() == Some(&backup.id))
                    .into_iter()
                    .next()
                    .expect("the backup's run");
                let log = run
                    .log_path
                    .as_ref()
                    .and_then(|p| fs::read_to_string(p).ok())
                    .unwrap_or_default();
                assert!(
                    log.contains("signal terminated received"),
                    "AC-M6-36-1 (a): restic logs the SIGTERM ({}): {log}",
                    variant.label
                );
                // (b) no lock left.
                let locks = restic_cmd(&variant.bin, &repo_dir, &password)
                    .args(["list", "locks", "--no-lock", "--no-cache"])
                    .output()
                    .expect("restic list locks");
                assert!(locks.status.success(), "restic list locks");
                assert_eq!(
                    String::from_utf8_lossy(&locks.stdout).trim(),
                    "",
                    "AC-M6-36-1 (b): no lock in the repository ({})",
                    variant.label
                );
                // (c) interrupted with an end time, written by the ending daemon.
                assert_eq!(
                    run.status,
                    RunStatus::Interrupted,
                    "AC-M6-36-1 (c) ({})",
                    variant.label
                );
                assert!(
                    run.finished_at.is_some(),
                    "AC-M6-36-1 (c): end time ({})",
                    variant.label
                );
                // (d) the slot is not handled: no backoff, no failure, no failure notification.
                let rs = runtime_state(&state_dir, &backup.id);
                assert_eq!(rs.backoff_attempt, 0, "AC-M6-36-1 (d): no backoff");
                assert_eq!(rs.last_failure_at, None, "AC-M6-36-1 (d): no failure");
                for event in ["backupFailed", "backupWarning"] {
                    assert!(
                        notifications_for(&notifications, event).is_empty(),
                        "AC-M6-36-1 (d): no {event} notification"
                    );
                }
                // (e) exit code 0.
                assert_eq!(status.code(), Some(0), "AC-M6-36-1 (e) ({})", variant.label);

                // (d) after a restart the backup runs again at once.
                let mut again = BinaryDaemon::spawn(&binary, root, &sub, &bus, &[], &[]);
                again.wait_ready();
                wait_until_within(Duration::from_secs(20), "the backup runs again", || {
                    runs_of(&history_runs(&state_dir), |r| {
                        r.backup_id() == Some(&backup.id) && r.id != run.id
                    })
                    .iter()
                    .any(|r| r.status != RunStatus::Pending)
                });
                send_signal(again.pid(), "TERM");
                let (again_status, _) = wait_exit(&mut again.child, Duration::from_secs(15));
                assert_eq!(again_status.code(), Some(0));
            }
        },
    );
}

/// The in-process daemon of the AC-M6-36-2/-3 tests, with the `SIGTERM` handler of this
/// process and the restic wrapper as its restic.
struct SignalRig {
    daemon: M3Daemon,
    wrapper: ResticWrapper,
    clock: Arc<kestic_core::clock::FakeClock>,
    _fakes: StdFakes,
}

fn signal_rig(root: &Path, sub: &str, bus: &str, variant: &ResticVariant) -> SignalRig {
    let stop = install_stop_signal().expect("SIGTERM handler");
    let fakes = StdFakes::start_with_power(false, 80.0);
    let wrapper = ResticWrapper::create(&root.join(sub).join("wrapper"), &variant.bin);
    let clock = Arc::new(kestic_core::clock::FakeClock::new(NOW));
    let mut opts = M3DaemonOpts::new(sub, bus, &wrapper.path, clock.clone());
    opts.stop_signal = Some(stop);
    let daemon = spawn_m3_daemon(root, opts);
    SignalRig {
        daemon,
        wrapper,
        clock,
        _fakes: fakes,
    }
}

/// Sends `SIGTERM` to this test process (the in-process daemon's handler) and returns when.
fn signal_self() -> (Instant, u128) {
    let at = (Instant::now(), now_ms());
    send_signal(std::process::id(), "TERM");
    at
}

fn new_folder(root: &Path, name: &str) -> RestoreTarget {
    RestoreTarget::NewFolder {
        dir: root.join(name).to_string_lossy().into_owned(),
    }
}

fn restore_request(dest: &Destination, snapshot_id: &str, target: RestoreTarget) -> RestoreRequest {
    RestoreRequest {
        destination_id: dest.id.as_str().to_string(),
        snapshot_id: snapshot_id.to_string(),
        selection: Vec::new(),
        target,
        overwrite: Default::default(),
        preview_id: None,
    }
}

/// Every D-Bus call that would start restic, made while the daemon stops; returns how many the
/// daemon answered with an error.
fn calls_that_would_start_restic(
    client: &KesticClient,
    a: &Destination,
    b: &Destination,
    root: &Path,
    b_password: &str,
    a_backup: &BackupId,
    snapshot_b: &str,
) -> Vec<(&'static str, bool)> {
    let new_dest = Destination::new(
        "New",
        DestinationKind::Local {
            path: root.join("new-repo"),
        },
    )
    .expect("dest");
    vec![
        (
            "StartBackup",
            client.start_backup(a_backup.as_str()).is_err(),
        ),
        (
            "StartRestore",
            client
                .start_restore(&restore_request(b, snapshot_b, new_folder(root, "after")))
                .is_err(),
        ),
        (
            "StartSnapshotQuery",
            client
                .start_snapshot_query(&SnapshotQueryRequest::ls(b.id.as_str(), snapshot_b, "/"))
                .is_err(),
        ),
        (
            "TestDestinationConnection",
            !matches!(
                client.test_destination_connection(
                    b,
                    "connect",
                    b_password,
                    &BackendSecrets::default()
                ),
                Ok(r) if r.ok
            ),
        ),
        (
            "AddDestination",
            client
                .add_destination(&new_dest, "connect", b_password, &BackendSecrets::default())
                .is_err(),
        ),
        (
            "UpdateDestination",
            client
                .update_destination_now(a, "connect", "", &BackendSecrets::default(), true)
                .is_err(),
        ),
        (
            "UpdateDestinationPassword",
            client
                .update_destination_password(b.id.as_str(), b_password)
                .is_err(),
        ),
    ]
}

/// What the daemon of the AC-M6-36-2 test is busy with when the signal comes.
struct Busy {
    a: LocalBackupFixture,
    a2: Backup,
    b: LocalBackupFixture,
    snap_b: String,
    tester: thread::JoinHandle<bool>,
    install_id: String,
    managed: PathBuf,
    a2_state_before: kestic_core::history::BackupRuntimeState,
    _mock: kestic_test_support::release_mock::ReleaseMock,
}

/// Destination A: maintenance (its `forget` lingers 3 s after `SIGTERM`), a restore waiting
/// behind it and both backups in the queue; destination B: a backup and a restore; plus a
/// snapshot query, a connection test and a restic installation.
fn make_busy(root: &Path, rig: &SignalRig, variant: &ResticVariant) -> Busy {
    let client = &rig.daemon.client;
    let wrapper = &rig.wrapper;
    // Destination A: a backup with `forget` pending, a second backup, a snapshot.
    let a = add_local_backup(client, &variant.bin, root, "S36B", "A1", |_| {});
    let a2 = add_folder_backup(client, &root.join("S36B/src-A2"), &a.dest.id, "A2", |_| {});
    let snap_a = backup_dir(
        &variant.bin,
        &a.repo_dir,
        &a.password,
        &a.src_dir,
        &RawSnapshot {
            tag: "",
            time: "2026-11-30 12:00:00",
            host: "h",
        },
    );
    wrapper.fail("forget", 1, "forget fails");
    client.start_backup(&a.backup_id_str).expect("A1 backup");
    wait_until("A1 ends with forget pending", || {
        client.get_state().is_ok_and(|s| {
            s.backups
                .iter()
                .any(|b| b.backup.id == a.backup.id && b.pending_forget && b.active_run.is_none())
        })
    });
    wrapper.clear_fail("forget");
    // Destination B: a backup and a snapshot.
    let b = add_local_backup(client, &variant.bin, root, "S36B", "B1", |_| {});
    let snap_b = backup_dir(
        &variant.bin,
        &b.repo_dir,
        &b.password,
        &b.src_dir,
        &RawSnapshot {
            tag: "",
            time: "2026-11-30 12:00:00",
            host: "h",
        },
    );

    // A: maintenance holds the exclusive lock (its `forget` lingers 3 s after SIGTERM),
    // a restore waits behind it.
    wrapper.hold_lingering("forget", Some(Duration::from_secs(3)), false);
    client
        .run_destination_maintenance(a.dest.id.as_str())
        .expect("maintenance A");
    wrapper.wait_for('P', "forget", 1, Duration::from_secs(10));
    client
        .start_restore(&restore_request(
            &a.dest,
            &snap_a,
            new_folder(root, "restore-a"),
        ))
        .expect("restore A waits");
    // B: a backup and a restore run.
    wrapper.hold("backup");
    client.start_backup(&b.backup_id_str).expect("B1 backup");
    wrapper.wait_for('S', "backup", 2, Duration::from_secs(10));
    wrapper.hold("restore");
    client
        .start_restore(&restore_request(
            &b.dest,
            &snap_b,
            new_folder(root, "restore-b"),
        ))
        .expect("restore B");
    wrapper.wait_for('S', "restore", 1, Duration::from_secs(10));
    // A snapshot query and a connection test.
    wrapper.hold("ls");
    client
        .start_snapshot_query(&SnapshotQueryRequest::ls(b.dest.id.as_str(), &snap_b, "/"))
        .expect("query");
    wrapper.wait_for('S', "ls", 1, Duration::from_secs(10));
    wrapper.hold("cat");
    let test_client = KesticClient::connect("net.daester.Kestic.S36B", KESTIC_OBJECT_PATH)
        .expect("second client");
    let (b_dest, b_pw) = (b.dest.clone(), b.password.clone());
    let tester = thread::spawn(move || {
        test_client
            .test_destination_connection(&b_dest, "connect", &b_pw, &BackendSecrets::default())
            .is_ok_and(|r| r.ok)
    });
    wrapper.wait_for('S', "cat", 1, Duration::from_secs(10));
    // A restic installation downloads from a server that stalls.
    let key = TestSigningKey::generate("kestic-test-release");
    let files = ReleaseFiles::signed("0.99.0", &key);
    let archive = asset_path("0.99.0", &files.archive_name("amd64"));
    let mock = serve(
        &files,
        &key,
        vec![(archive.clone(), Route::StallAfterHeaders)],
    );
    let install_id = client.start_restic_install().expect("install");
    wait_until("archive requested", || mock.requests().contains(&archive));
    let data_dir = kestic_data_dir(
        env::var_os("HOME").as_deref(),
        env::var_os("XDG_DATA_HOME").as_deref(),
    )
    .expect("data dir");
    let managed = app_managed_binary(&data_dir);
    fs::create_dir_all(managed.parent().expect("managed dir")).expect("mkdir managed dir");
    fs::write(&managed, b"previous restic").expect("write previous app-managed restic");
    // Both backups of A wait in its queue.
    client.start_all_backups().expect("start all");
    let a2_state_before = runtime_state(&rig.daemon.state_dir, &a2.id);

    Busy {
        a,
        a2,
        b,
        snap_b,
        tester,
        install_id,
        managed,
        a2_state_before,
        _mock: mock,
    }
}

/// AC-M6-36-2 (a)–(d): every restic process gets `SIGTERM`, the runs end as interrupted, nothing
/// starts after the signal, and a running restic installation is cancelled.
#[test]
fn ac_m6_36_2_sigterm_stops_every_restic_process() {
    run_isolated_test(
        "ac_m6_36_2_sigterm_stops_every_restic_process",
        "KESTIC_M6_36_2_INNER",
        |root| {
            let variant = latest_variant();
            let rig = signal_rig(root, "S36B", "net.daester.Kestic.S36B", &variant);
            let client = &rig.daemon.client;
            let wrapper = &rig.wrapper;

            let Busy {
                a,
                a2,
                b,
                snap_b,
                tester,
                install_id,
                managed,
                a2_state_before,
                _mock,
            } = make_busy(root, &rig, &variant);

            // Every held call has logged its start, so the count of restic calls on a
            // repository is complete here; a time stamp would not do, since restic runs in the
            // idle I/O class and its wrapper's log lines can come late under load.
            let repo_calls_before = wrapper.started_repo_count();
            signal_self();
            // While the daemon stops (A's `forget` lingers): a backup becomes due, and every
            // call that would start restic is made.
            thread::sleep(Duration::from_millis(300));
            // Read first: the fake clock keeps a finished installation for ten minutes only.
            let install = client
                .get_restic_install_status(&install_id)
                .expect("install status");
            rig.clock.advance(Duration::from_hours(3));
            let refused = calls_that_would_start_restic(
                client,
                &a.dest,
                &b.dest,
                root,
                &b.password,
                &a2.id,
                &snap_b,
            );
            let calls_done_ms = now_ms();

            let state_dir = rig.daemon.state_dir.clone();
            let outcome = rig.daemon.wait_stopped(Duration::from_secs(15));
            assert_eq!(outcome, Ok(DaemonOutcome::RanAndStopped));
            assert!(!tester.join().expect("connection test thread"));
            let forget_end = wrapper.of('E', "forget");
            assert!(
                forget_end.last().is_some_and(|e| e.at_ms > calls_done_ms),
                "precondition: the calls came while the daemon was stopping"
            );

            // (a) each restic process got SIGTERM.
            for sub in ["forget", "backup", "restore", "ls", "cat"] {
                assert!(
                    !wrapper.of('T', sub).is_empty(),
                    "AC-M6-36-2 (a): restic {sub} got SIGTERM: {:?}",
                    wrapper.entries()
                );
            }
            // (b) runs with a history row end as interrupted with an end time; the queued backup
            // gets none.
            let runs = history_runs(&state_dir);
            let maintenance = runs_of(&runs, |r| {
                r.kind == RunKind::Maintenance && r.destination_id == a.dest.id
            });
            let restores_a = runs_of(&runs, |r| {
                matches!(r.kind, RunKind::Restore { .. }) && r.destination_id == a.dest.id
            });
            let restores_b = runs_of(&runs, |r| {
                matches!(r.kind, RunKind::Restore { .. }) && r.destination_id == b.dest.id
            });
            let backups_b = runs_of(&runs, |r| r.backup_id() == Some(&b.backup.id));
            for (what, list) in [
                ("maintenance A", &maintenance),
                ("waiting restore A", &restores_a),
                ("restore B", &restores_b),
                ("backup B", &backups_b),
            ] {
                assert_eq!(list.len(), 1, "AC-M6-36-2 (b): one {what} run: {list:?}");
                assert_eq!(
                    list[0].status,
                    RunStatus::Interrupted,
                    "AC-M6-36-2 (b): {what} interrupted"
                );
                assert!(
                    list[0].finished_at.is_some(),
                    "AC-M6-36-2 (b): {what} has an end time"
                );
            }
            assert!(
                runs_of(&runs, |r| r.backup_id() == Some(&a2.id)).is_empty(),
                "AC-M6-36-2 (b): the queued backup has no row"
            );
            assert_eq!(
                runtime_state(&state_dir, &a2.id),
                a2_state_before,
                "AC-M6-36-2 (b): the queued backup stays due"
            );
            // (c) nothing started after the signal.
            assert_eq!(
                wrapper.started_repo_count(),
                repo_calls_before,
                "AC-M6-36-2 (c): no restic after SIGTERM: {:?}",
                wrapper.entries()
            );
            for (call, refused) in refused {
                assert!(
                    refused,
                    "AC-M6-36-2 (c): {call} is refused while the daemon stops"
                );
            }
            // (d) the installation is cancelled; the app-managed restic is unchanged.
            assert_eq!(install.state, "cancelled", "AC-M6-36-2 (d)");
            assert_eq!(
                fs::read(&managed).expect("app-managed restic"),
                b"previous restic",
                "AC-M6-36-2 (d): the app-managed restic is unchanged"
            );
        },
    );
}

/// AC-M6-36-2 (e): a backup stopped in its `forget` keeps that `forget` pending.
#[test]
fn ac_m6_36_2_e_forget_interrupted_keeps_forget_pending() {
    run_isolated_test(
        "ac_m6_36_2_e_forget_interrupted_keeps_forget_pending",
        "KESTIC_M6_36_2E_INNER",
        |root| {
            let variant = latest_variant();
            let rig = signal_rig(root, "S36E", "net.daester.Kestic.S36E", &variant);
            let client = &rig.daemon.client;
            let fx = add_local_backup(client, &variant.bin, root, "S36E", "F", |_| {});
            rig.wrapper.hold("forget");
            client.start_backup(&fx.backup_id_str).expect("backup");
            rig.wrapper
                .wait_for('S', "forget", 1, Duration::from_secs(20));
            signal_self();
            assert_eq!(
                rig.daemon.wait_stopped(Duration::from_secs(15)),
                Ok(DaemonOutcome::RanAndStopped)
            );
            let state_dir = daemon_paths(root, "S36E").1;
            let runs = runs_of(&history_runs(&state_dir), |r| {
                r.backup_id() == Some(&fx.backup.id)
            });
            assert_eq!(runs.len(), 1, "{runs:?}");
            assert_eq!(runs[0].status, RunStatus::Interrupted, "AC-M6-36-2 (e)");
            assert_eq!(
                runs[0].exit_code,
                Some(0),
                "precondition: the backup itself ended"
            );
            assert!(
                runtime_state(&state_dir, &fx.backup.id).pending_forget,
                "AC-M6-36-2 (e): forget stays pending"
            );
        },
    );
}

/// AC-M6-36-3 (a)–(e): one deadline for all restic processes, then `SIGKILL` of what is left
/// with everything it started.
#[test]
fn ac_m6_36_3_one_deadline_then_sigkill_of_the_tree() {
    run_isolated_test(
        "ac_m6_36_3_one_deadline_then_sigkill_of_the_tree",
        "KESTIC_M6_36_3_INNER",
        |root| {
            let variant = latest_variant();
            let rig = signal_rig(root, "S36C", "net.daester.Kestic.S36C", &variant);
            let client = &rig.daemon.client;
            let wrapper = &rig.wrapper;
            let first = add_local_backup(client, &variant.bin, root, "S36C", "Slow", |_| {});
            let second = add_local_backup(client, &variant.bin, root, "S36C", "Stuck", |_| {});
            // The backup of the first destination ends 4 s after SIGTERM; the `forget` of a
            // maintenance on the second never ends and has a child holding its stdout.
            wrapper.fail("forget", 1, "forget fails");
            client
                .start_backup(&second.backup_id_str)
                .expect("forget pending");
            wait_until("second ends", || {
                client.get_state().is_ok_and(|s| {
                    s.backups.iter().any(|b| {
                        b.backup.id == second.backup.id
                            && b.pending_forget
                            && b.active_run.is_none()
                    })
                })
            });
            wrapper.clear_fail("forget");
            wrapper.hold_lingering("backup", Some(Duration::from_secs(4)), true);
            wrapper.hold_lingering("forget", None, true);
            client
                .start_backup(&first.backup_id_str)
                .expect("slow backup");
            client
                .run_destination_maintenance(second.dest.id.as_str())
                .expect("stuck maintenance");
            wrapper.wait_for('C', "forget", 1, Duration::from_secs(10));
            wrapper.wait_for('P', "backup", 1, Duration::from_secs(10));
            let stuck = wrapper.lingering_pids("forget");
            assert_eq!(stuck.len(), 2, "precondition: call and child: {stuck:?}");

            let (signal_at, _) = signal_self();
            wait_until_within(Duration::from_secs(13), "the stuck tree is killed", || {
                stuck.iter().all(|pid| !alive(*pid))
            });
            let killed_after = signal_at.elapsed();
            // A lingering call that ends by itself logs its end with exit code 143; a killed one
            // logs nothing. The real restic calls before ended with other codes, and their log
            // lines may come late: restic runs in the idle I/O class, so under load the wrapper's
            // `E` line of an earlier call can carry a time after the signal.
            let ended_itself = |sub: &str| {
                wrapper
                    .of('E', sub)
                    .into_iter()
                    .filter(|e| e.detail.trim() == "143")
                    .count()
            };
            let outcome = rig.daemon.wait_stopped(Duration::from_secs(15));
            let ended_after = signal_at.elapsed();

            // (a) the slow one ended by itself and removed its marker.
            assert_eq!(
                ended_itself("backup"),
                1,
                "AC-M6-36-3 (a): no SIGKILL: {:?}",
                wrapper.entries()
            );
            assert!(
                !wrapper.lock_marker("backup").exists(),
                "AC-M6-36-3 (a): marker removed"
            );
            // (b) the stuck tree got SIGKILL at the deadline.
            assert!(
                killed_after >= Duration::from_secs(9) && killed_after <= Duration::from_secs(11),
                "AC-M6-36-3 (b): killed {killed_after:?} after the signal"
            );
            assert_eq!(
                ended_itself("forget"),
                0,
                "AC-M6-36-3 (b): killed, not ended: {:?}",
                wrapper.entries()
            );
            // (c) nothing of both trees is left.
            for pid in stuck.iter().chain(wrapper.lingering_pids("backup").iter()) {
                assert!(!alive(*pid), "AC-M6-36-3 (c): {pid} is gone");
            }
            // (d) both runs interrupted.
            let runs = history_runs(&daemon_paths(root, "S36C").1);
            let slow = runs_of(&runs, |r| r.backup_id() == Some(&first.backup.id));
            let maint = runs_of(&runs, |r| {
                r.kind == RunKind::Maintenance && r.destination_id == second.dest.id
            });
            assert_eq!(slow.len(), 1);
            assert_eq!(
                slow[0].status,
                RunStatus::Interrupted,
                "AC-M6-36-3 (d) backup"
            );
            assert_eq!(maint.len(), 1);
            assert_eq!(
                maint[0].status,
                RunStatus::Interrupted,
                "AC-M6-36-3 (d) maintenance"
            );
            // (e) the daemon ends within 15 s.
            assert_eq!(outcome, Ok(DaemonOutcome::RanAndStopped), "AC-M6-36-3 (e)");
            assert!(
                ended_after <= Duration::from_secs(15),
                "AC-M6-36-3 (e): {ended_after:?}"
            );
        },
    );
}

/// AC-M6-36-4 (d): a second `SIGTERM` does not move the deadline.
#[test]
fn ac_m6_36_4_d_second_sigterm_keeps_the_deadline() {
    run_isolated_test(
        "ac_m6_36_4_d_second_sigterm_keeps_the_deadline",
        "KESTIC_M6_36_4D_INNER",
        |root| {
            let variant = latest_variant();
            let rig = signal_rig(root, "S36D", "net.daester.Kestic.S36D", &variant);
            let fx = add_local_backup(
                &rig.daemon.client,
                &variant.bin,
                root,
                "S36D",
                "Stuck",
                |_| {},
            );
            rig.wrapper.hold_lingering("backup", None, false);
            rig.daemon
                .client
                .start_backup(&fx.backup_id_str)
                .expect("backup");
            rig.wrapper
                .wait_for('P', "backup", 1, Duration::from_secs(10));
            let stuck = rig.wrapper.lingering_pids("backup");

            let (signal_at, _) = signal_self();
            thread::sleep(Duration::from_secs(4));
            signal_self();
            wait_until_within(Duration::from_secs(13), "the stuck call is killed", || {
                stuck.iter().all(|pid| !alive(*pid))
            });
            let killed_after = signal_at.elapsed();
            assert!(
                killed_after >= Duration::from_secs(9) && killed_after <= Duration::from_secs(11),
                "AC-M6-36-4 (d): the deadline counts from the first signal: {killed_after:?}"
            );
            assert_eq!(
                rig.daemon.wait_stopped(Duration::from_secs(15)),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// AC-M6-36-4 (a)–(c), (e): the daemon process ends with exit code 0 when idle, during its start
/// and when its binary was just replaced; a second daemon takes over at once.
#[test]
fn ac_m6_36_4_sigterm_lifecycle() {
    run_isolated_test(
        "ac_m6_36_4_sigterm_lifecycle",
        "KESTIC_M6_36_4_INNER",
        |root| {
            let sni = start_fake_sni_watcher();
            let binary = find_kestic_binary();
            let bus = "net.daester.Kestic.S36L";

            // (a) idle: exit 0 within 2 s.
            let mut idle =
                BinaryDaemon::spawn(&binary, root, "idle", bus, &["--no-scheduler"], &[]);
            idle.wait_ready();
            send_signal(idle.pid(), "TERM");
            let (status, took) = wait_exit(&mut idle.child, Duration::from_secs(10));
            assert_eq!(status.code(), Some(0), "AC-M6-36-4 (a): exit code");
            assert!(
                took <= Duration::from_secs(2),
                "AC-M6-36-4 (a): ended after {took:?}"
            );

            // (b) bus name and instance lock are free: a second daemon takes over.
            let second = BinaryDaemon::spawn(&binary, root, "idle", bus, &["--no-scheduler"], &[]);
            second.wait_ready();
            let client = KesticClient::from_connection(sni.clone(), bus, KESTIC_OBJECT_PATH);
            assert!(
                client.is_bus_owned().expect("bus owned"),
                "AC-M6-36-4 (b): the second daemon owns the name"
            );
            drop(second);

            // (c) during the start: exit 0 within 5 s, and it never got ready.
            let mut starting = BinaryDaemon::spawn(
                &binary,
                root,
                "starting",
                "net.daester.Kestic.S36S",
                &["--no-scheduler"],
                &[("KESTIC_TEST_DAEMON_START_DELAY_MS", "2000")],
            );
            thread::sleep(Duration::from_millis(500));
            send_signal(starting.pid(), "TERM");
            let (status, took) = wait_exit(&mut starting.child, Duration::from_secs(10));
            assert_eq!(status.code(), Some(0), "AC-M6-36-4 (c): exit code");
            assert!(
                took <= Duration::from_secs(5),
                "AC-M6-36-4 (c): ended after {took:?}"
            );
            assert!(
                !starting.ready.exists(),
                "AC-M6-36-4 (c): it never got ready"
            );

            // (e) a replaced binary is not started after the signal.
            let bin_dir = root.join("bin");
            fs::create_dir_all(&bin_dir).expect("mkdir bin");
            let installed = bin_dir.join("kestic");
            fs::copy(&binary, &installed).expect("copy binary");
            let mut watched = BinaryDaemon::spawn(
                &installed,
                root,
                "watched",
                "net.daester.Kestic.S36W",
                &["--no-scheduler"],
                &[],
            );
            watched.wait_ready();
            fs::remove_file(&watched.ready).expect("rm ready");
            let replacement = bin_dir.join("kestic.new");
            fs::copy(&binary, &replacement).expect("copy replacement");
            fs::rename(&replacement, &installed).expect("replace binary");
            send_signal(watched.pid(), "TERM");
            let (status, _) = wait_exit(&mut watched.child, Duration::from_secs(10));
            assert_eq!(status.code(), Some(0), "AC-M6-36-4 (e): exit code");
            thread::sleep(Duration::from_secs(1));
            let watched_client = KesticClient::from_connection(
                sni.clone(),
                "net.daester.Kestic.S36W",
                KESTIC_OBJECT_PATH,
            );
            assert!(
                !watched_client.is_bus_owned().expect("bus owned"),
                "AC-M6-36-4 (e): no new daemon"
            );
            assert!(!watched.ready.exists(), "AC-M6-36-4 (e): no restart");
        },
    );
}

/// AC-M6-36-4 (f): the daemon handles the signal through `signal-hook`, without code of its own
/// that needs `unsafe`, and leaves `SIGINT` and `SIGHUP` alone.
#[test]
fn ac_m6_36_4_f_signal_handler_without_unsafe() {
    let root = kestic_test_support::workspace::workspace_root();
    let mut sources = Vec::new();
    for dir in ["daemon/src", "core/src"] {
        collect_rs(&root.join(dir), &mut sources);
    }
    let mut registrations = Vec::new();
    for path in &sources {
        let text = fs::read_to_string(path).expect("read source");
        for banned in [
            "unsafe {",
            "unsafe extern",
            "unsafe trait",
            "#[unsafe(",
            "unsafe fn",
            "unsafe impl",
            "libc::signal",
            "sigaction",
            "signal_hook::low_level",
        ] {
            assert!(
                !text.contains(banned),
                "AC-M6-36-4 (f): {} contains `{banned}`",
                path.display()
            );
        }
        for line in text
            .lines()
            .filter(|l| l.contains("signal_hook::") || l.contains("register("))
        {
            registrations.push(line.trim().to_string());
        }
    }
    assert!(
        registrations
            .iter()
            .any(|l| l.contains("signal_hook::flag::register(signal_hook::consts::SIGTERM")),
        "AC-M6-36-4 (f): SIGTERM goes through signal_hook::flag: {registrations:?}"
    );
    for other in ["SIGINT", "SIGHUP"] {
        assert!(
            !registrations.iter().any(|l| l.contains(other)),
            "AC-M6-36-4 (f): {other} keeps its default: {registrations:?}"
        );
    }
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir").map_while(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}
