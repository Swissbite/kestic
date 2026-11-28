// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `restic_section_view`.

use super::*;
use kestic_daemon::dbus_api::ResticBinaryDto;
use kestic_i18n::codes::{ResticPathProblem, ResticSourceCode, ResticStatusCode};

const SYSTEM: &str = "/usr/bin/restic";
const APP: &str = "/home/u/.local/share/kestic/restic";

fn binary(path: &str, version: &str, too_old: bool) -> Option<ResticBinaryDto> {
    Some(ResticBinaryDto {
        path: path.to_string(),
        version: Some(version.to_string()),
        too_old,
    })
}

/// A usable restic `version` at `path` from the source `source`.
fn usable(source: ResticSourceCode, path: &str, version: &str) -> ResticStatusDto {
    ResticStatusDto {
        valid: true,
        binary_path: Some(path.to_string()),
        version: Some(version.to_string()),
        error_kind: None,
        found_version: Some(version.to_string()),
        detail: None,
        path_problem: None,
        source,
        system: None,
        app_managed: None,
        offer_switch_to_system: false,
        install_id: None,
    }
}

/// The system restic 0.16.4 under «Automatic» without an app-managed restic.
fn system_too_old() -> ResticStatusDto {
    ResticStatusDto {
        valid: false,
        version: None,
        error_kind: Some(ResticStatusCode::TooOld),
        found_version: Some("0.16.4".to_string()),
        system: binary(SYSTEM, "0.16.4", true),
        ..usable(ResticSourceCode::Automatic, SYSTEM, "0.16.4")
    }
}

/// AC-M5-07-3: «Automatic», system restic 0.16.4, no app-managed restic: (a) the version row
/// names the version as too old and the section explains why 0.18.0 is needed; (b) nothing
/// else claims the problem, the download is offered by the section (`restic_offer`).
#[test]
fn ac_m5_07_3_explains_minimum_and_offers_download() {
    let view = restic_section_view(&system_too_old());
    // (a)
    assert!(view.explain_minimum, "{view:?}");
    assert_eq!(view.version, "0.16.4 · too old, at least 0.18.0");
    assert_eq!(view.source, "Automatic (System)");
    assert_eq!(view.program, SYSTEM);
    assert_eq!(view.problem, "", "the version row already says it");
    // (b)
    let offer = crate::restic_install_view::restic_offer(crate::restic_install_view::OfferInput {
        daemon_reachable: true,
        restic_valid: false,
        uses_app_managed: view.uses_app_managed,
        installing: false,
    });
    assert!(offer.download && offer.enabled, "{offer:?}");
    assert!(!view.offer_switch);
    assert!(!view.offer_remove);

    // A usable restic and no too old one: no explanation.
    let fine = ResticStatusDto {
        system: binary(SYSTEM, "0.20.0", false),
        ..usable(ResticSourceCode::Automatic, SYSTEM, "0.20.0")
    };
    assert!(!restic_section_view(&fine).explain_minimum);
    // A too old system restic beside the app-managed one in use is explained as well.
    let fallback = ResticStatusDto {
        system: binary(SYSTEM, "0.16.4", true),
        app_managed: binary(APP, "0.20.0", false),
        ..usable(ResticSourceCode::Automatic, APP, "0.20.0")
    };
    assert!(restic_section_view(&fallback).explain_minimum);
}

/// AC-M5-07-4: system 0.18.1 and app-managed 0.19.1: (a) both versions in the row
/// «Available:»; (b) source, version in use and program each in a row of their own; (c) a
/// supported version below the tested current one gets «recommended: 0.20.0».
#[test]
fn ac_m5_07_4_rows_and_both_versions() {
    let both = ResticStatusDto {
        system: binary(SYSTEM, "0.18.1", false),
        app_managed: binary(APP, "0.19.1", false),
        ..usable(ResticSourceCode::Automatic, SYSTEM, "0.18.1")
    };
    let view = restic_section_view(&both);
    // (a)
    assert_eq!(view.available, "System 0.18.1 · Managed by Kestic 0.19.1");
    // (b)
    assert_eq!(view.source, "Automatic (System)");
    assert_eq!(view.version, "0.18.1 · recommended: 0.20.0");
    assert_eq!(view.program, SYSTEM);
    assert_eq!(view.problem, "");
    assert!(!view.uses_app_managed);
    assert!(view.app_managed_installed);
    // (c) the tested current version and newer ones get no note
    let current = ResticStatusDto {
        system: binary(SYSTEM, "0.20.0", false),
        ..usable(ResticSourceCode::Automatic, SYSTEM, "0.20.0")
    };
    let view = restic_section_view(&current);
    assert_eq!(view.version, "0.20.0");
    assert_eq!(view.available, "", "one binary: no row «Available:»");
    assert!(!view.app_managed_installed);

    // «Automatic» on the app-managed restic names it as the one in use.
    let fallback = ResticStatusDto {
        system: binary(SYSTEM, "0.16.4", true),
        app_managed: binary(APP, "0.20.0", false),
        ..usable(ResticSourceCode::Automatic, APP, "0.20.0")
    };
    let view = restic_section_view(&fallback);
    assert_eq!(view.source, "Automatic (Managed by Kestic)");
    assert_eq!(view.available, "System 0.16.4 · Managed by Kestic 0.20.0");
    assert!(view.uses_app_managed);
    // The chosen sources without «Automatic» are named like in the source dialog.
    for (source, path, label) in [
        (ResticSourceCode::System, SYSTEM, "System"),
        (ResticSourceCode::AppManaged, APP, "Managed by Kestic"),
        (ResticSourceCode::Custom, "/opt/restic", "Custom path"),
    ] {
        assert_eq!(
            restic_section_view(&usable(source, path, "0.20.0")).source,
            label
        );
    }
    // Neither binary found: «Automatic» alone, the reason instead of version and program.
    let missing = ResticStatusDto {
        valid: false,
        binary_path: None,
        version: None,
        found_version: None,
        error_kind: Some(ResticStatusCode::NotFound),
        ..usable(ResticSourceCode::Automatic, SYSTEM, "0.20.0")
    };
    let view = restic_section_view(&missing);
    assert_eq!(view.source, "Automatic");
    assert_eq!(view.version, "");
    assert_eq!(view.program, "");
    assert_eq!(view.problem, "restic not found; at least 0.18.0 required");
}

/// AC-M5-07-5: (a) the hint to switch follows the daemon's offer; (c), (d) with «Automatic»
/// on the system restic and the app-managed one present, only the removal is offered.
#[test]
fn ac_m5_07_5_switch_hint_and_remove_offer() {
    // (a) «Managed by Kestic» and a system restic that is recent enough now
    let chosen = ResticStatusDto {
        system: binary(SYSTEM, "0.18.1", false),
        app_managed: binary(APP, "0.19.1", false),
        offer_switch_to_system: true,
        ..usable(ResticSourceCode::AppManaged, APP, "0.19.1")
    };
    let view = restic_section_view(&chosen);
    assert!(view.offer_switch);
    assert!(!view.offer_remove, "the app-managed restic is in use");
    assert!(view.uses_app_managed);
    // (c), (d) after the switch to «Automatic»
    let automatic = ResticStatusDto {
        offer_switch_to_system: false,
        ..usable(ResticSourceCode::Automatic, SYSTEM, "0.18.1")
    };
    let automatic = ResticStatusDto {
        system: chosen.system.clone(),
        app_managed: chosen.app_managed,
        ..automatic
    };
    let view = restic_section_view(&automatic);
    assert!(!view.offer_switch, "(d) no question");
    assert!(view.offer_remove, "(c) removal offered");
    assert!(!view.uses_app_managed);
    // No removal while an installation of it runs.
    let installing = ResticStatusDto {
        install_id: Some("install-1".to_string()),
        ..automatic.clone()
    };
    assert!(!restic_section_view(&installing).offer_remove);
    // No removal without the app-managed restic, while «Automatic» falls back to it, or with
    // another chosen source.
    let without = ResticStatusDto {
        app_managed: None,
        ..automatic.clone()
    };
    assert!(!restic_section_view(&without).offer_remove);
    let fallback = ResticStatusDto {
        system: binary(SYSTEM, "0.16.4", true),
        app_managed: binary(APP, "0.19.1", false),
        ..usable(ResticSourceCode::Automatic, APP, "0.19.1")
    };
    assert!(!restic_section_view(&fallback).offer_remove);
    let system = ResticStatusDto {
        source: ResticSourceCode::System,
        ..automatic
    };
    assert!(!restic_section_view(&system).offer_remove);
}

/// An invalid custom path: the translated reason, and the raw reason under «Details»; for
/// `other` the reason is the raw text itself.
#[test]
fn invalid_custom_path_shows_reason_and_details() {
    let invalid = |problem| ResticStatusDto {
        valid: false,
        version: None,
        found_version: None,
        error_kind: Some(ResticStatusCode::InvalidCustomPath),
        detail: Some("No such file or directory (os error 2)".to_string()),
        path_problem: Some(problem),
        ..usable(ResticSourceCode::Custom, "/opt/restic", "0.20.0")
    };
    let view = restic_section_view(&invalid(ResticPathProblem::NotFound));
    assert_eq!(
        view.problem,
        "Custom restic path is invalid: /opt/restic does not exist"
    );
    assert_eq!(
        view.problem_detail,
        "No such file or directory (os error 2)"
    );
    assert_eq!(view.version, "");
    assert_eq!(view.program, "", "the path is in the reason");
    assert_eq!(view.source, "Custom path");

    let other = restic_section_view(&invalid(ResticPathProblem::Other));
    assert_eq!(
        other.problem,
        "Custom restic path is invalid (/opt/restic): No such file or directory (os error 2)"
    );
    assert_eq!(
        other.problem_detail, "",
        "the reason already holds the raw text"
    );
}

/// Applying «Managed by Kestic» downloads it first only while it is not installed.
#[test]
fn applying_app_managed_downloads_it_first_when_missing() {
    assert!(needs_install_first("app_managed", false));
    assert!(!needs_install_first("app_managed", true));
    for kind in ["automatic", "system", "custom"] {
        assert!(!needs_install_first(kind, false), "{kind}");
    }
}

/// AC-M5-07-5: only the installation that the source dialog started for «Managed by Kestic»
/// switches the source, only when it succeeded, and not after another source was chosen.
#[test]
fn switch_after_install_only_for_its_own_installation() {
    let started = || {
        let mut pending = PendingSwitch::default();
        pending.start("install-1");
        pending
    };
    assert!(started().finish("install-1", "done"));
    // another installation (Download restic, Update Now) never switches
    assert!(!PendingSwitch::default().finish("install-2", "done"));
    let mut other = started();
    assert!(!other.finish("install-2", "done"));
    assert!(other.finish("install-1", "done"), "still waits for its own");
    // a failed or cancelled installation does not switch, and ends the wait
    let mut failed = started();
    assert!(!failed.finish("install-1", "error"));
    assert!(!failed.finish("install-1", "done"));
    assert!(!started().finish("install-1", "cancelled"));
    // a source chosen while it downloads wins
    let mut overridden = started();
    overridden.cancel();
    assert!(!overridden.finish("install-1", "done"));
}

/// AC-M5-07-5: a refused removal of the app-managed restic names it as in use; other refusals
/// keep the daemon's text.
#[test]
fn remove_failure_texts() {
    assert_eq!(
        remove_failure_text(ServiceErrorCode::InUse),
        "The restic managed by Kestic is still in use and cannot be deleted now."
    );
    assert_eq!(
        remove_failure_text(ServiceErrorCode::Internal),
        crate::i18n::tr(&ServiceErrorCode::Internal.message())
    );
}

/// AC-M5-07-4: paths are shown as they are, also with markup or placeholders in them.
#[test]
fn paths_are_shown_literally() {
    let path = "/tmp/<b>x</b>%2/restic";
    let view = restic_section_view(&ResticStatusDto {
        system: binary(path, "0.20.0", false),
        ..usable(ResticSourceCode::System, path, "0.20.0")
    });
    assert_eq!(view.program, path);
    let invalid = restic_section_view(&ResticStatusDto {
        valid: false,
        version: None,
        error_kind: Some(ResticStatusCode::InvalidCustomPath),
        path_problem: Some(ResticPathProblem::NotExecutable),
        detail: Some("Permission denied (os error 13)".to_string()),
        ..usable(ResticSourceCode::Custom, path, "0.20.0")
    });
    assert_eq!(
        invalid.problem,
        format!("Custom restic path is invalid: {path} cannot be started")
    );
}

/// AC-M5-07-5: a system restic that is only a link to the app-managed one is that one: it
/// counts as the app-managed restic in use, and no removal is offered.
#[test]
fn ac_m5_07_5_no_remove_offer_for_a_linked_system_restic() {
    let dir = std::env::temp_dir().join(format!("kestic-gui-linked-restic-{}", std::process::id()));
    std::fs::create_dir(&dir).expect("fresh temp dir");
    let app = dir.join("restic");
    std::fs::write(&app, "#!/bin/sh\n").expect("app-managed stand-in");
    let link = dir.join("system-restic");
    std::os::unix::fs::symlink(&app, &link).expect("link");
    let (app, link) = (app.to_string_lossy(), link.to_string_lossy());
    let view = restic_section_view(&ResticStatusDto {
        system: binary(&link, "0.20.0", false),
        app_managed: binary(&app, "0.20.0", false),
        ..usable(ResticSourceCode::Automatic, &link, "0.20.0")
    });
    std::fs::remove_dir_all(&dir).expect("clean up");
    assert!(!view.offer_remove, "{view:?}");
    assert!(view.uses_app_managed);
}
