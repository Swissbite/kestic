// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S09 sleep handling: portal / logind inhibitors during a run, interruption on
//! `PrepareForSleep(true)`, and resumption after wake-up (`AC-M3-09-1`, `-2`, `-3`).

mod common;

use common::fakes::{FakeLogindHandle, FakePortalInhibitHandle};
use common::m3::{
    LocalBackupFixture, M3Rig, ResticVariant, StdFakes, backup_summary, restic_variants, settle,
    wait_for_state,
};
use common::run_isolated_test;
use kestic_core::model::RunStatus;
use kestic_test_support::wait::{try_poll_every, wait_until, wait_until_within};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;
use time::macros::datetime;

/// XDG portal inhibit flags: 4 = suspend, 8 = idle.
const PORTAL_IDLE_AND_SUSPEND: u32 = 12;

fn start_rig(root: &Path, variant: &ResticVariant, base: &str) -> (M3Rig, LocalBackupFixture) {
    let rig = M3Rig::start(
        root,
        variant,
        base,
        datetime!(2026-12-01 10:59:30 UTC),
        |opts| opts.enable_portal_inhibit = true,
    );
    let fx = rig.add_backup("Docs", |_| {});
    (rig, fx)
}

/// AC-M3-09-1: while a run is active Kestic holds (a) a portal inhibit (idle + suspend) and
/// (b) a logind `sleep` inhibitor in `delay` mode; (c) both are released after the run ended.
#[test]
fn ac_m3_09_1_run_holds_portal_and_logind_inhibitors_and_releases_them_at_end() {
    run_isolated_test(
        "ac_m3_09_1_run_holds_portal_and_logind_inhibitors_and_releases_them_at_end",
        "KESTIC_M3_09_1_INNER",
        |root| {
            let _std_fakes = StdFakes::start();
            let logind = FakeLogindHandle::start();
            let portal = FakePortalInhibitHandle::start();
            for variant in restic_variants() {
                let portal_before = portal.recorder.inhibit_count.load(Ordering::SeqCst);
                let closed_before = portal.recorder.closed_count.load(Ordering::SeqCst);
                let logind_before = logind.inhibit_call_count();
                let (rig, fx) = start_rig(root, &variant, "M3S1");
                let client = &rig.daemon.client;
                rig.wrapper.hold("backup");

                client.start_backup(&fx.backup_id_str).expect("start run");
                wait_for_state(client, Duration::from_secs(15), "running", |s| {
                    backup_summary(s, &fx.backup.id).active_run.is_some()
                });
                // (a) portal inhibit: exactly one, idle + suspend.
                wait_until("portal inhibit", || {
                    portal.recorder.inhibit_count.load(Ordering::SeqCst) > portal_before
                });
                assert_eq!(
                    portal.recorder.inhibit_count.load(Ordering::SeqCst) - portal_before,
                    1,
                    "{}: (a) one portal inhibit",
                    variant.label
                );
                assert_eq!(
                    portal.recorder.last_flags.load(Ordering::SeqCst),
                    PORTAL_IDLE_AND_SUSPEND,
                    "{}: (a) idle + suspend flags",
                    variant.label
                );
                assert_eq!(
                    portal.recorder.last_reason.lock().expect("lock").as_str(),
                    "Backing up Docs",
                    "{}: (a) the inhibit reason names the running backup",
                    variant.label
                );
                // (b) logind: one `sleep` inhibitor with mode `delay`, FD held open.
                wait_until("logind inhibitor", || {
                    logind.inhibit_call_count() > logind_before
                });
                let calls = logind.inhibit_calls();
                let call = &calls[logind_before];
                assert_eq!(call.0, "sleep", "{}: (b) what", variant.label);
                assert_eq!(call.3, "delay", "{}: (b) mode", variant.label);
                assert_eq!(
                    calls.len() - logind_before,
                    1,
                    "{}: (b) one inhibit",
                    variant.label
                );
                assert_eq!(
                    logind.open_inhibitor_count(),
                    1,
                    "{}: (b) inhibitor FD is held while running",
                    variant.label
                );

                // (c) normal end releases both.
                rig.wrapper.release("backup");
                wait_for_state(client, Duration::from_secs(30), "run Ok", |s| {
                    backup_summary(s, &fx.backup.id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });
                wait_until("logind inhibitor released", || {
                    logind.open_inhibitor_count() == 0
                });
                wait_until("portal inhibit closed", || {
                    portal.recorder.closed_count.load(Ordering::SeqCst) > closed_before
                });
                assert_eq!(
                    portal.recorder.closed_count.load(Ordering::SeqCst) - closed_before,
                    1,
                    "{}: (c) portal inhibit released once",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-09-2 (lid closed): `PrepareForSleep(true)` during a run -> (a) SIGTERM to restic,
/// (b) run `unterbrochen`, (c) the `delay` inhibitor is released.
#[test]
fn ac_m3_09_2_prepare_for_sleep_terminates_restic_and_interrupts_run() {
    run_isolated_test(
        "ac_m3_09_2_prepare_for_sleep_terminates_restic_and_interrupts_run",
        "KESTIC_M3_09_2_INNER",
        |root| {
            let _std_fakes = StdFakes::start();
            let logind = FakeLogindHandle::start();
            let _portal = FakePortalInhibitHandle::start();
            for variant in restic_variants() {
                let (rig, fx) = start_rig(root, &variant, "M3S2");
                let client = &rig.daemon.client;
                rig.wrapper.hold("backup");
                client.start_backup(&fx.backup_id_str).expect("start run");
                wait_for_state(client, Duration::from_secs(15), "running", |s| {
                    backup_summary(s, &fx.backup.id).active_run.is_some()
                });
                rig.wrapper
                    .wait_for('S', "backup", 1, Duration::from_secs(5));
                wait_until("logind inhibitor held", || {
                    logind.open_inhibitor_count() == 1
                });

                logind.emit_prepare_for_sleep(true);
                let state = wait_for_state(client, Duration::from_secs(15), "interrupted", |s| {
                    let b = backup_summary(s, &fx.backup.id);
                    b.active_run.is_none()
                        && b.last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Interrupted)
                });
                // (a) restic received SIGTERM.
                assert_eq!(
                    rig.wrapper.of('T', "backup").len(),
                    1,
                    "{}: (a) SIGTERM reached the restic process: {:?}",
                    variant.label,
                    rig.wrapper.entries()
                );
                // (b) run interrupted.
                assert_eq!(
                    backup_summary(&state, &fx.backup.id)
                        .last_run
                        .as_ref()
                        .map(|r| r.status),
                    Some(RunStatus::Interrupted),
                    "{}: (b)",
                    variant.label
                );
                // (c) delay inhibitor released.
                wait_until("logind delay inhibitor released", || {
                    logind.open_inhibitor_count() == 0
                });
                rig.wrapper.release("backup");
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-09-3: after an interrupted run `PrepareForSleep(false)` makes the backup (a)
/// `ausstehend` and (b) it starts as soon as the run conditions are met.
#[test]
fn ac_m3_09_3_after_wake_up_interrupted_backup_is_pending_then_starts() {
    run_isolated_test(
        "ac_m3_09_3_after_wake_up_interrupted_backup_is_pending_then_starts",
        "KESTIC_M3_09_3_INNER",
        |root| {
            let std_fakes = StdFakes::start();
            let logind = FakeLogindHandle::start();
            let _portal = FakePortalInhibitHandle::start();
            for variant in restic_variants() {
                std_fakes.upower.set_on_battery(false, 100.0);
                let (rig, fx) = start_rig(root, &variant, "M3S3");
                let client = &rig.daemon.client;
                rig.wrapper.hold("backup");
                client.start_backup(&fx.backup_id_str).expect("start run");
                wait_for_state(client, Duration::from_secs(15), "running", |s| {
                    backup_summary(s, &fx.backup.id).active_run.is_some()
                });
                logind.emit_prepare_for_sleep(true);
                wait_for_state(client, Duration::from_secs(15), "interrupted", |s| {
                    backup_summary(s, &fx.backup.id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Interrupted)
                });
                rig.wrapper.release("backup");

                // Wake up while on battery: the backup waits.
                std_fakes.upower.set_on_battery(true, 60.0);
                logind.emit_prepare_for_sleep(false);
                let pending = wait_for_state(client, Duration::from_secs(10), "pending", |s| {
                    backup_summary(s, &fx.backup.id)
                        .pending_primary_code
                        .map(|c| c.as_str())
                        == Some("on_battery")
                });
                assert!(
                    backup_summary(&pending, &fx.backup.id).active_run.is_none(),
                    "{}: (a) pending, not running",
                    variant.label
                );
                settle();
                let still = client.get_state().expect("GetState");
                assert_eq!(
                    backup_summary(&still, &fx.backup.id)
                        .last_run
                        .as_ref()
                        .map(|r| r.status),
                    Some(RunStatus::Interrupted),
                    "{}: (a) no run started on battery",
                    variant.label
                );

                // (b) conditions met -> starts without waiting for another slot.
                std_fakes.upower.set_on_battery(false, 100.0);
                let resumed = wait_for_state(client, Duration::from_secs(30), "resumed Ok", |s| {
                    backup_summary(s, &fx.backup.id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });
                let statuses: Vec<RunStatus> = resumed
                    .recent_runs
                    .iter()
                    .filter(|r| r.backup_id() == Some(&fx.backup.id))
                    .map(|r| r.status)
                    .filter(|s| *s != RunStatus::Pending)
                    .collect();
                assert_eq!(
                    statuses.len(),
                    2,
                    "{}: (b) exactly one resumed run after the interrupted one: {statuses:?}",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// Lid closed during a run: a burst of UPower /
/// NetworkManager signals, as when the lid closes and the network goes to sleep, must not keep
/// the system monitor from delivering the `PrepareForSleep(true)` that follows. Before the fix
/// the listener awaited its environment read on the same connection whose full signal queue
/// blocked the reply: a deadlock, so the run was never interrupted.
#[test]
fn m3_acceptance_prepare_for_sleep_arrives_after_signal_burst() {
    run_isolated_test(
        "m3_acceptance_prepare_for_sleep_arrives_after_signal_burst",
        "KESTIC_M3_ACC_SLEEP_BURST_INNER",
        |_root| {
            let std_fakes = StdFakes::start();
            let logind = FakeLogindHandle::start();
            let _portal = FakePortalInhibitHandle::start();
            let (tx, rx) = std::sync::mpsc::channel();
            let monitor = kestic_daemon::monitors::SystemMonitorHandle::start(
                kestic_daemon::monitors::MonitorBusConfig {
                    use_session_bus: true,
                    ..Default::default()
                },
                tx,
                None,
            );
            // The monitor reports a power change once all its match rules are registered.
            let mut on_battery = false;
            wait_until_within(Duration::from_secs(10), "the monitor's match rules", || {
                on_battery = !on_battery;
                std_fakes.upower.set_on_battery(on_battery, 80.0);
                matches!(
                    rx.recv_timeout(Duration::from_millis(100)),
                    Ok(kestic_daemon::monitors::SystemMonitorEvent::EnvironmentChanged(_))
                )
            });

            for i in 0..300 {
                std_fakes.upower.set_on_battery(i % 2 == 0, 80.0);
            }
            logind.emit_prepare_for_sleep(true);

            // Receiving waits by itself, so the poll needs no pause in between.
            let got_sleep = try_poll_every(Duration::from_secs(10), Duration::ZERO, || {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(kestic_daemon::monitors::SystemMonitorEvent::PrepareForSleep(true)) => {
                        Some(true)
                    }
                    Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Some(false),
                }
            }) == Some(true);
            monitor.stop();

            assert!(
                got_sleep,
                "PrepareForSleep(true) must arrive within 10 s after a signal burst"
            );
        },
    );
}
