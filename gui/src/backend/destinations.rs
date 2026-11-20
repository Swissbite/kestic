// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination CRUD, connection testing, password updates, unlocking, and snapshot loading
//! for [`super::qobject::KesticApp`].

use super::{dbus_error_text, qobject};
use crate::state_cache::{
    build_destination, daemon_unavailable_text, make_client, refresh_state_now, shared_state,
    test_destination_connection, tr_text,
};
use cxx_kde_frameworks::ki18n::i18n;
use cxx_qt_lib::QString;
use kestic_daemon::dbus_api::KesticDbusError;
use kestic_i18n::codes::ServiceErrorCode;
use kestic_i18n::messages::connection_test_text;
use std::pin::Pin;

impl qobject::KesticApp {
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
            res.error_kind.map(|k| k.as_str()).unwrap_or(""),
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
        let dest = match build_destination(None, &fields_json.to_string()) {
            Ok(d) => d,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return QString::from("");
            }
        };
        let Ok(client) = make_client() else {
            self.as_mut()
                .set_last_action_error(QString::from(&daemon_unavailable_text()));
            return QString::from("");
        };
        match client.add_destination(
            &dest,
            &mode.to_string(),
            &repo_pw.to_string(),
            &rest_pw.to_string(),
        ) {
            Ok(id) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                QString::from(&id)
            }
            Err(err) => {
                let msg = dbus_error_text(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                if KesticDbusError::from(err).code() == ServiceErrorCode::SecretStoreLocked {
                    refresh_state_now();
                    self.as_mut().apply_cached_state();
                }
                QString::from("")
            }
        }
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
        let dest = match build_destination(Some(&id.to_string()), &fields_json.to_string()) {
            Ok(d) => d,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_destination(
            &dest,
            &mode.to_string(),
            &repo_pw.to_string(),
            &rest_pw.to_string(),
            verify_connection,
        ) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = dbus_error_text(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_destination_password(
        mut self: Pin<&mut Self>,
        dest_id: &QString,
        new_pw: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_destination_password(&dest_id.to_string(), &new_pw.to_string()) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                self.as_mut()
                    .passive_notification(i18n("Saved password updated"));
                true
            }
            Err(err) => {
                let msg = dbus_error_text(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn remove_destination(mut self: Pin<&mut Self>, dest_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.remove_destination(&dest_id.to_string()) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = dbus_error_text(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn unlock_destination(mut self: Pin<&mut Self>, dest_id: &QString) -> i32 {
        let Ok(client) = make_client() else {
            self.as_mut()
                .set_last_action_error(QString::from(&daemon_unavailable_text()));
            return -1;
        };
        match client.unlock_destination(&dest_id.to_string()) {
            Ok(count) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                let msg = if count == 0 {
                    i18n("No stale locks found.").to_string()
                } else if count == 1 {
                    i18n("1 stale lock removed.").to_string()
                } else {
                    crate::state_cache::i18n("%1 stale locks removed.").subs(&[&count.to_string()])
                };
                self.as_mut().passive_notification(QString::from(&msg));
                i32::try_from(count).unwrap_or(0)
            }
            Err(err) => {
                let msg = dbus_error_text(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                -1
            }
        }
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
                    let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                    guard.snapshots = Some(dto);
                    guard.snapshots_error = (!err_msg.is_empty()).then_some(err_msg.clone());
                }
                self.as_mut().set_snapshots_loading(false);
                self.as_mut().set_snapshots_error(QString::from(&err_msg));
                self.as_mut().set_snapshots_has_previous(has_prev);
                self.as_mut().snapshots_updated();
                true
            }
            Err(err) => {
                {
                    let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                    guard.snapshots = None;
                    guard.snapshots_error = Some(dbus_error_text(&err));
                }
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
