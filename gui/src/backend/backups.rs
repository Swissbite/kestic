// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backup CRUD, manual backup execution/cancellation, and exclude-rule helpers
//! for [`super::qobject::KesticApp`].

use super::{friendly_dbus_error, qobject};
use crate::state_cache::{
    build_new_backup, cached_destination_kind, clone_updated_backup, clone_updated_backup_excludes,
    clone_updated_backup_retention, clone_updated_backup_source,
    localized_condition_override_message, localized_manual_block_message,
    localized_start_all_summary, lookup_backup_summary, make_client, refresh_state_now,
    shared_state,
};
use cxx_kde_frameworks::ki18n::i18n;
use cxx_qt_lib::QString;
use kestic_core::backup::build_backup_command_spec;
use kestic_core::model::{BackupId, ExcludeRules, RetentionPolicy};
use kestic_core::provider::resolve_and_check;
use kestic_daemon::dbus_api::{REASON_CONDITION_NOT_MET, REASON_DESTINATION_UNREACHABLE};
use std::env;
use std::path::PathBuf;
use std::pin::Pin;

impl qobject::KesticApp {
    pub fn start_backup(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let b_id = backup_id.to_string();
        let Ok(client) = make_client() else {
            self.as_mut()
                .set_last_action_error(i18n("Background service unavailable."));
            return false;
        };
        match client.start_backup(&b_id) {
            Ok(_) => {
                self.as_mut()
                    .set_manual_run_blocked_message(QString::from(""));
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                self.as_mut().handle_start_backup_error(&b_id, &err);
                false
            }
        }
    }

    fn handle_start_backup_error(mut self: Pin<&mut Self>, b_id: &str, err: &zbus::Error) {
        let raw = err.to_string();
        let is_blocked = raw.contains("ManualRunBlocked");
        let err_is_condition = raw.contains(REASON_CONDITION_NOT_MET);
        refresh_state_now();
        self.as_mut().apply_cached_state();
        if !is_blocked {
            let msg = friendly_dbus_error(err);
            self.as_mut().set_last_action_error(QString::from(&msg));
            self.as_mut().passive_notification(QString::from(&msg));
            return;
        }
        {
            let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            if let Some((pending_id, _)) = &guard.pending_blocked
                && pending_id == b_id
            {
                guard.pending_blocked = None;
            }
        }
        let (backup_name, reason_code, unmet_text) = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, b_id))
                .map(|b| {
                    let code = if err_is_condition {
                        REASON_CONDITION_NOT_MET.to_string()
                    } else {
                        b.availability.blocked_reason.clone().unwrap_or_else(|| {
                            if b.has_unmet_run_condition {
                                REASON_CONDITION_NOT_MET.to_string()
                            } else {
                                REASON_DESTINATION_UNREACHABLE.to_string()
                            }
                        })
                    };
                    (b.backup.name.clone(), code, b.unmet_conditions_text.clone())
                })
                .unwrap_or_else(|| {
                    (
                        b_id.to_string(),
                        if err_is_condition {
                            REASON_CONDITION_NOT_MET.to_string()
                        } else {
                            REASON_DESTINATION_UNREACHABLE.to_string()
                        },
                        None,
                    )
                })
        };
        if reason_code == REASON_CONDITION_NOT_MET {
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
            let msg = localized_manual_block_message(&backup_name, &reason_code);
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
        match client.start_backup_override(&backup_id.to_string()) {
            Ok(_) => {
                self.as_mut()
                    .set_condition_override_backup_id(QString::from(""));
                self.as_mut()
                    .set_condition_override_backup_name(QString::from(""));
                self.as_mut()
                    .set_condition_override_message(QString::from(""));
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
                self.as_mut().passive_notification(QString::from(&err));
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
        let backup = match build_new_backup(
            &name.to_string(),
            &src_str,
            &dest_id.to_string(),
            &excludes_text.to_string(),
            detected,
            cached_destination_kind(&dest_id.to_string()).as_ref(),
        ) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return QString::from("");
            }
        };
        match client.add_backup(&backup) {
            Ok(id) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                QString::from(&id)
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                QString::from("")
            }
        }
    }

    pub fn update_backup(
        mut self: Pin<&mut Self>,
        id: &QString,
        name: &QString,
        source_path: &QString,
        dest_id: &QString,
        excludes_text: &QString,
        exclude_caches: bool,
    ) -> bool {
        let id_str = id.to_string();
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
        let Ok(client) = make_client() else {
            return false;
        };
        let src_str = source_path.to_string();
        let detected = if src_str.trim() != existing.source.path.to_string_lossy().as_ref() {
            client.detect_source(src_str.trim()).ok()
        } else {
            None
        };
        let updated = match clone_updated_backup(
            &existing,
            &name.to_string(),
            &src_str,
            &dest_id.to_string(),
            &excludes_text.to_string(),
            exclude_caches,
            detected,
        ) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
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

    pub fn update_backup_excludes(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        excludes_json: &QString,
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
        let excludes: ExcludeRules = match serde_json::from_str(&excludes_json.to_string()) {
            Ok(e) => e,
            Err(err) => {
                self.as_mut()
                    .set_last_action_error(QString::from(&err.to_string()));
                return false;
            }
        };
        let updated = match clone_updated_backup_excludes(&existing, excludes) {
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

    pub fn update_backup_retention(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        retention_json: &QString,
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
        let retention: RetentionPolicy = match serde_json::from_str(&retention_json.to_string()) {
            Ok(r) => r,
            Err(err) => {
                self.as_mut()
                    .set_last_action_error(QString::from(&err.to_string()));
                return false;
            }
        };
        let updated = match clone_updated_backup_retention(&existing, retention) {
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

    pub fn update_backup_source(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        source_path: &QString,
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
        let Ok(client) = make_client() else {
            return false;
        };
        let trimmed = source_path.to_string().trim().to_string();
        let source = client
            .detect_source(&trimmed)
            .unwrap_or_else(|_| kestic_core::model::Source::folder(PathBuf::from(&trimmed)));
        let updated = match clone_updated_backup_source(&existing, source) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
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

    pub fn rename_backup(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        new_name: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.rename_backup(&backup_id.to_string(), &new_name.to_string()) {
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

    pub fn reassign_backup(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        new_dest_id: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.reassign_backup(&backup_id.to_string(), &new_dest_id.to_string()) {
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

    pub fn remove_backup(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.remove_backup(&backup_id.to_string()) {
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
        match client.add_exclude_rule_for_item(&id_str, &trimmed) {
            Ok(_) => {
                refresh_state_now();
                self.as_mut().apply_cached_state();
                self.as_mut()
                    .passive_notification(i18n("Exclude rule added"));
                true
            }
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                false
            }
        }
    }

    pub fn get_backup_transparent_command(&self, backup_id: &QString) -> QString {
        let id_str = backup_id.to_string();
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = &guard.state else {
            return QString::from("");
        };
        let Ok(bid) = BackupId::parse(&id_str) else {
            return QString::from("");
        };
        let Some(backup) = state.config.backup(&bid) else {
            return QString::from("");
        };
        let Some(dest) = state.config.destination(&backup.destination_id) else {
            return QString::from("");
        };
        let state_dir = kestic_core::backup::default_state_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("~/.local/state/kestic"));
        let exe = env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("kestic"));
        if let Ok(resolved) = resolve_and_check(&state.config.restic_provider)
            && let Ok(spec) = build_backup_command_spec(backup, dest, &state_dir, &resolved, &exe)
        {
            return QString::from(&spec.to_transparent_shell_command());
        }
        QString::from(&format!(
            "RESTIC_PASSWORD_COMMAND='kestic secret get {}' restic -r {} backup {}",
            dest.id,
            dest.kind.repository_uri(),
            backup.source.path.display()
        ))
    }

    pub fn build_backup_config_toml_snippet(&self, backup_id: &QString) -> QString {
        let id_str = backup_id.to_string();
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = &guard.state else {
            return QString::from("");
        };
        let Ok(bid) = BackupId::parse(&id_str) else {
            return QString::from("");
        };
        let Some(backup) = state.config.backup(&bid) else {
            return QString::from("");
        };
        let snippet = kestic_core::config::backup_toml_snippet(backup);
        QString::from(&snippet)
    }
}
