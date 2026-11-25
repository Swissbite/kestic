// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! What the restic section of the settings shows about the restic source and the programs
//! Kestic found (`ResticStatusDto`).

use crate::state_cache::{i18n, tr};
use crate::ui_texts::GuiText;
use kestic_core::provider::{
    MIN_RESTIC_VERSION, TESTED_RESTIC_VERSION, is_outdated_text, same_binary,
};
use kestic_daemon::dbus_api::{ResticBinaryDto, ResticStatusDto};
use kestic_i18n::codes::{ResticPathProblem, ResticSourceCode, ResticStatusCode, ServiceErrorCode};
use std::path::Path;

/// Rows, messages and offers of the restic section, in the GUI language.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResticSectionView {
    /// «Source:» the chosen source; with «Automatic» also the one in use.
    pub source: String,
    /// «Version:» the version in use, or the too old one found, with its note; empty when
    /// there is none.
    pub version: String,
    /// «Program:» the path of the program in use; empty when there is none.
    pub program: String,
    /// «Available:» both versions when the system and the app-managed restic exist, else empty.
    pub available: String,
    /// Why restic cannot be used, when the version row does not say it; empty otherwise.
    pub problem: String,
    /// Raw reason of `problem`, shown under «Details»; empty when `problem` already holds it.
    pub problem_detail: String,
    /// A version is too old: the section explains why 0.18.0 is needed.
    pub explain_minimum: bool,
    /// The system restic is now recent enough to switch away from the app-managed one.
    pub offer_switch: bool,
    /// «Automatic» runs the system restic and the app-managed one is no longer needed.
    pub offer_remove: bool,
    /// The restic in use is the app-managed one.
    pub uses_app_managed: bool,
    /// The app-managed restic is installed.
    pub app_managed_installed: bool,
}

/// The restic section for `status`.
pub fn restic_section_view(status: &ResticStatusDto) -> ResticSectionView {
    let app_path = status.app_managed.as_ref().map(|app| app.path.as_str());
    // The binary in use, or the too old or unreadable one that would be used.
    let shown_binary = match status.error_kind {
        None | Some(ResticStatusCode::TooOld | ResticStatusCode::UnreadableVersion) => {
            status.binary_path.as_deref()
        }
        Some(ResticStatusCode::NotFound | ResticStatusCode::InvalidCustomPath) => None,
    };
    let is_app_managed =
        |path: &str| app_path.is_some_and(|app| same_binary(Path::new(path), Path::new(app)));
    let uses_app_managed = status.valid && shown_binary.is_some_and(is_app_managed);
    let (problem, problem_detail) = problem_texts(status);
    let any_too_old = status.error_kind == Some(ResticStatusCode::TooOld)
        || [&status.system, &status.app_managed]
            .into_iter()
            .flatten()
            .any(|binary| binary.too_old);
    ResticSectionView {
        source: source_text(status.source, shown_binary.map(is_app_managed)),
        version: version_text(status),
        program: shown_binary.unwrap_or_default().to_string(),
        available: available_text(status),
        problem,
        problem_detail,
        explain_minimum: any_too_old,
        offer_switch: status.offer_switch_to_system,
        offer_remove: status.source == ResticSourceCode::Automatic
            && status.valid
            && app_path.is_some()
            && shown_binary.is_some_and(|binary| !is_app_managed(binary))
            && status.install_id.is_none(),
        uses_app_managed,
        app_managed_installed: app_path.is_some(),
    }
}

/// The name of `source`; «Automatic» names the binary it runs (`runs_app_managed`) if any.
fn source_text(source: ResticSourceCode, runs_app_managed: Option<bool>) -> String {
    match source {
        ResticSourceCode::Automatic => match runs_app_managed {
            Some(app) => {
                let used = if app {
                    GuiText::ResticAppManaged
                } else {
                    GuiText::ResticSystem
                };
                i18n("Automatic (%1)").subs(&[&used.text()])
            }
            None => GuiText::ResticAutomatic.text(),
        },
        ResticSourceCode::System => GuiText::ResticSystem.text(),
        ResticSourceCode::AppManaged => GuiText::ResticAppManaged.text(),
        ResticSourceCode::Custom => GuiText::ResticCustom.text(),
    }
}

/// The version in use with «recommended: …» below the tested current version, or the too old
/// version found with «too old, at least …»; empty without either.
fn version_text(status: &ResticStatusDto) -> String {
    if status.valid {
        let version = status.version.as_deref().unwrap_or_default();
        if is_outdated_text(version) {
            i18n("%1 · recommended: %2").subs(&[version, &TESTED_RESTIC_VERSION.to_string()])
        } else {
            version.to_string()
        }
    } else if status.error_kind == Some(ResticStatusCode::TooOld) {
        i18n("%1 · too old, at least %2").subs(&[
            status.found_version.as_deref().unwrap_or_default(),
            &MIN_RESTIC_VERSION.to_string(),
        ])
    } else {
        String::new()
    }
}

/// Both versions when the system and the app-managed restic exist, else empty.
fn available_text(status: &ResticStatusDto) -> String {
    let (Some(system), Some(app)) = (&status.system, &status.app_managed) else {
        return String::new();
    };
    let version = |binary: &ResticBinaryDto| {
        binary
            .version
            .clone()
            .unwrap_or_else(|| i18n("unknown version").to_string())
    };
    i18n("System %1 · Managed by Kestic %2").subs(&[&version(system), &version(app)])
}

/// Why restic cannot be used and its raw reason for «Details», when the version row does not
/// already say it. A reason that is only the raw text has no details.
fn problem_texts(status: &ResticStatusDto) -> (String, String) {
    match status.error_kind {
        None | Some(ResticStatusCode::TooOld) => (String::new(), String::new()),
        Some(code) => {
            let text = status.error_message().map(|m| tr(&m)).unwrap_or_default();
            let translated = code == ResticStatusCode::InvalidCustomPath
                && status
                    .path_problem
                    .is_some_and(|problem| problem != ResticPathProblem::Other);
            let detail = if translated {
                status.detail.clone().unwrap_or_default()
            } else {
                String::new()
            };
            (text, detail)
        }
    }
}

/// Whether applying the source `kind` (a `ResticSourceCode`) first has to download the
/// app-managed restic: it is chosen but not installed.
pub fn needs_install_first(kind: &str, app_managed_installed: bool) -> bool {
    kind == ResticSourceCode::AppManaged.as_str() && !app_managed_installed
}

/// Whether the installation `finished_id`, ended in `state`, switches the source to the
/// app-managed restic: only the one the source dialog started for it (`pending_id`), and
/// only when it succeeded.
pub fn switch_after_install(pending_id: &str, finished_id: &str, state: &str) -> bool {
    !pending_id.is_empty() && pending_id == finished_id && state == "done"
}

/// Why `RemoveAppManagedRestic` failed: the restic is in use, or the daemon's text for `code`
/// (its general «in use» text is about destinations).
pub fn remove_failure_text(code: ServiceErrorCode) -> String {
    match code {
        ServiceErrorCode::InUse => {
            i18n("The restic managed by Kestic is still in use and cannot be deleted now.")
                .to_string()
        }
        other => tr(&other.message()),
    }
}

#[cfg(test)]
#[path = "restic_section_view_tests.rs"]
mod tests;
