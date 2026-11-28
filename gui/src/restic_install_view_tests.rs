// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `restic_install_view`.

use super::*;
use crate::i18n::tr;
use kestic_i18n::codes::{
    ResticInstallCode, ResticInstallOutcome, ResticInstallPhase, ServiceErrorCode,
};

fn running(phase: ResticInstallPhase) -> ResticInstallStatusDto {
    ResticInstallStatusDto {
        state: "running".to_string(),
        phase,
        version: Some("0.19.1".to_string()),
        bytes_done: 0,
        total_bytes: None,
        outcome: None,
        error_kind: None,
        error_detail: None,
    }
}

fn failed(code: ResticInstallCode, detail: &str) -> ResticInstallStatusDto {
    ResticInstallStatusDto {
        state: "error".to_string(),
        error_kind: Some(code),
        error_detail: Some(detail.to_string()),
        ..running(ResticInstallPhase::Verifying)
    }
}

/// AC-M5-06-4 (a), -5 (a), -6 (a), GUI part: a failed installation shows the text of its cause
/// and keeps the raw detail apart; nothing of a running installation stays.
#[test]
fn ac_m5_06_4_5_6_install_error_texts() {
    let signature = install_view(&failed(ResticInstallCode::SignatureInvalid, "BADSIG 1234"));
    // AC-M5-06-4 (a)
    assert_eq!(
        signature.error_text,
        "The signature of the restic release is not valid. Nothing was installed."
    );
    assert_eq!(signature.error_detail, "BADSIG 1234");
    assert_eq!(signature.state, "error");
    assert!(!signature.cancellable);
    assert_eq!(signature.result_text, "");

    let checksum = install_view(&failed(
        ResticInstallCode::ChecksumMismatch,
        "sha256 differs",
    ));
    // AC-M5-06-5 (a)
    assert_eq!(
        checksum.error_text,
        "The downloaded restic does not match its checksum. Nothing was installed."
    );
    assert_eq!(checksum.error_detail, "sha256 differs");

    let network = install_view(&failed(ResticInstallCode::Network, "connection refused"));
    // AC-M5-06-6 (a)
    assert_eq!(
        network.error_text,
        "Could not download restic. Check the internet connection and try again."
    );
    assert_eq!(network.error_detail, "connection refused");
}

/// A running installation names its step; only a download with a known size has a share, and
/// it can be stopped until the binary is replaced.
#[test]
fn install_view_running_phases_progress_and_cancel() {
    let fetching = install_view(&running(ResticInstallPhase::FetchingRelease));
    assert_eq!(
        fetching.phase_text,
        "Looking for the latest restic release…"
    );
    assert_eq!(fetching.progress, INDETERMINATE);
    assert!(fetching.cancellable);
    assert_eq!(fetching.error_text, "");

    let mut downloading = running(ResticInstallPhase::Downloading);
    downloading.bytes_done = 250;
    downloading.total_bytes = Some(1000);
    let view = install_view(&downloading);
    assert_eq!(view.phase_text, "Downloading restic…");
    assert!(
        (view.progress - 0.25).abs() < f64::EPSILON,
        "{}",
        view.progress
    );

    downloading.total_bytes = None;
    assert_eq!(install_view(&downloading).progress, INDETERMINATE);
    downloading.total_bytes = Some(0);
    assert_eq!(install_view(&downloading).progress, INDETERMINATE);
    downloading.bytes_done = 5000;
    downloading.total_bytes = Some(1000);
    assert_eq!(
        install_view(&downloading).progress,
        1.0,
        "never beyond the end"
    );

    let mut verifying = running(ResticInstallPhase::Verifying);
    verifying.bytes_done = 500;
    verifying.total_bytes = Some(1000);
    let view = install_view(&verifying);
    assert_eq!(
        view.progress, INDETERMINATE,
        "bytes of the download stay behind"
    );
    assert_eq!(
        view.phase_text,
        "Checking the signature and checksum of restic…"
    );

    let waiting = install_view(&running(ResticInstallPhase::WaitingForRuns));
    assert_eq!(
        waiting.phase_text,
        "Waiting until no run or preview uses restic any more…"
    );
    assert!(waiting.cancellable);

    let replacing = install_view(&running(ResticInstallPhase::Replacing));
    assert_eq!(replacing.phase_text, "Installing restic…");
    assert!(!replacing.cancellable, "the binary is being replaced");
}

/// A finished installation names the version; a cancelled one shows nothing.
#[test]
fn install_view_outcome_and_cancelled() {
    let done = ResticInstallStatusDto {
        state: "done".to_string(),
        outcome: Some(ResticInstallOutcome::Installed),
        ..running(ResticInstallPhase::Replacing)
    };
    let view = install_view(&done);
    assert_eq!(view.result_text, "restic 0.19.1 is installed.");
    assert_eq!(view.phase_text, "");
    assert!(!view.cancellable);

    let current = ResticInstallStatusDto {
        outcome: Some(ResticInstallOutcome::AlreadyCurrent),
        ..done
    };
    assert_eq!(
        install_view(&current).result_text,
        "restic 0.19.1 is already the latest version."
    );

    let cancelled = ResticInstallStatusDto {
        state: "cancelled".to_string(),
        ..running(ResticInstallPhase::Downloading)
    };
    let view = install_view(&cancelled);
    assert_eq!(view.state, "cancelled");
    assert_eq!(view.phase_text, "");
    assert_eq!(view.result_text, "");
    assert_eq!(view.error_text, "");
    assert!(!view.cancellable);
}

/// A status that cannot be read ends the view as failed with the reason, so the button is
/// offered again (daemon gone, or the installation unknown after a daemon restart).
#[test]
fn install_view_after_failed_poll() {
    let view = failed_poll_view("The background service is not running.");
    assert_eq!(view.state, "error");
    assert_eq!(view.error_text, "The background service is not running.");
    assert_eq!(view.error_detail, "");
    assert_eq!(view.phase_text, "");
    assert!(!view.cancellable);
}

/// AC-M5-06-12 (b): «Download restic» is offered whenever restic is unusable, «Update Now»
/// only while the restic in use is the valid one Kestic manages (chosen or through
/// «Automatic»); nothing can start without the daemon or while an installation runs.
#[test]
fn restic_offer_matrix() {
    let offer = |daemon_reachable, restic_valid, uses_app_managed, installing| {
        restic_offer(OfferInput {
            daemon_reachable,
            restic_valid,
            uses_app_managed,
            installing,
        })
    };
    let download = Offer {
        download: true,
        update: false,
        enabled: true,
    };
    let update = Offer {
        download: false,
        update: true,
        enabled: true,
    };
    let nothing = Offer {
        download: false,
        update: false,
        enabled: true,
    };
    // unusable restic, whatever the source: download
    assert_eq!(
        offer(true, false, false, false),
        download,
        "restic unusable"
    );
    assert_eq!(
        offer(true, false, true, false),
        download,
        "managed binary unusable"
    );
    // valid restic: update only for the managed one in use
    assert_eq!(offer(true, true, true, false), update);
    assert_eq!(offer(true, true, false, false), nothing);
    // disabled without the daemon or while installing
    assert_eq!(
        offer(false, false, false, false),
        Offer {
            enabled: false,
            ..download
        }
    );
    assert_eq!(
        offer(true, true, true, true),
        Offer {
            enabled: false,
            ..update
        }
    );
}

/// A second installation is refused with a text about the installation, not about a run on a
/// destination; other refusals keep the daemon's text.
#[test]
fn start_failure_names_the_running_installation() {
    assert_eq!(
        start_failure_text(ServiceErrorCode::AlreadyRunning),
        "A restic installation is already running."
    );
    assert_eq!(
        start_failure_text(ServiceErrorCode::NotFound),
        tr(&ServiceErrorCode::NotFound.message())
    );
}
