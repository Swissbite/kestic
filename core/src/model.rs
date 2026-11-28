// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Core domain model and glossary types.

use crate::clock::Clock;
pub use crate::excludes::*;
use crate::mount::normalize_lexical_path;
pub use crate::retention::*;
use crate::snapshot::{BACKUP_TAG_PREFIX, META_TAG_PREFIX};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use thiserror::Error;
use time::OffsetDateTime;

/// Base32 lowercase alphabet (`[a-z2-7]`, RFC 4648 without padding).
const BASE32_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Fixed character length for Kestic Base32 identifiers.
pub const ID_LEN: usize = 8;

/// Validation errors for domain identifiers and model invariants.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ModelError {
    #[error("invalid identifier '{0}': expected 8 characters in [a-z2-7]")]
    InvalidId(String),
    #[error("failed to obtain system entropy for ID generation: {0}")]
    Entropy(String),
    #[error("invalid destination configuration: {0}")]
    InvalidDestination(String),
    /// A network address (e.g. `smb://…`) given as the path of a local destination.
    #[error("invalid destination configuration: {0}")]
    NotALocalPath(#[from] crate::local_path::NotALocalPath),
    #[error("invalid backup configuration: {0}")]
    InvalidBackup(String),
    /// The repository URL or a variable of a generic destination is refused.
    #[error("invalid destination configuration: {0}")]
    Generic(#[from] crate::generic::GenericInputError),
    /// A REST address with a user name or password in it.
    #[error("{}", kestic_i18n::messages::REST_ADDRESS_WITH_LOGIN)]
    LoginInRestAddress,
}

fn is_valid_base32_id(raw: &str) -> bool {
    raw.len() == ID_LEN && raw.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'))
}

fn generate_base32_id() -> Result<String, ModelError> {
    let mut buf = [0u8; ID_LEN];
    getrandom::fill(&mut buf).map_err(|err| ModelError::Entropy(err.to_string()))?;
    let mut out = String::with_capacity(ID_LEN);
    for byte in buf {
        out.push(BASE32_ALPHABET[(byte & 0x1f) as usize] as char);
    }
    Ok(out)
}

macro_rules! define_entity_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Generates a new random 8-character Base32 identifier (`[a-z2-7]{8}`).
            pub fn generate() -> Result<Self, ModelError> {
                Ok(Self(generate_base32_id()?))
            }

            /// Generates a new random identifier guaranteed not to be in `existing`.
            pub fn generate_unique<'a>(
                existing: impl IntoIterator<Item = &'a Self>,
            ) -> Result<Self, ModelError> {
                let taken: BTreeSet<&str> = existing.into_iter().map(|id| id.as_str()).collect();
                for _ in 0..64 {
                    let candidate = Self::generate()?;
                    if !taken.contains(candidate.as_str()) {
                        return Ok(candidate);
                    }
                }
                unreachable!("collision across 64 40-bit Base32 samples is practically impossible")
            }

            /// Parses and validates an 8-character Base32 identifier.
            pub fn parse(raw: impl Into<String>) -> Result<Self, ModelError> {
                let raw = raw.into();
                if is_valid_base32_id(&raw) {
                    Ok(Self(raw))
                } else {
                    Err(ModelError::InvalidId(raw))
                }
            }

            /// Returns the identifier as `&str`.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = ModelError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                Self::parse(raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

define_entity_id!(
    DestinationId,
    "Stable 8-char Base32 identifier for a [`Destination`] (`F15`)."
);
define_entity_id!(
    BackupId,
    "Stable 8-char Base32 identifier for a [`Backup`] (`F15`, tag `kestic:<backup-id>`)."
);
define_entity_id!(RunId, "Unique 8-char Base32 identifier for a [`Run`].");

/// Configured restic executable source (glossary: `ResticProvider`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResticProvider {
    /// The system `restic` if it is at least the minimum version, otherwise the app-managed one.
    #[default]
    Automatic,
    /// System `restic` resolved from `$PATH`.
    System,
    /// The restic Kestic downloads to its fixed place `$XDG_DATA_HOME/kestic/restic`. A `path`
    /// elsewhere, written by an older Kestic, becomes [`ResticProvider::Custom`] on loading.
    AppManaged,
    /// User-configured explicit binary path.
    Custom { path: PathBuf },
}

/// Backend specification of a [`Destination`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DestinationKind {
    /// Local directory or mounted external drive.
    Local { path: PathBuf },
    /// restic REST server.
    Rest {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
    },
    /// SFTP server reached with ssh; the login uses an SSH key or the ssh-agent.
    Sftp {
        host: String,
        #[serde(default = "default_sftp_port")]
        port: u16,
        user: String,
        /// Absolute, or relative to the user's home folder on the server.
        path: String,
    },
    /// S3-compatible object storage; the keys are only in the password store.
    S3 {
        /// URL with scheme, e.g. `https://s3.example.com:9000`.
        endpoint: String,
        bucket: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<String>,
    },
    /// A backend of the generic form (B2, Azure, Google Cloud, Swift, rclone): repository URL and
    /// the environment variables it reads; secret values are only in the password store.
    Generic {
        url: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env: Vec<crate::generic::EnvVar>,
    },
}

fn default_sftp_port() -> u16 {
    crate::sftp::DEFAULT_PORT
}

impl DestinationKind {
    /// Whether the destination is reached over the network (all but a local directory or drive):
    /// only then do the network run conditions apply (a local directory or drive needs no network).
    pub fn is_network(&self) -> bool {
        !matches!(self, Self::Local { .. })
    }

    /// Returns the restic repository URI/path passed via `-r` or `RESTIC_REPOSITORY`.
    /// Translates internationalized (IDN/Umlaut) REST hostnames to Punycode ASCII and
    /// percent-encodes non-ASCII path characters so `restic` and HTTP clients accept them.
    pub fn repository_uri(&self) -> String {
        match self {
            Self::Local { path } => path.to_string_lossy().into_owned(),
            Self::Rest { url, .. } => {
                let normalized =
                    normalize_rest_url_to_ascii(url).unwrap_or_else(|_| url.trim().to_string());
                format!("rest:{normalized}")
            }
            Self::Sftp {
                host,
                port,
                user,
                path,
            } => crate::sftp::repository_uri(host, *port, user, path),
            Self::S3 {
                endpoint,
                bucket,
                path,
                ..
            } => crate::s3::repository_uri(endpoint, bucket, path.as_deref()),
            Self::Generic { url, .. } => url.trim().to_string(),
        }
    }

    /// Whether the connection test of a new repository checks the login with restic first: a
    /// wrong S3 key, backend variable or SSH key would otherwise only show when the repository
    /// is created.
    pub fn probes_login_before_init(&self) -> bool {
        matches!(
            self,
            Self::Sftp { .. } | Self::S3 { .. } | Self::Generic { .. }
        )
    }

    /// Whether only restic itself can tell if the destination is reachable: the generic backends
    /// have no address Kestic could check on its own, and an SFTP server may allow no command
    /// but SFTP.
    pub fn needs_restic_preflight(&self) -> bool {
        matches!(self, Self::Sftp { .. } | Self::Generic { .. })
    }

    /// Whether this destination uses separate REST backend credentials.
    pub fn uses_rest_credentials(&self) -> bool {
        matches!(self, Self::Rest { username: Some(u), .. } if !u.is_empty())
    }

    /// Normalized comparison key identifying the target repository location regardless of
    /// trailing slashes, `.` / `..` segments, symlinks, IDN/Punycode equivalence, or URL host casing.
    pub fn canonical_location_key(&self) -> String {
        match self {
            Self::Local { path } => {
                format!("local:{}", normalize_local_path(path).to_string_lossy())
            }
            Self::Rest { url, .. } => format!("rest:{}", normalize_rest_url(url)),
            Self::Sftp {
                host,
                port,
                user,
                path,
            } => crate::sftp::location_key(host, *port, user, path),
            Self::S3 {
                endpoint,
                bucket,
                path,
                ..
            } => crate::s3::location_key(endpoint, bucket, path.as_deref()),
            Self::Generic { url, .. } => format!("generic:{}", generic_location(url)),
        }
    }

    /// Returns `true` if `self` and `other` point to the same underlying repository location.
    pub fn same_location(&self, other: &Self) -> bool {
        self.canonical_location_key() == other.canonical_location_key()
    }
}

fn normalize_local_path(path: &std::path::Path) -> PathBuf {
    if let Ok(canon) = std::fs::canonicalize(path) {
        return canon;
    }

    let mut base = PathBuf::new();
    let mut tail_components: Vec<std::path::Component<'_>> = Vec::new();
    let raw_components: Vec<std::path::Component<'_>> = path.components().collect();

    // Try canonicalizing the deepest existing prefix so symlinks like `/var` or `/tmp` resolve
    // consistently even when the leaf directory was deleted from disk.
    for split in (1..=raw_components.len()).rev() {
        let prefix: PathBuf = raw_components[..split].iter().collect();
        if let Ok(canon) = std::fs::canonicalize(&prefix) {
            base = canon;
            tail_components = raw_components[split..].to_vec();
            break;
        }
    }

    let components = if base.as_os_str().is_empty() {
        raw_components
    } else {
        let mut combined: Vec<std::path::Component<'_>> = Vec::new();
        combined.extend(tail_components);
        combined
    };

    let mut out = base;
    for comp in components {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if matches!(
                    out.components().next_back(),
                    Some(std::path::Component::Normal(_))
                ) {
                    out.pop();
                } else if !out.has_root() {
                    out.push(comp);
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn strip_invisible_url_chars(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|&c| {
            !matches!(
                c,
                '\u{00AD}' // soft hyphen
                    | '\u{200B}' // zero-width space
                    | '\u{200C}' // zero-width non-joiner
                    | '\u{200D}' // zero-width joiner
                    | '\u{200E}' // left-to-right mark
                    | '\u{200F}' // right-to-left mark
                    | '\u{2060}' // word joiner
                    | '\u{FEFF}' // BOM / zero-width no-break space
            )
        })
        .collect()
}

/// Normalizes an `http://` or `https://` REST URL into RFC-3986 ASCII:
/// internationalized (Umlaut/Unicode) domain labels are converted to Punycode (`xn--...`)
/// and non-ASCII path/query characters are percent-encoded.
pub fn normalize_rest_url_to_ascii(raw: &str) -> Result<String, ModelError> {
    let cleaned = strip_invisible_url_chars(raw);
    let trimmed = cleaned.as_str();
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.is_empty() || (!lower.starts_with("http://") && !lower.starts_with("https://")) {
        return Err(ModelError::InvalidDestination(
            "REST destination URL must start with http:// or https://".to_string(),
        ));
    }
    let parsed = url::Url::parse(trimmed).map_err(|err| {
        ModelError::InvalidDestination(format!("invalid REST destination URL: {err}"))
    })?;
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err(ModelError::InvalidDestination(
            "REST destination URL must include a host".to_string(),
        ));
    }
    let mut formatted = parsed.to_string();
    if parsed.path() == "/"
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && !trimmed.ends_with('/')
        && formatted.ends_with('/')
    {
        formatted.pop();
    }
    Ok(formatted)
}

/// Refuses a REST address with a user name or password in it: both would stand in `config.toml`
/// and in restic's arguments. An address that does not parse passes here;
/// [`Destination::validate`] refuses it.
pub fn refuse_rest_login(url: &str) -> Result<(), ModelError> {
    let cleaned = strip_invisible_url_chars(url);
    match url::Url::parse(cleaned.trim()) {
        Ok(parsed) if !parsed.username().is_empty() || parsed.password().is_some() => {
            Err(ModelError::LoginInRestAddress)
        }
        _ => Ok(()),
    }
}

fn normalize_rest_url(raw: &str) -> String {
    if let Ok(ascii) = normalize_rest_url_to_ascii(raw) {
        return ascii.trim_end_matches('/').to_string();
    }
    let trimmed = raw.trim();
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return trimmed.trim_end_matches('/').to_string();
    };
    let (authority, path_part) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    let normalized_authority = match authority.rsplit_once('@') {
        Some((userinfo, host)) => format!("{userinfo}@{}", host.to_ascii_lowercase()),
        None => authority.to_ascii_lowercase(),
    };
    let normalized_path = path_part.trim_end_matches('/');
    format!(
        "{}://{}{}",
        scheme.to_ascii_lowercase(),
        normalized_authority,
        normalized_path
    )
}

/// Run conditions as they apply to a destination of `kind`: a local directory or drive needs
/// no network, so its network conditions are dropped (a local directory or drive needs no network).
impl RunConditions {
    pub fn for_destination(mut self, kind: &DestinationKind) -> Self {
        if !kind.is_network() {
            self.require_network = false;
            self.skip_on_metered_network = false;
            self.allowed_networks.clear();
        }
        self
    }
}

/// Secret Service reference keys for a [`Destination`] (never plaintext values).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationSecretRefs {
    /// Reference key for the repository password in [`crate::secrets::SecretStore`].
    pub repo_password_ref: String,
    /// Optional reference key for REST server password in [`crate::secrets::SecretStore`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest_password_ref: Option<String>,
    /// Reference key for the S3 access key ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_access_key_ref: Option<String>,
    /// Reference key for the S3 secret access key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_secret_key_ref: Option<String>,
    /// Reference keys of the secret variables of a generic destination, by variable name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub env_refs: std::collections::BTreeMap<String, String>,
}

impl DestinationSecretRefs {
    /// Derives canonical Secret Service reference keys for a destination of `kind`.
    pub fn for_destination(id: &DestinationId, kind: &DestinationKind) -> Self {
        let slot_ref = |wanted: &str| {
            kind.backend_secret_slots()
                .iter()
                .any(|s| s.kind == wanted)
                .then(|| format!("kestic:dest:{id}:{wanted}"))
        };
        Self {
            repo_password_ref: format!("kestic:dest:{id}:repo"),
            rest_password_ref: slot_ref(crate::backend_credentials::SECRET_KIND_REST),
            s3_access_key_ref: slot_ref(crate::backend_credentials::SECRET_KIND_S3_ACCESS_KEY),
            s3_secret_key_ref: slot_ref(crate::backend_credentials::SECRET_KIND_S3_SECRET_KEY),
            env_refs: match kind {
                DestinationKind::Generic { env, .. } => env
                    .iter()
                    .filter(|v| v.secret)
                    .map(|v| {
                        let kind = crate::generic::secret_kind(&v.name);
                        (v.name.clone(), format!("kestic:dest:{id}:{kind}"))
                    })
                    .collect(),
                _ => std::collections::BTreeMap::new(),
            },
        }
    }
}

/// ISO weekday for calendar schedules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Monday = 1,
    Tuesday = 2,
    Wednesday = 3,
    Thursday = 4,
    Friday = 5,
    Saturday = 6,
    Sunday = 7,
}

impl Weekday {
    pub const ALL: [Self; 7] = [
        Self::Monday,
        Self::Tuesday,
        Self::Wednesday,
        Self::Thursday,
        Self::Friday,
        Self::Saturday,
        Self::Sunday,
    ];

    pub fn from_iso_number(day: u8) -> Result<Self, ModelError> {
        match day {
            1 => Ok(Self::Monday),
            2 => Ok(Self::Tuesday),
            3 => Ok(Self::Wednesday),
            4 => Ok(Self::Thursday),
            5 => Ok(Self::Friday),
            6 => Ok(Self::Saturday),
            7 => Ok(Self::Sunday),
            other => Err(ModelError::InvalidBackup(format!(
                "weekday must be 1..=7, got {other}"
            ))),
        }
    }

    pub fn iso_number(self) -> u8 {
        self as u8
    }

    pub fn from_jiff(w: jiff::civil::Weekday) -> Self {
        match w {
            jiff::civil::Weekday::Monday => Self::Monday,
            jiff::civil::Weekday::Tuesday => Self::Tuesday,
            jiff::civil::Weekday::Wednesday => Self::Wednesday,
            jiff::civil::Weekday::Thursday => Self::Thursday,
            jiff::civil::Weekday::Friday => Self::Friday,
            jiff::civil::Weekday::Saturday => Self::Saturday,
            jiff::civil::Weekday::Sunday => Self::Sunday,
        }
    }
}

/// Ordinal weekday within a month for monthly maintenance schedules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NthWeekday {
    First,
    Second,
    Third,
    Fourth,
    Last,
}

/// Calendar schedule for `prune` / `check` destination maintenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaintenanceEntrySchedule {
    ManualOnly,
    Daily {
        hour: u8,
        minute: u8,
    },
    Weekly {
        weekday: Weekday,
        hour: u8,
        minute: u8,
    },
    Monthly {
        nth: NthWeekday,
        weekday: Weekday,
        hour: u8,
        minute: u8,
    },
}

impl MaintenanceEntrySchedule {
    pub fn default_prune() -> Self {
        Self::Weekly {
            weekday: Weekday::Sunday,
            hour: 12,
            minute: 0,
        }
    }

    pub fn default_check() -> Self {
        Self::Monthly {
            nth: NthWeekday::First,
            weekday: Weekday::Sunday,
            hour: 12,
            minute: 0,
        }
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        let (hour, minute) = match *self {
            Self::ManualOnly => return Ok(()),
            Self::Daily { hour, minute }
            | Self::Weekly { hour, minute, .. }
            | Self::Monthly { hour, minute, .. } => (hour, minute),
        };
        if hour >= 24 || minute >= 60 {
            return Err(ModelError::InvalidDestination(format!(
                "invalid maintenance time {hour:02}:{minute:02}"
            )));
        }
        Ok(())
    }
}

/// Maintenance schedule on a [`Destination`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaintenanceSchedule {
    pub prune: MaintenanceEntrySchedule,
    pub check: MaintenanceEntrySchedule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prune_interval_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_interval_days: Option<u32>,
}

impl Default for MaintenanceSchedule {
    fn default() -> Self {
        Self {
            prune: MaintenanceEntrySchedule::default_prune(),
            check: MaintenanceEntrySchedule::default_check(),
            prune_interval_days: None,
            check_interval_days: None,
        }
    }
}

impl MaintenanceSchedule {
    pub fn validate(&self) -> Result<(), ModelError> {
        self.prune.validate()?;
        self.check.validate()?;
        Ok(())
    }
}

/// Maintenance action types (glossary: `Maintenance::Prune`, `Maintenance::Check`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Maintenance {
    Prune,
    Check,
}

/// The location part of a generic repository URL for comparing: without trailing slashes,
/// except one right after a colon, where `remote:/` (the root) and `remote:` (the remote's own
/// folder) are different folders.
fn generic_location(url: &str) -> String {
    let url = url.trim();
    let trimmed = url.trim_end_matches('/');
    if trimmed.ends_with(':') && trimmed.len() < url.len() {
        format!("{trimmed}/")
    } else {
        trimmed.to_string()
    }
}

/// Configured backup repository (glossary: `Destination`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    pub id: DestinationId,
    pub name: String,
    pub kind: DestinationKind,
    pub secrets: DestinationSecretRefs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_kit_key_id: Option<String>,
    #[serde(default)]
    pub maintenance: MaintenanceSchedule,
}

impl Destination {
    /// Creates a new validated [`Destination`] with a freshly generated [`DestinationId`].
    pub fn new(name: impl Into<String>, kind: DestinationKind) -> Result<Self, ModelError> {
        let id = DestinationId::generate()?;
        Self::with_id(id, name, kind)
    }

    /// Creates a validated [`Destination`] with a pre-assigned [`DestinationId`].
    pub fn with_id(
        id: DestinationId,
        name: impl Into<String>,
        kind: DestinationKind,
    ) -> Result<Self, ModelError> {
        let name = name.into();
        let dest = Self {
            secrets: DestinationSecretRefs::for_destination(&id, &kind),
            id,
            name,
            kind,
            recovery_kit_key_id: None,
            maintenance: MaintenanceSchedule::default(),
        };
        dest.validate()?;
        Ok(dest)
    }

    /// Validates invariants on this destination.
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.name.trim().is_empty() {
            return Err(ModelError::InvalidDestination(
                "destination name must not be empty".to_string(),
            ));
        }
        match &self.kind {
            DestinationKind::Local { path } => {
                if path.as_os_str().is_empty() {
                    return Err(ModelError::InvalidDestination(
                        "local destination path must not be empty".to_string(),
                    ));
                }
                // restic gets the path as text: another path would be written to.
                let Some(text) = path.to_str().filter(|t| !t.contains('\0')) else {
                    return Err(ModelError::InvalidDestination(format!(
                        "local destination path must be valid UTF-8 without NUL characters: '{}'",
                        path.display()
                    )));
                };
                if crate::local_path::kio_address_scheme(text).is_some() {
                    return Err(crate::local_path::NotALocalPath(
                        crate::local_path::address_for_display(text),
                    )
                    .into());
                }
                if !path.is_absolute() {
                    return Err(ModelError::InvalidDestination(format!(
                        "local destination path must be absolute: '{}'",
                        path.display()
                    )));
                }
            }
            DestinationKind::Rest { url, .. } => {
                normalize_rest_url_to_ascii(url)?;
            }
            DestinationKind::Sftp {
                host,
                port,
                user,
                path,
            } => crate::sftp::validate(host, *port, user, path)?,
            DestinationKind::S3 {
                endpoint,
                bucket,
                path,
                region,
            } => {
                crate::s3::validate(endpoint, bucket, path.as_deref())?;
                crate::s3::validate_region(region.as_deref())?;
            }
            DestinationKind::Generic { url, env } => {
                crate::generic::validate_url(url)?;
                crate::generic::validate_env(env)?;
            }
        }
        if self.secrets.repo_password_ref.trim().is_empty() {
            return Err(ModelError::InvalidDestination(
                "repo_password_ref must not be empty".to_string(),
            ));
        }
        self.maintenance.validate()?;
        Ok(())
    }

    /// [`Self::validate`] plus the rules for a destination that is entered, added or changed: a
    /// REST address without a user name or password. Loading a config checks only
    /// [`Self::validate`], so destinations saved before these rules keep loading.
    pub fn validate_input(&self) -> Result<(), ModelError> {
        self.validate()?;
        if let DestinationKind::Rest { url, .. } = &self.kind {
            refuse_rest_login(url)?;
        }
        Ok(())
    }
}

/// Backup definition: Source -> Destination pair (glossary: `Backup`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    pub id: BackupId,
    pub name: String,
    pub source: Source,
    pub destination_id: DestinationId,
    #[serde(default)]
    pub excludes: ExcludeRules,
    #[serde(default)]
    pub schedule: Schedule,
    #[serde(default)]
    pub run_conditions: RunConditions,
    #[serde(default)]
    pub retention: RetentionPolicy,
    /// Optional per-backup override for missed-slot overdue threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overdue_threshold: Option<u32>,
}

impl Backup {
    /// Creates a new validated [`Backup`] with a fresh random [`BackupId`].
    pub fn new(
        name: impl Into<String>,
        source: Source,
        destination_id: DestinationId,
    ) -> Result<Self, ModelError> {
        let id = BackupId::generate()?;
        Self::with_id(id, name, source, destination_id)
    }

    /// Creates a validated [`Backup`] with a pre-assigned [`BackupId`].
    pub fn with_id(
        id: BackupId,
        name: impl Into<String>,
        source: Source,
        destination_id: DestinationId,
    ) -> Result<Self, ModelError> {
        let backup = Self {
            id,
            name: name.into(),
            source,
            destination_id,
            excludes: ExcludeRules::default(),
            schedule: Schedule::default(),
            run_conditions: RunConditions::default(),
            retention: RetentionPolicy::default(),
            overdue_threshold: None,
        };
        backup.validate()?;
        Ok(backup)
    }

    /// Effective overdue threshold in missed slots (`>= 1`), falling back to `global_default`.
    pub fn effective_overdue_threshold(&self, global_default: u32) -> u32 {
        self.overdue_threshold.unwrap_or(global_default).max(1)
    }

    /// Renames the backup while keeping its [`BackupId`] immutable.
    pub fn rename(&mut self, name: impl Into<String>) -> Result<(), ModelError> {
        let next = name.into();
        if next.trim().is_empty() {
            return Err(ModelError::InvalidBackup(
                "backup name must not be empty".to_string(),
            ));
        }
        self.name = next;
        Ok(())
    }

    /// Updates the backup source while keeping its [`BackupId`] immutable.
    pub fn set_source(&mut self, source: Source) -> Result<(), ModelError> {
        if source.path.as_os_str().is_empty() {
            return Err(ModelError::InvalidBackup(
                "backup source path must not be empty".to_string(),
            ));
        }
        if !source.path.is_absolute() {
            return Err(ModelError::InvalidBackup(format!(
                "backup source path must be absolute: '{}'",
                source.path.display()
            )));
        }
        self.source = source;
        Ok(())
    }

    /// Assigns this backup to another [`DestinationId`], keeping `self.id` stable.
    pub fn reassign_destination(&mut self, destination_id: DestinationId) {
        self.destination_id = destination_id;
    }

    /// Returns the restic snapshot tag identifying this backup (`kestic:<backup-id>`).
    pub fn tag(&self) -> String {
        format!("{BACKUP_TAG_PREFIX}{}", self.id)
    }

    /// Returns the restic metadata snapshot tag (`kestic-meta:<backup-id>`).
    pub fn meta_tag(&self) -> String {
        format!("{META_TAG_PREFIX}{}", self.id)
    }

    /// Validates invariants on this backup definition.
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.name.trim().is_empty() {
            return Err(ModelError::InvalidBackup(
                "backup name must not be empty".to_string(),
            ));
        }
        if self.source.path.as_os_str().is_empty() {
            return Err(ModelError::InvalidBackup(
                "backup source path must not be empty".to_string(),
            ));
        }
        if !self.source.path.is_absolute() {
            return Err(ModelError::InvalidBackup(format!(
                "backup source path must be absolute: '{}'",
                self.source.path.display()
            )));
        }
        if matches!(self.overdue_threshold, Some(0)) {
            return Err(ModelError::InvalidBackup(
                "overdue_threshold must be at least 1".to_string(),
            ));
        }
        self.schedule.validate()?;
        self.excludes.validate()?;
        self.retention.validate()?;
        Ok(())
    }
}

/// Relationship between two nested source paths on the same destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NestedRelationship {
    /// Candidate source is a subdirectory of `other_backup` (`candidate` inside `other`).
    ContainedIn,
    /// Candidate source encloses `other_backup` (`other` inside `candidate`).
    Contains,
    /// Identical normalized source path.
    SamePath,
}

/// Warning entry for nested sources on the same destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NestedSourceConflict {
    pub other_backup_id: BackupId,
    pub other_backup_name: String,
    pub other_source_path: PathBuf,
    pub relationship: NestedRelationship,
}

fn classify_component_nesting(a: &Path, b: &Path) -> Option<NestedRelationship> {
    let na = normalize_lexical_path(a);
    let nb = normalize_lexical_path(b);
    let ca: Vec<_> = na.components().collect();
    let cb: Vec<_> = nb.components().collect();
    if ca.is_empty() || cb.is_empty() {
        return None;
    }
    if ca == cb {
        Some(NestedRelationship::SamePath)
    } else if ca.starts_with(&cb) {
        Some(NestedRelationship::ContainedIn)
    } else if cb.starts_with(&ca) {
        Some(NestedRelationship::Contains)
    } else {
        None
    }
}

/// Checks whether `candidate_path` on `candidate_dest` nests with any existing backup
/// (excluding `candidate_id` if editing an existing backup).
pub fn check_nested_sources_for_candidate(
    candidate_id: Option<&BackupId>,
    candidate_path: &Path,
    candidate_dest: &DestinationId,
    backups: &[Backup],
) -> Vec<NestedSourceConflict> {
    let mut conflicts = Vec::new();
    for other in backups {
        if candidate_id == Some(&other.id) {
            continue;
        }
        if &other.destination_id != candidate_dest {
            continue;
        }
        if let Some(rel) = classify_component_nesting(candidate_path, &other.source.path) {
            conflicts.push(NestedSourceConflict {
                other_backup_id: other.id.clone(),
                other_backup_name: other.name.clone(),
                other_source_path: other.source.path.clone(),
                relationship: rel,
            });
        }
    }
    conflicts
}

/// Computes all nested-source conflicts across `backups` (bidirectional: both sides get a warning,
/// strictly per destination and using component-wise path matching).
pub fn find_nested_source_warnings(
    backups: &[Backup],
) -> HashMap<BackupId, Vec<NestedSourceConflict>> {
    let mut map: HashMap<BackupId, Vec<NestedSourceConflict>> = HashMap::new();
    for b in backups {
        let conflicts = check_nested_sources_for_candidate(
            Some(&b.id),
            &b.source.path,
            &b.destination_id,
            backups,
        );
        if !conflicts.is_empty() {
            map.insert(b.id.clone(), conflicts);
        }
    }
    map
}

/// Kind of a [`Run`] (glossary: `RunKind`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunKind {
    /// Backup run (`backup` -> `forget`), always tied to a specific [`BackupId`].
    Backup { backup_id: BackupId },
    /// Destination maintenance (`forget`, `prune`, `check`), tied only to a [`DestinationId`].
    Maintenance,
    /// Restore run, tied to a [`DestinationId`] and optionally a [`BackupId`].
    Restore {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backup_id: Option<BackupId>,
    },
    /// Metadata snapshot run, tied only to a [`DestinationId`].
    Meta,
}

/// Adds `wire_name` / `from_wire_name` to an enum whose serde names are the strings stored in
/// the history database.
macro_rules! wire_names {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        impl $name {
            /// String stored in the history database.
            pub fn wire_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }

            /// Parses a stored string; `None` when unknown.
            pub fn from_wire_name(name: &str) -> Option<Self> {
                match name {
                    $($wire => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

/// Lifecycle state of a [`Run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Due, waiting to start (`ausstehend`).
    Pending,
    /// Currently executing (`läuft`).
    Running,
    /// Completed successfully (`ok`).
    Ok,
    /// Completed with warnings, e.g., restic exit 3 (`Warnung`).
    Warning,
    /// Failed (`Fehler`).
    Error,
    /// Cancelled by user via SIGTERM (`abgebrochen`).
    Cancelled,
    /// Interrupted by logout, shutdown, suspend, or process kill (`unterbrochen`).
    Interrupted,
}

wire_names!(RunStatus {
    Pending => "pending",
    Running => "running",
    Ok => "ok",
    Warning => "warning",
    Error => "error",
    Cancelled => "cancelled",
    Interrupted => "interrupted",
});

/// Reason a [`Run`] is in [`RunStatus::Pending`] (`domain-model.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingReason {
    ConditionNotMet,
    SourceUnavailable,
    DestinationUnreachable,
    Queued,
    Backoff,
}

wire_names!(PendingReason {
    ConditionNotMet => "condition_not_met",
    SourceUnavailable => "source_unavailable",
    DestinationUnreachable => "destination_unreachable",
    Queued => "queued",
    Backoff => "backoff",
});

/// Record of an executed restic invocation for a [`Run`].
///
/// Must never contain plaintext secret values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRecord {
    pub executable: String,
    pub args: Vec<String>,
    pub env_keys: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    pub duration_ms: u64,
    pub exit_code: i32,
}

/// Summary statistics parsed from restic `--json` output.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunStats {
    pub files_new: u64,
    pub files_changed: u64,
    pub files_unmodified: u64,
    pub dirs_new: u64,
    pub dirs_changed: u64,
    pub dirs_unmodified: u64,
    pub data_added: u64,
    pub total_files_processed: u64,
    pub total_bytes_processed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
}

/// Snapshots the retention of a maintenance run removed for one backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceForget {
    pub backup_id: BackupId,
    /// Number of snapshots the successful `forget <id…>` was given (0 without one); `None` in
    /// runs recorded before, whose `forget` output held no readable result.
    #[serde(default)]
    pub removed_snapshots: Option<u64>,
}

/// Outcome of `restic check --json` (its `summary` message).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckSummary {
    pub num_errors: u64,
}

/// Result of a maintenance run: pending `forget`s, then `prune`, then `check`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaintenanceStats {
    /// One entry per backup whose pending `forget` succeeded, in run order.
    pub forgets: Vec<MaintenanceForget>,
    /// `prune` ran and exited 0.
    pub pruned: bool,
    /// Bytes `prune` freed; `None` when its output held no readable size.
    pub freed_bytes: Option<u64>,
    /// `check` ran and exited 0.
    pub checked: bool,
    /// Parsed `check` summary, also present when `check` found errors.
    pub check: Option<CheckSummary>,
}

/// What a successful maintenance run did, for its status line («Aufgeräumt und geprüft»).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceOutcome {
    CleanedUpAndVerified,
    CleanedUp,
    Verified,
    /// Only pending `forget`s ran.
    RetentionOnly,
}

impl MaintenanceStats {
    /// Which maintenance steps completed successfully.
    pub fn outcome(&self) -> MaintenanceOutcome {
        match (self.pruned, self.checked) {
            (true, true) => MaintenanceOutcome::CleanedUpAndVerified,
            (true, false) => MaintenanceOutcome::CleanedUp,
            (false, true) => MaintenanceOutcome::Verified,
            (false, false) => MaintenanceOutcome::RetentionOnly,
        }
    }
}

/// Result of a restore run (`RestoreStats`, stored in `runs.stats_json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestoreStats {
    /// Files that did not exist at the target before.
    pub files_new: u64,
    pub files_overwritten: u64,
    /// Existing, different files that stayed (from the preview).
    pub files_kept: u64,
    pub bytes_restored: u64,
    pub target_dir: String,
    pub overwrite: kestic_i18n::codes::RestoreOverwrite,
    /// Number of selected entries (0 = the whole snapshot).
    pub selection_count: u64,
    /// Why a failed restore failed; `None` otherwise and in rows stored before it existed.
    pub error_kind: Option<kestic_i18n::codes::ErrorKind>,
}

/// Single execution record (glossary: `Run`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub kind: RunKind,
    pub destination_id: DestinationId,
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_reason: Option<PendingReason>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_message: Option<String>,
    #[serde(default)]
    pub commands: Vec<CommandRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<RunStats>,
    /// Result of a maintenance run; `None` for other run kinds and for maintenance runs
    /// recorded before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance: Option<MaintenanceStats>,
    /// Result of a restore run; `None` for other run kinds and before the restore ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore: Option<RestoreStats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<PathBuf>,
}

impl Run {
    fn create(
        clock: &dyn Clock,
        kind: RunKind,
        destination_id: DestinationId,
        status: RunStatus,
        pending_reason: Option<PendingReason>,
    ) -> Result<Self, ModelError> {
        Ok(Self {
            id: RunId::generate()?,
            kind,
            destination_id,
            status,
            pending_reason,
            started_at: clock.now_utc(),
            finished_at: None,
            duration_ms: None,
            exit_code: None,
            summary_message: None,
            commands: Vec::new(),
            stats: None,
            maintenance: None,
            restore: None,
            log_path: None,
        })
    }

    /// Creates a new backup run belonging to `backup` and its `destination_id`.
    pub fn new_backup(clock: &dyn Clock, backup: &Backup) -> Result<Self, ModelError> {
        Self::create(
            clock,
            RunKind::Backup {
                backup_id: backup.id.clone(),
            },
            backup.destination_id.clone(),
            RunStatus::Running,
            None,
        )
    }

    /// Creates a pending backup run with a [`PendingReason`].
    pub fn new_pending_backup(
        clock: &dyn Clock,
        backup: &Backup,
        reason: PendingReason,
    ) -> Result<Self, ModelError> {
        Self::create(
            clock,
            RunKind::Backup {
                backup_id: backup.id.clone(),
            },
            backup.destination_id.clone(),
            RunStatus::Pending,
            Some(reason),
        )
    }

    /// Creates a maintenance run belonging to `destination_id`.
    pub fn new_maintenance(
        clock: &dyn Clock,
        destination_id: DestinationId,
    ) -> Result<Self, ModelError> {
        Self::create(
            clock,
            RunKind::Maintenance,
            destination_id,
            RunStatus::Running,
            None,
        )
    }

    /// Creates a restore run belonging to `destination_id` and optionally a [`BackupId`].
    pub fn new_restore(
        clock: &dyn Clock,
        destination_id: DestinationId,
        backup_id: Option<BackupId>,
    ) -> Result<Self, ModelError> {
        Self::create(
            clock,
            RunKind::Restore { backup_id },
            destination_id,
            RunStatus::Running,
            None,
        )
    }

    /// Creates a metadata snapshot run belonging to `destination_id`.
    pub fn new_meta(clock: &dyn Clock, destination_id: DestinationId) -> Result<Self, ModelError> {
        Self::create(
            clock,
            RunKind::Meta,
            destination_id,
            RunStatus::Running,
            None,
        )
    }

    /// Whether this is a backup run of `backup` (a restore of one of its snapshots is not).
    pub fn is_backup_run_of(&self, backup: &BackupId) -> bool {
        matches!(&self.kind, RunKind::Backup { backup_id } if backup_id == backup)
    }

    /// Returns the associated [`BackupId`], if any.
    pub fn backup_id(&self) -> Option<&BackupId> {
        match &self.kind {
            RunKind::Backup { backup_id } => Some(backup_id),
            RunKind::Restore { backup_id } => backup_id.as_ref(),
            RunKind::Maintenance | RunKind::Meta => None,
        }
    }

    /// Marks the run finished at `clock.now_utc()` with final status and exit code.
    pub fn finish(&mut self, clock: &dyn Clock, status: RunStatus, exit_code: Option<i32>) {
        let now = clock.now_utc();
        let elapsed_ms = crate::clock::elapsed_ms(self.started_at, now);
        self.status = status;
        if status != RunStatus::Pending {
            self.pending_reason = None;
        }
        self.finished_at = Some(now);
        self.duration_ms = Some(elapsed_ms);
        self.exit_code = exit_code;
    }
}
