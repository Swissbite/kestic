// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S10 catch-up at daemon start (`AC-M3-10-1`, `AC-M3-10-2`, `AC-M3-10-3`): an overdue backup is
//! seeded in `config.toml` and `history.db` before the daemon starts. `AC-M3-01-8`: slots before
//! a schedule change are not caught up.

mod common;

use common::revision::UpdateNow;

use common::m3::{
    M3DaemonOpts, M3Rig, ResticVariant, ResticWrapper, StdFakes, backup_summary, daemon_paths,
    notifications_for, restic_variants, seed_local_destination, settle, spawn_m3_daemon,
    wait_for_notifications, wait_for_state,
};
use common::run_isolated_test;
use kestic_core::clock::FakeClock;
use kestic_core::config::{AppConfig, save_config};
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, MaintenanceEntrySchedule, MaintenanceSchedule, ResticProvider, Run, RunStatus,
    Schedule, Source,
};
use kestic_daemon::dbus_api::DaemonStateDto;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use time::macros::datetime;

/// 2026-12-01 14:00 Europe/Zurich: the daily 12:00 slots of Nov 30 and Dec 1 were missed.
const NOW: time::OffsetDateTime = datetime!(2026-12-01 13:00:00 UTC);

struct Seed {
    backup: Backup,
    wrapper: ResticWrapper,
    stuck_run: Run,
}

/// Writes `config.toml` + `history.db` (baseline Nov 29 12:00 UTC, no success, one stuck `Running`
/// run) and initialises the repository with the real restic binary.
fn seed_overdue_backup(root: &Path, variant: &ResticVariant, sub: &str, clock: &FakeClock) -> Seed {
    let (cfg_path, state_dir) = daemon_paths(root, sub);
    let repo_dir = root.join(sub).join("repo");
    let src_dir = root.join(sub).join("src");
    fs::create_dir_all(&repo_dir).expect("mkdir repo");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("data.txt"), b"catch-up payload").expect("write src");

    let (dest, _password) = seed_local_destination(root, &variant.bin, "Seeded", &repo_dir);
    let mut backup = Backup::new("Overdue", Source::folder(&src_dir), dest.id.clone()).expect("bk");
    backup.schedule = Schedule::Daily {
        hour: 12,
        minute: 0,
    };

    let wrapper = ResticWrapper::create(&root.join(sub).join("wrapper"), &variant.bin);
    let mut cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: wrapper.path.clone(),
        },
        ..Default::default()
    };
    cfg.destinations.push(dest);
    cfg.backups.push(backup.clone());
    save_config(&cfg_path, &cfg).expect("save config");

    let mut history = RunHistory::open(&state_dir).expect("open history");
    let stuck_run = Run::new_backup(clock, &backup).expect("stuck run");
    history.upsert_run(&stuck_run, &[]).expect("seed stuck run");
    let mut state = history
        .get_backup_runtime_state(&backup.id)
        .expect("runtime state");
    state.baseline_at = Some(datetime!(2026-11-29 12:00:00 UTC));
    history
        .upsert_backup_runtime_state(&backup.id, &state)
        .expect("seed baseline");
    Seed {
        backup,
        wrapper,
        stuck_run,
    }
}

fn executed(state: &DaemonStateDto, seed: &Seed) -> usize {
    state
        .recent_runs
        .iter()
        .filter(|r| r.backup_id() == Some(&seed.backup.id))
        .filter(|r| r.id != seed.stuck_run.id && r.status != RunStatus::Pending)
        .count()
}

/// AC-M3-10-1 (§5.2, NFR-11): daemon start with an overdue backup and met conditions ->
/// (a) exactly one catch-up run, (b) the "overdue" notification.
#[test]
fn ac_m3_10_1_daemon_start_catches_up_overdue_backup_once_and_notifies() {
    run_isolated_test(
        "ac_m3_10_1_daemon_start_catches_up_overdue_backup_once_and_notifies",
        "KESTIC_M3_10_1_INNER",
        |root| {
            let fakes = StdFakes::start_with_power(false, 80.0);
            for variant in restic_variants() {
                fakes.notif.clear();
                fakes.upower.set_on_battery(false, 80.0);
                let sub = variant.sub("M3T1");
                let clock = Arc::new(FakeClock::new(NOW));
                let seed = seed_overdue_backup(root, &variant, &sub, &clock);
                let bus = variant.bus_name("M3T1");
                let daemon = spawn_m3_daemon(
                    root,
                    M3DaemonOpts::new(&sub, &bus, &seed.wrapper.path, Arc::clone(&clock) as _),
                );
                let client = &daemon.client;

                // (a) exactly one catch-up run (not one per missed slot).
                wait_for_state(client, Duration::from_secs(30), "catch-up Ok", |s| {
                    backup_summary(s, &seed.backup.id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.id != seed.stuck_run.id && r.status == RunStatus::Ok)
                });
                settle();
                let state = client.get_state().expect("GetState");
                assert_eq!(executed(&state, &seed), 1, "{}: (a) one run", variant.label);
                assert_eq!(
                    seed.wrapper.of('S', "backup").len(),
                    1,
                    "{}: (a) one restic backup process",
                    variant.label
                );
                // (b) "overdue" notification, exactly once.
                wait_for_notifications(&fakes.notif, "BackupOverdue", 1, Duration::from_secs(5));
                assert_eq!(
                    notifications_for(&fakes.notif, "BackupOverdue").len(),
                    1,
                    "{}: (b) exactly one overdue notification: {:?}",
                    variant.label,
                    fakes.notif.snapshot()
                );
                daemon.stop();
            }
        },
    );
}

/// AC-M3-10-2 (§5.2, negative): overdue backup, run condition unmet at daemon start ->
/// (a) `ausstehend` (run condition), (b) no run.
#[test]
fn ac_m3_10_2_daemon_start_with_unmet_condition_keeps_overdue_backup_pending() {
    run_isolated_test(
        "ac_m3_10_2_daemon_start_with_unmet_condition_keeps_overdue_backup_pending",
        "KESTIC_M3_10_2_INNER",
        |root| {
            let _fakes = StdFakes::start_with_power(true, 80.0);
            let variant = common::m3::latest_variant();
            let sub = variant.sub("M3T2");
            let clock = Arc::new(FakeClock::new(NOW));
            let seed = seed_overdue_backup(root, &variant, &sub, &clock);
            let bus = variant.bus_name("M3T2");
            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts::new(&sub, &bus, &seed.wrapper.path, Arc::clone(&clock) as _),
            );
            let client = &daemon.client;

            // (a) pending because of the run condition (battery).
            let pending = wait_for_state(client, Duration::from_secs(10), "pending", |s| {
                backup_summary(s, &seed.backup.id)
                    .pending_primary_code
                    .map(|c| c.as_str())
                    == Some("on_battery")
            });
            assert!(
                backup_summary(&pending, &seed.backup.id).is_overdue,
                "(a) the backup is overdue and waits for the run condition"
            );
            // (b) no run starts.
            settle();
            let state = client.get_state().expect("GetState");
            assert_eq!(executed(&state, &seed), 0, "(b) no run executed");
            assert_eq!(
                seed.wrapper.of('S', "backup").len(),
                0,
                "(b) no restic backup process"
            );
            daemon.stop();
        },
    );
}

/// AC-M3-10-3 (§5.1): a run still marked `läuft` in the history is set to `unterbrochen` when the
/// daemon starts, before new runs are planned.
#[test]
fn ac_m3_10_3_daemon_start_marks_running_run_interrupted_before_planning() {
    run_isolated_test(
        "ac_m3_10_3_daemon_start_marks_running_run_interrupted_before_planning",
        "KESTIC_M3_10_3_INNER",
        |root| {
            // On battery so the due backup is not planned into a run that would hide the check.
            let _fakes = StdFakes::start_with_power(true, 80.0);
            let variant = common::m3::latest_variant();
            let sub = variant.sub("M3T3");
            let clock = Arc::new(FakeClock::new(NOW));
            let seed = seed_overdue_backup(root, &variant, &sub, &clock);
            assert_eq!(seed.stuck_run.status, RunStatus::Running, "precondition");
            let bus = variant.bus_name("M3T3");
            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts::new(&sub, &bus, &seed.wrapper.path, Arc::clone(&clock) as _),
            );

            let detail = daemon
                .client
                .get_run_detail(seed.stuck_run.id.as_str())
                .expect("GetRunDetail");
            assert_eq!(
                detail.run.status,
                RunStatus::Interrupted,
                "the stuck run is interrupted at daemon start"
            );
            let persisted = RunHistory::open(&daemon.state_dir)
                .expect("open history")
                .get_run(&seed.stuck_run.id)
                .expect("query")
                .expect("run persisted");
            assert_eq!(persisted.status, RunStatus::Interrupted, "persisted");
            daemon.stop();
        },
    );
}

/// Saturday 2026-11-21 16:31 Europe/Zurich.
const SAT_1631: time::OffsetDateTime = datetime!(2026-11-21 15:31:00 UTC);
/// Saturday 2026-11-21 16:32:05 Europe/Zurich.
const SAT_1632: time::OffsetDateTime = datetime!(2026-11-21 15:32:05 UTC);

/// AC-M3-01-8 through the daemon: saved schedules count from the change. A destination added
/// on Friday that never ran `check` gets «Verify: daily 16:32» on Saturday at 16:31: (a) no
/// `check` starts at 16:31, (b) the first one starts at 16:32. At the same time a backup that ran
/// on Friday is changed from «Sundays 23:00» to «daily 14:00» and (c) does not start right away,
/// and a manual-only backup switched to «daily 12:00» (f) does not start right away either,
/// although both slots lie before 16:31. Neither old schedule is due on Saturday, so no run can
/// start before the changes are saved.
#[test]
fn ac_m3_01_8_saved_schedules_count_from_the_change() {
    run_isolated_test(
        "ac_m3_01_8_saved_schedules_count_from_the_change",
        "KESTIC_AC_M3_01_8_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let variant = restic_variants().pop().expect("latest restic");
            let rig = M3Rig::start(
                root,
                &variant,
                "M3A8",
                datetime!(2026-11-20 09:00:00 UTC),
                |_| {},
            );
            let client = &rig.daemon.client;
            let dest = rig.add_destination("Anchor Vault", |_| {});
            let no_conditions = |b: &mut Backup| {
                b.run_conditions.require_ac_power = false;
                b.run_conditions.require_network = false;
            };
            let ran = rig.add_backup_to(&dest, "Ran Friday", |b| {
                b.schedule = Schedule::Weekly {
                    weekday: 7,
                    hour: 23,
                    minute: 0,
                };
                no_conditions(b);
            });
            let manual = rig.add_backup_to(&dest, "Manual", |b| {
                b.schedule = Schedule::ManualOnly;
                no_conditions(b);
            });
            client.start_backup(ran.id.as_str()).expect("Friday run");
            wait_for_state(client, Duration::from_secs(30), "Friday run ok", |s| {
                backup_summary(s, &ran.id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });
            settle();

            let backups_before = rig.wrapper.of('S', "backup").len();
            rig.clock.set(SAT_1631);
            let maintenance = MaintenanceSchedule {
                check: MaintenanceEntrySchedule::Daily {
                    hour: 16,
                    minute: 32,
                },
                ..MaintenanceSchedule::default()
            };
            client
                .update_destination_maintenance(dest.id.as_str(), &maintenance)
                .expect("save verify schedule");
            let mut changed = ran.clone();
            changed.schedule = Schedule::Daily {
                hour: 14,
                minute: 0,
            };
            client.update_backup_now(&changed).expect("change schedule");
            let mut switched = manual.clone();
            switched.schedule = Schedule::Daily {
                hour: 12,
                minute: 0,
            };
            client
                .update_backup_now(&switched)
                .expect("switch from manual only");
            settle();
            assert!(
                rig.wrapper.of('S', "check").is_empty(),
                "(a) no check at 16:31: {:?}",
                rig.wrapper.entries()
            );
            assert_eq!(
                rig.wrapper.of('S', "backup").len(),
                backups_before,
                "no backup process starts right after the schedule changes"
            );
            let state = client.get_state().expect("GetState");
            assert!(
                backup_summary(&state, &ran.id).active_run.is_none(),
                "(c) changed backup does not run"
            );
            assert!(
                backup_summary(&state, &manual.id).active_run.is_none(),
                "(f) switched backup does not run"
            );

            rig.clock.set(SAT_1632);
            rig.wrapper
                .wait_for('S', "check", 1, Duration::from_secs(30));
            rig.daemon.stop();
        },
    );
}
