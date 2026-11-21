// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Schedule/condition/maintenance editing, pause/resume controls, settings, run details,
//! clipboard, and quit confirmation for [`super::qobject::KesticApp`].

use super::{Finish, qobject};
use crate::qt_bridge;
use crate::run_texts::{run_detail_subtitle, run_detail_title, run_result_summary, run_status};
use crate::state_cache::{
    build_provider, cached_daemon_now, cached_destination_kind, cached_today_iso_date,
    clone_updated_backup_conditions, clone_updated_backup_schedule, format_conditions_summary,
    format_duration_ms, format_maintenance_summary, format_schedule_summary, i18n, json_or,
    make_client, preview_next_schedule_slots, shared_state,
};
use crate::ui_texts::GuiText;
use cxx_qt_lib::QString;
use kestic_core::model::{
    BackupId, MaintenanceSchedule, NotificationSettings, RunConditions, Schedule,
};
use kestic_core::scheduler::PauseDuration;
use kestic_core::sync::LockUnpoisoned;
use std::env;
use std::pin::Pin;

/// Name of a configured backup from the cached daemon state.
fn cached_backup_name(id: &BackupId) -> Option<String> {
    let guard = shared_state().lock_unpoisoned();
    guard
        .state
        .as_ref()?
        .backups
        .iter()
        .find(|b| &b.backup.id == id)
        .map(|b| b.backup.name.clone())
}

fn parse_pause_duration(kind: &str) -> PauseDuration {
    match kind {
        "1h" => PauseDuration::Hours { hours: 1 },
        "3h" => PauseDuration::Hours { hours: 3 },
        "tomorrow" => PauseDuration::UntilTomorrow,
        _ => PauseDuration::Indefinite,
    }
}

impl qobject::KesticApp {
    pub fn run_destination_maintenance(self: Pin<&mut Self>, destination_id: &QString) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        self.finish_action(
            client.run_destination_maintenance(&destination_id.to_string()),
            Finish::Plain,
        )
        .map_or_else(QString::default, |run_id| QString::from(&run_id))
    }

    pub fn update_destination_maintenance(
        mut self: Pin<&mut Self>,
        dest_id: &QString,
        maintenance_json: &QString,
    ) -> bool {
        let Some(maint) = self
            .as_mut()
            .parse_json_or_err::<MaintenanceSchedule>(maintenance_json)
        else {
            return false;
        };
        if self.as_mut().ok_or_show(maint.validate()).is_none() {
            return false;
        }
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(
            client.update_destination_maintenance(&dest_id.to_string(), &maint),
            Finish::Saved,
        )
        .is_some()
    }

    pub fn update_backup_schedule(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        schedule_json: &QString,
        overdue_threshold: i32,
    ) -> bool {
        let Some(existing) = self.as_mut().existing_backup_or_err(&backup_id.to_string()) else {
            return false;
        };
        let Some(schedule) = self.as_mut().parse_json_or_err::<Schedule>(schedule_json) else {
            return false;
        };
        let threshold_opt = u32::try_from(overdue_threshold).ok().filter(|&t| t > 0);
        let Some(updated) = self.as_mut().ok_or_show(clone_updated_backup_schedule(
            &existing,
            schedule,
            threshold_opt,
        )) else {
            return false;
        };
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(client.update_backup(&updated), Finish::Saved)
            .is_some()
    }

    pub fn update_backup_conditions(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        conditions_json: &QString,
    ) -> bool {
        let Some(existing) = self.as_mut().existing_backup_or_err(&backup_id.to_string()) else {
            return false;
        };
        let Some(conditions) = self
            .as_mut()
            .parse_json_or_err::<RunConditions>(conditions_json)
        else {
            return false;
        };
        let dest_kind = cached_destination_kind(existing.destination_id.as_str());
        let Some(updated) = self.as_mut().ok_or_show(clone_updated_backup_conditions(
            &existing,
            conditions,
            dest_kind.as_ref(),
        )) else {
            return false;
        };
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(client.update_backup(&updated), Finish::Saved)
            .is_some()
    }

    pub fn update_settings(
        self: Pin<&mut Self>,
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
        let threshold = u32::try_from(default_overdue_threshold).unwrap_or(0).max(1);
        let notifications = NotificationSettings {
            on_error,
            on_warning,
            on_overdue,
            on_maintenance_failed,
            on_restic_update,
        };
        self.finish_action(
            client.update_settings(threshold, &notifications),
            Finish::Plain,
        )
        .is_some()
    }

    pub fn set_restic_provider(
        mut self: Pin<&mut Self>,
        kind: &QString,
        custom_path: &QString,
    ) -> bool {
        let Some(provider) = self
            .as_mut()
            .ok_or_show(build_provider(&kind.to_string(), &custom_path.to_string()))
        else {
            return false;
        };
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(client.set_restic_provider(&provider), Finish::Plain)
            .is_some()
    }

    pub fn update_expert_mode(self: Pin<&mut Self>, enabled: bool) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(client.set_expert_mode(enabled), Finish::Plain)
            .is_some()
    }

    pub fn pause_global(self: Pin<&mut Self>, duration_kind: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let duration = parse_pause_duration(&duration_kind.to_string());
        self.finish_action(client.pause_global(&duration), Finish::NotifyOnError)
            .is_some()
    }

    pub fn resume_global(self: Pin<&mut Self>) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(client.resume_global(), Finish::NotifyOnError)
            .is_some()
    }

    pub fn pause_backup(
        self: Pin<&mut Self>,
        backup_id: &QString,
        duration_kind: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let duration = parse_pause_duration(&duration_kind.to_string());
        self.finish_action(
            client.pause_backup(&backup_id.to_string(), &duration),
            Finish::NotifyOnError,
        )
        .is_some()
    }

    pub fn resume_backup(self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(
            client.resume_backup(&backup_id.to_string()),
            Finish::NotifyOnError,
        )
        .is_some()
    }

    pub fn load_run_detail(mut self: Pin<&mut Self>, run_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let Some(detail) = self
            .as_mut()
            .ok_or_show_dbus(client.get_run_detail(&run_id.to_string()))
        else {
            return false;
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

        let title = run_detail_title(&detail.run);
        let (status_text, status_icon, status_color) = run_status(&detail.run);
        let dur = detail
            .run
            .duration_ms
            .map_or_else(|| String::from("–"), format_duration_ms);
        let status_line = format!("{status_text} · {dur}");
        let subtitle = run_detail_subtitle(
            &detail.run,
            detail.backup_name.as_deref(),
            &detail.destination_name,
        );
        let result_summary = run_result_summary(&detail.run, &cached_backup_name);
        let cmds: Vec<CmdDisplay> = detail
            .transparent_commands
            .iter()
            .enumerate()
            .map(|(idx, cmd)| {
                let meta = detail.run.commands.get(idx).map_or_default(|c| {
                    i18n("Exit code %1 · %2")
                        .subs(&[&c.exit_code.to_string(), &format_duration_ms(c.duration_ms)])
                });
                CmdDisplay {
                    command: cmd.clone(),
                    meta,
                }
            })
            .collect();
        let transparent_joined = detail.transparent_commands.join("\n");
        let commands_json = json_or(&cmds, "[]");
        let unreadable_json = json_or(&detail.unreadable_files, "[]");
        let log_path_str = detail
            .run
            .log_path
            .as_ref()
            .map_or_default(|p| p.to_string_lossy().into_owned());
        let run_backup_id = detail
            .run
            .backup_id()
            .map_or_default(|b| b.as_str().to_string());

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
            .set_selected_run_result_summary(QString::from(&result_summary));
        let error_summary = detail.error_kind.map_or_default(|kind| {
            crate::state_cache::tr(&kind.message(
                detail.error_exit_code,
                detail.run.status == kestic_core::model::RunStatus::Warning,
            ))
        });
        self.as_mut()
            .set_selected_run_error_summary(QString::from(&error_summary));
        self.as_mut().set_selected_run_error_action(QString::from(
            detail.error_action.map_or("", |a| a.as_str()),
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
            let mut guard = shared_state().lock_unpoisoned();
            guard.run_detail = Some(detail);
        }
        self.as_mut().models_updated();
    }

    pub fn copy_to_clipboard(&self, text: &QString) {
        qt_bridge::ffi::copy_to_clipboard(&text.to_string());
    }

    pub fn confirm_quit(self: Pin<&mut Self>) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        self.ok_or_show_dbus(client.confirm_quit()).is_some()
    }

    pub fn home_dir(&self) -> QString {
        let home = env::var("HOME").unwrap_or_else(|_| "/home".to_string());
        QString::from(&home)
    }

    pub fn recovery_action_label(&self, action_code: &QString) -> QString {
        let label = match kestic_core::restic::RecoveryAction::parse(&action_code.to_string()) {
            Some(action) => crate::state_cache::tr(&action.label_message()),
            None => GuiText::Retry.text(),
        };
        QString::from(&label)
    }

    pub fn list_known_networks_json(&self) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("[]");
        };
        match client.list_known_networks() {
            Ok(list) => QString::from(&json_or(&list, "[]")),
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
                    _ => ("", i18n("Invalid schedule.").to_string()),
                },
            },
            Err(_) => ("", i18n("Invalid schedule.").to_string()),
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
        QString::from(&json_or(&slots, "[]"))
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
