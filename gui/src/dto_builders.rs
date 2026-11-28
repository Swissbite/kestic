// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backups and restic sources as the GUI builds them from its fields before sending them to
//! the daemon, and JSON for QML.

use crate::i18n::i18n;
use kestic_core::backup::{default_state_dir, exclude_file_path};
use kestic_core::model::{
    Backup, BackupId, DestinationId, DestinationKind, ExcludeRules, ResticProvider,
    RetentionPolicy, RunConditions, Schedule, Source,
};
use std::path::PathBuf;

/// `value` as JSON text for QML; `fallback` when it cannot be serialised.
pub fn json_or<T: serde::Serialize + ?Sized>(value: &T, fallback: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| fallback.to_string())
}

/// New backup as the wizard creates it; on a local destination it has no network run
/// conditions.
pub fn build_new_backup(
    name: &str,
    source_path: &str,
    dest_id: &str,
    excludes_text: &str,
    detected_source: Option<Source>,
    dest_kind: Option<&DestinationKind>,
) -> Result<Backup, String> {
    let d_id = DestinationId::parse(dest_id.trim()).map_err(|e| e.to_string())?;
    let source =
        detected_source.unwrap_or_else(|| Source::folder(PathBuf::from(source_path.trim())));
    let mut backup = Backup::new(name.trim(), source, d_id).map_err(|e| e.to_string())?;
    if let Some(kind) = dest_kind {
        backup.run_conditions = backup.run_conditions.clone().for_destination(kind);
    }
    backup.schedule = Schedule::default_scheduled();
    backup.excludes = ExcludeRules::from_text(excludes_text);
    Ok(backup)
}

pub fn clone_updated_backup_excludes(
    existing: &Backup,
    excludes: ExcludeRules,
) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.excludes = excludes;
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
}

pub fn clone_updated_backup_retention(
    existing: &Backup,
    retention: RetentionPolicy,
) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.retention = retention;
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
}

pub fn clone_updated_backup_source(existing: &Backup, source: Source) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.set_source(source).map_err(|e| e.to_string())?;
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
}

pub fn clone_updated_backup_schedule(
    existing: &Backup,
    schedule: Schedule,
    overdue_threshold: Option<u32>,
) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.schedule = schedule;
    updated.overdue_threshold = overdue_threshold;
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
}

/// Backup with the edited run conditions; network conditions are dropped for a local
/// destination.
pub fn clone_updated_backup_conditions(
    existing: &Backup,
    run_conditions: RunConditions,
    dest_kind: Option<&DestinationKind>,
) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.run_conditions = match dest_kind {
        Some(kind) => run_conditions.for_destination(kind),
        None => run_conditions,
    };
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
}

pub fn backup_exclude_file_path_str(backup_id: &BackupId) -> String {
    let state_dir = default_state_dir().unwrap_or_else(|_| PathBuf::from("~/.local/state/kestic"));
    exclude_file_path(&state_dir, backup_id)
        .to_string_lossy()
        .into_owned()
}

pub fn build_provider(kind: &str, custom_path: &str) -> Result<ResticProvider, String> {
    match kind {
        "custom" => {
            let trimmed = custom_path.trim();
            if trimmed.is_empty() {
                return Err(i18n("Custom restic path must not be empty.").to_string());
            }
            Ok(ResticProvider::Custom {
                path: PathBuf::from(trimmed),
            })
        }
        "automatic" => Ok(ResticProvider::Automatic),
        "app_managed" => Ok(ResticProvider::AppManaged),
        "system" => Ok(ResticProvider::System),
        other => Err(format!("unknown restic source '{other}'")),
    }
}

#[cfg(test)]
#[path = "dto_builders_tests.rs"]
mod tests;
