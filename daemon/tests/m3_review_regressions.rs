// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Hardened regression tests for M3 code-review fixes and test-strategy compliance (`M3-S01`–`M3-S13`).

mod common;

use common::fakes::{FakeLogindHandle, FakeNetworkManagerHandle, FakeUPowerHandle};
use common::{
    RestServerGuard, click_dbusmenu_item, fetch_dbusmenu_items, find_kestic_binary,
    restic_test_binaries, run_isolated_test, start_fake_notifications, start_fake_sni_watcher,
    wait_for_file, wait_for_sni_service,
};
use kestic_core::clock::FakeClock;
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationKind, MaintenanceEntrySchedule, NotificationSettings,
    PendingReason, ResticProvider, RunKind, RunStatus, Schedule, Source,
};
use kestic_core::mount::MountInspector;
use kestic_core::provider::check_binary_version;
use kestic_core::restic::{ResticFailureKind, ResticRuntime, classify_exit};
use kestic_core::scheduler::PauseDuration;
use kestic_core::secrets::{FileSecretStore, SecretError, SecretStore, SecretValue};
use kestic_daemon::dbus_api::{
    BackupSummaryDto, DaemonStateDto, DestinationSummaryDto, KESTIC_OBJECT_PATH, KesticClient,
};
use kestic_daemon::monitors::MonitorBusConfig;
use kestic_daemon::service::{DaemonOutcome, DaemonRunConfig, run_daemon};
use std::fs;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use time::macros::datetime;

fn default_restic_latest() -> PathBuf {
    restic_test_binaries()
        .into_iter()
        .last()
        .expect("latest restic binary")
        .1
}

fn wait_for_state<F>(
    client: &KesticClient,
    timeout: Duration,
    label: &str,
    mut pred: F,
) -> DaemonStateDto
where
    F: FnMut(&DaemonStateDto) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let state = client.get_state().expect("GetState");
        if pred(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for condition ({label}); last overall={:?}",
            state.overall
        );
        thread::sleep(Duration::from_millis(40));
    }
}

fn backup_summary<'a>(state: &'a DaemonStateDto, id: &BackupId) -> &'a BackupSummaryDto {
    state
        .backups
        .iter()
        .find(|b| &b.backup.id == id)
        .expect("backup summary present")
}

fn dest_summary<'a>(
    state: &'a DaemonStateDto,
    dest_id: &kestic_core::model::DestinationId,
) -> &'a DestinationSummaryDto {
    state
        .destinations
        .iter()
        .find(|d| &d.destination.id == dest_id)
        .expect("dest summary present")
}

fn assert_dbus_error_name(err: &zbus::Error, expected_name: &str) {
    match err {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(
                name.as_str(),
                expected_name,
                "unexpected D-Bus error: {err:?}"
            );
        }
        other => panic!("expected D-Bus MethodError `{expected_name}`, got {other:?}"),
    }
}

fn spawn_review_daemon(
    root: &Path,
    sub_name: &str,
    bus_name: &str,
    restic_bin: &Path,
    clock: Arc<FakeClock>,
    enable_monitors: bool,
    enable_scheduler: bool,
) -> (
    PathBuf,
    KesticClient,
    thread::JoinHandle<Result<DaemonOutcome, String>>,
) {
    let sub = root.join(sub_name);
    let cfg_path = sub.join("cfg").join("kestic").join("config.toml");
    let state_dir = sub.join("state").join("kestic");
    let ready_file = sub.join("daemon.ready");
    let secret_file = root.join("secrets.hex");

    let run_cfg = DaemonRunConfig {
        bus_name: bus_name.to_string(),
        assume_sni_available: true,
        ready_file: Some(ready_file.clone()),
        config_path: Some(cfg_path),
        state_dir: Some(state_dir.clone()),
        kestic_binary: Some(find_kestic_binary()),
        clock,
        secret_store: Some(Arc::new(FileSecretStore::new(secret_file))),
        suppress_ui_spawn: true,
        enable_job_view: false,
        enable_portal_inhibit: false,
        timezone: kestic_core::scheduler::TimezoneSpec::named("Europe/Zurich"),
        monitor_bus: MonitorBusConfig {
            use_session_bus: true,
            enable_watcher: enable_monitors,
            ..Default::default()
        },
        enable_system_monitors: enable_monitors,
        enable_scheduler,
        ..Default::default()
    };

    let handle = thread::spawn(move || run_daemon(run_cfg));
    wait_for_file(&ready_file, common::DAEMON_READY_TIMEOUT);

    let client = KesticClient::connect(bus_name, KESTIC_OBJECT_PATH).expect("connect KesticClient");
    let prov = client
        .set_restic_provider(&ResticProvider::Custom {
            path: restic_bin.to_path_buf(),
        })
        .expect("set restic provider");
    assert!(prov.valid, "restic provider must be valid: {prov:?}");
    (state_dir, client, handle)
}

#[test]
fn m3_review_maintenance_backoff_prevents_spin_and_notification_flood() {
    run_isolated_test(
        "m3_review_maintenance_backoff_prevents_spin_and_notification_flood",
        "KESTIC_M3_REV_MAINT_INNER",
        |root| {
            let (_notif_conn, notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let _upower = FakeUPowerHandle::start(false, 100.0);
            let _nm = FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]);

            let real_restic = default_restic_latest();
            let sub = root.join("maint");
            let fail_marker = sub.join("fail_prune");
            let wrapper = sub.join("restic-maint-wrapper.sh");
            let repo_dir = sub.join("repo");
            let src_dir = sub.join("src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("f.txt"), b"data").expect("write src");
            fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"prune\" ] && [ -f \"{}\" ]; then\n    printf '{{\"message_type\":\"exit_error\",\"code\":11,\"message\":\"Fatal: repository locked for test\"}}\\n' >&2\n    exit 11\n  fi\ndone\nexec \"{}\" \"$@\"\n",
                    fail_marker.display(),
                    real_restic.display(),
                ),
            )
            .expect("write wrapper");
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
                .expect("chmod wrapper");
            fs::write(&fail_marker, b"1").expect("create fail marker");

            // Sunday 2026-12-06 11:59:55 CET = 10:59:55 UTC
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-06 10:59:55 UTC)));
            let (state_dir, client, handle) = spawn_review_daemon(
                root,
                "maint",
                "net.daester.Kestic.M3RevMaint",
                &wrapper,
                Arc::clone(&clock),
                true,
                true,
            );

            let dest = Destination::new(
                "MaintDest",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            client
                .add_destination(&dest, "init", "pw-maint", "")
                .expect("init dest");
            let mut bk =
                Backup::new("ManualBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            bk.schedule = Schedule::ManualOnly;
            let bid_str = client.add_backup(&bk).expect("add bk");
            let _ = client.start_backup(&bid_str).expect("seed snap");
            wait_for_state(&client, Duration::from_secs(15), "seed snap ok", |s| {
                backup_summary(s, &bk.id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });

            // Trigger Sunday 12:00 CET maintenance slot -> fails attempt 1, enters 5m backoff.
            notif_capture.clear();
            clock.set(datetime!(2026-12-06 11:00:05 UTC));
            wait_for_state(
                &client,
                Duration::from_secs(15),
                "maintenance failure 1",
                |s| {
                    dest_summary(s, &dest.id)
                        .last_maintenance_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Error)
                },
            );

            // Wait >1 scheduler tick (1300ms): must NOT re-trigger every second!
            thread::sleep(Duration::from_millis(1300));
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
            let maint_notifs: Vec<_> = notif_capture
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
            clock.advance(Duration::from_secs(5 * 60));
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let finished_count = RunHistory::open(&state_dir)
                    .expect("open history")
                    .list_runs()
                    .expect("list runs")
                    .into_iter()
                    .filter(|r| r.kind == RunKind::Maintenance && r.status == RunStatus::Error)
                    .count();
                let state_now = client.get_state().expect("GetState");
                let still_running = dest_summary(&state_now, &dest.id).maintenance_running;
                if finished_count == 2 && !still_running {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "expected 2 finished Error maintenance runs at +5m, got {finished_count} (running={still_running})"
                );
                thread::sleep(Duration::from_millis(40));
            }
            let maint_notifs_after_2 = notif_capture
                .snapshot()
                .into_iter()
                .filter(|n| n.event_id.as_deref() == Some("MaintenanceFailed"))
                .count();
            assert_eq!(
                maint_notifs_after_2, 1,
                "scheduled maintenance retries must not spam MaintenanceFailed notifications"
            );

            // Clear failure marker, jump to next Sunday 12:00:05 CET -> backoff resets & maintenance succeeds!
            fs::remove_file(&fail_marker).expect("remove fail marker");
            clock.set(datetime!(2026-12-13 11:00:05 UTC));
            let ok_state = wait_for_state(
                &client,
                Duration::from_secs(15),
                "maintenance recovery ok",
                |s| {
                    let ds = dest_summary(s, &dest.id);
                    ds.last_prune_at_unix.is_some()
                        && ds
                            .last_maintenance_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );
            assert!(!dest_summary(&ok_state, &dest.id).has_problem);

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_review_start_all_queues_same_destination_and_notifies_when_warning_disabled() {
    run_isolated_test(
        "m3_review_start_all_queues_same_destination_and_notifies_when_warning_disabled",
        "KESTIC_M3_REV_STARTALL_INNER",
        |root| {
            let (_notif_conn, notif_capture) = start_fake_notifications();
            let sni_conn = start_fake_sni_watcher();
            let real_restic = default_restic_latest();

            let sub = root.join("sa");
            let hold_file = sub.join("hold_backup");
            let wrapper = sub.join("restic-sa-wrapper.sh");
            let repo_dir = sub.join("repo");
            let src1 = sub.join("src1");
            let src2 = sub.join("src2");
            let src_missing = sub.join("does_not_exist");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src1).expect("mkdir src1");
            fs::create_dir_all(&src2).expect("mkdir src2");
            fs::write(src1.join("a.txt"), b"alpha").expect("write a");
            fs::write(src2.join("b.txt"), b"beta").expect("write b");
            fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nis_backup=0\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"backup\" ]; then\n    is_backup=1\n    break\n  fi\ndone\nif [ \"$is_backup\" = \"1\" ] && [ -f \"{}\" ]; then\n  trap 'kill \"$pid\" 2>/dev/null; exit 143' TERM INT\n  while [ -f \"{}\" ]; do\n    sleep 0.05 &\n    pid=$!\n    wait \"$pid\"\n  done\nfi\nexec \"{}\" \"$@\"\n",
                    hold_file.display(),
                    hold_file.display(),
                    real_restic.display(),
                ),
            )
            .expect("write wrapper");
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
                .expect("chmod wrapper");

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 09:00:00 UTC)));
            let (_state_dir, client, handle) = spawn_review_daemon(
                root,
                "sa",
                "net.daester.Kestic.M3RevSA",
                &wrapper,
                Arc::clone(&clock),
                false,
                true,
            );
            let conn = client.connection().clone();
            let sni_service = wait_for_sni_service(&sni_conn, Duration::from_secs(5));

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

            let dest = Destination::new("SharedSA", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            client
                .add_destination(&dest, "init", "pw-sa", "")
                .expect("init dest");

            let mut bk1 =
                Backup::new("FirstBk", Source::folder(&src1), dest.id.clone()).expect("bk1");
            bk1.schedule = Schedule::ManualOnly;
            let mut bk2 =
                Backup::new("SecondBk", Source::folder(&src2), dest.id.clone()).expect("bk2");
            bk2.schedule = Schedule::ManualOnly;
            let mut bk_missing =
                Backup::new("MissingBk", Source::folder(&src_missing), dest.id.clone())
                    .expect("bk missing");
            bk_missing.schedule = Schedule::ManualOnly;

            let id1_str = client.add_backup(&bk1).expect("add bk1");
            let id2_str = client.add_backup(&bk2).expect("add bk2");
            let id_miss_str = client.add_backup(&bk_missing).expect("add missing");
            let id1 = BackupId::parse(&id1_str).expect("id1");
            let id2 = BackupId::parse(&id2_str).expect("id2");

            fs::write(&hold_file, b"1").expect("enable hold");
            let res = client.start_all_backups().expect("StartAllBackups");
            assert_eq!(
                res.started_run_ids.len(),
                1,
                "FirstBk must start immediately: {res:?}"
            );
            assert_eq!(
                res.queued_backup_ids,
                vec![id2_str.clone()],
                "SecondBk on same destination must be queued, not skipped: {res:?}"
            );
            assert_eq!(res.skipped.len(), 1, "Only MissingBk should skip: {res:?}");
            assert_eq!(res.skipped[0].backup_id, id_miss_str);

            fs::remove_file(&hold_file).expect("release hold");
            wait_for_state(
                &client,
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
            notif_capture.clear();
            let menu = fetch_dbusmenu_items(&conn, &sni_service);
            let start_all_item = menu
                .iter()
                .find(|m| {
                    m.label == "All Backups"
                        || m.label.contains("Back Up All")
                        || m.label.contains("Back up all")
                        || m.label == "Back Up All Now"
                })
                .expect("tray Back Up All item");
            click_dbusmenu_item(&conn, &sni_service, start_all_item.id);
            let notif_deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < notif_deadline {
                if notif_capture
                    .snapshot()
                    .iter()
                    .any(|n| n.event_id.as_deref() == Some("ManualRunBlocked"))
                {
                    break;
                }
                thread::sleep(Duration::from_millis(40));
            }
            let blocked_notifs: Vec<_> = notif_capture
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

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_review_network_switch_clears_network_backoff_but_preserves_lock_backoff() {
    run_isolated_test(
        "m3_review_network_switch_clears_network_backoff_but_preserves_lock_backoff",
        "KESTIC_M3_REV_NET_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let _upower = FakeUPowerHandle::start(false, 100.0);
            let nm = FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim", "Cafe"]);

            let restic_bin = default_restic_latest();
            let rest_data = root.join("rest-data");
            let mut rest = RestServerGuard::start_no_auth(&rest_data);
            let rest_port = rest.port;
            let rest_url = rest.url.clone();

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:55 UTC)));
            let (state_dir, client, handle) = spawn_review_daemon(
                root,
                "net",
                "net.daester.Kestic.M3RevNet",
                &restic_bin,
                Arc::clone(&clock),
                true,
                true,
            );

            let src_dir = root.join("src");
            let local_repo = root.join("local-repo");
            let restic_cache = root.join("restic-cache");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::create_dir_all(&local_repo).expect("mkdir local repo");
            fs::write(src_dir.join("file.txt"), b"network test").expect("write file");

            let rest_dest = Destination::new(
                "RestNetDest",
                DestinationKind::Rest {
                    url: rest_url,
                    username: None,
                },
            )
            .expect("rest dest");
            client
                .add_destination(&rest_dest, "init", "pw-rest", "")
                .expect("init rest");

            let local_dest = Destination::new(
                "LocalLockDest",
                DestinationKind::Local {
                    path: local_repo.clone(),
                },
            )
            .expect("local dest");
            client
                .add_destination(&local_dest, "init", "pw-local", "")
                .expect("init local");

            let mut rest_bk =
                Backup::new("RestNetBk", Source::folder(&src_dir), rest_dest.id.clone())
                    .expect("rest bk");
            rest_bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let rest_id_str = client.add_backup(&rest_bk).expect("add rest bk");
            let rest_id = BackupId::parse(&rest_id_str).expect("rest id");

            let mut local_bk = Backup::new(
                "LocalLockBk",
                Source::folder(&src_dir),
                local_dest.id.clone(),
            )
            .expect("local bk");
            local_bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let local_id_str = client.add_backup(&local_bk).expect("add local bk");
            let local_id = BackupId::parse(&local_id_str).expect("local id");

            // Create exclusive stale lock on local_repo and stop REST server.
            rest.stop();
            let mut excl_child = Command::new(&restic_bin)
                .args(["-r", &local_repo.to_string_lossy(), "check", "--read-data"])
                .env("RESTIC_PASSWORD", "pw-local")
                .env("RESTIC_CACHE_DIR", &restic_cache)
                .spawn()
                .expect("spawn restic check");
            let locks_dir = local_repo.join("locks");
            let lock_wait = Instant::now() + Duration::from_secs(10);
            while !fs::read_dir(&locks_dir)
                .ok()
                .is_some_and(|mut it| it.any(|e| e.is_ok()))
            {
                assert!(Instant::now() < lock_wait, "exclusive lock never appeared");
                thread::sleep(Duration::from_millis(5));
            }
            thread::sleep(Duration::from_millis(30));
            excl_child.kill().expect("kill restic check");
            let _ = excl_child.wait();

            // Trigger 12:00:05 CET -> RestNetBk enters Network backoff, LocalLockBk enters Lock backoff.
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            wait_for_state(&client, Duration::from_secs(15), "both in backoff", |s| {
                backup_summary(s, &rest_id).pending_primary_code.as_deref()
                    == Some("destination_unreachable")
                    && backup_summary(s, &local_id).pending_primary_code.as_deref()
                        == Some("backoff")
            });

            // Restart rest-server and switch active WiFi Heim -> Cafe (network still online = true!).
            let _rest_restarted = RestServerGuard::start_on_port(&rest_data, rest_port, None);
            nm.update(true, false, Some("Cafe"));

            // RestNetBk must clear BackoffKind::Network and succeed immediately.
            wait_for_state(
                &client,
                Duration::from_secs(15),
                "RestNetBk recovered",
                |s| {
                    backup_summary(s, &rest_id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );

            // Wait >1 scheduler tick and verify LocalLockBk kept its Lock backoff (did NOT re-run!).
            thread::sleep(Duration::from_millis(1200));
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
                    .as_deref(),
                Some("backoff")
            );

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_review_mid_run_network_drop_is_pending_not_permanent_error() {
    run_isolated_test(
        "m3_review_mid_run_network_drop_is_pending_not_permanent_error",
        "KESTIC_M3_REV_DROP_INNER",
        |root| {
            let (_notif_conn, notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let real_restic = default_restic_latest();

            let sub = root.join("drop");
            let drop_marker = sub.join("drop_net");
            let wrapper = sub.join("restic-drop-wrapper.sh");
            let repo_dir = sub.join("repo");
            let src_dir = sub.join("src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("f.txt"), b"mid run drop").expect("write");
            fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"backup\" ] && [ -f \"{}\" ]; then\n    printf 'Fatal: connection reset by peer\\n' >&2\n    exit 1\n  fi\ndone\nexec \"{}\" \"$@\"\n",
                    drop_marker.display(),
                    real_restic.display(),
                ),
            )
            .expect("write wrapper");
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
                .expect("chmod wrapper");
            fs::write(&drop_marker, b"1").expect("create drop marker");

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:55 UTC)));
            let (state_dir, client, handle) = spawn_review_daemon(
                root,
                "drop",
                "net.daester.Kestic.M3RevDrop",
                &wrapper,
                Arc::clone(&clock),
                false,
                true,
            );

            let dest =
                Destination::new("DropDest", DestinationKind::Local { path: repo_dir }).expect("d");
            client
                .add_destination(&dest, "init", "pw-drop", "")
                .expect("init");
            let mut bk =
                Backup::new("DropBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let bid_str = client.add_backup(&bk).expect("add");
            let bid = BackupId::parse(&bid_str).expect("bid");

            notif_capture.clear();
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let pending_state = wait_for_state(
                &client,
                Duration::from_secs(15),
                "mid-run drop pending",
                |s| {
                    backup_summary(s, &bid).pending_primary_code.as_deref()
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
                notif_capture
                    .snapshot()
                    .iter()
                    .all(|n| n.event_id.as_deref() != Some("BackupFailed")),
                "mid-run transient drop must not emit BackupFailed"
            );

            // Network returns -> +5m scheduled retry succeeds Ok (not blocked by has_error!).
            fs::remove_file(&drop_marker).expect("clear drop marker");
            clock.advance(Duration::from_secs(5 * 60));
            wait_for_state(&client, Duration::from_secs(15), "retry ok", |s| {
                backup_summary(s, &bid)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
                    && !backup_summary(s, &bid).has_error
            });

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_review_pause_overflow_returns_invalid_input_without_panic() {
    run_isolated_test(
        "m3_review_pause_overflow_returns_invalid_input_without_panic",
        "KESTIC_M3_REV_PAUSE_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let restic_bin = default_restic_latest();
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let (_state_dir, client, handle) = spawn_review_daemon(
                root,
                "pause",
                "net.daester.Kestic.M3RevPause",
                &restic_bin,
                Arc::clone(&clock),
                false,
                false,
            );

            let repo_dir = root.join("repo");
            let src_dir = root.join("src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            let dest = Destination::new("PauseDest", DestinationKind::Local { path: repo_dir })
                .expect("d");
            client
                .add_destination(&dest, "init", "pw-p", "")
                .expect("init");
            let bk = Backup::new("PauseBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            let bid_str = client.add_backup(&bk).expect("add");

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

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_review_unlock_refuses_foreign_host_lock() {
    run_isolated_test(
        "m3_review_unlock_refuses_foreign_host_lock",
        "KESTIC_M3_REV_LOCK_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let real_restic = default_restic_latest();

            let sub = root.join("locks");
            let repo_dir = sub.join("repo");
            let wrapper = sub.join("restic-lock-wrapper.sh");
            let unlock_marker = sub.join("unlock_was_called");
            let restic_cache = sub.join("restic-cache");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");

            let local_host = kestic_core::maintenance::current_local_hostname();
            let local_lock_id = "a".repeat(64);
            let foreign_lock_id = "b".repeat(64);

            // Seed repo with real restic first, then point custom wrapper that simulates
            // both a local stale lock AND a foreign-host lock in `list locks` / `cat lock`.
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let dest = Destination::new(
                "LockDest",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("d");
            let store = FileSecretStore::new(root.join("secrets.hex"));
            let resolved = check_binary_version(&real_restic).expect("resolved");
            let kbin = find_kestic_binary();
            let rt = ResticRuntime {
                resolved: &resolved,
                kestic_binary: &kbin,
                secret_store: &store,
                clock: clock.as_ref(),
                cache_dir: Some(&restic_cache),
            };
            kestic_core::destination::setup_destination(
                &dest,
                kestic_core::destination::DestinationSetupMode::InitNew,
                &SecretValue::from_str_secret("pw-lk"),
                None,
                &rt,
            )
            .expect("init repo");

            fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\ncmd=\"$*\"\ncase \"$cmd\" in\n  *\"list locks\"*)\n    printf '{local_lock_id}\\n{foreign_lock_id}\\n'\n    exit 0\n    ;;\n  *\"cat lock {local_lock_id}\"*) \n    printf '{{\"time\":\"2026-12-01T08:00:00Z\",\"exclusive\":true,\"hostname\":\"{local_host}\",\"username\":\"david\",\"pid\":999999}}\\n'\n    exit 0\n    ;;\n  *\"cat lock {foreign_lock_id}\"*) \n    printf '{{\"time\":\"2026-12-01T08:00:00Z\",\"exclusive\":true,\"hostname\":\"foreign-host\",\"username\":\"bob\",\"pid\":12345}}\\n'\n    exit 0\n    ;;\n  *\"unlock\"*)\n    touch \"{}\"\n    exit 0\n    ;;\nesac\nexec \"{}\" \"$@\"\n",
                    unlock_marker.display(),
                    real_restic.display(),
                ),
            )
            .expect("write lock wrapper");
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
                .expect("chmod wrapper");

            let cfg_path = sub.join("cfg").join("kestic").join("config.toml");
            let mut cfg = kestic_core::config::AppConfig::default();
            cfg.destinations.push(dest.clone());
            kestic_core::config::save_config(&cfg_path, &cfg).expect("save config");

            let (_state_dir, client, handle) = spawn_review_daemon(
                root,
                "locks",
                "net.daester.Kestic.M3RevLock",
                &wrapper,
                Arc::clone(&clock),
                false,
                false,
            );

            let unlock_err = client
                .unlock_destination(dest.id.as_str())
                .expect_err("UnlockDestination must refuse when foreign lock exists");
            assert_dbus_error_name(&unlock_err, "net.daester.Kestic.Error.InvalidInput");
            assert!(
                !unlock_marker.exists(),
                "restic unlock must NEVER be invoked when foreign-host locks are present"
            );

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

// ---------------------------------------------------------------------------------------------
// Second review round: regression tests for daemon scheduling/robustness fixes.
// ---------------------------------------------------------------------------------------------

/// Options for [`spawn_round2_daemon`].
struct Round2DaemonOpts<'a> {
    sub_name: &'a str,
    bus_name: &'a str,
    restic_bin: &'a Path,
    clock: Arc<FakeClock>,
    secret_store: Option<Arc<dyn SecretStore + Send + Sync>>,
    mount_inspector: Option<Arc<dyn MountInspector>>,
    enable_monitors: bool,
    enable_scheduler: bool,
}

fn spawn_round2_daemon(
    root: &Path,
    opts: Round2DaemonOpts<'_>,
) -> (
    PathBuf,
    KesticClient,
    thread::JoinHandle<Result<DaemonOutcome, String>>,
) {
    let sub = root.join(opts.sub_name);
    let cfg_path = sub.join("cfg").join("kestic").join("config.toml");
    let state_dir = sub.join("state").join("kestic");
    let ready_file = sub.join("daemon.ready");
    let secret_store: Arc<dyn SecretStore + Send + Sync> = opts
        .secret_store
        .unwrap_or_else(|| Arc::new(FileSecretStore::new(root.join("secrets.hex"))));

    let run_cfg = DaemonRunConfig {
        bus_name: opts.bus_name.to_string(),
        assume_sni_available: true,
        ready_file: Some(ready_file.clone()),
        config_path: Some(cfg_path),
        state_dir: Some(state_dir.clone()),
        kestic_binary: Some(find_kestic_binary()),
        clock: opts.clock,
        secret_store: Some(secret_store),
        mount_inspector: opts.mount_inspector,
        suppress_ui_spawn: true,
        enable_job_view: false,
        enable_portal_inhibit: false,
        timezone: kestic_core::scheduler::TimezoneSpec::named("Europe/Zurich"),
        monitor_bus: MonitorBusConfig {
            use_session_bus: true,
            enable_watcher: opts.enable_monitors,
            ..Default::default()
        },
        enable_system_monitors: opts.enable_monitors,
        enable_scheduler: opts.enable_scheduler,
        ..Default::default()
    };

    let handle = thread::spawn(move || run_daemon(run_cfg));
    wait_for_file(&ready_file, Duration::from_secs(10));

    let client =
        KesticClient::connect(opts.bus_name, KESTIC_OBJECT_PATH).expect("connect KesticClient");
    let prov = client
        .set_restic_provider(&ResticProvider::Custom {
            path: opts.restic_bin.to_path_buf(),
        })
        .expect("set restic provider");
    assert!(prov.valid, "restic provider must be valid: {prov:?}");
    (state_dir, client, handle)
}

/// Writes an executable restic wrapper: `hold_<subcommand>` marker files block that subcommand
/// (SIGTERM-aware), `fail_<subcommand>` marker files make it fail with the given stderr/exit code.
fn write_round2_wrapper(path: &Path, markers: &Path, real_restic: &Path, fail_stderr: &str) {
    let script = format!(
        "#!/bin/sh\n\
         markers=\"{markers}\"\n\
         sub=\"\"\n\
         for arg in \"$@\"; do\n\
           case \"$arg\" in\n\
             backup|forget|prune|check) sub=\"$arg\"; break ;;\n\
           esac\n\
         done\n\
         if [ -n \"$sub\" ] && [ -f \"$markers/fail_$sub\" ]; then\n\
           printf '%s\\n' '{fail_stderr}' >&2\n\
           exit \"$(cat \"$markers/fail_$sub\")\"\n\
         fi\n\
         if [ -n \"$sub\" ] && [ -f \"$markers/hold_$sub\" ]; then\n\
           trap 'kill \"$pid\" 2>/dev/null; exit 143' TERM INT\n\
           while [ -f \"$markers/hold_$sub\" ]; do\n\
             sleep 0.05 &\n\
             pid=$!\n\
             wait \"$pid\"\n\
           done\n\
         fi\n\
         exec \"{real}\" \"$@\"\n",
        markers = markers.display(),
        real = real_restic.display(),
    );
    fs::create_dir_all(markers).expect("mkdir markers");
    fs::write(path, script).expect("write wrapper");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod wrapper");
}

fn count_runs<F>(state_dir: &Path, mut pred: F) -> usize
where
    F: FnMut(&kestic_core::model::Run) -> bool,
{
    RunHistory::open(state_dir)
        .expect("open history")
        .list_runs()
        .expect("list runs")
        .iter()
        .filter(|r| pred(r))
        .count()
}

fn wait_until<F>(timeout: Duration, label: &str, mut pred: F)
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;
    while !pred() {
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        thread::sleep(Duration::from_millis(40));
    }
}

/// Review issue 1: a scheduled maintenance whose only work is a failing `forget` must enter the
/// 5/15/60 min backoff instead of re-running `restic forget` on every 1 s scheduler tick (F2).
#[test]
fn m3_review2_failed_maintenance_with_open_forget_enters_backoff() {
    run_isolated_test(
        "m3_review2_failed_maintenance_with_open_forget_enters_backoff",
        "KESTIC_M3_REV2_FORGET_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let sub = root.join("forget");
            let markers = sub.join("markers");
            let wrapper = sub.join("restic-forget-wrapper.sh");
            let repo_dir = sub.join("repo");
            let src_dir = sub.join("src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("f.txt"), b"forget data").expect("write src");
            write_round2_wrapper(
                &wrapper,
                &markers,
                &default_restic_latest(),
                "Fatal: forget failed for test",
            );

            // Tuesday 2026-12-01 11:00 CET: no prune/check slot is due (defaults: Sunday 12:00).
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let (state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "forget",
                    bus_name: "net.daester.Kestic.M3Rev2Forget",
                    restic_bin: &wrapper,
                    clock: Arc::clone(&clock),
                    secret_store: None,
                    mount_inspector: None,
                    enable_monitors: false,
                    enable_scheduler: true,
                },
            );

            let dest = Destination::new("ForgetDest", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            client
                .add_destination(&dest, "init", "pw-forget", "")
                .expect("init dest");
            let mut bk =
                Backup::new("ForgetBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            bk.schedule = Schedule::ManualOnly;
            let bid = BackupId::parse(client.add_backup(&bk).expect("add bk")).expect("bid");

            fs::write(markers.join("fail_forget"), b"1").expect("fail forget marker");
            client.start_backup(bid.as_str()).expect("start backup");
            wait_for_state(
                &client,
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

            // FR-05/F4: the open forget alone does not start a maintenance run before the slot.
            thread::sleep(Duration::from_millis(3000));
            assert_eq!(
                count_runs(&state_dir, is_maint),
                0,
                "open forget alone must not start maintenance outside its slot"
            );

            // Sunday 2026-12-06 12:00 CET: maintenance slot; forget and prune both fail.
            fs::write(markers.join("fail_prune"), b"1").expect("fail prune marker");
            clock.set(datetime!(2026-12-06 11:00:05 UTC));
            wait_until(Duration::from_secs(15), "first maintenance attempt", || {
                count_runs(&state_dir, is_finished_maint) == 1
            });
            thread::sleep(Duration::from_millis(3000));
            assert_eq!(
                count_runs(&state_dir, is_maint),
                1,
                "issue 1: failed maintenance must not re-run on every tick"
            );
            let s1 = client.get_state().expect("GetState");
            assert!(
                backup_summary(&s1, &bid).pending_forget,
                "issue 1: forget stays open after the failed attempt"
            );

            // +5 min -> exactly one retry (attempt 2), which fails again and backs off 15 min.
            clock.advance(Duration::from_secs(5 * 60 + 2));
            wait_until(
                Duration::from_secs(15),
                "second maintenance attempt",
                || count_runs(&state_dir, is_finished_maint) == 2,
            );
            thread::sleep(Duration::from_millis(2500));
            assert_eq!(
                count_runs(&state_dir, is_maint),
                2,
                "issue 1: retry after 5 min must happen exactly once"
            );

            // Recovered; +15 min -> attempt 3 succeeds, works off the forget first.
            fs::remove_file(markers.join("fail_forget")).expect("clear forget marker");
            fs::remove_file(markers.join("fail_prune")).expect("clear prune marker");
            clock.advance(Duration::from_secs(15 * 60 + 2));
            let ok_state = wait_for_state(
                &client,
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

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
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

/// Review issue 2 (F9): a scheduled start whose preflight hangs on a black-holed REST destination
/// must not block the daemon main loop, so `PrepareForSleep(true)` is handled (running run
/// interrupted, logind delay inhibitor released) well within logind's 5 s delay window, and a
/// start that completes its preflight after sleep began is still blocked by `sleep_in_progress`.
#[test]
fn m3_review2_hung_preflight_does_not_delay_prepare_for_sleep() {
    run_isolated_test(
        "m3_review2_hung_preflight_does_not_delay_prepare_for_sleep",
        "KESTIC_M3_REV2_SLEEP_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let logind = FakeLogindHandle::start();
            let sub = root.join("sleep");
            let markers = sub.join("markers");
            let wrapper = sub.join("restic-sleep-wrapper.sh");
            let repo_dir = sub.join("repo");
            let src_dir = sub.join("src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("f.txt"), b"sleep data").expect("write src");
            write_round2_wrapper(&wrapper, &markers, &default_restic_latest(), "unused");

            let mut rest = RestServerGuard::start_no_auth(&root.join("rest-data"));
            let rest_port = rest.port;

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:00 UTC)));
            let (state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "sleep",
                    bus_name: "net.daester.Kestic.M3Rev2Sleep",
                    restic_bin: &wrapper,
                    clock: Arc::clone(&clock),
                    secret_store: None,
                    mount_inspector: None,
                    enable_monitors: true,
                    enable_scheduler: true,
                },
            );

            let local_dest =
                Destination::new("SleepLocal", DestinationKind::Local { path: repo_dir })
                    .expect("local dest");
            client
                .add_destination(&local_dest, "init", "pw-local", "")
                .expect("init local");
            let mut local_bk = Backup::new(
                "SleepLocalBk",
                Source::folder(&src_dir),
                local_dest.id.clone(),
            )
            .expect("local bk");
            local_bk.schedule = Schedule::ManualOnly;
            let local_id =
                BackupId::parse(client.add_backup(&local_bk).expect("add local")).expect("id");

            let rest_dest = Destination::new(
                "SleepRest",
                DestinationKind::Rest {
                    url: rest.url.clone(),
                    username: None,
                },
            )
            .expect("rest dest");
            client
                .add_destination(&rest_dest, "init", "pw-rest", "")
                .expect("init rest");
            let mut rest_bk = Backup::new(
                "SleepRestBk",
                Source::folder(&src_dir),
                rest_dest.id.clone(),
            )
            .expect("rest bk");
            rest_bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let rest_id =
                BackupId::parse(client.add_backup(&rest_bk).expect("add rest")).expect("id");

            // Replace the REST server with a black hole (accepts, never answers).
            rest.stop();
            let black_hole = BlackHoleServer::bind(rest_port);
            assert_eq!(black_hole.port, rest_port);

            // Long-running manual backup on the local destination holds the logind delay inhibitor.
            fs::write(markers.join("hold_backup"), b"1").expect("hold marker");
            client.start_backup(local_id.as_str()).expect("start local");
            wait_until(Duration::from_secs(10), "local run + inhibitor", || {
                let s = client.get_state().expect("GetState");
                backup_summary(&s, &local_id).active_run.is_some()
                    && logind.open_inhibitor_count() == 1
            });

            // REST slot becomes due -> scheduled start runs its preflight against the black hole.
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            wait_until(Duration::from_secs(10), "preflight hits black hole", || {
                black_hole.accepted() >= 1
            });
            thread::sleep(Duration::from_millis(300));

            let sleep_started = Instant::now();
            logind.emit_prepare_for_sleep(true);
            let release_deadline = sleep_started + Duration::from_secs(15);
            while logind.open_inhibitor_count() != 0 && Instant::now() < release_deadline {
                thread::sleep(Duration::from_millis(20));
            }
            let release_elapsed = sleep_started.elapsed();
            assert_eq!(
                logind.open_inhibitor_count(),
                0,
                "issue 2: logind delay inhibitor must be released after PrepareForSleep(true)"
            );
            assert!(
                release_elapsed < Duration::from_millis(2500),
                "issue 2: PrepareForSleep(true) handling was delayed by a hung preflight: {release_elapsed:?}"
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
            thread::sleep(Duration::from_millis(1500));
            let s_after = client.get_state().expect("GetState after racing start");
            assert!(
                backup_summary(&s_after, &rest_id).active_run.is_none(),
                "issue 2: scheduled start racing with sleep must be blocked by sleep_in_progress"
            );
            assert_eq!(
                count_runs(&state_dir, |r| r.backup_id() == Some(&rest_id)
                    && matches!(r.status, RunStatus::Running | RunStatus::Ok)),
                0,
                "issue 2: no restic run for the racing scheduled start"
            );

            fs::remove_file(markers.join("hold_backup")).expect("release hold");
            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// Secret store wrapper whose availability probe can be locked and counted.
struct LockableSecretStore {
    inner: FileSecretStore,
    locked: Arc<AtomicBool>,
    availability_checks: Arc<AtomicUsize>,
}

impl SecretStore for LockableSecretStore {
    fn set_secret(
        &self,
        target_id: &str,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError> {
        self.inner.set_secret(target_id, kind, value)
    }

    fn get_secret(&self, target_id: &str, kind: &str) -> Result<Option<SecretValue>, SecretError> {
        self.inner.get_secret(target_id, kind)
    }

    fn delete_secret(&self, target_id: &str, kind: &str) -> Result<(), SecretError> {
        self.inner.delete_secret(target_id, kind)
    }

    fn check_availability(&self) -> Result<(), SecretError> {
        self.availability_checks.fetch_add(1, Ordering::SeqCst);
        if self.locked.load(Ordering::SeqCst) {
            return Err(SecretError::LockedOrPromptRequired(
                "locked for test".to_string(),
            ));
        }
        self.inner.check_availability()
    }
}

/// Review issue 3: while the password store is locked, a due scheduled backup must not re-probe
/// the Secret Service (and `restic version`) on every tick, and must still start once the store
/// is unlocked and the cached probe is retried.
#[test]
fn m3_review2_locked_secret_store_does_not_reprobe_every_tick() {
    run_isolated_test(
        "m3_review2_locked_secret_store_does_not_reprobe_every_tick",
        "KESTIC_M3_REV2_LOCKED_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let sub = root.join("locked");
            let repo_dir = sub.join("repo");
            let src_dir = sub.join("src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("f.txt"), b"locked data").expect("write src");

            let locked = Arc::new(AtomicBool::new(false));
            let checks = Arc::new(AtomicUsize::new(0));
            let store = Arc::new(LockableSecretStore {
                inner: FileSecretStore::new(root.join("secrets.hex")),
                locked: Arc::clone(&locked),
                availability_checks: Arc::clone(&checks),
            });

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:55 UTC)));
            let (_state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "locked",
                    bus_name: "net.daester.Kestic.M3Rev2Locked",
                    restic_bin: &default_restic_latest(),
                    clock: Arc::clone(&clock),
                    secret_store: Some(store),
                    mount_inspector: None,
                    enable_monitors: false,
                    enable_scheduler: true,
                },
            );

            let dest = Destination::new("LockedDest", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            client
                .add_destination(&dest, "init", "pw-locked", "")
                .expect("init dest");
            let mut bk =
                Backup::new("LockedBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let bid = BackupId::parse(client.add_backup(&bk).expect("add bk")).expect("bid");

            locked.store(true, Ordering::SeqCst);
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            wait_for_state(&client, Duration::from_secs(15), "store locked", |s| {
                s.blocking.as_ref().map(|b| b.kind.as_str()) == Some("secret_store_locked")
            });
            let checks_after_first = checks.load(Ordering::SeqCst);
            thread::sleep(Duration::from_millis(3500));
            assert_eq!(
                checks.load(Ordering::SeqCst),
                checks_after_first,
                "issue 3: locked store must not be re-probed on every scheduler tick"
            );
            let s_locked = client.get_state().expect("GetState locked");
            assert!(backup_summary(&s_locked, &bid).last_run.is_none());

            // Store unlocked outside Kestic; the cached probe is retried after the cooldown.
            locked.store(false, Ordering::SeqCst);
            clock.advance(Duration::from_secs(61));
            let ok_state = wait_for_state(
                &client,
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
                "issue 3: blocking state clears once the store is unlocked"
            );

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// Review issue 4 (F2): a manual run that ends with a transient network error must not block
/// the next scheduled run like a permanent error, and `does not exist` is no network keyword.
#[test]
fn m3_review2_manual_transient_error_does_not_block_schedule() {
    let misclassified = classify_exit(
        1,
        Some("Fatal: unable to read exclude file /tmp/x: file does not exist"),
        vec![],
        "",
    );
    assert_eq!(
        misclassified.kind,
        ResticFailureKind::GenericFailure,
        "issue 4: `does not exist` must not be classified as destination unreachable"
    );

    run_isolated_test(
        "m3_review2_manual_transient_error_does_not_block_schedule",
        "KESTIC_M3_REV2_TRANSIENT_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let sub = root.join("transient");
            let markers = sub.join("markers");
            let wrapper = sub.join("restic-transient-wrapper.sh");
            let repo_dir = sub.join("repo");
            let src_dir = sub.join("src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("f.txt"), b"transient data").expect("write src");
            write_round2_wrapper(
                &wrapper,
                &markers,
                &default_restic_latest(),
                "Fatal: connection reset by peer",
            );

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let (_state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "transient",
                    bus_name: "net.daester.Kestic.M3Rev2Transient",
                    restic_bin: &wrapper,
                    clock: Arc::clone(&clock),
                    secret_store: None,
                    mount_inspector: None,
                    enable_monitors: false,
                    enable_scheduler: true,
                },
            );

            let dest = Destination::new("TransDest", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            client
                .add_destination(&dest, "init", "pw-trans", "")
                .expect("init dest");
            let mut bk =
                Backup::new("TransBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let bid = BackupId::parse(client.add_backup(&bk).expect("add bk")).expect("bid");

            fs::write(markers.join("fail_backup"), b"1").expect("fail marker");
            client.start_backup(bid.as_str()).expect("start manual");
            wait_for_state(&client, Duration::from_secs(15), "manual error", |s| {
                backup_summary(s, &bid)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Error)
            });

            fs::remove_file(markers.join("fail_backup")).expect("clear fail marker");
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let ok_state = wait_for_state(
                &client,
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

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// Review issue 5: a user-cancelled scheduled backup or maintenance run counts as handling its
/// slot and must not restart on the next scheduler tick; the next regular slot runs normally.
#[test]
fn m3_review2_user_cancel_does_not_restart_same_slot() {
    run_isolated_test(
        "m3_review2_user_cancel_does_not_restart_same_slot",
        "KESTIC_M3_REV2_CANCEL_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let sub = root.join("cancel");
            let markers = sub.join("markers");
            let wrapper = sub.join("restic-cancel-wrapper.sh");
            let repo_dir = sub.join("repo");
            let maint_repo = sub.join("maint-repo");
            let src_dir = sub.join("src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&maint_repo).expect("mkdir maint repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("f.txt"), b"cancel data").expect("write src");
            write_round2_wrapper(&wrapper, &markers, &default_restic_latest(), "unused");

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:55 UTC)));
            let (state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "cancel",
                    bus_name: "net.daester.Kestic.M3Rev2Cancel",
                    restic_bin: &wrapper,
                    clock: Arc::clone(&clock),
                    secret_store: None,
                    mount_inspector: None,
                    enable_monitors: false,
                    enable_scheduler: true,
                },
            );

            let dest = Destination::new("CancelDest", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            client
                .add_destination(&dest, "init", "pw-cancel", "")
                .expect("init dest");
            let mut maint_dest =
                Destination::new("CancelMaint", DestinationKind::Local { path: maint_repo })
                    .expect("maint dest");
            maint_dest.maintenance.prune = MaintenanceEntrySchedule::Daily {
                hour: 13,
                minute: 0,
            };
            client
                .add_destination(&maint_dest, "init", "pw-maint", "")
                .expect("init maint dest");
            let mut bk =
                Backup::new("CancelBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let bid = BackupId::parse(client.add_backup(&bk).expect("add bk")).expect("bid");
            let is_backup_run = |r: &kestic_core::model::Run| {
                r.backup_id() == Some(&bid) && r.status != RunStatus::Pending
            };
            let is_maint_run = |r: &kestic_core::model::Run| {
                r.kind == RunKind::Maintenance && r.destination_id == maint_dest.id
            };

            // (a) Scheduled backup at 12:00 CET is cancelled by the user.
            fs::write(markers.join("hold_backup"), b"1").expect("hold backup");
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let running = wait_for_state(&client, Duration::from_secs(15), "backup running", |s| {
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
            wait_for_state(&client, Duration::from_secs(15), "backup cancelled", |s| {
                let bs = backup_summary(s, &bid);
                bs.active_run.is_none()
                    && bs
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Cancelled)
            });
            thread::sleep(Duration::from_millis(2500));
            assert_eq!(
                count_runs(&state_dir, is_backup_run),
                1,
                "issue 5: cancelled scheduled backup must not restart on the next tick"
            );
            fs::remove_file(markers.join("hold_backup")).expect("release backup hold");

            // (b) Scheduled maintenance (prune 13:00 CET) is cancelled by the user.
            fs::write(markers.join("hold_prune"), b"1").expect("hold prune");
            clock.set(datetime!(2026-12-01 12:00:05 UTC));
            let maint_running = wait_for_state(
                &client,
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
                &client,
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
            thread::sleep(Duration::from_millis(2500));
            assert_eq!(
                count_runs(&state_dir, is_maint_run),
                1,
                "issue 5: cancelled scheduled maintenance must not restart on the next tick"
            );
            fs::remove_file(markers.join("hold_prune")).expect("release prune hold");

            // (c) Next regular slots run normally.
            clock.set(datetime!(2026-12-02 11:00:05 UTC));
            wait_for_state(&client, Duration::from_secs(20), "next backup slot", |s| {
                backup_summary(s, &bid)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });
            clock.set(datetime!(2026-12-02 12:00:05 UTC));
            wait_for_state(&client, Duration::from_secs(20), "next prune slot", |s| {
                dest_summary(s, &maint_dest.id)
                    .last_maintenance_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// Mount inspector whose next `read_mountinfo` call hangs (simulated hung NFS) once armed.
struct HangingMountInspector {
    armed: AtomicBool,
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
        if self.armed.swap(false, Ordering::SeqCst) {
            self.hanging.store(true, Ordering::SeqCst);
            let mut released = self.release.lock().expect("lock");
            while !*released {
                released = self.release_cv.wait(released).expect("wait");
            }
        }
        Ok(String::new())
    }

    fn fs_uuid_for_device(&self, _source: &str, _mm: &str) -> Option<(String, Option<String>)> {
        None
    }
}

/// Review issue 6: a background source probe stuck on a hung mount must not keep
/// `source_probe_in_flight` forever; after it is stale a new probe runs.
#[test]
fn m3_review2_hung_source_probe_is_replaced_when_stale() {
    run_isolated_test(
        "m3_review2_hung_source_probe_is_replaced_when_stale",
        "KESTIC_M3_REV2_PROBE_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let sub = root.join("probe");
            let repo_dir = sub.join("repo");
            let src_dir = sub.join("drive-src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("f.txt"), b"drive data").expect("write src");

            let dest = Destination::new("ProbeDest", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            let mut bk =
                Backup::new("DriveBk", Source::drive(&src_dir), dest.id.clone()).expect("bk");
            bk.schedule = Schedule::ManualOnly;
            let cfg_path = sub.join("cfg").join("kestic").join("config.toml");
            let mut cfg = kestic_core::config::AppConfig::default();
            cfg.destinations.push(dest);
            cfg.backups.push(bk);
            kestic_core::config::save_config(&cfg_path, &cfg).expect("save config");

            let inspector = Arc::new(HangingMountInspector {
                armed: AtomicBool::new(false),
                hanging: AtomicBool::new(false),
                calls: AtomicUsize::new(0),
                release: Mutex::new(false),
                release_cv: Condvar::new(),
            });
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let (_state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "probe",
                    bus_name: "net.daester.Kestic.M3Rev2Probe",
                    restic_bin: &default_restic_latest(),
                    clock: Arc::clone(&clock),
                    secret_store: None,
                    mount_inspector: Some(Arc::clone(&inspector) as Arc<dyn MountInspector>),
                    enable_monitors: false,
                    enable_scheduler: true,
                },
            );

            inspector.armed.store(true, Ordering::SeqCst);
            wait_until(Duration::from_secs(5), "background probe hangs", || {
                inspector.hanging.load(Ordering::SeqCst)
            });
            let calls_while_hung = inspector.calls.load(Ordering::SeqCst);
            thread::sleep(Duration::from_millis(1500));
            assert_eq!(
                inspector.calls.load(Ordering::SeqCst),
                calls_while_hung,
                "a fresh in-flight probe is not duplicated"
            );

            clock.advance(Duration::from_secs(10 * 60));
            wait_until(Duration::from_secs(5), "stale probe replaced", || {
                inspector.calls.load(Ordering::SeqCst) > calls_while_hung
            });
            assert!(
                inspector.calls.load(Ordering::SeqCst) > calls_while_hung,
                "issue 6: a stale hung source probe must be replaced by a new probe"
            );

            inspector.release_all();
            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// Review issue 7: concurrent queue drains racing for the same destination must not drop a
/// queued backup when its start loses the race (`AlreadyRunning`): every queued backup runs.
#[test]
fn m3_review2_concurrent_drains_keep_queued_backups() {
    run_isolated_test(
        "m3_review2_concurrent_drains_keep_queued_backups",
        "KESTIC_M3_REV2_DRAIN_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let sub = root.join("drain");
            let markers = sub.join("markers");
            let wrapper = sub.join("restic-drain-wrapper.sh");
            let repo_dir = sub.join("repo");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            write_round2_wrapper(&wrapper, &markers, &default_restic_latest(), "unused");

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let bus_name = "net.daester.Kestic.M3Rev2Drain";
            let (_state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "drain",
                    bus_name,
                    restic_bin: &wrapper,
                    clock: Arc::clone(&clock),
                    secret_store: None,
                    mount_inspector: None,
                    enable_monitors: false,
                    enable_scheduler: true,
                },
            );
            let dest = Destination::new("DrainDest", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            client
                .add_destination(&dest, "init", "pw-drain", "")
                .expect("init dest");

            let mut ids = Vec::new();
            for idx in 0..8 {
                let src = sub.join(format!("src{idx}"));
                fs::create_dir_all(&src).expect("mkdir src");
                fs::write(src.join("f.txt"), format!("payload {idx}")).expect("write src");
                let mut bk = Backup::new(
                    format!("DrainBk{idx}"),
                    Source::folder(&src),
                    dest.id.clone(),
                )
                .expect("bk");
                bk.schedule = Schedule::ManualOnly;
                ids.push(BackupId::parse(client.add_backup(&bk).expect("add")).expect("id"));
            }

            fs::write(markers.join("hold_backup"), b"1").expect("hold");
            let res = client.start_all_backups().expect("StartAllBackups");
            assert_eq!(res.started_run_ids.len(), 1, "{res:?}");
            assert_eq!(res.queued_backup_ids.len(), 7, "{res:?}");

            // Hammer extra queue drains (ResumeGlobal drains the queue) while runs hand over.
            let stop = Arc::new(AtomicBool::new(false));
            let mut hammers = Vec::new();
            for _ in 0..12 {
                let stop_h = Arc::clone(&stop);
                hammers.push(thread::spawn(move || {
                    let c =
                        KesticClient::connect(bus_name, KESTIC_OBJECT_PATH).expect("hammer client");
                    while !stop_h.load(Ordering::SeqCst) {
                        c.resume_global().expect("ResumeGlobal");
                    }
                }));
            }
            fs::remove_file(markers.join("hold_backup")).expect("release hold");

            let deadline = Instant::now() + Duration::from_secs(45);
            let mut all_ok = false;
            while Instant::now() < deadline {
                let s = client.get_state().expect("GetState");
                all_ok = ids.iter().all(|id| {
                    backup_summary(&s, id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });
                if all_ok {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            stop.store(true, Ordering::SeqCst);
            for h in hammers {
                h.join().expect("join hammer");
            }
            assert!(
                all_ok,
                "issue 7: every queued backup must eventually run (none dropped by a drain race)"
            );

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

// ---------------------------------------------------------------------------------------------
// Third review round: residual lock issues (A) and locale-independent DTO codes (B).
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

/// Review round 3, A1: building the state DTO (GetState, tray) must not read Warning-run log
/// files under the `DaemonCore` lock; unreadable files are cached when the run finishes.
#[test]
fn m3_review3_state_building_does_not_read_run_logs() {
    run_isolated_test(
        "m3_review3_state_building_does_not_read_run_logs",
        "KESTIC_M3_REV3_STATELOG_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let (bid, _run, fifo) = seed_run_with_fifo_log(root, "statelog", RunStatus::Warning);
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let bus_name = "net.daester.Kestic.M3Rev3StateLog";
            let (_state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "statelog",
                    bus_name,
                    restic_bin: &default_restic_latest(),
                    clock,
                    secret_store: None,
                    mount_inspector: None,
                    enable_monitors: false,
                    enable_scheduler: false,
                },
            );

            let answered = get_state_answers_within(bus_name, Duration::from_secs(5));
            if !answered {
                // Unblock a daemon stuck reading the pipe so the test process can finish.
                release_fifo(&fifo);
            }
            assert!(
                answered,
                "A1: GetState must not block on a Warning run's log file"
            );
            let state = client.get_state().expect("GetState");
            assert_eq!(
                backup_summary(&state, &bid)
                    .last_run
                    .as_ref()
                    .map(|r| r.status),
                Some(RunStatus::Warning)
            );

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// Review round 3, A2: `GetRunDetail` reads the run log (and resolves restic) without holding
/// the `DaemonCore` lock, so a slow log read never blocks other D-Bus calls.
#[test]
fn m3_review3_run_detail_log_read_does_not_hold_lock() {
    run_isolated_test(
        "m3_review3_run_detail_log_read_does_not_hold_lock",
        "KESTIC_M3_REV3_DETAIL_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let (_bid, run, fifo) = seed_run_with_fifo_log(root, "detail", RunStatus::Ok);
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let bus_name = "net.daester.Kestic.M3Rev3Detail";
            let (_state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "detail",
                    bus_name,
                    restic_bin: &default_restic_latest(),
                    clock,
                    secret_store: None,
                    mount_inspector: None,
                    enable_monitors: false,
                    enable_scheduler: false,
                },
            );

            let run_id = run.id.as_str().to_string();
            let bus = bus_name.to_string();
            let detail_thread = thread::spawn(move || {
                let c = KesticClient::connect(&bus, KESTIC_OBJECT_PATH).expect("connect");
                c.get_run_detail(&run_id)
            });
            thread::sleep(Duration::from_millis(500));
            let answered = get_state_answers_within(bus_name, Duration::from_secs(3));
            release_fifo(&fifo);
            assert!(
                answered,
                "A2: GetState must answer while GetRunDetail waits for a slow log read"
            );
            let detail = detail_thread
                .join()
                .expect("join detail")
                .expect("GetRunDetail");
            assert_eq!(detail.run.id, run.id);
            assert_eq!(detail.run.status, RunStatus::Ok);

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// Mount inspector whose `read_mountinfo` blocks while `blocking` is set (hung NFS mount).
struct BlockingMountInspector {
    blocking: AtomicBool,
    calls: AtomicUsize,
}

impl MountInspector for BlockingMountInspector {
    fn read_mountinfo(&self) -> std::io::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        while self.blocking.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(20));
        }
        Ok(String::new())
    }

    fn fs_uuid_for_device(&self, _source: &str, _mm: &str) -> Option<(String, Option<String>)> {
        None
    }
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
    let dest =
        Destination::new("HungDest", DestinationKind::Local { path: repo_dir }).expect("dest");
    let restic = default_restic_latest();
    let resolved = check_binary_version(&restic).expect("resolved");
    let store = FileSecretStore::new(root.join("secrets.hex"));
    let kbin = find_kestic_binary();
    let seed_clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let cache = sub.join("restic-cache");
    let rt = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &kbin,
        secret_store: &store,
        clock: &seed_clock,
        cache_dir: Some(&cache),
    };
    kestic_core::destination::setup_destination(
        &dest,
        kestic_core::destination::DestinationSetupMode::InitNew,
        &SecretValue::from_str_secret("pw-hung"),
        None,
        &rt,
    )
    .expect("init repo");

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

/// Review round 3, A3: a source check hanging on a hung mount during a scheduled start is
/// bounded: the backup becomes `ausstehend` (source unavailable) and the destination claim is
/// released, so other backups on that destination can run.
#[test]
fn m3_review3_hung_source_stat_releases_destination_claim() {
    run_isolated_test(
        "m3_review3_hung_source_stat_releases_destination_claim",
        "KESTIC_M3_REV3_HUNGSTAT_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let (drive_id, folder_id) = seed_drive_and_folder_backups(root, "hungstat");
            let inspector = Arc::new(BlockingMountInspector {
                blocking: AtomicBool::new(false),
                calls: AtomicUsize::new(0),
            });
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:55 UTC)));
            let (_state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "hungstat",
                    bus_name: "net.daester.Kestic.M3Rev3HungStat",
                    restic_bin: &default_restic_latest(),
                    clock: Arc::clone(&clock),
                    secret_store: None,
                    mount_inspector: Some(Arc::clone(&inspector) as Arc<dyn MountInspector>),
                    enable_monitors: false,
                    enable_scheduler: true,
                },
            );

            inspector.blocking.store(true, Ordering::SeqCst);
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let pending = wait_for_state(
                &client,
                Duration::from_secs(25),
                "hung source -> pending source unavailable",
                |s| {
                    backup_summary(s, &drive_id).pending_primary_code.as_deref()
                        == Some("source_unavailable")
                },
            );
            assert!(backup_summary(&pending, &drive_id).active_run.is_none());

            let started = client.start_backup(folder_id.as_str());
            inspector.blocking.store(false, Ordering::SeqCst);
            assert!(
                started.is_ok(),
                "A3: destination claim must be released after the bounded source check: {started:?}"
            );
            wait_for_state(&client, Duration::from_secs(20), "folder backup ok", |s| {
                backup_summary(s, &folder_id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// Review round 3, A4: a background availability probe stuck on a hung check must not keep
/// `availability_probe_in_flight` forever; once stale, the next trigger starts a new probe.
#[test]
fn m3_review3_hung_availability_probe_is_replaced_when_stale() {
    run_isolated_test(
        "m3_review3_hung_availability_probe_is_replaced_when_stale",
        "KESTIC_M3_REV3_AVAILPROBE_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let (_drive_id, _folder_id) = seed_drive_and_folder_backups(root, "availprobe");
            let inspector = Arc::new(BlockingMountInspector {
                blocking: AtomicBool::new(false),
                calls: AtomicUsize::new(0),
            });
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let (_state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "availprobe",
                    bus_name: "net.daester.Kestic.M3Rev3AvailProbe",
                    restic_bin: &default_restic_latest(),
                    clock: Arc::clone(&clock),
                    secret_store: None,
                    mount_inspector: Some(Arc::clone(&inspector) as Arc<dyn MountInspector>),
                    enable_monitors: false,
                    enable_scheduler: false,
                },
            );

            inspector.blocking.store(true, Ordering::SeqCst);
            clock.advance(Duration::from_secs(61));
            let before_first = inspector.calls.load(Ordering::SeqCst);
            client
                .window_active(true, "overview")
                .expect("WindowActive");
            wait_until(Duration::from_secs(5), "first probe hangs", || {
                inspector.calls.load(Ordering::SeqCst) > before_first
            });
            let calls_hung = inspector.calls.load(Ordering::SeqCst);

            clock.advance(Duration::from_secs(10 * 60));
            client
                .window_active(true, "overview")
                .expect("WindowActive");
            wait_until(Duration::from_secs(3), "stale probe replaced", || {
                inspector.calls.load(Ordering::SeqCst) > calls_hung
            });
            assert!(
                inspector.calls.load(Ordering::SeqCst) > calls_hung,
                "A4: a stale hung availability probe must not block new probes"
            );

            inspector.blocking.store(false, Ordering::SeqCst);
            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// Review round 3, B: state DTOs carry locale-independent codes next to the daemon-locale
/// texts: `error_kind` + `error_detail` + `error_exit_code`, `pending_reason_codes` +
/// `pending_retry_at_unix` (backoff), `availability.source_status`, and
/// `SkippedBackupDto::error_kind`.
#[test]
fn m3_review3_state_dtos_carry_stable_codes() {
    run_isolated_test(
        "m3_review3_state_dtos_carry_stable_codes",
        "KESTIC_M3_REV3_CODES_INNER",
        |root| {
            let _sni_conn = start_fake_sni_watcher();
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let sub = root.join("codes");
            let markers = sub.join("markers");
            let wrapper = sub.join("restic-codes-wrapper.sh");
            let repo_dir = sub.join("repo");
            let lock_repo = sub.join("lock-repo");
            let src_dir = sub.join("src");
            for d in [&repo_dir, &lock_repo, &src_dir] {
                fs::create_dir_all(d).expect("mkdir");
            }
            fs::write(src_dir.join("f.txt"), b"codes").expect("write src");
            write_round2_wrapper(
                &wrapper,
                &markers,
                &default_restic_latest(),
                "Fatal: wrong password or no key found",
            );

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:55 UTC)));
            let (_state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "codes",
                    bus_name: "net.daester.Kestic.M3Rev3Codes",
                    restic_bin: &wrapper,
                    clock: Arc::clone(&clock),
                    secret_store: None,
                    mount_inspector: None,
                    enable_monitors: false,
                    enable_scheduler: true,
                },
            );
            let dest = Destination::new("CodesDest", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            client
                .add_destination(&dest, "init", "pw-codes", "")
                .expect("init dest");
            let lock_dest =
                Destination::new("LockDest", DestinationKind::Local { path: lock_repo })
                    .expect("lock dest");
            client
                .add_destination(&lock_dest, "init", "pw-lock", "")
                .expect("init lock dest");

            let mut pw_bk =
                Backup::new("PwBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            pw_bk.schedule = Schedule::ManualOnly;
            let pw_id = BackupId::parse(client.add_backup(&pw_bk).expect("add")).expect("id");
            let mut missing_bk = Backup::new(
                "MissingSrcBk",
                Source::folder(sub.join("no-such-dir")),
                lock_dest.id.clone(),
            )
            .expect("bk");
            missing_bk.schedule = Schedule::ManualOnly;
            let missing_id =
                BackupId::parse(client.add_backup(&missing_bk).expect("add")).expect("id");

            // (a) Exit 12 -> error_kind `wrong_password`, raw text as detail, exit code 12.
            fs::write(markers.join("fail_backup"), b"12").expect("fail marker");
            client.start_backup(pw_id.as_str()).expect("start");
            let s_err = wait_for_state(&client, Duration::from_secs(15), "pw error", |s| {
                backup_summary(s, &pw_id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Error)
            });
            let pw = backup_summary(&s_err, &pw_id);
            assert_eq!(pw.error_kind.as_deref(), Some("wrong_password"));
            assert_eq!(pw.error_exit_code, Some(12));
            assert!(
                pw.error_detail
                    .as_deref()
                    .is_some_and(|d| d.contains("wrong password or no key found")),
                "error_detail carries the raw restic text: {:?}",
                pw.error_detail
            );

            // (b) Missing source -> availability.source_status `path_missing`.
            let s_avail = wait_for_state(&client, Duration::from_secs(5), "source probe", |s| {
                !backup_summary(s, &missing_id).availability.source_available
            });
            assert_eq!(
                backup_summary(&s_avail, &missing_id)
                    .availability
                    .source_status
                    .as_deref(),
                Some("path_missing")
            );

            // (c) StartAll skips the wrong-password backup with error_kind `wrong_password`.
            let res = client.start_all_backups().expect("StartAllBackups");
            let skipped_pw = res
                .skipped
                .iter()
                .find(|sk| sk.backup_id == pw_id.as_str())
                .expect("pw backup skipped");
            assert_eq!(skipped_pw.reason, "permanent_error");
            assert_eq!(skipped_pw.error_kind.as_deref(), Some("wrong_password"));

            // (d) Scheduled Exit 11 -> pending backoff with codes and retry time (+5 min).
            fs::write(markers.join("fail_backup"), b"11").expect("lock marker");
            let mut lock_bk =
                Backup::new("LockBk", Source::folder(&src_dir), lock_dest.id.clone()).expect("bk");
            lock_bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let lock_id = BackupId::parse(client.add_backup(&lock_bk).expect("add")).expect("id");
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let s_backoff = wait_for_state(&client, Duration::from_secs(15), "lock backoff", |s| {
                backup_summary(s, &lock_id).pending_primary_code.as_deref() == Some("backoff")
            });
            let lb = backup_summary(&s_backoff, &lock_id);
            assert_eq!(lb.pending_reason_codes, vec!["backoff".to_string()]);
            assert_eq!(lb.pending_reasons.len(), lb.pending_reason_codes.len());
            assert_eq!(
                lb.pending_retry_at_unix,
                Some(datetime!(2026-12-01 11:05:05 UTC).unix_timestamp())
            );

            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

/// Review round 3, C3: a running maintenance shows maintenance wording («Maintaining ‹Dest›»)
/// in the overall headline and the tray menu instead of «‹Dest›: backing up · 0%».
#[test]
fn m3_review3_running_maintenance_shows_maintenance_wording() {
    run_isolated_test(
        "m3_review3_running_maintenance_shows_maintenance_wording",
        "KESTIC_M3_REV3_MAINTTEXT_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let sni_conn = start_fake_sni_watcher();
            let sub = root.join("mainttext");
            let markers = sub.join("markers");
            let wrapper = sub.join("restic-mainttext-wrapper.sh");
            let repo_dir = sub.join("repo");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            write_round2_wrapper(&wrapper, &markers, &default_restic_latest(), "unused");

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 11:59:55 UTC)));
            let (_state_dir, client, handle) = spawn_round2_daemon(
                root,
                Round2DaemonOpts {
                    sub_name: "mainttext",
                    bus_name: "net.daester.Kestic.M3Rev3MaintText",
                    restic_bin: &wrapper,
                    clock: Arc::clone(&clock),
                    secret_store: None,
                    mount_inspector: None,
                    enable_monitors: false,
                    enable_scheduler: true,
                },
            );
            let sni_service = wait_for_sni_service(&sni_conn, Duration::from_secs(5));
            let mut dest = Destination::new("MaintZiel", DestinationKind::Local { path: repo_dir })
                .expect("dest");
            dest.maintenance.prune = MaintenanceEntrySchedule::Daily {
                hour: 13,
                minute: 0,
            };
            client
                .add_destination(&dest, "init", "pw-mt", "")
                .expect("init dest");

            fs::write(markers.join("hold_prune"), b"1").expect("hold prune");
            clock.set(datetime!(2026-12-01 12:00:05 UTC));
            let running = wait_for_state(&client, Duration::from_secs(15), "maintenance", |s| {
                dest_summary(s, &dest.id).maintenance_running
            });
            assert_eq!(
                running.overall.headline, "Maintaining MaintZiel",
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

            fs::remove_file(markers.join("hold_prune")).expect("release prune");
            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}
