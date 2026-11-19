// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Schedule/condition/maintenance editing, pause/resume controls, settings, run details,
//! clipboard, and quit confirmation for [`super::qobject::KesticApp`].

use super::{friendly_dbus_error, qobject};
use crate::smoke;
use crate::state_cache::{
    build_provider, cached_daemon_now, cached_destination_kind, cached_today_iso_date,
    clone_updated_backup_conditions, clone_updated_backup_schedule, format_bytes,
    format_conditions_summary, format_datetime_display, format_duration_ms,
    format_maintenance_summary, format_run_kind_name, format_run_status, format_schedule_summary,
    lookup_backup_summary, make_client, preview_next_schedule_slots, refresh_state_now,
    shared_state,
};
use cxx_kde_frameworks::ki18n::i18n;
use cxx_qt_lib::QString;
use kestic_core::model::{MaintenanceSchedule, NotificationSettings, RunConditions, Schedule};
use kestic_core::scheduler::PauseDuration;
use std::env;
use std::pin::Pin;

fn parse_pause_duration(kind: &str) -> PauseDuration {
    match kind {
        "1h" => PauseDuration::Hours { hours: 1 },
        "3h" => PauseDuration::Hours { hours: 3 },
        "tomorrow" => PauseDuration::UntilTomorrow,
        _ => PauseDuration::Indefinite,
    }
}

impl qobject::KesticApp {
    pub fn run_destination_maintenance(
        mut self: Pin<&mut Self>,
        destination_id: &QString,
    ) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.run_destination_maintenance(&destination_id.to_string()) {
            Ok(run_id) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                QString::from(&run_id)
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                QString::from("")
            }
        }
    }

    pub fn update_destination_maintenance(
        mut self: Pin<&mut Self>,
        dest_id: &QString,
        maintenance_json: &QString,
    ) -> bool {
        let maint: MaintenanceSchedule = match serde_json::from_str(&maintenance_json.to_string()) {
            Ok(m) => m,
            Err(err) => {
                self.as_mut()
                    .set_last_action_error(QString::from(&err.to_string()));
                return false;
            }
        };
        if let Err(err) = maint.validate() {
            self.as_mut()
                .set_last_action_error(QString::from(&err.to_string()));
            return false;
        }
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_destination_maintenance(&dest_id.to_string(), &maint) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_backup_schedule(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        schedule_json: &QString,
        overdue_threshold: i32,
    ) -> bool {
        let id_str = backup_id.to_string();
        let existing = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, &id_str))
                .map(|b| b.backup.clone())
        };
        let Some(existing) = existing else {
            self.as_mut()
                .set_last_action_error(i18n("Backup not found."));
            return false;
        };
        let schedule: Schedule = match serde_json::from_str(&schedule_json.to_string()) {
            Ok(s) => s,
            Err(err) => {
                self.as_mut()
                    .set_last_action_error(QString::from(&err.to_string()));
                return false;
            }
        };
        let threshold_opt = (overdue_threshold > 0).then_some(overdue_threshold as u32);
        let updated = match clone_updated_backup_schedule(&existing, schedule, threshold_opt) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_backup(&updated) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_backup_conditions(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        conditions_json: &QString,
    ) -> bool {
        let id_str = backup_id.to_string();
        let existing = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, &id_str))
                .map(|b| b.backup.clone())
        };
        let Some(existing) = existing else {
            self.as_mut()
                .set_last_action_error(i18n("Backup not found."));
            return false;
        };
        let conditions: RunConditions = match serde_json::from_str(&conditions_json.to_string()) {
            Ok(c) => c,
            Err(err) => {
                self.as_mut()
                    .set_last_action_error(QString::from(&err.to_string()));
                return false;
            }
        };
        let dest_kind = cached_destination_kind(existing.destination_id.as_str());
        let updated =
            match clone_updated_backup_conditions(&existing, conditions, dest_kind.as_ref()) {
                Ok(b) => b,
                Err(err) => {
                    self.as_mut().set_last_action_error(QString::from(&err));
                    return false;
                }
            };
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_backup(&updated) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_settings(
        mut self: Pin<&mut Self>,
        default_overdue_threshold: i32,
        on_error: bool,
        on_warning: bool,
        on_overdue: bool,
        on_maintenance_failed: bool,
        on_restic_update: bool,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let threshold = default_overdue_threshold.max(1) as u32;
        let notifications = NotificationSettings {
            on_error,
            on_warning,
            on_overdue,
            on_maintenance_failed,
            on_restic_update,
        };
        match client.update_settings(threshold, &notifications) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn set_restic_provider(
        mut self: Pin<&mut Self>,
        kind: &QString,
        custom_path: &QString,
    ) -> bool {
        let provider = match build_provider(&kind.to_string(), &custom_path.to_string()) {
            Ok(p) => p,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        let Ok(client) = make_client() else {
            return false;
        };
        match client.set_restic_provider(&provider) {
            Ok(_) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                false
            }
        }
    }

    pub fn update_expert_mode(mut self: Pin<&mut Self>, enabled: bool) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.set_expert_mode(enabled) {
            Ok(()) => {
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn pause_global(mut self: Pin<&mut Self>, duration_kind: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let duration = parse_pause_duration(&duration_kind.to_string());
        match client.pause_global(&duration) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn resume_global(mut self: Pin<&mut Self>) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.resume_global() {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn pause_backup(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        duration_kind: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let duration = parse_pause_duration(&duration_kind.to_string());
        match client.pause_backup(&backup_id.to_string(), &duration) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn resume_backup(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.resume_backup(&backup_id.to_string()) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn load_run_detail(mut self: Pin<&mut Self>, run_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let detail = match client.get_run_detail(&run_id.to_string()) {
            Ok(detail) => detail,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        self.as_mut().apply_run_detail(detail);
        true
    }

    fn apply_run_detail(mut self: Pin<&mut Self>, detail: kestic_daemon::dbus_api::RunDetailDto) {
        #[derive(serde::Serialize)]
        struct CmdDisplay {
            command: String,
            meta: String,
        }

        let title = crate::state_cache::i18n("Run from %1")
            .subs(&[&format_datetime_display(detail.run.started_at)]);
        let (status_text, status_icon, status_color) =
            format_run_status(&detail.run.kind, detail.run.status);
        let dur = detail
            .run
            .duration_ms
            .map(format_duration_ms)
            .unwrap_or_else(|| String::from("–"));
        let status_line = format!("{status_text} · {dur}");
        let kind_title = format_run_kind_name(
            &detail.run,
            detail.backup_name.as_deref(),
            &detail.destination_name,
        );
        let added = detail
            .run
            .stats
            .as_ref()
            .map(|s| format_bytes(s.data_added))
            .unwrap_or_else(|| String::from("0 B"));
        let subtitle = crate::state_cache::i18n("%1 · %2 added").subs(&[&kind_title, &added]);
        let stats_summary = if let Some(s) = &detail.run.stats {
            crate::state_cache::i18n("%1 new · %2 changed · %3 unchanged · %4 processed").subs(&[
                &s.files_new.to_string(),
                &s.files_changed.to_string(),
                &s.files_unmodified.to_string(),
                &format_bytes(s.total_bytes_processed),
            ])
        } else {
            i18n("No file statistics available for this run.").to_string()
        };
        let cmds: Vec<CmdDisplay> = detail
            .transparent_commands
            .iter()
            .enumerate()
            .map(|(idx, cmd)| {
                let meta = detail
                    .run
                    .commands
                    .get(idx)
                    .map(|c| {
                        crate::state_cache::i18n("Exit %1 · %2")
                            .subs(&[&c.exit_code.to_string(), &format_duration_ms(c.duration_ms)])
                    })
                    .unwrap_or_default();
                CmdDisplay {
                    command: cmd.clone(),
                    meta,
                }
            })
            .collect();
        let transparent_joined = detail.transparent_commands.join("\n");
        let commands_json = serde_json::to_string(&cmds).unwrap_or_else(|_| "[]".into());
        let unreadable_json =
            serde_json::to_string(&detail.unreadable_files).unwrap_or_else(|_| "[]".into());
        let log_path_str = detail
            .run
            .log_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let run_backup_id = detail
            .run
            .backup_id()
            .map(|b| b.as_str().to_string())
            .unwrap_or_default();

        self.as_mut()
            .set_selected_run_id(QString::from(detail.run.id.as_str()));
        self.as_mut().set_selected_run_title(QString::from(&title));
        self.as_mut()
            .set_selected_run_status_line(QString::from(&status_line));
        self.as_mut()
            .set_selected_run_subtitle(QString::from(&subtitle));
        self.as_mut()
            .set_selected_run_status_icon(QString::from(status_icon));
        self.as_mut()
            .set_selected_run_status_color(QString::from(status_color));
        self.as_mut()
            .set_selected_run_stats_summary(QString::from(&stats_summary));
        let error_summary = crate::overall_text::localized_error_kind(
            detail.error_kind.as_deref(),
            detail.error_exit_code,
            detail.run.status == kestic_core::model::RunStatus::Warning,
            detail.error_summary.as_deref(),
        );
        self.as_mut()
            .set_selected_run_error_summary(QString::from(&error_summary));
        self.as_mut().set_selected_run_error_action(QString::from(
            detail.error_action.as_deref().unwrap_or(""),
        ));
        self.as_mut().set_selected_run_error_raw(QString::from(
            detail.error_raw_detail.as_deref().unwrap_or(""),
        ));
        self.as_mut()
            .set_selected_run_unreadable_json(QString::from(&unreadable_json));
        self.as_mut()
            .set_selected_run_commands_json(QString::from(&commands_json));
        self.as_mut()
            .set_selected_run_transparent_command(QString::from(&transparent_joined));
        self.as_mut()
            .set_selected_run_log_path(QString::from(&log_path_str));
        self.as_mut()
            .set_selected_run_backup_id(QString::from(&run_backup_id));

        {
            let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard.run_detail = Some(detail);
        }
        self.as_mut().models_updated();
    }

    pub fn copy_to_clipboard(&self, text: &QString) {
        smoke::ffi::copy_to_clipboard(&text.to_string());
    }

    pub fn confirm_quit(mut self: Pin<&mut Self>) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.confirm_quit() {
            Ok(()) => true,
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn home_dir(&self) -> QString {
        let home = env::var("HOME").unwrap_or_else(|_| "/home".to_string());
        QString::from(&home)
    }

    pub fn recovery_action_label(&self, action_code: &QString) -> QString {
        match action_code.to_string().as_str() {
            "update_stored_password" => i18n("Update Saved Password…"),
            "add_to_excludes" => i18n("Add to Exclude Rules…"),
            "check_restic_source" => i18n("Configure restic Source…"),
            "unlock_repository" => i18n("Release Lock…"),
            _ => i18n("Retry"),
        }
    }

    pub fn list_known_networks_json(&self) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("[]");
        };
        match client.list_known_networks() {
            Ok(list) => {
                QString::from(&serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()))
            }
            Err(_) => QString::from("[]"),
        }
    }

    pub fn today_iso_date(&self) -> QString {
        QString::from(&cached_today_iso_date())
    }

    pub fn validate_schedule_json(&self, schedule_json: &QString) -> QString {
        let parsed = serde_json::from_str::<Schedule>(&schedule_json.to_string());
        let (field, message) = match parsed {
            Ok(schedule) => match schedule.validate() {
                Ok(()) => ("", String::new()),
                Err(_) => match &schedule {
                    Schedule::SpecificWeekdays { weekdays, .. } if weekdays.is_empty() => {
                        ("weekdays", i18n("Select at least one weekday.").to_string())
                    }
                    Schedule::EveryNDays { interval_days, .. }
                        if !(2..=365).contains(interval_days) =>
                    {
                        (
                            "interval",
                            i18n("Interval must be between 2 and 365 days.").to_string(),
                        )
                    }
                    Schedule::EveryNDays { .. } => (
                        "start_date",
                        i18n("Enter a valid start date in YYYY-MM-DD format.").to_string(),
                    ),
                    _ => ("", i18n("Invalid schedule configuration.").to_string()),
                },
            },
            Err(_) => ("", i18n("Invalid schedule configuration.").to_string()),
        };
        let payload = serde_json::json!({
            "field": field,
            "message": message,
        });
        QString::from(&payload.to_string())
    }

    pub fn preview_schedule_slots_json(&self, schedule_json: &QString) -> QString {
        let Ok(schedule) = serde_json::from_str::<Schedule>(&schedule_json.to_string()) else {
            return QString::from("[]");
        };
        if schedule.validate().is_err() {
            return QString::from("[]");
        }
        let slots = preview_next_schedule_slots(&schedule, cached_daemon_now());
        QString::from(&serde_json::to_string(&slots).unwrap_or_else(|_| "[]".into()))
    }

    pub fn format_schedule_summary_json(&self, schedule_json: &QString) -> QString {
        let Ok(schedule) = serde_json::from_str::<Schedule>(&schedule_json.to_string()) else {
            return QString::from("");
        };
        QString::from(&format_schedule_summary(&schedule))
    }

    pub fn format_conditions_summary_json(&self, conditions_json: &QString) -> QString {
        let Ok(conditions) = serde_json::from_str::<RunConditions>(&conditions_json.to_string())
        else {
            return QString::from("");
        };
        QString::from(&format_conditions_summary(&conditions))
    }

    pub fn format_maintenance_summary_json(&self, maintenance_json: &QString) -> QString {
        let Ok(maint) = serde_json::from_str::<MaintenanceSchedule>(&maintenance_json.to_string())
        else {
            return QString::from("");
        };
        QString::from(&format_maintenance_summary(&maint))
    }
}
