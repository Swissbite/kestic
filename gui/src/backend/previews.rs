// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Exclude lint/path testing, dry-run previews, retention previews, maintenance previews,
//! and source detection / nested-warning checks for [`super::qobject::KesticApp`].

use super::daemon_call::Lane;
use super::{dbus_error_text, qobject};
use crate::daemon_events::wake_gui;
use crate::retention_texts::{RetentionPreviewView, retention_preview_view};
use crate::state_cache::{daemon_unavailable_text, format_bytes, json_or, shared_state, tr_text};
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

    /// Starts a preview with `start`; the request answers with its id, or with an empty text
    /// after a failure.
    fn start_preview(
        mut self: Pin<&mut Self>,
        slots: &'static PreviewSlots,
        lane: Lane,
        start: impl FnOnce(&KesticClient) -> zbus::Result<String> + Send + 'static,
    ) -> i32 {
        (slots.result_json)(self.as_mut(), QString::from(""));
        (slots.error)(self.as_mut(), QString::from(""));
        (slots.state)(self.as_mut(), QString::from("running"));
        self.request_daemon_with(
            lane,
            start,
            |_| false,
            move |mut app, started| match started {
                Ok(Ok(preview_id)) => {
                    (slots.id)(app.as_mut(), QString::from(&preview_id));
                    (true, preview_id)
                }
                Ok(Err(err)) => {
                    app.preview_failed(slots, &dbus_error_text(&err));
                    (false, String::new())
                }
                Err(_) => {
                    app.preview_failed(slots, &daemon_unavailable_text());
                    (false, String::new())
                }
            },
        )
    }

    /// Fetches the status of a preview with `fetch`; publishes it and answers the request with it
    /// as JSON (empty after a failure).
    fn poll_preview<T: Serialize + Send + 'static>(
        self: Pin<&mut Self>,
        slots: &'static PreviewSlots,
        preview_id: &QString,
        fetch: impl FnOnce(&KesticClient, &str) -> Result<PreviewStatusDto<T>, KesticDbusError>
        + Send
        + 'static,
    ) -> i32 {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return 0;
        }
        let lane = Lane::slow(&format!("preview:{pid}"));
        self.request_daemon_with(
            lane,
            move |client| fetch(client, &pid),
            |_| false,
            move |app, status| app.show_preview_status(slots, status),
        )
    }

    /// Publishes a fetched preview status; returns whether it arrived and its JSON.
    fn show_preview_status<T: Serialize>(
        mut self: Pin<&mut Self>,
        slots: &PreviewSlots,
        status: Result<Result<PreviewStatusDto<T>, KesticDbusError>, String>,
    ) -> (bool, String) {
        match status {
            Ok(Ok(status)) => {
                (slots.state)(self.as_mut(), QString::from(&status.state));
                if let Some(ref res) = status.result {
                    (slots.result_json)(self.as_mut(), QString::from(&json_or(res, "{}")));
                }
                if let Some(kind) = status.error_kind {
                    let text = tr_text(&kind.text(status.error_detail.as_deref()));
                    (slots.error)(self.as_mut(), QString::from(&text));
                }
                (true, json_or(&status, "{}"))
            }
            Ok(Err(err)) => {
                self.preview_failed(slots, &dbus_error_text(&err));
                (false, String::new())
            }
            Err(_) => (false, String::new()),
        }
    }
}

impl qobject::KesticApp {
    /// A size as the models show it, for the sizes QML gets as numbers.
    pub fn format_bytes(&self, bytes: i64) -> QString {
        QString::from(&format_bytes(u64::try_from(bytes).unwrap_or(0)))
    }

    /// The exclude rule for `item_path` as JSON in the request's result (empty after a failure).
    pub fn format_exclude_rule_for_item(self: Pin<&mut Self>, item_path: &QString) -> i32 {
        let path = item_path.to_string().trim().to_string();
        self.ask_daemon(Lane::slow("exclude-rules"), move |client| {
            client
                .format_exclude_rule_for_item(&path)
                .ok()
                .map(|dto| json_or(&dto, "{}"))
        })
    }

    /// The warnings about `excludes_json` as a JSON list in the request's result (`[]` after a
    /// failure).
    pub fn lint_excludes(self: Pin<&mut Self>, excludes_json: &QString) -> i32 {
        let excludes = serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()).ok();
        self.ask_daemon(Lane::slow("exclude-rules"), move |client| {
            let warnings = excludes.and_then(|e| client.lint_excludes(&e).ok());
            Some(warnings.map_or_else(|| "[]".to_string(), |w| json_or(&w, "[]")))
        })
    }

    /// Whether `test_path` would be excluded, as JSON in the request's result (empty after a
    /// failure).
    pub fn test_exclude_path(
        self: Pin<&mut Self>,
        backup_id: &QString,
        excludes_json: &QString,
        test_path: &QString,
    ) -> i32 {
        let excludes = serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()).ok();
        let (backup_id, test_path) = (backup_id.to_string(), test_path.to_string());
        self.ask_daemon(Lane::slow("exclude-rules"), move |client| {
            let excludes = excludes?;
            client
                .test_exclude_path(&backup_id, &excludes, &test_path)
                .ok()
                .map(|res| json_or(&res, "{}"))
        })
    }

    /// Asks the daemon with `call`; the request answers with its text, or fails with an empty
    /// one when `call` gives none or the daemon cannot be reached.
    fn ask_daemon(
        self: Pin<&mut Self>,
        lane: Lane,
        call: impl FnOnce(&KesticClient) -> Option<String> + Send + 'static,
    ) -> i32 {
        self.request_daemon_with(
            lane,
            call,
            |_| false,
            |_, answer| match answer {
                Ok(Some(text)) => (true, text),
                Ok(None) | Err(_) => (false, String::new()),
            },
        )
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
    ) -> i32 {
        let excludes_opt = if excludes_json.to_string().trim().is_empty() {
            None
        } else {
            match serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()) {
                Ok(e) => Some(e),
                Err(err) => {
                    self.as_mut().preview_failed(&DRY_RUN, &err.to_string());
                    return 0;
                }
            }
        };
        let backup_id = backup_id.to_string();
        let lane = Lane::slow(&format!("dry-run:{backup_id}"));
        self.start_preview(&DRY_RUN, lane, move |client| {
            client.start_backup_dry_run(&backup_id, excludes_opt.as_ref())
        })
    }

    pub fn poll_backup_dry_run(self: Pin<&mut Self>, preview_id: &QString) -> i32 {
        self.poll_preview(
            &DRY_RUN,
            preview_id,
            KesticClient::get_backup_dry_run_status,
        )
    }

    /// Cancels the dry run; its status shows what happened.
    pub fn cancel_backup_dry_run(self: Pin<&mut Self>, preview_id: &QString) {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return;
        }
        let lane = Lane::slow(&format!("preview:{pid}"));
        self.call_daemon(
            lane,
            move |client| {
                // A preview that ended meanwhile needs no cancel; the status tells.
                if let Err(err) = client.cancel_backup_dry_run(&pid) {
                    eprintln!("kestic: cancelling the dry run failed: {err}");
                }
                client.get_backup_dry_run_status(&pid)
            },
            |app, status| {
                app.show_preview_status(&DRY_RUN, status);
            },
        );
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
    ) -> i32 {
        let retention = match serde_json::from_str::<RetentionPolicy>(&retention_json.to_string()) {
            Ok(r) => r,
            Err(err) => {
                self.as_mut().preview_failed(&RETENTION, &err.to_string());
                return 0;
            }
        };
        let backup_id = backup_id.to_string();
        let lane = Lane::slow(&format!("retention-preview:{backup_id}"));
        self.start_preview(&RETENTION, lane, move |client| {
            client.start_retention_preview(&backup_id, &retention)
        })
    }

    /// Status of the retention preview with its result as the page shows it (translated
    /// reasons, local times, summary).
    pub fn poll_retention_preview(self: Pin<&mut Self>, preview_id: &QString) -> i32 {
        self.poll_preview(&RETENTION, preview_id, retention_status)
    }

    /// Cancels the retention preview; its status shows what happened.
    pub fn cancel_retention_preview(self: Pin<&mut Self>, preview_id: &QString) {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return;
        }
        let lane = Lane::slow(&format!("preview:{pid}"));
        self.call_daemon(
            lane,
            move |client| {
                // A preview that ended meanwhile needs no cancel; the status tells.
                if let Err(err) = client.cancel_retention_preview(&pid) {
                    eprintln!("kestic: cancelling the retention preview failed: {err}");
                }
                retention_status(client, &pid)
            },
            |app, status| {
                app.show_preview_status(&RETENTION, status);
            },
        );
    }

    pub fn preview_destination_maintenance(mut self: Pin<&mut Self>, dest_id: &QString) {
        let dest = dest_id.to_string();
        self.as_mut().set_maintenance_preview_loading(true);
        self.as_mut().set_last_action_error(QString::from(""));
        {
            let mut guard = shared_state().lock_unpoisoned();
            guard.maintenance_preview = None;
        }
        let asked = dest.clone();
        self.call_daemon(
            Lane::Alone,
            move |client| {
                client
                    .preview_destination_maintenance(&asked)
                    .map(|summary| json_or(&summary, "{}"))
                    .map_err(|err| dbus_error_text(&err))
            },
            move |_, result| {
                let outcome = result.unwrap_or_else(|_| Err(daemon_unavailable_text()));
                shared_state().lock_unpoisoned().maintenance_preview = Some((dest, outcome));
                wake_gui();
            },
        );
    }

    /// What the daemon finds at `path` (`DetectedSourceDto`) as JSON in the request's result
    /// (empty after a failure).
    pub fn detect_source(self: Pin<&mut Self>, path: &QString) -> i32 {
        let path = path.to_string().trim().to_string();
        self.ask_daemon(Lane::slow("source"), move |client| {
            client
                .detect_source(&path)
                .ok()
                .map(|src| json_or(&src, "{}"))
        })
    }

    /// The backups whose sources overlap the candidate's, as a JSON list in the request's result
    /// (`[]` after a failure).
    pub fn check_nested_sources(
        self: Pin<&mut Self>,
        candidate_id: &QString,
        candidate_path: &QString,
        destination_id: &QString,
    ) -> i32 {
        let candidate_id = candidate_id.to_string();
        let candidate_path = candidate_path.to_string().trim().to_string();
        let destination_id = destination_id.to_string().trim().to_string();
        self.ask_daemon(Lane::slow("source"), move |client| {
            let conflicts = client
                .check_nested_sources(&candidate_id, &candidate_path, &destination_id)
                .ok();
            Some(conflicts.map_or_else(|| "[]".to_string(), |c| json_or(&c, "[]")))
        })
    }
}

/// Status of the retention preview `pid` with its result as the page shows it (translated
/// reasons, local times, summary).
fn retention_status(
    client: &KesticClient,
    pid: &str,
) -> Result<PreviewStatusDto<RetentionPreviewView>, KesticDbusError> {
    let status = client.get_retention_preview_status(pid)?;
    Ok(PreviewStatusDto {
        preview_id: status.preview_id,
        state: status.state,
        result: status.result.as_ref().map(retention_preview_view),
        error_kind: status.error_kind,
        error_detail: status.error_detail,
    })
}
