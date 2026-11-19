// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S09 sleep handling: portal / logind inhibitors during a run, interruption on
//! `PrepareForSleep(true)`, and resumption after wake-up (`AC-M3-09-1`, `-2`, `-3`).

mod common;

use common::fakes::{
    FakeLogindHandle, FakeNetworkManagerHandle, FakePortalInhibitHandle, FakeUPowerHandle,
};
use common::m3::{
    M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, add_local_backup, backup_summary,
    restic_variants, settle, spawn_m3_daemon, wait_for_state,
};
use common::{run_isolated_test, start_fake_notifications, start_fake_sni_watcher};
use kestic_core::clock::FakeClock;
use kestic_core::model::{BackupId, RunStatus};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use time::macros::datetime;

/// XDG portal inhibit flags: 4 = suspend, 8 = idle.
const PORTAL_IDLE_AND_SUSPEND: u32 = 12;

struct Fakes {
    upower: FakeUPowerHandle,
    logind: FakeLogindHandle,
    portal: FakePortalInhibitHandle,
    _nm: FakeNetworkManagerHandle,
    _notif: zbus::Connection,
    _sni: zbus::Connection,
}

fn start_fakes() -> Fakes {
    let (notif, _capture) = start_fake_notifications();
    Fakes {
        _sni: start_fake_sni_watcher(),
        upower: FakeUPowerHandle::start(false, 100.0),
        logind: FakeLogindHandle::start(),
        portal: FakePortalInhibitHandle::start(),
        _nm: FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]),
        _notif: notif,
    }
}

struct Rig {
    daemon: M3Daemon,
    wrapper: ResticWrapper,
    bid: BackupId,
    bid_str: String,
}

fn start_rig(root: &Path, variant: &ResticVariant, base: &str) -> Rig {
    let sub = variant.sub(base);
    let wrapper = ResticWrapper::create(&root.join(&sub).join("wrapper"), &variant.bin);
    let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:30 UTC)));
    let bus = variant.bus_name(base);
    let mut opts = M3DaemonOpts::new(&sub, &bus, &wrapper.path, Arc::clone(&clock) as _);
    opts.enable_portal_inhibit = true;
    let daemon = spawn_m3_daemon(root, opts);
    let fx = add_local_backup(&daemon.client, root, &sub, "Docs", |_| {});
    Rig {
        daemon,
        wrapper,
        bid: fx.backup.id,
        bid_str: fx.backup_id_str,
    }
}

fn wait_until(what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pred() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// AC-M3-09-1 (F9): while a run is active Kestic holds (a) a portal inhibit (idle + suspend) and
/// (b) a logind `sleep` inhibitor in `delay` mode; (c) both are released after the run ended.
#[test]
fn ac_m3_09_1_run_holds_portal_and_logind_inhibitors_and_releases_them_at_end() {
    run_isolated_test(
        "ac_m3_09_1_run_holds_portal_and_logind_inhibitors_and_releases_them_at_end",
        "KESTIC_M3_09_1_INNER",
        |root| {
            let fakes = start_fakes();
            for variant in restic_variants() {
                let portal_before = fakes.portal.recorder.inhibit_count.load(Ordering::SeqCst);
                let closed_before = fakes.portal.recorder.closed_count.load(Ordering::SeqCst);
                let logind_before = fakes.logind.inhibit_call_count();
                let rig = start_rig(root, &variant, "M3S1");
                let client = &rig.daemon.client;
                rig.wrapper.hold("backup");

                client.start_backup(&rig.bid_str).expect("start run");
                wait_for_state(client, Duration::from_secs(15), "running", |s| {
                    backup_summary(s, &rig.bid).active_run.is_some()
                });
                // (a) portal inhibit: exactly one, idle + suspend.
                wait_until("portal inhibit", || {
                    fakes.portal.recorder.inhibit_count.load(Ordering::SeqCst) > portal_before
                });
                assert_eq!(
                    fakes.portal.recorder.inhibit_count.load(Ordering::SeqCst) - portal_before,
                    1,
                    "{}: (a) one portal inhibit",
                    variant.label
                );
                assert_eq!(
                    fakes.portal.recorder.last_flags.load(Ordering::SeqCst),
                    PORTAL_IDLE_AND_SUSPEND,
                    "{}: (a) idle + suspend flags",
                    variant.label
                );
                // (b) logind: one `sleep` inhibitor with mode `delay`, FD held open.
                wait_until("logind inhibitor", || {
                    fakes.logind.inhibit_call_count() > logind_before
                });
                let calls = fakes.logind.inhibit_calls();
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
                    fakes.logind.open_inhibitor_count(),
                    1,
                    "{}: (b) inhibitor FD is held while running",
                    variant.label
                );

                // (c) normal end releases both.
                rig.wrapper.release("backup");
                wait_for_state(client, Duration::from_secs(30), "run Ok", |s| {
                    backup_summary(s, &rig.bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });
                wait_until("logind inhibitor released", || {
                    fakes.logind.open_inhibitor_count() == 0
                });
                wait_until("portal inhibit closed", || {
                    fakes.portal.recorder.closed_count.load(Ordering::SeqCst) > closed_before
                });
                assert_eq!(
                    fakes.portal.recorder.closed_count.load(Ordering::SeqCst) - closed_before,
                    1,
                    "{}: (c) portal inhibit released once",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-09-2 (F9, lid closed): `PrepareForSleep(true)` during a run -> (a) SIGTERM to restic,
/// (b) run `unterbrochen`, (c) the `delay` inhibitor is released.
#[test]
fn ac_m3_09_2_prepare_for_sleep_terminates_restic_and_interrupts_run() {
    run_isolated_test(
        "ac_m3_09_2_prepare_for_sleep_terminates_restic_and_interrupts_run",
        "KESTIC_M3_09_2_INNER",
        |root| {
            let fakes = start_fakes();
            for variant in restic_variants() {
                let rig = start_rig(root, &variant, "M3S2");
                let client = &rig.daemon.client;
                rig.wrapper.hold("backup");
                client.start_backup(&rig.bid_str).expect("start run");
                wait_for_state(client, Duration::from_secs(15), "running", |s| {
                    backup_summary(s, &rig.bid).active_run.is_some()
                });
                rig.wrapper
                    .wait_for('S', "backup", 1, Duration::from_secs(5));
                wait_until("logind inhibitor held", || {
                    fakes.logind.open_inhibitor_count() == 1
                });

                fakes.logind.emit_prepare_for_sleep(true);
                let state = wait_for_state(client, Duration::from_secs(15), "interrupted", |s| {
                    let b = backup_summary(s, &rig.bid);
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
                    backup_summary(&state, &rig.bid)
                        .last_run
                        .as_ref()
                        .map(|r| r.status),
                    Some(RunStatus::Interrupted),
                    "{}: (b)",
                    variant.label
                );
                // (c) delay inhibitor released.
                wait_until("logind delay inhibitor released", || {
                    fakes.logind.open_inhibitor_count() == 0
                });
                rig.wrapper.release("backup");
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-09-3 (F9): after an interrupted run `PrepareForSleep(false)` makes the backup (a)
/// `ausstehend` and (b) it starts as soon as the run conditions are met.
#[test]
fn ac_m3_09_3_after_wake_up_interrupted_backup_is_pending_then_starts() {
    run_isolated_test(
        "ac_m3_09_3_after_wake_up_interrupted_backup_is_pending_then_starts",
        "KESTIC_M3_09_3_INNER",
        |root| {
            let fakes = start_fakes();
            for variant in restic_variants() {
                fakes.upower.set_on_battery(false, 100.0);
                let rig = start_rig(root, &variant, "M3S3");
                let client = &rig.daemon.client;
                rig.wrapper.hold("backup");
                client.start_backup(&rig.bid_str).expect("start run");
                wait_for_state(client, Duration::from_secs(15), "running", |s| {
                    backup_summary(s, &rig.bid).active_run.is_some()
                });
                fakes.logind.emit_prepare_for_sleep(true);
                wait_for_state(client, Duration::from_secs(15), "interrupted", |s| {
                    backup_summary(s, &rig.bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Interrupted)
                });
                rig.wrapper.release("backup");

                // Wake up while on battery: the backup waits.
                fakes.upower.set_on_battery(true, 60.0);
                fakes.logind.emit_prepare_for_sleep(false);
                let pending = wait_for_state(client, Duration::from_secs(10), "pending", |s| {
                    backup_summary(s, &rig.bid).pending_primary_code.as_deref()
                        == Some("on_battery")
                });
                assert!(
                    backup_summary(&pending, &rig.bid).active_run.is_none(),
                    "{}: (a) pending, not running",
                    variant.label
                );
                settle();
                let still = client.get_state().expect("GetState");
                assert_eq!(
                    backup_summary(&still, &rig.bid)
                        .last_run
                        .as_ref()
                        .map(|r| r.status),
                    Some(RunStatus::Interrupted),
                    "{}: (a) no run started on battery",
                    variant.label
                );

                // (b) conditions met -> starts without waiting for another slot.
                fakes.upower.set_on_battery(false, 100.0);
                let resumed = wait_for_state(client, Duration::from_secs(30), "resumed Ok", |s| {
                    backup_summary(s, &rig.bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });
                let statuses: Vec<RunStatus> = resumed
                    .recent_runs
                    .iter()
                    .filter(|r| r.backup_id() == Some(&rig.bid))
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
