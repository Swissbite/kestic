// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Identity of a restic repository and the rules for editing the connection of a destination:
//! a repository keeps its ID when its files are copied or moved or the server gets a new
//! address, so an edited connection is saved only when it leads to the repository with the ID
//! Kestic knows. Saved credentials go only to the server they were entered for.

use crate::model::DestinationKind;
use std::fmt;
use thiserror::Error;

/// The ID of a restic repository (`restic cat config`, field `id`): 64 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepositoryId(String);

/// Why a text is no repository ID.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepositoryIdError {
    /// The output of `restic cat config` is no JSON object with a string field `id`.
    #[error("restic printed no repository config: {0}")]
    NoConfig(String),
    /// The ID is not 64 hex digits.
    #[error("'{0}' is no restic repository ID")]
    Malformed(String),
}

impl RepositoryId {
    /// Checks `id`: 64 hex digits, stored in lowercase.
    pub fn parse(id: &str) -> Result<Self, RepositoryIdError> {
        let trimmed = id.trim();
        if trimmed.len() != 64 || !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(RepositoryIdError::Malformed(trimmed.to_string()));
        }
        Ok(Self(trimmed.to_ascii_lowercase()))
    }

    /// The ID from the output of `restic cat config` (a JSON object, possibly indented).
    pub fn from_cat_config(stdout: &str) -> Result<Self, RepositoryIdError> {
        // The whole output, the output from its first `{` (after a line that is no JSON) and, for
        // `init --json`, which may print more than one JSON line, every line.
        let from_brace = stdout.find('{').map_or("", |at| &stdout[at..]);
        let id = [stdout.trim(), from_brace.trim()]
            .into_iter()
            .chain(stdout.lines().map(str::trim))
            .filter_map(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            .find_map(|value| value.get("id")?.as_str().map(ToOwned::to_owned))
            .ok_or_else(|| RepositoryIdError::NoConfig(first_line(stdout)))?;
        Self::parse(&id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether an edited connection leads to the repository Kestic knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryMatch {
    /// The same repository: the edit may be saved.
    Same,
    /// Another repository, or none Kestic could compare with: nothing is saved.
    Other,
}

/// Compares the ID found at the edited connection with the known one: the stored ID, or for a
/// destination without a stored ID the one read at its saved address. Without a known ID every
/// repository counts as another one.
pub fn decide(known: Option<&RepositoryId>, found: &RepositoryId) -> RepositoryMatch {
    if known == Some(found) {
        RepositoryMatch::Same
    } else {
        RepositoryMatch::Other
    }
}

/// Whether the edit changes where or how the repository is reached (address, user, region,
/// variables); a new name alone does not.
pub fn connection_changed(old: &DestinationKind, new: &DestinationKind) -> bool {
    old != new
}

/// Whether credentials saved for `old` may be used for `new`: only on the same server. S3: the
/// same endpoint (scheme, host, port). REST: the same scheme and host (the port may change). Generic form: the same repository
/// URL up to the path, with the same variables that are not secret. SFTP and local destinations
/// have no saved backend credentials.
pub fn same_server(old: &DestinationKind, new: &DestinationKind) -> bool {
    match (old, new) {
        (DestinationKind::S3 { endpoint: a, .. }, DestinationKind::S3 { endpoint: b, .. }) => {
            matches!(
                (crate::s3::normalized_endpoint(a), crate::s3::normalized_endpoint(b)),
                (Ok(a), Ok(b)) if a == b
            )
        }
        (DestinationKind::Rest { url: a, .. }, DestinationKind::Rest { url: b, .. }) => {
            matches!((rest_server(a), rest_server(b)), (Some(a), Some(b)) if a == b)
        }
        (
            DestinationKind::Generic { url: a, env: env_a },
            DestinationKind::Generic { url: b, env: env_b },
        ) => generic_server(a) == generic_server(b) && plain_vars(env_a) == plain_vars(env_b),
        (DestinationKind::Local { .. }, DestinationKind::Local { .. })
        | (DestinationKind::Sftp { .. }, DestinationKind::Sftp { .. }) => true,
        _ => false,
    }
}

/// Scheme and host of a REST URL, in ASCII lowercase: a change to `http` would send the saved
/// password unencrypted.
fn rest_server(url: &str) -> Option<(String, String)> {
    let ascii = crate::model::normalize_rest_url_to_ascii(url).ok()?;
    let parsed = url::Url::parse(&ascii).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    Some((parsed.scheme().to_string(), host))
}

/// The variables of a generic destination that are not secret, with their values: they can name
/// the server too (`OS_AUTH_URL`, `AZURE_ENDPOINT_SUFFIX`, a remote's `RCLONE_CONFIG_*`).
fn plain_vars(env: &[crate::generic::EnvVar]) -> Vec<(&str, Option<&str>)> {
    let mut vars: Vec<(&str, Option<&str>)> = env
        .iter()
        .filter(|v| !v.secret)
        .map(|v| (v.name.as_str(), v.value.as_deref()))
        .collect();
    vars.sort_unstable();
    vars
}

/// The part of a generic repository URL before its path: everything up to the last `:` when
/// there are at least two (`b2:bucket:path` → `b2:bucket`), else the whole URL (`b2:bucket`).
fn generic_server(url: &str) -> &str {
    let trimmed = url.trim();
    if trimmed.matches(':').count() >= 2 {
        trimmed
            .rsplit_once(':')
            .map_or(trimmed, |(server, _)| server)
    } else {
        trimmed
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}
