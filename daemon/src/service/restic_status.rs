// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The restic status the daemon reports: the configured source, the binary it resolves to or
//! why there is none, and the system and app-managed restic it found.

use crate::dbus_api::{ResticBinaryDto, ResticStatusDto};
use kestic_core::model::ResticProvider;
use kestic_core::provider::{
    CheckedBinary, ResolvedRestic, ResticLocations, ResticProviderError, ResticSurvey, same_binary,
};
use kestic_i18n::codes::{ResticSourceCode, ResticStatusCode};

/// The code of the configured source.
pub(super) fn source_code(provider: &ResticProvider) -> ResticSourceCode {
    match provider {
        ResticProvider::Automatic => ResticSourceCode::Automatic,
        ResticProvider::System => ResticSourceCode::System,
        ResticProvider::AppManaged => ResticSourceCode::AppManaged,
        ResticProvider::Custom { .. } => ResticSourceCode::Custom,
    }
}

fn binary_dto(checked: &CheckedBinary) -> ResticBinaryDto {
    ResticBinaryDto {
        path: checked.path.to_string_lossy().into_owned(),
        version: checked.found_version().map(ToString::to_string),
        too_old: checked.too_old(),
    }
}

/// The active binary part of the status: the resolved binary, or why there is none.
fn resolution_dto(
    resolved: &Result<ResolvedRestic, ResticProviderError>,
    source: ResticSourceCode,
) -> ResticStatusDto {
    let path = |p: &std::path::Path| Some(p.to_string_lossy().into_owned());
    let mut dto = ResticStatusDto {
        valid: false,
        binary_path: None,
        version: None,
        error_kind: None,
        found_version: None,
        detail: None,
        path_problem: None,
        source,
        system: None,
        app_managed: None,
        offer_switch_to_system: false,
        install_id: None,
    };
    match resolved {
        Ok(resolved) => {
            dto.valid = true;
            dto.binary_path = path(&resolved.binary);
            dto.version = Some(resolved.version.to_string());
            dto.found_version = Some(resolved.version.to_string());
        }
        Err(ResticProviderError::NotFound { .. }) => {
            dto.error_kind = Some(ResticStatusCode::NotFound);
        }
        Err(ResticProviderError::TooOld { binary, found, .. }) => {
            dto.error_kind = Some(ResticStatusCode::TooOld);
            dto.binary_path = path(binary);
            dto.found_version = Some(found.to_string());
        }
        Err(ResticProviderError::InvalidCustomPath {
            path: bad,
            problem,
            reason,
        }) => {
            dto.error_kind = Some(ResticStatusCode::InvalidCustomPath);
            dto.binary_path = path(bad);
            dto.path_problem = Some(*problem);
            dto.detail = Some(reason.clone());
        }
        Err(ResticProviderError::UnreadableVersion { binary, detail }) => {
            dto.error_kind = Some(ResticStatusCode::UnreadableVersion);
            dto.binary_path = path(binary);
            dto.detail = Some(detail.clone());
        }
    }
    dto
}

/// Checks the system and the app-managed restic at `locations` once and resolves `provider`
/// from them; returns the status without `install_id` and the resolved binary.
pub(super) fn restic_status_with(
    provider: &ResticProvider,
    locations: &ResticLocations,
) -> (ResticStatusDto, Result<ResolvedRestic, ResticProviderError>) {
    let survey = ResticSurvey::probe(locations);
    let resolved = survey.resolve(provider);
    let mut dto = resolution_dto(&resolved, source_code(provider));
    dto.system = survey.system.as_ref().map(binary_dto);
    dto.app_managed = survey.app_managed.as_ref().map(binary_dto);
    // A system restic that is only a link to the app-managed one is no alternative to it.
    dto.offer_switch_to_system = *provider == ResticProvider::AppManaged
        && survey.system.as_ref().is_some_and(|system| {
            system.usable()
                && locations
                    .app_managed
                    .as_deref()
                    .is_none_or(|app| !same_binary(&system.path, app))
        });
    (dto, resolved)
}

#[cfg(test)]
mod tests;
