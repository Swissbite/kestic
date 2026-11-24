// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Integration tests for M5-S06b: restic processes pass the run gate, so none starts while the
//! app-managed restic is being replaced, and the replacement waits for running ones.

use kestic_core::backend_credentials::BackendCredentials;
use kestic_core::clock::FakeClock;
use kestic_core::destination::probe_repository;
use kestic_core::model::{Destination, DestinationKind};
use kestic_core::provider::{ResolvedRestic, check_binary_version};
use kestic_core::restic::{ResticCommandSpec, ResticRuntime, run_restic};
use kestic_core::restic_gate::ResticBinaryGate;
use kestic_core::secrets::{InMemorySecretStore, SecretStore, SecretValue};
use kestic_test_support::tools::write_executable;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use time::macros::datetime;

/// The tests share the gate of the process (`ResticBinaryGate::global`); under `cargo test`
/// they run on threads of one process, so they take turns.
static SHARED_GATE: Mutex<()> = Mutex::new(());

fn take_turn() -> MutexGuard<'static, ()> {
    SHARED_GATE.lock().unwrap_or_else(PoisonError::into_inner)
}

const BLOCKED: Duration = Duration::from_millis(400);
const DONE: Duration = Duration::from_secs(10);

/// A restic stand-in that appends one line per start to `marker`, waits while `hold` exists and
/// answers `version --json` like restic 0.19.1.
fn stand_in(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let marker = dir.join("starts");
    let hold = dir.join("hold");
    let path = dir.join("restic");
    write_executable(
        &path,
        &format!(
            "#!/bin/sh\necho start >> '{marker}'\nwhile [ -f '{hold}' ]; do sleep 0.05; done\n\
             echo '{{\"message_type\":\"version\",\"version\":\"0.19.1\"}}'\n",
            marker = marker.display(),
            hold = hold.display(),
        ),
    );
    (path, marker, hold)
}

fn starts(marker: &Path) -> usize {
    std::fs::read_to_string(marker).map_or(0, |text| text.lines().count())
}

fn wait_for_starts(marker: &Path, n: usize) {
    let deadline = Instant::now() + DONE;
    while starts(marker) < n {
        assert!(Instant::now() < deadline, "restic did not start");
        thread::sleep(Duration::from_millis(20));
    }
}

fn local_destination(dir: &Path) -> Destination {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    Destination::new("Gate", DestinationKind::Local { path: repo }).expect("destination")
}

fn run_snapshots(restic: &Path, dir: &Path) {
    let dest = local_destination(dir);
    let mut spec =
        ResticCommandSpec::for_destination(restic, "kestic", &dest, vec!["snapshots".to_string()]);
    spec.low_priority = false;
    spec.password_command_override = Some("echo password".to_string());
    let clock = FakeClock::new(datetime!(2026-12-01 12:00:00 UTC));
    run_restic(&spec, &dest, None, &clock, None, &[], |_| {}).expect("run restic");
}

/// The connection test's `restic cat config` (its result does not matter here).
fn probe(restic: &Path, dir: &Path) {
    let dest = local_destination(dir);
    let resolved = ResolvedRestic {
        binary: restic.to_path_buf(),
        version: semver::Version::new(0, 19, 1),
    };
    let store = InMemorySecretStore::default();
    let clock = FakeClock::new(datetime!(2026-12-01 12:00:00 UTC));
    let helper = dir.join("kestic");
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
        cache_dir: None,
    };
    // Only whether and when restic starts counts; the stand-in is no repository.
    drop(probe_repository(
        &dest,
        &BackendCredentials::none(),
        &runtime,
    ));
}

/// AC-M5-06-8 (c): a run (`run_restic`), a version check and a connection test that start while
/// the binary is being replaced wait until the replacement is done.
#[test]
fn ac_m5_06_8_restic_waits_during_replacement() {
    let _turn = take_turn();
    let dir = tempfile::tempdir().expect("dir");
    let (restic, marker, _hold) = stand_in(dir.path());
    let lease = ResticBinaryGate::global()
        .replace_when_idle(&|| false)
        .expect("idle gate");

    let (run_done, run_rx) = mpsc::channel();
    let run_restic_bin = restic.clone();
    let run_dir = dir.path().to_path_buf();
    let run = thread::spawn(move || {
        run_snapshots(&run_restic_bin, &run_dir);
        run_done.send(()).expect("send");
    });
    let (check_done, check_rx) = mpsc::channel();
    let check_bin = restic.clone();
    let check = thread::spawn(move || {
        let resolved = check_binary_version(&check_bin).expect("version check");
        check_done.send(resolved.version.to_string()).expect("send");
    });

    let (probe_done, probe_rx) = mpsc::channel();
    let probe_bin = restic.clone();
    let probe_dir = dir.path().join("probe");
    let probing = thread::spawn(move || {
        probe(&probe_bin, &probe_dir);
        probe_done.send(()).expect("send");
    });

    // (c) none of them starts restic while the replace lease is held
    assert!(run_rx.recv_timeout(BLOCKED).is_err());
    assert!(check_rx.recv_timeout(Duration::ZERO).is_err());
    assert!(probe_rx.recv_timeout(Duration::ZERO).is_err());
    assert_eq!(starts(&marker), 0, "restic started during the replacement");

    drop(lease);
    assert_eq!(run_rx.recv_timeout(DONE), Ok(()));
    assert_eq!(check_rx.recv_timeout(DONE), Ok("0.19.1".to_string()));
    assert_eq!(probe_rx.recv_timeout(DONE), Ok(()));
    assert_eq!(starts(&marker), 3);
    run.join().expect("run");
    check.join().expect("check");
    probing.join().expect("probe");
}

/// AC-M5-06-8 (b): the replacement waits while a restic process of a run is still running.
#[test]
fn ac_m5_06_8_replacement_waits_for_running_restic() {
    let _turn = take_turn();
    let dir = tempfile::tempdir().expect("dir");
    let (restic, marker, hold) = stand_in(dir.path());
    std::fs::write(&hold, b"").expect("hold");
    let run_dir = dir.path().to_path_buf();
    let run = thread::spawn(move || run_snapshots(&restic, &run_dir));
    wait_for_starts(&marker, 1);
    assert_eq!(ResticBinaryGate::global().readers(), 1);

    let (replaced, replaced_rx) = mpsc::channel();
    let replacer = thread::spawn(move || {
        let lease = ResticBinaryGate::global().replace_when_idle(&|| false);
        replaced.send(lease.is_some()).expect("send");
    });
    assert!(
        replaced_rx.recv_timeout(BLOCKED).is_err(),
        "the binary must not be replaced while restic runs"
    );
    std::fs::remove_file(&hold).expect("release");
    run.join().expect("run");
    assert_eq!(replaced_rx.recv_timeout(DONE), Ok(true));
    replacer.join().expect("replacer");
}

/// A repository password that is no UTF-8 cannot be redacted from restic's output: the run
/// fails before restic starts, so no restic is left running without its read lease.
#[test]
fn run_restic_with_unusable_secret_starts_nothing() {
    let _turn = take_turn();
    let dir = tempfile::tempdir().expect("dir");
    let (restic, marker, _hold) = stand_in(dir.path());
    let dest = local_destination(dir.path());
    let store = InMemorySecretStore::default();
    store
        .set_secret(
            dest.id.as_str(),
            "repo",
            &SecretValue::from_bytes(vec![0xff, 0xfe]),
        )
        .expect("store secret");
    let mut spec =
        ResticCommandSpec::for_destination(&restic, "kestic", &dest, vec!["snapshots".to_string()]);
    spec.low_priority = false;
    let clock = FakeClock::new(datetime!(2026-12-01 12:00:00 UTC));
    let result = run_restic(&spec, &dest, Some(&store), &clock, None, &[], |_| {});
    // (a) the run fails
    assert!(result.is_err(), "a password that is no UTF-8 is refused");
    thread::sleep(BLOCKED);
    // (b) restic never started
    assert_eq!(starts(&marker), 0);
    // (c) no read lease is left
    assert_eq!(ResticBinaryGate::global().readers(), 0);
}
