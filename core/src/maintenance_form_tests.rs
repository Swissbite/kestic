// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::maintenance::PrunePreviewSummary;

fn schedule(json: &str) -> MaintenanceSchedule {
    serde_json::from_str(json).expect("maintenance json")
}

/// AC-M6-34-2 (a) and (c): loading a saved schedule gives the fields the QML showed
/// (`normalizeEntry`): a field the entry does not have offers the first Sunday at 12:00.
#[test]
fn ac_m6_34_2_maintenance_form_loads_like_former_qml() {
    let form = MaintenanceForm::from_schedule(&MaintenanceSchedule::default());
    assert_eq!(
        form.prune,
        MaintenanceEntryForm {
            kind: MaintenanceFormKind::Weekly,
            nth: NthWeekday::First,
            weekday: Weekday::Sunday,
            hour: 12,
            minute: 0
        },
        "(c) default clean up"
    );
    assert_eq!(
        form.check,
        MaintenanceEntryForm {
            kind: MaintenanceFormKind::Monthly,
            nth: NthWeekday::First,
            weekday: Weekday::Sunday,
            hour: 12,
            minute: 0
        },
        "(c) default verify"
    );

    let form = MaintenanceForm::from_schedule(&schedule(
        r#"{"prune":{"kind":"daily","hour":3,"minute":15},
            "check":{"kind":"manual_only"}}"#,
    ));
    assert_eq!(form.prune.kind, MaintenanceFormKind::Daily, "(c) daily");
    assert_eq!(
        (form.prune.hour, form.prune.minute),
        (3, 15),
        "(c) its time"
    );
    assert_eq!(form.prune.weekday, Weekday::Sunday, "(c) offers Sunday");
    assert_eq!(
        form.check.kind,
        MaintenanceFormKind::ManualOnly,
        "(c) manual"
    );
    assert_eq!(
        (form.check.hour, form.check.minute),
        (12, 0),
        "(c) offers 12:00"
    );
}

/// AC-M6-34-2 (a) and (c): the fields give the schedule the QML built (`buildEntryObj`), and
/// clean up and verify count as changed on their own.
#[test]
fn ac_m6_34_2_maintenance_form_state_detects_changes() {
    let saved = MaintenanceSchedule::default();
    let mut form = MaintenanceForm::from_schedule(&saved);
    let state = form.state(&saved);
    assert!(!state.dirty, "(c) unchanged");
    assert!(!state.prune_dirty, "(c) clean up unchanged");
    assert_eq!(state.maintenance, saved, "(a) same schedule");

    form.check.nth = NthWeekday::Last;
    let state = form.state(&saved);
    assert!(state.dirty, "(c) verify changed");
    assert!(!state.prune_dirty, "(c) clean up still unchanged");

    let mut form = MaintenanceForm::from_schedule(&saved);
    form.prune.kind = MaintenanceFormKind::Daily;
    form.prune.nth = NthWeekday::Third;
    let state = form.state(&saved);
    assert!(state.prune_dirty, "(c) clean up changed");
    assert_eq!(
        serde_json::to_value(&state.maintenance.prune).expect("json"),
        serde_json::json!({"kind": "daily", "hour": 12, "minute": 0}),
        "(c) daily drops weekday and nth"
    );

    form.prune.kind = MaintenanceFormKind::Weekly;
    form.prune.nth = NthWeekday::Third;
    assert!(
        !form.state(&saved).prune_dirty,
        "(c) a hidden nth of a weekly entry does not count"
    );

    let with_intervals = schedule(
        r#"{"prune":{"kind":"manual_only"},"check":{"kind":"manual_only"},
            "prune_interval_days":7,"check_interval_days":30}"#,
    );
    let state = MaintenanceForm::from_schedule(&with_intervals).state(&with_intervals);
    assert!(!state.dirty, "(c) intervals are not compared");
    assert_eq!(
        (
            state.maintenance.prune_interval_days,
            state.maintenance.check_interval_days
        ),
        (None, None),
        "(c) the editor saves clean up and verify only"
    );
}

/// AC-M6-34-2 (a): the preview names only the size of restic's English summary line.
#[test]
fn ac_m6_34_2_prune_preview_freed_size() {
    let size = |line: &str| {
        PrunePreviewSummary {
            freed_summary: Some(line.to_string()),
            ..PrunePreviewSummary::default()
        }
        .freed_size()
    };
    assert_eq!(
        size("to delete: 12 packs (3.4 MiB)").as_deref(),
        Some("3.4 MiB"),
        "(a) packs and size"
    );
    assert_eq!(size("will remove 0 B").as_deref(), Some("0 B"), "(a) bytes");
    assert_eq!(
        size("unused size: 7GiB").as_deref(),
        Some("7GiB"),
        "(a) no blank"
    );
    assert_eq!(
        size("v1.2.3 TiB").as_deref(),
        Some("2.3 TiB"),
        "(a) the first number that has a unit"
    );
    assert_eq!(size("to delete: 3 packs").as_deref(), None, "(a) no size");
    assert_eq!(size("12  MiB").as_deref(), None, "(a) two blanks");
    assert_eq!(
        PrunePreviewSummary::default().freed_size(),
        None,
        "(a) no line"
    );
}
