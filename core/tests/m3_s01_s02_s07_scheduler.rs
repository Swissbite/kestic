// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

use kestic_core::clock::FakeClock;
use kestic_core::config::AppConfig;
use kestic_core::history::{BackupRuntimeState, PauseState, RunHistory};
use kestic_core::model::{
    Backup, Destination, DestinationKind, MaintenanceSchedule, PendingReason, Run, RunConditions,
    RunStatus, Schedule, Source, Weekday,
};
use kestic_core::scheduler::{
    BackupScheduleEvalInput, DestinationMaintenanceEvalInput, DestinationMaintenanceNeed,
    PauseDuration, SystemEnvironment, TimezoneSpec, UnmetPrecondition, backup_slots_between,
    compute_backoff_until, compute_pause_until, evaluate_backup_schedule,
    evaluate_destination_maintenance, next_maintenance_slot, plan_destination_actions,
    resolve_local_slot,
};
use std::collections::{HashMap, HashSet};
use tempfile::TempDir;
use time::OffsetDateTime;
use time::macros::datetime;

fn zurich() -> TimezoneSpec {
    TimezoneSpec::named("Europe/Zurich")
}

fn zdt(year: i16, month: i8, day: i8, hour: u8, minute: u8) -> OffsetDateTime {
    let tz = zurich().resolve();
    let date = jiff::civil::Date::new(year, month, day).expect("valid date");
    resolve_local_slot(date, hour, minute, &tz)
}

fn make_backup(name: &str, schedule: Schedule) -> (Destination, Backup) {
    let dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: "/tmp/kestic-sched-test-dest".into(),
        },
    )
    .expect("dest");
    let mut bk = Backup::new(
        name,
        Source::folder("/tmp/kestic-sched-test-src"),
        dest.id.clone(),
    )
    .expect("backup");
    bk.schedule = schedule;
    (dest, bk)
}

#[test]
fn ac_m3_01_1_daily_1200_due_exactly_once_next_day() {
    let (_dest, bk) = make_backup(
        "Daily",
        Schedule::Daily {
            hour: 12,
            minute: 0,
        },
    );
    let mon_1200 = zdt(2026, 11, 16, 12, 0);
    let tue_1159 = zdt(2026, 11, 17, 11, 59);
    let tue_1200 = zdt(2026, 11, 17, 12, 0);

    let rs = BackupRuntimeState {
        baseline_at: Some(mon_1200),
        ..Default::default()
    };
    let input = BackupScheduleEvalInput {
        backup: &bk,
        runtime_state: &rs,
        last_success_at: Some(mon_1200),
        overdue_anchor_at: None,
        global_pause: None,
        global_overdue_threshold: 2,
        source_available: true,
        destination_reachable: true,
        destination_busy: false,
        has_permanent_error: false,
    };

    let before = evaluate_backup_schedule(
        &input,
        &SystemEnvironment::desktop_connected(),
        tue_1159,
        &zurich(),
    );
    assert!(!before.should_start_now);
    assert_eq!(before.due_slot, None);

    let at_due = evaluate_backup_schedule(
        &input,
        &SystemEnvironment::desktop_connected(),
        tue_1200,
        &zurich(),
    );
    assert!(at_due.should_start_now);
    assert_eq!(at_due.due_slot, Some(tue_1200));
    // Exactly one run is due: a single slot lies between the last success and Tue 12:00.
    assert_eq!(
        backup_slots_between(&bk.schedule, mon_1200, tue_1200, &zurich()),
        vec![tue_1200]
    );
}

#[test]
fn ac_m3_01_2_specific_weekdays_mon_wed_fri_2000() {
    let sched = Schedule::SpecificWeekdays {
        weekdays: vec![Weekday::Monday, Weekday::Wednesday, Weekday::Friday],
        hour: 20,
        minute: 0,
    };
    // 2026-11-16 is Monday
    let mon_2001 = zdt(2026, 11, 16, 20, 1);
    let fri_2001 = zdt(2026, 11, 20, 20, 1);
    let slots = backup_slots_between(&sched, mon_2001, fri_2001, &zurich());
    // (a) due exactly Wed 20:00 and Fri 20:00.
    assert_eq!(
        slots,
        vec![zdt(2026, 11, 18, 20, 0), zdt(2026, 11, 20, 20, 0)]
    );
    // (b) neither Tuesday nor Thursday.
    assert!(
        !slots.contains(&zdt(2026, 11, 17, 20, 0)),
        "no Tuesday slot"
    );
    assert!(
        !slots.contains(&zdt(2026, 11, 19, 20, 0)),
        "no Thursday slot"
    );
}

#[test]
fn ac_m3_01_3_every_3_days_from_2026_12_01_0900() {
    let sched = Schedule::EveryNDays {
        interval_days: 3,
        start_date: "2026-12-01".to_string(),
        hour: 9,
        minute: 0,
    };
    let before = zdt(2026, 11, 30, 23, 59);
    let end = zdt(2026, 12, 10, 9, 1);
    let slots = backup_slots_between(&sched, before, end, &zurich());
    assert_eq!(
        slots,
        vec![
            zdt(2026, 12, 1, 9, 0),
            zdt(2026, 12, 4, 9, 0),
            zdt(2026, 12, 7, 9, 0),
            zdt(2026, 12, 10, 9, 0),
        ]
    );
}

#[test]
fn ac_m3_01_4_missed_slots_catch_up_once_and_mark_overdue() {
    let (_dest, bk) = make_backup(
        "CatchUp",
        Schedule::Daily {
            hour: 12,
            minute: 0,
        },
    );
    let sun_1200 = zdt(2026, 11, 15, 12, 0);
    let tue_1400 = zdt(2026, 11, 17, 14, 0);
    let rs = BackupRuntimeState::default();

    let eval = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &rs,
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        tue_1400,
        &zurich(),
    );

    // Exactly one due catch-up slot (latest missed) and overdue because 2 slots missed (Mon & Tue).
    assert!(eval.should_start_now);
    assert_eq!(eval.due_slot, Some(zdt(2026, 11, 17, 12, 0)));
    assert_eq!(eval.missed_slots_for_overdue, 2);
    assert!(eval.is_overdue);
}

#[test]
fn ac_m3_01_5_dst_spring_gap_0230_resolves_to_0300_next_valid_minute_once() {
    // Sun 2027-03-28 Europe/Zurich: 02:00 CET jumps to 03:00 CEST (01:00:00 UTC).
    let sched = Schedule::Daily {
        hour: 2,
        minute: 30,
    };
    let start = datetime!(2027-03-28 00:00:00 UTC);
    let end = datetime!(2027-03-28 03:00:00 UTC);
    let slots = backup_slots_between(&sched, start, end, &zurich());
    // (a) due at 03:00 CEST (= 01:00 UTC), the next valid minute after the gap.
    assert_eq!(
        slots.first().copied(),
        Some(datetime!(2027-03-28 01:00:00 UTC))
    );
    // (b) exactly once that day.
    assert_eq!(
        slots.len(),
        1,
        "exactly one slot over the DST gap: {slots:?}"
    );
}

#[test]
fn ac_m3_01_6_dst_autumn_fold_0230_runs_exactly_once() {
    // Sun 2026-10-25 Europe/Zurich: 02:30 occurs twice (00:30 UTC in +02:00, 01:30 UTC in +01:00).
    let sched = Schedule::Daily {
        hour: 2,
        minute: 30,
    };
    let start = datetime!(2026-10-24 22:00:00 UTC);
    let end = datetime!(2026-10-25 04:00:00 UTC);
    let slots = backup_slots_between(&sched, start, end, &zurich());
    assert_eq!(slots, vec![datetime!(2026-10-25 00:30:00 UTC)]);
}

#[test]
fn ac_m3_01_7_suspend_gap_triggers_single_catchup_after_wakeup() {
    let (_dest, bk) = make_backup(
        "Suspend",
        Schedule::Daily {
            hour: 12,
            minute: 0,
        },
    );
    let sun_1200 = zdt(2026, 11, 15, 12, 0);
    let mon_1500 = zdt(2026, 11, 16, 15, 0);
    let rs = BackupRuntimeState::default();
    let eval = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &rs,
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        mon_1500,
        &zurich(),
    );
    assert!(eval.should_start_now);
    assert_eq!(eval.due_slot, Some(zdt(2026, 11, 16, 12, 0)));
    assert!(!eval.is_overdue);
}

#[test]
fn ac_m3_02_1_and_02_5_pending_reasons_and_strict_precedence_order() {
    let (_dest, mut bk) = make_backup(
        "Precedence",
        Schedule::Daily {
            hour: 12,
            minute: 0,
        },
    );
    bk.run_conditions = RunConditions {
        require_ac_power: true,
        require_network: true,
        skip_on_metered_network: true,
        allowed_networks: vec!["Home".to_string()],
    };
    let sun_1200 = zdt(2026, 11, 15, 12, 0);
    let mon_1200 = zdt(2026, 11, 16, 12, 0);
    let mon_1203 = zdt(2026, 11, 16, 12, 3);
    let rs = BackupRuntimeState {
        backoff_attempt: 1,
        backoff_until: Some(zdt(2026, 11, 16, 12, 5)),
        ..Default::default()
    };

    // Simultaneous failure: source unavailable + dest unreachable + battery + no network + metered + wrong SSID + queued + backoff
    let env_all_bad = SystemEnvironment {
        on_battery: true,
        battery_percent: Some(34),
        network_connected: false,
        network_metered: true,
        active_networks: vec!["Cafe".to_string()],
    };
    let eval_all = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &rs,
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: false,
            destination_reachable: false,
            destination_busy: true,
            has_permanent_error: false,
        },
        &env_all_bad,
        mon_1203,
        &zurich(),
    );
    assert_eq!(
        eval_all.unmet_preconditions,
        vec![
            UnmetPrecondition::SourceUnavailable,
            UnmetPrecondition::DestinationUnreachable,
            UnmetPrecondition::OnBattery,
            UnmetPrecondition::NoNetwork,
            UnmetPrecondition::MeteredNetwork,
            UnmetPrecondition::AllowedNetworkMismatch {
                allowed: vec!["Home".to_string()]
            },
            UnmetPrecondition::Queued,
            UnmetPrecondition::Backoff,
        ]
    );
    assert_eq!(
        eval_all.unmet_preconditions[0].to_pending_reason(),
        PendingReason::SourceUnavailable
    );

    // Individual PendingReason checks (`AC-M3-02-1`)
    let check_single = |src: bool,
                        dst: bool,
                        env: SystemEnvironment,
                        busy: bool,
                        backoff: Option<OffsetDateTime>| {
        let state = BackupRuntimeState {
            backoff_until: backoff,
            ..Default::default()
        };
        evaluate_backup_schedule(
            &BackupScheduleEvalInput {
                backup: &bk,
                runtime_state: &state,
                last_success_at: Some(sun_1200),
                overdue_anchor_at: None,
                global_pause: None,
                global_overdue_threshold: 2,
                source_available: src,
                destination_reachable: dst,
                destination_busy: busy,
                has_permanent_error: false,
            },
            &env,
            mon_1200,
            &zurich(),
        )
        .unmet_preconditions[0]
            .to_pending_reason()
    };

    let home_env = SystemEnvironment {
        on_battery: false,
        battery_percent: None,
        network_connected: true,
        network_metered: false,
        active_networks: vec!["Home".to_string()],
    };
    assert_eq!(
        check_single(false, true, home_env.clone(), false, None),
        PendingReason::SourceUnavailable
    );
    assert_eq!(
        check_single(true, false, home_env.clone(), false, None),
        PendingReason::DestinationUnreachable
    );
    assert_eq!(
        check_single(
            true,
            true,
            SystemEnvironment {
                on_battery: true,
                ..home_env.clone()
            },
            false,
            None
        ),
        PendingReason::ConditionNotMet
    );
    assert_eq!(
        check_single(true, true, home_env.clone(), true, None),
        PendingReason::Queued
    );
    assert_eq!(
        check_single(
            true,
            true,
            home_env,
            false,
            Some(mon_1200 + time::Duration::minutes(5))
        ),
        PendingReason::Backoff
    );
}

#[test]
fn ac_m3_02_2_and_02_3_overdue_threshold_is_configurable() {
    let (_dest, mut bk) = make_backup(
        "Overdue",
        Schedule::SpecificWeekdays {
            weekdays: vec![Weekday::Monday, Weekday::Wednesday, Weekday::Friday],
            hour: 20,
            minute: 0,
        },
    );
    let mon_2000 = zdt(2026, 11, 16, 20, 0);
    let wed_2001 = zdt(2026, 11, 18, 20, 1);
    let fri_2000 = zdt(2026, 11, 20, 20, 0);
    let rs = BackupRuntimeState::default();

    let eval_wed = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &rs,
            last_success_at: Some(mon_2000),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        wed_2001,
        &zurich(),
    );
    assert!(!eval_wed.is_overdue);

    let eval_fri = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &rs,
            last_success_at: Some(mon_2000),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        fri_2000,
        &zurich(),
    );
    assert!(eval_fri.is_overdue);

    // AC-M3-02-3: threshold = 3 with daily 12:00
    bk.schedule = Schedule::Daily {
        hour: 12,
        minute: 0,
    };
    bk.overdue_threshold = Some(3);
    let sun_1200 = zdt(2026, 11, 15, 12, 0);
    let tue_1200 = zdt(2026, 11, 17, 12, 0);
    let wed_1200 = zdt(2026, 11, 18, 12, 0);
    let eval_2 = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &rs,
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        tue_1200,
        &zurich(),
    );
    assert!(!eval_2.is_overdue);
    let eval_3 = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &rs,
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        wed_1200,
        &zurich(),
    );
    assert!(eval_3.is_overdue);
}

/// AC-M3-02-4: an overdue backup is (a) no longer overdue after a run ending `ok` or `Warnung`,
/// (b) still overdue after a run ending `Fehler`.
#[test]
fn ac_m3_02_4_ok_and_warning_clear_overdue_error_keeps_it() {
    let sun_1200 = zdt(2026, 11, 15, 12, 0);
    let wed_1200 = zdt(2026, 11, 18, 12, 0);
    let wed_1205 = zdt(2026, 11, 18, 12, 5);
    let rs = BackupRuntimeState::default();

    // Overdue state of a backup: success on Sunday, Monday/Tuesday/Wednesday missed.
    let overdue_after = |status: Option<RunStatus>| -> bool {
        let (_dest, bk) = make_backup(
            "Overdue02-4",
            Schedule::Daily {
                hour: 12,
                minute: 0,
            },
        );
        let dir = TempDir::new().expect("tempdir");
        let mut history = RunHistory::open(dir.path()).expect("history");
        let sun_clock = FakeClock::new(sun_1200);
        let mut success = Run::new_backup(&sun_clock, &bk).expect("success run");
        success.finish(&sun_clock, RunStatus::Ok, Some(0));
        history.upsert_run(&success, &[]).expect("save success");

        let before = evaluate_backup_schedule(
            &BackupScheduleEvalInput {
                backup: &bk,
                runtime_state: &rs,
                last_success_at: history.last_successful_backup_at(&bk.id).expect("query"),
                overdue_anchor_at: None,
                global_pause: None,
                global_overdue_threshold: 2,
                source_available: true,
                destination_reachable: true,
                destination_busy: false,
                has_permanent_error: false,
            },
            &SystemEnvironment::desktop_connected(),
            wed_1200,
            &zurich(),
        );
        assert!(before.is_overdue, "precondition: overdue before the run");

        if let Some(status) = status {
            let clock = FakeClock::new(wed_1200);
            let mut run = Run::new_backup(&clock, &bk).expect("run");
            run.finish(
                &clock,
                status,
                Some(match status {
                    RunStatus::Ok => 0,
                    RunStatus::Warning => 3,
                    _ => 1,
                }),
            );
            history.upsert_run(&run, &[]).expect("save run");
        }
        evaluate_backup_schedule(
            &BackupScheduleEvalInput {
                backup: &bk,
                runtime_state: &rs,
                last_success_at: history.last_successful_backup_at(&bk.id).expect("query"),
                overdue_anchor_at: None,
                global_pause: None,
                global_overdue_threshold: 2,
                source_available: true,
                destination_reachable: true,
                destination_busy: false,
                has_permanent_error: false,
            },
            &SystemEnvironment::desktop_connected(),
            wed_1205,
            &zurich(),
        )
        .is_overdue
    };

    // (a) `ok` and `Warnung` end the overdue state.
    assert!(!overdue_after(Some(RunStatus::Ok)), "(a) ok clears overdue");
    assert!(
        !overdue_after(Some(RunStatus::Warning)),
        "(a) warning clears overdue"
    );
    // (b) `Fehler` keeps it overdue.
    assert!(
        overdue_after(Some(RunStatus::Error)),
        "(b) error keeps the backup overdue"
    );
}

#[test]
fn ac_m3_03_1_new_backup_defaults_to_ac_power_and_network() {
    let (_dest, bk) = make_backup("DefaultCond", Schedule::ManualOnly);
    assert!(bk.run_conditions.require_ac_power);
    assert!(bk.run_conditions.require_network);
    assert!(!bk.run_conditions.skip_on_metered_network);
    assert!(bk.run_conditions.allowed_networks.is_empty());

    let cfg = AppConfig::default();
    assert_eq!(cfg.default_overdue_threshold, 2);
}

#[test]
fn ac_m3_04_6_backoff_measured_from_end_of_each_attempt() {
    let t1_end = zdt(2026, 11, 16, 12, 0) + time::Duration::seconds(10);
    let retry1 = compute_backoff_until(1, t1_end).expect("retry 1");
    assert_eq!(
        retry1,
        zdt(2026, 11, 16, 12, 5) + time::Duration::seconds(10)
    );

    let t2_end = retry1 + time::Duration::seconds(10);
    let retry2 = compute_backoff_until(2, t2_end).expect("retry 2");
    assert_eq!(
        retry2,
        zdt(2026, 11, 16, 12, 20) + time::Duration::seconds(20)
    );

    let t3_end = retry2 + time::Duration::seconds(10);
    let retry3 = compute_backoff_until(3, t3_end).expect("retry 3");
    assert_eq!(
        retry3,
        zdt(2026, 11, 16, 13, 20) + time::Duration::seconds(30)
    );

    assert_eq!(compute_backoff_until(4, retry3), None);
}

#[test]
fn ac_m3_05_2_default_maintenance_schedule_december_2026() {
    let maint = MaintenanceSchedule::default();
    let start = zdt(2026, 12, 1, 0, 0);
    let next_prune = next_maintenance_slot(&maint.prune, start, &zurich()).expect("prune slot");
    let next_check = next_maintenance_slot(&maint.check, start, &zurich()).expect("check slot");
    // 2026-12-06 is the first Sunday of December 2026.
    assert_eq!(next_prune, zdt(2026, 12, 6, 12, 0));
    assert_eq!(next_check, zdt(2026, 12, 6, 12, 0));

    // (a) `prune` every Sunday 12:00.
    let next_prune_after =
        next_maintenance_slot(&maint.prune, zdt(2026, 12, 6, 12, 1), &zurich()).expect("prune");
    assert_eq!(next_prune_after, zdt(2026, 12, 13, 12, 0));
    // (b) `check` only on the first Sunday of the month: after Dec 6 the next is Jan 3, 2027.
    let next_check_after =
        next_maintenance_slot(&maint.check, zdt(2026, 12, 6, 12, 1), &zurich()).expect("jan check");
    assert_eq!(next_check_after, zdt(2027, 1, 3, 12, 0));
}

/// AC-M3-05-4 / FR-05, F4: an open `forget` is caught up by the next maintenance run at its
/// slot, not by an extra maintenance run of its own.
#[test]
fn ac_m3_05_4_open_forget_waits_for_the_maintenance_slot() {
    let (dest, _bk) = make_backup("ForgetWaits", Schedule::ManualOnly);
    let state = kestic_core::history::DestinationMaintenanceState {
        baseline_at: Some(zdt(2026, 12, 1, 10, 0)),
        ..Default::default()
    };
    let eval_at = |now| {
        evaluate_destination_maintenance(
            &DestinationMaintenanceEvalInput {
                destination: &dest,
                state: &state,
                has_pending_forget: true,
                global_pause: None,
                destination_busy: false,
                destination_reachable: true,
            },
            &SystemEnvironment::desktop_connected(),
            now,
            &zurich(),
        )
    };
    // Tuesday after the failed forget: no prune/check slot yet -> no maintenance run.
    let tuesday = eval_at(zdt(2026, 12, 1, 11, 0));
    assert!(tuesday.has_pending_forget, "forget is open");
    assert!(
        !tuesday.due_now,
        "open forget alone does not start maintenance"
    );
    // Sunday 12:00 prune slot -> maintenance is due (and works the open forget off first).
    let sunday = eval_at(zdt(2026, 12, 6, 12, 0));
    assert!(sunday.due_now, "maintenance due at the prune slot");
}

fn eval_pause_case(
    backup: &Backup,
    runtime_state: &BackupRuntimeState,
    last_success_at: OffsetDateTime,
    now: OffsetDateTime,
) -> kestic_core::scheduler::BackupScheduleEvaluation {
    evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup,
            runtime_state,
            last_success_at: Some(last_success_at),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        now,
        &zurich(),
    )
}

#[test]
fn ac_m3_07_1_to_07_4_pause_per_backup_global_and_until_tomorrow() {
    let (dest, bk_a) = make_backup(
        "A",
        Schedule::Daily {
            hour: 12,
            minute: 0,
        },
    );
    let mut bk_b = Backup::new(
        "B",
        Source::folder("/tmp/kestic-sched-test-src-b"),
        dest.id.clone(),
    )
    .expect("b");
    bk_b.schedule = Schedule::Daily {
        hour: 12,
        minute: 0,
    };

    // AC-M3-07-1: A paused, B on same destination runs normally.
    let sun_1200 = zdt(2026, 11, 15, 12, 0);
    let mon_1200 = zdt(2026, 11, 16, 12, 0);
    let rs_a = BackupRuntimeState {
        pause: Some(PauseState {
            paused_since: zdt(2026, 11, 16, 10, 0),
            paused_until: None,
        }),
        ..Default::default()
    };
    let rs_b = BackupRuntimeState::default();
    let eval_a = eval_pause_case(&bk_a, &rs_a, sun_1200, mon_1200);
    let eval_b = eval_pause_case(&bk_b, &rs_b, sun_1200, mon_1200);
    assert!(eval_a.is_paused && !eval_a.should_start_now);
    assert!(!eval_b.is_paused && eval_b.should_start_now);

    let mut dest_map = HashMap::new();
    dest_map.insert(bk_a.id.clone(), dest.id.clone());
    dest_map.insert(bk_b.id.clone(), dest.id.clone());
    let actions = plan_destination_actions(&[eval_a, eval_b], &[], &HashSet::new(), &dest_map);
    assert_eq!(actions.len(), 1);

    // AC-M3-07-2: Global pause blocks both backups and maintenance.
    let global_pause = PauseState {
        paused_since: zdt(2026, 11, 16, 10, 0),
        paused_until: None,
    };
    // (a) no scheduled run for a due backup while globally paused.
    let eval_global = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk_b,
            runtime_state: &rs_b,
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: Some(&global_pause),
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        mon_1200,
        &zurich(),
    );
    assert!(eval_global.is_paused, "(a) globally paused");
    assert!(!eval_global.should_start_now, "(a) no scheduled run");
    // (b) no maintenance run while globally paused (Sunday 15.11. prune slot is due).
    let state_prune_due = kestic_core::history::DestinationMaintenanceState {
        baseline_at: Some(zdt(2026, 11, 8, 13, 0)),
        ..Default::default()
    };
    let maint_need = evaluate_destination_maintenance(
        &DestinationMaintenanceEvalInput {
            destination: &dest,
            state: &state_prune_due,
            has_pending_forget: true,
            global_pause: Some(&global_pause),
            destination_busy: false,
            destination_reachable: true,
        },
        &SystemEnvironment::desktop_connected(),
        mon_1200,
        &zurich(),
    );
    assert!(!maint_need.due_now, "(b) no maintenance run while paused");

    // AC-M3-07-3: Pause "for 4 hours" from 10:00 -> at 14:00 unpaused and catches up missed 12:00 once.
    let rs_4h = BackupRuntimeState {
        pause: Some(PauseState {
            paused_since: zdt(2026, 11, 16, 10, 0),
            paused_until: compute_pause_until(
                zdt(2026, 11, 16, 10, 0),
                PauseDuration::Hours { hours: 4 },
                &[&bk_b.schedule],
                &zurich(),
            ),
        }),
        ..Default::default()
    };
    let mon_1400 = zdt(2026, 11, 16, 14, 0);
    let eval_4h = eval_pause_case(&bk_b, &rs_4h, sun_1200, mon_1400);
    assert!(!eval_4h.is_paused);
    assert!(eval_4h.should_start_now);
    assert_eq!(eval_4h.due_slot, Some(mon_1200));

    // AC-M3-07-4: Pause over 3 slots -> not overdue during pause; after pause only subsequent slots count.
    let wed_1400 = zdt(2026, 11, 18, 14, 0);
    let thu_1200 = zdt(2026, 11, 19, 12, 0);
    let fri_1200 = zdt(2026, 11, 20, 12, 0);
    let rs_3slots = BackupRuntimeState {
        pause: Some(PauseState {
            paused_since: zdt(2026, 11, 16, 10, 0),
            paused_until: Some(wed_1400),
        }),
        ..Default::default()
    };
    let during = eval_pause_case(&bk_b, &rs_3slots, sun_1200, zdt(2026, 11, 18, 12, 30));
    assert!(!during.is_overdue);
    let after_1 = eval_pause_case(&bk_b, &rs_3slots, sun_1200, thu_1200);
    assert!(!after_1.is_overdue);
    let after_2 = eval_pause_case(&bk_b, &rs_3slots, sun_1200, fri_1200);
    assert!(after_2.is_overdue);

    // "Until tomorrow" skips today's schedule until tomorrow's scheduled time (e.g. 08:00 -> tomorrow 08:00).
    let sched_0800 = Schedule::Daily { hour: 8, minute: 0 };
    let until_tmr = compute_pause_until(
        zdt(2026, 11, 16, 7, 30),
        PauseDuration::UntilTomorrow,
        &[&sched_0800],
        &zurich(),
    );
    assert_eq!(until_tmr, Some(zdt(2026, 11, 17, 8, 0)));
}

#[test]
fn ac_m3_10_3_startup_marks_running_as_interrupted() {
    let dir = TempDir::new().expect("tempdir");
    let mut history = RunHistory::open(dir.path()).expect("history");
    let clock = FakeClock::new(zdt(2026, 11, 16, 12, 0));
    let (_dest, bk) = make_backup(
        "Startup",
        Schedule::Daily {
            hour: 12,
            minute: 0,
        },
    );
    let running = Run::new_backup(&clock, &bk).expect("run");
    history.upsert_run(&running, &[]).expect("save");

    clock.advance(std::time::Duration::from_secs(1800));
    let changed = history.mark_interrupted_on_startup(&clock).expect("mark");
    assert_eq!(changed, 1);
    let loaded = history.get_run(&running.id).expect("get").expect("some");
    assert_eq!(loaded.status, RunStatus::Interrupted);
}

#[test]
fn ac_m3_13_1_and_13_5_missing_source_stays_pending_and_checks_each_slot() {
    let (_dest, bk) = make_backup(
        "MissingSrc",
        Schedule::Daily {
            hour: 12,
            minute: 0,
        },
    );
    let sun_1200 = zdt(2026, 11, 15, 12, 0);
    let mon_1200 = zdt(2026, 11, 16, 12, 0);
    let tue_1200 = zdt(2026, 11, 17, 12, 0);

    let eval_1 = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &Default::default(),
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: false,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        mon_1200,
        &zurich(),
    );
    assert!(!eval_1.should_start_now);
    assert!(!eval_1.is_overdue);
    assert_eq!(
        eval_1.unmet_preconditions,
        vec![UnmetPrecondition::SourceUnavailable]
    );

    let eval_2 = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &Default::default(),
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: false,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        tue_1200,
        &zurich(),
    );
    assert!(!eval_2.should_start_now);
    assert!(eval_2.is_overdue);
}

#[test]
fn ac_m3_05_1_and_05_3_destination_queue_prefers_backups_over_maintenance() {
    let (dest, bk_a) = make_backup(
        "A",
        Schedule::Daily {
            hour: 12,
            minute: 0,
        },
    );
    let mut bk_b = Backup::new(
        "B",
        Source::folder("/tmp/kestic-sched-test-src-b"),
        dest.id.clone(),
    )
    .expect("b");
    bk_b.schedule = Schedule::Daily {
        hour: 12,
        minute: 0,
    };

    let eval_a = BackupScheduleEvalInput {
        backup: &bk_a,
        runtime_state: &Default::default(),
        last_success_at: Some(zdt(2026, 11, 15, 12, 0)),
        overdue_anchor_at: None,
        global_pause: None,
        global_overdue_threshold: 2,
        source_available: true,
        destination_reachable: true,
        destination_busy: false,
        has_permanent_error: false,
    };
    let eval_b = BackupScheduleEvalInput {
        backup: &bk_b,
        ..eval_a.clone()
    };
    let now = zdt(2026, 11, 16, 12, 0);
    let ea = evaluate_backup_schedule(
        &eval_a,
        &SystemEnvironment::desktop_connected(),
        now,
        &zurich(),
    );
    let eb = evaluate_backup_schedule(
        &eval_b,
        &SystemEnvironment::desktop_connected(),
        now,
        &zurich(),
    );
    let maint = DestinationMaintenanceNeed {
        destination_id: dest.id.clone(),
        has_pending_forget: true,
        prune_due: true,
        check_due: false,
        next_prune_at: None,
        next_check_at: None,
        in_backoff: false,
        due_now: true,
    };
    let mut map = HashMap::new();
    map.insert(bk_a.id.clone(), dest.id.clone());
    map.insert(bk_b.id.clone(), dest.id.clone());
    let actions = plan_destination_actions(&[ea, eb], &[maint], &HashSet::new(), &map);
    assert_eq!(actions.len(), 1);
    assert!(matches!(
        actions[0],
        kestic_core::scheduler::DestinationTickAction::StartBackup { .. }
    ));
}

#[test]
fn ac_m3_04_1_exhausted_backoff_waits_for_next_regular_slot() {
    let (_dest, bk) = make_backup(
        "RetryAndResume",
        Schedule::Daily {
            hour: 12,
            minute: 0,
        },
    );
    let sun_1200 = zdt(2026, 11, 15, 12, 0);
    let mon_1400 = zdt(2026, 11, 16, 14, 0);
    let tue_1200 = zdt(2026, 11, 17, 12, 0);

    // AC-M3-04-1 (d): after 3 retries exhausted on Monday (at 13:20) nothing happens at 14:00; the
    // next attempt is the regular Tuesday 12:00 slot.
    let rs_exhausted = BackupRuntimeState {
        backoff_attempt: 4,
        backoff_until: None,
        last_failure_at: Some(zdt(2026, 11, 16, 13, 20)),
        ..Default::default()
    };
    let same_day = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &rs_exhausted,
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        mon_1400,
        &zurich(),
    );
    assert!(!same_day.should_start_now);
    assert!(same_day.backoff_exhausted_until_next_slot);

    let next_day = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &rs_exhausted,
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        tue_1200,
        &zurich(),
    );
    assert!(next_day.should_start_now);
    assert!(!next_day.backoff_exhausted_until_next_slot);
}

/// AC-M3-07-4 (c): a manual resume stores the overdue anchor, so only slots after the resume count.
#[test]
fn ac_m3_07_4_manual_resume_resets_overdue_anchor() {
    let (_dest, bk) = make_backup(
        "Resume",
        Schedule::Daily {
            hour: 12,
            minute: 0,
        },
    );
    let sun_1200 = zdt(2026, 11, 15, 12, 0);
    let dir = TempDir::new().expect("tempdir");
    let history = RunHistory::open(dir.path()).expect("history");
    let thu_1000 = zdt(2026, 11, 19, 10, 0);
    let rs_resumed = BackupRuntimeState {
        overdue_anchor_at: Some(thu_1000),
        last_failure_at: Some(zdt(2026, 11, 16, 13, 20)),
        ..Default::default()
    };
    history
        .upsert_backup_runtime_state(&bk.id, &rs_resumed)
        .expect("upsert");
    let loaded = history.get_backup_runtime_state(&bk.id).expect("load");
    assert_eq!(loaded.overdue_anchor_at, Some(thu_1000));
    assert_eq!(loaded.last_failure_at, rs_resumed.last_failure_at);

    let after_resume = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup: &bk,
            runtime_state: &loaded,
            last_success_at: Some(sun_1200),
            overdue_anchor_at: None,
            global_pause: None,
            global_overdue_threshold: 2,
            source_available: true,
            destination_reachable: true,
            destination_busy: false,
            has_permanent_error: false,
        },
        &SystemEnvironment::desktop_connected(),
        zdt(2026, 11, 19, 12, 0), // 1 slot after resume (< threshold 2)
        &zurich(),
    );
    assert!(!after_resume.is_overdue);
    assert_eq!(after_resume.missed_slots_for_overdue, 1);
}

#[test]
fn pause_duration_u32_max_is_rejected_and_compute_pause_until_never_panics() {
    use kestic_core::scheduler::{MAX_PAUSE_HOURS, validate_pause_duration};
    assert!(validate_pause_duration(PauseDuration::Hours { hours: 0 }).is_err());
    assert!(validate_pause_duration(PauseDuration::Hours { hours: u32::MAX }).is_err());
    assert!(
        validate_pause_duration(PauseDuration::Hours {
            hours: MAX_PAUSE_HOURS
        })
        .is_ok()
    );

    let now = zdt(2026, 11, 16, 12, 0);
    let clamped = compute_pause_until(
        now,
        PauseDuration::Hours { hours: u32::MAX },
        &[],
        &zurich(),
    )
    .expect("finite pause_until");
    assert!(clamped > now);
}

#[test]
fn failed_maintenance_respects_backoff_and_does_not_restart_every_tick() {
    use kestic_core::history::DestinationMaintenanceState;
    use kestic_core::model::MaintenanceEntrySchedule;
    use std::path::PathBuf;
    let mut dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: PathBuf::from("/tmp/kestic-maint-backoff-r"),
        },
    )
    .unwrap();
    dest.maintenance = MaintenanceSchedule {
        prune: MaintenanceEntrySchedule::Daily {
            hour: 12,
            minute: 0,
        },
        check: MaintenanceEntrySchedule::ManualOnly,
        ..MaintenanceSchedule::default()
    };

    let sun_1100 = zdt(2026, 11, 15, 11, 0);
    let sun_1200 = zdt(2026, 11, 15, 12, 0);
    let sun_1201 = zdt(2026, 11, 15, 12, 1);
    let sun_1205 = zdt(2026, 11, 15, 12, 5);
    let mon_1200 = zdt(2026, 11, 16, 12, 0);

    let st_backoff = DestinationMaintenanceState {
        baseline_at: Some(sun_1100),
        last_prune_at: None,
        last_check_at: None,
        maintenance_backoff_attempt: 1,
        maintenance_backoff_until: Some(sun_1205),
        last_maintenance_failure_at: Some(sun_1200),
    };
    let need_1201 = evaluate_destination_maintenance(
        &DestinationMaintenanceEvalInput {
            destination: &dest,
            state: &st_backoff,
            has_pending_forget: false,
            global_pause: None,
            destination_busy: false,
            destination_reachable: true,
        },
        &SystemEnvironment::desktop_connected(),
        sun_1201,
        &zurich(),
    );
    assert!(need_1201.in_backoff);
    assert!(!need_1201.due_now);

    let need_1205 = evaluate_destination_maintenance(
        &DestinationMaintenanceEvalInput {
            destination: &dest,
            state: &st_backoff,
            has_pending_forget: false,
            global_pause: None,
            destination_busy: false,
            destination_reachable: true,
        },
        &SystemEnvironment::desktop_connected(),
        sun_1205,
        &zurich(),
    );
    assert!(!need_1205.in_backoff);
    assert!(need_1205.due_now);

    let st_exhausted = DestinationMaintenanceState {
        baseline_at: Some(sun_1100),
        last_prune_at: None,
        last_check_at: None,
        maintenance_backoff_attempt: 4,
        maintenance_backoff_until: None,
        last_maintenance_failure_at: Some(sun_1205),
    };
    let need_same_day = evaluate_destination_maintenance(
        &DestinationMaintenanceEvalInput {
            destination: &dest,
            state: &st_exhausted,
            has_pending_forget: false,
            global_pause: None,
            destination_busy: false,
            destination_reachable: true,
        },
        &SystemEnvironment::desktop_connected(),
        sun_1205,
        &zurich(),
    );
    assert!(!need_same_day.due_now);

    let need_next_slot = evaluate_destination_maintenance(
        &DestinationMaintenanceEvalInput {
            destination: &dest,
            state: &st_exhausted,
            has_pending_forget: false,
            global_pause: None,
            destination_busy: false,
            destination_reachable: true,
        },
        &SystemEnvironment::desktop_connected(),
        mon_1200,
        &zurich(),
    );
    assert!(need_next_slot.due_now);
}

/// AC-M3-14-2 (FR-03, negative, unit part): "on weekdays" without a weekday and "every 0 days" are
/// rejected by `Schedule::validate` (the editor blocks saving), a valid schedule passes.
#[test]
fn ac_m3_14_2_schedule_validate_rejects_empty_weekdays_and_zero_day_interval() {
    let no_weekday = Schedule::SpecificWeekdays {
        weekdays: Vec::new(),
        hour: 20,
        minute: 0,
    };
    assert!(
        no_weekday.validate().is_err(),
        "(a) weekday schedule without a weekday must be rejected"
    );
    let every_zero_days = Schedule::EveryNDays {
        interval_days: 0,
        start_date: "2026-12-01".to_string(),
        hour: 9,
        minute: 0,
    };
    assert!(
        every_zero_days.validate().is_err(),
        "(a) \"every 0 days\" must be rejected"
    );
    let valid = Schedule::SpecificWeekdays {
        weekdays: vec![Weekday::Monday],
        hour: 20,
        minute: 0,
    };
    assert!(valid.validate().is_ok(), "control: a valid schedule passes");
}
