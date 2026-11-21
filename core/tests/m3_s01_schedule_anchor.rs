// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! AC-M3-01-8: a new or changed schedule counts from the moment it is saved; slots before the
//! change are not caught up (simulated clock, Europe/Zurich).

mod common;

use common::zurich::{zdt, zurich};
use kestic_core::history::{BackupRuntimeState, DestinationMaintenanceState};
use kestic_core::model::{
    Backup, Destination, DestinationKind, MaintenanceEntrySchedule, MaintenanceSchedule, Schedule,
    Source,
};
use kestic_core::scheduler::{
    BackupScheduleEvalInput, BackupScheduleEvaluation, DestinationMaintenanceEvalInput,
    DestinationMaintenanceNeed, SystemEnvironment, anchor_backup_schedule_at,
    anchor_changed_maintenance_schedules, evaluate_backup_schedule,
    evaluate_destination_maintenance,
};
use time::OffsetDateTime;

fn destination(maintenance: MaintenanceSchedule) -> Destination {
    let mut dest = Destination::new(
        "NAS",
        DestinationKind::Local {
            path: "/tmp/kestic-anchor-test-dest".into(),
        },
    )
    .expect("dest");
    dest.maintenance = maintenance;
    dest
}

fn maintenance_at(
    dest: &Destination,
    state: &DestinationMaintenanceState,
    now: OffsetDateTime,
) -> DestinationMaintenanceNeed {
    evaluate_destination_maintenance(
        &DestinationMaintenanceEvalInput {
            destination: dest,
            state,
            has_pending_forget: false,
            global_pause: None,
            destination_busy: false,
            destination_reachable: true,
        },
        &SystemEnvironment::desktop_connected(),
        now,
        &zurich(),
    )
}

fn backup_with(schedule: Schedule) -> Backup {
    let dest = destination(MaintenanceSchedule::default());
    let mut backup = Backup::new(
        "Documents",
        Source::folder("/tmp/kestic-anchor-test-src"),
        dest.id,
    )
    .expect("backup");
    backup.schedule = schedule;
    backup
}

fn backup_at(
    backup: &Backup,
    rs: &BackupRuntimeState,
    last_success_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> BackupScheduleEvaluation {
    evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup,
            runtime_state: rs,
            last_success_at,
            overdue_anchor_at: rs.overdue_anchor_at,
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

fn daily(hour: u8, minute: u8) -> MaintenanceEntrySchedule {
    MaintenanceEntrySchedule::Daily { hour, minute }
}

/// AC-M3-01-8 (a), (b): a destination registered the day before has never run `check`; at 16:31
/// «Verify: daily 16:32» is saved. Without the anchor the slot of the day before would count as
/// missed (counter-check); with it there is (a) no maintenance at 16:31 and (b) the first
/// `check` is due at 16:32.
#[test]
fn ac_m3_01_8_a_b_new_check_schedule_first_runs_at_its_slot() {
    let registered = zdt(2026, 11, 20, 10, 0);
    let saved = zdt(2026, 11, 21, 16, 31);
    let old = MaintenanceSchedule::default();
    let new = MaintenanceSchedule {
        check: daily(16, 32),
        ..MaintenanceSchedule::default()
    };
    let dest = destination(new.clone());
    let mut state = DestinationMaintenanceState {
        baseline_at: Some(registered),
        ..DestinationMaintenanceState::default()
    };
    assert!(
        maintenance_at(&dest, &state, saved).check_due,
        "counter-check: without the anchor yesterday's slot would be caught up"
    );

    anchor_changed_maintenance_schedules(&mut state, &old, &new, saved);
    let at_save = maintenance_at(&dest, &state, saved);
    assert!(!at_save.due_now, "(a) no maintenance at 16:31");
    let at_slot = maintenance_at(&dest, &state, zdt(2026, 11, 21, 16, 32));
    assert!(at_slot.check_due, "(b) first check due at 16:32");
}

/// AC-M3-01-8 (c), (d), (e): a backup «daily 12:00» that last succeeded yesterday 12:00 is
/// changed to «daily 14:00» today at 15:00. Without the anchor today's 14:00 would be caught up at
/// once and tomorrow would be overdue (counter-checks); with it (c) nothing starts at 15:00,
/// (d) the next run is due tomorrow at 14:00 and (e) the overdue count starts at the change, so
/// tomorrow's slot is the only missed one and the backup is not overdue.
#[test]
fn ac_m3_01_8_c_d_e_changed_backup_schedule_does_not_catch_up_at_once() {
    let last_success = zdt(2026, 11, 20, 12, 0);
    let changed = zdt(2026, 11, 21, 15, 0);
    let backup = backup_with(Schedule::Daily {
        hour: 14,
        minute: 0,
    });
    let mut rs = BackupRuntimeState {
        baseline_at: Some(zdt(2026, 11, 1, 9, 0)),
        ..BackupRuntimeState::default()
    };
    assert!(
        backup_at(&backup, &rs, Some(last_success), changed).should_start_now,
        "counter-check: without the anchor today's 14:00 would be caught up"
    );
    let tomorrow_1400 = zdt(2026, 11, 22, 14, 0);
    assert!(
        backup_at(&backup, &rs, Some(last_success), tomorrow_1400).is_overdue,
        "counter-check: without the anchor tomorrow would be overdue"
    );

    anchor_backup_schedule_at(&mut rs, changed);
    assert!(
        !backup_at(&backup, &rs, Some(last_success), changed).should_start_now,
        "(c) nothing starts at 15:00"
    );
    let tomorrow = backup_at(&backup, &rs, Some(last_success), tomorrow_1400);
    assert_eq!(
        tomorrow.due_slot,
        Some(tomorrow_1400),
        "(d) next run tomorrow at 14:00"
    );
    assert_eq!(
        tomorrow.missed_slots_for_overdue, 1,
        "(e) overdue count starts at the change"
    );
}

/// AC-M3-01-8 (f), (g): switching a backup from «manual only» to «daily 12:00» at 15:00 still
/// counts from the switch: (f) nothing starts at 15:00 and (g) the first run is due tomorrow at
/// 12:00.
#[test]
fn ac_m3_01_8_f_g_switch_from_manual_only_counts_from_the_switch() {
    let switched = zdt(2026, 11, 21, 15, 0);
    let backup = backup_with(Schedule::Daily {
        hour: 12,
        minute: 0,
    });
    let mut rs = BackupRuntimeState::default();
    anchor_backup_schedule_at(&mut rs, switched);
    assert!(
        !backup_at(&backup, &rs, None, switched).should_start_now,
        "(f) nothing starts at 15:00"
    );
    let tomorrow = backup_at(&backup, &rs, None, zdt(2026, 11, 22, 12, 0));
    assert_eq!(
        tomorrow.due_slot,
        Some(zdt(2026, 11, 22, 12, 0)),
        "(g) first run tomorrow at 12:00"
    );
}

/// AC-M3-01-8 (h): only the verify schedule changes on Monday 10:00 while Sunday's clean-up slot
/// was missed: the clean-up schedule keeps its anchor and still catches up its missed slot, while
/// the changed verify schedule is not due.
#[test]
fn ac_m3_01_8_h_unchanged_schedule_keeps_its_anchor() {
    let last_prune = zdt(2026, 11, 15, 12, 0);
    let changed = zdt(2026, 11, 23, 10, 0);
    let old = MaintenanceSchedule::default();
    let new = MaintenanceSchedule {
        check: daily(9, 0),
        ..MaintenanceSchedule::default()
    };
    let dest = destination(new.clone());
    let mut state = DestinationMaintenanceState {
        baseline_at: Some(zdt(2026, 11, 1, 9, 0)),
        last_prune_at: Some(last_prune),
        ..DestinationMaintenanceState::default()
    };
    anchor_changed_maintenance_schedules(&mut state, &old, &new, changed);
    let need = maintenance_at(&dest, &state, changed);
    assert!(need.prune_due, "(h) missed clean-up slot still caught up");
    assert!(!need.check_due, "changed verify schedule not due");
}
