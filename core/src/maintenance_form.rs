// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The fields of the maintenance editor: read from a [`MaintenanceSchedule`], written back to
//! one and compared with the saved one.

use crate::model::{MaintenanceEntrySchedule, MaintenanceSchedule, NthWeekday, Weekday};
use serde::{Deserialize, Serialize};

/// How often one maintenance task runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceFormKind {
    ManualOnly,
    Daily,
    Weekly,
    Monthly,
}

/// The fields of one maintenance task (clean up or verify). Fields the chosen kind does not use
/// keep their value, so switching back shows them again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceEntryForm {
    pub kind: MaintenanceFormKind,
    pub nth: NthWeekday,
    pub weekday: Weekday,
    pub hour: u8,
    pub minute: u8,
}

/// The fields of the maintenance editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceForm {
    pub prune: MaintenanceEntryForm,
    pub check: MaintenanceEntryForm,
}

/// What the editor shows for its current fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MaintenanceFormState {
    /// The schedule the fields stand for.
    pub maintenance: MaintenanceSchedule,
    /// Clean up or verify differs from the saved schedule.
    pub dirty: bool,
    /// Clean up differs from the saved schedule: saving it asks for a confirmation.
    pub prune_dirty: bool,
}

impl MaintenanceEntryForm {
    /// The fields for `entry`; a field the entry does not have offers the first Sunday at 12:00.
    pub fn from_entry(entry: &MaintenanceEntrySchedule) -> Self {
        let base = Self {
            kind: MaintenanceFormKind::ManualOnly,
            nth: NthWeekday::First,
            weekday: Weekday::Sunday,
            hour: 12,
            minute: 0,
        };
        match *entry {
            MaintenanceEntrySchedule::ManualOnly => base,
            MaintenanceEntrySchedule::Daily { hour, minute } => Self {
                kind: MaintenanceFormKind::Daily,
                hour,
                minute,
                ..base
            },
            MaintenanceEntrySchedule::Weekly {
                weekday,
                hour,
                minute,
            } => Self {
                kind: MaintenanceFormKind::Weekly,
                weekday,
                hour,
                minute,
                ..base
            },
            MaintenanceEntrySchedule::Monthly {
                nth,
                weekday,
                hour,
                minute,
            } => Self {
                kind: MaintenanceFormKind::Monthly,
                nth,
                weekday,
                hour,
                minute,
            },
        }
    }

    /// The entry the fields stand for.
    pub fn to_entry(&self) -> MaintenanceEntrySchedule {
        let Self {
            nth,
            weekday,
            hour,
            minute,
            ..
        } = *self;
        match self.kind {
            MaintenanceFormKind::ManualOnly => MaintenanceEntrySchedule::ManualOnly,
            MaintenanceFormKind::Daily => MaintenanceEntrySchedule::Daily { hour, minute },
            MaintenanceFormKind::Weekly => MaintenanceEntrySchedule::Weekly {
                weekday,
                hour,
                minute,
            },
            MaintenanceFormKind::Monthly => MaintenanceEntrySchedule::Monthly {
                nth,
                weekday,
                hour,
                minute,
            },
        }
    }
}

impl MaintenanceForm {
    /// The fields for `schedule`.
    pub fn from_schedule(schedule: &MaintenanceSchedule) -> Self {
        Self {
            prune: MaintenanceEntryForm::from_entry(&schedule.prune),
            check: MaintenanceEntryForm::from_entry(&schedule.check),
        }
    }

    /// The schedule the fields stand for and whether it differs from `saved`.
    pub fn state(&self, saved: &MaintenanceSchedule) -> MaintenanceFormState {
        let prune = self.prune.to_entry();
        let check = self.check.to_entry();
        let prune_dirty = prune != saved.prune;
        MaintenanceFormState {
            dirty: prune_dirty || check != saved.check,
            prune_dirty,
            maintenance: MaintenanceSchedule {
                prune,
                check,
                ..MaintenanceSchedule::default()
            },
        }
    }
}

#[cfg(test)]
#[path = "maintenance_form_tests.rs"]
mod tests;
