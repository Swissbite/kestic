// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Exclude lint/path testing, dry-run previews, retention previews, maintenance previews,
//! and source detection / nested-warning checks for [`super::qobject::KesticApp`].

use super::{dbus_error_text, qobject};
use crate::daemon_events::wake_gui;
use crate::retention_texts::retention_preview_view;
use crate::state_cache::{
    daemon_unavailable_text, format_bytes, json_or, make_client, shared_state, tr_text,
};
use cxx_qt_lib::QString;
use kestic_core::model::{ExcludeRules, RetentionPolicy, normalize_folder_rule};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{KesticClient, KesticDbusError, PreviewStatusDto};
use serde::Serialize;
use std::env;
use std::path::PathBuf;
use std::pin::Pin;

type Setter = fn(Pin<&mut qobject::KesticApp>, QString);

/// The properties through which one kind of preview reports to QML.
struct PreviewSlots {
    id: Setter,
    state: Setter,
    result_json: Setter,
    error: Setter,
}

const DRY_RUN: PreviewSlots = PreviewSlots {
    id: qobject::KesticApp::set_dry_run_preview_id,
    state: qobject::KesticApp::set_dry_run_state,
    result_json: qobject::KesticApp::set_dry_run_result_json,
    error: qobject::KesticApp::set_dry_run_error,
};

const RETENTION: PreviewSlots = PreviewSlots {
    id: qobject::KesticApp::set_retention_preview_id,
    state: qobject::KesticApp::set_retention_preview_state,
    result_json: qobject::KesticApp::set_retention_preview_result_json,
    error: qobject::KesticApp::set_retention_preview_error,
};

impl qobject::KesticApp {
    /// Puts the preview into the error state with `text`.
    fn preview_failed(mut self: Pin<&mut Self>, slots: &PreviewSlots, text: &str) {
        (slots.state)(self.as_mut(), QString::from("error"));
        (slots.error)(self.as_mut(), QString::from(text));
    }

    /// Starts a preview with `start`; returns its id, or an empty string after a failure.
    fn start_preview(
        mut self: Pin<&mut Self>,
        slots: &PreviewSlots,
        start: impl FnOnce(&KesticClient) -> zbus::Result<String>,
    ) -> QString {
        let Ok(client) = make_client() else {
            self.as_mut()
                .preview_failed(slots, &daemon_unavailable_text());
            return QString::from("");
        };
        (slots.result_json)(self.as_mut(), QString::from(""));
        (slots.error)(self.as_mut(), QString::from(""));
        match start(&client) {
            Ok(preview_id) => {
                (slots.id)(self.as_mut(), QString::from(&preview_id));
                (slots.state)(self.as_mut(), QString::from("running"));
                QString::from(&preview_id)
            }
            Err(err) => {
                self.preview_failed(slots, &dbus_error_text(&err));
                QString::from("")
            }
        }
    }

    /// Fetches the status of a preview with `fetch`, publishes it and returns it as JSON.
    fn poll_preview<T: Serialize>(
        mut self: Pin<&mut Self>,
        slots: &PreviewSlots,
        preview_id: &QString,
        fetch: impl FnOnce(&KesticClient, &str) -> Result<PreviewStatusDto<T>, KesticDbusError>,
    ) -> QString {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return QString::from("");
        }
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match fetch(&client, &pid) {
            Ok(status) => {
                (slots.state)(self.as_mut(), QString::from(&status.state));
                if let Some(ref res) = status.result {
                    (slots.result_json)(self.as_mut(), QString::from(&json_or(res, "{}")));
                }
                if let Some(kind) = status.error_kind {
                    let text = tr_text(&kind.text(status.error_detail.as_deref()));
                    (slots.error)(self.as_mut(), QString::from(&text));
                }
                QString::from(&json_or(&status, "{}"))
            }
            Err(err) => {
                self.preview_failed(slots, &dbus_error_text(&err));
                QString::from("")
            }
        }
    }
}

impl qobject::KesticApp {
    /// A size as the models show it, for the sizes QML gets as numbers.
    pub fn format_bytes(&self, bytes: i64) -> QString {
        QString::from(&format_bytes(u64::try_from(bytes).unwrap_or(0)))
    }

    pub fn format_exclude_rule_for_item(&self, item_path: &QString) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.format_exclude_rule_for_item(item_path.to_string().trim()) {
            Ok(dto) => QString::from(&json_or(&dto, "{}")),
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
            Ok(warnings) => QString::from(&json_or(&warnings, "[]")),
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
            Ok(res) => QString::from(&json_or(&res, "{}")),
            Err(_) => QString::from(""),
        }
    }

    pub fn normalize_exclude_folder(&self, folder_path: &QString) -> QString {
        let home = env::var_os("HOME").map(PathBuf::from);
        let normalized = normalize_folder_rule(&folder_path.to_string(), home.as_deref());
        QString::from(&normalized)
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
                    self.as_mut().preview_failed(&DRY_RUN, &err.to_string());
                    return QString::from("");
                }
            }
        };
        self.start_preview(&DRY_RUN, |client| {
            client.start_backup_dry_run(&backup_id.to_string(), excludes_opt.as_ref())
        })
    }

    pub fn poll_backup_dry_run(self: Pin<&mut Self>, preview_id: &QString) -> QString {
        self.poll_preview(
            &DRY_RUN,
            preview_id,
            KesticClient::get_backup_dry_run_status,
        )
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
        QString::from(&json_or(&policy, "{}"))
    }

    pub fn start_retention_preview(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        retention_json: &QString,
    ) -> QString {
        let retention = match serde_json::from_str::<RetentionPolicy>(&retention_json.to_string()) {
            Ok(r) => r,
            Err(err) => {
                self.as_mut().preview_failed(&RETENTION, &err.to_string());
                return QString::from("");
            }
        };
        self.start_preview(&RETENTION, |client| {
            client.start_retention_preview(&backup_id.to_string(), &retention)
        })
    }

    /// Status of the retention preview with its result as the page shows it (translated
    /// reasons, local times, summary).
    pub fn poll_retention_preview(self: Pin<&mut Self>, preview_id: &QString) -> QString {
        self.poll_preview(&RETENTION, preview_id, |client, pid| {
            let status = client.get_retention_preview_status(pid)?;
            Ok(PreviewStatusDto {
                preview_id: status.preview_id,
                state: status.state,
                result: status.result.as_ref().map(retention_preview_view),
                error_kind: status.error_kind,
                error_detail: status.error_detail,
            })
        })
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
            let mut guard = shared_state().lock_unpoisoned();
            guard.maintenance_preview = None;
        }
        std::thread::spawn(move || {
            let outcome = match make_client() {
                Ok(client) => match client.preview_destination_maintenance(&dest) {
                    Ok(summary) => Ok(json_or(&summary, "{}")),
                    Err(err) => Err(dbus_error_text(&err)),
                },
                Err(_) => Err(daemon_unavailable_text()),
            };
            shared_state().lock_unpoisoned().maintenance_preview = Some((dest, outcome));
            wake_gui();
        });
    }

    pub fn detect_source(&self, path: &QString) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.detect_source(path.to_string().trim()) {
            Ok(src) => QString::from(&json_or(&src, "{}")),
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
            Ok(conflicts) => QString::from(&json_or(&conflicts, "[]")),
            Err(_) => QString::from("[]"),
        }
    }
}
