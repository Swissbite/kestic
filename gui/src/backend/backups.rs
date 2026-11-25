// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backup CRUD, manual backup execution/cancellation, and exclude-rule helpers
//! for [`super::qobject::KesticApp`].

use super::{Finish, dbus_error_text, qobject};
use crate::state_cache::{
    build_new_backup, cached_destination_kind, clone_updated_backup_excludes,
    clone_updated_backup_retention, clone_updated_backup_source, daemon_unavailable_text, i18n,
    localized_condition_override_message, localized_manual_block_message,
    localized_start_all_summary, localized_unmet_conditions, lookup_backup_summary, make_client,
    refresh_state_now, shared_state,
};
use cxx_qt_lib::QString;
use kestic_core::backup::build_backup_command_spec;
use kestic_core::config::AppConfig;
use kestic_core::model::{Backup, BackupId, ExcludeRules, RetentionPolicy};
use kestic_core::provider::resolve_and_check;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::KesticDbusError;
use kestic_i18n::codes::{BlockedReason, ServiceErrorCode};
use std::env;
use std::path::PathBuf;
use std::pin::Pin;

impl qobject::KesticApp {
    pub fn start_backup(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let b_id = backup_id.to_string();
        let Ok(client) = make_client() else {
            self.as_mut().show_action_error(&daemon_unavailable_text());
            return false;
        };
        match client.start_backup(&b_id) {
            Ok(_) => {
                self.as_mut()
                    .set_manual_run_blocked_message(QString::from(""));
                self.finish_action(Ok::<_, zbus::Error>(()), Finish::Plain);
                true
            }
            Err(err) => {
                self.as_mut().handle_start_backup_error(&b_id, &err);
                false
            }
        }
    }

    fn handle_start_backup_error(mut self: Pin<&mut Self>, b_id: &str, err: &zbus::Error) {
        let typed = KesticDbusError::from(err.clone());
        let is_blocked = typed.code() == ServiceErrorCode::ManualRunBlocked;
        let err_is_condition = typed.blocked_reason() == Some(BlockedReason::ConditionNotMet);
        refresh_state_now();
        self.as_mut().apply_cached_state();
        if !is_blocked {
            let msg = dbus_error_text(err);
            self.as_mut().show_action_error(&msg);
            self.as_mut().passive_notification(QString::from(&msg));
            return;
        }
        {
            let mut guard = shared_state().lock_unpoisoned();
            if let Some((pending_id, _)) = &guard.pending_blocked
                && pending_id == b_id
            {
                guard.pending_blocked = None;
            }
        }
        let (backup_name, reason_code, unmet_text) = {
            let guard = shared_state().lock_unpoisoned();
            let summary = guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, b_id));
            let code = match summary {
                _ if err_is_condition => BlockedReason::ConditionNotMet,
                Some(b) => b.availability.blocked_reason.unwrap_or({
                    if b.has_unmet_run_condition {
                        BlockedReason::ConditionNotMet
                    } else {
                        BlockedReason::DestinationUnreachable
                    }
                }),
                None => BlockedReason::DestinationUnreachable,
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

    pub fn start_backup_override(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let result = client.start_backup_override(&backup_id.to_string());
        if result.is_ok() {
            self.as_mut()
                .set_condition_override_backup_id(QString::from(""));
            self.as_mut()
                .set_condition_override_backup_name(QString::from(""));
            self.as_mut()
                .set_condition_override_message(QString::from(""));
        }
        self.finish_action(result, Finish::NotifyOnError).is_some()
    }

    pub fn start_all_backups(mut self: Pin<&mut Self>) {
        let Ok(client) = make_client() else {
            return;
        };
        match client.start_all_backups() {
            Ok(res) => {
                if let Some(msg) = localized_start_all_summary(&res) {
                    self.as_mut()
                        .set_start_all_skipped_message(QString::from(&msg));
                    self.as_mut().passive_notification(QString::from(&msg));
                } else {
                    self.as_mut()
                        .set_start_all_skipped_message(QString::from(""));
                }
                refresh_state_now();
                self.as_mut().apply_cached_state();
            }
            Err(err) => {
                self.as_mut()
                    .passive_notification(QString::from(&dbus_error_text(&err)));
            }
        }
    }

    pub fn dismiss_start_all_skipped(mut self: Pin<&mut Self>) {
        self.as_mut()
            .set_start_all_skipped_message(QString::from(""));
    }

    pub fn cancel_run(mut self: Pin<&mut Self>, run_id: &QString) {
        if let Ok(client) = make_client() {
            let _ = client.cancel_run(&run_id.to_string());
            refresh_state_now();
            self.as_mut().apply_cached_state();
        }
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
        with_cached_backup(backup_id, |config, backup| {
            let dest = config.destination(&backup.destination_id)?;
            let state_dir = kestic_core::backup::default_state_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("~/.local/state/kestic"));
            let exe = env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("kestic"));
            if let Ok(resolved) = resolve_and_check(&config.restic_provider)
                && let Ok(spec) =
                    build_backup_command_spec(backup, dest, &state_dir, &resolved, &exe)
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
