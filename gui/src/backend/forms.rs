// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The editor pages' fields through `core`: retention, schedule, exclude rules and maintenance
//! read their fields from a saved value, and learn what their current fields stand for.

use super::qobject;
use crate::dto_builders::json_or;
use crate::i18n::i18n;
use crate::time_texts::cached_today_iso_date;
use cxx_qt_lib::QString;
use kestic_core::exclude_form::{ExcludeForm, apply_lint_fix};
use kestic_core::maintenance::PrunePreviewSummary;
use kestic_core::maintenance_form::MaintenanceForm;
use kestic_core::model::{ExcludeRules, MaintenanceSchedule, RetentionPolicy, Schedule};
use kestic_core::retention_form::RetentionForm;
use kestic_core::schedule_form::ScheduleForm;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// `json` read as `T`; `None` when it is not one.
fn parsed<T: DeserializeOwned>(json: &QString) -> Option<T> {
    serde_json::from_str(&json.to_string()).ok()
}

/// `value` as JSON for QML.
fn to_qml<T: Serialize>(value: &T) -> QString {
    QString::from(&json_or(value, "{}"))
}

/// A saved retention policy; one that cannot be read has no rules.
fn saved_policy(json: &QString) -> RetentionPolicy {
    parsed(json).unwrap_or_else(RetentionPolicy::empty)
}

/// A saved schedule; one that cannot be read runs daily at 12:00.
fn saved_schedule(json: &QString) -> Schedule {
    parsed(json).unwrap_or_else(Schedule::default_scheduled)
}

/// A count of 0 or below from QML (the global threshold) as 0.
fn threshold(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

impl qobject::KesticApp {
    pub fn retention_form_json(&self, policy_json: &QString) -> QString {
        to_qml(&RetentionForm::from_policy(&saved_policy(policy_json)))
    }

    pub fn retention_form_state(&self, form_json: &QString, saved_json: &QString) -> QString {
        match parsed::<RetentionForm>(form_json) {
            Some(form) => to_qml(&form.state(&saved_policy(saved_json))),
            None => QString::from("{}"),
        }
    }

    /// The fields after loading `schedule_json` with its threshold over `form_json` (empty:
    /// the editor's first fields).
    pub fn schedule_form_json(
        &self,
        schedule_json: &QString,
        overdue_threshold: i32,
        form_json: &QString,
    ) -> QString {
        let base = parsed::<ScheduleForm>(form_json).unwrap_or_default();
        to_qml(&base.load(
            &saved_schedule(schedule_json),
            threshold(overdue_threshold),
            &cached_today_iso_date(),
            threshold(*self.default_overdue_threshold()),
        ))
    }

    pub fn schedule_form_state(
        &self,
        form_json: &QString,
        saved_json: &QString,
        saved_overdue_threshold: i32,
    ) -> QString {
        match parsed::<ScheduleForm>(form_json) {
            Some(form) => to_qml(&form.state(
                &saved_schedule(saved_json),
                threshold(saved_overdue_threshold),
                &cached_today_iso_date(),
            )),
            None => QString::from("{}"),
        }
    }

    pub fn exclude_form_json(&self, excludes_json: &QString) -> QString {
        to_qml(&ExcludeForm::from_rules(
            &parsed(excludes_json).unwrap_or_default(),
        ))
    }

    pub fn exclude_form_state(&self, form_json: &QString, saved_json: &QString) -> QString {
        match parsed::<ExcludeForm>(form_json) {
            Some(form) => {
                to_qml(&form.state(&parsed::<ExcludeRules>(saved_json).unwrap_or_default()))
            }
            None => QString::from("{}"),
        }
    }

    pub fn exclude_lint_fix(
        &self,
        lines_text: &QString,
        parent_rule: &QString,
        suggested_fix: &QString,
    ) -> QString {
        QString::from(&apply_lint_fix(
            &lines_text.to_string(),
            &parent_rule.to_string(),
            &suggested_fix.to_string(),
        ))
    }

    pub fn maintenance_form_json(&self, maintenance_json: &QString) -> QString {
        to_qml(&MaintenanceForm::from_schedule(
            &parsed(maintenance_json).unwrap_or_default(),
        ))
    }

    pub fn maintenance_form_state(&self, form_json: &QString, saved_json: &QString) -> QString {
        match parsed::<MaintenanceForm>(form_json) {
            Some(form) => {
                to_qml(&form.state(&parsed::<MaintenanceSchedule>(saved_json).unwrap_or_default()))
            }
            None => QString::from("{}"),
        }
    }

    /// The prune preview `summary_json` as text lines; empty or unreadable: the preview failed.
    pub fn maintenance_preview_text(&self, summary_json: &QString) -> QString {
        QString::from(&prune_preview_text(parsed(summary_json)))
    }
}

/// The lines the confirmation shows for a prune preview; `None`: the preview failed.
fn prune_preview_text(summary: Option<PrunePreviewSummary>) -> String {
    let Some(summary) = summary else {
        return i18n("The preview could not read the repository. It may be unreachable or locked.")
            .to_string();
    };
    let mut parts = Vec::new();
    if let Some(n) = summary.delete_packs {
        parts.push(i18n("Data blocks to remove: %1").subs(&[&n.to_string()]));
    }
    if let Some(n) = summary.keep_packs {
        parts.push(i18n("Data blocks to keep: %1").subs(&[&n.to_string()]));
    }
    if let Some(size) = summary.freed_size() {
        parts.push(i18n("Space to free: %1").subs(&[&size]));
    }
    if parts.is_empty() {
        parts.push(i18n("The preview reported no data to remove.").to_string());
    }
    parts.join("\n")
}
