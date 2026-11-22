// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination CRUD, connection testing, password updates, unlocking, and snapshot loading
//! for [`super::qobject::KesticApp`].

use super::{Finish, dbus_error_text, qobject};
use crate::destination_input::{
    build_destination, generic_fields_error, test_destination_connection,
};
use crate::state_cache::{
    daemon_unavailable_text, i18n, i18np, make_client, refresh_state_now, shared_state, tr, tr_text,
};
use cxx_qt_lib::QString;
use kestic_core::destination_types::cards_ready_first;
use kestic_core::local_path::{NotALocalPath, local_path_from_dialog_url};
use kestic_core::model::DestinationKind;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{BackendSecrets, KesticDbusError};
use kestic_i18n::codes::{ConnectionNoteCode, ConnectionTestCode, ServiceErrorCode};
use kestic_i18n::messages::connection_test_text;
use std::pin::Pin;

impl qobject::KesticApp {
    /// Cards of the wizard step «destination type», ready ones first, as
    /// `{"cards": [DestinationTypeCardStatus…], "error": "<text or empty>"}`. The daemon checks
    /// the programs anew on every call («Check Again»).
    pub fn destination_type_cards_json(&self) -> QString {
        let listed = make_client()
            .map_err(|_| daemon_unavailable_text())
            .and_then(|client| {
                client
                    .list_destination_types()
                    .map_err(|e| dbus_error_text(&e))
            });
        let (cards, error) = match listed {
            Ok(types) => (cards_ready_first(&types), String::new()),
            Err(text) => (Vec::new(), text),
        };
        let reply = serde_json::json!({ "cards": cards, "error": error });
        QString::from(&reply.to_string())
    }

    /// Mounted removable drives for the destination type «External Drive / USB», as
    /// `{"drives": [RemovableDriveDto…], "error": "<text or empty>"}`; every call asks anew.
    pub fn removable_drives_json(&self) -> QString {
        let listed = make_client()
            .map_err(|_| daemon_unavailable_text())
            .and_then(|client| {
                client
                    .list_removable_drives()
                    .map_err(|e| dbus_error_text(&e))
            });
        let (drives, error) = match listed {
            Ok(drives) => (drives, String::new()),
            Err(text) => (Vec::new(), text),
        };
        let reply = serde_json::json!({ "drives": drives, "error": error });
        QString::from(&reply.to_string())
    }

    /// Folder chosen in the folder dialog as `{"path": "…", "error": "…"}`: a `file://` URL
    /// becomes its local path; a network address (e.g. `smb://…`) is rejected with a hint.
    pub fn local_folder_from_url(&self, url: &QString) -> QString {
        let reply = match local_path_from_dialog_url(&url.to_string()) {
            Ok(path) => serde_json::json!({ "path": path.to_string_lossy(), "error": "" }),
            Err(NotALocalPath(address)) => serde_json::json!({
                "path": "",
                "error": tr_text(&ConnectionTestCode::NotALocalPath.text(&address)),
            }),
        };
        QString::from(&reply.to_string())
    }

    /// Why the repository URL or the variables of the generic form are refused, in the GUI
    /// language; empty when they are accepted.
    pub fn generic_fields_error(&self, url: &QString, env_json: &QString) -> QString {
        let text = generic_fields_error(&url.to_string(), &env_json.to_string())
            .map_or_else(String::new, |err| err.text());
        QString::from(&text)
    }

    /// Whether «Geheim» starts switched on for a new variable named `name`.
    pub fn env_var_secret_by_default(&self, name: &QString) -> bool {
        kestic_core::generic::secret_by_default(&name.to_string())
    }

    pub fn test_destination(
        mut self: Pin<&mut Self>,
        fields_json: &QString,
        mode: &QString,
        repo_pw: &QString,
        backend_secrets_json: &QString,
    ) -> bool {
        let res = test_destination_connection(
            &fields_json.to_string(),
            &mode.to_string(),
            &repo_pw.to_string(),
            &backend_secrets_json.to_string(),
        );
        self.as_mut().set_connection_test_ok(res.ok);
        let bucket = match build_destination(None, &fields_json.to_string()).map(|d| d.kind) {
            Ok(DestinationKind::S3 { bucket, .. }) => bucket,
            _ => String::new(),
        };
        // Notes without confirmation belong to the result line; the others need a confirmation.
        let (to_confirm, informational): (Vec<ConnectionNoteCode>, Vec<ConnectionNoteCode>) =
            res.notes.iter().partition(|n| n.needs_confirmation());
        let mut lines = vec![tr_text(&connection_test_text(
            res.ok,
            res.error_kind,
            &res.detail,
        ))];
        lines.extend(informational.iter().map(|n| tr(&n.message(&bucket))));
        self.as_mut()
            .set_connection_test_message(QString::from(&lines.join("\n")));
        self.as_mut().set_connection_test_error_kind(QString::from(
            res.error_kind.map_or("", |k| k.as_str()),
        ));
        let note = to_confirm
            .iter()
            .map(|n| tr(&n.message(&bucket)))
            .collect::<Vec<_>>()
            .join("\n");
        self.as_mut().set_connection_test_note(QString::from(&note));
        res.ok
    }

    pub fn add_destination(
        mut self: Pin<&mut Self>,
        fields_json: &QString,
        mode: &QString,
        repo_pw: &QString,
        backend_secrets_json: &QString,
    ) -> QString {
        let Some(dest) = self
            .as_mut()
            .ok_or_show(build_destination(None, &fields_json.to_string()))
        else {
            return QString::from("");
        };
        let Some(backend) = self
            .as_mut()
            .ok_or_show(BackendSecrets::parse(&backend_secrets_json.to_string()))
        else {
            return QString::from("");
        };
        let Ok(client) = make_client() else {
            self.as_mut().show_action_error(&daemon_unavailable_text());
            return QString::from("");
        };
        let result =
            client.add_destination(&dest, &mode.to_string(), &repo_pw.to_string(), &backend);
        let secret_store_locked = result.as_ref().err().is_some_and(|err| {
            KesticDbusError::from(err.clone()).code() == ServiceErrorCode::SecretStoreLocked
        });
        let id = self.as_mut().finish_action(result, Finish::Plain);
        if secret_store_locked {
            refresh_state_now();
            self.as_mut().apply_cached_state();
        }
        QString::from(&id.unwrap_or_default())
    }

    pub fn update_destination(
        mut self: Pin<&mut Self>,
        id: &QString,
        fields_json: &QString,
        mode: &QString,
        repo_pw: &QString,
        backend_secrets_json: &QString,
        verify_connection: bool,
    ) -> bool {
        let Some(dest) = self.as_mut().ok_or_show(build_destination(
            Some(&id.to_string()),
            &fields_json.to_string(),
        )) else {
            return false;
        };
        let Some(backend) = self
            .as_mut()
            .ok_or_show(BackendSecrets::parse(&backend_secrets_json.to_string()))
        else {
            return false;
        };
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(
            client.update_destination(
                &dest,
                &mode.to_string(),
                &repo_pw.to_string(),
                &backend,
                verify_connection,
            ),
            Finish::Saved,
        )
        .is_some()
    }

    pub fn update_destination_password(
        mut self: Pin<&mut Self>,
        dest_id: &QString,
        new_pw: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let result = client.update_destination_password(&dest_id.to_string(), &new_pw.to_string());
        let updated = self.as_mut().finish_action(result, Finish::Plain).is_some();
        if updated {
            self.as_mut()
                .passive_notification(QString::from(&i18n("Saved password updated").to_string()));
        }
        updated
    }

    pub fn remove_destination(self: Pin<&mut Self>, dest_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        self.finish_action(
            client.remove_destination(&dest_id.to_string()),
            Finish::NotifyOnError,
        )
        .is_some()
    }

    pub fn unlock_destination(mut self: Pin<&mut Self>, dest_id: &QString) -> i32 {
        let Ok(client) = make_client() else {
            self.as_mut().show_action_error(&daemon_unavailable_text());
            return -1;
        };
        let result = client.unlock_destination(&dest_id.to_string());
        let Some(count) = self.as_mut().finish_action(result, Finish::NotifyOnError) else {
            return -1;
        };
        let msg = if count == 0 {
            i18n("No stale locks found.").to_string()
        } else {
            i18np(
                "%1 stale lock removed.",
                "%1 stale locks removed.",
                count,
                &[],
            )
        };
        self.as_mut().passive_notification(QString::from(&msg));
        i32::try_from(count).unwrap_or(0)
    }

    pub fn load_backup_snapshots(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        self.as_mut().set_snapshots_loading(true);
        self.as_mut().set_snapshots_error(QString::from(""));
        let Ok(client) = make_client() else {
            self.as_mut().set_snapshots_loading(false);
            self.as_mut()
                .set_snapshots_error(QString::from(&daemon_unavailable_text()));
            return false;
        };
        match client.list_backup_snapshots(&backup_id.to_string()) {
            Ok(dto) => {
                let err_msg = dto.current_error.clone().unwrap_or_default();
                let has_prev = !dto.previous_destinations.is_empty();
                {
                    let mut guard = shared_state().lock_unpoisoned();
                    guard.snapshots = Some(dto);
                }
                self.as_mut().set_snapshots_loading(false);
                self.as_mut().set_snapshots_error(QString::from(&err_msg));
                self.as_mut().set_snapshots_has_previous(has_prev);
                self.as_mut().snapshots_updated();
                true
            }
            Err(err) => {
                shared_state().lock_unpoisoned().snapshots = None;
                self.as_mut().set_snapshots_loading(false);
                self.as_mut()
                    .set_snapshots_error(QString::from(&dbus_error_text(&err)));
                self.as_mut().set_snapshots_has_previous(false);
                self.as_mut().snapshots_updated();
                false
            }
        }
    }
}
