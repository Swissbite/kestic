// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S07 pausing: global pause blocks scheduled runs and maintenance but not manual runs, timed
//! pause expiry with single catch-up, and overdue accounting / tray while paused
//! (`AC-M3-07-2`, `AC-M3-07-3`, `AC-M3-07-4`).

mod common;

use common::fakes::{FakeNetworkManagerHandle, FakeUPowerHandle};
use common::m3::{
    M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, add_local_backup, backup_summary,
    notifications_for, restic_variants, settle, spawn_m3_daemon, wait_for_notifications,
    wait_for_state,
};
use common::{
    fetch_dbusmenu_items, read_sni_string_prop, run_isolated_test, start_fake_notifications,
    start_fake_sni_watcher, wait_for_sni_service,
};
use kestic_core::clock::FakeClock;
use kestic_core::model::{Backup, BackupId, RunStatus};
use kestic_core::scheduler::PauseDuration;
use kestic_daemon::dbus_api::DaemonStateDto;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use time::macros::datetime;

fn executed_runs(state: &DaemonStateDto, bid: &BackupId) -> usize {
    state
        .recent_runs
        .iter()
        .filter(|r| r.backup_id() == Some(bid))
        .filter(|r| !matches!(r.status, RunStatus::Pending))
        .count()
}

struct Rig {
    daemon: M3Daemon,
    clock: Arc<FakeClock>,
    wrapper: ResticWrapper,
    bid: BackupId,
    bid_str: String,
}

fn start_rig(
    root: &Path,
    variant: &ResticVariant,
    base: &str,
    start: OffsetDateTime,
    configure: impl FnOnce(&mut Backup),
) -> Rig {
    let sub = variant.sub(base);
    let wrapper = ResticWrapper::create(&root.join(&sub).join("wrapper"), &variant.bin);
    let clock = Arc::new(FakeClock::new(start));
    let bus = variant.bus_name(base);
    let daemon = spawn_m3_daemon(
        root,
        M3DaemonOpts::new(&sub, &bus, &wrapper.path, Arc::clone(&clock) as _),
    );
    let fx = add_local_backup(&daemon.client, root, &sub, "Docs", configure);
    // One tick so scheduler baselines (backup and maintenance) are recorded before pausing.
    std::thread::sleep(Duration::from_millis(1300));
    Rig {
        daemon,
        clock,
        wrapper,
        bid: fx.backup.id,
        bid_str: fx.backup_id_str,
    }
}

/// AC-M3-07-2 (FR-13a, P14): global pause -> (a) no scheduled run, (b) no maintenance run,
/// (c) a manual run still starts.
#[test]
fn ac_m3_07_2_global_pause_blocks_schedule_and_maintenance_but_not_manual_runs() {
    run_isolated_test(
        "ac_m3_07_2_global_pause_blocks_schedule_and_maintenance_but_not_manual_runs",
        "KESTIC_M3_07_2_INNER",
        |root| {
            let (_nc, _notif) = start_fake_notifications();
            let _sni = start_fake_sni_watcher();
            let _upower = FakeUPowerHandle::start(false, 100.0);
            let _nm = FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]);
            for variant in restic_variants() {
                // Friday 11:59:30 CET; Sunday 12:00 brings backup slots and prune/check due.
                let rig = start_rig(
                    root,
                    &variant,
                    "M3P2",
                    datetime!(2026-12-04 10:59:30 UTC),
                    |_| {},
                );
                let client = &rig.daemon.client;
                let repo_before = rig.wrapper.started_repo_count();
                client
                    .pause_global(&PauseDuration::Indefinite)
                    .expect("PauseGlobal");

                rig.clock.set(datetime!(2026-12-06 11:00:05 UTC));
                settle();
                let paused = client.get_state().expect("GetState");
                assert!(paused.overall.paused, "{}: globally paused", variant.label);
                assert_eq!(
                    rig.wrapper.of('S', "backup").len(),
                    0,
                    "{}: (a) no scheduled backup while paused",
                    variant.label
                );
                assert_eq!(
                    executed_runs(&paused, &rig.bid),
                    0,
                    "{}: (a) no run recorded",
                    variant.label
                );
                assert_eq!(
                    rig.wrapper.started_repo_count(),
                    repo_before,
                    "{}: (b) no maintenance (or any other restic) process while paused: {:?}",
                    variant.label,
                    rig.wrapper.entries()
                );

                client.start_backup(&rig.bid_str).expect("manual start");
                wait_for_state(client, Duration::from_secs(30), "manual Ok", |s| {
                    backup_summary(s, &rig.bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });
                assert_eq!(
                    rig.wrapper.of('S', "backup").len(),
                    1,
                    "{}: (c) the manual run started despite the pause",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-07-3 (FR-13a, P14): pause "for 4 hours" from 10:00 -> at 14:00 (a) the pause is lifted and
/// (b) the slot missed during the pause is caught up exactly once.
#[test]
fn ac_m3_07_3_timed_pause_expires_and_missed_slot_is_caught_up_once() {
    run_isolated_test(
        "ac_m3_07_3_timed_pause_expires_and_missed_slot_is_caught_up_once",
        "KESTIC_M3_07_3_INNER",
        |root| {
            let (_nc, _notif) = start_fake_notifications();
            let _sni = start_fake_sni_watcher();
            let _upower = FakeUPowerHandle::start(false, 100.0);
            let _nm = FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]);
            for variant in restic_variants() {
                // 10:00 CET.
                let rig = start_rig(
                    root,
                    &variant,
                    "M3P3",
                    datetime!(2026-12-01 09:00:00 UTC),
                    |_| {},
                );
                let client = &rig.daemon.client;
                client
                    .pause_global(&PauseDuration::Hours { hours: 4 })
                    .expect("PauseGlobal(4h)");

                // 12:30 CET: 12:00 slot passed during the pause.
                rig.clock.set(datetime!(2026-12-01 11:30:00 UTC));
                settle();
                let during = client.get_state().expect("GetState");
                assert!(
                    during.overall.paused,
                    "{}: still paused at 12:30",
                    variant.label
                );
                assert_eq!(
                    executed_runs(&during, &rig.bid),
                    0,
                    "{}: nothing runs during the pause",
                    variant.label
                );

                // 14:00 CET: pause over.
                rig.clock.set(datetime!(2026-12-01 13:00:05 UTC));
                let done = wait_for_state(client, Duration::from_secs(30), "catch-up Ok", |s| {
                    backup_summary(s, &rig.bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });
                assert!(
                    !done.overall.paused,
                    "{}: (a) pause lifted at 14:00",
                    variant.label
                );
                settle();
                let after = client.get_state().expect("GetState");
                assert_eq!(
                    executed_runs(&after, &rig.bid),
                    1,
                    "{}: (b) the missed slot is caught up exactly once",
                    variant.label
                );
                assert_eq!(
                    rig.wrapper.of('S', "backup").len(),
                    1,
                    "{}: (b) one restic backup process",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-07-4 (FR-13a, P14): pause across three slots -> (a) no overdue warning during the pause,
/// (b) tray shows "Paused since ..." with SNI status `Active`, (c) after the pause only later slots
/// count towards "overdue".
#[test]
fn ac_m3_07_4_pause_over_three_slots_has_no_overdue_alert_and_counts_only_later_slots() {
    run_isolated_test(
        "ac_m3_07_4_pause_over_three_slots_has_no_overdue_alert_and_counts_only_later_slots",
        "KESTIC_M3_07_4_INNER",
        |root| {
            let (_nc, notif) = start_fake_notifications();
            let sni = start_fake_sni_watcher();
            // On battery so that no catch-up run can start after the pause.
            let _upower = FakeUPowerHandle::start(true, 50.0);
            let _nm = FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]);
            let variant = common::m3::latest_variant();
            let rig = start_rig(
                root,
                &variant,
                "M3P4",
                datetime!(2026-12-01 09:00:00 UTC),
                |_| {},
            );
            let client = &rig.daemon.client;
            let sni_service = wait_for_sni_service(&sni, Duration::from_secs(5));

            client
                .pause_global(&PauseDuration::Indefinite)
                .expect("PauseGlobal");
            // Slots on Dec 1, 2, 3 (12:00 CET) pass while paused; now Dec 3 15:00 CET.
            rig.clock.set(datetime!(2026-12-03 14:00:00 UTC));
            settle();

            let paused = client.get_state().expect("GetState");
            // (a) no overdue state and no notification during the pause.
            assert!(
                !backup_summary(&paused, &rig.bid).is_overdue,
                "(a) not overdue while paused"
            );
            assert_eq!(
                notifications_for(&notif, "BackupOverdue").len(),
                0,
                "(a) no overdue warning during the pause: {:?}",
                notif.snapshot()
            );
            // (b) tray: status Active and "Paused since ...".
            assert_eq!(
                read_sni_string_prop(client.connection(), &sni_service, "Status"),
                "Active",
                "(b) SNI status during pause"
            );
            let menu = fetch_dbusmenu_items(client.connection(), &sni_service);
            assert!(
                menu.iter().any(|e| e.label.contains("Paused since")),
                "(b) tray menu names the pause start: {menu:?}"
            );

            // Resume Dec 3 15:00 CET: slots before this moment never count.
            client.resume_global().expect("ResumeGlobal");
            rig.clock.set(datetime!(2026-12-04 11:00:05 UTC));
            settle();
            let one_slot = client.get_state().expect("GetState");
            assert!(
                !backup_summary(&one_slot, &rig.bid).is_overdue,
                "(c) one slot after the pause is not overdue (three paused slots are ignored)"
            );
            assert_eq!(
                notifications_for(&notif, "BackupOverdue").len(),
                0,
                "(c) no overdue notification after the first slot following the pause"
            );
            rig.clock.set(datetime!(2026-12-05 11:00:05 UTC));
            wait_for_state(
                client,
                Duration::from_secs(10),
                "overdue after 2 slots",
                |s| backup_summary(s, &rig.bid).is_overdue,
            );
            wait_for_notifications(&notif, "BackupOverdue", 1, Duration::from_secs(5));
            rig.daemon.stop();
        },
    );
}
