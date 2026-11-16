// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Restic binary discovery and version checking (`ResticProvider`, `FR-33`, `NFR-05`, `P31`, `M1-S04`).

use crate::model::ResticProvider;
use semver::Version;
use serde::Deserialize;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

/// Minimum supported restic version (`FR-33`, `NFR-05`).
pub const MIN_RESTIC_VERSION: Version = Version::new(0, 18, 0);

/// Successfully resolved and validated restic binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRestic {
    pub binary: PathBuf,
    pub version: Version,
}

/// Errors returned when resolving or validating a [`ResticProvider`].
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ResticProviderError {
    #[error("restic nicht gefunden (mindestens {min_version} nötig)")]
    NotFound { min_version: String },
    #[error("eigener restic-Pfad existiert nicht oder ist nicht ausführbar: {path}")]
    InvalidCustomPath { path: PathBuf, reason: String },
    #[error("Version von restic nicht erkennbar ({binary}): {detail}")]
    UnreadableVersion { binary: PathBuf, detail: String },
    #[error("restic {found} ist zu alt, mindestens {min_version} nötig ({binary})")]
    TooOld {
        binary: PathBuf,
        found: Version,
        min_version: String,
    },
}

#[derive(Deserialize)]
struct ResticVersionJson {
    version: String,
}

/// Resolves and validates the restic executable configured by `provider`.
pub fn resolve_and_check(provider: &ResticProvider) -> Result<ResolvedRestic, ResticProviderError> {
    let binary = match provider {
        ResticProvider::System => {
            which::which("restic").map_err(|_| ResticProviderError::NotFound {
                min_version: MIN_RESTIC_VERSION.to_string(),
            })?
        }
        ResticProvider::AppManaged { path } | ResticProvider::Custom { path } => {
            validate_executable_path(path)?;
            path.clone()
        }
    };

    check_binary_version(&binary)
}

/// Validates that `path` exists, is a regular file (or symlink to one), and has an executable bit.
pub fn validate_executable_path(path: &Path) -> Result<(), ResticProviderError> {
    if path.as_os_str().is_empty() || !path.exists() {
        return Err(ResticProviderError::InvalidCustomPath {
            path: path.to_path_buf(),
            reason: "file does not exist".to_string(),
        });
    }
    let meta = std::fs::metadata(path).map_err(|err| ResticProviderError::InvalidCustomPath {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })?;
    if !meta.is_file() {
        return Err(ResticProviderError::InvalidCustomPath {
            path: path.to_path_buf(),
            reason: "path is not a file".to_string(),
        });
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(ResticProviderError::InvalidCustomPath {
            path: path.to_path_buf(),
            reason: "file is not executable".to_string(),
        });
    }
    Ok(())
}

/// Probes `binary` via `restic version --json` and falls back to plain-text `restic version` (`P31`).
pub fn check_binary_version(binary: &Path) -> Result<ResolvedRestic, ResticProviderError> {
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

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: ResticVersionJson =
        serde_json::from_str(stdout.trim()).map_err(|err| format!("invalid JSON: {err}"))?;
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

/// Extracts a `restic` version from `restic version` text output (e.g. `restic 0.16.4 compiled with go1.22`).
pub fn parse_version_text(text: &str) -> Option<Version> {
    for token in text.split_whitespace() {
        if let Ok(ver) = parse_semver_token(token) {
            return Some(ver);
        }
    }
    None
}

fn parse_semver_token(raw: &str) -> Result<Version, String> {
    let cleaned = raw.trim().trim_start_matches('v');
    Version::parse(cleaned).map_err(|err| format!("invalid version '{raw}': {err}"))
}
