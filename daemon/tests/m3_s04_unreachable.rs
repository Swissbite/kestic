// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S04 unreachable destination (`rest-server` stopped), backoff 5/15/60 min, network-change
//! retry, availability throttle, and manual-start blocking (`AC-M3-04-1`, `-2`, `-3`, `-7`, `-11`).

mod common;

use common::fakes::{FakeNetworkManagerHandle, FakeUPowerHandle};
use common::m3::{
    M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, assert_dbus_error_name, backup_summary,
    dest_summary, latest_variant, notifications_for, restic_variants, runtime_state, settle,
    spawn_m3_daemon, wait_for_backoff_attempt, wait_for_notifications, wait_for_state,
};
use common::{
    CapturedKesticSignal, KesticSignalCollector, RestServerGuard, fetch_dbusmenu_items,
    run_isolated_test, start_fake_notifications, start_fake_sni_watcher, wait_for_sni_service,
};
use kestic_core::clock::{Clock, FakeClock, SystemClock};
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, DestinationKind, RunStatus, Schedule, Source,
};
use kestic_core::scheduler::TimezoneSpec;
use kestic_daemon::dbus_api::{KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH};
use kestic_i18n::codes::BlockedReason;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use time::macros::datetime;

/// 2026-12-01 11:59:30 Europe/Zurich.
const BEFORE_SLOT: OffsetDateTime = datetime!(2026-12-01 10:59:30 UTC);
/// 2026-12-01 12:00:05 Europe/Zurich (first attempt, 5 s after the 12:00 slot).
const SLOT_1: OffsetDateTime = datetime!(2026-12-01 11:00:05 UTC);
/// 2026-12-02 12:00:05 Europe/Zurich.
const SLOT_2: OffsetDateTime = datetime!(2026-12-02 11:00:05 UTC);

struct RestRig {
    daemon: M3Daemon,
    clock: Arc<FakeClock>,
    wrapper: ResticWrapper,
    rest: RestServerGuard,
    rest_port: u16,
    rest_data: std::path::PathBuf,
    dest_id: DestinationId,
    backup_id: BackupId,
    backup_id_str: String,
}

/// Daemon (fake clock, restic wrapper) with a REST destination and a daily 12:00 backup; the
/// repository is initialised while `rest-server` is still running.
fn start_rest_rig(
    root: &Path,
    variant: &ResticVariant,
    base: &str,
    start: OffsetDateTime,
) -> RestRig {
    let sub = variant.sub(base);
    let wrapper = ResticWrapper::create(&root.join(&sub).join("wrapper"), &variant.bin);
    let clock = Arc::new(FakeClock::new(start));
    let bus = variant.bus_name(base);
    let daemon = spawn_m3_daemon(
        root,
        M3DaemonOpts::new(&sub, &bus, &wrapper.path, Arc::clone(&clock) as _),
    );
    let rest_data = root.join(&sub).join("rest-data");
    let rest = RestServerGuard::start_no_auth(&rest_data);
    let rest_port = rest.port;
    let dest = Destination::new(
        "RestRepo",
        DestinationKind::Rest {
            url: rest.url.clone(),
            username: None,
        },
    )
    .expect("rest dest");
    daemon
        .client
        .add_destination(&dest, "init", "rest-pw", "")
        .expect("init rest destination");
    let src_dir = root.join(&sub).join("src");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("file.txt"), b"hello rest").expect("write src");
    let mut backup =
        Backup::new("RestBackup", Source::folder(&src_dir), dest.id.clone()).expect("backup");
    backup.schedule = Schedule::Daily {
        hour: 12,
        minute: 0,
    };
    let backup_id_str = daemon.client.add_backup(&backup).expect("add backup");
    RestRig {
        daemon,
        clock,
        wrapper,
        rest,
        rest_port,
        rest_data,
        dest_id: dest.id,
        backup_id: BackupId::parse(&backup_id_str).expect("backup id"),
        backup_id_str,
    }
}

impl RestRig {
    fn wait_pending_unreachable(&self) {
        wait_for_state(
            &self.daemon.client,
            Duration::from_secs(10),
            "pending destination_unreachable",
            |s| {
                backup_summary(s, &self.backup_id)
                    .pending_primary_code
                    .map(|c| c.as_str())
                    == Some("destination_unreachable")
            },
        );
    }

    fn restart_rest_server(&mut self) {
        self.rest = RestServerGuard::start_on_port(&self.rest_data, self.rest_port, None);
    }
}

fn start_fakes() -> (
    FakeUPowerHandle,
    FakeNetworkManagerHandle,
    zbus::Connection,
    Arc<common::NotificationCapture>,
    zbus::Connection,
) {
    let (notif_conn, notif) = start_fake_notifications();
    let sni = start_fake_sni_watcher();
    let upower = FakeUPowerHandle::start(false, 100.0);
    let nm = FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]);
    (upower, nm, notif_conn, notif, sni)
}

/// AC-M3-04-1 (FR-30, F2, P32): with the `rest-server` stopped, the due 12:00 slot (a) fails the
/// pre-check, (b) spawns no restic process, (c) is `ausstehend` (destination unreachable) and (d)
/// retries at 12:05, 12:20 and 13:20 only, then waits for the next regular slot.
#[test]
fn ac_m3_04_1_unreachable_destination_retries_at_5_15_60_minutes_then_next_slot() {
    run_isolated_test(
        "ac_m3_04_1_unreachable_destination_retries_at_5_15_60_minutes_then_next_slot",
        "KESTIC_M3_04_1_INNER",
        |root| {
            let (_upower, _nm, _nc, _notif, _sni) = start_fakes();
            let variant = latest_variant();
            let mut rig = start_rest_rig(root, &variant, "M3U1", BEFORE_SLOT);
            rig.rest.stop();
            let restic_before = rig.wrapper.started_repo_count();

            // 12:00:05 -> attempt 1 fails at the pre-check.
            rig.clock.set(SLOT_1);
            rig.wait_pending_unreachable();
            let rs1 = wait_for_backoff_attempt(
                &rig.daemon.state_dir,
                &rig.backup_id,
                1,
                Duration::from_secs(5),
            );
            let state = rig.daemon.client.get_state().expect("GetState");
            let avail = &dest_summary(&state, &rig.dest_id).availability;
            // (a) the pre-check failed.
            assert!(!avail.reachable, "(a) pre-check must fail");
            assert!(
                avail.detail.is_some(),
                "(a) pre-check failure carries a detail"
            );
            // (b) no restic process was spawned.
            assert_eq!(
                rig.wrapper.started_repo_count(),
                restic_before,
                "(b) no restic process for the failed pre-check: {:?}",
                rig.wrapper.entries()
            );
            // (c) `ausstehend` because the destination is unreachable.
            assert_eq!(
                backup_summary(&state, &rig.backup_id)
                    .pending_primary_code
                    .map(|c| c.as_str()),
                Some("destination_unreachable"),
                "(c)"
            );
            // (d) retry 1 at 12:05 (5 min after the end of the attempt at 12:00:05).
            assert_eq!(
                rs1.backoff_until,
                Some(datetime!(2026-12-01 11:05:05 UTC)),
                "(d) first retry 12:05"
            );

            // No attempt before the retry time.
            rig.clock.set(datetime!(2026-12-01 11:05:04 UTC));
            settle();
            assert_eq!(
                runtime_state(&rig.daemon.state_dir, &rig.backup_id).backoff_attempt,
                1,
                "(d) no attempt before 12:05"
            );

            // 12:05:05 -> attempt 2 fails -> retry 2 at 12:20.
            rig.clock.set(datetime!(2026-12-01 11:05:05 UTC));
            let rs2 = wait_for_backoff_attempt(
                &rig.daemon.state_dir,
                &rig.backup_id,
                2,
                Duration::from_secs(10),
            );
            assert_eq!(
                rs2.backoff_until,
                Some(datetime!(2026-12-01 11:20:05 UTC)),
                "(d) second retry 12:20"
            );

            // 12:20:05 -> attempt 3 fails -> retry 3 at 13:20.
            rig.clock.set(datetime!(2026-12-01 11:20:05 UTC));
            let rs3 = wait_for_backoff_attempt(
                &rig.daemon.state_dir,
                &rig.backup_id,
                3,
                Duration::from_secs(10),
            );
            assert_eq!(
                rs3.backoff_until,
                Some(datetime!(2026-12-01 12:20:05 UTC)),
                "(d) third retry 13:20"
            );

            // 13:20:05 -> attempt 4 fails -> retries exhausted.
            rig.clock.set(datetime!(2026-12-01 12:20:05 UTC));
            let rs4 = wait_for_backoff_attempt(
                &rig.daemon.state_dir,
                &rig.backup_id,
                4,
                Duration::from_secs(10),
            );
            assert_eq!(rs4.backoff_until, None, "(d) no further retry scheduled");

            // Later the same day: no attempt at all.
            rig.clock.set(datetime!(2026-12-01 13:30:00 UTC));
            settle();
            assert_eq!(
                runtime_state(&rig.daemon.state_dir, &rig.backup_id).backoff_attempt,
                4,
                "(d) no attempt after 13:20 on the same day"
            );

            // Next regular slot 12:00 on the following day -> a new attempt series starts.
            rig.clock.set(SLOT_2);
            let rs5 = wait_for_backoff_attempt(
                &rig.daemon.state_dir,
                &rig.backup_id,
                1,
                Duration::from_secs(10),
            );
            assert_eq!(
                rs5.backoff_until,
                Some(datetime!(2026-12-02 11:05:05 UTC)),
                "(d) next regular slot restarts the 5/15/60 series"
            );
            assert_eq!(
                rig.wrapper.of('S', "backup").len(),
                0,
                "(b) never a backup process while unreachable"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M3-04-2 (FR-30): `ausstehend` because unreachable; when NetworkManager reports a network
/// change and `rest-server` runs again, the run starts immediately (no clock advance).
#[test]
fn ac_m3_04_2_network_change_retries_unreachable_destination_immediately() {
    run_isolated_test(
        "ac_m3_04_2_network_change_retries_unreachable_destination_immediately",
        "KESTIC_M3_04_2_INNER",
        |root| {
            let (_upower, nm, _nc, _notif, _sni) = start_fakes();
            for variant in restic_variants() {
                nm.update(true, false, Some("Heim"));
                let mut rig = start_rest_rig(root, &variant, "M3U2", BEFORE_SLOT);
                rig.rest.stop();
                rig.clock.set(SLOT_1);
                rig.wait_pending_unreachable();
                assert_eq!(
                    rig.wrapper.of('S', "backup").len(),
                    0,
                    "{}: nothing ran while unreachable",
                    variant.label
                );

                rig.restart_rest_server();
                nm.update(false, false, None);
                std::thread::sleep(Duration::from_millis(150));
                nm.update(true, false, Some("Heim"));

                let done = wait_for_state(
                    &rig.daemon.client,
                    Duration::from_secs(20),
                    "run after network change",
                    |s| {
                        backup_summary(s, &rig.backup_id)
                            .last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                    },
                );
                // (a) started right away: the clock never left the first attempt.
                assert_eq!(
                    backup_summary(&done, &rig.backup_id)
                        .last_run
                        .as_ref()
                        .map(|r| r.started_at),
                    Some(SLOT_1),
                    "{}: (a) run started immediately without waiting for the 12:05 retry",
                    variant.label
                );
                assert_eq!(
                    rig.wrapper.of('S', "backup").len(),
                    1,
                    "{}: (a)",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-04-3 (FR-30, Alert): scheduled unreachable run (a) sends no notification until overdue,
/// (b) a manual start is blocked with a visible reason, (c) `ManualRunBlocked` is emitted only when
/// the state changed between display and click.
#[test]
fn ac_m3_04_3_no_alert_until_overdue_and_manual_run_blocked_only_on_state_change() {
    run_isolated_test(
        "ac_m3_04_3_no_alert_until_overdue_and_manual_run_blocked_only_on_state_change",
        "KESTIC_M3_04_3_INNER",
        |root| {
            let (_upower, _nm, _nc, notif, sni) = start_fakes();
            let variant = latest_variant();
            let mut rig = start_rest_rig(root, &variant, "M3U3", BEFORE_SLOT);
            let client = rig.daemon.client.clone();
            let sni_service = wait_for_sni_service(&sni, Duration::from_secs(5));
            let signals = KesticSignalCollector::start(
                &variant.bus_name("M3U3"),
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );
            let manual_blocked = |sigs: &[CapturedKesticSignal]| {
                sigs.iter()
                    .filter(|s| matches!(s, CapturedKesticSignal::ManualRunBlocked { .. }))
                    .count()
            };

            rig.rest.stop();
            client.check_availability(true).expect("CheckAvailability");
            rig.clock.set(SLOT_1);
            rig.wait_pending_unreachable();
            settle();

            // (b) the displayed state disables "back up now" with a visible reason.
            let state = client.get_state().expect("GetState");
            let summary = backup_summary(&state, &rig.backup_id);
            assert!(!summary.availability.available, "(b) backup not available");
            assert_eq!(
                summary.availability.blocked_reason.map(|c| c.as_str()),
                Some(BlockedReason::DestinationUnreachable.as_str()),
                "(b) reason code"
            );
            assert_eq!(
                summary
                    .availability
                    .blocked_reason
                    .map(|r| r.message(summary.availability.source_status).english()),
                Some("Destination unreachable".to_string()),
                "(b) visible reason text"
            );
            let tray_entry = fetch_dbusmenu_items(client.connection(), &sni_service)
                .into_iter()
                .find(|e| e.label.contains("RestBackup"))
                .expect("tray lists the backup");
            assert!(
                !tray_entry.enabled,
                "(b) tray entry disabled: {tray_entry:?}"
            );

            // (a) no desktop notification while merely pending; (c) no ManualRunBlocked without click.
            assert_eq!(
                notif.snapshot().len(),
                0,
                "(a) no notification until overdue: {:?}",
                notif.snapshot()
            );
            assert_eq!(
                manual_blocked(&signals.snapshot()),
                0,
                "(c) no ManualRunBlocked without a click"
            );

            // (a) the second missed slot makes the backup overdue -> first and only notification.
            rig.clock.set(SLOT_2);
            wait_for_state(&client, Duration::from_secs(10), "overdue", |s| {
                backup_summary(s, &rig.backup_id).is_overdue
            });
            wait_for_notifications(&notif, "BackupOverdue", 1, Duration::from_secs(5));
            settle();
            assert_eq!(
                notif.snapshot().len(),
                1,
                "(a) exactly the overdue notification: {:?}",
                notif.snapshot()
            );

            // (c) state change between display and click: shown as available, server then dies.
            client
                .pause_backup(
                    &rig.backup_id_str,
                    &kestic_core::scheduler::PauseDuration::Indefinite,
                )
                .expect("pause so the scheduler does not interfere");
            rig.restart_rest_server();
            let avail = client.check_availability(true).expect("CheckAvailability");
            assert!(
                avail.backups.iter().all(|b| b.available),
                "backup is displayed as available: {avail:?}"
            );
            rig.rest.stop();
            let err = client
                .start_backup(&rig.backup_id_str)
                .expect_err("click on a stale 'available' state must be blocked");
            assert_dbus_error_name(&err, "net.daester.Kestic.Error.ManualRunBlocked");
            let sigs = signals.wait_until(Duration::from_secs(5), |s| manual_blocked(s) >= 1);
            assert_eq!(manual_blocked(&sigs), 1, "(c) exactly one ManualRunBlocked");
            assert!(
                sigs.iter().any(|s| matches!(
                    s,
                    CapturedKesticSignal::ManualRunBlocked { reason, .. }
                        if reason == BlockedReason::DestinationUnreachable.as_str()
                )),
                "(c) ManualRunBlocked reason: {sigs:?}"
            );
            assert_eq!(
                notifications_for(&notif, "ManualRunBlocked").len(),
                1,
                "(c) notification because no window is active"
            );
            drop(signals);
            rig.daemon.stop();
        },
    );
}

/// AC-M3-04-7 (P32): real clock, stopped `rest-server`; the status is `ausstehend` (destination
/// unreachable) at most 15 s after the due time.
#[test]
fn ac_m3_04_7_real_clock_status_pending_within_15_seconds_of_due_time() {
    run_isolated_test(
        "ac_m3_04_7_real_clock_status_pending_within_15_seconds_of_due_time",
        "KESTIC_M3_04_7_INNER",
        |root| {
            let (_upower, _nm, _nc, _notif, _sni) = start_fakes();
            let variant = latest_variant();
            let sub = variant.sub("M3U7");
            let bus = variant.bus_name("M3U7");
            let mut opts = M3DaemonOpts::new(&sub, &bus, &variant.bin, Arc::new(SystemClock));
            opts.timezone = TimezoneSpec::named("UTC");
            let daemon = spawn_m3_daemon(root, opts);
            let rest_data = root.join(&sub).join("rest-data");
            let mut rest = RestServerGuard::start_no_auth(&rest_data);
            let dest = Destination::new(
                "RestRepo",
                DestinationKind::Rest {
                    url: rest.url.clone(),
                    username: None,
                },
            )
            .expect("dest");
            daemon
                .client
                .add_destination(&dest, "init", "rest-pw", "")
                .expect("init");
            rest.stop();

            // Due time: first whole UTC minute at least 4 s away.
            let now = SystemClock.now_utc();
            let mut due = now
                .replace_second(0)
                .expect("second")
                .replace_nanosecond(0)
                .expect("ns")
                + time::Duration::minutes(1);
            if due - now < time::Duration::seconds(4) {
                due += time::Duration::minutes(1);
            }
            let src_dir = root.join(&sub).join("src");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("f.txt"), b"x").expect("write");
            let mut backup =
                Backup::new("RealClock", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            backup.schedule = Schedule::Daily {
                hour: due.hour(),
                minute: due.minute(),
            };
            let bid_str = daemon.client.add_backup(&backup).expect("add backup");
            let bid = BackupId::parse(&bid_str).expect("bid");

            // Before the due time nothing is pending (the measurement below is not trivially true).
            while SystemClock.now_utc() < due - time::Duration::seconds(1) {
                std::thread::sleep(Duration::from_millis(200));
            }
            let early = daemon.client.get_state().expect("GetState");
            assert_eq!(
                backup_summary(&early, &bid).pending_primary_code,
                None,
                "not pending before the due time"
            );

            // (a) measured status latency after the due time.
            let observed = loop {
                let s = daemon.client.get_state().expect("GetState");
                if backup_summary(&s, &bid)
                    .pending_primary_code
                    .map(|c| c.as_str())
                    == Some("destination_unreachable")
                {
                    break SystemClock.now_utc();
                }
                assert!(
                    SystemClock.now_utc() < due + time::Duration::seconds(30),
                    "never became pending (destination unreachable)"
                );
                std::thread::sleep(Duration::from_millis(100));
            };
            let latency = observed - due;
            assert!(
                latency <= time::Duration::seconds(15),
                "(a) pending status {latency} after the due time (limit 15 s)"
            );
            daemon.stop();
        },
    );
}

/// AC-M3-04-11 (FR-30): a NetworkManager network change renews the pre-check at most once per 60 s;
/// (b) "back up now" is disabled with "destination unreachable" afterwards and (c) enabled again
/// after the next check once the server runs.
#[test]
fn ac_m3_04_11_network_change_renews_precheck_at_most_once_per_60_seconds() {
    run_isolated_test(
        "ac_m3_04_11_network_change_renews_precheck_at_most_once_per_60_seconds",
        "KESTIC_M3_04_11_INNER",
        |root| {
            let (_upower, nm, _nc, _notif, _sni) = start_fakes();
            let variant = latest_variant();
            let mut rig =
                start_rest_rig(root, &variant, "M3U11", datetime!(2026-12-01 07:00:00 UTC));
            let client = rig.daemon.client.clone();
            let t0 = rig.clock.now_utc();
            let check_time = |rig: &RestRig| {
                let s = rig.daemon.client.get_state().expect("GetState");
                let d = dest_summary(&s, &rig.dest_id).availability.clone();
                (d.reachable, d.checked_at_unix)
            };

            // Display: reachable.
            let avail = client.check_availability(true).expect("CheckAvailability");
            assert!(
                avail.destinations.iter().all(|d| d.reachable),
                "precondition"
            );

            // Server dies; a network change renews the pre-check (a).
            rig.rest.stop();
            nm.update(false, false, None);
            std::thread::sleep(Duration::from_millis(150));
            nm.update(true, false, Some("Heim"));
            wait_for_state(&client, Duration::from_secs(10), "renewed pre-check", |s| {
                !dest_summary(s, &rig.dest_id).availability.reachable
            });
            let (reachable, checked_first) = check_time(&rig);
            assert!(!reachable, "(a) pre-check renewed after the network change");
            assert_eq!(checked_first, t0.unix_timestamp(), "(a) check ran at t0");
            // (b) "back up now" disabled with the reason "destination unreachable".
            let state = client.get_state().expect("GetState");
            let summary = backup_summary(&state, &rig.backup_id);
            assert!(!summary.availability.available, "(b) disabled");
            assert_eq!(
                summary.availability.blocked_reason.map(|c| c.as_str()),
                Some(BlockedReason::DestinationUnreachable.as_str()),
                "(b) reason"
            );

            // Server is back, but a second network change within 60 s must not renew the check.
            rig.restart_rest_server();
            rig.clock.advance(Duration::from_secs(30));
            nm.update(false, false, None);
            std::thread::sleep(Duration::from_millis(150));
            nm.update(true, false, Some("Heim"));
            settle();
            assert_eq!(
                check_time(&rig),
                (false, checked_first),
                "(a) at most one renewed check per 60 s"
            );

            // After 60 s the next network change renews the check -> enabled again (c).
            rig.clock.advance(Duration::from_secs(40));
            nm.update(false, false, None);
            std::thread::sleep(Duration::from_millis(150));
            nm.update(true, false, Some("Heim"));
            let after = wait_for_state(&client, Duration::from_secs(10), "check renewed", |s| {
                dest_summary(s, &rig.dest_id).availability.reachable
            });
            assert!(
                backup_summary(&after, &rig.backup_id)
                    .availability
                    .available,
                "(c) back up now enabled after the next check"
            );
            assert!(
                dest_summary(&after, &rig.dest_id)
                    .availability
                    .checked_at_unix
                    > checked_first,
                "(c) a new check happened"
            );
            rig.daemon.stop();
        },
    );
}
