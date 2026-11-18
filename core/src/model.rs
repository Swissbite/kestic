// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Core domain model and glossary types (`F15`, `P38`, `M1-S01`).

use crate::clock::Clock;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeSet, HashMap};
use std::env;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use thiserror::Error;
use time::OffsetDateTime;

/// Base32 lowercase alphabet (`[a-z2-7]`, RFC 4648 without padding).
const BASE32_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Fixed character length for Kestic Base32 identifiers (`F15`, `AC-M1-01-1`).
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
    #[error("invalid backup configuration: {0}")]
    InvalidBackup(String),
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
    /// System `restic` resolved from `$PATH`.
    #[default]
    System,
    /// App-managed download under `$XDG_DATA_HOME/kestic/` (M5).
    AppManaged { path: PathBuf },
    /// User-configured explicit binary path (`FR-33`).
    Custom { path: PathBuf },
}

/// Backend specification of a [`Destination`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DestinationKind {
    /// Local directory or mounted external drive (`FR-01`).
    Local { path: PathBuf },
    /// restic REST server (`FR-01`).
    Rest {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
    },
}

impl DestinationKind {
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
        }
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

/// Secret Service reference keys for a [`Destination`] (never plaintext values, `NFR-02`, `NFR-03`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationSecretRefs {
    /// Reference key for the repository password in [`crate::secrets::SecretStore`].
    pub repo_password_ref: String,
    /// Optional reference key for REST server password in [`crate::secrets::SecretStore`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest_password_ref: Option<String>,
}

impl DestinationSecretRefs {
    /// Derives canonical Secret Service reference keys for a destination ID.
    pub fn for_destination(id: &DestinationId, with_rest_auth: bool) -> Self {
        Self {
            repo_password_ref: format!("kestic:dest:{id}:repo"),
            rest_password_ref: with_rest_auth.then(|| format!("kestic:dest:{id}:rest")),
        }
    }
}

/// Placeholder maintenance schedule on a [`Destination`] (expanded in M3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaintenanceSchedule {
    pub prune_interval_days: Option<u32>,
    pub check_interval_days: Option<u32>,
}

/// Maintenance action types (glossary: `Maintenance::Prune`, `Maintenance::Check`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Maintenance {
    Prune,
    Check,
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
        let uses_rest_auth = kind.uses_rest_credentials();
        let dest = Self {
            secrets: DestinationSecretRefs::for_destination(&id, uses_rest_auth),
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
        }
        if self.secrets.repo_password_ref.trim().is_empty() {
            return Err(ModelError::InvalidDestination(
                "repo_password_ref must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

/// Kind of backup source (glossary: `Source`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    #[default]
    Folder,
    Drive,
}

/// Persistent hardware/mount identity for a removable or mounted drive (`M2-S09`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceIdentity {
    /// Block device filesystem UUID (resolved via udev / `/dev/disk/by-uuid`, never mountinfo).
    FsUuid { fs_uuid: String },
    /// Filesystem without UUID (e.g. `nfs4`, `sshfs`, `tmpfs`).
    MountSource {
        mount_source: String,
        fs_type: String,
    },
}

/// Backup source directory/drive (glossary: `Source`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub path: PathBuf,
    #[serde(default)]
    pub kind: SourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<SourceIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_label: Option<String>,
}

impl Source {
    /// Creates a [`Source`] of kind [`SourceKind::Folder`].
    pub fn folder(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: SourceKind::Folder,
            identity: None,
            drive_label: None,
        }
    }

    /// Creates a [`Source`] of kind [`SourceKind::Drive`].
    pub fn drive(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: SourceKind::Drive,
            identity: None,
            drive_label: None,
        }
    }

    /// Creates a [`Source`] of kind [`SourceKind::Drive`] with detected identity & label (`M2-S09`).
    pub fn drive_with_identity(
        path: impl Into<PathBuf>,
        identity: Option<SourceIdentity>,
        drive_label: Option<String>,
    ) -> Self {
        Self {
            path: path.into(),
            kind: SourceKind::Drive,
            identity,
            drive_label,
        }
    }
}

/// Parsed single exclude rule entry (glossary: `ExcludeRule::Negated`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "pattern", rename_all = "snake_case")]
pub enum ExcludeRule {
    Pattern(String),
    Negated(String),
}

impl ExcludeRule {
    /// Parses a non-empty, non-comment line into an [`ExcludeRule`].
    pub fn parse_line(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }
        if let Some(rest) = trimmed.strip_prefix('!') {
            Some(Self::Negated(rest.to_string()))
        } else {
            Some(Self::Pattern(trimmed.to_string()))
        }
    }

    /// Formats the rule as a line for `--exclude-file`.
    pub fn to_file_line(&self) -> String {
        match self {
            Self::Pattern(pat) => pat.clone(),
            Self::Negated(pat) => format!("!{pat}"),
        }
    }
}

/// Standard desktop trash exclusion patterns emitted when `exclude_trash` is active (`M2-S02`).
pub const TRASH_EXCLUDE_PATTERNS: [&str; 2] = ["$HOME/.local/share/Trash", "**/.Trash-*"];

/// Exclude configuration for a [`Backup`] (glossary: `ExcludeRules`, `M2-S01`, `M2-S02`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExcludeRules {
    /// Simple-mode excluded folders (`$HOME/...` inside `$HOME`, `/...` outside `$HOME`, `P8`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
    /// Expert-mode raw exclude lines (`**`, `!negation`, `$VAR`, `--exclude-file` rules).
    pub lines: Vec<String>,
    /// Passed to `--exclude-caches`.
    pub exclude_caches: bool,
    /// Template: exclude user/per-volume Trash directories (`M2-S02`).
    pub exclude_trash: bool,
    /// Passed to `--exclude-larger-than <size>` (e.g. `"2G"`, `"500M"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_larger_than: Option<String>,
    /// Use `--iexclude-file` instead of `--exclude-file` (case-insensitive matching).
    pub ignore_case: bool,
    /// Passed via repeatable `--exclude-if-present <entry>` (e.g. `".nobackup"`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclude_if_present: Vec<String>,
}

/// Validates restic `--exclude-larger-than` size literal (digits + optional `k/K/m/M/g/G/t/T`).
pub fn validate_exclude_larger_than(raw: &str) -> Result<String, ModelError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ModelError::InvalidBackup(
            "exclude_larger_than must not be blank".to_string(),
        ));
    }
    let last = trimmed.chars().last().expect("non-empty");
    let digits = if matches!(last, 'k' | 'K' | 'm' | 'M' | 'g' | 'G' | 't' | 'T') {
        &trimmed[..trimmed.len() - last.len_utf8()]
    } else {
        trimmed
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ModelError::InvalidBackup(format!(
            "invalid exclude_larger_than size '{raw}'; expected e.g. 500M or 2G"
        )));
    }
    let num: u64 = digits.parse().map_err(|_| {
        ModelError::InvalidBackup(format!("exclude_larger_than number too large: '{raw}'"))
    })?;
    if num == 0 {
        return Err(ModelError::InvalidBackup(
            "exclude_larger_than must be greater than 0".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

/// Normalizes a simple folder exclude rule (`$HOME/...` when under `home_dir`, else absolute).
pub fn normalize_folder_rule(raw: &str, home_dir: Option<&Path>) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed == "$HOME" || trimmed.starts_with("$HOME/") {
        return trimmed.trim_end_matches('/').to_string();
    }

    let path = Path::new(trimmed);
    let norm = lexical_normalize(path);
    if let Some(home) = home_dir {
        let norm_home = lexical_normalize(home);
        if norm == norm_home {
            return "$HOME".to_string();
        }
        if let Ok(rel) = norm.strip_prefix(&norm_home) {
            return format!("$HOME/{}", rel.to_string_lossy());
        }
    }
    norm.to_string_lossy().into_owned()
}

impl ExcludeRules {
    /// Creates [`ExcludeRules`] from multi-line expert syntax text input.
    pub fn from_text(text: &str) -> Self {
        let lines = text.lines().map(|l| l.to_string()).collect();
        Self {
            folders: Vec::new(),
            lines,
            exclude_caches: false,
            exclude_trash: false,
            exclude_larger_than: None,
            ignore_case: false,
            exclude_if_present: Vec::new(),
        }
    }

    /// Parses all active [`ExcludeRule`] entries from `self.lines` (ignoring comments and blank lines).
    pub fn parsed_rules(&self) -> Vec<ExcludeRule> {
        self.lines
            .iter()
            .filter_map(|line| ExcludeRule::parse_line(line))
            .collect()
    }

    /// Returns the number of active expert-syntax rules (`M2-S02` `"N further rules in restic syntax"`).
    pub fn expert_rule_count(&self) -> usize {
        self.parsed_rules().len()
    }

    /// Returns `true` if any rule contributes a line to the generated `--exclude-file`.
    pub fn has_exclude_file_entries(&self) -> bool {
        self.exclude_trash
            || self.folders.iter().any(|f| !f.trim().is_empty())
            || self.lines.iter().any(|l| !l.trim().is_empty())
    }

    /// Normalizes all folder rules relative to `home_dir` (`P8`, `AC-M2-01-7`, `AC-M2-01-9`).
    pub fn normalize_folders(&mut self, home_dir: Option<&Path>) {
        let mut out = Vec::with_capacity(self.folders.len());
        for raw in &self.folders {
            let norm = normalize_folder_rule(raw, home_dir);
            if !norm.is_empty() && !out.contains(&norm) {
                out.push(norm);
            }
        }
        self.folders = out;
    }

    /// Serializes all exclude file entries using the current process `HOME`.
    pub fn to_exclude_file_contents(&self) -> String {
        let home = env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        self.to_exclude_file_contents_with_home(home.as_deref())
    }

    /// Serializes all exclude file entries for `$XDG_STATE_HOME/kestic/excludes/<backup-id>.txt`
    /// (`M2-S01`, `P8`, verbatim expert syntax without regex conversion).
    pub fn to_exclude_file_contents_with_home(&self, home_dir: Option<&Path>) -> String {
        let mut out = String::new();
        for folder in &self.folders {
            let norm = normalize_folder_rule(folder, home_dir);
            if !norm.is_empty() {
                out.push_str(&norm);
                out.push('\n');
            }
        }
        if self.exclude_trash {
            for pat in TRASH_EXCLUDE_PATTERNS {
                out.push_str(pat);
                out.push('\n');
            }
        }
        for line in &self.lines {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                out.push_str(trimmed);
                out.push('\n');
            }
        }
        out
    }

    /// Validates exclude options (`exclude_larger_than`, `folders`, `exclude_if_present`).
    pub fn validate(&self) -> Result<(), ModelError> {
        for f in &self.folders {
            if f.contains('\n') || f.contains('\r') {
                return Err(ModelError::InvalidBackup(
                    "exclude folder rule must not contain newlines".to_string(),
                ));
            }
        }
        if let Some(ref size) = self.exclude_larger_than
            && !size.trim().is_empty()
        {
            validate_exclude_larger_than(size)?;
        }
        for marker in &self.exclude_if_present {
            if marker.trim().is_empty() || marker.contains('\n') || marker.contains('\r') {
                return Err(ModelError::InvalidBackup(
                    "exclude_if_present filename must not be empty".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// Advisory warning emitted when a `!negation` rule is shadowed by a whole-directory exclusion (`M2-S03`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludeLintWarning {
    /// 1-based line number inside `ExcludeRules::lines`.
    pub line_number: usize,
    /// The shadowed negation rule (e.g. `"!dir/inner"`).
    pub rule: String,
    /// The parent rule that excludes the whole directory (e.g. `"dir"`).
    pub parent_rule: String,
    /// Suggested replacement for the parent rule (e.g. `"dir/*"`).
    pub suggested_fix: String,
}

fn strip_trailing_slash(s: &str) -> &str {
    if s == "/" { s } else { s.trim_end_matches('/') }
}

fn is_whole_dir_pattern(pat: &str) -> bool {
    let trimmed = pat.trim();
    if trimmed.is_empty() {
        return false;
    }
    let clean = strip_trailing_slash(trimmed);
    !clean.ends_with("/*") && !clean.ends_with("/**") && !clean.ends_with('\\')
}

fn pattern_components(pat: &str) -> Vec<&str> {
    let clean = strip_trailing_slash(pat.trim());
    clean
        .trim_start_matches('/')
        .split('/')
        .filter(|c| !c.is_empty())
        .collect()
}

fn is_ancestor_exclusion(parent_pat: &str, negated_pat: &str) -> bool {
    if !is_whole_dir_pattern(parent_pat) {
        return false;
    }
    let parent_anchored = parent_pat.trim().starts_with('/');
    let neg_anchored = negated_pat.trim().starts_with('/');
    let parent_comps = pattern_components(parent_pat);
    let neg_comps = pattern_components(negated_pat);
    if parent_comps.is_empty() || neg_comps.len() <= parent_comps.len() {
        return false;
    }

    if parent_anchored || parent_pat.contains('/') {
        if parent_anchored != neg_anchored {
            return false;
        }
        return neg_comps.starts_with(&parent_comps);
    }

    // Single-component unanchored parent pattern (e.g. `dir` excludes any `dir` subtree)
    let target = parent_comps[0];
    let has_wildcards = target.contains('*') || target.contains('?') || target.contains('[');
    if !has_wildcards {
        return neg_comps[..neg_comps.len() - 1].contains(&target);
    }
    false
}

/// Checks `rules` for ineffective `!` negation rules shadowed by whole-directory exclusions (`FR-27`, `M2-S03`).
/// Non-blocking: produces warnings only, never prevents saving (`AC-M2-03-3`).
pub fn lint_exclude_rules(rules: &ExcludeRules) -> Vec<ExcludeLintWarning> {
    let mut active_exclusions: Vec<(String, Option<String>)> = Vec::new();
    for folder in &rules.folders {
        let trimmed = folder.trim();
        if !trimmed.is_empty() {
            let clean = strip_trailing_slash(trimmed);
            active_exclusions.push((trimmed.to_string(), Some(format!("{clean}/*"))));
        }
    }

    let mut warnings = Vec::new();
    for (idx, raw_line) in rules.lines.iter().enumerate() {
        let line_no = idx + 1;
        let Some(parsed) = ExcludeRule::parse_line(raw_line) else {
            continue;
        };
        match parsed {
            ExcludeRule::Pattern(pat) => {
                let clean = strip_trailing_slash(pat.trim());
                let fix = if is_whole_dir_pattern(&pat) {
                    Some(format!("{clean}/*"))
                } else {
                    None
                };
                active_exclusions.push((pat, fix));
            }
            ExcludeRule::Negated(neg_pat) => {
                // Only consider exclusions active *before* or unconditionally via folders.
                if let Some((parent, Some(suggested_fix))) = active_exclusions
                    .iter()
                    .rev()
                    .find(|(p, _)| is_ancestor_exclusion(p, &neg_pat))
                {
                    let already_wildcarded = active_exclusions
                        .iter()
                        .any(|(p, _)| strip_trailing_slash(p) == suggested_fix);
                    if !already_wildcarded {
                        warnings.push(ExcludeLintWarning {
                            line_number: line_no,
                            rule: format!("!{neg_pat}"),
                            parent_rule: parent.clone(),
                            suggested_fix: suggested_fix.clone(),
                        });
                    }
                }
            }
        }
    }
    warnings
}

/// Result of testing a path against [`ExcludeRules`] (`[Pfad testen…]`, `M2-S02`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludePathTestResult {
    pub included: bool,
    pub matched_rule: Option<String>,
    /// 1-based line number in expert `lines` if matched by an expert rule.
    pub matched_line: Option<usize>,
    pub matched_source: String,
}

fn expand_home_var(pattern: &str, home_dir: Option<&Path>) -> String {
    if let Some(home) = home_dir {
        let home_str = home.to_string_lossy();
        pattern.replace("$HOME", &home_str)
    } else if let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty()) {
        pattern.replace("$HOME", &home.to_string_lossy())
    } else {
        pattern.to_string()
    }
}

fn glob_segment_matches(pat: &str, text: &str, ignore_case: bool) -> bool {
    let p_chars: Vec<char> = if ignore_case {
        pat.to_lowercase().chars().collect()
    } else {
        pat.chars().collect()
    };
    let t_chars: Vec<char> = if ignore_case {
        text.to_lowercase().chars().collect()
    } else {
        text.chars().collect()
    };
    glob_segment_rec(&p_chars, &t_chars)
}

fn glob_segment_rec(p: &[char], t: &[char]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    match p[0] {
        '*' => {
            for skip in 0..=t.len() {
                if glob_segment_rec(&p[1..], &t[skip..]) {
                    return true;
                }
            }
            false
        }
        '?' => !t.is_empty() && glob_segment_rec(&p[1..], &t[1..]),
        '[' => {
            if t.is_empty() {
                return false;
            }
            if let Some(close) = p[1..].iter().position(|&c| c == ']') {
                let class = &p[1..1 + close];
                let negate = class.first().is_some_and(|&c| c == '!' || c == '^');
                let inner = if negate { &class[1..] } else { class };
                let ch = t[0];
                let mut matched = false;
                let mut i = 0;
                while i < inner.len() {
                    if i + 2 < inner.len() && inner[i + 1] == '-' {
                        if inner[i] <= ch && ch <= inner[i + 2] {
                            matched = true;
                        }
                        i += 3;
                    } else {
                        if inner[i] == ch {
                            matched = true;
                        }
                        i += 1;
                    }
                }
                if negate {
                    matched = !matched;
                }
                matched && glob_segment_rec(&p[2 + close..], &t[1..])
            } else {
                t[0] == '[' && glob_segment_rec(&p[1..], &t[1..])
            }
        }
        c => !t.is_empty() && t[0] == c && glob_segment_rec(&p[1..], &t[1..]),
    }
}

fn glob_components_match(pat_comps: &[&str], path_comps: &[&str], ignore_case: bool) -> bool {
    if pat_comps.is_empty() {
        return path_comps.is_empty();
    }
    if pat_comps[0] == "**" {
        for skip in 0..=path_comps.len() {
            if glob_components_match(&pat_comps[1..], &path_comps[skip..], ignore_case) {
                return true;
            }
        }
        return false;
    }
    if path_comps.is_empty() {
        return false;
    }
    glob_segment_matches(pat_comps[0], path_comps[0], ignore_case)
        && glob_components_match(&pat_comps[1..], &path_comps[1..], ignore_case)
}

fn restic_pattern_matches(
    pattern: &str,
    path_comps: &[&str],
    match_ancestors: bool,
    ignore_case: bool,
) -> bool {
    let trimmed = strip_trailing_slash(pattern.trim());
    if trimmed.is_empty() {
        return false;
    }
    let pat_comps: Vec<&str> = trimmed
        .trim_start_matches('/')
        .split('/')
        .filter(|c| !c.is_empty())
        .collect();
    if pat_comps.is_empty() {
        return false;
    }

    let check_slice = |slice: &[&str]| -> bool {
        if pat_comps.len() == 1 && !trimmed.starts_with('/') && pat_comps[0] != "**" {
            // Unanchored single pattern matches any single basename component (when checking sub-slices)
            slice
                .last()
                .is_some_and(|seg| glob_segment_matches(pat_comps[0], seg, ignore_case))
        } else if trimmed.starts_with('/') {
            glob_components_match(&pat_comps, slice, ignore_case)
        } else {
            // Unanchored multi-component or wildcard pattern matches suffix OR full relative slice
            for start in 0..slice.len() {
                if glob_components_match(&pat_comps, &slice[start..], ignore_case) {
                    return true;
                }
            }
            false
        }
    };

    if check_slice(path_comps) {
        return true;
    }
    if match_ancestors {
        for prefix_len in 1..path_comps.len() {
            if check_slice(&path_comps[..prefix_len]) {
                return true;
            }
        }
    }
    false
}

/// Evaluates whether `test_path` would be backed up or excluded under `rules` (`M2-S02`).
pub fn evaluate_exclude_path(
    rules: &ExcludeRules,
    source_root: &Path,
    home_dir: Option<&Path>,
    test_path: &str,
) -> ExcludePathTestResult {
    let raw_test = test_path.trim();
    if raw_test.is_empty() {
        return ExcludePathTestResult {
            included: true,
            matched_rule: None,
            matched_line: None,
            matched_source: "empty".to_string(),
        };
    }

    let candidate_path = if Path::new(raw_test).is_absolute() {
        lexical_normalize(Path::new(raw_test))
    } else {
        lexical_normalize(&source_root.join(raw_test))
    };
    let candidate_str = candidate_path.to_string_lossy().into_owned();
    let abs_comps: Vec<&str> = candidate_str
        .trim_start_matches('/')
        .split('/')
        .filter(|c| !c.is_empty())
        .collect();

    let norm_src = lexical_normalize(source_root);
    let rel_str_owned = candidate_path
        .strip_prefix(&norm_src)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| candidate_str.clone());
    let rel_comps: Vec<&str> = rel_str_owned.split('/').filter(|c| !c.is_empty()).collect();

    let mut excluded = false;
    let mut matched_rule: Option<String> = None;
    let mut matched_line: Option<usize> = None;
    let mut matched_source = "none".to_string();
    let mut ancestor_blocked_by_whole_dir: Option<(String, Option<usize>, String)> = None;

    // 1. Folder rules (anchored patterns, act as whole-directory exclusions).
    for folder in &rules.folders {
        let norm_folder = normalize_folder_rule(folder, home_dir);
        if norm_folder.is_empty() {
            continue;
        }
        let expanded = expand_home_var(&norm_folder, home_dir);
        let comps = if expanded.starts_with('/') {
            &abs_comps
        } else {
            &rel_comps
        };
        if restic_pattern_matches(&expanded, comps, true, rules.ignore_case) {
            excluded = true;
            matched_rule = Some(folder.clone());
            matched_line = None;
            matched_source = "folder".to_string();
            ancestor_blocked_by_whole_dir = Some((folder.clone(), None, "folder".to_string()));
        }
    }

    // 2. Trash template patterns.
    if rules.exclude_trash {
        for pat in TRASH_EXCLUDE_PATTERNS {
            let expanded = expand_home_var(pat, home_dir);
            let comps = if expanded.starts_with('/') {
                &abs_comps
            } else {
                &rel_comps
            };
            if restic_pattern_matches(&expanded, comps, true, rules.ignore_case) {
                excluded = true;
                matched_rule = Some(pat.to_string());
                matched_line = None;
                matched_source = "trash".to_string();
            }
        }
    }

    // 3. Expert syntax lines (evaluated sequentially with line numbers).
    for (idx, raw_line) in rules.lines.iter().enumerate() {
        let line_no = idx + 1;
        let Some(parsed) = ExcludeRule::parse_line(raw_line) else {
            continue;
        };
        match parsed {
            ExcludeRule::Pattern(pat) => {
                let expanded = expand_home_var(&pat, home_dir);
                let comps = if expanded.starts_with('/') {
                    &abs_comps
                } else {
                    &rel_comps
                };
                // Check whether an ancestor directory (not just exact path) is wholly excluded.
                let whole_dir = is_whole_dir_pattern(&expanded);
                let hits_strict_ancestor = abs_comps.len() > 1
                    && whole_dir
                    && comps.len() > 1
                    && restic_pattern_matches(
                        &expanded,
                        &comps[..comps.len() - 1],
                        true,
                        rules.ignore_case,
                    );

                if restic_pattern_matches(&expanded, comps, whole_dir, rules.ignore_case) {
                    excluded = true;
                    matched_rule = Some(pat.clone());
                    matched_line = Some(line_no);
                    matched_source = "expert".to_string();
                    if hits_strict_ancestor {
                        ancestor_blocked_by_whole_dir =
                            Some((pat, Some(line_no), "expert".to_string()));
                    }
                }
            }
            ExcludeRule::Negated(neg) => {
                let expanded = expand_home_var(&neg, home_dir);
                let comps = if expanded.starts_with('/') {
                    &abs_comps
                } else {
                    &rel_comps
                };
                if restic_pattern_matches(&expanded, comps, false, rules.ignore_case) {
                    // If a parent dir was skipped via whole-dir exclusion, negation has no effect.
                    if let Some((ref blocker_rule, blocker_line, ref blocker_src)) =
                        ancestor_blocked_by_whole_dir
                    {
                        excluded = true;
                        matched_rule = Some(blocker_rule.clone());
                        matched_line = blocker_line;
                        matched_source = blocker_src.clone();
                    } else {
                        excluded = false;
                        matched_rule = Some(format!("!{neg}"));
                        matched_line = Some(line_no);
                        matched_source = "expert".to_string();
                    }
                }
            }
        }
    }

    ExcludePathTestResult {
        included: !excluded,
        matched_rule,
        matched_line,
        matched_source,
    }
}

/// Schedule definition (glossary: `Schedule`, expanded in M3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    #[default]
    ManualOnly,
    Hourly,
    Daily {
        hour: u8,
        minute: u8,
    },
    Weekly {
        weekday: u8,
        hour: u8,
        minute: u8,
    },
}

/// Conditions controlling when scheduled runs may start (glossary: `RunConditions`, M3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunConditions {
    pub skip_on_battery: bool,
    pub skip_on_metered_network: bool,
}

/// Count specifier for retention rules (`--keep-daily 7` or `--keep-yearly unlimited`, `AC-M2-05-1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepCount {
    Count(u32),
    Unlimited,
}

impl From<u32> for KeepCount {
    fn from(n: u32) -> Self {
        Self::Count(n)
    }
}

impl fmt::Display for KeepCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Count(n) => write!(f, "{n}"),
            Self::Unlimited => f.write_str("unlimited"),
        }
    }
}

impl Serialize for KeepCount {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Count(n) => serializer.serialize_u32(*n),
            Self::Unlimited => serializer.serialize_str("unlimited"),
        }
    }
}

impl<'de> Deserialize<'de> for KeepCount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeepCountVisitor;
        impl serde::de::Visitor<'_> for KeepCountVisitor {
            type Value = KeepCount;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a non-negative integer or \"unlimited\"")
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                let n = u32::try_from(v).map_err(|_| E::custom("keep count exceeds u32"))?;
                Ok(KeepCount::Count(n))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                if v < 0 {
                    return Err(E::custom("keep count cannot be negative"));
                }
                let n = u32::try_from(v).map_err(|_| E::custom("keep count exceeds u32"))?;
                Ok(KeepCount::Count(n))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if v.eq_ignore_ascii_case("unlimited") {
                    Ok(KeepCount::Unlimited)
                } else {
                    Err(E::custom(format!("expected \"unlimited\", got '{v}'")))
                }
            }
        }
        deserializer.deserialize_any(KeepCountVisitor)
    }
}

/// Built-in retention presets (`M2-S05`, `M2-S06`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPreset {
    Standard,
    Economical,
    Long,
    Custom,
}

/// Validates a restic `--keep-within*` duration (`y`, `m`, `d`, `h` chunks, e.g. `48h`, `30d`, `1y6m`).
pub fn validate_restic_duration(raw: &str) -> Result<String, ModelError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ModelError::InvalidBackup(
            "retention duration must not be empty".to_string(),
        ));
    }
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    let mut total_value: u64 = 0;
    while i < bytes.len() {
        let num_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == num_start || i >= bytes.len() {
            return Err(ModelError::InvalidBackup(format!(
                "invalid retention duration '{raw}': expected <number><y|m|d|h>"
            )));
        }
        let num_str = &trimmed[num_start..i];
        let val: u64 = num_str.parse().map_err(|_| {
            ModelError::InvalidBackup(format!("invalid number in duration '{raw}'"))
        })?;
        total_value = total_value.saturating_add(val);
        let unit = bytes[i];
        if !matches!(unit, b'y' | b'm' | b'd' | b'h') {
            return Err(ModelError::InvalidBackup(format!(
                "invalid duration unit '{}' in '{raw}': allowed units are y, m, d, h",
                unit as char
            )));
        }
        i += 1;
    }
    if total_value == 0 {
        return Err(ModelError::InvalidBackup(format!(
            "retention duration '{raw}' must be greater than 0"
        )));
    }
    Ok(trimmed.to_string())
}

/// Snapshot retention configuration (glossary: `RetentionPolicy`, `M2-S05`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default = "RetentionPolicy::empty")]
pub struct RetentionPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_last: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_hourly: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_daily: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_weekly: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_monthly: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_yearly: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within_hourly: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within_daily: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within_weekly: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within_monthly: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within_yearly: Option<String>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self::standard()
    }
}

impl RetentionPolicy {
    /// Empty retention policy (invalid until at least one rule is set, `AC-M2-05-4`).
    pub fn empty() -> Self {
        Self {
            keep_last: None,
            keep_hourly: None,
            keep_daily: None,
            keep_weekly: None,
            keep_monthly: None,
            keep_yearly: None,
            keep_within: None,
            keep_within_hourly: None,
            keep_within_daily: None,
            keep_within_weekly: None,
            keep_within_monthly: None,
            keep_within_yearly: None,
        }
    }

    /// Standard preset: 7 daily, 4 weekly, 12 monthly (`AC-M2-05-3`).
    pub fn standard() -> Self {
        Self {
            keep_daily: Some(KeepCount::Count(7)),
            keep_weekly: Some(KeepCount::Count(4)),
            keep_monthly: Some(KeepCount::Count(12)),
            ..Self::empty()
        }
    }

    /// Economical preset ("Sparsam"): 7 daily, 4 weekly (`M2-S06`).
    pub fn economical() -> Self {
        Self {
            keep_daily: Some(KeepCount::Count(7)),
            keep_weekly: Some(KeepCount::Count(4)),
            ..Self::empty()
        }
    }

    /// Long preset ("Lang"): 7 daily, 4 weekly, 12 monthly, unlimited yearly (`M2-S06`).
    pub fn long() -> Self {
        Self {
            keep_daily: Some(KeepCount::Count(7)),
            keep_weekly: Some(KeepCount::Count(4)),
            keep_monthly: Some(KeepCount::Count(12)),
            keep_yearly: Some(KeepCount::Unlimited),
            ..Self::empty()
        }
    }

    /// Identifies which preset this policy matches, or [`RetentionPreset::Custom`].
    pub fn preset(&self) -> RetentionPreset {
        if *self == Self::standard() {
            RetentionPreset::Standard
        } else if *self == Self::economical() {
            RetentionPreset::Economical
        } else if *self == Self::long() {
            RetentionPreset::Long
        } else {
            RetentionPreset::Custom
        }
    }

    /// Validates all retention counts/durations and ensures at least one active rule (`AC-M2-05-4`).
    pub fn validate(&self) -> Result<(), ModelError> {
        let counts = [
            ("keep_last", self.keep_last),
            ("keep_hourly", self.keep_hourly),
            ("keep_daily", self.keep_daily),
            ("keep_weekly", self.keep_weekly),
            ("keep_monthly", self.keep_monthly),
            ("keep_yearly", self.keep_yearly),
        ];
        let mut active_rules = 0usize;
        for (name, opt) in counts {
            if let Some(kc) = opt {
                match kc {
                    KeepCount::Count(0) => {
                        return Err(ModelError::InvalidBackup(format!(
                            "{name} must be at least 1 (or unset)"
                        )));
                    }
                    KeepCount::Count(_) | KeepCount::Unlimited => active_rules += 1,
                }
            }
        }

        let durations = [
            ("keep_within", &self.keep_within),
            ("keep_within_hourly", &self.keep_within_hourly),
            ("keep_within_daily", &self.keep_within_daily),
            ("keep_within_weekly", &self.keep_within_weekly),
            ("keep_within_monthly", &self.keep_within_monthly),
            ("keep_within_yearly", &self.keep_within_yearly),
        ];
        for (_name, opt) in durations {
            if let Some(raw) = opt {
                if !raw.trim().is_empty() {
                    validate_restic_duration(raw)?;
                    active_rules += 1;
                }
            }
        }

        if active_rules == 0 {
            return Err(ModelError::InvalidBackup(
                "retention policy must specify at least one active keep rule".to_string(),
            ));
        }
        Ok(())
    }

    /// Converts active retention rules into restic `forget` CLI flags.
    /// Never emits `--unsafe-allow-remove-all` (`AC-M2-05-5`, `FR-05`).
    pub fn to_forget_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        let count_flags = [
            ("--keep-last", self.keep_last),
            ("--keep-hourly", self.keep_hourly),
            ("--keep-daily", self.keep_daily),
            ("--keep-weekly", self.keep_weekly),
            ("--keep-monthly", self.keep_monthly),
            ("--keep-yearly", self.keep_yearly),
        ];
        for (flag, opt) in count_flags {
            if let Some(val) = opt {
                args.push(flag.to_string());
                args.push(val.to_string());
            }
        }

        let duration_flags = [
            ("--keep-within", &self.keep_within),
            ("--keep-within-hourly", &self.keep_within_hourly),
            ("--keep-within-daily", &self.keep_within_daily),
            ("--keep-within-weekly", &self.keep_within_weekly),
            ("--keep-within-monthly", &self.keep_within_monthly),
            ("--keep-within-yearly", &self.keep_within_yearly),
        ];
        for (flag, opt) in duration_flags {
            if let Some(raw) = opt {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    args.push(flag.to_string());
                    args.push(trimmed.to_string());
                }
            }
        }
        args
    }
}

/// Recovery kit metadata associated with a repository key (glossary: `RecoveryKit`, M6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryKit {
    pub destination_id: DestinationId,
    pub key_id: String,
    pub created_at: OffsetDateTime,
}

/// Metadata snapshot reference (`kestic-meta:<backup-id>`, glossary: `MetaSnapshot`, M6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaSnapshot {
    pub snapshot_id: String,
    pub backup_id: BackupId,
    pub created_at: OffsetDateTime,
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
        };
        backup.validate()?;
        Ok(backup)
    }

    /// Renames the backup while keeping its [`BackupId`] immutable (`AC-M1-01-2`).
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

    /// Updates the backup source while keeping its [`BackupId`] immutable (`AC-M1-01-2`).
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

    /// Assigns this backup to another [`DestinationId`] (`M1-S22`), keeping `self.id` stable.
    pub fn reassign_destination(&mut self, destination_id: DestinationId) {
        self.destination_id = destination_id;
    }

    /// Returns the restic snapshot tag identifying this backup (`kestic:<backup-id>`, `FR-28`).
    pub fn tag(&self) -> String {
        format!("kestic:{}", self.id)
    }

    /// Returns the restic metadata snapshot tag (`kestic-meta:<backup-id>`, `FR-49`).
    pub fn meta_tag(&self) -> String {
        format!("kestic-meta:{}", self.id)
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
        self.excludes.validate()?;
        self.retention.validate()?;
        Ok(())
    }
}

/// Relationship between two nested source paths on the same destination (`M2-S10`).
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

/// Warning entry for nested sources on the same destination (`FR-31`, `M2-S10`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NestedSourceConflict {
    pub other_backup_id: BackupId,
    pub other_backup_name: String,
    pub other_source_path: PathBuf,
    pub relationship: NestedRelationship,
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(seg) => out.push(seg),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

fn classify_component_nesting(a: &Path, b: &Path) -> Option<NestedRelationship> {
    let na = lexical_normalize(a);
    let nb = lexical_normalize(b);
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
/// (excluding `candidate_id` if editing an existing backup) (`FR-31`, `AC-M2-10-1..3`).
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
/// strictly per destination and using component-wise path matching) (`M2-S10`).
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

/// Kind of a [`Run`] (glossary: `RunKind`, `P38`, `AC-M1-01-3`).
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

/// Lifecycle state of a [`Run`] (`AC-M1-01-3`).
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

/// Record of an executed restic invocation for a [`Run`] (`FR-34`, `AC-M1-08-3`).
///
/// Must never contain plaintext secret values (`NFR-02`).
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

/// Single execution record (glossary: `Run`, `AC-M1-01-3`).
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
            log_path: None,
        })
    }

    /// Creates a new backup run belonging to `backup` and its `destination_id` (`AC-M1-01-3`).
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

    /// Creates a maintenance run belonging to `destination_id` (`AC-M1-01-3`).
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

    /// Creates a restore run belonging to `destination_id` and optionally a [`BackupId`] (`AC-M1-01-3`).
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

    /// Creates a metadata snapshot run belonging to `destination_id` (`AC-M1-01-3`).
    pub fn new_meta(clock: &dyn Clock, destination_id: DestinationId) -> Result<Self, ModelError> {
        Self::create(
            clock,
            RunKind::Meta,
            destination_id,
            RunStatus::Running,
            None,
        )
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
        let elapsed_ms = (now - self.started_at).whole_milliseconds().max(0) as u64;
        self.status = status;
        self.pending_reason = None;
        self.finished_at = Some(now);
        self.duration_ms = Some(elapsed_ms);
        self.exit_code = exit_code;
    }
}
