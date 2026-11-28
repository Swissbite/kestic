// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Regression tests for the M3 daemon: scheduling, locking and locale-independent DTO codes
//! (`M3-S01`–`M3-S13`).

mod common;

use common::fakes::FakeLogindHandle;
use common::m3::{
    DesktopFakes, FakeSetup, LockableSecretStore, M3DaemonOpts, M3Rig, StdFakes,
    assert_dbus_error_name, backup_summary, connect_rest_repo, count_runs, daemon_paths,
    dest_summary, latest_variant, seed_local_destination, spawn_m3_daemon, wait_for_state,
};
use common::{
    click_dbusmenu_item, fetch_dbusmenu_items, run_isolated_test, start_fake_sni_watcher,
    wait_for_sni_service,
};
use kestic_core::clock::FakeClock;
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationKind, MaintenanceEntrySchedule, NotificationSettings,
    PendingReason, RunKind, RunStatus, Schedule, Source,
};
use kestic_core::mount::MountInspector;
use kestic_core::restic::{ResticFailureKind, classify_exit};
use kestic_core::scheduler::PauseDuration;
use kestic_daemon::dbus_api::{KESTIC_OBJECT_PATH, KesticClient};
use kestic_test_support::process::KillOnDrop;
use kestic_test_support::tools::find_lock_file;
use kestic_test_support::tools::{RestServerGuard, write_executable};
use kestic_test_support::wait::{poll_within, try_poll_every, wait_for_value, wait_until_within};
use std::fs;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use time::macros::datetime;

#[test]
fn m3_maintenance_backoff_prevents_spin_and_notification_flood() {
    run_isolated_test(
        "m3_maintenance_backoff_prevents_spin_and_notification_flood",
        "KESTIC_M3_REG_MAINT_INNER",
        |root| {
            let fakes = StdFakes::start();

            // Sunday 2026-12-06 11:59:55 CET = 10:59:55 UTC
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegMaint",
                datetime!(2026-12-06 10:59:55 UTC),
                |_| {},
            );
            rig.wrapper.fail(
                "prune",
                11,
                r#"{"message_type":"exit_error","code":11,"message":"Fatal: repository locked for test"}"#,
            );
            let state_dir = rig.daemon.state_dir.clone();
            let client = &rig.daemon.client;
            let clock = &rig.clock;

            let fx = rig.add_backup("ManualBk", |b| b.schedule = Schedule::ManualOnly);
            client.start_backup(&fx.backup_id_str).expect("seed snap");
            wait_for_state(client, Duration::from_secs(15), "seed snap ok", |s| {
                backup_summary(s, &fx.backup.id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });

            // Trigger Sunday 12:00 CET maintenance slot -> fails attempt 1, enters 5m backoff.
            fakes.notif.clear();
            clock.set(datetime!(2026-12-06 11:00:05 UTC));
            wait_for_state(
                client,
                Duration::from_secs(15),
                "maintenance failure 1",
                |s| {
                    dest_summary(s, &fx.dest.id)
                        .last_maintenance_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Error)
                },
            );

            // More than one scheduler tick: must NOT re-trigger every second!
            clock.wait_ticks(2);
            let maint_runs_after_1 = RunHistory::open(&state_dir)
                .expect("open history")
                .list_runs()
                .expect("list runs")
                .into_iter()
                .filter(|r| r.kind == RunKind::Maintenance)
                .count();
            assert_eq!(
                maint_runs_after_1, 1,
                "failed maintenance must NOT spin every second"
            );
            let maint_notifs: Vec<_> = fakes
                .notif
                .snapshot()
                .into_iter()
                .filter(|n| n.event_id.as_deref() == Some("MaintenanceFailed"))
                .collect();
            assert_eq!(
                maint_notifs.len(),
                1,
                "failed maintenance must emit exactly 1 notification before backoff"
            );

            // +5m -> attempt 2 fails, still only 1 notification!
            clock.advance(Duration::from_mins(5));
            wait_for_value(
                Duration::from_secs(15),
                "2 finished Error maintenance runs at +5m (finished, running)",
                || {
                    let finished_count = RunHistory::open(&state_dir)
                        .expect("open history")
                        .list_runs()
                        .expect("list runs")
                        .into_iter()
                        .filter(|r| r.kind == RunKind::Maintenance && r.status == RunStatus::Error)
                        .count();
                    let state_now = client.get_state().expect("GetState");
                    (
                        finished_count,
                        dest_summary(&state_now, &fx.dest.id).maintenance_running,
                    )
                },
                |&(finished, running)| finished == 2 && !running,
            );
            let maint_notifs_after_2 = fakes
                .notif
                .snapshot()
                .into_iter()
                .filter(|n| n.event_id.as_deref() == Some("MaintenanceFailed"))
                .count();
            assert_eq!(
                maint_notifs_after_2, 1,
                "scheduled maintenance retries must not spam MaintenanceFailed notifications"
            );

            // Clear failure marker, jump to next Sunday 12:00:05 CET -> backoff resets & maintenance succeeds!
            rig.wrapper.clear_fail("prune");
            clock.set(datetime!(2026-12-13 11:00:05 UTC));
            let ok_state = wait_for_state(
                client,
                Duration::from_secs(15),
                "maintenance recovery ok",
                |s| {
                    let ds = dest_summary(s, &fx.dest.id);
                    ds.last_prune_at_unix.is_some()
                        && ds
                            .last_maintenance_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );
            assert!(!dest_summary(&ok_state, &fx.dest.id).has_problem);

            rig.daemon.stop();
        },
    );
}

#[test]
fn m3_start_all_queues_same_destination_and_notifies_when_warning_disabled() {
    run_isolated_test(
        "m3_start_all_queues_same_destination_and_notifies_when_warning_disabled",
        "KESTIC_M3_REG_STARTALL_INNER",
        |root| {
            let fakes = DesktopFakes::start();
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegStartAll",
                datetime!(2026-12-01 09:00:00 UTC),
                |o| o.enable_monitors = false,
            );
            let client = &rig.daemon.client;
            let conn = client.connection().clone();
            let sni_service = wait_for_sni_service(&fakes.sni_conn, Duration::from_secs(5));

            // Disable warning notifications explicitly (`on_warning: false`).
            client
                .update_settings(
                    2,
                    &NotificationSettings {
                        on_warning: false,
                        ..Default::default()
                    },
                )
                .expect("update settings");

            let manual = |b: &mut Backup| b.schedule = Schedule::ManualOnly;
            let first = rig.add_backup("FirstBk", manual);
            let second = rig.add_backup_to(&first.dest, "SecondBk", manual);
            let src_missing = root.join(&rig.sub).join("does_not_exist");
            let missing = rig.add_backup_to(&first.dest, "MissingBk", |b| {
                b.schedule = Schedule::ManualOnly;
                b.source = Source::folder(&src_missing);
            });
            let id1 = first.backup.id;
            let id2 = second.id;
            let id2_str = id2.as_str().to_string();
            let id_miss_str = missing.id.as_str().to_string();

            rig.wrapper.hold("backup");
            let res = client.start_all_backups().expect("StartAllBackups");
            assert_eq!(
                res.started_run_ids.len(),
                1,
                "FirstBk must start immediately: {res:?}"
            );
            assert_eq!(
                res.queued_backup_ids,
                vec![id2_str],
                "SecondBk on same destination must be queued, not skipped: {res:?}"
            );
            assert_eq!(res.skipped.len(), 1, "Only MissingBk should skip: {res:?}");
            assert_eq!(res.skipped[0].backup_id, id_miss_str);

            rig.wrapper.release("backup");
            wait_for_state(
                client,
                Duration::from_secs(20),
                "both queued backups ok",
                |s| {
                    backup_summary(s, &id1)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                        && backup_summary(s, &id2)
                            .last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );

            // Trigger tray StartAllBackups with `on_warning = false` -> `ManualRunBlocked` notification STILL fires!
            fakes.notif.clear();
            let menu = fetch_dbusmenu_items(&conn, &sni_service);
            let start_all_item = menu
                .iter()
                .find(|m| m.label == "All Backups")
                .expect("tray Back Up All item");
            click_dbusmenu_item(&conn, &sni_service, start_all_item.id);
            wait_until_within(
                Duration::from_secs(5),
                "ManualRunBlocked notification",
                || {
                    fakes
                        .notif
                        .snapshot()
                        .iter()
                        .any(|n| n.event_id.as_deref() == Some("ManualRunBlocked"))
                },
            );
            let blocked_notifs: Vec<_> = fakes
                .notif
                .snapshot()
                .into_iter()
                .filter(|n| n.event_id.as_deref() == Some("ManualRunBlocked"))
                .collect();
            assert_eq!(
                blocked_notifs.len(),
                1,
                "ManualRunBlocked notification must not be suppressed when on_warning == false"
            );
            assert!(
                blocked_notifs[0].body.contains("MissingBk"),
                "Skipped notification must name MissingBk: {:?}",
                blocked_notifs[0]
            );

            rig.daemon.stop();
        },
    );
}

#[test]
fn m3_network_switch_clears_network_backoff_but_preserves_lock_backoff() {
    run_isolated_test(
        "m3_network_switch_clears_network_backoff_but_preserves_lock_backoff",
        "KESTIC_M3_REG_NET_INNER",
        |root| {
            let fakes = StdFakes::start_with(FakeSetup {
                known_networks: &["Heim", "Cafe"],
                ..FakeSetup::default()
            });

            let rest_data = root.join("rest-data");
            let mut rest = RestServerGuard::start_no_auth(&rest_data);
            let rest_port = rest.port;
            let rest_url = rest.url.clone();

            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegNet",
                datetime!(2026-12-01 10:59:55 UTC),
                |_| {},
            );
            let restic_bin = rig.variant.bin.clone();
            let state_dir = rig.daemon.state_dir.clone();
            let client = &rig.daemon.client;
            let clock = &rig.clock;
            let restic_cache = root.join("restic-cache");

            let rest_dest = Destination::new(
                "RestNetDest",
                DestinationKind::Rest {
                    url: rest_url,
                    username: None,
                },
            )
            .expect("rest dest");
            connect_rest_repo(client, &restic_bin, &rest, &rest_dest);
            let rest_id = rig.add_backup_to(&rest_dest, "RestNetBk", |_| {}).id;
            let local = rig.add_backup("LocalLockBk", |_| {});
            let local_repo = local.repo_dir.clone();
            let local_id = local.backup.id.clone();

            // Create exclusive stale lock on local_repo and stop REST server.
            rest.stop();
            let mut excl_child = KillOnDrop::spawn(
                Command::new(&restic_bin)
                    .args(["-r", &local_repo.to_string_lossy(), "check", "--read-data"])
                    .env("RESTIC_PASSWORD", &local.password)
                    .env("RESTIC_CACHE_DIR", &restic_cache),
                "restic check",
            );
            let locks_dir = local_repo.join("locks");
            // A lock file with content: restic has written the whole exclusive lock.
            wait_until_within(Duration::from_secs(10), "exclusive lock", || {
                find_lock_file(&locks_dir).is_some()
            });
            excl_child.kill().expect("kill restic check");
            let _ = excl_child.wait();

            // Trigger 12:00:05 CET -> RestNetBk enters Network backoff, LocalLockBk enters Lock backoff.
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            wait_for_state(client, Duration::from_secs(15), "both in backoff", |s| {
                backup_summary(s, &rest_id)
                    .pending_primary_code
                    .map(|c| c.as_str())
                    == Some("destination_unreachable")
                    && backup_summary(s, &local_id)
                        .pending_primary_code
                        .map(|c| c.as_str())
                        == Some("backoff")
            });

            // Restart rest-server and switch active WiFi Heim -> Cafe (network still online = true!).
            let _rest_restarted = RestServerGuard::start_on_port(&rest_data, rest_port, None);
            fakes.nm.update(true, false, Some("Cafe"));

            // RestNetBk must clear BackoffKind::Network and succeed immediately.
            wait_for_state(
                client,
                Duration::from_secs(15),
                "RestNetBk recovered",
                |s| {
                    backup_summary(s, &rest_id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );

            // More than one scheduler tick: LocalLockBk keeps its Lock backoff (does NOT re-run!).
            clock.wait_ticks(2);
            let local_runs: Vec<_> = RunHistory::open(&state_dir)
                .expect("open history")
                .list_runs()
                .expect("list runs")
                .into_iter()
                .filter(|r| r.backup_id() == Some(&local_id))
                .collect();
            assert_eq!(
                local_runs.len(),
                1,
                "network switch must NOT clear Lock backoff on LocalLockBk"
            );
            let s_after = client.get_state().expect("GetState after net switch");
            assert_eq!(
                backup_summary(&s_after, &local_id)
                    .pending_primary_code
                    .map(|c| c.as_str()),
                Some("backoff")
            );

            rig.daemon.stop();
        },
    );
}

#[test]
fn m3_mid_run_network_drop_is_pending_not_permanent_error() {
    run_isolated_test(
        "m3_mid_run_network_drop_is_pending_not_permanent_error",
        "KESTIC_M3_REG_DROP_INNER",
        |root| {
            let fakes = DesktopFakes::start();
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegDrop",
                datetime!(2026-12-01 10:59:55 UTC),
                |o| {
                    o.enable_monitors = false;
                },
            );
            rig.wrapper
                .fail("backup", 1, "Fatal: connection reset by peer");
            let state_dir = rig.daemon.state_dir.clone();
            let client = &rig.daemon.client;
            let clock = &rig.clock;

            let bid = rig.add_backup("DropBk", |_| {}).backup.id;

            fakes.notif.clear();
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let pending_state = wait_for_state(
                client,
                Duration::from_secs(15),
                "mid-run drop pending",
                |s| {
                    backup_summary(s, &bid)
                        .pending_primary_code
                        .map(|c| c.as_str())
                        == Some("destination_unreachable")
                },
            );
            let bs = backup_summary(&pending_state, &bid);
            assert!(
                !bs.has_error,
                "mid-run network drop must NOT set permanent has_error"
            );
            let hist_runs = RunHistory::open(&state_dir)
                .expect("open history")
                .list_runs()
                .expect("list runs");
            assert_eq!(hist_runs.len(), 1);
            assert_eq!(hist_runs[0].status, RunStatus::Pending);
            assert_eq!(
                hist_runs[0].pending_reason,
                Some(PendingReason::DestinationUnreachable)
            );
            assert!(
                fakes
                    .notif
                    .snapshot()
                    .iter()
                    .all(|n| n.event_id.as_deref() != Some("BackupFailed")),
                "mid-run transient drop must not emit BackupFailed"
            );

            // Network returns -> +5m scheduled retry succeeds Ok (not blocked by has_error!).
            rig.wrapper.clear_fail("backup");
            clock.advance(Duration::from_mins(5));
            wait_for_state(client, Duration::from_secs(15), "retry ok", |s| {
                backup_summary(s, &bid)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
                    && !backup_summary(s, &bid).has_error
            });

            rig.daemon.stop();
        },
    );
}

#[test]
fn m3_pause_overflow_returns_invalid_input_without_panic() {
    run_isolated_test(
        "m3_pause_overflow_returns_invalid_input_without_panic",
        "KESTIC_M3_REG_PAUSE_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegPause",
                datetime!(2026-12-01 10:00:00 UTC),
                |o| {
                    o.enable_monitors = false;
                    o.enable_scheduler = false;
                },
            );
            let client = &rig.daemon.client;
            let bid_str = rig.add_backup("PauseBk", |_| {}).backup_id_str;

            let err_global_max = client
                .pause_global(&PauseDuration::Hours { hours: u32::MAX })
                .expect_err("PauseGlobal(u32::MAX) must fail cleanly");
            assert_dbus_error_name(&err_global_max, "net.daester.Kestic.Error.InvalidInput");

            let err_global_zero = client
                .pause_global(&PauseDuration::Hours { hours: 0 })
                .expect_err("PauseGlobal(0h) must fail cleanly");
            assert_dbus_error_name(&err_global_zero, "net.daester.Kestic.Error.InvalidInput");

            let err_backup_max = client
                .pause_backup(&bid_str, &PauseDuration::Hours { hours: u32::MAX })
                .expect_err("PauseBackup(u32::MAX) must fail cleanly");
            assert_dbus_error_name(&err_backup_max, "net.daester.Kestic.Error.InvalidInput");

            let state = client
                .get_state()
                .expect("daemon remains alive after invalid pause");
            assert!(!state.overall.paused);

            rig.daemon.stop();
        },
    );
}

#[test]
fn m3_unlock_refuses_foreign_host_lock() {
    run_isolated_test(
        "m3_unlock_refuses_foreign_host_lock",
        "KESTIC_M3_REG_LOCK_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let real_restic = latest_variant().bin;

            let sub = root.join("locks");
            let repo_dir = sub.join("repo");
            let wrapper = sub.join("restic-lock-wrapper.sh");
            let unlock_marker = sub.join("unlock_was_called");
            let _restic_cache = sub.join("restic-cache");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");

            let local_host = kestic_core::maintenance::current_local_hostname();
            let local_lock_id = "a".repeat(64);
            let foreign_lock_id = "b".repeat(64);

            // Seed repo with real restic first, then point custom wrapper that simulates
            // both a local stale lock AND a foreign-host lock in `list locks` / `cat lock`.
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let (dest, _password) =
                seed_local_destination(root, &real_restic, "LockDest", &repo_dir);

            write_executable(
                &wrapper,
                &format!(
                    "#!/bin/sh\ncmd=\"$*\"\ncase \"$cmd\" in\n  *\"list locks\"*)\n    printf '{local_lock_id}\\n{foreign_lock_id}\\n'\n    exit 0\n    ;;\n  *\"cat lock {local_lock_id}\"*) \n    printf '{{\"time\":\"2026-12-01T08:00:00Z\",\"exclusive\":true,\"hostname\":\"{local_host}\",\"username\":\"david\",\"pid\":999999}}\\n'\n    exit 0\n    ;;\n  *\"cat lock {foreign_lock_id}\"*) \n    printf '{{\"time\":\"2026-12-01T08:00:00Z\",\"exclusive\":true,\"hostname\":\"foreign-host\",\"username\":\"bob\",\"pid\":12345}}\\n'\n    exit 0\n    ;;\n  *\"unlock\"*)\n    touch \"{}\"\n    exit 0\n    ;;\nesac\nexec \"{}\" \"$@\"\n",
                    unlock_marker.display(),
                    real_restic.display(),
                ),
            );

            let cfg_path = sub.join("cfg").join("kestic").join("config.toml");
            let mut cfg = kestic_core::config::AppConfig::default();
            cfg.destinations.push(dest.clone());
            kestic_core::config::save_config(&cfg_path, &cfg).expect("save config");

            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts {
                    enable_monitors: false,
                    enable_scheduler: false,
                    ..M3DaemonOpts::new("locks", "net.daester.Kestic.M3RevLock", &wrapper, clock)
                },
            );
            let client = &daemon.client;

            let unlock_err = client
                .unlock_destination(dest.id.as_str())
                .expect_err("UnlockDestination must refuse when foreign lock exists");
            assert_dbus_error_name(&unlock_err, "net.daester.Kestic.Error.LockInUse");
            assert!(
                !unlock_marker.exists(),
                "restic unlock must NEVER be invoked when foreign-host locks are present"
            );

            daemon.stop();
        },
    );
}

// ---------------------------------------------------------------------------------------------
// Scheduling and robustness.
// ---------------------------------------------------------------------------------------------

/// A scheduled maintenance whose only work is a failing `forget` must enter the
/// 5/15/60 min backoff instead of re-running `restic forget` on every 1 s scheduler tick.
#[test]
fn m3_failed_maintenance_with_open_forget_enters_backoff() {
    run_isolated_test(
        "m3_failed_maintenance_with_open_forget_enters_backoff",
        "KESTIC_M3_REG_FORGET_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            // Tuesday 2026-12-01 11:00 CET: no prune/check slot is due (defaults: Sunday 12:00).
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegForget",
                datetime!(2026-12-01 10:00:00 UTC),
                |o| {
                    o.enable_monitors = false;
                },
            );
            let state_dir = rig.daemon.state_dir.clone();
            let client = &rig.daemon.client;
            let clock = &rig.clock;

            let fx = rig.add_backup("ForgetBk", |b| b.schedule = Schedule::ManualOnly);
            let dest = fx.dest.clone();
            let bid = fx.backup.id;

            rig.wrapper
                .fail("forget", 1, "Fatal: forget failed for test");
            client.start_backup(bid.as_str()).expect("start backup");
            wait_for_state(
                client,
                Duration::from_secs(20),
                "backup warning + pending forget",
                |s| {
                    let bs = backup_summary(s, &bid);
                    bs.pending_forget
                        && bs
                            .last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Warning)
                },
            );

            let is_maint = |r: &kestic_core::model::Run| r.kind == RunKind::Maintenance;
            let is_finished_maint = |r: &kestic_core::model::Run| {
                r.kind == RunKind::Maintenance && r.status != RunStatus::Running
            };

            // FR-05: the open forget alone does not start a maintenance run before the slot.
            clock.wait_ticks(3);
            assert_eq!(
                count_runs(&state_dir, is_maint),
                0,
                "open forget alone must not start maintenance outside its slot"
            );

            // Sunday 2026-12-06 12:00 CET: maintenance slot; forget and prune both fail.
            rig.wrapper
                .fail("prune", 1, "Fatal: forget failed for test");
            clock.set(datetime!(2026-12-06 11:00:05 UTC));
            wait_until_within(Duration::from_secs(15), "first maintenance attempt", || {
                count_runs(&state_dir, is_finished_maint) == 1
            });
            clock.wait_ticks(3);
            assert_eq!(
                count_runs(&state_dir, is_maint),
                1,
                "failed maintenance must not re-run on every tick"
            );
            let s1 = client.get_state().expect("GetState");
            assert!(
                backup_summary(&s1, &bid).pending_forget,
                "forget stays open after the failed attempt"
            );

            // +5 min -> exactly one retry (attempt 2), which fails again and backs off 15 min.
            clock.advance(Duration::from_secs(5 * 60 + 2));
            wait_until_within(
                Duration::from_secs(15),
                "second maintenance attempt",
                || count_runs(&state_dir, is_finished_maint) == 2,
            );
            clock.wait_ticks(3);
            assert_eq!(
                count_runs(&state_dir, is_maint),
                2,
                "retry after 5 min must happen exactly once"
            );

            // Recovered; +15 min -> attempt 3 succeeds, works off the forget first.
            rig.wrapper.clear_fail("forget");
            rig.wrapper.clear_fail("prune");
            clock.advance(Duration::from_secs(15 * 60 + 2));
            let ok_state = wait_for_state(
                client,
                Duration::from_secs(20),
                "maintenance recovers",
                |s| {
                    !backup_summary(s, &bid).pending_forget
                        && dest_summary(s, &dest.id)
                            .last_maintenance_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );
            assert_eq!(dest_summary(&ok_state, &dest.id).pending_forget_count, 0);
            assert_eq!(count_runs(&state_dir, is_maint), 3);

            rig.daemon.stop();
        },
    );
}

/// TCP listener that accepts connections but never answers until [`BlackHoleServer::respond_ok`].
struct BlackHoleServer {
    accepted: Arc<AtomicUsize>,
    streams: Arc<Mutex<Vec<TcpStream>>>,
    stop: Arc<AtomicBool>,
    port: u16,
    thread: Option<thread::JoinHandle<()>>,
}

impl BlackHoleServer {
    fn bind(port: u16) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind black-hole listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let accepted = Arc::new(AtomicUsize::new(0));
        let streams = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (acc_bg, streams_bg, stop_bg) = (
            Arc::clone(&accepted),
            Arc::clone(&streams),
            Arc::clone(&stop),
        );
        let thread = thread::spawn(move || {
            while !stop_bg.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        streams_bg.lock().expect("lock").push(stream);
                        acc_bg.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(_) => thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Self {
            accepted,
            streams,
            stop,
            port,
            thread: Some(thread),
        }
    }

    fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    /// Answers every held connection with `200 OK`, so a pending preflight succeeds.
    fn respond_ok(&self) {
        let mut streams = self.streams.lock().expect("lock");
        for mut s in streams.drain(..) {
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .expect("write black-hole response");
            s.flush().expect("flush black-hole response");
        }
    }
}

impl Drop for BlackHoleServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.streams.lock().expect("lock").clear();
        if let Some(t) = self.thread.take() {
            t.join().expect("join black-hole thread");
        }
    }
}

/// A scheduled start whose preflight hangs on a black-holed REST destination
/// must not block the daemon main loop, so `PrepareForSleep(true)` is handled (running run
/// interrupted, logind delay inhibitor released) well within logind's 5 s delay window, and a
/// start that completes its preflight after sleep began is still blocked by `sleep_in_progress`.
#[test]
fn m3_hung_preflight_does_not_delay_prepare_for_sleep() {
    run_isolated_test(
        "m3_hung_preflight_does_not_delay_prepare_for_sleep",
        "KESTIC_M3_REG_SLEEP_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            let logind = FakeLogindHandle::start();
            let mut rest = RestServerGuard::start_no_auth(&root.join("rest-data"));
            let rest_port = rest.port;

            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegSleep",
                datetime!(2026-12-01 10:59:00 UTC),
                |_| {},
            );
            let state_dir = rig.daemon.state_dir.clone();
            let client = &rig.daemon.client;
            let clock = &rig.clock;

            let local_id = rig
                .add_backup("SleepLocalBk", |b| b.schedule = Schedule::ManualOnly)
                .backup
                .id;
            let rest_dest = Destination::new(
                "SleepRest",
                DestinationKind::Rest {
                    url: rest.url.clone(),
                    username: None,
                },
            )
            .expect("rest dest");
            connect_rest_repo(client, &rig.variant.bin, &rest, &rest_dest);
            let rest_id = rig.add_backup_to(&rest_dest, "SleepRestBk", |_| {}).id;

            // Replace the REST server with a black hole (accepts, never answers).
            rest.stop();
            let black_hole = BlackHoleServer::bind(rest_port);
            assert_eq!(black_hole.port, rest_port);

            // Long-running manual backup on the local destination holds the logind delay inhibitor.
            rig.wrapper.hold("backup");
            client.start_backup(local_id.as_str()).expect("start local");
            wait_until_within(Duration::from_secs(10), "local run + inhibitor", || {
                let s = client.get_state().expect("GetState");
                backup_summary(&s, &local_id).active_run.is_some()
                    && logind.open_inhibitor_count() == 1
            });

            // REST slot becomes due -> scheduled start runs its preflight against the black hole.
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            wait_until_within(Duration::from_secs(10), "preflight hits black hole", || {
                black_hole.accepted() >= 1
            });
            // One whole tick with the preflight hung in its connection.
            clock.wait_ticks(2);

            let sleep_started = Instant::now();
            logind.emit_prepare_for_sleep(true);
            // The assertions below report a release that does not come in time.
            try_poll_every(Duration::from_secs(15), Duration::from_millis(20), || {
                (logind.open_inhibitor_count() == 0).then_some(())
            });
            let release_elapsed = sleep_started.elapsed();
            assert_eq!(
                logind.open_inhibitor_count(),
                0,
                "logind delay inhibitor must be released after PrepareForSleep(true)"
            );
            assert!(
                release_elapsed < Duration::from_millis(2500),
                "PrepareForSleep(true) handling was delayed by a hung preflight: {release_elapsed:?}"
            );
            let s_sleep = client.get_state().expect("GetState while sleeping");
            assert_eq!(
                backup_summary(&s_sleep, &local_id)
                    .last_run
                    .as_ref()
                    .map(|r| r.status),
                Some(RunStatus::Interrupted)
            );

            // The hung preflight now succeeds: the racing scheduled start must stay blocked.
            black_hole.respond_ok();
            clock.wait_ticks(2);
            let s_after = client.get_state().expect("GetState after racing start");
            assert!(
                backup_summary(&s_after, &rest_id).active_run.is_none(),
                "scheduled start racing with sleep must be blocked by sleep_in_progress"
            );
            assert_eq!(
                count_runs(&state_dir, |r| r.backup_id() == Some(&rest_id)
                    && matches!(r.status, RunStatus::Running | RunStatus::Ok)),
                0,
                "no restic run for the racing scheduled start"
            );

            rig.wrapper.release("backup");
            rig.daemon.stop();
        },
    );
}

/// While the password store is locked, a due scheduled backup must not re-probe
/// the Secret Service (and `restic version`) on every tick, and must still start once the store
/// is unlocked and the cached probe is retried.
#[test]
fn m3_locked_secret_store_does_not_reprobe_every_tick() {
    run_isolated_test(
        "m3_locked_secret_store_does_not_reprobe_every_tick",
        "KESTIC_M3_REG_LOCKED_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            let store = Arc::new(LockableSecretStore::new(false));
            let locked = Arc::clone(&store.locked);
            let checks = Arc::clone(&store.availability_checks);

            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegLocked",
                datetime!(2026-12-01 10:59:55 UTC),
                |o| {
                    o.secret_store = Some(store);
                    o.enable_monitors = false;
                },
            );
            let client = &rig.daemon.client;
            let clock = &rig.clock;
            let bid = rig.add_backup("LockedBk", |_| {}).backup.id;

            locked.store(true, Ordering::SeqCst);
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            wait_for_state(client, Duration::from_secs(15), "store locked", |s| {
                s.blocking.as_ref().map(|b| b.kind.as_str()) == Some("secret_store_locked")
            });
            let checks_after_first = checks.load(Ordering::SeqCst);
            clock.wait_ticks(3);
            assert_eq!(
                checks.load(Ordering::SeqCst),
                checks_after_first,
                "locked store must not be re-probed on every scheduler tick"
            );
            let s_locked = client.get_state().expect("GetState locked");
            assert!(backup_summary(&s_locked, &bid).last_run.is_none());

            // Store unlocked outside Kestic; the cached probe is retried after the cooldown.
            locked.store(false, Ordering::SeqCst);
            clock.advance(Duration::from_secs(61));
            let ok_state = wait_for_state(
                client,
                Duration::from_secs(20),
                "backup runs after unlock",
                |s| {
                    backup_summary(s, &bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );
            assert!(
                ok_state.blocking.is_none(),
                "blocking state clears once the store is unlocked"
            );

            rig.daemon.stop();
        },
    );
}

/// A manual run that ends with a transient network error must not block
/// the next scheduled run like a permanent error, and `does not exist` is no network keyword.
#[test]
fn m3_manual_transient_error_does_not_block_schedule() {
    let misclassified = classify_exit(
        1,
        Some("Fatal: unable to read exclude file /tmp/x: file does not exist"),
        vec![],
        "",
    );
    assert_eq!(
        misclassified.kind,
        ResticFailureKind::GenericFailure,
        "`does not exist` must not be classified as destination unreachable"
    );

    run_isolated_test(
        "m3_manual_transient_error_does_not_block_schedule",
        "KESTIC_M3_REG_TRANSIENT_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegTransient",
                datetime!(2026-12-01 10:00:00 UTC),
                |o| {
                    o.enable_monitors = false;
                },
            );
            let client = &rig.daemon.client;
            let clock = &rig.clock;
            let bid = rig.add_backup("TransBk", |_| {}).backup.id;

            rig.wrapper
                .fail("backup", 1, "Fatal: connection reset by peer");
            client.start_backup(bid.as_str()).expect("start manual");
            wait_for_state(client, Duration::from_secs(15), "manual error", |s| {
                backup_summary(s, &bid)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Error)
            });

            rig.wrapper.clear_fail("backup");
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let ok_state = wait_for_state(
                client,
                Duration::from_secs(20),
                "scheduled run after transient manual error",
                |s| {
                    backup_summary(s, &bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );
            assert!(!backup_summary(&ok_state, &bid).has_error);

            rig.daemon.stop();
        },
    );
}

/// A scheduled run that fails
/// with a non-transient, non-permanent error (Exit 1 «disk full») is alerted but blocks nothing:
/// (a) the same slot does not restart, (b) the next regular slot runs (no schedule block).
#[test]
fn m3_acceptance_disk_full_error_does_not_block_next_slot() {
    assert!(
        !classify_exit(
            1,
            Some("Fatal: unable to save snapshot: write /tmp/pack: no space left on device"),
            vec![],
            "",
        )
        .kind
        .is_permanent(),
        "disk full is no permanent error"
    );

    run_isolated_test(
        "m3_acceptance_disk_full_error_does_not_block_next_slot",
        "KESTIC_M3_ACC_DISKFULL_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegDiskFull",
                datetime!(2026-12-01 10:00:00 UTC),
                |o| {
                    o.enable_monitors = false;
                },
            );
            let state_dir = rig.daemon.state_dir.clone();
            let client = &rig.daemon.client;
            let clock = &rig.clock;
            let bid = rig.add_backup("FullBk", |_| {}).backup.id;

            rig.wrapper.fail(
                "backup",
                1,
                "Fatal: unable to save snapshot: write /tmp/pack: no space left on device",
            );
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            wait_for_state(client, Duration::from_secs(20), "scheduled error", |s| {
                backup_summary(s, &bid)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Error)
            });

            rig.wrapper.clear_fail("backup");
            clock.set(datetime!(2026-12-01 11:30:00 UTC));
            clock.wait_ticks(3);
            assert_eq!(
                count_runs(&state_dir, |r| r.backup_id() == Some(&bid)),
                1,
                "(a) the failed slot must not restart before the next regular slot"
            );

            clock.set(datetime!(2026-12-02 11:00:05 UTC));
            wait_for_state(
                client,
                Duration::from_secs(20),
                "(b) next regular slot runs",
                |s| {
                    backup_summary(s, &bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );

            rig.daemon.stop();
        },
    );
}

/// A user-cancelled scheduled backup or maintenance run counts as handling its
/// slot and must not restart on the next scheduler tick; the next regular slot runs normally.
#[test]
fn m3_user_cancel_does_not_restart_same_slot() {
    run_isolated_test(
        "m3_user_cancel_does_not_restart_same_slot",
        "KESTIC_M3_REG_CANCEL_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegCancel",
                datetime!(2026-12-01 10:59:55 UTC),
                |o| {
                    o.enable_monitors = false;
                },
            );
            let state_dir = rig.daemon.state_dir.clone();
            let client = &rig.daemon.client;
            let clock = &rig.clock;
            let bid = rig.add_backup("CancelBk", |_| {}).backup.id;
            let maint_dest = rig.add_destination("CancelMaint", |d| {
                d.maintenance.prune = MaintenanceEntrySchedule::Daily {
                    hour: 13,
                    minute: 0,
                };
            });
            let is_backup_run = |r: &kestic_core::model::Run| {
                r.backup_id() == Some(&bid) && r.status != RunStatus::Pending
            };
            let is_maint_run = |r: &kestic_core::model::Run| {
                r.kind == RunKind::Maintenance && r.destination_id == maint_dest.id
            };

            // (a) Scheduled backup at 12:00 CET is cancelled by the user.
            rig.wrapper.hold("backup");
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let running = wait_for_state(client, Duration::from_secs(15), "backup running", |s| {
                backup_summary(s, &bid).active_run.is_some()
            });
            let run_id = backup_summary(&running, &bid)
                .active_run
                .as_ref()
                .expect("active run")
                .id
                .as_str()
                .to_string();
            client.cancel_run(&run_id).expect("cancel backup");
            wait_for_state(client, Duration::from_secs(15), "backup cancelled", |s| {
                let bs = backup_summary(s, &bid);
                bs.active_run.is_none()
                    && bs
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Cancelled)
            });
            clock.wait_ticks(3);
            assert_eq!(
                count_runs(&state_dir, is_backup_run),
                1,
                "cancelled scheduled backup must not restart on the next tick"
            );
            rig.wrapper.release("backup");

            // (b) Scheduled maintenance (prune 13:00 CET) is cancelled by the user.
            rig.wrapper.hold("prune");
            clock.set(datetime!(2026-12-01 12:00:05 UTC));
            let maint_running = wait_for_state(
                client,
                Duration::from_secs(15),
                "maintenance running",
                |s| dest_summary(s, &maint_dest.id).maintenance_running,
            );
            let maint_run_id = dest_summary(&maint_running, &maint_dest.id)
                .last_maintenance_run
                .as_ref()
                .expect("maintenance run")
                .id
                .as_str()
                .to_string();
            client
                .cancel_run(&maint_run_id)
                .expect("cancel maintenance");
            wait_for_state(
                client,
                Duration::from_secs(15),
                "maintenance cancelled",
                |s| {
                    let ds = dest_summary(s, &maint_dest.id);
                    !ds.maintenance_running
                        && ds
                            .last_maintenance_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Cancelled)
                },
            );
            clock.wait_ticks(3);
            assert_eq!(
                count_runs(&state_dir, is_maint_run),
                1,
                "cancelled scheduled maintenance must not restart on the next tick"
            );
            rig.wrapper.release("prune");

            // (c) Next regular slots run normally.
            clock.set(datetime!(2026-12-02 11:00:05 UTC));
            wait_for_state(client, Duration::from_secs(20), "next backup slot", |s| {
                backup_summary(s, &bid)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });
            clock.set(datetime!(2026-12-02 12:00:05 UTC));
            wait_for_state(client, Duration::from_secs(20), "next prune slot", |s| {
                dest_summary(s, &maint_dest.id)
                    .last_maintenance_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });

            rig.daemon.stop();
        },
    );
}

/// Mount inspector simulating a hung NFS mount: `read_mountinfo` blocks for the next call only
/// (`hang_next`, until `release_all`) or for every call while `hang_all` is set.
#[derive(Default)]
struct HangingMountInspector {
    hang_next: AtomicBool,
    hang_all: AtomicBool,
    hanging: AtomicBool,
    calls: AtomicUsize,
    release: Mutex<bool>,
    release_cv: Condvar,
}

impl HangingMountInspector {
    fn release_all(&self) {
        *self.release.lock().expect("lock") = true;
        self.release_cv.notify_all();
    }
}

impl MountInspector for HangingMountInspector {
    fn read_mountinfo(&self) -> std::io::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let hang_this_call = self.hang_next.swap(false, Ordering::SeqCst);
        if hang_this_call || self.hang_all.load(Ordering::SeqCst) {
            self.hanging.store(true, Ordering::SeqCst);
            let mut released = self.release.lock().expect("lock");
            while !*released && (hang_this_call || self.hang_all.load(Ordering::SeqCst)) {
                released = self
                    .release_cv
                    .wait_timeout(released, Duration::from_millis(20))
                    .expect("wait")
                    .0;
            }
        }
        Ok(String::new())
    }

    fn fs_uuid_for_device(&self, _source: &str, _mm: &str) -> Option<(String, Option<String>)> {
        None
    }
}

/// A background source probe stuck on a hung mount must not keep
/// `source_probe_in_flight` forever; after it is stale a new probe runs.
#[test]
fn m3_hung_source_probe_is_replaced_when_stale() {
    run_isolated_test(
        "m3_hung_source_probe_is_replaced_when_stale",
        "KESTIC_M3_REG_PROBE_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            let variant = latest_variant();
            let sub = variant.sub("M3RegProbe");
            let repo_dir = root.join(&sub).join("repo");
            let src_dir = root.join(&sub).join("drive-src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("f.txt"), b"drive data").expect("write src");

            let dest = Destination::new("ProbeDest", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            let mut bk =
                Backup::new("DriveBk", Source::drive(&src_dir), dest.id.clone()).expect("bk");
            bk.schedule = Schedule::ManualOnly;
            let mut cfg = kestic_core::config::AppConfig::default();
            cfg.destinations.push(dest);
            cfg.backups.push(bk);
            kestic_core::config::save_config(&daemon_paths(root, &sub).0, &cfg)
                .expect("save config");

            let inspector = Arc::new(HangingMountInspector::default());
            let rig = M3Rig::start(
                root,
                &variant,
                "M3RegProbe",
                datetime!(2026-12-01 10:00:00 UTC),
                |o| {
                    o.mount_inspector = Some(Arc::clone(&inspector) as Arc<dyn MountInspector>);
                    o.enable_monitors = false;
                },
            );
            let clock = &rig.clock;

            inspector.hang_next.store(true, Ordering::SeqCst);
            wait_until_within(Duration::from_secs(5), "background probe hangs", || {
                inspector.hanging.load(Ordering::SeqCst)
            });
            let calls_while_hung = inspector.calls.load(Ordering::SeqCst);
            clock.wait_ticks(2);
            assert_eq!(
                inspector.calls.load(Ordering::SeqCst),
                calls_while_hung,
                "a fresh in-flight probe is not duplicated"
            );

            clock.advance(Duration::from_mins(10));
            wait_until_within(Duration::from_secs(5), "stale probe replaced", || {
                inspector.calls.load(Ordering::SeqCst) > calls_while_hung
            });
            assert!(
                inspector.calls.load(Ordering::SeqCst) > calls_while_hung,
                "a stale hung source probe must be replaced by a new probe"
            );

            inspector.release_all();
            rig.daemon.stop();
        },
    );
}

/// Issue #180, item 2: `DetectSource` reads the mount table without the core's lock, so a hung
/// mount blocks only that call: (a) `GetState` answers while it hangs, (b) at once, and (c)
/// `DetectSource` answers once the mount returns.
#[test]
fn detect_source_on_a_hung_mount_does_not_block_other_calls() {
    run_isolated_test(
        "detect_source_on_a_hung_mount_does_not_block_other_calls",
        "KESTIC_M3_REG_DETECT_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            let variant = latest_variant();
            let inspector = Arc::new(HangingMountInspector::default());
            let rig = M3Rig::start(
                root,
                &variant,
                "M3RegDetect",
                datetime!(2026-12-01 10:00:00 UTC),
                |o| {
                    o.mount_inspector = Some(Arc::clone(&inspector) as Arc<dyn MountInspector>);
                    o.enable_monitors = false;
                },
            );
            let src_dir = root.join("detect-src");
            fs::create_dir_all(&src_dir).expect("mkdir src");

            let calls_before = inspector.calls.load(Ordering::SeqCst);
            inspector.hang_all.store(true, Ordering::SeqCst);
            let bus_name = rig.variant.bus_name("M3RegDetect");
            let src_path = src_dir.to_string_lossy().into_owned();
            let detect = thread::spawn(move || {
                let c = KesticClient::connect(&bus_name, KESTIC_OBJECT_PATH).expect("client");
                c.detect_source(&src_path)
            });
            wait_until_within(Duration::from_secs(5), "DetectSource hangs", || {
                inspector.calls.load(Ordering::SeqCst) > calls_before
                    && inspector.hanging.load(Ordering::SeqCst)
            });

            // A separate thread, so a GetState stuck behind the lock fails the test instead of
            // hanging it.
            let bus_name = rig.variant.bus_name("M3RegDetect");
            let (state_tx, state_rx) = mpsc::channel();
            thread::spawn(move || {
                let c = KesticClient::connect(&bus_name, KESTIC_OBJECT_PATH).expect("client");
                let started = Instant::now();
                let state = c.get_state();
                state_tx.send((state.is_ok(), started.elapsed())).ok();
            });
            let answer = state_rx.recv_timeout(Duration::from_secs(10)).ok();
            inspector.release_all();
            assert!(
                answer.is_some_and(|(ok, _)| ok),
                "(a) GetState answers while DetectSource hangs: {answer:?}"
            );
            let elapsed = answer.map(|(_, elapsed)| elapsed);
            assert!(
                elapsed.is_some_and(|e| e < Duration::from_secs(2)),
                "(b) GetState answers without waiting for the mount: {elapsed:?}"
            );

            let detected = detect
                .join()
                .expect("join DetectSource")
                .expect("DetectSource after the mount returns");
            assert_eq!(
                detected.path, src_dir,
                "(c) DetectSource answers once the mount returns"
            );

            rig.daemon.stop();
        },
    );
}

/// Issue #180, item 5: the child of an isolated test that hangs is killed after its limit:
/// (a) the run is reported as timed out, (b) soon after the limit, (c) with the child's output,
/// and (d) the processes the child started are killed too.
#[test]
fn isolated_test_child_is_killed_after_its_limit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("grandchild.pid");
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(r#"sleep 60 & echo $! > "$1"; echo started; wait"#)
        .arg("sh")
        .arg(&pid_file);
    let started = Instant::now();
    let out = common::output_within(cmd, Duration::from_millis(500));
    let elapsed = started.elapsed();
    assert!(out.timed_out, "(a) the hung child is reported as timed out");
    assert!(
        elapsed < Duration::from_secs(10),
        "(b) the child is killed soon after the limit: {elapsed:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "started\n",
        "(c) the output up to the kill is kept"
    );
    let grandchild = fs::read_to_string(&pid_file).expect("grandchild pid");
    let grandchild = grandchild.trim();
    wait_until_within(
        Duration::from_secs(5),
        "(d) the process the child started is killed",
        || !process_alive(grandchild),
    );
    assert!(
        !process_alive(grandchild),
        "(d) the process the child started is killed"
    );
}

/// Whether `pid` runs: it exists in `/proc` and is not a zombie waiting to be reaped.
fn process_alive(pid: &str) -> bool {
    fs::read_to_string(PathBuf::from("/proc").join(pid).join("stat")).is_ok_and(|stat| {
        stat.rsplit_once(')')
            .is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z'))
    })
}

/// Concurrent queue drains racing for the same destination must not drop a
/// queued backup when its start loses the race (`AlreadyRunning`): every queued backup runs.
#[test]
fn m3_concurrent_drains_keep_queued_backups() {
    run_isolated_test(
        "m3_concurrent_drains_keep_queued_backups",
        "KESTIC_M3_REG_DRAIN_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegDrain",
                datetime!(2026-12-01 10:00:00 UTC),
                |o| {
                    o.enable_monitors = false;
                },
            );
            let bus_name = rig.variant.bus_name("M3RegDrain");
            let client = &rig.daemon.client;
            let dest = rig.add_destination("DrainDest", |_| {});
            let ids: Vec<BackupId> = (0..8)
                .map(|idx| {
                    rig.add_backup_to(&dest, &format!("DrainBk{idx}"), |b| {
                        b.schedule = Schedule::ManualOnly;
                    })
                    .id
                })
                .collect();

            rig.wrapper.hold("backup");
            let res = client.start_all_backups().expect("StartAllBackups");
            assert_eq!(res.started_run_ids.len(), 1, "{res:?}");
            assert_eq!(res.queued_backup_ids.len(), 7, "{res:?}");

            // Hammer extra queue drains (ResumeGlobal drains the queue) while runs hand over.
            let stop = Arc::new(AtomicBool::new(false));
            let mut hammers = Vec::new();
            for _ in 0..12 {
                let stop_h = Arc::clone(&stop);
                let bus = bus_name.clone();
                hammers.push(thread::spawn(move || {
                    let c = KesticClient::connect(&bus, KESTIC_OBJECT_PATH).expect("hammer client");
                    while !stop_h.load(Ordering::SeqCst) {
                        c.resume_global().expect("ResumeGlobal");
                    }
                }));
            }
            rig.wrapper.release("backup");

            let all_ok =
                try_poll_every(Duration::from_secs(45), Duration::from_millis(100), || {
                    let s = client.get_state().expect("GetState");
                    ids.iter()
                        .all(|id| {
                            backup_summary(&s, id)
                                .last_run
                                .as_ref()
                                .is_some_and(|r| r.status == RunStatus::Ok)
                        })
                        .then_some(())
                })
                .is_some();
            stop.store(true, Ordering::SeqCst);
            for h in hammers {
                h.join().expect("join hammer");
            }
            assert!(
                all_ok,
                "every queued backup must eventually run (none dropped by a drain race)"
            );

            rig.daemon.stop();
        },
    );
}

// ---------------------------------------------------------------------------------------------
// Lock handling and locale-independent DTO codes.
// ---------------------------------------------------------------------------------------------

/// Creates a named pipe; reading it blocks until a writer opens it (simulates a log file on a
/// hung network mount).
fn make_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo failed for {}", path.display());
}

/// Opens `fifo` for writing (non-blocking for the reader side) and closes it again, so a
/// blocked reader sees EOF.
fn release_fifo(fifo: &Path) {
    let mut writer = fs::OpenOptions::new()
        .write(true)
        .open(fifo)
        .expect("open fifo for writing");
    writer.write_all(b"").expect("write fifo");
}

/// Opens the write end of `fifo` once a reader has the pipe open (also a reader still waiting
/// in `open`) and returns it; that reader then blocks until the writer is dropped.
fn writer_once_read(fifo: &Path) -> fs::File {
    use std::os::unix::fs::OpenOptionsExt;
    // Linux `O_NONBLOCK`: opening the write end fails instead of waiting while there is no reader.
    const O_NONBLOCK: i32 = 0o4000;
    poll_within(
        Duration::from_secs(10),
        "a reader of the run log pipe",
        || {
            fs::OpenOptions::new()
                .write(true)
                .custom_flags(O_NONBLOCK)
                .open(fifo)
                .ok()
        },
    )
}

/// Seeds a config with one local destination and one backup plus a finished run of that backup
/// whose log is a named pipe; returns `(backup_id, run)`.
fn seed_run_with_fifo_log(
    root: &Path,
    sub_name: &str,
    status: RunStatus,
) -> (BackupId, kestic_core::model::Run, PathBuf) {
    let sub = root.join(sub_name);
    let repo_dir = sub.join("repo");
    let src_dir = sub.join("src");
    fs::create_dir_all(&repo_dir).expect("mkdir repo");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    let dest =
        Destination::new("FifoDest", DestinationKind::Local { path: repo_dir }).expect("dest");
    let mut bk = Backup::new("FifoBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
    bk.schedule = Schedule::ManualOnly;
    let cfg_path = sub.join("cfg").join("kestic").join("config.toml");
    let mut cfg = kestic_core::config::AppConfig::default();
    cfg.destinations.push(dest);
    cfg.backups.push(bk.clone());
    kestic_core::config::save_config(&cfg_path, &cfg).expect("save config");

    let fifo = sub.join("run-log.fifo");
    make_fifo(&fifo);
    let seed_clock = FakeClock::new(datetime!(2026-12-01 09:00:00 UTC));
    let mut run = kestic_core::model::Run::new_backup(&seed_clock, &bk).expect("seed run");
    run.log_path = Some(fifo.clone());
    run.finish(&seed_clock, status, Some(0));
    let state_dir = sub.join("state").join("kestic");
    fs::create_dir_all(&state_dir).expect("mkdir state");
    RunHistory::open(&state_dir)
        .expect("open history")
        .upsert_run(&run, &[])
        .expect("seed run");
    (bk.id, run, fifo)
}

/// Calls `GetState` on a fresh connection in a helper thread; returns whether it answered
/// within `timeout`.
fn get_state_answers_within(bus_name: &str, timeout: Duration) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let bus = bus_name.to_string();
    thread::spawn(move || {
        let c = KesticClient::connect(&bus, KESTIC_OBJECT_PATH).expect("connect");
        let answered = c.get_state().is_ok();
        tx.send(answered).expect("send GetState result");
    });
    rx.recv_timeout(timeout).unwrap_or(false)
}

/// Building the state DTO (GetState, tray) must not read Warning-run log
/// files under the `DaemonCore` lock; unreadable files are cached when the run finishes.
#[test]
fn m3_state_building_does_not_read_run_logs() {
    run_isolated_test(
        "m3_state_building_does_not_read_run_logs",
        "KESTIC_M3_REG_STATELOG_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let variant = latest_variant();
            let (bid, _run, fifo) =
                seed_run_with_fifo_log(root, &variant.sub("M3RegStateLog"), RunStatus::Warning);
            let rig = M3Rig::start(
                root,
                &variant,
                "M3RegStateLog",
                datetime!(2026-12-01 10:00:00 UTC),
                |o| {
                    o.enable_monitors = false;
                    o.enable_scheduler = false;
                },
            );
            let client = &rig.daemon.client;
            let bus_name = variant.bus_name("M3RegStateLog");

            let answered = get_state_answers_within(&bus_name, Duration::from_secs(5));
            if !answered {
                // Unblock a daemon stuck reading the pipe so the test process can finish.
                release_fifo(&fifo);
            }
            assert!(
                answered,
                "GetState must not block on a Warning run's log file"
            );
            let state = client.get_state().expect("GetState");
            assert_eq!(
                backup_summary(&state, &bid)
                    .last_run
                    .as_ref()
                    .map(|r| r.status),
                Some(RunStatus::Warning)
            );

            rig.daemon.stop();
        },
    );
}

/// `GetRunDetail` reads the run log (and resolves restic) without holding
/// the `DaemonCore` lock, so a slow log read never blocks other D-Bus calls.
#[test]
fn m3_run_detail_log_read_does_not_hold_lock() {
    run_isolated_test(
        "m3_run_detail_log_read_does_not_hold_lock",
        "KESTIC_M3_REG_DETAIL_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let variant = latest_variant();
            let (_bid, run, fifo) =
                seed_run_with_fifo_log(root, &variant.sub("M3RegDetail"), RunStatus::Ok);
            let rig = M3Rig::start(
                root,
                &variant,
                "M3RegDetail",
                datetime!(2026-12-01 10:00:00 UTC),
                |o| {
                    o.enable_monitors = false;
                    o.enable_scheduler = false;
                },
            );
            let bus_name = variant.bus_name("M3RegDetail");

            let run_id = run.id.as_str().to_string();
            let bus = bus_name.to_string();
            let detail_thread = thread::spawn(move || {
                let c = KesticClient::connect(&bus, KESTIC_OBJECT_PATH).expect("connect");
                c.get_run_detail(&run_id)
            });
            let writer = writer_once_read(&fifo);
            let answered = get_state_answers_within(&bus_name, Duration::from_secs(3));
            // The reader sees the end of the log once the only writer is gone.
            drop(writer);
            assert!(
                answered,
                "GetState must answer while GetRunDetail waits for a slow log read"
            );
            let detail = detail_thread
                .join()
                .expect("join detail")
                .expect("GetRunDetail");
            assert_eq!(detail.run.id, run.id);
            assert_eq!(detail.run.status, RunStatus::Ok);

            rig.daemon.stop();
        },
    );
}

/// Writes a config with one initialised local destination, a scheduled drive backup, and a
/// manual folder backup on the same destination; returns `(drive_id, folder_id)`.
fn seed_drive_and_folder_backups(root: &Path, sub_name: &str) -> (BackupId, BackupId) {
    let sub = root.join(sub_name);
    let repo_dir = sub.join("repo");
    let drive_src = sub.join("drive-src");
    let folder_src = sub.join("folder-src");
    for d in [&repo_dir, &drive_src, &folder_src] {
        fs::create_dir_all(d).expect("mkdir");
    }
    fs::write(drive_src.join("d.txt"), b"drive").expect("write drive src");
    fs::write(folder_src.join("f.txt"), b"folder").expect("write folder src");
    let (dest, _password) =
        seed_local_destination(root, &latest_variant().bin, "HungDest", &repo_dir);

    let mut drive_bk =
        Backup::new("HungDriveBk", Source::drive(&drive_src), dest.id.clone()).expect("bk");
    drive_bk.schedule = Schedule::Daily {
        hour: 12,
        minute: 0,
    };
    let mut folder_bk =
        Backup::new("FolderBk", Source::folder(&folder_src), dest.id.clone()).expect("bk");
    folder_bk.schedule = Schedule::ManualOnly;
    let ids = (drive_bk.id.clone(), folder_bk.id.clone());
    let cfg_path = sub.join("cfg").join("kestic").join("config.toml");
    let mut cfg = kestic_core::config::AppConfig::default();
    cfg.destinations.push(dest);
    cfg.backups.push(drive_bk);
    cfg.backups.push(folder_bk);
    kestic_core::config::save_config(&cfg_path, &cfg).expect("save config");
    ids
}

/// A source check hanging on a hung mount during a scheduled start is
/// bounded: the backup becomes `ausstehend` (source unavailable) and the destination claim is
/// released, so other backups on that destination can run.
#[test]
fn m3_hung_source_stat_releases_destination_claim() {
    run_isolated_test(
        "m3_hung_source_stat_releases_destination_claim",
        "KESTIC_M3_REG_HUNGSTAT_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            let variant = latest_variant();
            let (drive_id, folder_id) =
                seed_drive_and_folder_backups(root, &variant.sub("M3RegHungStat"));
            let inspector = Arc::new(HangingMountInspector::default());
            let rig = M3Rig::start(
                root,
                &variant,
                "M3RegHungStat",
                datetime!(2026-12-01 10:59:55 UTC),
                |o| {
                    o.mount_inspector = Some(Arc::clone(&inspector) as Arc<dyn MountInspector>);
                    o.enable_monitors = false;
                },
            );
            let client = &rig.daemon.client;
            let clock = &rig.clock;

            inspector.hang_all.store(true, Ordering::SeqCst);
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let pending = wait_for_state(
                client,
                Duration::from_secs(25),
                "hung source -> pending source unavailable",
                |s| {
                    backup_summary(s, &drive_id)
                        .pending_primary_code
                        .map(|c| c.as_str())
                        == Some("source_unavailable")
                },
            );
            assert!(backup_summary(&pending, &drive_id).active_run.is_none());

            let started = client.start_backup(folder_id.as_str());
            inspector.hang_all.store(false, Ordering::SeqCst);
            assert!(
                started.is_ok(),
                "destination claim must be released after the bounded source check: {started:?}"
            );
            wait_for_state(client, Duration::from_secs(20), "folder backup ok", |s| {
                backup_summary(s, &folder_id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });

            rig.daemon.stop();
        },
    );
}

/// A background availability probe stuck on a hung check must not keep
/// `availability_probe_in_flight` forever; once stale, the next trigger starts a new probe.
#[test]
fn m3_hung_availability_probe_is_replaced_when_stale() {
    run_isolated_test(
        "m3_hung_availability_probe_is_replaced_when_stale",
        "KESTIC_M3_REG_AVAILPROBE_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let variant = latest_variant();
            seed_drive_and_folder_backups(root, &variant.sub("M3RegAvailProbe"));
            let inspector = Arc::new(HangingMountInspector::default());
            let rig = M3Rig::start(
                root,
                &variant,
                "M3RegAvailProbe",
                datetime!(2026-12-01 10:00:00 UTC),
                |o| {
                    o.mount_inspector = Some(Arc::clone(&inspector) as Arc<dyn MountInspector>);
                    o.enable_monitors = false;
                    o.enable_scheduler = false;
                },
            );
            let client = &rig.daemon.client;
            let clock = &rig.clock;

            inspector.hang_all.store(true, Ordering::SeqCst);
            clock.advance(Duration::from_secs(61));
            let before_first = inspector.calls.load(Ordering::SeqCst);
            client
                .window_active(true, "overview")
                .expect("WindowActive");
            wait_until_within(Duration::from_secs(5), "first probe hangs", || {
                inspector.calls.load(Ordering::SeqCst) > before_first
            });
            let calls_hung = inspector.calls.load(Ordering::SeqCst);

            clock.advance(Duration::from_mins(10));
            client
                .window_active(true, "overview")
                .expect("WindowActive");
            wait_until_within(Duration::from_secs(3), "stale probe replaced", || {
                inspector.calls.load(Ordering::SeqCst) > calls_hung
            });
            assert!(
                inspector.calls.load(Ordering::SeqCst) > calls_hung,
                "a stale hung availability probe must not block new probes"
            );

            inspector.hang_all.store(false, Ordering::SeqCst);
            rig.daemon.stop();
        },
    );
}

/// State DTOs carry locale-independent codes next to the daemon-locale
/// texts: `error_kind` + `error_detail` + `error_exit_code`, `pending_reason_codes` +
/// `pending_retry_at_unix` (backoff), `availability.source_status`, and
/// `SkippedBackupDto::error_kind`.
#[test]
fn m3_s04_state_dtos_carry_stable_codes() {
    run_isolated_test(
        "m3_s04_state_dtos_carry_stable_codes",
        "KESTIC_M3_REG_CODES_INNER",
        |root| {
            let _fakes = DesktopFakes::start();
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegCodes",
                datetime!(2026-12-01 10:59:55 UTC),
                |o| {
                    o.enable_monitors = false;
                },
            );
            let client = &rig.daemon.client;
            let clock = &rig.clock;

            let manual = |b: &mut Backup| b.schedule = Schedule::ManualOnly;
            let pw = rig.add_backup("PwBk", manual);
            let lock_dest = rig.add_destination("LockDest", |_| {});
            let no_such_dir = root.join(&rig.sub).join("no-such-dir");
            let missing = rig.add_backup_to(&lock_dest, "MissingSrcBk", |b| {
                b.schedule = Schedule::ManualOnly;
                b.source = Source::folder(&no_such_dir);
            });
            let pw_id = pw.backup.id;
            let missing_id = missing.id;

            // (a) Exit 12 -> error_kind `wrong_password`, raw text as detail, exit code 12.
            rig.wrapper
                .fail("backup", 12, "Fatal: wrong password or no key found");
            client.start_backup(pw_id.as_str()).expect("start");
            let s_err = wait_for_state(client, Duration::from_secs(15), "pw error", |s| {
                backup_summary(s, &pw_id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Error)
            });
            let pw = backup_summary(&s_err, &pw_id);
            assert_eq!(pw.error_kind.map(|c| c.as_str()), Some("wrong_password"));
            assert_eq!(pw.error_exit_code, Some(12));
            assert!(
                pw.error_detail
                    .as_deref()
                    .is_some_and(|d| d.contains("wrong password or no key found")),
                "error_detail carries the raw restic text: {:?}",
                pw.error_detail
            );

            // (b) Missing source -> availability.source_status `path_missing`.
            let s_avail = wait_for_state(client, Duration::from_secs(5), "source probe", |s| {
                !backup_summary(s, &missing_id).availability.source_available
            });
            assert_eq!(
                backup_summary(&s_avail, &missing_id)
                    .availability
                    .source_status
                    .map(|c| c.as_str()),
                Some("path_missing")
            );

            // (c) StartAll skips the wrong-password backup with error_kind `wrong_password`.
            let res = client.start_all_backups().expect("StartAllBackups");
            let skipped_pw = res
                .skipped
                .iter()
                .find(|sk| sk.backup_id == pw_id.as_str())
                .expect("pw backup skipped");
            assert_eq!(skipped_pw.reason.as_str(), "permanent_error");
            assert_eq!(
                skipped_pw.error_kind.map(|c| c.as_str()),
                Some("wrong_password")
            );

            // (d) Scheduled Exit 11 -> pending backoff with codes and retry time (+5 min).
            rig.wrapper
                .fail("backup", 11, "Fatal: wrong password or no key found");
            let lock_id = rig.add_backup_to(&lock_dest, "LockBk", |_| {}).id;
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let s_backoff = wait_for_state(client, Duration::from_secs(15), "lock backoff", |s| {
                backup_summary(s, &lock_id)
                    .pending_primary_code
                    .map(|c| c.as_str())
                    == Some("backoff")
            });
            let lb = backup_summary(&s_backoff, &lock_id);
            assert_eq!(
                lb.pending_reason_codes
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>(),
                vec!["backoff"]
            );
            assert_eq!(
                lb.pending_retry_at_unix,
                Some(datetime!(2026-12-01 11:05:05 UTC).unix_timestamp())
            );

            rig.daemon.stop();
        },
    );
}

/// A running maintenance shows maintenance wording («Maintaining ‹Dest›»)
/// in the overall headline and the tray menu instead of «‹Dest›: backing up · 0%».
#[test]
fn m3_running_maintenance_shows_maintenance_wording() {
    run_isolated_test(
        "m3_running_maintenance_shows_maintenance_wording",
        "KESTIC_M3_REG_MAINTTEXT_INNER",
        |root| {
            let fakes = DesktopFakes::start();
            let rig = M3Rig::start(
                root,
                &latest_variant(),
                "M3RegMaintText",
                datetime!(2026-12-01 11:59:55 UTC),
                |o| {
                    o.enable_monitors = false;
                },
            );
            let client = &rig.daemon.client;
            let clock = &rig.clock;
            let sni_service = wait_for_sni_service(&fakes.sni_conn, Duration::from_secs(5));
            let dest = rig.add_destination("MaintZiel", |d| {
                d.maintenance.prune = MaintenanceEntrySchedule::Daily {
                    hour: 13,
                    minute: 0,
                };
            });

            rig.wrapper.hold("prune");
            clock.set(datetime!(2026-12-01 12:00:05 UTC));
            let running = wait_for_state(client, Duration::from_secs(15), "maintenance", |s| {
                dest_summary(s, &dest.id).maintenance_running
            });
            let active = running.overall.active_run_info();
            assert_eq!(
                kestic_i18n::messages::overall_headline(
                    &running.overall.counts(running.backups.len()),
                    active.as_ref(),
                    None,
                )
                .english(),
                "Maintaining MaintZiel",
                "C3: overall headline uses maintenance wording"
            );
            let items = fetch_dbusmenu_items(client.connection(), &sni_service);
            let labels: Vec<&str> = items.iter().map(|m| m.label.as_str()).collect();
            assert!(
                labels.contains(&"Maintaining MaintZiel"),
                "C3: tray menu shows maintenance wording: {labels:?}"
            );
            assert!(
                labels.iter().all(|l| !l.contains("backing up")),
                "C3: no backup wording for a maintenance run: {labels:?}"
            );

            rig.wrapper.release("prune");
            rig.daemon.stop();
        },
    );
}
