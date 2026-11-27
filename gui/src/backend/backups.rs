// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backup CRUD, manual backup execution/cancellation, and exclude-rule helpers
//! for [`super::qobject::KesticApp`].

use super::daemon_call::Lane;
use super::{Finish, dbus_error_text, qobject};
use crate::daemon_events::request_refresh;
use crate::state_cache::{
    build_new_backup, cached_destination_kind, clone_updated_backup_excludes,
    clone_updated_backup_retention, clone_updated_backup_source, daemon_unavailable_text, i18n,
    localized_condition_override_message, localized_manual_block_message,
    localized_start_all_summary, localized_unmet_conditions, lookup_backup_summary, make_client,
    shared_state,
};
use cxx_qt_lib::QString;
use kestic_core::backup::build_backup_command_spec;
use kestic_core::config::AppConfig;
use kestic_core::model::{Backup, BackupId, ExcludeRules, RetentionPolicy};
use kestic_core::provider::resolved_from_report;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::KesticDbusError;
use kestic_i18n::codes::{BlockedReason, ServiceErrorCode};
use std::env;
use std::path::PathBuf;
use std::pin::Pin;

impl qobject::KesticApp {
    pub fn start_backup(self: Pin<&mut Self>, backup_id: &QString) {
        let b_id = backup_id.to_string();
        // A refused start is explained from the state as it is after the refusal.
        self.call_daemon_with(
            Lane::slow(&format!("start:{b_id}")),
            move |client| {
                let result = client.start_backup(&b_id);
                (b_id, result)
            },
            |(_, result)| result.is_err(),
            |mut app, outcome| match outcome {
                Ok((_, Ok(_))) => {
                    app.as_mut()
                        .set_manual_run_blocked_message(QString::from(""));
                    app.finish_daemon_action(Ok(Ok::<_, zbus::Error>(())), Finish::Plain);
                }
                Ok((b_id, Err(err))) => app.handle_start_backup_error(&b_id, &err),
                Err(_) => app.show_action_error(&daemon_unavailable_text()),
            },
        );
    }

    fn handle_start_backup_error(mut self: Pin<&mut Self>, b_id: &str, err: &zbus::Error) {
        let typed = KesticDbusError::from(err.clone());
        let is_blocked = typed.code() == ServiceErrorCode::ManualRunBlocked;
        let err_reason = typed.blocked_reason();
        self.as_mut().apply_cached_state();
        if !is_blocked {
            let msg = dbus_error_text(err);
            self.as_mut().show_action_error(&msg);
            self.as_mut().passive_notification(QString::from(&msg));
            return;
        }
        {
            let mut guard = shared_state().lock_unpoisoned();
            // This call reports the block itself; the daemon's signal for it is not shown again.
            let shared = &mut *guard;
            for slot in [
                &mut shared.pending_blocked,
                &mut shared.blocked_after_refresh,
            ] {
                if slot
                    .as_ref()
                    .is_some_and(|(pending_id, _)| pending_id == b_id)
                {
                    *slot = None;
                }
            }
        }
        let (backup_name, reason_code, unmet_text) = {
            let guard = shared_state().lock_unpoisoned();
            let summary = guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, b_id));
            // The reason the daemon gave comes first; the state explains the others.
            let code = match (err_reason, summary) {
                (Some(reason), _) => reason,
                (None, Some(b)) => b.availability.blocked_reason.unwrap_or({
                    if b.has_unmet_run_condition {
                        BlockedReason::ConditionNotMet
                    } else {
                        BlockedReason::DestinationUnreachable
                    }
                }),
                (None, None) => BlockedReason::DestinationUnreachable,
            };
            match summary {
                Some(b) => (b.backup.name.clone(), code, localized_unmet_conditions(b)),
                None => (b_id.to_string(), code, None),
            }
        };
        if reason_code == BlockedReason::ConditionNotMet {
            let msg = localized_condition_override_message(&backup_name, unmet_text.as_deref());
            self.as_mut()
                .set_condition_override_backup_id(QString::from(b_id));
            self.as_mut()
                .set_condition_override_backup_name(QString::from(&backup_name));
            self.as_mut()
                .set_condition_override_message(QString::from(&msg));
            self.as_mut().condition_override_requested(
                QString::from(b_id),
                QString::from(&backup_name),
                QString::from(&msg),
            );
        } else {
            let msg = localized_manual_block_message(&backup_name, reason_code);
            self.as_mut()
                .set_manual_run_blocked_backup_id(QString::from(b_id));
            self.as_mut()
                .set_manual_run_blocked_message(QString::from(&msg));
        }
    }

    pub fn start_backup_override(self: Pin<&mut Self>, backup_id: &QString) {
        let b_id = backup_id.to_string();
        self.call_daemon(
            Lane::slow(&format!("start:{b_id}")),
            move |client| client.start_backup_override(&b_id),
            |mut app, result| {
                if matches!(result, Ok(Ok(_))) {
                    app.as_mut()
                        .set_condition_override_backup_id(QString::from(""));
                    app.as_mut()
                        .set_condition_override_backup_name(QString::from(""));
                    app.as_mut()
                        .set_condition_override_message(QString::from(""));
                }
                app.finish_daemon_action(result, Finish::NotifyOnError);
            },
        );
    }

    pub fn start_all_backups(self: Pin<&mut Self>) {
        self.call_daemon(
            Lane::slow("start-all"),
            |client| client.start_all_backups(),
            |mut app, result| match result {
                Ok(Ok(res)) => {
                    let msg = localized_start_all_summary(&res).unwrap_or_default();
                    app.as_mut()
                        .set_start_all_skipped_message(QString::from(&msg));
                    if !msg.is_empty() {
                        app.as_mut().passive_notification(QString::from(&msg));
                    }
                    request_refresh();
                }
                Ok(Err(err)) => app
                    .as_mut()
                    .passive_notification(QString::from(&dbus_error_text(&err))),
                Err(_) => app
                    .as_mut()
                    .passive_notification(QString::from(&daemon_unavailable_text())),
            },
        );
    }

    pub fn dismiss_start_all_skipped(mut self: Pin<&mut Self>) {
        self.as_mut()
            .set_start_all_skipped_message(QString::from(""));
    }

    pub fn cancel_run(self: Pin<&mut Self>, run_id: &QString) {
        let run_id = run_id.to_string();
        // A run that ended meanwhile cannot be cancelled any more; the state shows how it ended.
        self.call_daemon(
            Lane::Alone,
            move |client| client.cancel_run(&run_id).is_ok(),
            |_, _| request_refresh(),
        );
    }

    pub fn dismiss_manual_run_blocked(mut self: Pin<&mut Self>) {
        self.as_mut()
            .set_manual_run_blocked_message(QString::from(""));
        self.as_mut()
            .set_manual_run_blocked_backup_id(QString::from(""));
    }

    pub fn add_backup(
        mut self: Pin<&mut Self>,
        name: &QString,
        source_path: &QString,
        dest_id: &QString,
        excludes_text: &QString,
    ) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        let src_str = source_path.to_string();
        let detected = client.detect_source(src_str.trim()).ok();
        let Some(backup) = self.as_mut().ok_or_show(build_new_backup(
            &name.to_string(),
            &src_str,
            &dest_id.to_string(),
            &excludes_text.to_string(),
            detected,
            cached_destination_kind(&dest_id.to_string()).as_ref(),
        )) else {
            return QString::from("");
        };
        self.finish_action(client.add_backup(&backup), Finish::Plain)
            .map_or_else(QString::default, |id| QString::from(&id))
    }

    pub fn update_backup_excludes(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        excludes_json: &QString,
    ) -> bool {
        let Some(existing) = self.as_mut().existing_backup_or_err(&backup_id.to_string()) else {
            return false;
        };
        let Some(excludes) = self
            .as_mut()
            .parse_json_or_err::<ExcludeRules>(excludes_json)
        else {
            return false;
        };
        let Some(updated) = self
            .as_mut()
            .ok_or_show(clone_updated_backup_excludes(&existing, excludes))
        else {
            return false;
        };
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(client.update_backup(&updated), Finish::Saved)
            .is_some()
    }

    pub fn update_backup_retention(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        retention_json: &QString,
    ) -> bool {
        let Some(existing) = self.as_mut().existing_backup_or_err(&backup_id.to_string()) else {
            return false;
        };
        let Some(retention) = self
            .as_mut()
            .parse_json_or_err::<RetentionPolicy>(retention_json)
        else {
            return false;
        };
        let Some(updated) = self
            .as_mut()
            .ok_or_show(clone_updated_backup_retention(&existing, retention))
        else {
            return false;
        };
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(client.update_backup(&updated), Finish::Saved)
            .is_some()
    }

    pub fn update_backup_source(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        source_path: &QString,
    ) -> bool {
        let Some(existing) = self.as_mut().existing_backup_or_err(&backup_id.to_string()) else {
            return false;
        };
        let Ok(client) = make_client() else {
            return false;
        };
        let trimmed = source_path.to_string().trim().to_string();
        let source = client
            .detect_source(&trimmed)
            .unwrap_or_else(|_| kestic_core::model::Source::folder(PathBuf::from(&trimmed)));
        let Some(updated) = self
            .as_mut()
            .ok_or_show(clone_updated_backup_source(&existing, source))
        else {
            return false;
        };
        self.finish_action(client.update_backup(&updated), Finish::Saved)
            .is_some()
    }

    pub fn rename_backup(self: Pin<&mut Self>, backup_id: &QString, new_name: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(
            client.rename_backup(&backup_id.to_string(), &new_name.to_string()),
            Finish::Plain,
        )
        .is_some()
    }

    pub fn reassign_backup(
        self: Pin<&mut Self>,
        backup_id: &QString,
        new_dest_id: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(
            client.reassign_backup(&backup_id.to_string(), &new_dest_id.to_string()),
            Finish::Plain,
        )
        .is_some()
    }

    pub fn remove_backup(self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(client.remove_backup(&backup_id.to_string()), Finish::Plain)
            .is_some()
    }

    pub fn add_exclude_path_to_backup(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        path: &QString,
    ) -> bool {
        let id_str = backup_id.to_string();
        let trimmed = path.to_string().trim().to_string();
        if trimmed.is_empty() {
            return false;
        }
        let Ok(client) = make_client() else {
            return false;
        };
        let result = client.add_exclude_rule_for_item(&id_str, &trimmed);
        if self.as_mut().finish_action(result, Finish::Plain).is_none() {
            return false;
        }
        self.as_mut()
            .passive_notification(QString::from(&i18n("Exclude rule added").to_string()));
        true
    }

    pub fn get_backup_transparent_command(&self, backup_id: &QString) -> QString {
        // The restic the daemon checked last: running `restic version` here would block the
        // window for as long as restic takes.
        let restic = {
            let guard = shared_state().lock_unpoisoned();
            guard
                .state
                .as_ref()
                .map(|s| &s.restic_status)
                .filter(|status| status.valid)
                .and_then(|status| {
                    resolved_from_report(status.binary_path.as_deref()?, status.version.as_deref()?)
                })
        };
        with_cached_backup(backup_id, |config, backup| {
            let dest = config.destination(&backup.destination_id)?;
            let state_dir = kestic_core::backup::default_state_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("~/.local/state/kestic"));
            let exe = env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("kestic"));
            if let Some(resolved) = restic.as_ref()
                && let Ok(spec) =
                    build_backup_command_spec(backup, dest, &state_dir, resolved, &exe)
            {
                return Some(spec.to_transparent_shell_command());
            }
            Some(format!(
                "RESTIC_PASSWORD_COMMAND='kestic secret get {}' restic -r {} backup {}",
                dest.id,
                dest.kind.repository_uri(),
                backup.source.path.display()
            ))
        })
    }

    pub fn build_backup_config_toml_snippet(&self, backup_id: &QString) -> QString {
        with_cached_backup(backup_id, |_, backup| {
            Some(kestic_core::config::backup_toml_snippet(backup))
        })
    }
}

/// Text computed by `text` from the cached backup `backup_id`; empty when the state, the
/// backup or the text is missing.
fn with_cached_backup(
    backup_id: &QString,
    text: impl FnOnce(&AppConfig, &Backup) -> Option<String>,
) -> QString {
    let guard = shared_state().lock_unpoisoned();
    let found = guard.state.as_ref().and_then(|state| {
        let backup = BackupId::parse(backup_id.to_string())
            .ok()
            .and_then(|id| state.config.backup(&id))?;
        text(&state.config, backup)
    });
    QString::from(&found.unwrap_or_default())
}
