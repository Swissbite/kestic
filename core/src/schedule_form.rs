// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The fields of the schedule editor: read from a [`Schedule`] and its overdue threshold,
//! written back to them and compared with the saved ones.

use crate::model::Weekday;
use crate::retention::{EVERY_N_DAYS_RANGE, Schedule};
use serde::{Deserialize, Serialize};

/// The interval the editor offers for «every n days» before the user picks one.
pub const DEFAULT_EVERY_N_DAYS: u32 = 3;

/// How often the editor runs a backup. A weekly schedule is shown as one weekday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleFormKind {
    Daily,
    SpecificWeekdays,
    EveryNDays,
    ManualOnly,
}

/// The fields of the schedule editor. Fields the chosen kind does not use keep their value, so
/// switching back shows them again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleForm {
    pub kind: ScheduleFormKind,
    pub weekdays: Vec<Weekday>,
    pub interval_days: u32,
    /// `YYYY-MM-DD`, as typed or picked; checked by [`Schedule::validate`].
    pub start_date: String,
    pub hour: u8,
    pub minute: u8,
    /// The backup uses the global overdue threshold.
    pub use_global_overdue: bool,
    /// The backup's own threshold, used when `use_global_overdue` is off.
    pub custom_overdue_threshold: u32,
}

/// A calendar day for the date picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PickerDate {
    pub year: i16,
    pub month: i8,
    pub day: i8,
}

/// What the editor shows for its current fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScheduleFormState {
    /// The schedule the fields stand for (not yet validated).
    pub schedule: Schedule,
    /// The backup's overdue threshold; 0 = the global one.
    pub overdue_threshold: u32,
    /// The schedule or the threshold differs from the saved one.
    pub dirty: bool,
    /// The day the date picker opens on: the start date, or `today` when it is not a date.
    pub picker_date: PickerDate,
    /// The longest interval «every n days» allows; the page's field goes up to it.
    pub interval_days_max: u32,
}

impl Default for ScheduleForm {
    /// The fields the editor starts with: daily at 12:00 with the global overdue threshold.
    fn default() -> Self {
        Self {
            kind: ScheduleFormKind::Daily,
            weekdays: vec![Weekday::Monday],
            interval_days: DEFAULT_EVERY_N_DAYS,
            start_date: String::new(),
            hour: 12,
            minute: 0,
            use_global_overdue: true,
            custom_overdue_threshold: 2,
        }
    }
}

/// The weekday of an ISO number; Monday for a number out of range.
fn weekday_or_monday(iso: u8) -> Weekday {
    Weekday::from_iso_number(iso).unwrap_or(Weekday::Monday)
}

/// `schedule` with a weekly schedule as one weekday, the weekdays in order and the start date
/// trimmed (`today` when empty), so two schedules that run alike compare equal.
fn normalized(schedule: &Schedule, today: &str) -> Schedule {
    match schedule {
        Schedule::Weekly {
            weekday,
            hour,
            minute,
        } => Schedule::SpecificWeekdays {
            weekdays: vec![weekday_or_monday(*weekday)],
            hour: *hour,
            minute: *minute,
        },
        Schedule::SpecificWeekdays {
            weekdays,
            hour,
            minute,
        } => {
            let mut weekdays = weekdays.clone();
            weekdays.sort();
            Schedule::SpecificWeekdays {
                weekdays,
                hour: *hour,
                minute: *minute,
            }
        }
        Schedule::EveryNDays {
            interval_days,
            start_date,
            hour,
            minute,
        } => {
            let start = if start_date.is_empty() {
                today
            } else {
                start_date
            };
            Schedule::EveryNDays {
                interval_days: *interval_days,
                start_date: start.trim().to_string(),
                hour: *hour,
                minute: *minute,
            }
        }
        other => other.clone(),
    }
}

fn picker_date(raw: &str) -> Option<PickerDate> {
    let date = raw.trim().parse::<jiff::civil::Date>().ok()?;
    Some(PickerDate {
        year: date.year(),
        month: date.month(),
        day: date.day(),
    })
}

impl ScheduleForm {
    /// The fields after loading `schedule` and its `overdue_threshold` (0 = global) over
    /// `self`: only the fields of the schedule's kind change. An empty start date becomes
    /// `today`; without an own threshold the field offers `default_overdue` (at least 1).
    pub fn load(
        mut self,
        schedule: &Schedule,
        overdue_threshold: u32,
        today: &str,
        default_overdue: u32,
    ) -> Self {
        match schedule {
            Schedule::ManualOnly => {
                self.kind = ScheduleFormKind::ManualOnly;
                self.hour = 12;
                self.minute = 0;
            }
            Schedule::Daily { hour, minute } => {
                self.kind = ScheduleFormKind::Daily;
                (self.hour, self.minute) = (*hour, *minute);
            }
            Schedule::Weekly {
                weekday,
                hour,
                minute,
            } => {
                self.kind = ScheduleFormKind::SpecificWeekdays;
                self.weekdays = vec![weekday_or_monday(*weekday)];
                (self.hour, self.minute) = (*hour, *minute);
            }
            Schedule::SpecificWeekdays {
                weekdays,
                hour,
                minute,
            } => {
                self.kind = ScheduleFormKind::SpecificWeekdays;
                self.weekdays = if weekdays.is_empty() {
                    vec![Weekday::Monday]
                } else {
                    weekdays.clone()
                };
                (self.hour, self.minute) = (*hour, *minute);
            }
            Schedule::EveryNDays {
                interval_days,
                start_date,
                hour,
                minute,
            } => {
                self.kind = ScheduleFormKind::EveryNDays;
                self.interval_days = *interval_days;
                self.start_date.clone_from(start_date);
                (self.hour, self.minute) = (*hour, *minute);
            }
        }
        if self.start_date.is_empty() {
            self.start_date = today.to_string();
        }
        self.use_global_overdue = overdue_threshold == 0;
        self.custom_overdue_threshold = if overdue_threshold > 0 {
            overdue_threshold
        } else {
            default_overdue.max(1)
        };
        self
    }

    /// The schedule the fields stand for, with the start date trimmed.
    pub fn to_schedule(&self) -> Schedule {
        let (hour, minute) = (self.hour, self.minute);
        match self.kind {
            ScheduleFormKind::ManualOnly => Schedule::ManualOnly,
            ScheduleFormKind::Daily => Schedule::Daily { hour, minute },
            ScheduleFormKind::SpecificWeekdays => Schedule::SpecificWeekdays {
                weekdays: self.weekdays.clone(),
                hour,
                minute,
            },
            ScheduleFormKind::EveryNDays => Schedule::EveryNDays {
                interval_days: self.interval_days,
                start_date: self.start_date.trim().to_string(),
                hour,
                minute,
            },
        }
    }

    /// The backup's overdue threshold; 0 = the global one.
    pub fn overdue_threshold(&self) -> u32 {
        if self.use_global_overdue {
            0
        } else {
            self.custom_overdue_threshold
        }
    }

    /// The schedule, the threshold and whether they differ from the saved ones.
    pub fn state(&self, saved: &Schedule, saved_overdue: u32, today: &str) -> ScheduleFormState {
        let schedule = self.to_schedule();
        let overdue_threshold = self.overdue_threshold();
        let fallback = picker_date(today).unwrap_or(PickerDate {
            year: 1970,
            month: 1,
            day: 1,
        });
        ScheduleFormState {
            dirty: normalized(&schedule, today) != normalized(saved, today)
                || overdue_threshold != saved_overdue,
            picker_date: picker_date(&self.start_date).unwrap_or(fallback),
            interval_days_max: *EVERY_N_DAYS_RANGE.end(),
            schedule,
            overdue_threshold,
        }
    }
}

#[cfg(test)]
#[path = "schedule_form_tests.rs"]
mod tests;
