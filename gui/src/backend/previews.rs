// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Exclude lint/path testing, dry-run previews, retention previews, maintenance previews,
//! and source detection / nested-warning checks for [`super::qobject::KesticApp`].

use super::{dbus_error_text, qobject};
use crate::state_cache::{
    daemon_unavailable_text, format_retention_summary, make_client, shared_state, tr_text,
};
use cxx_qt_lib::QString;
use kestic_core::model::{ExcludeRules, RetentionPolicy, normalize_folder_rule};
use std::env;
use std::path::PathBuf;
use std::pin::Pin;

impl qobject::KesticApp {
    pub fn format_exclude_rule_for_item(&self, item_path: &QString) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.format_exclude_rule_for_item(item_path.to_string().trim()) {
            Ok(dto) => {
                QString::from(&serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(_) => QString::from(""),
        }
    }

    pub fn lint_excludes(&self, excludes_json: &QString) -> QString {
        let Ok(excludes) = serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()) else {
            return QString::from("[]");
        };
        let Ok(client) = make_client() else {
            return QString::from("[]");
        };
        match client.lint_excludes(&excludes) {
            Ok(warnings) => QString::from(
                &serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".to_string()),
            ),
            Err(_) => QString::from("[]"),
        }
    }

    pub fn test_exclude_path(
        &self,
        backup_id: &QString,
        excludes_json: &QString,
        test_path: &QString,
    ) -> QString {
        let Ok(excludes) = serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()) else {
            return QString::from("");
        };
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.test_exclude_path(&backup_id.to_string(), &excludes, &test_path.to_string()) {
            Ok(res) => {
                QString::from(&serde_json::to_string(&res).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(_) => QString::from(""),
        }
    }

    pub fn normalize_exclude_folder(&self, folder_path: &QString) -> QString {
        let home = env::var_os("HOME").map(PathBuf::from);
        let normalized = normalize_folder_rule(&folder_path.to_string(), home.as_deref());
        QString::from(&normalized)
    }

    pub fn build_exclude_file_preview(&self, excludes_json: &QString) -> QString {
        let Ok(excludes) = serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()) else {
            return QString::from("");
        };
        QString::from(&excludes.to_exclude_file_contents())
    }

    pub fn start_backup_dry_run(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        excludes_json: &QString,
    ) -> QString {
        let excludes_opt = if excludes_json.to_string().trim().is_empty() {
            None
        } else {
            match serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()) {
                Ok(e) => Some(e),
                Err(err) => {
                    self.as_mut().set_dry_run_state(QString::from("error"));
                    self.as_mut()
                        .set_dry_run_error(QString::from(&err.to_string()));
                    return QString::from("");
                }
            }
        };
        let Ok(client) = make_client() else {
            self.as_mut().set_dry_run_state(QString::from("error"));
            self.as_mut()
                .set_dry_run_error(QString::from(&daemon_unavailable_text()));
            return QString::from("");
        };
        self.as_mut().set_dry_run_result_json(QString::from(""));
        self.as_mut().set_dry_run_error(QString::from(""));
        match client.start_backup_dry_run(&backup_id.to_string(), excludes_opt.as_ref()) {
            Ok(preview_id) => {
                self.as_mut()
                    .set_dry_run_preview_id(QString::from(&preview_id));
                self.as_mut().set_dry_run_state(QString::from("running"));
                QString::from(&preview_id)
            }
            Err(err) => {
                let msg = dbus_error_text(&err);
                self.as_mut().set_dry_run_state(QString::from("error"));
                self.as_mut().set_dry_run_error(QString::from(&msg));
                QString::from("")
            }
        }
    }

    pub fn poll_backup_dry_run(mut self: Pin<&mut Self>, preview_id: &QString) -> QString {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return QString::from("");
        }
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.get_backup_dry_run_status(&pid) {
            Ok(status) => {
                self.as_mut()
                    .set_dry_run_state(QString::from(&status.state));
                if let Some(ref res) = status.result {
                    let rjson = serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string());
                    self.as_mut().set_dry_run_result_json(QString::from(&rjson));
                }
                if let Some(kind) = status.error_kind {
                    let text = tr_text(&kind.text(status.error_detail.as_deref()));
                    self.as_mut().set_dry_run_error(QString::from(&text));
                }
                QString::from(&serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(err) => {
                self.as_mut().set_dry_run_state(QString::from("error"));
                self.as_mut()
                    .set_dry_run_error(QString::from(&dbus_error_text(&err)));
                QString::from("")
            }
        }
    }

    pub fn cancel_backup_dry_run(mut self: Pin<&mut Self>, preview_id: &QString) {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return;
        }
        if let Ok(client) = make_client() {
            let _ = client.cancel_backup_dry_run(&pid);
            let _ = self.as_mut().poll_backup_dry_run(preview_id);
        }
    }

    pub fn retention_preset_json(&self, preset_name: &QString) -> QString {
        let policy = match preset_name.to_string().as_str() {
            "economical" => RetentionPolicy::economical(),
            "long" => RetentionPolicy::long(),
            _ => RetentionPolicy::standard(),
        };
        QString::from(&serde_json::to_string(&policy).unwrap_or_else(|_| "{}".to_string()))
    }

    pub fn format_retention_summary_json(&self, retention_json: &QString) -> QString {
        let Ok(policy) = serde_json::from_str::<RetentionPolicy>(&retention_json.to_string())
        else {
            return QString::from("");
        };
        QString::from(&format_retention_summary(&policy))
    }

    pub fn start_retention_preview(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        retention_json: &QString,
    ) -> QString {
        let retention = match serde_json::from_str::<RetentionPolicy>(&retention_json.to_string()) {
            Ok(r) => r,
            Err(err) => {
                self.as_mut()
                    .set_retention_preview_state(QString::from("error"));
                self.as_mut()
                    .set_retention_preview_error(QString::from(&err.to_string()));
                return QString::from("");
            }
        };
        let Ok(client) = make_client() else {
            self.as_mut()
                .set_retention_preview_state(QString::from("error"));
            self.as_mut()
                .set_retention_preview_error(QString::from(&daemon_unavailable_text()));
            return QString::from("");
        };
        self.as_mut()
            .set_retention_preview_result_json(QString::from(""));
        self.as_mut().set_retention_preview_error(QString::from(""));
        match client.start_retention_preview(&backup_id.to_string(), &retention) {
            Ok(preview_id) => {
                self.as_mut()
                    .set_retention_preview_id(QString::from(&preview_id));
                self.as_mut()
                    .set_retention_preview_state(QString::from("running"));
                QString::from(&preview_id)
            }
            Err(err) => {
                let msg = dbus_error_text(&err);
                self.as_mut()
                    .set_retention_preview_state(QString::from("error"));
                self.as_mut()
                    .set_retention_preview_error(QString::from(&msg));
                QString::from("")
            }
        }
    }

    pub fn poll_retention_preview(mut self: Pin<&mut Self>, preview_id: &QString) -> QString {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return QString::from("");
        }
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.get_retention_preview_status(&pid) {
            Ok(status) => {
                self.as_mut()
                    .set_retention_preview_state(QString::from(&status.state));
                if let Some(ref res) = status.result {
                    let rjson = serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string());
                    self.as_mut()
                        .set_retention_preview_result_json(QString::from(&rjson));
                }
                if let Some(kind) = status.error_kind {
                    let text = tr_text(&kind.text(status.error_detail.as_deref()));
                    self.as_mut()
                        .set_retention_preview_error(QString::from(&text));
                }
                QString::from(&serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(err) => {
                self.as_mut()
                    .set_retention_preview_state(QString::from("error"));
                self.as_mut()
                    .set_retention_preview_error(QString::from(&dbus_error_text(&err)));
                QString::from("")
            }
        }
    }

    pub fn cancel_retention_preview(mut self: Pin<&mut Self>, preview_id: &QString) {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return;
        }
        if let Ok(client) = make_client() {
            let _ = client.cancel_retention_preview(&pid);
            let _ = self.as_mut().poll_retention_preview(preview_id);
        }
    }

    pub fn preview_destination_maintenance(mut self: Pin<&mut Self>, dest_id: &QString) {
        let dest = dest_id.to_string();
        self.as_mut().set_maintenance_preview_loading(true);
        self.as_mut().set_last_action_error(QString::from(""));
        {
            let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard.maintenance_preview = None;
        }
        std::thread::spawn(move || {
            let outcome = match make_client() {
                Ok(client) => match client.preview_destination_maintenance(&dest) {
                    Ok(summary) => {
                        Ok(serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_string()))
                    }
                    Err(err) => Err(dbus_error_text(&err)),
                },
                Err(_) => Err(daemon_unavailable_text()),
            };
            let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard.maintenance_preview = Some((dest, outcome));
        });
    }

    pub fn detect_source(&self, path: &QString) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.detect_source(path.to_string().trim()) {
            Ok(src) => {
                QString::from(&serde_json::to_string(&src).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(_) => QString::from(""),
        }
    }

    pub fn check_nested_sources(
        &self,
        candidate_id: &QString,
        candidate_path: &QString,
        destination_id: &QString,
    ) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("[]");
        };
        match client.check_nested_sources(
            &candidate_id.to_string(),
            candidate_path.to_string().trim(),
            destination_id.to_string().trim(),
        ) {
            Ok(conflicts) => QString::from(
                &serde_json::to_string(&conflicts).unwrap_or_else(|_| "[]".to_string()),
            ),
            Err(_) => QString::from("[]"),
        }
    }
}
