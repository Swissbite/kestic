// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S12 Plasma progress display via `org.kde.JobViewServerV2` (`AC-M3-12-1`, `AC-M3-12-2`).

mod common;

use common::fakes::{FakeJobViewHandle, FakeNetworkManagerHandle, FakeUPowerHandle};
use common::m3::{
    M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, add_local_backup, backup_summary,
    latest_variant, restic_variants, spawn_m3_daemon, wait_for_state,
};
use common::{
    CapturedKesticSignal, KesticSignalCollector, run_isolated_test, start_fake_notifications,
    start_fake_sni_watcher,
};
use kestic_core::clock::FakeClock;
use kestic_core::model::{Run, RunStatus};
use kestic_daemon::dbus_api::{KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use time::macros::datetime;

const JOB_VIEW_DEST: &str = "org.kde.JobViewServer";

struct Rig {
    daemon: M3Daemon,
    wrapper: ResticWrapper,
    fixture: common::m3::LocalBackupFixture,
}

fn start_rig(root: &Path, variant: &ResticVariant, base: &str) -> Rig {
    let sub = variant.sub(base);
    let wrapper = ResticWrapper::create(&root.join(&sub).join("wrapper"), &variant.bin);
    let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:30 UTC)));
    let bus = variant.bus_name(base);
    let mut opts = M3DaemonOpts::new(&sub, &bus, &wrapper.path, clock);
    opts.enable_job_view = true;
    let daemon = spawn_m3_daemon(root, opts);
    let fixture = add_local_backup(&daemon.client, root, &sub, "Docs", |_| {});
    Rig {
        daemon,
        wrapper,
        fixture,
    }
}

fn start_fakes() -> (
    FakeJobViewHandle,
    [zbus::Connection; 2],
    FakeUPowerHandle,
    FakeNetworkManagerHandle,
) {
    let (nc, _capture) = start_fake_notifications();
    (
        FakeJobViewHandle::start(JOB_VIEW_DEST),
        [nc, start_fake_sni_watcher()],
        FakeUPowerHandle::start(false, 100.0),
        FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]),
    )
}

fn wait_until(what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pred() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// AC-M3-12-1 (FR-53): a starting backup run is announced to the `JobViewServerV2` fake -> (a)
/// `requestView` with Kestic's desktop id, (b) progress in percent, (c) completion with status.
#[test]
fn ac_m3_12_1_run_is_announced_with_progress_and_completion_status() {
    run_isolated_test(
        "ac_m3_12_1_run_is_announced_with_progress_and_completion_status",
        "KESTIC_M3_12_1_INNER",
        |root| {
            let (job_view, _conns, _upower, _nm) = start_fakes();
            let rec = &job_view.recorder;
            for variant in restic_variants() {
                let requests_before = rec.request_count.load(Ordering::SeqCst);
                let terminations_before = rec.terminations.lock().expect("lock").len();
                rec.updates.lock().expect("lock").clear();
                let rig = start_rig(root, &variant, "M3J1");
                let client = &rig.daemon.client;
                let bid = &rig.fixture.backup.id;
                rig.wrapper.hold("backup");

                client
                    .start_backup(&rig.fixture.backup_id_str)
                    .expect("start run");
                // (a) requestView with the desktop id of Kestic.
                wait_until("requestView", || {
                    rec.request_count.load(Ordering::SeqCst) > requests_before
                });
                assert_eq!(
                    rec.request_count.load(Ordering::SeqCst) - requests_before,
                    1,
                    "{}: (a) one view per run",
                    variant.label
                );
                assert_eq!(
                    *rec.desktop_entry.lock().expect("lock"),
                    "net.daester.Kestic",
                    "{}: (a)",
                    variant.label
                );
                // (b) progress in percent (the held run reports 25 %).
                wait_until("progress update", || {
                    rec.updates
                        .lock()
                        .expect("lock")
                        .iter()
                        .any(|(pct, _)| *pct == 25)
                });
                assert!(
                    rec.updates
                        .lock()
                        .expect("lock")
                        .iter()
                        .any(|(pct, msg)| *pct == 25 && msg.contains("25%")),
                    "{}: (b) percent and info message: {:?}",
                    variant.label,
                    rec.updates.lock().expect("lock")
                );

                // (c) completion: ok -> code 0.
                rig.wrapper.release("backup");
                wait_for_state(client, Duration::from_secs(30), "run Ok", |s| {
                    backup_summary(s, bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });
                wait_until("terminate", || {
                    rec.terminations.lock().expect("lock").len() > terminations_before
                });
                {
                    let terms = rec.terminations.lock().expect("lock");
                    assert_eq!(
                        terms[terminations_before].0, 0,
                        "{}: (c) ok run completes with code 0",
                        variant.label
                    );
                }

                // (c) completion with a warning carries a non-zero status and a message.
                let bad = rig.fixture.src_dir.join("unreadable.secret");
                fs::write(&bad, b"x").expect("write");
                fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).expect("chmod");
                client
                    .start_backup(&rig.fixture.backup_id_str)
                    .expect("second run");
                wait_for_state(client, Duration::from_secs(30), "run Warning", |s| {
                    backup_summary(s, bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Warning)
                });
                wait_until("second terminate", || {
                    rec.terminations.lock().expect("lock").len() > terminations_before + 1
                });
                let _ = fs::set_permissions(&bad, fs::Permissions::from_mode(0o644));
                {
                    let terms = rec.terminations.lock().expect("lock");
                    assert_ne!(
                        terms[terminations_before + 1].0,
                        0,
                        "{}: (c) warning completes with a non-zero status",
                        variant.label
                    );
                    assert!(
                        !terms[terminations_before + 1].1.is_empty(),
                        "{}: (c) completion carries the status message",
                        variant.label
                    );
                }
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-12-2 (FR-53, FR-21): cancelling in the Plasma progress display ends the run `abgebrochen`.
#[test]
fn ac_m3_12_2_cancel_in_plasma_progress_cancels_the_run() {
    run_isolated_test(
        "ac_m3_12_2_cancel_in_plasma_progress_cancels_the_run",
        "KESTIC_M3_12_2_INNER",
        |root| {
            let (job_view, _conns, _upower, _nm) = start_fakes();
            let variant = latest_variant();
            let rig = start_rig(root, &variant, "M3J2");
            let client = &rig.daemon.client;
            rig.wrapper.hold("backup");
            let signals = KesticSignalCollector::start(
                &variant.bus_name("M3J2"),
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );
            let run_id = client
                .start_backup(&rig.fixture.backup_id_str)
                .expect("start run");
            wait_until("progress shown in Plasma", || {
                !job_view.recorder.updates.lock().expect("lock").is_empty()
            });

            job_view.emit_cancel_requested();
            signals.wait_until(Duration::from_secs(15), |sigs| {
                sigs.iter().any(|s| {
                    if let CapturedKesticSignal::RunUpdated(json) = s
                        && let Ok(run) = serde_json::from_str::<Run>(json)
                    {
                        return run.id.as_str() == run_id && run.status == RunStatus::Cancelled;
                    }
                    false
                })
            });
            let state = client.get_state().expect("GetState");
            assert_eq!(
                backup_summary(&state, &rig.fixture.backup.id)
                    .last_run
                    .as_ref()
                    .map(|r| r.status),
                Some(RunStatus::Cancelled),
                "(a) the run ends `abgebrochen`"
            );
            assert_eq!(
                rig.wrapper.of('T', "backup").len(),
                1,
                "(a) restic was terminated"
            );
            drop(signals);
            rig.wrapper.release("backup");
            rig.daemon.stop();
        },
    );
}
