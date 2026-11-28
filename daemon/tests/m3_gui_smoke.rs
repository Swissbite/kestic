// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Comprehensive offscreen GUI smoke suite for Milestone M3 (`M3-S01`–`M3-S14`).
//!
//! Each test runs isolated via [`common::run_isolated_test`] with its own private
//! `dbus-daemon` with a headless keyring, `XDG_*` directories, injectable D-Bus system
//! fakes (`FakeUPowerHandle`, `FakeNetworkManagerHandle`), `FakeClock`, and `kestic --smoke-m1`,
//! verifying UI state, keyboard navigation, accessibility labels, screenshots in EN/DE and
//! wide/narrow layouts, and persisted `config.toml` / daemon behavior.

mod common;

use common::revision::UpdateNow;

use common::load_config;
use common::m3::{
    FakeSetup, M3Rig, StdFakes, backup_summary, connect_local_repo, connect_rest_repo,
    latest_variant, wait_for_state,
};
use common::{find_kestic_binary, run_isolated_test};
use kestic_core::clock::FakeClock;
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, BackupId, CheckSummary, Destination, DestinationKind, MaintenanceEntrySchedule,
    MaintenanceForget, MaintenanceSchedule, MaintenanceStats, Run, RunConditions, RunId, RunStatus,
    Schedule, Source,
};
use kestic_daemon::dbus_api::{KESTIC_BUS_NAME, RunProgressPhase};
use kestic_test_support::tools::RestServerGuard;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use time::macros::datetime;

fn run_m3_gui_smoke(root: &Path, scenario: &str, layout: &str, lang: &str, screenshot_rel: &str) {
    run_m3_gui_smoke_with_env(root, scenario, layout, lang, screenshot_rel, &[]);
}

fn run_m3_gui_smoke_with_env(
    root: &Path,
    scenario: &str,
    layout: &str,
    lang: &str,
    screenshot_rel: &str,
    extra_env: &[(&str, &str)],
) {
    let iso = kestic_test_support::gui::IsolatedEnv::adopt(root);
    let dbus_addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("isolated session bus");
    kestic_test_support::gui::SmokeContext {
        kestic_bin: &find_kestic_binary(),
        iso: &iso,
        dbus_addr: &dbus_addr,
    }
    .run_with_env(scenario, layout, lang, screenshot_rel, &[], extra_env);
}

/// Notification, StatusNotifierWatcher, UPower and NetworkManager fakes (active network
/// `HomeWifi`) every GUI smoke daemon needs on the session bus.
fn gui_fakes(on_battery: bool, known_networks: &[&str]) -> StdFakes {
    StdFakes::start_with(FakeSetup {
        on_battery,
        active_network: Some("HomeWifi"),
        known_networks,
        ..FakeSetup::default()
    })
}

/// Daemon on the real bus name the GUI process connects to, in the sandbox `root` itself.
fn start_rig(root: &Path, start: time::OffsetDateTime, enable_scheduler: bool) -> M3Rig {
    M3Rig::start_on(root, &latest_variant(), "", KESTIC_BUS_NAME, start, |o| {
        o.enable_scheduler = enable_scheduler
    })
}

fn make_source(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(format!("src-{}", name.replace(' ', "-")));
    fs::create_dir_all(&dir).expect("mkdir source");
    fs::write(dir.join("note.txt"), name.as_bytes()).expect("write source file");
    dir
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

fn saved_backup(rig: &M3Rig, id: &BackupId) -> Backup {
    load_config(&rig.daemon.config_path)
        .expect("load config")
        .backups
        .into_iter()
        .find(|b| &b.id == id)
        .expect("saved backup present")
}

/// Overview with three backups on one destination: «Docs OK» backed up, «Docs Pending» waiting
/// for AC power (on battery) and «Docs Overdue» overdue. Keep the fakes alive with the rig.
fn status_rows_env(root: &Path) -> (M3Rig, StdFakes) {
    let fakes = gui_fakes(true, &["HomeWifi"]);
    let rig = start_rig(root, datetime!(2026-12-01 10:00:00 UTC), false);
    let dest = rig.add_destination("Status Vault", |_| {});
    let overdue_id = rig
        .add_backup_to(&dest, "Docs Overdue", |b| {
            b.run_conditions = no_conditions()
        })
        .id;

    rig.clock.set(datetime!(2026-12-04 10:00:00 UTC));
    let ok_id = rig
        .add_backup_to(&dest, "Docs OK", |b| b.run_conditions = no_conditions())
        .id;
    let pending_id = rig.add_backup_to(&dest, "Docs Pending", |_| {}).id;

    rig.clock.set(datetime!(2026-12-04 12:05:00 UTC));
    rig.daemon
        .client
        .start_backup(ok_id.as_str())
        .expect("start Docs OK");
    wait_for_state(
        &rig.daemon.client,
        Duration::from_secs(20),
        "Docs OK done",
        |s| {
            backup_summary(s, &ok_id)
                .last_run
                .as_ref()
                .is_some_and(|r| r.status == RunStatus::Ok)
                && backup_summary(s, &ok_id).active_run.is_none()
        },
    );
    let state = wait_for_state(
        &rig.daemon.client,
        Duration::from_secs(5),
        "3 states",
        |s| {
            backup_summary(s, &overdue_id).is_overdue
                && backup_summary(s, &pending_id)
                    .pending_primary_code
                    .map(|c| c.as_str())
                    == Some("on_battery")
        },
    );
    assert!(
        !backup_summary(&state, &pending_id).is_overdue,
        "waiting backup must not be overdue"
    );
    assert!(
        !backup_summary(&state, &ok_id).is_overdue,
        "ok backup must not be overdue"
    );
    (rig, fakes)
}

/// AC-M3-08-3: ok / waiting / overdue backups with status, reason and next slot.
#[test]
fn m3_08_3_overview_lists_ok_waiting_and_overdue_with_status_reason_and_next_slot() {
    run_isolated_test(
        "m3_08_3_overview_lists_ok_waiting_and_overdue_with_status_reason_and_next_slot",
        "KESTIC_M3_GUI_STATUS_ROWS_INNER",
        |root| {
            let (rig, _fakes) = status_rows_env(root);
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m3_gui_smoke(
                    root,
                    "m3-overview-status-rows",
                    layout,
                    lang,
                    &smoke_artifact("overview-status", layout, lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// Overview of a running backup (`docs/spec/ui/hauptbereiche.md`, restic-integration
/// «Fortschritt beim Sichern»): while restic still finds files, (a) the card shows a bar without
/// end point and the amount looked through and (b) the headline names no percentage; after the
/// search (c) the card shows percentage, amount and time left and (d) the headline the
/// percentage. The C++ scenarios check each text; screenshots in English and German.
#[test]
fn overview_shows_no_percentage_while_scanning_and_percentage_afterwards() {
    run_isolated_test(
        "overview_shows_no_percentage_while_scanning_and_percentage_afterwards",
        "KESTIC_M3_GUI_PROGRESS_INNER",
        |root| {
            let _fakes = gui_fakes(false, &["HomeWifi"]);
            let rig = start_rig(root, datetime!(2026-12-01 10:00:00 UTC), false);
            let wrapper = &rig.wrapper;
            let dest = rig.add_destination("Progress Vault", |_| {});
            let docs_id = rig
                .add_backup_to(&dest, "Docs", |b| b.run_conditions = no_conditions())
                .id;

            let scanning_line = concat!(
                r#"{"message_type":"status","percent_done":0.99,"total_files":4000,"#,
                r#""files_done":3500,"total_bytes":5000000000,"bytes_done":4900000000}"#,
                "\n",
            );
            for (phase, scenario, name) in [
                (
                    RunProgressPhase::Scanning,
                    "m3-overview-progress-scanning",
                    "overview-progress-scanning",
                ),
                (
                    RunProgressPhase::ScanFinished,
                    "m3-overview-progress-percent",
                    "overview-progress-percent",
                ),
            ] {
                if phase == RunProgressPhase::Scanning {
                    wrapper.hold_with_output("backup", scanning_line);
                } else {
                    wrapper.hold("backup");
                }
                rig.daemon
                    .client
                    .start_backup(docs_id.as_str())
                    .expect("start Docs");
                wait_for_state(&rig.daemon.client, Duration::from_secs(20), name, |s| {
                    s.overall
                        .active_run_progress
                        .as_ref()
                        .is_some_and(|p| p.phase == phase && p.bytes_done > 0)
                });
                for lang in ["en", "de"] {
                    run_m3_gui_smoke(
                        root,
                        scenario,
                        "wide",
                        lang,
                        &smoke_artifact(name, "wide", lang),
                    );
                }
                wrapper.release("backup");
                wait_for_state(
                    &rig.daemon.client,
                    Duration::from_secs(30),
                    "run done",
                    |s| backup_summary(s, &docs_id).active_run.is_none(),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// Overview with a seeded permanent-error backup (Exit 12), an unavailable-source backup and a
/// startable backup; «Back Up All Now» must skip the first two.
struct PauseAllFixture {
    rig: M3Rig,
    _fakes: StdFakes,
    docs_id: BackupId,
    locked_id: BackupId,
    locked_run_id: RunId,
}

fn pause_all_fixture(root: &Path) -> PauseAllFixture {
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

    let fakes = gui_fakes(false, &["HomeWifi"]);
    let rig = start_rig(root, start, false);
    connect_local_repo(&rig.daemon.client, &rig.variant.bin, &dest);
    let docs_id = rig
        .add_backup_to(&dest, "Documents", |b| b.run_conditions = no_conditions())
        .id;
    let offline = Backup::new(
        "Offline Media",
        Source::folder(root.join("src-missing")),
        dest.id.clone(),
    )
    .expect("offline");
    rig.daemon.client.add_backup(&offline).expect("add offline");
    rig.daemon.client.add_backup(&locked).expect("add locked");
    let locked_id = locked.id.clone();
    wait_for_state(
        &rig.daemon.client,
        Duration::from_secs(5),
        "locked error",
        |s| backup_summary(s, &locked_id).has_error,
    );

    PauseAllFixture {
        rig,
        _fakes: fakes,
        docs_id,
        locked_id,
        locked_run_id: err_run.id.clone(),
    }
}

/// AC-M3-14-4 (global): pause menu with four choices, localized pause line and resume button
/// (checked by the `m3-overview-pause` scenario).
#[test]
fn m3_14_4_global_pause_menu_pause_line_and_resume_button() {
    run_isolated_test(
        "m3_14_4_global_pause_menu_pause_line_and_resume_button",
        "KESTIC_M3_GUI_PAUSE_ALL_INNER",
        |root| {
            let fixture = pause_all_fixture(root);
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-overview-pause",
                    "wide",
                    lang,
                    &smoke_artifact("overview-pause", "wide", lang),
                );
            }
            fixture.rig.daemon.stop();
        },
    );
}

/// AC-M3-08-6 (b): «Back Up All Now» starts only the startable backup and names the skipped ones.
#[test]
fn m3_08_6_back_up_all_skips_error_and_unavailable_backups() {
    run_isolated_test(
        "m3_08_6_back_up_all_skips_error_and_unavailable_backups",
        "KESTIC_M3_GUI_START_ALL_INNER",
        |root| {
            let fixture = pause_all_fixture(root);
            let client = &fixture.rig.daemon.client;
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-overview-start-all",
                    "wide",
                    lang,
                    &smoke_artifact("overview-start-all", "wide", lang),
                );
                // (a) only the third backup (`Documents`) was started, the skipped ones were not.
                let state = wait_for_state(client, Duration::from_secs(20), "idle", |s| {
                    backup_summary(s, &fixture.docs_id).active_run.is_none()
                        && backup_summary(s, &fixture.docs_id)
                            .last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                });
                let locked_run = backup_summary(&state, &fixture.locked_id).last_run.as_ref();
                assert_eq!(
                    locked_run.map(|r| r.id.clone()),
                    Some(fixture.locked_run_id.clone()),
                    "(a) the permanent-error backup must not start a new run"
                );
            }
            fixture.rig.daemon.stop();
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
            let _fakes = gui_fakes(true, &["HomeWifi"]);
            let rig = start_rig(root, clock_start, true);
            connect_local_repo(&rig.daemon.client, &rig.variant.bin, &dest);
            rig.daemon.client.add_backup(&b0).expect("add b0");
            let b1_id = rig.add_backup_to(&dest, "Battery Backup", |_| {}).id;
            // Past 12:00 CET (11:00 UTC) so b1 evaluates as waiting (`on_battery`).
            rig.clock.set(datetime!(2026-12-01 11:05:00 UTC));
            wait_for_state(
                &rig.daemon.client,
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
            wait_for_state(
                &rig.daemon.client,
                Duration::from_secs(20),
                "override ran",
                |s| {
                    backup_summary(s, &b1_id).active_run.is_none()
                        && backup_summary(s, &b1_id)
                            .last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                },
            );
            rig.daemon.stop();
        },
    );
}

/// One editor fixture: a single default backup (daily 12:00, AC power + network) on a vault
/// with the known networks `HomeWifi` and `OfficeWifi`.
/// Editor fixture on a REST destination (local `rest-server`): network run conditions only
/// apply to network destinations, so the run-condition editor tests use one.
fn network_editor_env(root: &Path) -> (M3Rig, StdFakes, RestServerGuard, BackupId) {
    let fakes = gui_fakes(false, &["HomeWifi", "OfficeWifi"]);
    let rig = start_rig(root, datetime!(2026-12-01 10:00:00 UTC), false);
    let server = RestServerGuard::start_no_auth(&root.join("rest-data"));
    let dest = Destination::new(
        "Network Vault",
        DestinationKind::Rest {
            url: server.url.clone(),
            username: None,
        },
    )
    .expect("rest dest");
    connect_rest_repo(&rig.daemon.client, &rig.variant.bin, &server, &dest);
    let id = rig.add_backup_to(&dest, "Documents", |_| {}).id;
    (rig, fakes, server, id)
}

fn editor_env(root: &Path) -> (M3Rig, StdFakes, Destination, BackupId) {
    let fakes = gui_fakes(false, &["HomeWifi", "OfficeWifi"]);
    let rig = start_rig(root, datetime!(2026-12-01 10:00:00 UTC), false);
    let dest = rig.add_destination("Editor Vault", |_| {});
    let id = rig.add_backup_to(&dest, "Documents", |_| {}).id;
    (rig, fakes, dest, id)
}

/// AC-M3-14-1: the schedule editor of a new backup.
#[test]
fn m3_14_1_schedule_editor_preselects_daily_1200_and_offers_the_frequencies() {
    run_isolated_test(
        "m3_14_1_schedule_editor_preselects_daily_1200_and_offers_the_frequencies",
        "KESTIC_M3_GUI_SCHED_DEFAULTS_INNER",
        |root| {
            let (rig, _fakes, _dest, id) = editor_env(root);
            assert_eq!(
                saved_backup(&rig, &id).schedule,
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
            rig.daemon.stop();
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
            let (rig, _fakes, _dest, id) = editor_env(root);
            let before = fs::read(&rig.daemon.config_path).expect("read config");
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
                fs::read(&rig.daemon.config_path).expect("read config"),
                before,
                "invalid or unsaved input must never reach config.toml"
            );
            assert!(matches!(
                saved_backup(&rig, &id).schedule,
                Schedule::Daily { .. }
            ));
            rig.daemon.stop();
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
            let (rig, _fakes, _dest, id) = editor_env(root);
            run_m3_gui_smoke(
                root,
                "m3-schedule-editor-unsaved",
                "wide",
                "en",
                &smoke_artifact("schedule-unsaved", "wide", "en"),
            );
            // [Save] (the last step of the scenario) persisted «every 4 days».
            let saved = saved_backup(&rig, &id);
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
            rig.daemon
                .client
                .update_backup_now(&reset)
                .expect("reset backup");
            run_m3_gui_smoke(
                root,
                "m3-schedule-editor-unsaved",
                "wide",
                "de",
                &smoke_artifact("schedule-unsaved", "wide", "de"),
            );
            assert!(matches!(
                saved_backup(&rig, &id).schedule,
                Schedule::EveryNDays {
                    interval_days: 4,
                    ..
                }
            ));
            rig.daemon.stop();
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
            let (rig, _fakes, _server, id) = network_editor_env(root);
            assert_eq!(
                saved_backup(&rig, &id).run_conditions,
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
            rig.daemon.stop();
        },
    );
}

/// On a local destination the run-condition editor shows no network conditions, only a hint,
/// and is not dirty.
#[test]
fn m3_acceptance_conditions_editor_hides_network_for_local_destination() {
    run_isolated_test(
        "m3_acceptance_conditions_editor_hides_network_for_local_destination",
        "KESTIC_M3_GUI_COND_LOCAL_INNER",
        |root| {
            let (rig, _fakes, _dest, _id) = editor_env(root);
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-conditions-editor-local",
                    "wide",
                    lang,
                    &smoke_artifact("conditions-local", "wide", lang),
                );
            }
            rig.daemon.stop();
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
            let (rig, _fakes, _server, id) = network_editor_env(root);
            run_m3_gui_smoke(
                root,
                "m3-conditions-editor-unsaved",
                "wide",
                "en",
                &smoke_artifact("conditions-unsaved", "wide", "en"),
            );
            let saved = saved_backup(&rig, &id);
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
            rig.daemon
                .client
                .update_backup_now(&reset)
                .expect("reset backup");
            run_m3_gui_smoke(
                root,
                "m3-conditions-editor-unsaved",
                "wide",
                "de",
                &smoke_artifact("conditions-unsaved", "wide", "de"),
            );
            assert!(
                saved_backup(&rig, &id)
                    .run_conditions
                    .skip_on_metered_network
            );
            rig.daemon.stop();
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
            let (rig, _fakes, _dest, _id) = editor_env(root);
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-pause-menu-keyboard",
                    "wide",
                    lang,
                    &smoke_artifact("pause-menu", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

fn saved_maintenance_of(rig: &M3Rig, dest: &Destination) -> MaintenanceSchedule {
    load_config(&rig.daemon.config_path)
        .expect("load config")
        .destinations
        .into_iter()
        .find(|d| d.id == dest.id)
        .expect("saved dest")
        .maintenance
}

/// AC-M3-14-5, AC-M3-05-7 (a): the maintenance editor shows the prune preview before confirming;
/// the confirmed prune schedule is persisted.
#[test]
fn m3_14_5_05_7_maintenance_editor_shows_prune_preview_before_confirming() {
    run_isolated_test(
        "m3_14_5_05_7_maintenance_editor_shows_prune_preview_before_confirming",
        "KESTIC_M3_GUI_MAINT_INNER",
        |root| {
            let (rig, _fakes, dest, _id) = editor_env(root);
            run_m3_gui_smoke(
                root,
                "m3-maintenance-editor-prune-confirm",
                "wide",
                "en",
                &smoke_artifact("prune-confirm", "wide", "en"),
            );
            let saved = saved_maintenance_of(&rig, &dest);
            assert!(
                matches!(saved.prune, MaintenanceEntrySchedule::Daily { .. }),
                "expected confirmed prune schedule Daily persisted: {:?}",
                saved.prune
            );
            // German pass on the default schedule.
            rig.daemon
                .client
                .update_destination_maintenance(&dest.id.to_string(), &Default::default())
                .expect("reset maintenance");
            run_m3_gui_smoke(
                root,
                "m3-maintenance-editor-prune-confirm",
                "wide",
                "de",
                &smoke_artifact("prune-confirm", "wide", "de"),
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M3-14-6 (maintenance editor): (a) every control is reachable by keyboard and (b) has an
/// accessible name (checked by the `m3-maintenance-editor-discard` scenario); (c) a discarded
/// check-frequency and clean-up change is not persisted.
#[test]
fn m3_14_6_maintenance_editor_keyboard_names_and_discarded_changes_not_persisted() {
    run_isolated_test(
        "m3_14_6_maintenance_editor_keyboard_names_and_discarded_changes_not_persisted",
        "KESTIC_M3_GUI_MAINT_DISCARD_INNER",
        |root| {
            let (rig, _fakes, dest, _id) = editor_env(root);
            let before = saved_maintenance_of(&rig, &dest);
            run_m3_gui_smoke(
                root,
                "m3-maintenance-editor-discard",
                "wide",
                "en",
                &smoke_artifact("prune-discard", "wide", "en"),
            );
            let after = saved_maintenance_of(&rig, &dest);
            assert_eq!(
                after.check, before.check,
                "(c) discarded check frequency change must NOT be persisted"
            );
            assert_eq!(
                after.prune, before.prune,
                "(c) discarded clean-up change must NOT be persisted"
            );
            rig.daemon.stop();
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
            let (rig, _fakes, _dest, _id) = editor_env(root);
            run_m3_gui_smoke(
                root,
                "m3-settings-monitoring",
                "wide",
                "en",
                &smoke_artifact("settings", "wide", "en"),
            );
            let saved = load_config(&rig.daemon.config_path).expect("load config");
            assert_eq!(saved.default_overdue_threshold, 4);
            assert!(!saved.notifications.on_overdue);
            rig.daemon.stop();
        },
    );
}

/// AC-M0-11-8: Ctrl+Shift+B in the backup detail page starts "Back Up Now" for the shown
/// backup, and the action shows the shortcut (a real key press in the product window). On the
/// overview, where no backup is open, it starts nothing.
#[test]
fn ac_m0_11_8_ctrl_shift_b_starts_back_up_now_in_the_detail_page() {
    run_isolated_test(
        "ac_m0_11_8_ctrl_shift_b_starts_back_up_now_in_the_detail_page",
        "KESTIC_M3_GUI_BACKUP_NOW_SHORTCUT_INNER",
        |root| {
            let (rig, _fakes, _dest, _id) = editor_env(root);
            run_m3_gui_smoke(
                root,
                "m3-backup-now-shortcut",
                "wide",
                "en",
                &smoke_artifact("backup-now-shortcut", "wide", "en"),
            );
            rig.daemon.stop();
        },
    );
}

#[path = "../../i18n/tests/common/language.rs"]
mod language;

/// Every page, editor, wizard, the settings window and «About Kestic», opened in English and in
/// German: (a) the German window shows no English text that has a German translation, (b) every
/// visible text in both languages has short sentences and (c) no technical words. The visible texts per page are
/// kept as review lists (`target/smoke-artifacts/m3-all-pages-texts-<lang>.tsv`). The smoke also
/// checks that the backups list shows exactly the rows of the Rust list model (AC-M0-04-1).
#[test]
fn every_page_shows_simple_texts_in_the_gui_language() {
    run_isolated_test(
        "every_page_shows_simple_texts_in_the_gui_language",
        "KESTIC_M3_GUI_ALL_PAGES_INNER",
        |root| {
            let (rig, _fakes, _dest, _id) = editor_env(root);
            let catalog =
                kestic_test_support::po::catalog(&kestic_test_support::workspace::workspace_root());
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
                    &[("KESTIC_SMOKE_TEXT_DUMP", dump.to_str().expect("utf-8 path"))],
                );
                let raw = fs::read_to_string(&dump).expect("read text dump");
                fs::copy(
                    &dump,
                    kestic_test_support::workspace::workspace_root().join(format!(
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
                assert_eq!(pages.len(), 21, "{lang}: every page recorded: {pages:?}");
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
            rig.daemon.stop();
        },
    );
}

/// Leaving a dirty editor asks first on every path: (a) the breadcrumb while the exclude-rule
/// preview sits on top of the editor, (b) [Save] in that prompt saves the editor below the
/// preview, (c) closing the window.
#[test]
fn m3_editor_guard_covers_pages_above_the_editor_and_window_close() {
    run_isolated_test(
        "m3_editor_guard_covers_pages_above_the_editor_and_window_close",
        "KESTIC_M3_GUI_EDITOR_GUARD_INNER",
        |root| {
            let (rig, _fakes, _dest, id) = editor_env(root);
            run_m3_gui_smoke(
                root,
                "m3-editor-guard-paths",
                "wide",
                "en",
                &smoke_artifact("editor-guard-paths", "wide", "en"),
            );
            assert!(
                saved_backup(&rig, &id).excludes.exclude_caches,
                "(b) [Save] in the prompt persisted the edited exclude rules"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M3-05-9: the run detail page of a maintenance run shows (a) the title «Maintenance on …»,
/// (b) the status line «Cleaned up and verified · …», (c) the destination below it, and in the
/// «Result» tab (d) the removed snapshots per backup, (e) the freed space and (f) the check
/// result; the C++ scenario checks each statement. Screenshots in English and German, German
/// also narrow.
#[test]
fn ac_m3_05_9_maintenance_run_detail_shows_steps_freed_space_and_check_result() {
    run_isolated_test(
        "ac_m3_05_9_maintenance_run_detail_shows_steps_freed_space_and_check_result",
        "KESTIC_AC_M3_05_9_INNER",
        |root| {
            let start = datetime!(2026-11-21 15:25:00 UTC);
            let seed_clock = FakeClock::new(start);
            let dest = Destination::new(
                "Restic Synology",
                DestinationKind::Local {
                    path: root.join("repo-synology"),
                },
            )
            .expect("dest");
            let docs = Backup::new(
                "Documents",
                Source::folder(make_source(root, "docs")),
                dest.id.clone(),
            )
            .expect("backup");
            let mut run = Run::new_maintenance(&seed_clock, dest.id.clone()).expect("run");
            run.maintenance = Some(MaintenanceStats {
                forgets: vec![MaintenanceForget {
                    backup_id: docs.id.clone(),
                    removed_snapshots: Some(2),
                }],
                pruned: true,
                freed_bytes: Some(1_262_485),
                checked: true,
                check: Some(CheckSummary::default()),
            });
            seed_clock.advance(Duration::from_secs(6));
            run.finish(&seed_clock, RunStatus::Ok, Some(0));
            RunHistory::open(&root.join("state").join("kestic"))
                .expect("open history")
                .upsert_run(&run, &[])
                .expect("seed maintenance run");

            let _fakes = gui_fakes(false, &["HomeWifi"]);
            let rig = start_rig(root, start + Duration::from_secs(60), false);
            connect_local_repo(&rig.daemon.client, &rig.variant.bin, &dest);
            rig.daemon.client.add_backup(&docs).expect("add backup");
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "de")] {
                run_m3_gui_smoke(
                    root,
                    "m3-maintenance-run-detail",
                    layout,
                    lang,
                    &smoke_artifact("maintenance-run-detail", layout, lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// With one backup on one destination the list headers use the
/// singular («1 Backup», «1 Destination» / «1 Ziel»), and the hint why the destination cannot
/// be removed is the tooltip of the disabled «Remove Destination…» action, not a line in the page
/// body. The C++ scenario checks each text in English and German.
#[test]
fn finding2_3_singular_list_headers_and_remove_hint_only_on_the_action() {
    run_isolated_test(
        "finding2_3_singular_list_headers_and_remove_hint_only_on_the_action",
        "KESTIC_FINDING2_3_INNER",
        |root| {
            let (rig, _fakes, _dest, _id) = editor_env(root);
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-singular-counts-and-remove-hint",
                    "wide",
                    lang,
                    &smoke_artifact("singular-counts-remove-hint", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// The destination detail page shows «Maintenance running» while a scheduled maintenance run is
/// active (its `prune` is held) and resets it after the run has ended, without any action in the
/// GUI.
#[test]
fn finding4_destination_page_resets_maintenance_running_after_the_run() {
    run_isolated_test(
        "finding4_destination_page_resets_maintenance_running_after_the_run",
        "KESTIC_FINDING4_INNER",
        |root| {
            // Friday 2026-12-04 11:59:30 and Sunday 2026-12-06 12:00:05 Europe/Zurich: prune
            // (every Sunday) and check (first Sunday) are due on Sunday.
            let friday = datetime!(2026-12-04 10:59:30 UTC);
            let sunday_1200 = datetime!(2026-12-06 11:00:05 UTC);
            let _fakes = gui_fakes(false, &["HomeWifi"]);
            let rig = start_rig(root, friday, true);
            let dest = rig.add_destination("Status Vault", |_| {});
            std::thread::sleep(Duration::from_millis(1300));
            rig.wrapper.hold("prune");
            rig.clock.set(sunday_1200);
            wait_for_state(
                &rig.daemon.client,
                Duration::from_secs(30),
                "maintenance running",
                |s| {
                    s.destinations
                        .iter()
                        .any(|d| d.destination.id == dest.id && d.maintenance_running)
                },
            );
            let marker = root.join("maintenance-running-seen");
            let scene_ended = AtomicBool::new(false);
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    while !marker.exists() && !scene_ended.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    rig.wrapper.release("prune");
                });
                // Ends the helper thread also when the scene fails before writing the marker.
                let _end_helper = SetOnDrop(&scene_ended);
                run_m3_gui_smoke_with_env(
                    root,
                    "m3-maintenance-status-reset",
                    "wide",
                    "en",
                    &smoke_artifact("maintenance-status-reset", "wide", "en"),
                    &[("KESTIC_SMOKE_MARKER", marker.to_str().expect("utf-8 path"))],
                );
            });
            rig.daemon.stop();
        },
    );
}

/// Version and build commit of this build, for the About page checks of the smoke scenes.
fn build_env() -> [(&'static str, &'static str); 2] {
    [
        ("KESTIC_SMOKE_VERSION", kestic_daemon::daemon_version()),
        ("KESTIC_SMOKE_COMMIT", kestic_daemon::build_commit()),
    ]
}

/// GUI with a daemon that has neither a destination nor a backup.
fn empty_env(root: &Path) -> (M3Rig, StdFakes) {
    let fakes = gui_fakes(false, &[]);
    let rig = start_rig(root, datetime!(2026-12-01 10:00:00 UTC), false);
    (rig, fakes)
}

/// AC-M1-13-8: the ☰ button of the main menu, in the wide and the narrow window, (a) is
/// visible at the right end of the header with a name for screen readers; (b) a mouse click
/// opens the menu with «Configure Kestic…», «Configure Keyboard Shortcuts…» and «Handbook»
/// (both disabled) and «About Kestic»; (c) «About Kestic» opens the About page; (d) with the
/// keyboard alone Tab reaches the button, Space opens the menu, the arrow keys reach «About
/// Kestic» and Return opens it; (e) Escape closes the menu; (f) Ctrl+Shift+, opens the
/// settings. The C++ scene checks each statement; screenshots of the open menu and the About
/// page per layout and language.
#[test]
fn ac_m1_13_8_main_menu_button_works_with_mouse_and_keyboard() {
    run_isolated_test(
        "ac_m1_13_8_main_menu_button_works_with_mouse_and_keyboard",
        "KESTIC_M1_MAIN_MENU_INNER",
        |root| {
            let (rig, _fakes) = empty_env(root);
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m3_gui_smoke_with_env(
                    root,
                    "m3-main-menu",
                    layout,
                    lang,
                    &smoke_artifact("main-menu", layout, lang),
                    &build_env(),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M1-13-8 (b): the menu is static. Over an overview with three backups (ok, waiting,
/// overdue) it holds exactly the same two entries as over the welcome screen. Screenshots of the
/// open menu in English and German.
#[test]
fn ac_m1_13_8_main_menu_is_the_same_over_a_configured_overview() {
    run_isolated_test(
        "ac_m1_13_8_main_menu_is_the_same_over_a_configured_overview",
        "KESTIC_M1_MAIN_MENU_CONFIGURED_INNER",
        |root| {
            let (rig, _fakes) = status_rows_env(root);
            for lang in ["en", "de"] {
                run_m3_gui_smoke(
                    root,
                    "m3-main-menu-configured",
                    "wide",
                    lang,
                    &smoke_artifact("main-menu-configured", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M1-13-9: «About Kestic» with a background service of another build shows (a) the hint
/// to reopen Kestic, (b) a clean commit as a link to its GitHub page and (c) a `-dirty` commit
/// as text without a link. Started from the tray (`--show-about`), (d) the About window opens and
/// (e) in German shows Kirigami's own texts in German too (the menu path is checked by the
/// AC-M1-13-8 scene).
#[test]
fn ac_m1_13_9_about_page_links_commits_and_flags_another_build() {
    run_isolated_test(
        "ac_m1_13_9_about_page_links_commits_and_flags_another_build",
        "KESTIC_M1_ABOUT_BUILDS_INNER",
        |root| {
            let (rig, _fakes) = empty_env(root);
            for lang in ["en", "de"] {
                run_m3_gui_smoke_with_env(
                    root,
                    "m3-about-builds-differ",
                    "wide",
                    lang,
                    &smoke_artifact("about-builds-differ", "wide", lang),
                    &build_env(),
                );
            }
            let iso = kestic_test_support::gui::IsolatedEnv::adopt(root);
            let dbus_addr =
                std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("isolated session bus");
            let kestic_bin = find_kestic_binary();
            let smoke = kestic_test_support::gui::SmokeContext {
                kestic_bin: &kestic_bin,
                iso: &iso,
                dbus_addr: &dbus_addr,
            };
            for lang in ["en", "de"] {
                smoke.run_with_env(
                    "m3-show-about-on-start",
                    "wide",
                    lang,
                    &smoke_artifact("show-about-on-start", "wide", lang),
                    &["--show-about"],
                    &build_env(),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// Sets the flag when dropped, also while unwinding from a failed assertion.
struct SetOnDrop<'a>(&'a AtomicBool);

impl Drop for SetOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Changes made by a second D-Bus client while the smoke `m3-revision-conflict` runs: for each
/// file `change-<n>` the smoke writes, step 1–5 renames the backup and step 6 sets the
/// destination's `prune` to daily at `prune_hour`. Returns how many changes it made.
fn change_elsewhere_on_request(
    sync: &Path,
    backup: &BackupId,
    dest: &Destination,
    prune_hour: u8,
    stop: &AtomicBool,
) -> usize {
    let client = kestic_daemon::dbus_api::KesticClient::connect(
        KESTIC_BUS_NAME,
        kestic_daemon::dbus_api::KESTIC_OBJECT_PATH,
    )
    .expect("second D-Bus client");
    let mut done = 0;
    for step in 1..=6 {
        while !sync.join(format!("change-{step}")).exists() {
            if stop.load(Ordering::SeqCst) {
                return done;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if step <= 5 {
            client
                .rename_backup(backup.as_str(), &format!("Documents {step}"))
                .expect("rename backup elsewhere");
        } else {
            let maintenance = MaintenanceSchedule {
                prune: MaintenanceEntrySchedule::Daily {
                    hour: prune_hour,
                    minute: 0,
                },
                ..MaintenanceSchedule::default()
            };
            client
                .update_destination_maintenance(dest.id.as_str(), &maintenance)
                .expect("change destination elsewhere");
        }
        done += 1;
    }
    done
}

/// AC-M6-36-10 (a)–(e): every backup editor page and the form «Ziel bearbeiten» keep the revision
/// of their opening; a change a second D-Bus client made meanwhile ends the save in `Conflict`,
/// the page says so and shows the current state; the window's own saves right after another
/// and a page opened right after a save do not conflict.
#[test]
fn ac_m6_36_10_editor_pages_refuse_to_overwrite_a_change_made_elsewhere() {
    run_isolated_test(
        "ac_m6_36_10_editor_pages_refuse_to_overwrite_a_change_made_elsewhere",
        "KESTIC_M6_36_10_INNER",
        |root| {
            let (rig, _fakes, dest, id) = editor_env(root);
            for (lang, prune_hour) in [("en", 3), ("de", 4)] {
                let sync = root.join(format!("revision-sync-{lang}"));
                fs::create_dir_all(&sync).expect("mkdir sync");
                let stop = AtomicBool::new(false);
                let changes = std::thread::scope(|scope| {
                    let changer = scope.spawn(|| {
                        change_elsewhere_on_request(&sync, &id, &dest, prune_hour, &stop)
                    });
                    // Also when the smoke fails, so the changer ends and the failure shows.
                    let stop_changer = SetOnDrop(&stop);
                    run_m3_gui_smoke_with_env(
                        root,
                        "m3-revision-conflict",
                        "wide",
                        lang,
                        &smoke_artifact("revision-conflict", "wide", lang),
                        &[("KESTIC_SMOKE_SYNC_DIR", &sync.to_string_lossy())],
                    );
                    drop(stop_changer);
                    changer.join().expect("changer thread")
                });
                assert_eq!(changes, 6, "every change elsewhere was asked for ({lang})");
                // The changes made elsewhere stay: the refused saves overwrote nothing.
                assert_eq!(saved_backup(&rig, &id).name, "Documents 5", "({lang})");
                assert_eq!(
                    saved_maintenance_of(&rig, &dest).prune,
                    MaintenanceEntrySchedule::Daily {
                        hour: prune_hour,
                        minute: 0
                    },
                    "({lang})"
                );
            }
            rig.daemon.stop();
        },
    );
}
