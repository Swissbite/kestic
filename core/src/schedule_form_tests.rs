// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

const TODAY: &str = "2026-11-28";

fn every(interval_days: u32, start_date: &str) -> Schedule {
    Schedule::EveryNDays {
        interval_days,
        start_date: start_date.to_string(),
        hour: 8,
        minute: 30,
    }
}

/// AC-M6-34-2 (a) and (c): loading a saved schedule sets the fields of its kind as the QML did
/// (`loadFromSaved`): a weekly schedule becomes one weekday, no weekday becomes Monday, a manual
/// schedule resets the time to 12:00, an empty start date becomes today, and the other fields
/// keep their value.
#[test]
fn ac_m6_34_2_schedule_form_loads_like_former_qml() {
    let initial = ScheduleForm::default();
    assert_eq!(initial.kind, ScheduleFormKind::Daily, "(c) initial kind");
    assert_eq!(initial.weekdays, vec![Weekday::Monday], "(c) initial weekday");
    assert_eq!((initial.hour, initial.minute), (12, 0), "(c) initial time");

    let weekly = ScheduleForm::default().load(
        &Schedule::Weekly {
            weekday: 3,
            hour: 7,
            minute: 5,
        },
        0,
        TODAY,
        2,
    );
    assert_eq!(weekly.kind, ScheduleFormKind::SpecificWeekdays, "(c) weekly");
    assert_eq!(weekly.weekdays, vec![Weekday::Wednesday], "(c) its weekday");
    assert_eq!((weekly.hour, weekly.minute), (7, 5), "(c) its time");
    let weekly_zero = ScheduleForm::default().load(
        &Schedule::Weekly {
            weekday: 0,
            hour: 7,
            minute: 5,
        },
        0,
        TODAY,
        2,
    );
    assert_eq!(weekly_zero.weekdays, vec![Weekday::Monday], "(c) weekday 0");

    let no_days = ScheduleForm::default().load(
        &Schedule::SpecificWeekdays {
            weekdays: vec![],
            hour: 9,
            minute: 0,
        },
        0,
        TODAY,
        2,
    );
    assert_eq!(no_days.weekdays, vec![Weekday::Monday], "(c) no weekday");

    let every_n = ScheduleForm::default().load(&every(10, ""), 0, TODAY, 2);
    assert_eq!(every_n.kind, ScheduleFormKind::EveryNDays, "(c) every n days");
    assert_eq!(every_n.interval_days, 10, "(c) interval");
    assert_eq!(every_n.start_date, TODAY, "(c) empty start date → today");

    let mut edited = ScheduleForm::default().load(&every(10, "2026-01-02"), 0, TODAY, 2);
    edited.hour = 22;
    let manual = edited.load(&Schedule::ManualOnly, 0, TODAY, 2);
    assert_eq!(manual.kind, ScheduleFormKind::ManualOnly, "(c) manual");
    assert_eq!((manual.hour, manual.minute), (12, 0), "(c) manual resets the time");
    assert_eq!(manual.interval_days, 10, "(c) hidden interval kept");
    assert_eq!(manual.start_date, "2026-01-02", "(c) hidden start date kept");

    let own = ScheduleForm::default().load(&Schedule::ManualOnly, 5, TODAY, 2);
    assert!(!own.use_global_overdue, "(c) own threshold");
    assert_eq!(own.custom_overdue_threshold, 5, "(c) its value");
    let global = ScheduleForm::default().load(&Schedule::ManualOnly, 0, TODAY, 0);
    assert!(global.use_global_overdue, "(c) global threshold");
    assert_eq!(global.custom_overdue_threshold, 1, "(c) offers at least 1");
}

/// AC-M6-34-2 (a) and (c): the fields give the schedule the QML built (`currentScheduleObj`).
#[test]
fn ac_m6_34_2_schedule_form_writes_like_former_qml() {
    let mut form = ScheduleForm::default();
    assert_eq!(
        form.to_schedule(),
        Schedule::Daily {
            hour: 12,
            minute: 0
        },
        "(c) daily"
    );
    form.kind = ScheduleFormKind::EveryNDays;
    form.start_date = " 2026-03-04 ".to_string();
    assert_eq!(
        serde_json::to_value(form.to_schedule()).expect("json"),
        serde_json::json!({
            "kind": "every_n_days",
            "interval_days": DEFAULT_EVERY_N_DAYS,
            "start_date": "2026-03-04",
            "hour": 12,
            "minute": 0
        }),
        "(c) every n days, start date trimmed"
    );
    form.kind = ScheduleFormKind::SpecificWeekdays;
    form.weekdays = vec![Weekday::Friday, Weekday::Monday];
    assert_eq!(
        serde_json::to_value(form.to_schedule()).expect("json"),
        serde_json::json!({
            "kind": "specific_weekdays",
            "weekdays": ["friday", "monday"],
            "hour": 12,
            "minute": 0
        }),
        "(c) weekdays in the order picked"
    );
    form.kind = ScheduleFormKind::ManualOnly;
    assert_eq!(form.to_schedule(), Schedule::ManualOnly, "(c) manual");
    form.use_global_overdue = false;
    form.custom_overdue_threshold = 4;
    assert_eq!(form.overdue_threshold(), 4, "(c) own threshold");
    form.use_global_overdue = true;
    assert_eq!(form.overdue_threshold(), 0, "(c) global threshold");
}

/// AC-M6-34-2 (a) and (c): the editor counts as changed where the QML's normalised schedules
/// differed: a weekly schedule equals its one weekday, the order of weekdays does not count,
/// nor does a missing start date that is today; a changed threshold counts.
#[test]
fn ac_m6_34_2_schedule_form_state_detects_changes() {
    let weekly = Schedule::Weekly {
        weekday: 5,
        hour: 6,
        minute: 0,
    };
    let form = ScheduleForm::default().load(&weekly, 0, TODAY, 2);
    assert!(!form.state(&weekly, 0, TODAY).dirty, "(c) weekly = one weekday");
    assert!(form.state(&weekly, 3, TODAY).dirty, "(c) threshold changed");

    let days = Schedule::SpecificWeekdays {
        weekdays: vec![Weekday::Sunday, Weekday::Monday],
        hour: 6,
        minute: 0,
    };
    let mut form = ScheduleForm::default().load(&days, 0, TODAY, 2);
    form.weekdays = vec![Weekday::Monday, Weekday::Sunday];
    assert!(!form.state(&days, 0, TODAY).dirty, "(c) order of weekdays");
    form.weekdays.push(Weekday::Tuesday);
    assert!(form.state(&days, 0, TODAY).dirty, "(c) a weekday more");

    let saved = every(4, "");
    let form = ScheduleForm::default().load(&saved, 0, TODAY, 2);
    assert!(!form.state(&saved, 0, TODAY).dirty, "(c) empty start = today");
    let mut form = form;
    form.interval_days = 5;
    let state = form.state(&saved, 0, TODAY);
    assert!(state.dirty, "(c) interval changed");
    assert_eq!(
        state.schedule,
        Schedule::EveryNDays {
            interval_days: 5,
            start_date: TODAY.to_string(),
            hour: 8,
            minute: 30
        },
        "(a) the schedule of the fields"
    );
}

/// AC-M6-34-2 (a): the date picker opens on the start date, or on today when it is no date.
#[test]
fn ac_m6_34_2_schedule_form_picker_date() {
    let mut form = ScheduleForm::default();
    form.start_date = "2027-02-03".to_string();
    assert_eq!(
        form.state(&Schedule::ManualOnly, 0, TODAY).picker_date,
        PickerDate {
            year: 2027,
            month: 2,
            day: 3
        },
        "(a) start date"
    );
    form.start_date = "soon".to_string();
    assert_eq!(
        form.state(&Schedule::ManualOnly, 0, TODAY).picker_date,
        PickerDate {
            year: 2026,
            month: 11,
            day: 28
        },
        "(a) today"
    );
}
