// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The restic section of the settings for [`super::qobject::KesticApp`]: its rows and offers,
//! changing the source, removing the restic managed by Kestic (`RemoveAppManagedRestic`) and
//! installing it (`StartResticInstall`, `GetResticInstallStatus`, `CancelResticInstall`).

use super::{dbus_error_text, qobject};
use crate::restic_install_view::{
    InstallView, OfferInput, failed_poll_view, install_view, restic_offer, start_failure_text,
};
use crate::restic_section_view::{ResticSectionView, needs_install_first, remove_failure_text};
use crate::state_cache::{daemon_unavailable_text, make_client, refresh_state_now};
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kestic_core::model::ResticProvider;
use kestic_daemon::dbus_api::KesticDbusError;
use kestic_i18n::codes::ServiceErrorCode;
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

    /// Publishes the rows and offers of the restic section.
    pub(super) fn show_restic_section(mut self: Pin<&mut Self>, view: &ResticSectionView) {
        self.as_mut()
            .set_restic_source_text(QString::from(&view.source));
        self.as_mut()
            .set_restic_version_text(QString::from(&view.version));
        self.as_mut()
            .set_restic_program_text(QString::from(&view.program));
        self.as_mut()
            .set_restic_available_text(QString::from(&view.available));
        self.as_mut()
            .set_restic_problem_text(QString::from(&view.problem));
        self.as_mut()
            .set_restic_problem_detail(QString::from(&view.problem_detail));
        self.as_mut()
            .set_restic_explain_minimum(view.explain_minimum);
        self.as_mut().set_restic_offer_switch(view.offer_switch);
        self.as_mut().set_restic_offer_remove(view.offer_remove);
        self.as_mut()
            .set_restic_uses_app_managed(view.uses_app_managed);
        self.as_mut()
            .set_restic_app_managed_installed(view.app_managed_installed);
    }

    /// Recomputes which restic action the settings and the blocking bar offer.
    pub(super) fn update_restic_offer(mut self: Pin<&mut Self>) {
        let offer = restic_offer(OfferInput {
            daemon_reachable: *self.daemon_reachable(),
            restic_valid: *self.restic_valid(),
            uses_app_managed: *self.restic_uses_app_managed(),
            installing: self.restic_install_state().to_string() == "running",
        });
        self.as_mut().set_restic_offer_download(offer.download);
        self.as_mut().set_restic_offer_update(offer.update);
        self.as_mut().set_restic_offer_enabled(offer.enabled);
    }

    /// Applies the source `kind` of the source dialog. «Managed by Kestic» that is not installed
    /// yet is downloaded first; the switch follows when exactly this installation succeeded.
    pub fn apply_restic_source(
        mut self: Pin<&mut Self>,
        kind: &QString,
        custom_path: &QString,
    ) -> bool {
        let kind = kind.to_string();
        if !needs_install_first(&kind, *self.restic_app_managed_installed()) {
            return self.set_restic_provider(&QString::from(&kind), custom_path);
        }
        if self.restic_install_state().to_string() == "running" {
            self.as_mut()
                .show_action_error(&start_failure_text(ServiceErrorCode::AlreadyRunning));
            return false;
        }
        if !self.as_mut().start_restic_install() {
            return false;
        }
        let install_id = self.restic_install_id.clone();
        self.as_mut()
            .rust_mut()
            .restic_switch_after_install
            .start(&install_id);
        true
    }

    /// Switches to the source «Automatic» from the hint of the restic section.
    pub fn switch_restic_to_automatic(mut self: Pin<&mut Self>) -> bool {
        self.as_mut()
            .rust_mut()
            .restic_switch_after_install
            .cancel();
        self.switch_restic_source(&ResticProvider::Automatic)
    }

    /// Switches the source to `provider` outside the source dialog; a failure shows in the
    /// restic section.
    fn switch_restic_source(mut self: Pin<&mut Self>, provider: &ResticProvider) -> bool {
        let result = make_client()
            .map_err(|_| daemon_unavailable_text())
            .and_then(|client| {
                client
                    .set_restic_provider(provider)
                    .map_err(|err| dbus_error_text(&err))
            });
        let error = result.as_ref().err().cloned().unwrap_or_default();
        self.as_mut().set_restic_action_error(QString::from(&error));
        refresh_state_now();
        self.as_mut().apply_cached_state();
        result.is_ok()
    }

    /// Deletes the restic managed by Kestic; a refusal shows why and keeps it.
    pub fn remove_app_managed_restic(mut self: Pin<&mut Self>) -> bool {
        let result = make_client()
            .map_err(|_| daemon_unavailable_text())
            .and_then(|client| {
                client
                    .remove_app_managed_restic()
                    .map_err(|err| remove_failure_text(KesticDbusError::from(err).code()))
            });
        let error = result.as_ref().err().cloned().unwrap_or_default();
        self.as_mut().set_restic_action_error(QString::from(&error));
        refresh_state_now();
        self.as_mut().apply_cached_state();
        result.is_ok()
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
            let switch = self
                .as_mut()
                .rust_mut()
                .restic_switch_after_install
                .finish(&install_id, &view.state);
            if switch {
                self.as_mut()
                    .switch_restic_source(&ResticProvider::AppManaged);
            }
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
