// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destinations of the generic form: the restic repository URL of a backend without a form of
//! its own (Backblaze B2, Azure, Google Cloud, Swift, rclone) and the environment variables that
//! backend reads. Secret variables are kept only in the password store.

use crate::destination_types::RequiredProgram;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Repository schemes the generic form accepts.
pub const SCHEMES: [&str; 5] = ["b2", "azure", "gs", "swift", "rclone"];

/// An environment variable of a generic destination. The value of a secret variable is never
/// stored here, only in the password store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    #[serde(default)]
    pub secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// Why the fields of the generic form are refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GenericInputError {
    /// The repository URL does not start with a known scheme; carries what was entered.
    #[error("unsupported repository scheme in '{0}': expected b2:, azure:, gs:, swift: or rclone:")]
    UnsupportedScheme(String),
    /// The variable name would override Kestic's own environment or is no valid name.
    #[error(
        "environment variable '{0}' is not allowed: names are A-Z, 0-9 and _, and must not \
         override RESTIC_PASSWORD*, RESTIC_REPOSITORY*, PATH, HOME or LD_*"
    )]
    ReservedVariable(String),
    /// Any other problem with the variables (a name twice, a stored secret value).
    #[error("invalid environment variable: {0}")]
    InvalidVariable(String),
}

/// The scheme of `url` (`b2` for `b2:bucket:path`), if it is one of [`SCHEMES`].
pub fn scheme(url: &str) -> Option<&'static str> {
    let (head, _) = url.trim().split_once(':')?;
    SCHEMES.iter().copied().find(|s| *s == head)
}

/// Checks the repository URL: a known scheme followed by the backend's address.
pub fn validate_url(url: &str) -> Result<(), GenericInputError> {
    let trimmed = url.trim();
    let address_given = trimmed
        .split_once(':')
        .is_some_and(|(_, rest)| !rest.trim().is_empty());
    if scheme(trimmed).is_none() || !address_given || trimmed.chars().any(char::is_control) {
        return Err(GenericInputError::UnsupportedScheme(trimmed.to_string()));
    }
    Ok(())
}

/// Checks a variable name: `[A-Z_][A-Z0-9_]*` and none of the names Kestic sets itself
/// (`RESTIC_PASSWORD*`, `RESTIC_REPOSITORY*`) or that redirect the process (`PATH`, `HOME`,
/// `LD_*`).
pub fn validate_variable_name(name: &str) -> Result<(), GenericInputError> {
    let mut chars = name.chars();
    let well_formed = chars
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    let reserved = name.starts_with("RESTIC_PASSWORD")
        || name.starts_with("RESTIC_REPOSITORY")
        || name == "PATH"
        || name == "HOME"
        || name.starts_with("LD_");
    if !well_formed || reserved {
        return Err(GenericInputError::ReservedVariable(name.to_string()));
    }
    Ok(())
}

/// Checks the variables: valid names, each name once, a value only for variables that are not
/// secret (and no NUL byte in it, which no process environment can hold).
pub fn validate_env(vars: &[EnvVar]) -> Result<(), GenericInputError> {
    let mut seen = std::collections::BTreeSet::new();
    for var in vars {
        validate_variable_name(&var.name)?;
        if !seen.insert(var.name.as_str()) {
            return Err(GenericInputError::InvalidVariable(format!(
                "'{}' is given twice",
                var.name
            )));
        }
        match (&var.value, var.secret) {
            (Some(_), true) => {
                return Err(GenericInputError::InvalidVariable(format!(
                    "the value of the secret variable '{}' belongs in the password store",
                    var.name
                )));
            }
            (Some(value), false) if value.contains('\0') => {
                return Err(GenericInputError::InvalidVariable(format!(
                    "the value of '{}' contains a NUL byte",
                    var.name
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether «Geheim» is switched on for a new variable: its name contains `KEY`, `SECRET`,
/// `TOKEN` or `PASSWORD`.
pub fn secret_by_default(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["KEY", "SECRET", "TOKEN", "PASSWORD"]
        .iter()
        .any(|word| upper.contains(word))
}

/// Password store kind of the secret variable `name`.
pub fn secret_kind(name: &str) -> String {
    format!("env_{name}")
}

/// The program restic needs for `url` besides itself: `rclone` for the scheme `rclone:`.
pub fn required_program(url: &str) -> Option<RequiredProgram> {
    (scheme(url) == Some("rclone")).then_some(RequiredProgram::Rclone)
}

/// The program `kind` needs that is not in `search_path` (the `PATH` the daemon starts restic
/// with): `rclone` for a generic destination with the scheme `rclone:`.
pub fn missing_program(
    kind: &crate::model::DestinationKind,
    search_path: Option<&std::ffi::OsStr>,
) -> Option<RequiredProgram> {
    let crate::model::DestinationKind::Generic { url, .. } = kind else {
        return None;
    };
    required_program(url)
        .filter(|p| !crate::destination_types::program_in_path(p.executable(), search_path))
}

/// The variables that may reach restic: a hand-edited configuration with a reserved name is
/// refused when loaded, and this filter keeps such a name out of the process even so.
pub(crate) fn allowed(vars: &[EnvVar]) -> impl Iterator<Item = &EnvVar> {
    vars.iter()
        .filter(|v| validate_variable_name(&v.name).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_needs_an_address_after_it() {
        assert_eq!(scheme("b2:bucket:path"), Some("b2"));
        assert!(validate_url("rclone:").is_err());
        assert!(validate_url("rclone:remote:backup").is_ok());
        assert!(validate_url("gs:bucket:/\u{0}").is_err());
    }

    #[test]
    fn secret_value_is_not_stored_in_the_variable() {
        let var = EnvVar {
            name: "B2_ACCOUNT_KEY".to_string(),
            secret: true,
            value: Some("x".to_string()),
        };
        assert!(matches!(
            validate_env(&[var]),
            Err(GenericInputError::InvalidVariable(_))
        ));
    }
}
