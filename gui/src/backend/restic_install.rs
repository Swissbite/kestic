// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The restic section of the settings for [`super::qobject::KesticApp`]: its rows and offers,
//! changing the source, removing the restic managed by Kestic (`RemoveAppManagedRestic`) and
//! installing it (`StartResticInstall`, `GetResticInstallStatus`, `CancelResticInstall`).

use super::daemon_call::{Lane, send_to_daemon};
use super::{dbus_error_text, qobject};
use crate::restic_install_view::{
    InstallView, OfferInput, failed_poll_view, install_view, restic_offer, start_failure_text,
};
use crate::restic_section_view::{ResticSectionView, needs_install_first, remove_failure_text};
use crate::state_cache::daemon_unavailable_text;
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kestic_core::model::ResticProvider;
use kestic_daemon::dbus_api::{KesticClient, KesticDbusError};
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
        self.as_mut().rust_mut().restic_uses_app_managed = view.uses_app_managed;
        self.as_mut()
            .set_restic_app_managed_installed(view.app_managed_installed);
    }

    /// Recomputes which restic action the settings and the blocking bar offer.
    pub(super) fn update_restic_offer(mut self: Pin<&mut Self>) {
        let offer = restic_offer(OfferInput {
            daemon_reachable: *self.daemon_reachable(),
            restic_valid: *self.restic_valid(),
            uses_app_managed: self.restic_uses_app_managed,
            installing: self.installing_restic(),
        });
        self.as_mut().set_restic_offer_download(offer.download);
        self.as_mut().set_restic_offer_update(offer.update);
        self.as_mut().set_restic_offer_enabled(offer.enabled);
    }

    /// Whether an installation of restic runs (the `state` of its status is `running`) or was
    /// asked for.
    fn installing_restic(&self) -> bool {
        self.restic_install_starting || self.restic_install_state().to_string() == "running"
    }

    /// The oldest restic Kestic runs, for the texts of the settings.
    pub fn restic_minimum_version(&self) -> QString {
        QString::from(&kestic_core::provider::MIN_RESTIC_VERSION.to_string())
    }

    /// Applies the source `kind` of the source dialog. «Managed by Kestic» that is not installed
    /// yet is downloaded first; the switch follows when exactly this installation succeeded. The
    /// request succeeds when the daemon took the source or the download started.
    pub fn apply_restic_source(
        mut self: Pin<&mut Self>,
        kind: &QString,
        custom_path: &QString,
    ) -> i32 {
        let kind = kind.to_string();
        if !needs_install_first(&kind, *self.restic_app_managed_installed()) {
            return self.set_restic_provider(&QString::from(&kind), custom_path);
        }
        if self.installing_restic() {
            self.as_mut()
                .show_action_error(&start_failure_text(ServiceErrorCode::AlreadyRunning));
            return 0;
        }
        self.as_mut().rust_mut().restic_install_starting = true;
        self.request_daemon_with(
            Lane::slow("restic-install"),
            request_restic_install,
            |_| false,
            |mut app, started| {
                app.as_mut().rust_mut().restic_install_starting = false;
                match started.unwrap_or_else(|_| Err(daemon_unavailable_text())) {
                    Ok(install_id) => {
                        app.as_mut()
                            .rust_mut()
                            .restic_switch_after_install
                            .start(&install_id);
                        app.follow_restic_install(install_id);
                        (true, String::new())
                    }
                    Err(text) => {
                        app.as_mut().show_action_error(&text);
                        (false, String::new())
                    }
                }
            },
        )
    }

    /// Switches to the source «Automatic» from the hint of the restic section.
    pub fn switch_restic_to_automatic(mut self: Pin<&mut Self>) {
        self.as_mut()
            .rust_mut()
            .restic_switch_after_install
            .cancel();
        self.switch_restic_source(ResticProvider::Automatic);
    }

    /// Switches the source to `provider` outside the source dialog; a failure shows in the
    /// restic section.
    fn switch_restic_source(self: Pin<&mut Self>, provider: ResticProvider) {
        self.call_daemon_with(
            Lane::slow("restic-source"),
            move |client| {
                client
                    .set_restic_provider(&provider)
                    .map(drop)
                    .map_err(|err| dbus_error_text(&err))
            },
            |_| true,
            |app, result| app.show_restic_action(result),
        );
    }

    /// Deletes the restic managed by Kestic; a refusal shows why and keeps it.
    pub fn remove_app_managed_restic(self: Pin<&mut Self>) {
        self.call_daemon_with(
            Lane::slow("restic-source"),
            |client| {
                client
                    .remove_app_managed_restic()
                    .map_err(|err| remove_failure_text(KesticDbusError::from(err).code()))
            },
            |_| true,
            |app, result| app.show_restic_action(result),
        );
    }

    /// Shows how a change of the restic in use ended, with the state fetched after it.
    fn show_restic_action(mut self: Pin<&mut Self>, result: Result<Result<(), String>, String>) {
        let error = match result {
            Ok(Ok(())) => String::new(),
            Ok(Err(text)) => text,
            Err(_) => daemon_unavailable_text(),
        };
        self.as_mut().set_restic_action_error(QString::from(&error));
        self.apply_cached_state();
    }

    pub fn start_restic_install(mut self: Pin<&mut Self>) {
        if self.installing_restic() {
            return;
        }
        self.as_mut().rust_mut().restic_install_starting = true;
        self.call_daemon(
            Lane::slow("restic-install"),
            request_restic_install,
            |mut app, started| {
                app.as_mut().rust_mut().restic_install_starting = false;
                match started.unwrap_or_else(|_| Err(daemon_unavailable_text())) {
                    Ok(install_id) => app.follow_restic_install(install_id),
                    Err(text) => app.show_install(&failed_poll_view(&text)),
                }
            },
        );
    }

    /// Follows the installation `install_id` that the daemon started.
    fn follow_restic_install(mut self: Pin<&mut Self>, install_id: String) {
        self.as_mut().rust_mut().restic_install_id = install_id;
        self.poll_restic_install();
    }

    /// Asks for the status of the installation, unless an answer is still on its way.
    pub fn poll_restic_install(mut self: Pin<&mut Self>) {
        let install_id = self.restic_install_id.clone();
        if install_id.is_empty() || self.restic_install_polling {
            return;
        }
        self.as_mut().rust_mut().restic_install_polling = true;
        let asked = install_id.clone();
        self.call_daemon_with(
            Lane::slow("restic-install"),
            move |client| {
                client
                    .get_restic_install_status(&asked)
                    .map_err(|err| dbus_error_text(&err))
            },
            // The restic in use may have changed once the installation ended.
            |status| {
                status
                    .as_ref()
                    .map_or(true, |s| install_view(s).state != "running")
            },
            move |mut app, status| {
                app.as_mut().rust_mut().restic_install_polling = false;
                let view = match status.unwrap_or_else(|_| Err(daemon_unavailable_text())) {
                    Ok(status) => install_view(&status),
                    Err(text) => failed_poll_view(&text),
                };
                app.show_install_status(&install_id, &view);
            },
        );
    }

    /// Shows the status `view` of the installation `install_id`; an ended one also switches the
    /// source when the source dialog asked for it.
    fn show_install_status(mut self: Pin<&mut Self>, install_id: &str, view: &InstallView) {
        let finished = view.state != "running";
        if finished {
            self.as_mut().rust_mut().restic_install_id.clear();
            let switch = self
                .as_mut()
                .rust_mut()
                .restic_switch_after_install
                .finish(install_id, &view.state);
            if switch {
                self.as_mut()
                    .switch_restic_source(ResticProvider::AppManaged);
            }
        }
        self.as_mut().show_install(view);
        if finished {
            self.as_mut().apply_cached_state();
        }
    }

    /// Asks the daemon to stop the installation; what happened shows the next poll.
    pub fn cancel_restic_install(self: Pin<&mut Self>) {
        let install_id = self.restic_install_id.clone();
        if install_id.is_empty() {
            return;
        }
        // On the lane of the polls, so the next poll shows the cancel.
        send_to_daemon(Lane::slow("restic-install"), move |client| {
            // A cancel that fails shows in the poll: the installation has ended or is unknown.
            if let Err(err) = client.cancel_restic_install(&install_id) {
                eprintln!("kestic: cancelling the restic installation failed: {err}");
            }
        });
        self.poll_restic_install();
    }
}

/// Asks the daemon to install the restic managed by Kestic; the id of the installation, or why
/// it did not start.
fn request_restic_install(client: &KesticClient) -> Result<String, String> {
    client
        .start_restic_install()
        .map_err(|err| start_failure_text(KesticDbusError::from(err).code()))
}
