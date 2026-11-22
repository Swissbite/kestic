// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination CRUD, connection testing, password updates, unlocking, and snapshot loading
//! for [`super::qobject::KesticApp`].

use super::{Finish, dbus_error_text, qobject};
use crate::state_cache::{
    build_destination, daemon_unavailable_text, i18n, i18np, make_client, refresh_state_now,
    shared_state, test_destination_connection, tr_text,
};
use cxx_qt_lib::QString;
use kestic_core::destination_types::cards_ready_first;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::KesticDbusError;
use kestic_i18n::codes::ServiceErrorCode;
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

    pub fn test_destination(
        mut self: Pin<&mut Self>,
        fields_json: &QString,
        mode: &QString,
        repo_pw: &QString,
        rest_pw: &QString,
    ) -> bool {
        let res = test_destination_connection(
            &fields_json.to_string(),
            &mode.to_string(),
            &repo_pw.to_string(),
            &rest_pw.to_string(),
        );
        self.as_mut().set_connection_test_ok(res.ok);
        let text = connection_test_text(res.ok, res.error_kind, &res.detail);
        self.as_mut()
            .set_connection_test_message(QString::from(&tr_text(&text)));
        self.as_mut().set_connection_test_error_kind(QString::from(
            res.error_kind.map_or("", |k| k.as_str()),
        ));
        res.ok
    }

    pub fn add_destination(
        mut self: Pin<&mut Self>,
        fields_json: &QString,
        mode: &QString,
        repo_pw: &QString,
        rest_pw: &QString,
    ) -> QString {
        let Some(dest) = self
            .as_mut()
            .ok_or_show(build_destination(None, &fields_json.to_string()))
        else {
            return QString::from("");
        };
        let Ok(client) = make_client() else {
            self.as_mut().show_action_error(&daemon_unavailable_text());
            return QString::from("");
        };
        let result = client.add_destination(
            &dest,
            &mode.to_string(),
            &repo_pw.to_string(),
            &rest_pw.to_string(),
        );
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
        rest_pw: &QString,
        verify_connection: bool,
    ) -> bool {
        let Some(dest) = self.as_mut().ok_or_show(build_destination(
            Some(&id.to_string()),
            &fields_json.to_string(),
        )) else {
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
                &rest_pw.to_string(),
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
