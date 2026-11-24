// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Installation of the restic managed by Kestic from the settings (`StartResticInstall`,
//! `GetResticInstallStatus`, `CancelResticInstall`) for [`super::qobject::KesticApp`].

use super::{dbus_error_text, qobject};
use crate::restic_install_view::{
    InstallView, OfferInput, failed_poll_view, install_view, restic_offer, start_failure_text,
};
use crate::state_cache::{daemon_unavailable_text, make_client, refresh_state_now};
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kestic_daemon::dbus_api::KesticDbusError;
use std::pin::Pin;

impl qobject::KesticApp {
    /// Publishes `view` and the restic actions on offer with it.
    fn show_install(mut self: Pin<&mut Self>, view: &InstallView) {
        self.as_mut()
            .set_restic_install_state(QString::from(&view.state));
        self.as_mut()
            .set_restic_install_phase_text(QString::from(&view.phase_text));
        self.as_mut().set_restic_install_progress(view.progress);
        self.as_mut()
            .set_restic_install_cancellable(view.cancellable);
        self.as_mut()
            .set_restic_install_result(QString::from(&view.result_text));
        self.as_mut()
            .set_restic_install_error(QString::from(&view.error_text));
        self.as_mut()
            .set_restic_install_error_detail(QString::from(&view.error_detail));
        self.update_restic_offer();
    }

    /// Recomputes which restic action the settings and the blocking bar offer.
    pub(super) fn update_restic_offer(mut self: Pin<&mut Self>) {
        let source_kind = self.restic_provider_kind().to_string();
        let offer = restic_offer(OfferInput {
            daemon_reachable: *self.daemon_reachable(),
            restic_valid: *self.restic_valid(),
            source_kind: &source_kind,
            installing: self.restic_install_state().to_string() == "running",
        });
        self.as_mut().set_restic_offer_download(offer.download);
        self.as_mut().set_restic_offer_update(offer.update);
        self.as_mut().set_restic_offer_enabled(offer.enabled);
    }

    pub fn start_restic_install(mut self: Pin<&mut Self>) -> bool {
        if self.restic_install_state().to_string() == "running" {
            return false;
        }
        let started = make_client()
            .map_err(|_| daemon_unavailable_text())
            .and_then(|client| {
                client
                    .start_restic_install()
                    .map_err(|err| start_failure_text(KesticDbusError::from(err).code()))
            });
        match started {
            Ok(install_id) => {
                self.as_mut().rust_mut().restic_install_id = install_id;
                self.as_mut().poll_restic_install();
                true
            }
            Err(text) => {
                self.show_install(&failed_poll_view(&text));
                false
            }
        }
    }

    pub fn poll_restic_install(mut self: Pin<&mut Self>) {
        let install_id = self.restic_install_id.clone();
        if install_id.is_empty() {
            return;
        }
        let status = make_client()
            .map_err(|_| daemon_unavailable_text())
            .and_then(|client| {
                client
                    .get_restic_install_status(&install_id)
                    .map_err(|err| dbus_error_text(&err))
            });
        let view = match status {
            Ok(status) => install_view(&status),
            Err(text) => failed_poll_view(&text),
        };
        let finished = view.state != "running";
        if finished {
            self.as_mut().rust_mut().restic_install_id.clear();
        }
        self.as_mut().show_install(&view);
        if finished {
            // The restic in use may have changed.
            refresh_state_now();
            self.as_mut().apply_cached_state();
        }
    }

    /// Asks the daemon to stop the installation; what happened shows the next poll.
    pub fn cancel_restic_install(self: Pin<&mut Self>) {
        let install_id = self.restic_install_id.clone();
        if install_id.is_empty() {
            return;
        }
        // A cancel that fails shows in the poll: the installation has ended or is unknown.
        if let Ok(client) = make_client()
            && let Err(err) = client.cancel_restic_install(&install_id)
        {
            eprintln!("kestic: cancelling the restic installation failed: {err}");
        }
        self.poll_restic_install();
    }
}
