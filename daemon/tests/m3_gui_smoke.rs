// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Comprehensive offscreen GUI smoke suite for Milestone M3 (`M3-S01`–`M3-S14`).
//!
//! Each test runs isolated via [`common::run_isolated_test`] with its own private
//! `dbus-daemon`, `XDG_*` directories, `KESTIC_SECRET_FILE`, injectable D-Bus system
//! fakes (`FakeUPowerHandle`, `FakeNetworkManagerHandle`), `FakeClock`, and `kestic --smoke-m1`,
//! verifying UI state, keyboard navigation, accessibility labels, screenshots in EN/DE and
//! wide/narrow layouts, and persisted `config.toml` / daemon behavior.

mod common;

use common::fakes::{FakeNetworkManagerHandle, FakeUPowerHandle};
use common::{
    find_kestic_binary, restic_test_binaries, run_isolated_test, start_fake_notifications,
    start_fake_sni_watcher, wait_for_file,
};
use kestic_core::clock::FakeClock;
use kestic_core::config::load_config;
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationKind, MaintenanceEntrySchedule, ResticProvider, Run,
    RunConditions, RunStatus, Schedule, Source,
};
use kestic_core::scheduler::TimezoneSpec;
use kestic_core::secrets::FileSecretStore;
use kestic_daemon::dbus_api::{
    BackupSummaryDto, DaemonStateDto, KESTIC_BUS_NAME, KESTIC_OBJECT_PATH, KesticClient,
};
use kestic_daemon::monitors::MonitorBusConfig;
use kestic_daemon::service::{DaemonOutcome, DaemonRunConfig, run_daemon};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

fn find_target_dir() -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| common::workspace_root().join("target"))
}

fn install_compiled_german_catalog(root: &Path) {
    let built_mo = find_target_dir().join("locale/de/LC_MESSAGES/kestic.mo");
    if built_mo.is_file() {
        let dest_dir = root.join("data/locale/de/LC_MESSAGES");
        fs::create_dir_all(&dest_dir).expect("mkdir data/locale/de/LC_MESSAGES");
        fs::copy(&built_mo, dest_dir.join("kestic.mo")).expect("copy compiled kestic.mo");
    }
}

fn spawn_m3_gui_daemon(
    root: &Path,
    restic_bin: &Path,
    clock: Arc<FakeClock>,
    enable_monitors: bool,
    enable_scheduler: bool,
) -> (
    PathBuf,
    PathBuf,
    KesticClient,
    thread::JoinHandle<Result<DaemonOutcome, String>>,
) {
    let cfg_path = root.join("cfg").join("kestic").join("config.toml");
    let state_dir = root.join("state").join("kestic");
    let ready_file = root.join("daemon.ready");
    let _ = fs::remove_file(&ready_file);
    let secret_file = root.join("secrets.hex");

    let run_cfg = DaemonRunConfig {
        bus_name: KESTIC_BUS_NAME.to_string(),
        assume_sni_available: true,
        ready_file: Some(ready_file.clone()),
        config_path: Some(cfg_path.clone()),
        state_dir: Some(state_dir.clone()),
        kestic_binary: Some(find_kestic_binary()),
        clock,
        secret_store: Some(Arc::new(FileSecretStore::new(secret_file))),
        mount_inspector: None,
        suppress_ui_spawn: true,
        enable_job_view: false,
        enable_portal_inhibit: false,
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
    wait_for_file(&ready_file, Duration::from_secs(8));

    let client =
        KesticClient::connect(KESTIC_BUS_NAME, KESTIC_OBJECT_PATH).expect("connect KesticClient");
    let prov = client
        .set_restic_provider(&ResticProvider::Custom {
            path: restic_bin.to_path_buf(),
        })
        .expect("set restic provider");
    assert!(prov.valid, "restic provider must be valid: {prov:?}");
    (cfg_path, state_dir, client, handle)
}

fn run_m3_gui_smoke(root: &Path, scenario: &str, layout: &str, lang: &str, screenshot_rel: &str) {
    install_compiled_german_catalog(root);
    let kestic_bin = find_kestic_binary();
    let screenshot = common::workspace_root().join(screenshot_rel);
    if let Some(parent) = screenshot.parent() {
        fs::create_dir_all(parent).expect("mkdir screenshot parent");
    }
    if screenshot.exists() {
        fs::remove_file(&screenshot).expect("remove stale screenshot");
    }

    let mut cmd = Command::new(&kestic_bin);
    cmd.args([
        "--smoke-m1",
        scenario,
        "--smoke-layout",
        layout,
        "--smoke-lang",
        lang,
        "--screenshot",
    ])
    .arg(&screenshot)
    .env("HOME", root.join("home"))
    .env("XDG_CONFIG_HOME", root.join("cfg"))
    .env("XDG_DATA_HOME", root.join("data"))
    .env("XDG_CACHE_HOME", root.join("cache"))
    .env("XDG_STATE_HOME", root.join("state"))
    .env("XDG_RUNTIME_DIR", root.join("run"))
    .env("KESTIC_SECRET_FILE", root.join("secrets.hex"))
    .env("QT_QPA_PLATFORM", "offscreen")
    .env_remove("LC_ALL")
    .env_remove("LC_MESSAGES")
    .env("LANG", "en_US.UTF-8")
    .env("LANGUAGE", lang);

    let output = cmd.output().expect("run kestic --smoke-m1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "M3 GUI smoke `{scenario}` ({layout}/{lang}) failed (status={:?}):\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        stderr
    );
    assert!(
        stderr.trim().is_empty(),
        "M3 GUI smoke `{scenario}` ({layout}/{lang}) emitted QML/Qt warnings:\n{stderr}"
    );

    let png = fs::read(&screenshot).expect("read captured M3 PNG screenshot");
    assert!(
        png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 2048,
        "M3 screenshot {} must be a valid non-trivial PNG (got {} bytes)",
        screenshot.display(),
        png.len()
    );
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
            "timed out waiting for condition ({label}); backups={:?}",
            state
                .backups
                .iter()
                .map(|b| (
                    &b.backup.name,
                    &b.pending_primary_code,
                    b.is_overdue,
                    b.has_error,
                    b.error_action.as_deref(),
                    b.active_run.as_ref().map(|r| &r.status),
                    b.last_run.as_ref().map(|r| &r.status),
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

#[test]
fn m3_gui_smoke_overview_schedule_pause_and_start_all() {
    run_isolated_test(
        "m3_gui_smoke_overview_schedule_pause_and_start_all",
        "KESTIC_M3_GUI_OVERVIEW_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let _upower = FakeUPowerHandle::start(false, 100.0);
            let _nm = FakeNetworkManagerHandle::start(true, false, Some("HomeWifi"), &["HomeWifi"]);

            // 2026-12-01 10:00:00 UTC -> after registration, advance to 2026-12-04 12:00 UTC
            // so Daily 12:00 backup has >= 2 missed slots (`isOverdue = true`).
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let restic_bin = default_restic_latest();
            let (_cfg_path, _state_dir, client, handle) =
                spawn_m3_gui_daemon(root, &restic_bin, Arc::clone(&clock), true, false);

            let repo_dir = root.join("repo-overview");
            let src_ok = root.join("src-overview-ok");
            let src_missing = root.join("src-overview-missing");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_ok).expect("mkdir src_ok");
            fs::write(src_ok.join("doc.txt"), b"overview payload").expect("write doc.txt");

            let dest = Destination::new(
                "Local Vault",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            client
                .add_destination(&dest, "init", "m3-smoke-pw", "")
                .expect("init dest");

            let mut b0 =
                Backup::new("Documents", Source::folder(&src_ok), dest.id.clone()).expect("b0");
            b0.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let b0_id_str = client.add_backup(&b0).expect("add b0");
            let b0_id = BackupId::parse(&b0_id_str).expect("parse b0 id");

            // Second backup points to a missing folder so `Back Up All Now` skips it and shows
            // `startAllSkippedBanner` (`AC-M3-08-6`).
            let b1 = Backup::new(
                "Offline Media",
                Source::folder(&src_missing),
                dest.id.clone(),
            )
            .expect("b1");
            let _ = client.add_backup(&b1).expect("add b1");

            // Advance 3 days -> 3 missed slots -> Backup 0 overdue; scheduler disabled so it stays
            // ready until `Back Up All Now` is triggered inside the smoke test.
            clock.set(datetime!(2026-12-04 12:00:00 UTC));
            wait_for_state(&client, Duration::from_secs(5), "b0 overdue", |s| {
                backup_summary(s, &b0_id).is_overdue
            });

            // 1. Wide / EN
            run_m3_gui_smoke(
                root,
                "m3-overview-schedule-and-pause",
                "wide",
                "en",
                "target/smoke-artifacts/m3-overview-wide-en.png",
            );
            wait_for_state(
                &client,
                Duration::from_secs(15),
                "b0 idle after pass 1",
                |s| {
                    backup_summary(s, &b0_id).active_run.is_none()
                        && backup_summary(s, &b0_id)
                            .last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );

            // 2. Wide / DE (advance 4 days so b0 becomes overdue again)
            clock.advance(Duration::from_secs(4 * 24 * 3600));
            wait_for_state(&client, Duration::from_secs(5), "b0 overdue pass 2", |s| {
                backup_summary(s, &b0_id).is_overdue
            });
            run_m3_gui_smoke(
                root,
                "m3-overview-schedule-and-pause",
                "wide",
                "de",
                "target/smoke-artifacts/m3-overview-wide-de.png",
            );
            wait_for_state(
                &client,
                Duration::from_secs(15),
                "b0 idle after pass 2",
                |s| backup_summary(s, &b0_id).active_run.is_none(),
            );

            // 3. Narrow / EN (advance 4 days so b0 becomes overdue again)
            clock.advance(Duration::from_secs(4 * 24 * 3600));
            wait_for_state(&client, Duration::from_secs(5), "b0 overdue pass 3", |s| {
                backup_summary(s, &b0_id).is_overdue
            });
            run_m3_gui_smoke(
                root,
                "m3-overview-schedule-and-pause",
                "narrow",
                "en",
                "target/smoke-artifacts/m3-overview-narrow-en.png",
            );
            wait_for_state(
                &client,
                Duration::from_secs(15),
                "b0 idle after pass 3",
                |s| backup_summary(s, &b0_id).active_run.is_none(),
            );

            client.confirm_quit().expect("quit daemon");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_gui_smoke_exit12_and_on_battery_override() {
    run_isolated_test(
        "m3_gui_smoke_exit12_and_on_battery_override",
        "KESTIC_M3_GUI_EXIT12_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            // On battery at 45% -> unmet RunCondition for Backup 1 (`on_battery`).
            let _upower = FakeUPowerHandle::start(true, 45.0);
            let _nm = FakeNetworkManagerHandle::start(true, false, Some("HomeWifi"), &["HomeWifi"]);

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:55:00 UTC)));
            let restic_bin = default_restic_latest();
            let (_cfg_path, state_dir, client, handle) =
                spawn_m3_gui_daemon(root, &restic_bin, Arc::clone(&clock), true, true);

            let repo_dir = root.join("repo-exit12");
            let src0 = root.join("src-exit12");
            let src1 = root.join("src-battery");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src0).expect("mkdir src0");
            fs::create_dir_all(&src1).expect("mkdir src1");
            fs::write(src0.join("a.txt"), b"encrypted").expect("write a.txt");
            fs::write(src1.join("b.txt"), b"battery override").expect("write b.txt");

            let dest = Destination::new(
                "Locked Vault",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            client
                .add_destination(&dest, "init", "m3-smoke-pw", "")
                .expect("init dest");

            let mut b0 =
                Backup::new("Encrypted Docs", Source::folder(&src0), dest.id.clone()).expect("b0");
            b0.schedule = Schedule::ManualOnly;
            let b0_id_str = client.add_backup(&b0).expect("add b0");
            let b0_id = BackupId::parse(&b0_id_str).expect("parse b0");

            // Seed Exit-12 run directly in RunHistory before GUI starts, then trigger a no-op
            // config refresh so DaemonService reloads `has_error=true` + `error_action="update_stored_password"`.
            let mut err_run = Run::new_backup(&*clock, &b0).expect("new err run");
            err_run.summary_message = Some("wrong password or no key found".to_string());
            err_run.finish(&*clock, RunStatus::Error, Some(12));
            {
                let mut history = RunHistory::open(&state_dir).expect("open history");
                history
                    .upsert_run(&err_run, &[])
                    .expect("upsert exit 12 run");
            }

            let mut b1 =
                Backup::new("Battery Backup", Source::folder(&src1), dest.id.clone()).expect("b1");
            b1.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            b1.run_conditions.require_ac_power = true;
            let b1_id_str = client.add_backup(&b1).expect("add b1");
            let b1_id = BackupId::parse(&b1_id_str).expect("parse b1");

            // Advance past 12:00 CET (11:00 UTC) so b1 evaluates as pending (`on_battery`).
            clock.set(datetime!(2026-12-01 11:05:00 UTC));
            wait_for_state(
                &client,
                Duration::from_secs(5),
                "exit12 + on_battery",
                |s| {
                    let s0 = backup_summary(s, &b0_id);
                    let s1 = backup_summary(s, &b1_id);
                    s0.error_action.as_deref() == Some("update_stored_password")
                        && s1.pending_primary_code.as_deref() == Some("on_battery")
                },
            );

            run_m3_gui_smoke(
                root,
                "m3-exit12-and-condition-override",
                "wide",
                "en",
                "target/smoke-artifacts/m3-condition-override-en.png",
            );

            // Confirm the manual condition override in the dialog caused Backup 1 to actually execute.
            wait_for_state(
                &client,
                Duration::from_secs(15),
                "override completed",
                |s| {
                    backup_summary(s, &b1_id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );

            client.confirm_quit().expect("quit daemon");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_gui_smoke_schedule_and_conditions_editors() {
    run_isolated_test(
        "m3_gui_smoke_schedule_and_conditions_editors",
        "KESTIC_M3_GUI_EDITORS_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let _upower = FakeUPowerHandle::start(false, 100.0);
            let _nm = FakeNetworkManagerHandle::start(
                true,
                false,
                Some("HomeWifi"),
                &["HomeWifi", "OfficeWifi"],
            );

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let restic_bin = default_restic_latest();
            let (cfg_path, _state_dir, client, handle) =
                spawn_m3_gui_daemon(root, &restic_bin, Arc::clone(&clock), true, false);

            let repo_dir = root.join("repo-editors");
            let src_dir = root.join("src-editors");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::write(src_dir.join("note.txt"), b"editor test").expect("write note.txt");

            let dest = Destination::new(
                "Editor Vault",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            client
                .add_destination(&dest, "init", "m3-smoke-pw", "")
                .expect("init dest");

            let mut backup =
                Backup::new("Documents", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            backup.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let bid_str = client.add_backup(&backup).expect("add backup");
            let bid = BackupId::parse(&bid_str).expect("parse bid");

            // 1. Wide / EN
            run_m3_gui_smoke(
                root,
                "m3-schedule-and-conditions-editors",
                "wide",
                "en",
                "target/smoke-artifacts/m3-schedule-editor-wide-en.png",
            );

            let saved = load_config(&cfg_path).expect("load config after EN editors");
            let saved_bk = saved
                .backups
                .iter()
                .find(|b| b.id == bid)
                .expect("saved backup present");
            assert!(
                matches!(
                    saved_bk.schedule,
                    Schedule::EveryNDays {
                        interval_days: 4,
                        hour: 12,
                        minute: 0,
                        ..
                    }
                ),
                "expected EveryNDays(4) persisted from BackupScheduleEditPage: {:?}",
                saved_bk.schedule
            );
            assert!(
                saved_bk.run_conditions.skip_on_metered_network,
                "expected skip_on_metered_network=true persisted from BackupConditionsEditPage"
            );
            assert!(
                saved_bk
                    .run_conditions
                    .allowed_networks
                    .contains(&"HomeWifi".to_string()),
                "expected HomeWifi in allowed_networks: {:?}",
                saved_bk.run_conditions.allowed_networks
            );

            // Reset backup schedule/conditions and run Wide / DE to verify localized editor & wizard summary.
            let mut reset_bk = saved_bk.clone();
            reset_bk.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            reset_bk.run_conditions = RunConditions::default();
            client.update_backup(&reset_bk).expect("reset backup");

            run_m3_gui_smoke(
                root,
                "m3-schedule-and-conditions-editors",
                "wide",
                "de",
                "target/smoke-artifacts/m3-schedule-editor-wide-de.png",
            );

            let saved_de = load_config(&cfg_path).expect("load config after DE editors");
            assert!(matches!(
                saved_de.backups[0].schedule,
                Schedule::EveryNDays {
                    interval_days: 4,
                    ..
                }
            ));

            client.confirm_quit().expect("quit daemon");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}

#[test]
fn m3_gui_smoke_destination_maintenance_and_settings() {
    run_isolated_test(
        "m3_gui_smoke_destination_maintenance_and_settings",
        "KESTIC_M3_GUI_MAINT_SETTINGS_INNER",
        |root| {
            let (_notif_conn, _notif_capture) = start_fake_notifications();
            let _sni_conn = start_fake_sni_watcher();
            let _upower = FakeUPowerHandle::start(false, 100.0);
            let _nm = FakeNetworkManagerHandle::start(true, false, Some("HomeWifi"), &["HomeWifi"]);

            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 10:00:00 UTC)));
            let restic_bin = default_restic_latest();
            let (cfg_path, _state_dir, client, handle) =
                spawn_m3_gui_daemon(root, &restic_bin, Arc::clone(&clock), true, false);

            let repo_dir = root.join("repo-maint");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");

            let dest = Destination::new(
                "Maintenance Vault",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            let did = dest.id.clone();
            client
                .add_destination(&dest, "init", "m3-smoke-pw", "")
                .expect("init dest");

            let before_check = load_config(&cfg_path)
                .expect("load initial config")
                .destinations[0]
                .maintenance
                .check
                .clone();

            run_m3_gui_smoke(
                root,
                "m3-destination-maintenance-and-settings",
                "wide",
                "en",
                "target/smoke-artifacts/m3-prune-confirm-en.png",
            );

            let saved = load_config(&cfg_path).expect("load config after maint + settings");
            let saved_dest = saved
                .destinations
                .iter()
                .find(|d| d.id == did)
                .expect("saved dest present");
            assert!(
                matches!(
                    saved_dest.maintenance.prune,
                    MaintenanceEntrySchedule::Daily { .. }
                ),
                "expected confirmed prune schedule Daily persisted: {:?}",
                saved_dest.maintenance.prune
            );
            assert_eq!(
                saved_dest.maintenance.check, before_check,
                "discarded check frequency change must NOT be persisted"
            );
            assert_eq!(
                saved.default_overdue_threshold, 4,
                "expected default_overdue_threshold=4 persisted from SettingsDialog"
            );
            assert!(
                !saved.notifications.on_overdue,
                "expected notifications.on_overdue=false persisted from SettingsDialog"
            );

            client.confirm_quit().expect("quit daemon");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}
