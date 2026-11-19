// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! TOML configuration persistence with `config_version` checks (`ADR-0005`, `F13`, `NFR-03`, `M1-S02`).

use crate::model::{
    Backup, BackupId, DEFAULT_OVERDUE_THRESHOLD, Destination, DestinationId, DestinationKind,
    ModelError, NotificationSettings, ResticProvider, RunId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Highest configuration schema version supported by this build of Kestic.
pub const CURRENT_CONFIG_VERSION: u32 = 1;

fn default_overdue_threshold() -> u32 {
    DEFAULT_OVERDUE_THRESHOLD
}

/// Errors occurring while locating, loading, validating, or saving `config.toml`.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("unable to determine XDG_CONFIG_HOME or HOME for config.toml")]
    MissingConfigDir,
    #[error(
        "configuration version {found} is newer than supported version {max_supported}; refusing to load or overwrite"
    )]
    UnsupportedVersion { found: u32, max_supported: u32 },
    #[error("configuration file is missing required `config_version` field")]
    MissingVersion,
    #[error("invalid configuration: {0}")]
    Validation(String),
    #[error("invalid model data in configuration: {0}")]
    Model(#[from] ModelError),
    #[error("failed to parse TOML configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize TOML configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("I/O error accessing configuration: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Deserialize)]
struct ConfigHeader {
    config_version: Option<u32>,
}

/// Top-level persisted configuration (`$XDG_CONFIG_HOME/kestic/config.toml`).
///
/// Must never store secret values — only Secret Service references (`NFR-02`, `NFR-03`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub config_version: u32,
    #[serde(default)]
    pub restic_provider: ResticProvider,
    #[serde(default)]
    pub expert_mode: bool,
    #[serde(default = "default_overdue_threshold")]
    pub default_overdue_threshold: u32,
    #[serde(default)]
    pub notifications: NotificationSettings,
    #[serde(default)]
    pub destinations: Vec<Destination>,
    #[serde(default)]
    pub backups: Vec<Backup>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            config_version: CURRENT_CONFIG_VERSION,
            restic_provider: ResticProvider::default(),
            expert_mode: false,
            default_overdue_threshold: DEFAULT_OVERDUE_THRESHOLD,
            notifications: NotificationSettings::default(),
            destinations: Vec::new(),
            backups: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Validates cross-entity invariants across destinations and backups.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.config_version == 0 {
            return Err(ConfigError::Validation(
                "config_version must be >= 1".to_string(),
            ));
        }
        if self.config_version > CURRENT_CONFIG_VERSION {
            return Err(ConfigError::UnsupportedVersion {
                found: self.config_version,
                max_supported: CURRENT_CONFIG_VERSION,
            });
        }
        if self.default_overdue_threshold == 0 {
            return Err(ConfigError::Validation(
                "default_overdue_threshold must be >= 1".to_string(),
            ));
        }

        let mut dest_ids = BTreeSet::new();
        for dest in &self.destinations {
            dest.validate()?;
            if !dest_ids.insert(&dest.id) {
                return Err(ConfigError::Validation(format!(
                    "duplicate destination id '{}'",
                    dest.id
                )));
            }
        }

        let mut backup_ids = BTreeSet::new();
        for backup in &self.backups {
            backup.validate()?;
            if !backup_ids.insert(&backup.id) {
                return Err(ConfigError::Validation(format!(
                    "duplicate backup id '{}'",
                    backup.id
                )));
            }
            if !dest_ids.contains(&backup.destination_id) {
                return Err(ConfigError::Validation(format!(
                    "backup '{}' references unknown destination '{}'",
                    backup.id, backup.destination_id
                )));
            }
        }

        Ok(())
    }

    /// Looks up a [`Destination`] by ID.
    pub fn destination(&self, id: &DestinationId) -> Option<&Destination> {
        self.destinations.iter().find(|d| &d.id == id)
    }

    /// Finds another configured [`Destination`] targeting the same repository location as `kind`,
    /// optionally excluding `exclude_id` (for editing an existing destination).
    pub fn find_duplicate_destination(
        &self,
        kind: &DestinationKind,
        exclude_id: Option<&DestinationId>,
    ) -> Option<&Destination> {
        self.destinations
            .iter()
            .find(|d| exclude_id != Some(&d.id) && d.kind.same_location(kind))
    }

    /// Looks up a [`Backup`] by ID.
    pub fn backup(&self, id: &BackupId) -> Option<&Backup> {
        self.backups.iter().find(|b| &b.id == id)
    }

    /// Removes and returns the [`Backup`] with `id`, or returns an error if not found (`M1-S19`).
    pub fn remove_backup(&mut self, id: &BackupId) -> Result<Backup, ConfigError> {
        let Some(pos) = self.backups.iter().position(|b| &b.id == id) else {
            return Err(ConfigError::Validation(format!("backup '{id}' not found")));
        };
        Ok(self.backups.remove(pos))
    }

    /// Removes and returns the [`Destination`] with `id` if no [`Backup`] references it (`AC-M1-19-4`).
    ///
    /// If any backup references `id`, returns [`ConfigError::Validation`] listing the referencing
    /// backup names and IDs without modifying `self`.
    pub fn remove_destination(&mut self, id: &DestinationId) -> Result<Destination, ConfigError> {
        let referencing: Vec<String> = self
            .backups
            .iter()
            .filter(|b| &b.destination_id == id)
            .map(|b| format!("{} ({})", b.name, b.id))
            .collect();
        if !referencing.is_empty() {
            return Err(ConfigError::Validation(format!(
                "destination '{id}' is still referenced by backup(s): {}",
                referencing.join(", ")
            )));
        }
        let Some(pos) = self.destinations.iter().position(|d| &d.id == id) else {
            return Err(ConfigError::Validation(format!(
                "destination '{id}' not found"
            )));
        };
        Ok(self.destinations.remove(pos))
    }

    /// Reassigns `backup_id` to `new_dest_id` (`M1-S22`), keeping `backup.id` unchanged.
    pub fn reassign_backup(
        &mut self,
        backup_id: &BackupId,
        new_dest_id: &DestinationId,
    ) -> Result<(), ConfigError> {
        if self.destination(new_dest_id).is_none() {
            return Err(ConfigError::Validation(format!(
                "destination '{new_dest_id}' not found"
            )));
        }
        let Some(backup) = self.backups.iter_mut().find(|b| &b.id == backup_id) else {
            return Err(ConfigError::Validation(format!(
                "backup '{backup_id}' not found"
            )));
        };
        backup.reassign_destination(new_dest_id.clone());
        Ok(())
    }
}

/// Formats a single [`Backup`] configuration as pretty TOML without secrets (`§8.11`).
pub fn backup_toml_snippet(backup: &crate::model::Backup) -> String {
    toml::to_string_pretty(backup).unwrap_or_default()
}

/// Resolves the canonical configuration file path (`$XDG_CONFIG_HOME/kestic/config.toml`).
pub fn default_config_path() -> Result<PathBuf, ConfigError> {
    let base = if let Some(xdg) = env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(xdg)
    } else if let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(home).join(".config")
    } else {
        return Err(ConfigError::MissingConfigDir);
    };
    Ok(base.join("kestic").join("config.toml"))
}

/// Parses and validates a TOML configuration string.
pub fn parse_config_str(raw: &str) -> Result<AppConfig, ConfigError> {
    let header: ConfigHeader = toml::from_str(raw)?;
    let version = header.config_version.ok_or(ConfigError::MissingVersion)?;
    if version > CURRENT_CONFIG_VERSION {
        return Err(ConfigError::UnsupportedVersion {
            found: version,
            max_supported: CURRENT_CONFIG_VERSION,
        });
    }
    let config: AppConfig = toml::from_str(raw)?;
    config.validate()?;
    Ok(config)
}

/// Loads and validates `config.toml` from `path`.
///
/// Rejects files with `config_version > CURRENT_CONFIG_VERSION` without modifying them (`AC-M1-02-3`).
pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
    let raw = fs::read_to_string(path)?;
    parse_config_str(&raw)
}

/// Loads `config.toml` if it exists, returning `Ok(None)` if the file does not exist.
pub fn load_optional_config(path: &Path) -> Result<Option<AppConfig>, ConfigError> {
    match fs::read_to_string(path) {
        Ok(raw) => parse_config_str(&raw).map(Some),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(ConfigError::Io(err)),
    }
}

/// Validates and atomically writes `config` as TOML to `path`.
pub fn save_config(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    config.validate()?;
    let serialized = toml::to_string_pretty(config)?;

    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let nonce = RunId::generate()?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.toml");
    let tmp_path = parent.join(format!(".{file_name}.{nonce}.tmp"));

    let write_res = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(serialized.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, path)?;
        Ok(())
    })();

    if write_res.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_res?;
    Ok(())
}
