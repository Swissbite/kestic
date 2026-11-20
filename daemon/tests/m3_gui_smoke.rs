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
    RestServerGuard, find_kestic_binary, restic_test_binaries, run_isolated_test,
    start_fake_notifications, start_fake_sni_watcher, wait_for_file,
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
    run_m3_gui_smoke_with_env(root, scenario, layout, lang, screenshot_rel, &[]);
}

fn run_m3_gui_smoke_with_env(
    root: &Path,
    scenario: &str,
    layout: &str,
    lang: &str,
    screenshot_rel: &str,
    extra_env: &[(&str, &Path)],
) {
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
    for (key, value) in extra_env {
        cmd.env(key, value);
    }

    let output = cmd.output().expect("run kestic --smoke-m1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "M3 GUI smoke `{scenario}` ({layout}/{lang}) failed (status={}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
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
                    b.error_action.map(|c| c.as_str()),
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

/// Everything a GUI smoke test needs: private fakes on the session bus plus a running daemon.
struct Env {
    cfg_path: PathBuf,
    client: KesticClient,
    handle: thread::JoinHandle<Result<DaemonOutcome, String>>,
    clock: Arc<FakeClock>,
    _fakes: (
        zbus::Connection,
        zbus::Connection,
        FakeUPowerHandle,
        FakeNetworkManagerHandle,
    ),
}

fn start_env(
    root: &Path,
    on_battery: bool,
    known_networks: &[&str],
    start: time::OffsetDateTime,
    enable_scheduler: bool,
) -> Env {
    let (notif_conn, _notif_capture) = start_fake_notifications();
    let sni_conn = start_fake_sni_watcher();
    let upower = FakeUPowerHandle::start(on_battery, 100.0);
    let nm = FakeNetworkManagerHandle::start(true, false, Some("HomeWifi"), known_networks);
    let clock = Arc::new(FakeClock::new(start));
    let (cfg_path, _state_dir, client, handle) = spawn_m3_gui_daemon(
        root,
        &default_restic_latest(),
        Arc::clone(&clock),
        true,
        enable_scheduler,
    );
    Env {
        cfg_path,
        client,
        handle,
        clock,
        _fakes: (notif_conn, sni_conn, upower, nm),
    }
}

fn stop_env(env: Env) {
    env.client.confirm_quit().expect("quit daemon");
    assert_eq!(
        env.handle.join().expect("join daemon"),
        Ok(DaemonOutcome::RanAndStopped)
    );
}

fn add_local_destination(env: &Env, root: &Path, name: &str) -> Destination {
    let repo = root.join(format!("repo-{}", name.replace(' ', "-")));
    fs::create_dir_all(&repo).expect("mkdir repo");
    let dest = Destination::new(name, DestinationKind::Local { path: repo }).expect("dest");
    env.client
        .add_destination(&dest, "init", "m3-smoke-pw", "")
        .expect("init dest");
    dest
}

fn make_source(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(format!("src-{}", name.replace(' ', "-")));
    fs::create_dir_all(&dir).expect("mkdir source");
    fs::write(dir.join("note.txt"), name.as_bytes()).expect("write source file");
    dir
}

/// Backup as the «New backup» flow creates it (P35: daily 12:00, AC power + network).
fn add_default_backup(env: &Env, root: &Path, dest: &Destination, name: &str) -> BackupId {
    let mut b = Backup::new(
        name,
        Source::folder(make_source(root, name)),
        dest.id.clone(),
    )
    .expect("backup");
    b.schedule = Schedule::default_scheduled();
    BackupId::parse(env.client.add_backup(&b).expect("add backup")).expect("parse id")
}

fn no_conditions() -> RunConditions {
    RunConditions {
        require_ac_power: false,
        require_network: false,
        ..RunConditions::default()
    }
}

fn smoke_artifact(name: &str, layout: &str, lang: &str) -> String {
    format!("target/smoke-artifacts/m3-{name}-{layout}-{lang}.png")
}

fn saved_backup(env: &Env, id: &BackupId) -> Backup {
    load_config(&env.cfg_path)
        .expect("load config")
        .backups
        .into_iter()
        .find(|b| &b.id == id)
        .expect("saved backup present")
}

/// AC-M3-08-3: ok / waiting / overdue backups with status, reason and next slot.
#[test]
fn m3_08_3_overview_lists_ok_waiting_and_overdue_with_status_reason_and_next_slot() {
    run_isolated_test(
        "m3_08_3_overview_lists_ok_waiting_and_overdue_with_status_reason_and_next_slot",
        "KESTIC_M3_GUI_STATUS_ROWS_INNER",
        |root| {
            // On battery: the backup with default conditions is waiting for AC power.
            let env = start_env(
                root,
                true,
                &["HomeWifi"],
                datetime!(2026-12-01 10:00:00 UTC),
                false,
            );
            let dest = add_local_destination(&env, root, "Status Vault");
            let mut overdue = Backup::new(
                "Docs Overdue",
                Source::folder(make_source(root, "overdue")),
                dest.id.clone(),
            )
            .expect("overdue backup");
            overdue.schedule = Schedule::default_scheduled();
            overdue.run_conditions = no_conditions();
            let overdue_id = BackupId::parse(env.client.add_backup(&overdue).expect("add"))
                .expect("parse overdue id");

            env.clock.set(datetime!(2026-12-04 10:00:00 UTC));
            let mut ok = Backup::new(
                "Docs OK",
                Source::folder(make_source(root, "ok")),
                dest.id.clone(),
            )
            .expect("ok backup");
            ok.schedule = Schedule::default_scheduled();
            ok.run_conditions = no_conditions();
            let ok_id =
                BackupId::parse(env.client.add_backup(&ok).expect("add")).expect("parse ok id");
            let pending_id = add_default_backup(&env, root, &dest, "Docs Pending");

            env.clock.set(datetime!(2026-12-04 12:05:00 UTC));
            env.client
                .start_backup(ok_id.as_str())
                .expect("start Docs OK");
            wait_for_state(&env.client, Duration::from_secs(20), "Docs OK done", |s| {
                backup_summary(s, &ok_id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
                    && backup_summary(s, &ok_id).active_run.is_none()
            });
            let state = wait_for_state(&env.client, Duration::from_secs(5), "3 states", |s| {
                backup_summary(s, &overdue_id).is_overdue
                    && backup_summary(s, &pending_id)
                        .pending_primary_code
                        .map(|c| c.as_str())
                        == Some("on_battery")
            });
            assert!(
                !backup_summary(&state, &pending_id).is_overdue,
                "waiting backup must not be overdue"
            );
            assert!(
                !backup_summary(&state, &ok_id).is_overdue,
                "ok backup must not be overdue"
            );

            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m3_gui_smoke(
                    root,
                    "m3-overview-status-rows",
                    layout,
                    lang,
                    &smoke_artifact("overview-status", layout, lang),
                );
            }
            stop_env(env);
        },
    );
}

/// AC-M3-14-4 (global), AC-M3-08-6 (b): pause menu, localized pause line, resume button,
/// «Back Up All Now» message with the skipped backups.
#[test]
fn m3_14_4_08_6_global_pause_resume_button_and_back_up_all_skip_message() {
    run_isolated_test(
        "m3_14_4_08_6_global_pause_resume_button_and_back_up_all_skip_message",
        "KESTIC_M3_GUI_PAUSE_ALL_INNER",
        |root| {
            let start = datetime!(2026-12-01 10:00:00 UTC);
            let seed_clock = FakeClock::new(start);
            let dest = Destination::new(
                "Local Vault",
                DestinationKind::Local {
                    path: root.join("repo-local-vault"),
                },
            )
            .expect("dest");
            // Backup with a permanent error (Exit 12): seeded in the run history BEFORE the
            // daemon starts so it loads `has_error` cleanly; «Back Up All Now» must skip it.
            let locked = Backup::new(
                "Locked Docs",
                Source::folder(make_source(root, "locked")),
                dest.id.clone(),
            )
            .expect("locked");
            let mut err_run = Run::new_backup(&seed_clock, &locked).expect("err run");
            err_run.summary_message = Some("wrong password or no key found".to_string());
            err_run.finish(&seed_clock, RunStatus::Error, Some(12));
            {
                let mut history =
                    RunHistory::open(&root.join("state").join("kestic")).expect("open history");
                history.upsert_run(&err_run, &[]).expect("seed exit 12");
            }

            let env = start_env(root, false, &["HomeWifi"], start, false);
            fs::create_dir_all(root.join("repo-local-vault")).expect("mkdir repo");
            env.client
                .add_destination(&dest, "init", "m3-smoke-pw", "")
                .expect("init dest");
            let mut docs = Backup::new(
                "Documents",
                Source::folder(make_source(root, "documents")),
                dest.id.clone(),
            )
            .expect("docs");
            docs.schedule = Schedule::default_scheduled();
            docs.run_conditions = no_conditions();
            let docs_id = BackupId::parse(env.client.add_backup(&docs).expect("add docs"))
                .expect("parse docs id");
            let offline = Backup::new(
                "Offline Media",
                Source::folder(root.join("src-missing")),
                dest.id.clone(),
            )
            .expect("offline");
            env.client.add_backup(&offline).expect("add offline");
            env.client.add_backup(&locked).expect("add locked");
            let locked_id = locked.id.clone();
            wait_for_state(&env.client, Duration::from_secs(5), "locked error", |s| {
                backup_summary(s, &locked_id).has_error
            });

            for (layout, lang) in [("wide", "en"), ("wide", "de")] {
                run_m3_gui_smoke(
                    root,
                    "m3-overview-pause-and-start-all",
                    layout,
                    lang,
                    &smoke_artifact("overview-pause", layout, lang),
                );
                // (a) only the third backup (`Documents`) was started, the skipped ones were not.
                let state = wait_for_state(&env.client, Duration::from_secs(20), "idle", |s| {
                    backup_summary(s, &docs_id).active_run.is_none()
                        && backup_summary(s, &docs_id)
                            .last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                });
                let locked_run = backup_summary(&state, &locked_id).last_run.as_ref();
                assert_eq!(
                    locked_run.map(|r| r.id.clone()),
                    Some(err_run.id.clone()),
                    "(a) the permanent-error backup must not start a new run"
                );
            }
            stop_env(env);
        },
    );
}

/// AC-M3-08-5 (overview and detail) and the manual override of unmet run conditions.
#[test]
fn m3_08_5_exit12_replaces_back_up_now_in_overview_and_detail() {
    run_isolated_test(
        "m3_08_5_exit12_replaces_back_up_now_in_overview_and_detail",
        "KESTIC_M3_GUI_EXIT12_INNER",
        |root| {
            let clock_start = datetime!(2026-12-01 10:55:00 UTC);
            let clock = FakeClock::new(clock_start);
            let dest = Destination::new(
                "Locked Vault",
                DestinationKind::Local {
                    path: root.join("repo-locked"),
                },
            )
            .expect("dest");
            let mut b0 = Backup::new(
                "Encrypted Docs",
                Source::folder(make_source(root, "exit12")),
                dest.id.clone(),
            )
            .expect("b0");
            b0.schedule = Schedule::ManualOnly;
            let b0_id = b0.id.clone();
            // Seed the Exit-12 run BEFORE the daemon starts so it loads `has_error` cleanly.
            let mut err_run = Run::new_backup(&clock, &b0).expect("err run");
            err_run.summary_message = Some("wrong password or no key found".to_string());
            err_run.finish(&clock, RunStatus::Error, Some(12));
            {
                let mut history =
                    RunHistory::open(&root.join("state").join("kestic")).expect("open history");
                history.upsert_run(&err_run, &[]).expect("seed exit 12");
            }

            // On battery at 100%: Backup 1 (default conditions) is waiting for AC power.
            let env = start_env(root, true, &["HomeWifi"], clock_start, true);
            fs::create_dir_all(root.join("repo-locked")).expect("mkdir repo");
            env.client
                .add_destination(&dest, "init", "m3-smoke-pw", "")
                .expect("init dest");
            env.client.add_backup(&b0).expect("add b0");
            let mut b1 = Backup::new(
                "Battery Backup",
                Source::folder(make_source(root, "battery")),
                dest.id.clone(),
            )
            .expect("b1");
            b1.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            let b1_id =
                BackupId::parse(env.client.add_backup(&b1).expect("add b1")).expect("parse b1");
            // Past 12:00 CET (11:00 UTC) so b1 evaluates as waiting (`on_battery`).
            env.clock.set(datetime!(2026-12-01 11:05:00 UTC));
            wait_for_state(
                &env.client,
                Duration::from_secs(5),
                "exit12 + on_battery",
                |s| {
                    backup_summary(s, &b0_id).error_action.map(|c| c.as_str())
                        == Some("update_stored_password")
                        && backup_summary(s, &b1_id)
                            .pending_primary_code
                            .map(|c| c.as_str())
                            == Some("on_battery")
                },
            );

            run_m3_gui_smoke(
                root,
                "m3-exit12-and-condition-override",
                "wide",
                "en",
                &smoke_artifact("condition-override", "wide", "en"),
            );
            // The manual override really ran Backup 1.
            wait_for_state(&env.client, Duration::from_secs(20), "override ran", |s| {
                backup_summary(s, &b1_id).active_run.is_none()
                    && backup_summary(s, &b1_id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
            });
            stop_env(env);
        },
    );
}

/// One editor fixture: a single default backup (daily 12:00, AC power + network) on a vault
/// with the known networks `HomeWifi` and `OfficeWifi`.
/// Editor fixture on a REST destination (local `rest-server`): network run conditions only
/// apply to network destinations (decision 2026-11-20), so the run-condition editor tests use one.
fn network_editor_env(root: &Path) -> (Env, RestServerGuard, BackupId) {
    let env = start_env(
        root,
        false,
        &["HomeWifi", "OfficeWifi"],
        datetime!(2026-12-01 10:00:00 UTC),
        false,
    );
    let server = RestServerGuard::start_no_auth(&root.join("rest-data"));
    let dest = Destination::new(
        "Network Vault",
        DestinationKind::Rest {
            url: server.url.clone(),
            username: None,
        },
    )
    .expect("rest dest");
    env.client
        .add_destination(&dest, "init", "m3-smoke-pw", "")
        .expect("init rest dest");
    let id = add_default_backup(&env, root, &dest, "Documents");
    (env, server, id)
}

fn editor_env(root: &Path) -> (Env, Destination, BackupId) {
    let env = start_env(
        root,
        false,
        &["HomeWifi", "OfficeWifi"],
        datetime!(2026-12-01 10:00:00 UTC),
        false,
    );
    let dest = add_local_destination(&env, root, "Editor Vault");
    let id = add_default_backup(&env, root, &dest, "Documents");
    (env, dest, id)
}

/// AC-M3-14-1: the schedule editor of a new backup.
#[test]
fn m3_14_1_schedule_editor_preselects_daily_1200_and_offers_the_frequencies() {
    run_isolated_test(
        "m3_14_1_schedule_editor_preselects_daily_1200_and_offers_the_frequencies",
        "KESTIC_M3_GUI_SCHED_DEFAULTS_INNER",
        |root| {
            let (env, _dest, id) = editor_env(root);
            assert_eq!(
                saved_backup(&env, &id).schedule,
                Schedule::Daily {
                    hour: 12,
                    minute: 0
                },
                "a new backup starts with daily 12:00"
            );
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-schedule-editor-defaults",
                    "wide",
                    lang,
                    &smoke_artifact("schedule-defaults", "wide", lang),
                );
            }
            stop_env(env);
        },
    );
}

/// AC-M3-14-2 (no weekday / every 0 days) and AC-M3-14-6 (schedule editor).
#[test]
fn m3_14_2_14_6_schedule_editor_blocks_invalid_input_and_is_keyboard_operable() {
    run_isolated_test(
        "m3_14_2_14_6_schedule_editor_blocks_invalid_input_and_is_keyboard_operable",
        "KESTIC_M3_GUI_SCHED_VALIDATION_INNER",
        |root| {
            let (env, _dest, id) = editor_env(root);
            let before = fs::read(&env.cfg_path).expect("read config");
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-schedule-editor-validation",
                    "wide",
                    lang,
                    &smoke_artifact("schedule-validation", "wide", lang),
                );
            }
            assert_eq!(
                fs::read(&env.cfg_path).expect("read config"),
                before,
                "invalid or unsaved input must never reach config.toml"
            );
            assert!(matches!(
                saved_backup(&env, &id).schedule,
                Schedule::Daily { .. }
            ));
            stop_env(env);
        },
    );
}

/// AC-M3-14-8 (d) and AC-M3-14-9 for the schedule editor.
#[test]
fn m3_14_8_14_9_schedule_editor_asks_before_leaving_and_discard_keeps_config() {
    run_isolated_test(
        "m3_14_8_14_9_schedule_editor_asks_before_leaving_and_discard_keeps_config",
        "KESTIC_M3_GUI_SCHED_UNSAVED_INNER",
        |root| {
            let (env, _dest, id) = editor_env(root);
            run_m3_gui_smoke(
                root,
                "m3-schedule-editor-unsaved",
                "wide",
                "en",
                &smoke_artifact("schedule-unsaved", "wide", "en"),
            );
            // [Save] (the last step of the scenario) persisted «every 4 days».
            let saved = saved_backup(&env, &id);
            assert!(
                matches!(
                    saved.schedule,
                    Schedule::EveryNDays {
                        interval_days: 4,
                        hour: 12,
                        minute: 0,
                        ..
                    }
                ),
                "expected EveryNDays(4) persisted: {:?}",
                saved.schedule
            );
            // German pass with the same prompt (labels «Speichern» / «Verwerfen»).
            let mut reset = saved;
            reset.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            env.client.update_backup(&reset).expect("reset backup");
            run_m3_gui_smoke(
                root,
                "m3-schedule-editor-unsaved",
                "wide",
                "de",
                &smoke_artifact("schedule-unsaved", "wide", "de"),
            );
            assert!(matches!(
                saved_backup(&env, &id).schedule,
                Schedule::EveryNDays {
                    interval_days: 4,
                    ..
                }
            ));
            stop_env(env);
        },
    );
}

/// AC-M3-14-3 and AC-M3-14-6 (run-conditions editor of a new backup).
#[test]
fn m3_14_3_conditions_editor_of_new_backup_defaults_to_ac_power_and_network() {
    run_isolated_test(
        "m3_14_3_conditions_editor_of_new_backup_defaults_to_ac_power_and_network",
        "KESTIC_M3_GUI_COND_DEFAULTS_INNER",
        |root| {
            let (env, _server, id) = network_editor_env(root);
            assert_eq!(
                saved_backup(&env, &id).run_conditions,
                RunConditions::default(),
                "a new backup starts with the default run conditions"
            );
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-conditions-editor-defaults",
                    "wide",
                    lang,
                    &smoke_artifact("conditions-defaults", "wide", lang),
                );
            }
            stop_env(env);
        },
    );
}

/// M3 manual acceptance (2026-11-19, decision 2026-11-20): on a local destination the
/// run-condition editor shows no network conditions, only a hint, and is not dirty.
#[test]
fn m3_acceptance_conditions_editor_hides_network_for_local_destination() {
    run_isolated_test(
        "m3_acceptance_conditions_editor_hides_network_for_local_destination",
        "KESTIC_M3_GUI_COND_LOCAL_INNER",
        |root| {
            let (env, _dest, _id) = editor_env(root);
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-conditions-editor-local",
                    "wide",
                    lang,
                    &smoke_artifact("conditions-local", "wide", lang),
                );
            }
            stop_env(env);
        },
    );
}

/// AC-M3-14-9 for the run-conditions editor.
#[test]
fn m3_14_9_conditions_editor_asks_before_leaving_and_discard_keeps_config() {
    run_isolated_test(
        "m3_14_9_conditions_editor_asks_before_leaving_and_discard_keeps_config",
        "KESTIC_M3_GUI_COND_UNSAVED_INNER",
        |root| {
            let (env, _server, id) = network_editor_env(root);
            run_m3_gui_smoke(
                root,
                "m3-conditions-editor-unsaved",
                "wide",
                "en",
                &smoke_artifact("conditions-unsaved", "wide", "en"),
            );
            let saved = saved_backup(&env, &id);
            assert!(
                saved.run_conditions.skip_on_metered_network,
                "expected skip_on_metered_network=true persisted"
            );
            assert_eq!(
                saved.run_conditions.allowed_networks,
                vec!["HomeWifi".to_string()],
                "expected HomeWifi persisted"
            );
            let mut reset = saved;
            reset.run_conditions = RunConditions::default();
            env.client.update_backup(&reset).expect("reset backup");
            run_m3_gui_smoke(
                root,
                "m3-conditions-editor-unsaved",
                "wide",
                "de",
                &smoke_artifact("conditions-unsaved", "wide", "de"),
            );
            assert!(
                saved_backup(&env, &id)
                    .run_conditions
                    .skip_on_metered_network
            );
            stop_env(env);
        },
    );
}

/// AC-M3-14-4 (per backup) and AC-M3-14-6 (pause menu operated by keyboard only).
#[test]
fn m3_14_4_14_6_backup_pause_menu_offers_four_choices_and_works_by_keyboard() {
    run_isolated_test(
        "m3_14_4_14_6_backup_pause_menu_offers_four_choices_and_works_by_keyboard",
        "KESTIC_M3_GUI_PAUSE_MENU_INNER",
        |root| {
            let (env, _dest, _id) = editor_env(root);
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-pause-menu-keyboard",
                    "wide",
                    lang,
                    &smoke_artifact("pause-menu", "wide", lang),
                );
            }
            stop_env(env);
        },
    );
}

/// AC-M3-14-5, AC-M3-05-7 (a) and AC-M3-14-6 (maintenance editor).
#[test]
fn m3_14_5_05_7_maintenance_editor_shows_prune_preview_before_confirming() {
    run_isolated_test(
        "m3_14_5_05_7_maintenance_editor_shows_prune_preview_before_confirming",
        "KESTIC_M3_GUI_MAINT_INNER",
        |root| {
            let (env, dest, _id) = editor_env(root);
            let before_check = load_config(&env.cfg_path)
                .expect("load config")
                .destinations[0]
                .maintenance
                .check
                .clone();
            run_m3_gui_smoke(
                root,
                "m3-maintenance-editor",
                "wide",
                "en",
                &smoke_artifact("prune-confirm", "wide", "en"),
            );
            let saved = load_config(&env.cfg_path).expect("load config");
            let saved_dest = saved
                .destinations
                .iter()
                .find(|d| d.id == dest.id)
                .expect("saved dest");
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
            // German pass on the default schedule.
            env.client
                .update_destination_maintenance(&dest.id.to_string(), &Default::default())
                .expect("reset maintenance");
            run_m3_gui_smoke(
                root,
                "m3-maintenance-editor",
                "wide",
                "de",
                &smoke_artifact("prune-confirm", "wide", "de"),
            );
            stop_env(env);
        },
    );
}

/// Settings dialog: monitoring and notification controls persist.
#[test]
fn m3_settings_monitoring_and_notification_controls_persist() {
    run_isolated_test(
        "m3_settings_monitoring_and_notification_controls_persist",
        "KESTIC_M3_GUI_SETTINGS_INNER",
        |root| {
            let (env, _dest, _id) = editor_env(root);
            run_m3_gui_smoke(
                root,
                "m3-settings-monitoring",
                "wide",
                "en",
                &smoke_artifact("settings", "wide", "en"),
            );
            let saved = load_config(&env.cfg_path).expect("load config");
            assert_eq!(saved.default_overdue_threshold, 4);
            assert!(!saved.notifications.on_overdue);
            stop_env(env);
        },
    );
}

#[path = "../../i18n/tests/common/language.rs"]
mod language;

/// Every page, editor, wizard and the settings window, opened in English and in German: (a) the
/// German window shows no English text that has a German translation, (b) every visible text in
/// both languages has short sentences and (c) no technical words. The visible texts per page are
/// kept as review lists (`target/smoke-artifacts/m3-all-pages-texts-<lang>.tsv`). The smoke also
/// checks that the backups list shows exactly the rows of the Rust list model (AC-M0-04-1).
#[test]
fn every_page_shows_simple_texts_in_the_gui_language() {
    run_isolated_test(
        "every_page_shows_simple_texts_in_the_gui_language",
        "KESTIC_M3_GUI_ALL_PAGES_INNER",
        |root| {
            let (env, _dest, _id) = editor_env(root);
            let catalog = language::catalog(&common::workspace_root());
            let english_with_translation: std::collections::HashSet<&str> = catalog
                .iter()
                .filter(|e| !e.msgid().is_empty())
                .filter(|e| e.german().iter().any(|de| !e.english().contains(de)))
                .flat_map(|e| e.english().iter().map(String::as_str))
                .collect();
            for lang in ["en", "de"] {
                let dump = root.join(format!("texts-{lang}.tsv"));
                run_m3_gui_smoke_with_env(
                    root,
                    "m3-all-pages-texts",
                    "wide",
                    lang,
                    &smoke_artifact("all-pages", "wide", lang),
                    &[("KESTIC_SMOKE_TEXT_DUMP", dump.as_path())],
                );
                let raw = fs::read_to_string(&dump).expect("read text dump");
                fs::copy(
                    &dump,
                    common::workspace_root().join(format!(
                        "target/smoke-artifacts/m3-all-pages-texts-{lang}.tsv"
                    )),
                )
                .expect("keep text dump as artifact");
                let texts: Vec<(&str, String)> = raw
                    .lines()
                    .filter_map(|l| l.split_once('\t'))
                    .map(|(page, t)| (page, t.replace("\\n", "\n")))
                    .collect();
                let pages: std::collections::HashSet<&str> =
                    texts.iter().map(|(p, _)| *p).collect();
                assert_eq!(pages.len(), 19, "{lang}: every page recorded: {pages:?}");
                if lang == "de" {
                    let english: Vec<_> = texts
                        .iter()
                        .filter(|(_, t)| english_with_translation.contains(t.as_str()))
                        .collect();
                    assert!(
                        english.is_empty(),
                        "(a) English texts in the German window: {english:#?}"
                    );
                }
                let long: Vec<_> = texts
                    .iter()
                    .filter(|(_, t)| !language::long_sentences(t).is_empty())
                    .collect();
                assert!(
                    long.is_empty(),
                    "(b) {lang}: sentences longer than {} words: {long:#?}",
                    language::MAX_WORDS_PER_SENTENCE
                );
                let jargon: Vec<_> = texts
                    .iter()
                    .filter(|(_, t)| !language::jargon(t).is_empty())
                    .collect();
                assert!(
                    jargon.is_empty(),
                    "(c) {lang}: technical words: {jargon:#?}"
                );
            }
            stop_env(env);
        },
    );
}
