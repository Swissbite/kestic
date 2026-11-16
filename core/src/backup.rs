// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Backup execution with `kestic:<backup-id>` tags, `--group-by tags`, and exclude files
//! (`FR-28`, `F13`, `FR-30`, `M1-S07`).

use crate::clock::Clock;
use crate::destination::{DEFAULT_CONNECT_TIMEOUT, DestinationError, preflight_check_destination};
use crate::model::{Backup, BackupId, Destination, RunId, RunStatus};
use crate::provider::ResolvedRestic;
use crate::restic::{
    CancelHandle, ResticCommandSpec, ResticExecutionResult, ResticRunError, ResticStreamEvent,
    run_restic,
};
use crate::secrets::SecretStore;
use crate::snapshot::ResticSnapshot;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors occurring when preparing or running a backup or querying its snapshots.
#[derive(Debug, Error)]
pub enum BackupRunError {
    #[error("unable to determine XDG_STATE_HOME or HOME for state directory")]
    MissingStateDir,
    #[error("Quelle nicht verfügbar: {0}")]
    SourceUnavailable(String),
    #[error(transparent)]
    Destination(#[from] DestinationError),
    #[error(transparent)]
    Runner(#[from] ResticRunError),
    #[error("failed to parse snapshot JSON from restic: {0}")]
    SnapshotParse(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolves the canonical Kestic state directory (`$XDG_STATE_HOME/kestic`).
pub fn default_state_dir() -> Result<PathBuf, BackupRunError> {
    let base = if let Some(xdg) = env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(xdg)
    } else if let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(home).join(".local").join("state")
    } else {
        return Err(BackupRunError::MissingStateDir);
    };
    Ok(base.join("kestic"))
}

/// Returns `$XDG_STATE_HOME/kestic/excludes/<backup-id>.txt` (`F13`, `AC-M1-07-3`).
pub fn exclude_file_path(state_dir: &Path, backup_id: &BackupId) -> PathBuf {
    state_dir.join("excludes").join(format!("{backup_id}.txt"))
}

/// Atomically writes `backup.excludes` into `$XDG_STATE_HOME/kestic/excludes/<backup-id>.txt`.
pub fn write_exclude_file(state_dir: &Path, backup: &Backup) -> Result<PathBuf, BackupRunError> {
    let path = exclude_file_path(state_dir, &backup.id);
    let parent = path
        .parent()
        .expect("exclude file path always has parent directory");
    fs::create_dir_all(parent)?;

    let content = backup.excludes.to_exclude_file_contents();
    let nonce = RunId::generate().map_err(|e| std::io::Error::other(e.to_string()))?;
    let tmp = parent.join(format!(".{}.{nonce}.tmp", backup.id));

    let write_res = (|| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, &path)?;
        Ok(())
    })();
    if write_res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_res?;
    Ok(path)
}

/// Builds the [`ResticCommandSpec`] for running a backup (`FR-28`, `F13`, `FR-34`).
pub fn build_backup_command_spec(
    backup: &Backup,
    destination: &Destination,
    state_dir: &Path,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
) -> Result<ResticCommandSpec, BackupRunError> {
    let exclude_path = write_exclude_file(state_dir, backup)?;

    let mut args = vec![
        "backup".to_string(),
        "--tag".to_string(),
        backup.tag(),
        "--group-by".to_string(),
        "tags".to_string(),
    ];

    if !backup.excludes.lines.is_empty() {
        args.push("--exclude-file".to_string());
        args.push(exclude_path.to_string_lossy().into_owned());
    }
    if backup.excludes.exclude_caches {
        args.push("--exclude-caches".to_string());
    }

    args.push(backup.source.path.to_string_lossy().into_owned());

    Ok(ResticCommandSpec::for_destination(
        &resolved.binary,
        kestic_binary,
        destination,
        args,
    ))
}

/// Checks whether the backup source is currently available (`FR-29`, `FR-30`).
pub fn check_source_available(backup: &Backup) -> Result<(), BackupRunError> {
    if !backup.source.path.exists() {
        return Err(BackupRunError::SourceUnavailable(format!(
            "Quellpfad existiert nicht: {}",
            backup.source.path.display()
        )));
    }
    Ok(())
}

/// Executes a backup run with tag `kestic:<backup-id>`, `--group-by tags`, and exclude file (`M1-S07`).
#[allow(clippy::too_many_arguments)]
pub fn execute_backup<F>(
    backup: &Backup,
    destination: &Destination,
    state_dir: &Path,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
    secret_store: &dyn SecretStore,
    clock: &dyn Clock,
    cancel: Option<&CancelHandle>,
    extra_env: &[(&str, &str)],
    on_event: F,
) -> Result<ResticExecutionResult, BackupRunError>
where
    F: FnMut(&ResticStreamEvent) + Send,
{
    check_source_available(backup)?;
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;

    let spec = build_backup_command_spec(backup, destination, state_dir, resolved, kestic_binary)?;
    let res = run_restic(
        &spec,
        destination,
        Some(secret_store),
        clock,
        cancel,
        extra_env,
        on_event,
    )?;
    Ok(res)
}

/// Lists all snapshots in `destination` belonging strictly to `backup_id` (`kestic:<backup-id>`, `AC-M1-07-4`).
pub fn list_snapshots_for_backup(
    backup_id: &BackupId,
    destination: &Destination,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
    secret_store: &dyn SecretStore,
    clock: &dyn Clock,
    extra_env: &[(&str, &str)],
) -> Result<Vec<ResticSnapshot>, BackupRunError> {
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;

    let mut spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        kestic_binary,
        destination,
        vec![
            "snapshots".to_string(),
            "--no-lock".to_string(),
            "--tag".to_string(),
            format!("kestic:{backup_id}"),
        ],
    );
    spec.low_priority = false;

    let res = run_restic(
        &spec,
        destination,
        Some(secret_store),
        clock,
        None,
        extra_env,
        |_| {},
    )?;

    if res.presentation.status == RunStatus::Error {
        return Err(BackupRunError::Destination(
            DestinationError::CommandFailed {
                exit_code: res.exit_code,
                message: res
                    .presentation
                    .raw_detail
                    .unwrap_or(res.presentation.summary_de),
            },
        ));
    }

    // `restic snapshots --json` outputs a JSON array line.
    for line in &res.log_lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            return Ok(serde_json::from_str(trimmed)?);
        }
    }
    Ok(Vec::new())
}
