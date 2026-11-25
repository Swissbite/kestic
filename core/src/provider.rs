// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Restic binary discovery and version checking (`ResticProvider`).

use crate::model::ResticProvider;
use crate::restic_gate::ResticBinaryGate;
use kestic_i18n::codes::ResticPathProblem;
use semver::Version;
use serde::Deserialize;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

/// Minimum supported restic version.
pub const MIN_RESTIC_VERSION: Version = Version::new(0, 18, 0);

/// The newest restic version the tests run against (`RESTIC_LATEST_VERSION` in
/// `scripts/bootstrap.sh`). An older version still works fully; settings only name this
/// version as the recommended one.
pub const TESTED_RESTIC_VERSION: Version = Version::new(0, 20, 0);

/// Whether `version` is older than the newest tested version.
pub fn is_outdated(version: &Version) -> bool {
    *version < TESTED_RESTIC_VERSION
}

/// [`is_outdated`] for a version as text (`0.18.0`); unreadable text is not outdated.
pub fn is_outdated_text(version: &str) -> bool {
    Version::parse(version.trim()).is_ok_and(|v| is_outdated(&v))
}

/// Successfully resolved and validated restic binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRestic {
    pub binary: PathBuf,
    pub version: Version,
}

/// Errors returned when resolving or validating a [`ResticProvider`].
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ResticProviderError {
    #[error("restic not found (at least {min_version} required)")]
    NotFound { min_version: String },
    #[error("restic path cannot be used ({path}): {reason}")]
    InvalidCustomPath {
        path: PathBuf,
        problem: ResticPathProblem,
        reason: String,
    },
    #[error("unable to determine restic version ({binary}): {detail}")]
    UnreadableVersion { binary: PathBuf, detail: String },
    #[error("restic {found} is too old, at least {min_version} required ({binary})")]
    TooOld {
        binary: PathBuf,
        found: Version,
        min_version: String,
    },
}

impl ResticProviderError {
    fn not_found() -> Self {
        Self::NotFound {
            min_version: MIN_RESTIC_VERSION.to_string(),
        }
    }
}

#[derive(Deserialize)]
struct ResticVersionJson {
    message_type: Option<String>,
    version: String,
}

/// Where the system restic and the restic managed by Kestic are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResticLocations {
    /// `restic` found in `PATH`; `None` when there is none.
    pub system: Option<PathBuf>,
    /// The fixed place of the app-managed restic, whether it exists or not; `None` without
    /// `XDG_DATA_HOME` and `HOME`.
    pub app_managed: Option<PathBuf>,
}

impl ResticLocations {
    /// The locations seen from this process: `restic` in its `PATH` and the app-managed binary
    /// below its `XDG_DATA_HOME`.
    pub fn from_env() -> Self {
        let app_managed = crate::config::default_data_dir()
            .map(|dir| crate::restic_install::app_managed_binary(&dir));
        Self {
            system: which::which("restic").ok(),
            app_managed,
        }
    }

    /// Checks the app-managed binary; `None` when it does not exist. A removal of it that
    /// runs ends first.
    fn check_app_managed<T>(&self, check: impl FnOnce(&Path) -> T) -> Option<T> {
        let path = self.app_managed.as_deref()?;
        let _lease = ResticBinaryGate::global().read_for(path);
        path.symlink_metadata().ok()?;
        Some(check(path))
    }
}

/// A binary that was found and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedBinary {
    pub path: PathBuf,
    pub result: Result<ResolvedRestic, ResticProviderError>,
}

impl CheckedBinary {
    fn check(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            result: check_binary_version(path),
        }
    }

    /// The version the binary reports, also when it is too old.
    pub fn found_version(&self) -> Option<&Version> {
        match &self.result {
            Ok(resolved) => Some(&resolved.version),
            Err(ResticProviderError::TooOld { found, .. }) => Some(found),
            Err(_) => None,
        }
    }

    /// Whether the binary reports a version below [`MIN_RESTIC_VERSION`].
    pub fn too_old(&self) -> bool {
        matches!(self.result, Err(ResticProviderError::TooOld { .. }))
    }

    /// Whether the binary can be used.
    pub fn usable(&self) -> bool {
        self.result.is_ok()
    }
}

/// Both binaries Kestic can use without a custom path, each checked once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResticSurvey {
    /// `None` when there is no `restic` in `PATH`.
    pub system: Option<CheckedBinary>,
    /// `None` when the app-managed binary does not exist.
    pub app_managed: Option<CheckedBinary>,
}

impl ResticSurvey {
    /// Checks the system and the app-managed restic of `locations`.
    pub fn probe(locations: &ResticLocations) -> Self {
        Self {
            system: locations.system.as_deref().map(CheckedBinary::check),
            app_managed: locations.check_app_managed(CheckedBinary::check),
        }
    }

    /// The binary `provider` uses, from the checks of this survey; a custom path is checked
    /// now.
    pub fn resolve(
        &self,
        provider: &ResticProvider,
    ) -> Result<ResolvedRestic, ResticProviderError> {
        resolve_by(
            provider,
            || self.system.as_ref().map(|c| c.result.clone()),
            || self.app_managed.as_ref().map(|c| c.result.clone()),
        )
    }
}

/// Resolves and validates the restic executable configured by `provider`, looking for the
/// system and the app-managed restic in `locations`. Only the binaries the provider needs are
/// checked.
pub fn resolve_with(
    provider: &ResticProvider,
    locations: &ResticLocations,
) -> Result<ResolvedRestic, ResticProviderError> {
    resolve_by(
        provider,
        || locations.system.as_deref().map(check_binary_version),
        || locations.check_app_managed(check_binary_version),
    )
}

/// Resolution of `provider` from the checks of the system and the app-managed binary (`None`:
/// not there), which run only when needed. «Automatic» takes the system restic if it is usable,
/// else an existing app-managed one; without either it reports why the system restic is not
/// usable.
fn resolve_by(
    provider: &ResticProvider,
    system: impl FnOnce() -> Option<Result<ResolvedRestic, ResticProviderError>>,
    app_managed: impl FnOnce() -> Option<Result<ResolvedRestic, ResticProviderError>>,
) -> Result<ResolvedRestic, ResticProviderError> {
    let missing = || Err(ResticProviderError::not_found());
    match provider {
        ResticProvider::System => system().unwrap_or_else(missing),
        ResticProvider::AppManaged => app_managed().unwrap_or_else(missing),
        ResticProvider::Custom { path } => check_binary_version(path),
        ResticProvider::Automatic => match system() {
            Some(Ok(resolved)) => Ok(resolved),
            system_err => app_managed().or(system_err).unwrap_or_else(missing),
        },
    }
}

/// Resolves and validates the restic executable configured by `provider` (see
/// [`resolve_with`]) at the locations of this process.
pub fn resolve_and_check(provider: &ResticProvider) -> Result<ResolvedRestic, ResticProviderError> {
    resolve_with(provider, &ResticLocations::from_env())
}

/// Validates that `path` exists, is a regular file (or symlink to one), and has an executable bit.
pub fn validate_executable_path(path: &Path) -> Result<(), ResticProviderError> {
    let invalid = |problem, reason: String| ResticProviderError::InvalidCustomPath {
        path: path.to_path_buf(),
        problem,
        reason,
    };
    if path.as_os_str().is_empty() {
        return Err(invalid(
            ResticPathProblem::NotFound,
            "the path is empty".to_string(),
        ));
    }
    // A relative path would be checked against the daemon's working folder but looked up in
    // `PATH` when restic starts.
    if !path.is_absolute() {
        return Err(invalid(
            ResticPathProblem::Other,
            "the path is not absolute".to_string(),
        ));
    }
    let meta = std::fs::metadata(path).map_err(|err| {
        let problem = match err.kind() {
            ErrorKind::NotFound | ErrorKind::NotADirectory => ResticPathProblem::NotFound,
            ErrorKind::PermissionDenied => ResticPathProblem::PermissionDenied,
            _ => ResticPathProblem::Other,
        };
        invalid(problem, err.to_string())
    })?;
    if !meta.is_file() {
        return Err(invalid(
            ResticPathProblem::NotAFile,
            "path is not a file".to_string(),
        ));
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(invalid(
            ResticPathProblem::NotExecutable,
            "file is not executable".to_string(),
        ));
    }
    Ok(())
}

/// Probes `binary` via `restic version --json` and falls back to plain-text `restic version`.
pub fn check_binary_version(binary: &Path) -> Result<ResolvedRestic, ResticProviderError> {
    // Held for the whole check, so the binary is neither replaced nor removed between the calls.
    let _lease = ResticBinaryGate::global().read_for(binary);
    validate_executable_path(binary)?;

    let version = probe_version_json(binary)
        .or_else(|_| probe_version_text(binary))
        .map_err(|detail| ResticProviderError::UnreadableVersion {
            binary: binary.to_path_buf(),
            detail,
        })?;

    if version < MIN_RESTIC_VERSION {
        return Err(ResticProviderError::TooOld {
            binary: binary.to_path_buf(),
            found: version,
            min_version: MIN_RESTIC_VERSION.to_string(),
        });
    }

    Ok(ResolvedRestic {
        binary: binary.to_path_buf(),
        version,
    })
}

fn probe_version_json(binary: &Path) -> Result<Version, String> {
    let out = Command::new(binary)
        .args(["version", "--json"])
        .output()
        .map_err(|err| format!("failed to execute `{}`: {err}", binary.display()))?;

    if !out.status.success() {
        return Err(format!(
            "`restic version --json` exited with {}",
            out.status
        ));
    }

    parse_version_json(&out.stdout)
}

/// The version in the output of `restic version --json`: one message of type `version`.
pub(crate) fn parse_version_json(stdout: &[u8]) -> Result<Version, String> {
    let stdout = String::from_utf8_lossy(stdout);
    let parsed: ResticVersionJson =
        serde_json::from_str(stdout.trim()).map_err(|err| format!("invalid JSON: {err}"))?;
    if parsed.message_type.as_deref() != Some("version") {
        return Err(format!(
            "no version message: message_type {:?}",
            parsed.message_type
        ));
    }
    parse_semver_token(&parsed.version)
}

fn probe_version_text(binary: &Path) -> Result<Version, String> {
    let out = Command::new(binary)
        .arg("version")
        .output()
        .map_err(|err| format!("failed to execute `{}`: {err}", binary.display()))?;

    if !out.status.success() {
        return Err(format!("`restic version` exited with {}", out.status));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_version_text(&stdout)
        .ok_or_else(|| format!("could not find semver in `{}`", stdout.trim()))
}

/// Extracts the version from the text of `restic version` (`restic 0.16.4 compiled with go1.22
/// …`); `None` for the output of any other program.
pub fn parse_version_text(text: &str) -> Option<Version> {
    let mut tokens = text.split_whitespace();
    if tokens.next()? != "restic" {
        return None;
    }
    parse_semver_token(tokens.next()?).ok()
}

/// Whether `a` and `b` are the same file, also through links.
pub fn same_binary(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn parse_semver_token(raw: &str) -> Result<Version, String> {
    let cleaned = raw.trim().trim_start_matches('v');
    Version::parse(cleaned).map_err(|err| format!("invalid version '{raw}': {err}"))
}
