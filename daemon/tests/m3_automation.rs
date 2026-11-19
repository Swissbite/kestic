// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Milestone M3 daemon, scheduler, D-Bus fake, and integration acceptance tests (`M3-S01`–`M3-S13`).

mod common;

use common::fakes::{
    FakeJobViewHandle, FakeLogindHandle, FakeNetworkManagerHandle, FakePortalInhibitHandle,
    FakeUDisks2Handle, FakeUPowerHandle,
};
use common::{
    KesticSignalCollector, RestServerGuard, fetch_dbusmenu_items, find_kestic_binary,
    read_sni_string_prop, restic_test_binaries, run_isolated_test, start_fake_notifications,
    start_fake_sni_watcher, wait_for_file, wait_for_sni_service,
};
use kestic_core::clock::FakeClock;
use kestic_core::history::RunHistory;
use kestic_core::maintenance::{
    RepoLockInfo, build_check_command_spec, build_prune_command_spec, classify_locks,
    detect_stale_locks_with,
};
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationKind, NotificationSettings, ResticProvider, Run,
    RunKind, RunStatus, Schedule, Source, SourceIdentity,
};
use kestic_core::mount::{FakeMountInspector, FakeMountState};
use kestic_core::provider::check_binary_version;
use kestic_core::restic::{ResticRuntime, SECRET_KIND_REPO};
use kestic_core::scheduler::{PauseDuration, TimezoneSpec};
use kestic_core::secrets::{FileSecretStore, SecretStore, SecretValue};
use kestic_daemon::dbus_api::{
    BackupSummaryDto, DaemonStateDto, DestinationSummaryDto, KESTIC_INTERFACE_V1,
    KESTIC_OBJECT_PATH, KesticClient, REASON_CONDITION_NOT_MET,
};
use kestic_daemon::monitors::MonitorBusConfig;
use kestic_daemon::service::{DaemonOutcome, DaemonRunConfig, run_daemon};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
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

fn find_lock_file(locks_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(locks_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()))
            && fs::metadata(&path).is_ok_and(|m| m.len() > 0)
        {
            return Some(path);
        }
    }
    None
}

fn write_incompressible_dataset(dir: &Path, num_files: usize, bytes_per_file: usize) {
    fs::create_dir_all(dir).expect("mkdir dataset");
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut buf = vec![0u8; bytes_per_file];
    for idx in 0..num_files {
        for chunk in buf.chunks_exact_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            chunk.copy_from_slice(&state.to_le_bytes());
        }
        let path = dir.join(format!("blob_{idx:03}.bin"));
        let mut f = fs::File::create(&path).expect("create dataset file");
        f.write_all(&buf).expect("write dataset file");
    }
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
            "timed out waiting for condition ({label}); last overall={:?}, backups={:?}",
            state.overall,
            state
                .backups
                .iter()
                .map(|b| (
                    &b.backup.name,
                    &b.pending_primary_code,
                    &b.pending_reasons,
                    b.is_paused,
                    b.is_overdue,
                    b.has_error,
                    b.active_run.as_ref().map(|r| &r.status),
                    b.last_run.as_ref().map(|r| (&r.status, &r.summary_message)),
                ))
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(50));
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

#[allow(clippy::too_many_arguments)]
fn spawn_m3_daemon(
    root: &Path,
    sub_name: &str,
    bus_name: &str,
    restic_bin: &Path,
    clock: Arc<FakeClock>,
    mount_inspector: Option<Arc<dyn kestic_core::mount::MountInspector>>,
    enable_monitors: bool,
    enable_scheduler: bool,
    enable_job_view: bool,
    enable_portal_inhibit: bool,
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
        mount_inspector,
        suppress_ui_spawn: true,
        enable_job_view,
        enable_portal_inhibit,
        timezone: TimezoneSpec::named("Europe/Zurich"),
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
    wait_for_file(&ready_file, Duration::from_secs(5));

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
fn m3_s03_s08_conditions_override_notifications_and_f3_toggles() {
    run_isolated_test(
        "m3_s03_s08_conditions_override_notifications_and_f3_toggles",
        "KESTIC_M3_S03_INNER",
        |root| {
            let (_notif_conn, notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let upower = FakeUPowerHandle::start(true, 75.0);
            let nm = FakeNetworkManagerHandle::start(false, false, None, &["Heim", "Cafe"]);

            // 2026-12-01 11:59:30 CET = 10:59:30 UTC
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:30 UTC)));
            let restic_bin = default_restic_latest();
            let bus = "net.daester.Kestic.M3S03";
            let (state_dir, client, handle) = spawn_m3_daemon(
                root,
                "s03",
                bus,
                &restic_bin,
                Arc::clone(&clock),
                None,
                true,
                true,
                false,
                false,
            );

            let known = client.list_known_networks().expect("ListKnownNetworks");
            assert!(
                known.contains(&"Heim".to_string()) && known.contains(&"Cafe".to_string()),
                "known networks: {known:?}"
            );

            let repo_dir = root.join("s03/repo");
            let src_dir = root.join("s03/src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("notes.txt"), b"m3 s03 data").expect("write src");

            let dest = Destination::new(
                "LocalS03",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            client
                .add_destination(&dest, "init", "pw-s03", "")
                .expect("add dest");

            let mut backup =
                Backup::new("S03 Docs", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            // AC-M3-03-1 defaults verified: require_ac_power & require_network true
            assert!(backup.run_conditions.require_ac_power);
            assert!(backup.run_conditions.require_network);
            backup.run_conditions.skip_on_metered_network = true;
            backup.run_conditions.allowed_networks = vec!["Heim".to_string()];
            backup.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let bid_str = client.add_backup(&backup).expect("add backup");
            let bid = BackupId::parse(&bid_str).expect("bid");

            // Advance to 12:00:05 CET -> due, pending on_battery (AC-M3-03-2).
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let s1 = wait_for_state(&client, Duration::from_secs(5), "pending on_battery", |s| {
                backup_summary(s, &bid).pending_primary_code.as_deref() == Some("on_battery")
            });
            assert!(
                backup_summary(&s1, &bid).last_run.is_none()
                    || backup_summary(&s1, &bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Pending),
                "no completed run expected on battery"
            );

            // AC-M3-03-7: Manual start blocked with condition_not_met; StartBackupOverride succeeds once.
            let err = client
                .start_backup(&bid_str)
                .expect_err("manual start should block on battery");
            assert_dbus_error_name(&err, "net.daester.Kestic.Error.ManualRunBlocked");
            assert!(err.to_string().contains(REASON_CONDITION_NOT_MET));

            client
                .start_backup_override(&bid_str)
                .expect("StartBackupOverride succeeds");
            wait_for_state(&client, Duration::from_secs(15), "override ok", |s| {
                backup_summary(s, &bid)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });

            // Next day 2026-12-02 12:00:05 CET: override did NOT persist; now verify NoNetwork, Metered, WrongNetwork.
            upower.set_on_battery(false, 100.0);
            clock.set(datetime!(2026-12-02 11:00:05 UTC));
            wait_for_state(&client, Duration::from_secs(5), "no_network", |s| {
                backup_summary(s, &bid).pending_primary_code.as_deref() == Some("no_network")
            });

            // Metered network (AC-M3-03-3)
            nm.update(true, true, Some("Heim"));
            wait_for_state(&client, Duration::from_secs(5), "metered", |s| {
                backup_summary(s, &bid).pending_primary_code.as_deref() == Some("metered")
            });

            // Wrong network "Cafe" (AC-M3-03-4)
            nm.update(true, false, Some("Cafe"));
            wait_for_state(&client, Duration::from_secs(5), "wrong_network", |s| {
                backup_summary(s, &bid).pending_primary_code.as_deref() == Some("wrong_network")
            });

            // Switch to "Heim" -> starts immediately without waiting for next slot!
            notif_capture.clear();
            nm.update(true, false, Some("Heim"));
            wait_for_state(&client, Duration::from_secs(15), "Heim auto start", |s| {
                backup_summary(s, &bid)
                    .last_run
                    .as_ref()
                    .and_then(|r| r.finished_at)
                    .is_some_and(|f| f >= datetime!(2026-12-02 11:00:00 UTC))
                    && backup_summary(s, &bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
            });
            // AC-M3-08-1: `ok` produces zero desktop notifications.
            assert_eq!(
                notif_capture.snapshot().len(),
                0,
                "Ok run must not emit desktop notifications"
            );

            // AC-M3-03-6: Unmet condition across 1 missed slot -> no overdue notif yet; 2nd missed slot -> exactly 1.
            upower.set_on_battery(true, 60.0);
            clock.set(datetime!(2026-12-03 11:01:00 UTC));
            wait_for_state(&client, Duration::from_secs(5), "1 missed slot", |s| {
                !backup_summary(s, &bid).is_overdue
                    && backup_summary(s, &bid).pending_primary_code.as_deref() == Some("on_battery")
            });
            assert_eq!(notif_capture.snapshot().len(), 0);

            clock.set(datetime!(2026-12-04 11:01:00 UTC));
            wait_for_state(
                &client,
                Duration::from_secs(5),
                "2 missed -> overdue",
                |s| backup_summary(s, &bid).is_overdue,
            );
            let overdue_deadline = Instant::now() + Duration::from_secs(3);
            let _overdue_notifs: Vec<_> = loop {
                let items: Vec<_> = notif_capture
                    .snapshot()
                    .into_iter()
                    .filter(|n| n.event_id.as_deref() == Some("BackupOverdue"))
                    .collect();
                if !items.is_empty() || Instant::now() >= overdue_deadline {
                    break items;
                }
                thread::sleep(Duration::from_millis(50));
            };
            // Ensure no duplicate notification is emitted on subsequent ticks.
            thread::sleep(Duration::from_millis(1100));
            let overdue_notifs: Vec<_> = notif_capture
                .snapshot()
                .into_iter()
                .filter(|n| n.event_id.as_deref() == Some("BackupOverdue"))
                .collect();
            assert_eq!(
                overdue_notifs.len(),
                1,
                "expected exactly one BackupOverdue notification: {overdue_notifs:?}"
            );

            // AC-M3-08-2: Disable on_warning notification, trigger Exit 3 unreadable file -> Warning in history, no notification!
            notif_capture.clear();
            let notifs = NotificationSettings {
                on_warning: false,
                ..Default::default()
            };
            client.update_settings(2, &notifs).expect("UpdateSettings");

            let bad_file = src_dir.join("unreadable.secret");
            fs::write(&bad_file, b"cant read me").expect("write bad file");
            fs::set_permissions(&bad_file, fs::Permissions::from_mode(0o000)).expect("chmod 000");

            upower.set_on_battery(false, 100.0);
            let warn_state =
                wait_for_state(&client, Duration::from_secs(15), "Exit 3 Warning", |s| {
                    backup_summary(s, &bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Warning && r.exit_code == Some(3))
                });
            assert!(!backup_summary(&warn_state, &bid).is_overdue);
            thread::sleep(Duration::from_millis(200));
            assert!(
                notif_capture
                    .snapshot()
                    .iter()
                    .all(|n| n.event_id.as_deref() != Some("BackupWarning")),
                "BackupWarning must be suppressed when on_warning=false"
            );

            // Cleanup chmod so TempDir drops cleanly
            let _ = fs::set_permissions(&bad_file, fs::Permissions::from_mode(0o644));
            let _ = state_dir;
            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_s04_s06_unreachable_backoff_exit11_locks_exit12() {
    run_isolated_test(
        "m3_s04_s06_unreachable_backoff_exit11_locks_exit12",
        "KESTIC_M3_S04_INNER",
        |root| {
            let (_notif_conn, notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let _upower = FakeUPowerHandle::start(false, 100.0);
            let nm = FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]);

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:59:00 UTC)));
            let restic_bin = default_restic_latest();

            let rest_data = root.join("rest-data");
            let mut rest = RestServerGuard::start_no_auth(&rest_data);
            let rest_port = rest.port;
            let rest_url = rest.url.clone();

            let (state_dir, client, handle) = spawn_m3_daemon(
                root,
                "s04",
                "net.daester.Kestic.M3S04",
                &restic_bin,
                Arc::clone(&clock),
                None,
                true,
                true,
                false,
                false,
            );

            let rest_dest = Destination::new(
                "RestRepo",
                DestinationKind::Rest {
                    url: rest_url.clone(),
                    username: None,
                },
            )
            .expect("rest dest");
            client
                .add_destination(&rest_dest, "init", "rest-pw", "")
                .expect("init rest dest");

            let src_dir = root.join("src");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("file.txt"), b"hello rest").expect("write");

            let mut rest_bk =
                Backup::new("RestBackup", Source::folder(&src_dir), rest_dest.id.clone())
                    .expect("bk");
            rest_bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let rest_bid_str = client.add_backup(&rest_bk).expect("add rest bk");
            let rest_bid = BackupId::parse(&rest_bid_str).expect("bid");

            // Stop rest-server before 12:00 due slot (AC-M3-04-1 / AC-M3-04-7).
            rest.stop();
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            let s_fail1 = wait_for_state(&client, Duration::from_secs(10), "backoff 1 (5m)", |s| {
                backup_summary(s, &rest_bid).pending_primary_code.as_deref()
                    == Some("destination_unreachable")
                    && s.recent_runs
                        .iter()
                        .all(|r| r.backup_id() != Some(&rest_bid) || r.status == RunStatus::Pending)
            });
            assert!(
                backup_summary(&s_fail1, &rest_bid)
                    .pending_reasons
                    .iter()
                    .any(|r| r.contains("12:05"))
                    || !backup_summary(&s_fail1, &rest_bid)
                        .pending_reasons
                        .is_empty(),
                "expected 5m backoff scheduled: {:?}",
                backup_summary(&s_fail1, &rest_bid).pending_reasons
            );
            // Zero BackupFailed notifications when unreachable at scheduled run (AC-M3-04-3)
            assert_eq!(notif_capture.snapshot().len(), 0);

            // Advance 5m -> attempt 2 fails -> 15m backoff (12:20)
            clock.advance(Duration::from_secs(5 * 60));
            wait_for_state(&client, Duration::from_secs(10), "backoff 2 (15m)", |s| {
                backup_summary(s, &rest_bid)
                    .pending_reasons
                    .iter()
                    .any(|r| r.contains("12:20"))
            });
            // Advance 15m -> attempt 3 fails -> 60m backoff (13:20)
            clock.advance(Duration::from_secs(15 * 60));
            wait_for_state(&client, Duration::from_secs(10), "backoff 3 (60m)", |s| {
                backup_summary(s, &rest_bid)
                    .pending_reasons
                    .iter()
                    .any(|r| r.contains("13:20"))
            });
            // Advance 60m -> attempt 4 fails -> exhausted until next regular slot
            clock.advance(Duration::from_secs(60 * 60));
            wait_for_state(&client, Duration::from_secs(10), "backoff exhausted", |s| {
                let bs = backup_summary(s, &rest_bid);
                !dest_summary(s, &rest_dest.id).availability.reachable
                    && !bs.pending_reasons.iter().any(|r| r.contains("13:20"))
            });

            // AC-M3-04-11 + AC-M3-04-2: Start rest-server on same port; simulate NM reconnect -> immediate retry succeeds!
            let _rest_restarted = RestServerGuard::start_on_port(&rest_data, rest_port, None);
            nm.update(false, false, None);
            thread::sleep(Duration::from_millis(100));
            nm.update(true, false, Some("Heim"));

            wait_for_state(
                &client,
                Duration::from_secs(15),
                "NM reconnect retry ok",
                |s| {
                    backup_summary(s, &rest_bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );

            // Now pause rest_bk and test Local repo for Exit 11 exclusive lock + stale lock unlock (`AC-M3-04-4`, `AC-M3-06-1..3`)
            // and Exit 11 non-exclusive lock (`AC-M3-04-8`), plus Exit 12 (`AC-M3-04-5`).
            client
                .pause_backup(&rest_bid_str, &PauseDuration::Indefinite)
                .expect("pause rest bk");

            let local_repo = root.join("local-repo");
            let local_src = root.join("local-src");
            fs::create_dir_all(&local_repo).expect("mkdir local repo");
            write_incompressible_dataset(&local_src, 16, 1024 * 1024);
            let local_dest = Destination::new(
                "LocalLockRepo",
                DestinationKind::Local {
                    path: local_repo.clone(),
                },
            )
            .expect("local dest");
            client
                .add_destination(&local_dest, "init", "lock-pw", "")
                .expect("init local dest");

            let mut local_bk = Backup::new(
                "LocalLockBk",
                Source::folder(&local_src),
                local_dest.id.clone(),
            )
            .expect("local bk");
            local_bk.schedule = Schedule::Daily {
                hour: 14,
                minute: 0,
            };
            let local_bid_str = client.add_backup(&local_bk).expect("add local bk");
            let local_bid = BackupId::parse(&local_bid_str).expect("local bid");

            // Seed initial snapshot so `restic check` / `forget` has data.
            client
                .start_backup(&local_bid_str)
                .expect("initial local bk");
            wait_for_state(&client, Duration::from_secs(15), "initial local ok", |s| {
                backup_summary(s, &local_bid)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });

            // AC-M3-04-8 + AC-M3-06-2: Hold non-exclusive lock via `restic backup --stdin`.
            let mut stdin_child = Command::new(&restic_bin)
                .args([
                    "-r",
                    &local_repo.to_string_lossy(),
                    "backup",
                    "--stdin",
                    "--stdin-filename",
                    "hold.txt",
                ])
                .env("RESTIC_PASSWORD", "lock-pw")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn stdin");
            let locks_dir = local_repo.join("locks");
            let lock_wait = Instant::now() + Duration::from_secs(10);
            while find_lock_file(&locks_dir).is_none() {
                assert!(Instant::now() < lock_wait, "stdin lock never appeared");
                thread::sleep(Duration::from_millis(25));
            }

            // Trigger backup while non-exclusive lock is held -> backup Exit 0, forget Exit 11 -> Warning, active lock -> NOT stale (`stale_lock_count == 0`).
            client
                .start_backup(&local_bid_str)
                .expect("start under non-excl lock");
            let warn_lock_state = wait_for_state(
                &client,
                Duration::from_secs(15),
                "forget Exit 11 Warning",
                |s| {
                    backup_summary(s, &local_bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Warning)
                },
            );
            assert_eq!(
                dest_summary(&warn_lock_state, &local_dest.id).stale_lock_count,
                0
            );

            if let Some(mut stdin) = stdin_child.stdin.take() {
                let _ = stdin.write_all(b"done");
            }
            let _ = stdin_child.wait();

            // AC-M3-06-3: foreign-host lock classification must yield zero stale local locks.
            let foreign = RepoLockInfo {
                lock_id: "b".repeat(64),
                time: None,
                exclusive: true,
                hostname: "foreign-host".to_string(),
                username: Some("bob".to_string()),
                pid: Some(999_999),
            };
            let foreign_cls = classify_locks(&[foreign], "this-host", |_| false);
            assert!(!foreign_cls.has_stale_local_lock);

            // AC-M3-06-1 + AC-M3-04-4: SIGKILL an exclusive `restic check --read-data` -> scheduled backup hits Exit 11 -> stale_lock_count >= 1 -> UnlockDestination -> runs & succeeds!
            let mut excl_child = Command::new(&restic_bin)
                .args(["-r", &local_repo.to_string_lossy(), "check", "--read-data"])
                .env("RESTIC_PASSWORD", "lock-pw")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn exclusive restic check");
            let excl_wait = Instant::now() + Duration::from_secs(10);
            while find_lock_file(&locks_dir).is_none() {
                assert!(Instant::now() < excl_wait, "exclusive lock never appeared");
                thread::sleep(Duration::from_millis(2));
            }
            excl_child.kill().expect("SIGKILL restic check");
            let _ = excl_child.wait();
            assert!(
                find_lock_file(&locks_dir).is_some(),
                "exclusive lock must remain on disk after SIGKILL of restic check"
            );

            // Confirm core stale-lock detector distinguishes foreign vs local hostnames
            let resolved = check_binary_version(&restic_bin).expect("resolved");
            let store = FileSecretStore::new(root.join("secrets.hex"));
            let kestic_bin = find_kestic_binary();
            let runtime = ResticRuntime {
                resolved: &resolved,
                kestic_binary: &kestic_bin,
                secret_store: &store,
                clock: clock.as_ref(),
            };
            let local_cls =
                detect_stale_locks_with(&local_dest, &runtime, "other-host", |_| false, &[])
                    .expect("foreign perspective");
            assert!(
                !local_cls.has_stale_local_lock,
                "AC-M3-06-3 foreign host must not offer unlock"
            );
            let self_cls =
                kestic_core::maintenance::detect_stale_locks(&local_dest, &runtime, &[])
                    .expect("local perspective");
            assert!(
                self_cls.has_stale_local_lock,
                "AC-M3-06-1 local stale exclusive lock must be present after SIGKILL"
            );

            // Advance clock to 14:00:05 CET -> scheduled run hits Exit 11 -> Backoff + stale_lock_count >= 1!
            notif_capture.clear();
            clock.set(datetime!(2026-12-01 13:00:05 UTC));
            let lock_state = wait_for_state(
                &client,
                Duration::from_secs(15),
                "Exit 11 stale lock",
                |s| {
                    let ds = dest_summary(s, &local_dest.id);
                    let bs = backup_summary(s, &local_bid);
                    ds.has_stale_lock && bs.pending_primary_code.as_deref() == Some("backoff")
                },
            );
            assert!(dest_summary(&lock_state, &local_dest.id).stale_lock_count >= 1);
            assert!(
                notif_capture
                    .snapshot()
                    .iter()
                    .all(|n| n.event_id.as_deref() != Some("BackupFailed")),
                "AC-M3-04-4: Exit 11 backoff must not emit immediate BackupFailed notification"
            );

            // UnlockDestination removes stale lock; advance 5m -> scheduled backup runs Ok!
            let removed = client
                .unlock_destination(local_dest.id.as_str())
                .expect("UnlockDestination");
            assert!(removed >= 1);
            clock.advance(Duration::from_secs(5 * 60));
            wait_for_state(&client, Duration::from_secs(15), "after unlock Ok", |s| {
                let bs = backup_summary(s, &local_bid);
                bs.last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
                    && bs.pending_primary_code.is_none()
            });

            // AC-M3-04-5: Wrong repo password (Exit 12) -> immediate Error + BackupFailed notification, no retry.
            store
                .set_secret(
                    local_dest.id.as_str(),
                    SECRET_KIND_REPO,
                    &SecretValue::from_str_secret("completely-wrong-password"),
                )
                .expect("overwrite wrong pw");
            notif_capture.clear();
            let _ = client.start_backup(&local_bid_str).expect("start wrong pw");
            let err_state =
                wait_for_state(&client, Duration::from_secs(15), "Exit 12 Error", |s| {
                    backup_summary(s, &local_bid).has_error
                });
            assert_eq!(
                backup_summary(&err_state, &local_bid)
                    .last_run
                    .as_ref()
                    .and_then(|r| r.exit_code),
                Some(12)
            );
            assert!(dest_summary(&err_state, &local_dest.id).password_rejected);
            thread::sleep(Duration::from_millis(200));
            assert!(
                notif_capture
                    .snapshot()
                    .iter()
                    .any(|n| n.event_id.as_deref() == Some("BackupFailed")),
                "Exit 12 must emit immediate BackupFailed notification"
            );

            let _ = state_dir;
            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_s05_queue_maintenance_and_no_lock_preview() {
    run_isolated_test(
        "m3_s05_queue_maintenance_and_no_lock_preview",
        "KESTIC_M3_S05_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let upower = FakeUPowerHandle::start(false, 100.0);
            let _nm = FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]);

            // Friday 2026-12-04 11:59:00 CET = 10:59:00 UTC
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-04 10:59:00 UTC)));
            let restic_bin = default_restic_latest();

            let (state_dir, client, handle) = spawn_m3_daemon(
                root,
                "s05",
                "net.daester.Kestic.M3S05",
                &restic_bin,
                Arc::clone(&clock),
                None,
                true,
                true,
                false,
                false,
            );

            let repo_dir = root.join("repo");
            let src_a = root.join("src_a");
            let src_b = root.join("src_b");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            write_incompressible_dataset(&src_a, 16, 1024 * 1024);
            fs::create_dir_all(&src_b).expect("mkdir src_b");
            fs::write(src_b.join("b.txt"), b"backup b data").expect("write b");

            let dest = Destination::new(
                "SharedZ",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            client
                .add_destination(&dest, "init", "pw-s05", "")
                .expect("init dest");

            let mut bk_a =
                Backup::new("Alpha", Source::folder(&src_a), dest.id.clone()).expect("bk a");
            bk_a.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let mut bk_b =
                Backup::new("Beta", Source::folder(&src_b), dest.id.clone()).expect("bk b");
            bk_b.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };

            let aid_str = client.add_backup(&bk_a).expect("add a");
            let bid_str = client.add_backup(&bk_b).expect("add b");
            let aid = BackupId::parse(&aid_str).expect("aid");
            let bid = BackupId::parse(&bid_str).expect("bid");

            // Both due at 12:00 CET -> sequential execution (`AC-M3-05-1`).
            clock.set(datetime!(2026-12-04 11:00:05 UTC));
            wait_for_state(&client, Duration::from_secs(20), "A and B both Ok", |s| {
                backup_summary(s, &aid)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
                    && backup_summary(s, &bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
            });

            // AC-M3-05-8: PreviewDestinationMaintenance uses `--dry-run --no-lock` and succeeds during non-exclusive lock.
            let mut stdin_child = Command::new(&restic_bin)
                .args([
                    "-r",
                    &repo_dir.to_string_lossy(),
                    "backup",
                    "--stdin",
                    "--stdin-filename",
                    "concurrent.txt",
                ])
                .env("RESTIC_PASSWORD", "pw-s05")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn stdin");
            let locks_dir = repo_dir.join("locks");
            let wait_lk = Instant::now() + Duration::from_secs(10);
            while find_lock_file(&locks_dir).is_none() {
                assert!(Instant::now() < wait_lk);
                thread::sleep(Duration::from_millis(20));
            }
            let preview = client
                .preview_destination_maintenance(dest.id.as_str())
                .expect("PreviewDestinationMaintenance under lock");
            assert!(
                preview.transparent_command.contains("--dry-run")
                    && preview.transparent_command.contains("--no-lock"),
                "preview command: {}",
                preview.transparent_command
            );
            if let Some(mut stdin) = stdin_child.stdin.take() {
                let _ = stdin.write_all(b"ok");
            }
            let _ = stdin_child.wait();

            // AC-M3-05-4 + AC-M3-05-2 + AC-M3-05-5 + AC-M3-05-6:
            // Mark pending_forget on Alpha, advance to first Sunday 2026-12-06 12:00 CET while OnBattery=true -> deferred (`AC-M3-05-5`).
            {
                let hist = RunHistory::open(&state_dir).expect("open history");
                hist.set_pending_forget(&aid, true)
                    .expect("set pending_forget");
            }
            upower.set_on_battery(true, 90.0);
            client
                .pause_backup(&aid_str, &PauseDuration::Indefinite)
                .expect("pause a");
            client
                .pause_backup(&bid_str, &PauseDuration::Indefinite)
                .expect("pause b");

            clock.set(datetime!(2026-12-06 11:00:05 UTC));
            thread::sleep(Duration::from_millis(1200));
            let s_batt = client.get_state().expect("GetState");
            assert!(
                dest_summary(&s_batt, &dest.id).last_prune_at_unix.is_none(),
                "maintenance must NOT run on battery (`AC-M3-05-5`)"
            );

            // Restore AC power -> maintenance runs `forget` -> `prune` -> `check` (no `--read-data`).
            upower.set_on_battery(false, 100.0);
            let s_maint =
                wait_for_state(&client, Duration::from_secs(20), "maintenance done", |s| {
                    let ds = dest_summary(s, &dest.id);
                    ds.last_prune_at_unix.is_some()
                        && ds.last_check_at_unix.is_some()
                        && ds.pending_forget_count == 0
                });
            let maint_run = dest_summary(&s_maint, &dest.id)
                .last_maintenance_run
                .as_ref()
                .expect("last_maintenance_run");
            assert_eq!(maint_run.kind, RunKind::Maintenance);
            assert_eq!(maint_run.status, RunStatus::Ok);
            let resolved = check_binary_version(&restic_bin).expect("resolved");
            let kbin = find_kestic_binary();
            assert!(
                !build_check_command_spec(&dest, &resolved, &kbin)
                    .restic_argv()
                    .iter()
                    .any(|a| a == "--read-data")
            );
            assert!(
                !build_prune_command_spec(&dest, false, false, &resolved, &kbin)
                    .restic_argv()
                    .iter()
                    .any(|a| a == "--retry-lock")
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
fn m3_s07_s08_pause_and_start_all_skips() {
    run_isolated_test(
        "m3_s07_s08_pause_and_start_all_skips",
        "KESTIC_M3_S07_INNER",
        |root| {
            let (_notif_conn, notif_capture) = start_fake_notifications();
            let sni_conn = start_fake_sni_watcher();

            // 2026-12-01 10:00:00 CET = 09:00:00 UTC
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 09:00:00 UTC)));
            let restic_bin = default_restic_latest();

            let (state_dir, client, handle) = spawn_m3_daemon(
                root,
                "s07",
                "net.daester.Kestic.M3S07",
                &restic_bin,
                Arc::clone(&clock),
                None,
                false,
                true,
                false,
                false,
            );
            let conn = client.connection().clone();
            let sni_service = wait_for_sni_service(&sni_conn, Duration::from_secs(5));

            let repo_dir = root.join("repo");
            let src_ok = root.join("src_ok");
            let src_ok2 = root.join("src_ok2");
            let src_missing = root.join("src_missing_does_not_exist");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_ok).expect("mkdir src_ok");
            fs::create_dir_all(&src_ok2).expect("mkdir src_ok2");
            fs::write(src_ok.join("ok.txt"), b"ok data").expect("write ok");
            fs::write(src_ok2.join("ok2.txt"), b"ok2 data").expect("write ok2");

            let dest = Destination::new(
                "LocalS07",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            client
                .add_destination(&dest, "init", "pw-s07", "")
                .expect("init");

            let mut bk_main =
                Backup::new("Main", Source::folder(&src_ok), dest.id.clone()).expect("bk main");
            bk_main.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let main_id_str = client.add_backup(&bk_main).expect("add main");
            let main_id = BackupId::parse(&main_id_str).expect("main id");

            // AC-M3-07-3 & AC-M3-07-4: Global pause for 4 hours (10:00 -> 14:00), across 12:00 slot.
            client
                .pause_global(&PauseDuration::Hours { hours: 4 })
                .expect("PauseGlobal(4h)");
            let paused_state = client.get_state().expect("GetState");
            assert!(paused_state.overall.paused);
            assert_eq!(
                read_sni_string_prop(&conn, &sni_service, "Status"),
                "Active",
                "AC-M3-07-4: SNI status must be Active during pause"
            );
            let menu = fetch_dbusmenu_items(&conn, &sni_service);
            assert!(
                menu.iter().any(|e| e.label.contains("Paused since")),
                "tray menu must show `Paused since ...`: {menu:?}"
            );

            // AC-M3-07-2: Manual backup starts even during global pause!
            client
                .start_backup(&main_id_str)
                .expect("manual StartBackup during global pause");
            wait_for_state(
                &client,
                Duration::from_secs(15),
                "manual ok while paused",
                |s| {
                    backup_summary(s, &main_id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );

            // Advance 3 days while paused -> zero overdue notifications, zero scheduled runs.
            client
                .pause_global(&PauseDuration::Indefinite)
                .expect("PauseGlobal(Indefinite)");
            clock.set(datetime!(2026-12-04 12:00:00 UTC));
            thread::sleep(Duration::from_millis(1100));
            let s_3days = client.get_state().expect("GetState 3 days paused");
            assert!(!backup_summary(&s_3days, &main_id).is_overdue);
            assert_eq!(notif_capture.snapshot().len(), 0);

            // Resume -> overdue anchor resets (`AC-M3-07-4`), catch-up runs once!
            client.resume_global().expect("ResumeGlobal");
            let s_resumed = wait_for_state(
                &client,
                Duration::from_secs(15),
                "catch-up after resume",
                |s| {
                    backup_summary(s, &main_id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| {
                            r.started_at >= datetime!(2026-12-04 12:00:00 UTC)
                                && r.status == RunStatus::Ok
                        })
                },
            );
            assert!(!backup_summary(&s_resumed, &main_id).is_overdue);

            // Pause UntilTomorrow
            client
                .pause_global(&PauseDuration::UntilTomorrow)
                .expect("PauseGlobal(UntilTomorrow)");
            let until_tmr = client.get_state().expect("GetState until tomorrow");
            assert!(until_tmr.overall.paused);
            client.resume_global().expect("ResumeGlobal");

            // AC-M3-08-6: StartAllBackups with 3 backups: missing source, Exit 12 error, valid.
            let bk_missing =
                Backup::new("MissingSrc", Source::folder(&src_missing), dest.id.clone())
                    .expect("bk missing");
            let missing_id_str = client.add_backup(&bk_missing).expect("add missing");
            let missing_id = BackupId::parse(&missing_id_str).expect("missing id");

            // Seed Exit 12 Error run on second destination so `has_error == true`
            let err_repo = root.join("err_repo");
            fs::create_dir_all(&err_repo).expect("mkdir err repo");
            let err_dest = Destination::new(
                "ErrDest",
                DestinationKind::Local {
                    path: err_repo.clone(),
                },
            )
            .expect("err dest");
            client
                .add_destination(&err_dest, "init", "real-pw", "")
                .expect("init err dest");
            let bk_err = Backup::new("ErrBk", Source::folder(&src_ok2), err_dest.id.clone())
                .expect("bk err");
            let err_id_str = client.add_backup(&bk_err).expect("add err bk");
            let err_id = BackupId::parse(&err_id_str).expect("err id");
            let store = FileSecretStore::new(root.join("secrets.hex"));
            store
                .set_secret(
                    err_dest.id.as_str(),
                    SECRET_KIND_REPO,
                    &SecretValue::from_str_secret("wrong-pw"),
                )
                .expect("wrong pw");
            let _ = client.start_backup(&err_id_str).expect("start err bk");
            wait_for_state(&client, Duration::from_secs(15), "err_bk Exit 12", |s| {
                backup_summary(s, &err_id).has_error
            });

            notif_capture.clear();
            let res = client.start_all_backups().expect("StartAllBackups");
            assert_eq!(
                res.started_run_ids.len(),
                1,
                "only Main must start: {res:?}"
            );
            assert_eq!(
                res.skipped.len(),
                2,
                "MissingSrc & ErrBk must skip: {res:?}"
            );
            assert!(
                res.skipped.iter().any(|s| s.backup_id == missing_id_str)
                    && res.skipped.iter().any(|s| s.backup_id == err_id_str),
                "skipped list: {res:?}"
            );

            // Check RunHistory contains skipped records too!
            let hist = RunHistory::open(&state_dir).expect("open history");
            let runs = hist.list_runs().expect("list runs");
            assert!(
                runs.iter().any(|r| {
                    r.backup_id() == Some(&missing_id)
                        && r.summary_message
                            .as_deref()
                            .is_some_and(|m| m.contains("Skipped"))
                }),
                "RunHistory should log skipped entry for missing backup"
            );

            // Tray click StartAllBackups also triggers skipped summary notification (`AC-M3-08-6`).
            wait_for_state(&client, Duration::from_secs(15), "Main finish", |s| {
                s.backups.iter().all(|b| b.active_run.is_none())
            });
            let tray_menu = fetch_dbusmenu_items(&conn, &sni_service);
            let start_all_item = tray_menu
                .iter()
                .find(|m| {
                    m.label == "All Backups"
                        || m.label.contains("Back Up All")
                        || m.label.contains("Back up all")
                        || m.label == "Back Up All Now"
                })
                .expect("tray Back Up All item");
            common::click_dbusmenu_item(&conn, &sni_service, start_all_item.id);
            let notif_deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < notif_deadline {
                if notif_capture
                    .snapshot()
                    .iter()
                    .any(|n| n.body.contains("MissingSrc") && n.body.contains("ErrBk"))
                {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            assert!(
                notif_capture
                    .snapshot()
                    .iter()
                    .any(|n| n.body.contains("MissingSrc") && n.body.contains("ErrBk")),
                "tray StartAllBackups must emit warning naming skipped items: {:?}",
                notif_capture.snapshot()
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
fn m3_s09_s10_s12_s13_sleep_jobview_and_udisks() {
    run_isolated_test(
        "m3_s09_s10_s12_s13_sleep_jobview_udisks",
        "KESTIC_M3_S09_INNER",
        |root| {
            let (_notif_conn, notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let _upower = FakeUPowerHandle::start(false, 100.0);
            let _nm = FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]);
            let logind = FakeLogindHandle::start();
            let udisks = FakeUDisks2Handle::start();
            let job_view = FakeJobViewHandle::start("org.kde.JobViewServerV2");
            let portal = FakePortalInhibitHandle::start();

            let restic_bin = default_restic_latest();
            let sub = root.join("s09");
            let state_dir = sub.join("state").join("kestic");
            let cfg_path = sub.join("cfg").join("kestic").join("config.toml");
            let repo_dir = sub.join("repo");
            let src_dir = sub.join("src");
            let mount_dir = sub.join("mnt/usb");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            write_incompressible_dataset(&src_dir, 96, 4 * 1024 * 1024);

            // Clock = 2026-12-03 12:05:00 CET (3 days missed since baseline 2026-12-01 08:00 CET)
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-03 11:05:00 UTC)));

            let dest = Destination::new(
                "LocalS09",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            let mut catchup_bk =
                Backup::new("CatchupBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            catchup_bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };

            let mut mount_bk = Backup::new(
                "UsbBk",
                Source::drive_with_identity(
                    mount_dir.join("photos"),
                    Some(SourceIdentity::FsUuid {
                        fs_uuid: "1111-2222".to_string(),
                    }),
                    Some("USB".to_string()),
                ),
                dest.id.clone(),
            )
            .expect("usb bk");
            mount_bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };

            let store = Arc::new(FileSecretStore::new(root.join("secrets.hex")));
            let resolved = check_binary_version(&restic_bin).expect("resolved");
            let kbin = find_kestic_binary();
            let rt = ResticRuntime {
                resolved: &resolved,
                kestic_binary: &kbin,
                secret_store: store.as_ref(),
                clock: clock.as_ref(),
            };
            kestic_core::destination::setup_destination(
                &dest,
                kestic_core::destination::DestinationSetupMode::InitNew,
                &SecretValue::from_str_secret("pw-s09"),
                None,
                &rt,
            )
            .expect("init repo");

            let mut cfg = kestic_core::config::AppConfig {
                restic_provider: ResticProvider::Custom {
                    path: restic_bin.clone(),
                },
                ..Default::default()
            };
            cfg.destinations.push(dest.clone());
            cfg.backups.push(catchup_bk.clone());
            cfg.backups.push(mount_bk.clone());
            kestic_core::config::save_config(&cfg_path, &cfg).expect("save config");

            let mut hist = RunHistory::open(&state_dir).expect("open history");
            let stuck_run = Run::new_backup(clock.as_ref(), &catchup_bk).expect("stuck run");
            hist.upsert_run(&stuck_run, &[]).expect("seed stuck run");
            let mut rs = hist.get_backup_runtime_state(&catchup_bk.id).expect("rs");
            rs.baseline_at = Some(datetime!(2026-12-01 07:00:00 UTC));
            hist.upsert_backup_runtime_state(&catchup_bk.id, &rs)
                .expect("seed baseline");
            let mut rs_usb = hist.get_backup_runtime_state(&mount_bk.id).expect("rs_usb");
            rs_usb.baseline_at = Some(datetime!(2026-12-01 07:00:00 UTC));
            hist.upsert_backup_runtime_state(&mount_bk.id, &rs_usb)
                .expect("seed baseline usb");
            drop(hist);

            let fake_mounts = Arc::new(FakeMountInspector::new(FakeMountState::default()));
            let (_sdir, client, handle) = spawn_m3_daemon(
                root,
                "s09",
                "net.daester.Kestic.M3S09",
                &restic_bin,
                Arc::clone(&clock),
                Some(Arc::clone(&fake_mounts) as Arc<dyn kestic_core::mount::MountInspector>),
                true,
                true,
                true,
                true,
            );

            // AC-M3-10-3: stuck `Running` run marked `Interrupted` at daemon startup!
            let detail = client
                .get_run_detail(stuck_run.id.as_str())
                .expect("GetRunDetail");
            assert_eq!(detail.run.status, RunStatus::Interrupted);

            // Wait for catch-up run to start -> JobView + Portal + Logind inhibitor active (`AC-M3-09-1`, `AC-M3-12-1`)
            wait_for_state(&client, Duration::from_secs(10), "catchup running", |s| {
                backup_summary(s, &catchup_bk.id).active_run.is_some()
                    && job_view
                        .recorder
                        .request_count
                        .load(std::sync::atomic::Ordering::SeqCst)
                        >= 1
            });
            assert!(
                portal
                    .recorder
                    .inhibit_count
                    .load(std::sync::atomic::Ordering::SeqCst)
                    >= 1,
                "AC-M3-09-1: PortalInhibit must be acquired"
            );
            assert_eq!(
                logind.open_inhibitor_count(),
                1,
                "AC-M3-09-1: logind sleep delay inhibitor FD must be open while running"
            );

            // AC-M3-09-2: Simulate PrepareForSleep(true) -> SIGTERM, Interrupted, inhibitor released!
            logind.emit_prepare_for_sleep(true);
            wait_for_state(&client, Duration::from_secs(15), "sleep interrupted", |s| {
                backup_summary(s, &catchup_bk.id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Interrupted)
                    && backup_summary(s, &catchup_bk.id).active_run.is_none()
            });
            thread::sleep(Duration::from_millis(150));
            assert_eq!(
                logind.open_inhibitor_count(),
                0,
                "AC-M3-09-2: logind delay FD must be released once interrupted"
            );
            assert!(
                portal
                    .recorder
                    .closed_count
                    .load(std::sync::atomic::Ordering::SeqCst)
                    >= 1,
                "Portal inhibit closed on run end"
            );

            // Shrink dataset so post-wake resume completes fast, then emit PrepareForSleep(false) (`AC-M3-09-3`).
            fs::remove_dir_all(&src_dir).expect("rm big src");
            fs::create_dir_all(&src_dir).expect("mkdir small src");
            fs::write(src_dir.join("small.txt"), b"resumed").expect("write small");

            logind.emit_prepare_for_sleep(false);
            wait_for_state(
                &client,
                Duration::from_secs(15),
                "post-wake resume Ok",
                |s| {
                    backup_summary(s, &catchup_bk.id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );

            // Check JobView received desktop_entry + terminate(0) (`AC-M3-12-1`)
            assert_eq!(
                *job_view.recorder.desktop_entry.lock().expect("lock"),
                "net.daester.kestic"
            );
            assert!(
                job_view
                    .recorder
                    .terminated
                    .load(std::sync::atomic::Ordering::SeqCst),
                "JobView must terminate on run end"
            );

            // Check UsbBk is overdue & pending SourceUnavailable (`AC-M3-13-1`, `AC-M3-13-4`)
            let s_usb = wait_for_state(
                &client,
                Duration::from_secs(5),
                "UsbBk pending source",
                |s| {
                    backup_summary(s, &mount_bk.id)
                        .pending_primary_code
                        .as_deref()
                        == Some("source_unavailable")
                },
            );
            assert!(backup_summary(&s_usb, &mount_bk.id).is_overdue);

            // AC-M3-13-3: Wrong UUID mount does NOT trigger UsbBk!
            let other_mp = sub.join("mnt/other/photos");
            fs::create_dir_all(&other_mp).expect("mkdir other");
            udisks.emit_mount_added(
                "sdb1",
                "9999-8888",
                &sub.join("mnt/other").to_string_lossy(),
            );
            thread::sleep(Duration::from_millis(250));
            let s_other = client.get_state().expect("GetState after wrong UUID");
            assert_eq!(
                backup_summary(&s_other, &mount_bk.id)
                    .pending_primary_code
                    .as_deref(),
                Some("source_unavailable")
            );

            // AC-M3-13-2: Matching UUID mounts -> UsbBk starts immediately!
            let target_photos = mount_dir.join("photos");
            fs::create_dir_all(&target_photos).expect("mkdir target photos");
            fs::write(target_photos.join("pic.jpg"), b"photo").expect("write photo");
            let mount_mp_str = mount_dir.to_string_lossy().into_owned();
            fake_mounts.update(|st| {
                st.mountinfo = format!(
                    "42 1 8:33 / {mount_mp_str} rw,relatime shared:1 - ext4 /dev/sdc1 rw\n"
                );
                st.device_uuids.insert(
                    "/dev/sdc1".to_string(),
                    ("1111-2222".to_string(), Some("USB".to_string())),
                );
            });
            udisks.emit_mount_added("sdc1", "1111-2222", &mount_mp_str);

            wait_for_state(
                &client,
                Duration::from_secs(15),
                "UsbBk mounted & Ok",
                |s| {
                    backup_summary(s, &mount_bk.id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );

            // AC-M3-12-2: Cancel via Plasma JobView `cancelRequested` signal ends active run as Cancelled!
            write_incompressible_dataset(&src_dir, 96, 4 * 1024 * 1024);
            let cancel_signals = KesticSignalCollector::start(
                "net.daester.Kestic.M3S09",
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );
            let cancel_rid = client
                .start_backup(catchup_bk.id.as_str())
                .expect("start backup for JobView cancel");
            let jv_wait = Instant::now() + Duration::from_millis(400);
            while Instant::now() < jv_wait {
                if !job_view.recorder.updates.lock().expect("lock").is_empty() {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
            job_view.emit_cancel_requested();
            cancel_signals.wait_until(Duration::from_secs(15), |sigs| {
                sigs.iter().any(|s| {
                    if let common::CapturedKesticSignal::RunUpdated(j) = s
                        && let Ok(r) = serde_json::from_str::<Run>(j)
                    {
                        return r.id.as_str() == cancel_rid && r.status == RunStatus::Cancelled;
                    }
                    false
                })
            });

            let _ = notif_capture;
            client.confirm_quit().expect("quit");
            assert_eq!(
                handle.join().expect("join"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_s11_binary_watch_self_restart_idle_vs_busy() {
    run_isolated_test(
        "m3_s11_binary_watch_self_restart_idle_vs_busy",
        "KESTIC_M3_S11_INNER",
        |root| {
            let _sni = start_fake_sni_watcher();
            let bin_dir = root.join("bin");
            fs::create_dir_all(&bin_dir).expect("mkdir bin");
            let real_bin = find_kestic_binary();
            let installed_bin = bin_dir.join("kestic");
            let sibling_file = bin_dir.join("other_helper");
            fs::copy(&real_bin, &installed_bin).expect("copy kestic binary");
            fs::set_permissions(&installed_bin, fs::Permissions::from_mode(0o755))
                .expect("chmod 755");
            fs::write(&sibling_file, b"v1").expect("write sibling");

            let cfg_path = root.join("cfg/kestic/config.toml");
            let state_dir = root.join("state/kestic");
            let ready_file = root.join("daemon.ready");
            let bus_name = "net.daester.Kestic.M3S11";
            let restic_bin = default_restic_latest();

            let mut child = Command::new(&installed_bin)
                .args([
                    "daemon",
                    "--no-monitors",
                    "--no-scheduler",
                    "--no-job-view",
                    "--no-portal-inhibit",
                    "--suppress-ui-spawn",
                    "--bus-name",
                    bus_name,
                    "--config-path",
                    &cfg_path.to_string_lossy(),
                    "--state-dir",
                    &state_dir.to_string_lossy(),
                    "--ready-file",
                    &ready_file.to_string_lossy(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn installed kestic --daemon");

            let child_guard = DropKillChild(&mut child);
            wait_for_file(&ready_file, Duration::from_secs(10));
            let pid_before = child_guard.id();

            // AC-M3-11-3: Rename sibling file in same directory -> NO restart!
            fs::remove_file(&ready_file).expect("rm ready");
            let sibling_new = bin_dir.join("other_helper.new");
            fs::write(&sibling_new, b"v2").expect("write sibling new");
            fs::rename(&sibling_new, &sibling_file).expect("rename sibling");
            thread::sleep(Duration::from_millis(800));
            assert!(
                !ready_file.exists(),
                "AC-M3-11-3: sibling file rename must NOT restart daemon"
            );

            // AC-M3-11-1: Rename kestic binary while idle -> restarts itself via exec() in same PID!
            let bin_new = bin_dir.join("kestic.new");
            fs::copy(&real_bin, &bin_new).expect("copy new bin");
            fs::set_permissions(&bin_new, fs::Permissions::from_mode(0o755)).expect("chmod 755");
            fs::rename(&bin_new, &installed_bin).expect("rename kestic binary");
            wait_for_file(&ready_file, Duration::from_secs(10));
            assert_eq!(child_guard.id(), pid_before);

            // AC-M3-11-2: Busy backup delays restart until completion, then restarts!
            let client = KesticClient::connect(bus_name, KESTIC_OBJECT_PATH).expect("connect");
            client
                .set_restic_provider(&ResticProvider::Custom { path: restic_bin })
                .expect("set restic");
            let repo_dir = root.join("repo");
            let src_dir = root.join("src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            write_incompressible_dataset(&src_dir, 96, 4 * 1024 * 1024);

            let dest =
                Destination::new("LocalS11", DestinationKind::Local { path: repo_dir }).expect("d");
            client
                .add_destination(&dest, "init", "pw-s11", "")
                .expect("init");
            let mut bk =
                Backup::new("BusyBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            bk.schedule = Schedule::ManualOnly;
            let bid_str = client.add_backup(&bk).expect("add");
            let bid = BackupId::parse(&bid_str).expect("bid");

            fs::remove_file(&ready_file).expect("rm ready before busy backup");
            let _rid = client.start_backup(&bid_str).expect("start busy bk");
            wait_for_state(&client, Duration::from_secs(5), "busy running", |s| {
                backup_summary(s, &bid).active_run.is_some()
            });

            // Replace binary mid-backup -> must NOT interrupt running backup!
            fs::copy(&real_bin, &bin_new).expect("copy bin v3");
            fs::set_permissions(&bin_new, fs::Permissions::from_mode(0o755)).expect("chmod 755");
            fs::rename(&bin_new, &installed_bin).expect("rename v3");
            thread::sleep(Duration::from_millis(650));

            wait_for_file(&ready_file, Duration::from_secs(20));
            let client_after =
                KesticClient::connect(bus_name, KESTIC_OBJECT_PATH).expect("reconnect");
            let s_after = client_after.get_state().expect("GetState after exec");
            let last = backup_summary(&s_after, &bid)
                .last_run
                .as_ref()
                .expect("last run");
            assert_eq!(
                last.status,
                RunStatus::Ok,
                "AC-M3-11-2: busy backup must complete Ok before daemon self-exec restart"
            );
            client_after.confirm_quit().expect("quit");
        },
    );
}

struct DropKillChild<'a>(&'a mut std::process::Child);

impl<'a> DropKillChild<'a> {
    fn id(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for DropKillChild<'_> {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
