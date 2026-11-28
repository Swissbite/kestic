// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination CRUD, connection testing, password updates, unlocking, and snapshot loading
//! for [`super::qobject::KesticApp`].

use super::daemon_call::Lane;
use super::revisions::{
    CONFLICT_ANSWER, cached_destination_revision, is_conflict_reply, record_own, sent_revision,
};
use super::{Finish, dbus_error_text, qobject};
use crate::destination_input::{
    build_destination, generic_credentials_error, generic_env_error, generic_url_error,
    revision_of_fields, test_destination_connection,
};
use crate::state_cache::{daemon_unavailable_text, i18n, i18np, shared_state, tr, tr_text};
use cxx_qt_lib::QString;
use kestic_core::destination_types::cards_ready_first;
use kestic_core::local_path::{NotALocalPath, local_path_from_dialog_url};
use kestic_core::model::DestinationKind;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::BackendSecrets;
use kestic_i18n::codes::{ConnectionNoteCode, ConnectionTestCode, DestinationUpdateOutcome};
use kestic_i18n::messages::connection_test_text;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};

/// How many snapshot lists the window has asked for; a list answers the latest one only.
static SNAPSHOTS_ASKED: AtomicU64 = AtomicU64::new(0);

impl qobject::KesticApp {
    /// Cards of the wizard step «destination type», ready ones first, as
    /// `{"cards": [DestinationTypeCardStatus…], "error": "<text or empty>"}` in the request's
    /// result. The daemon checks the programs anew on every call («Check Again»).
    pub fn destination_type_cards_json(self: Pin<&mut Self>) -> i32 {
        self.request_daemon_with(
            Lane::slow("destination-types"),
            |client| client.list_destination_types(),
            |_| false,
            |_, listed| {
                let (cards, error) = match listed {
                    Ok(Ok(types)) => (cards_ready_first(&types), String::new()),
                    Ok(Err(err)) => (Vec::new(), dbus_error_text(&err)),
                    Err(_) => (Vec::new(), daemon_unavailable_text()),
                };
                (
                    true,
                    serde_json::json!({ "cards": cards, "error": error }).to_string(),
                )
            },
        )
    }

    /// Mounted removable drives for the destination type «External Drive / USB», as
    /// `{"drives": [RemovableDriveDto…], "error": "<text or empty>"}` in the request's result;
    /// every call asks anew.
    pub fn removable_drives_json(self: Pin<&mut Self>) -> i32 {
        self.request_daemon_with(
            Lane::slow("removable-drives"),
            |client| client.list_removable_drives(),
            |_| false,
            |_, listed| {
                let (drives, error) = match listed {
                    Ok(Ok(drives)) => (drives, String::new()),
                    Ok(Err(err)) => (Vec::new(), dbus_error_text(&err)),
                    Err(_) => (Vec::new(), daemon_unavailable_text()),
                };
                (
                    true,
                    serde_json::json!({ "drives": drives, "error": error }).to_string(),
                )
            },
        )
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

    /// Why the repository URL and the variables of the generic form are refused, in the GUI
    /// language, as `{"url": "<text or empty>", "env": "<text or empty>"}`; `env_json` holds the
    /// variables with the typed values of the secret ones, for the check of the credential
    /// variables an `azure:` or `gs:` destination needs.
    pub fn generic_fields_errors(&self, url: &QString, env_json: &QString) -> QString {
        let text = |err: Option<crate::destination_input::DestinationInputError>| {
            err.map_or_else(String::new, |e| e.text())
        };
        let (url, env_json) = (url.to_string(), env_json.to_string());
        let env_error =
            generic_env_error(&env_json).or_else(|| generic_credentials_error(&url, &env_json));
        let reply = serde_json::json!({
            "url": text(generic_url_error(&url)),
            "env": text(env_error),
        });
        QString::from(&reply.to_string())
    }

    /// Which S3 and SFTP fields of `fields_json` `core` accepts (JSON object of booleans).
    pub fn destination_field_checks(&self, fields_json: &QString) -> QString {
        QString::from(&crate::destination_input::field_checks(&fields_json.to_string()).to_string())
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
        let host_key = res
            .host_key
            .as_ref()
            .and_then(|key| serde_json::to_string(key).ok())
            .unwrap_or_default();
        self.as_mut()
            .set_connection_test_host_key(QString::from(&host_key));
        res.ok
    }

    /// Trusts the SFTP host key the last connection test showed (`TrustHostKey`).
    pub fn trust_host_key(self: Pin<&mut Self>, host_key_id: &QString) -> i32 {
        let key = host_key_id.to_string();
        self.request_daemon(
            Lane::settings(),
            move |client| client.trust_host_key(&key),
            |app, result| {
                (
                    app.finish_daemon_action(result, Finish::Plain).is_some(),
                    String::new(),
                )
            },
        )
    }

    pub fn add_destination(
        mut self: Pin<&mut Self>,
        fields_json: &QString,
        mode: &QString,
        repo_pw: &QString,
        backend_secrets_json: &QString,
    ) -> i32 {
        let Some(dest) = self
            .as_mut()
            .ok_or_show(build_destination(None, &fields_json.to_string()))
        else {
            return 0;
        };
        let Some(backend) = self
            .as_mut()
            .ok_or_show(BackendSecrets::parse(&backend_secrets_json.to_string()))
        else {
            return 0;
        };
        let (mode, repo_pw) = (mode.to_string(), repo_pw.to_string());
        // The state is fetched before the answer: it holds the new destination (a backup added
        // next needs its kind) or shows a locked password store.
        self.request_daemon(
            Lane::slow("add-destination"),
            move |client| client.add_destination(&dest, &mode, &repo_pw, &backend),
            |mut app, result| {
                let id = app.as_mut().finish_daemon_action(result, Finish::Plain);
                app.as_mut().apply_cached_state();
                (id.is_some(), id.unwrap_or_default())
            },
        )
    }

    pub fn update_destination(
        mut self: Pin<&mut Self>,
        id: &QString,
        fields_json: &QString,
        mode: &QString,
        repo_pw: &QString,
        backend_secrets_json: &QString,
        verify_connection: bool,
    ) -> i32 {
        // The page adds the revision of its opening to the fields.
        let revision = revision_of_fields(&fields_json.to_string());
        let Some(dest) = self.as_mut().ok_or_show(build_destination(
            Some(&id.to_string()),
            &fields_json.to_string(),
        )) else {
            return 0;
        };
        let Some(backend) = self
            .as_mut()
            .ok_or_show(BackendSecrets::parse(&backend_secrets_json.to_string()))
        else {
            return 0;
        };
        let (mode, repo_pw) = (mode.to_string(), repo_pw.to_string());
        self.request_daemon(
            Lane::slow(&format!("update-destination:{}", dest.id)),
            move |client| {
                let current = client
                    .get_state()
                    .ok()
                    .and_then(|s| {
                        s.destinations
                            .into_iter()
                            .find(|d| d.destination.id == dest.id)
                            .map(|d| d.revision)
                    })
                    .unwrap_or_default();
                let sent = sent_revision(&revision, &current);
                let saved = client.update_destination(
                    &dest,
                    &mode,
                    &repo_pw,
                    &backend,
                    verify_connection,
                    &sent,
                );
                if let Ok(result) = &saved
                    && update_refusal(result.outcome).is_none()
                {
                    record_own(&sent, &result.revision);
                }
                saved
            },
            |mut app, result| {
                if is_conflict_reply(&result) {
                    return (false, CONFLICT_ANSWER.to_string());
                }
                let Some(result) = app.as_mut().finish_daemon_action(result, Finish::Plain) else {
                    return (false, String::new());
                };
                match update_refusal(result.outcome) {
                    None => {
                        app.as_mut().set_has_unsaved_changes(false);
                        (true, result.revision)
                    }
                    Some(code) => {
                        // The edit page shows it like the connection test, with its action.
                        app.as_mut().show_action_error(&tr_text(&code.text("")));
                        app.as_mut()
                            .set_connection_test_error_kind(QString::from(code.as_str()));
                        (false, String::new())
                    }
                }
            },
        )
    }

    pub fn destination_revision(&self, dest_id: &QString) -> QString {
        QString::from(&cached_destination_revision(&dest_id.to_string()))
    }

    pub fn update_destination_password(
        self: Pin<&mut Self>,
        dest_id: &QString,
        new_pw: &QString,
    ) -> i32 {
        let (dest_id, new_pw) = (dest_id.to_string(), new_pw.to_string());
        self.request_daemon(
            Lane::slow(&format!("password:{dest_id}")),
            move |client| client.update_destination_password(&dest_id, &new_pw),
            |mut app, result| {
                let updated = app
                    .as_mut()
                    .finish_daemon_action(result, Finish::Plain)
                    .is_some();
                if updated {
                    app.as_mut().passive_notification(QString::from(
                        &i18n("Saved password updated").to_string(),
                    ));
                }
                (updated, String::new())
            },
        )
    }

    pub fn remove_destination(self: Pin<&mut Self>, dest_id: &QString) -> i32 {
        let dest_id = dest_id.to_string();
        self.request_daemon(
            Lane::slow(&format!("remove-destination:{dest_id}")),
            move |client| client.remove_destination(&dest_id),
            |app, result| {
                (
                    app.finish_daemon_action(result, Finish::NotifyOnError)
                        .is_some(),
                    String::new(),
                )
            },
        )
    }

    pub fn unlock_destination(self: Pin<&mut Self>, dest_id: &QString) {
        let dest_id = dest_id.to_string();
        // Removing the lock may take a while; other actions do not wait for it.
        self.call_daemon_with(
            Lane::slow(&format!("unlock:{dest_id}")),
            move |client| client.unlock_destination(&dest_id),
            |_| false,
            |mut app, result| {
                let Some(count) = app
                    .as_mut()
                    .finish_daemon_action(result, Finish::NotifyOnError)
                else {
                    return;
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
                app.as_mut().passive_notification(QString::from(&msg));
            },
        );
    }

    /// Loads the snapshot list of `backup_id`; `snapshotsUpdated` tells QML when it is there.
    pub fn load_backup_snapshots(mut self: Pin<&mut Self>, backup_id: &QString) {
        self.as_mut().set_snapshots_loading(true);
        self.as_mut().set_snapshots_error(QString::from(""));
        let b_id = backup_id.to_string();
        let wanted = b_id.clone();
        let asked = SNAPSHOTS_ASKED.fetch_add(1, Ordering::SeqCst) + 1;
        self.call_daemon(
            // A stale reply is dropped (below), so the requests need no order of their own.
            Lane::Alone,
            move |client| client.list_backup_snapshots(&b_id),
            move |mut app, listed| {
                // Only the list asked for last is shown, whichever backup the earlier ones were for.
                if SNAPSHOTS_ASKED.load(Ordering::SeqCst) != asked {
                    return;
                }
                let (dto, error) = match listed {
                    Ok(Ok(dto)) => {
                        let error = dto.current_error.clone().unwrap_or_default();
                        (Some(dto), error)
                    }
                    Ok(Err(err)) => (None, dbus_error_text(&err)),
                    Err(_) => (None, daemon_unavailable_text()),
                };
                let has_prev = dto
                    .as_ref()
                    .is_some_and(|d| !d.previous_destinations.is_empty());
                shared_state().lock_unpoisoned().snapshots = dto;
                app.as_mut().set_snapshots_loading(false);
                app.as_mut().set_snapshots_error(QString::from(&error));
                app.as_mut().set_snapshots_has_previous(has_prev);
                app.as_mut().set_snapshots_backup_id(QString::from(&wanted));
                app.as_mut().snapshots_updated();
            },
        );
    }
}

/// The connection-test code under which the edit page shows a refused `UpdateDestination`, or
/// `None` when it was saved.
fn update_refusal(outcome: DestinationUpdateOutcome) -> Option<ConnectionTestCode> {
    match outcome {
        DestinationUpdateOutcome::Saved => None,
        DestinationUpdateOutcome::OtherRepository => Some(ConnectionTestCode::OtherRepository),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-M5-05-7 (a), (b): an `UpdateDestination` that found another repository after a passed
    /// test is not «saved» but shown as another backup destination.
    #[test]
    fn ac_m5_05_7_refused_update_is_shown_as_other_repository() {
        assert_eq!(update_refusal(DestinationUpdateOutcome::Saved), None);
        assert_eq!(
            update_refusal(DestinationUpdateOutcome::OtherRepository),
            Some(ConnectionTestCode::OtherRepository)
        );
    }
}
