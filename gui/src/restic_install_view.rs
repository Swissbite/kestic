// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! What the restic section of the settings shows for an installation of the restic managed by
//! Kestic (`ResticInstallStatusDto`).

use crate::i18n::{i18n, tr};
use kestic_daemon::dbus_api::ResticInstallStatusDto;
use kestic_i18n::codes::{ResticInstallPhase, ServiceErrorCode};

/// Shown value of the progress bar while its end is unknown.
pub const INDETERMINATE: f64 = -1.0;

/// Texts and controls of one installation, in the GUI language.
#[derive(Debug, Clone, PartialEq)]
pub struct InstallView {
    /// `"running"`, `"done"`, `"cancelled"` or `"error"`.
    pub state: String,
    /// The step of a running installation.
    pub phase_text: String,
    /// Share of the file being downloaded (0 to 1), or [`INDETERMINATE`].
    pub progress: f64,
    /// The installation can still be stopped (it does not replace the binary yet).
    pub cancellable: bool,
    /// Result of a finished installation.
    pub result_text: String,
    /// Why the installation failed.
    pub error_text: String,
    /// Raw error text (English), shown under «Details».
    pub error_detail: String,
}

/// The view of `status`.
pub fn install_view(status: &ResticInstallStatusDto) -> InstallView {
    let mut view = InstallView {
        state: status.state.clone(),
        phase_text: String::new(),
        progress: INDETERMINATE,
        cancellable: false,
        result_text: String::new(),
        error_text: String::new(),
        error_detail: String::new(),
    };
    match status.state.as_str() {
        "running" => {
            view.phase_text = tr(&status.phase.message());
            view.progress = download_share(status);
            view.cancellable = status.phase != ResticInstallPhase::Replacing;
        }
        "done" => {
            if let Some(outcome) = status.outcome {
                let version = status.version.as_deref().unwrap_or_default();
                view.result_text = tr(&outcome.message(version));
            }
        }
        "error" => {
            view.error_detail = status.error_detail.clone().unwrap_or_default();
            if let Some(code) = status.error_kind {
                view.error_text = tr(&code.message());
            }
        }
        _ => {}
    }
    view
}

/// Share of the file being downloaded; only while downloading and when its size is known.
fn download_share(status: &ResticInstallStatusDto) -> f64 {
    match status.total_bytes {
        Some(total) if total > 0 && status.phase == ResticInstallPhase::Downloading => {
            (status.bytes_done as f64 / total as f64).min(1.0)
        }
        _ => INDETERMINATE,
    }
}

/// The view after the status could not be read (`text`: why, in the GUI language): the
/// installation counts as failed, so it can be started again.
pub fn failed_poll_view(text: &str) -> InstallView {
    InstallView {
        state: "error".to_string(),
        phase_text: String::new(),
        progress: INDETERMINATE,
        cancellable: false,
        result_text: String::new(),
        error_text: text.to_string(),
        error_detail: String::new(),
    }
}

/// Why `StartResticInstall` failed: the daemon's text for `code`, except for an installation
/// that already runs (its general text names a destination).
pub fn start_failure_text(code: ServiceErrorCode) -> String {
    match code {
        ServiceErrorCode::AlreadyRunning => {
            i18n("A restic installation is already running.").to_string()
        }
        other => tr(&other.message()),
    }
}

/// What decides which restic action the settings and the blocking bar offer.
#[derive(Debug, Clone, Copy)]
pub struct OfferInput {
    pub daemon_reachable: bool,
    /// The restic in use runs and is new enough.
    pub restic_valid: bool,
    /// The restic in use is the one Kestic manages (chosen or through «Automatic»).
    pub uses_app_managed: bool,
    /// An installation runs.
    pub installing: bool,
}

/// The restic actions on offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer {
    /// «Download restic»: restic is missing or unusable.
    pub download: bool,
    /// «Update Now»: the restic in use is the one Kestic manages.
    pub update: bool,
    /// The offered action can be started now.
    pub enabled: bool,
}

/// The restic actions for `input`.
pub fn restic_offer(input: OfferInput) -> Offer {
    Offer {
        download: !input.restic_valid,
        update: input.restic_valid && input.uses_app_managed,
        enabled: input.daemon_reachable && !input.installing,
    }
}

#[cfg(test)]
#[path = "restic_install_view_tests.rs"]
mod tests;
