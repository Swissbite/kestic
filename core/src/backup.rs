// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backup execution with `kestic:<backup-id>` tags, `--group-by tags`, exclude files,
//! dry-run previews, and retention (`forget`) execution.

use crate::clock::Clock;
use crate::destination::{DEFAULT_CONNECT_TIMEOUT, DestinationError, preflight_check_destination};
use crate::model::{
    Backup, BackupId, Destination, ModelError, RunId, RunStatus, evaluate_exclude_path,
};
use crate::mount::{MountInspector, SourceCheckResult, check_source_status_with, current_home_dir};
use crate::provider::ResolvedRestic;
use crate::restic::{
    CancelHandle, ResticCommandSpec, ResticExecutionResult, ResticFailureKind, ResticRunError,
    ResticRuntime, ResticStreamEvent, run_restic,
};
use crate::secrets::SecretStore;
use crate::snapshot::{ResticSnapshot, deserialize_null_default};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Errors occurring when preparing or running a backup, dry-run, or `forget`.
#[derive(Debug, Error)]
pub enum BackupRunError {
    #[error("unable to determine XDG_STATE_HOME or HOME for state directory")]
    MissingStateDir,
    #[error("source unavailable: {0:?}")]
    SourceUnavailable(SourceCheckResult),
    #[error(transparent)]
    InvalidModel(#[from] ModelError),
    #[error(transparent)]
    Destination(#[from] DestinationError),
    #[error(transparent)]
    Runner(#[from] ResticRunError),
    #[error("failed to parse JSON from restic: {0}")]
    SnapshotParse(#[from] serde_json::Error),
    #[error("operation cancelled")]
    Cancelled,
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

/// Returns `$XDG_STATE_HOME/kestic/excludes/<backup-id>.txt`.
pub fn exclude_file_path(state_dir: &Path, backup_id: &BackupId) -> PathBuf {
    state_dir.join("excludes").join(format!("{backup_id}.txt"))
}

fn write_exclude_file_at(
    path: &Path,
    backup: &Backup,
    home_dir: Option<&Path>,
) -> Result<(), BackupRunError> {
    let parent = path
        .parent()
        .expect("exclude file path always has parent directory");
    fs::create_dir_all(parent)?;

    let content = backup.excludes.to_exclude_file_contents_with_home(home_dir);
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
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if write_res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_res?;
    Ok(())
}

struct PreviewExcludeGuard(Option<PathBuf>);

impl Drop for PreviewExcludeGuard {
    fn drop(&mut self) {
        if let Some(ref path) = self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

/// Atomically writes `backup.excludes` into `$XDG_STATE_HOME/kestic/excludes/<backup-id>.txt`
/// using explicit `home_dir`.
pub fn write_exclude_file_with_home(
    state_dir: &Path,
    backup: &Backup,
    home_dir: Option<&Path>,
) -> Result<PathBuf, BackupRunError> {
    let path = exclude_file_path(state_dir, &backup.id);
    write_exclude_file_at(&path, backup, home_dir)?;
    Ok(path)
}

/// Atomically writes `backup.excludes` into `$XDG_STATE_HOME/kestic/excludes/<backup-id>.txt`.
pub fn write_exclude_file(state_dir: &Path, backup: &Backup) -> Result<PathBuf, BackupRunError> {
    let home = current_home_dir();
    write_exclude_file_with_home(state_dir, backup, home.as_deref())
}

fn build_backup_command_spec_for_exclude_path(
    backup: &Backup,
    destination: &Destination,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
    exclude_path: &Path,
    dry_run: bool,
) -> ResticCommandSpec {
    let mut args = vec![
        "backup".to_string(),
        "--tag".to_string(),
        backup.tag(),
        "--group-by".to_string(),
        "tags".to_string(),
    ];

    if backup.excludes.has_exclude_file_entries() {
        if backup.excludes.ignore_case {
            args.push("--iexclude-file".to_string());
        } else {
            args.push("--exclude-file".to_string());
        }
        args.push(exclude_path.to_string_lossy().into_owned());
    }
    if backup.excludes.exclude_caches {
        args.push("--exclude-caches".to_string());
    }
    if let Some(ref larger) = backup.excludes.exclude_larger_than {
        let trimmed = larger.trim();
        if !trimmed.is_empty() {
            args.push("--exclude-larger-than".to_string());
            args.push(trimmed.to_string());
        }
    }
    for marker in &backup.excludes.exclude_if_present {
        let trimmed = marker.trim();
        if !trimmed.is_empty() {
            args.push("--exclude-if-present".to_string());
            args.push(trimmed.to_string());
        }
    }
    if dry_run {
        args.push("--dry-run".to_string());
        args.push("-vv".to_string());
    }

    args.push("--".to_string());
    args.push(backup.source.path.to_string_lossy().into_owned());

    ResticCommandSpec::for_destination(&resolved.binary, kestic_binary, destination, args)
}

/// Builds the [`ResticCommandSpec`] for running a backup.
pub fn build_backup_command_spec_with_home(
    backup: &Backup,
    destination: &Destination,
    state_dir: &Path,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
    home_dir: Option<&Path>,
) -> Result<ResticCommandSpec, BackupRunError> {
    backup.excludes.validate()?;
    let exclude_path = write_exclude_file_with_home(state_dir, backup, home_dir)?;
    Ok(build_backup_command_spec_for_exclude_path(
        backup,
        destination,
        resolved,
        kestic_binary,
        &exclude_path,
        false,
    ))
}

/// Builds the [`ResticCommandSpec`] for running a backup.
pub fn build_backup_command_spec(
    backup: &Backup,
    destination: &Destination,
    state_dir: &Path,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
) -> Result<ResticCommandSpec, BackupRunError> {
    let home = current_home_dir();
    build_backup_command_spec_with_home(
        backup,
        destination,
        state_dir,
        resolved,
        kestic_binary,
        home.as_deref(),
    )
}

/// Checks whether the backup source is currently available using a custom [`MountInspector`].
pub fn check_source_available_with(
    backup: &Backup,
    home_dir: Option<&Path>,
    inspector: &dyn MountInspector,
) -> Result<(), BackupRunError> {
    match check_source_status_with(backup, home_dir, inspector) {
        SourceCheckResult::Available => Ok(()),
        other => Err(BackupRunError::SourceUnavailable(other)),
    }
}

/// Execution context for running or dry-running a backup with an explicit [`MountInspector`].
pub struct BackupExecutionContext<'a> {
    pub state_dir: &'a Path,
    pub home_dir: Option<&'a Path>,
    pub inspector: &'a dyn MountInspector,
    pub cancel: Option<&'a CancelHandle>,
    pub extra_env: &'a [(&'a str, &'a str)],
}

/// Turns a restic run that ended in an error into the matching [`BackupRunError`].
pub(crate) fn check_command_succeeded(res: &ResticExecutionResult) -> Result<(), BackupRunError> {
    if res.presentation.status == RunStatus::Error {
        return Err(BackupRunError::Destination(
            DestinationError::CommandFailed {
                exit_code: res.exit_code,
                message: res.presentation.detail_or_summary(),
            },
        ));
    }
    Ok(())
}

/// Executes a backup run using explicit [`BackupExecutionContext`].
pub fn execute_backup_with_inspector<F>(
    backup: &Backup,
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    ctx: &BackupExecutionContext<'_>,
    on_event: F,
) -> Result<ResticExecutionResult, BackupRunError>
where
    F: FnMut(&ResticStreamEvent) + Send,
{
    check_source_available_with(backup, ctx.home_dir, ctx.inspector)?;
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;

    let spec = build_backup_command_spec_with_home(
        backup,
        destination,
        ctx.state_dir,
        runtime.resolved,
        runtime.kestic_binary,
        ctx.home_dir,
    )?
    .with_cache_dir(runtime.cache_dir);
    let res = runtime.run(&spec, destination, ctx.cancel, ctx.extra_env, on_event)?;
    Ok(res)
}

/// Size estimate for one excluded folder reported alongside backup dry-run preview.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludedFolderEstimate {
    pub path: String,
    pub bytes: u64,
}

/// Result of `restic backup --dry-run -vv --json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupDryRunResult {
    pub total_files: u64,
    pub total_bytes: u64,
    pub files_new: u64,
    pub files_changed: u64,
    pub files_unmodified: u64,
    pub data_added: u64,
    pub files: Vec<String>,
    pub largest_excluded_folders: Vec<ExcludedFolderEstimate>,
    pub cancelled: bool,
    pub transparent_command: String,
}

fn dir_size_bytes(dir: &Path, budget: &mut usize) -> u64 {
    if *budget == 0 {
        return 0;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut sum = 0u64;
    for entry in entries.flatten() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_file() {
            sum = sum.saturating_add(meta.len());
        } else if meta.is_dir() {
            sum = sum.saturating_add(dir_size_bytes(&entry.path(), budget));
        }
    }
    sum
}

/// Estimates largest excluded subdirectories inside `backup.source.path`.
pub fn compute_largest_excluded_folders(
    backup: &Backup,
    home_dir: Option<&Path>,
    max_results: usize,
) -> Vec<ExcludedFolderEstimate> {
    let root = &backup.source.path;
    if !root.is_dir() || max_results == 0 {
        return Vec::new();
    }
    let mut estimates: BTreeMap<String, u64> = BTreeMap::new();
    let mut stack = vec![root.clone()];
    let mut visited_dirs = 0usize;
    let mut size_budget = 25_000usize;

    while let Some(dir) = stack.pop() {
        if visited_dirs > 5_000 || size_budget == 0 {
            break;
        }
        visited_dirs += 1;
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.file_type().is_symlink() || !meta.is_dir() {
                continue;
            }
            let p = entry.path();
            let p_str = p.to_string_lossy();
            let eval = evaluate_exclude_path(&backup.excludes, root, home_dir, &p_str);
            if !eval.included {
                let bytes = dir_size_bytes(&p, &mut size_budget);
                estimates.insert(p_str.into_owned(), bytes);
            } else {
                stack.push(p);
            }
        }
    }

    let mut list: Vec<ExcludedFolderEstimate> = estimates
        .into_iter()
        .map(|(path, bytes)| ExcludedFolderEstimate { path, bytes })
        .collect();
    list.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    list.truncate(max_results);
    list
}

/// Executes `restic backup --dry-run -vv --json` with identical backup arguments.
/// Writes any draft excludes to an isolated temporary preview file that is removed on completion.
pub fn execute_backup_dry_run_with<F>(
    backup: &Backup,
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    ctx: &BackupExecutionContext<'_>,
    mut on_event: F,
) -> Result<BackupDryRunResult, BackupRunError>
where
    F: FnMut(&ResticStreamEvent) + Send,
{
    check_source_available_with(backup, ctx.home_dir, ctx.inspector)?;
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;
    backup.excludes.validate()?;

    let preview_nonce = RunId::generate().map_err(|e| std::io::Error::other(e.to_string()))?;
    let preview_exclude_path = ctx
        .state_dir
        .join("excludes")
        .join(format!("{}.preview-{preview_nonce}.txt", backup.id));
    let _preview_guard = if backup.excludes.has_exclude_file_entries() {
        write_exclude_file_at(&preview_exclude_path, backup, ctx.home_dir)?;
        PreviewExcludeGuard(Some(preview_exclude_path.clone()))
    } else {
        PreviewExcludeGuard(None)
    };

    let mut spec = build_backup_command_spec_for_exclude_path(
        backup,
        destination,
        runtime.resolved,
        runtime.kestic_binary,
        &preview_exclude_path,
        true,
    )
    .with_cache_dir(runtime.cache_dir);
    spec.low_priority = false;

    let transparent_command = format!(
        "{} {}",
        spec.restic_binary.display(),
        spec.subcommand_args.join(" ")
    );

    let mut files = Vec::new();
    let mut scan_total_files = 0u64;
    let mut scan_total_bytes = 0u64;

    let res = runtime.run(&spec, destination, ctx.cancel, ctx.extra_env, |ev| {
        if let ResticStreamEvent::VerboseStatus(vs) = ev {
            if vs.action == "scan_finished" {
                if let Some(tf) = vs.total_files {
                    scan_total_files = tf;
                }
                if let Some(ds) = vs.data_size {
                    scan_total_bytes = ds;
                }
            } else if !vs.item.is_empty() && !vs.item.ends_with('/') {
                files.push(vs.item.clone());
            }
        }
        on_event(ev);
    })?;

    if res.presentation.status == RunStatus::Cancelled {
        return Ok(BackupDryRunResult {
            cancelled: true,
            transparent_command,
            ..Default::default()
        });
    }

    check_command_succeeded(&res)?;

    let (total_files, total_bytes, files_new, files_changed, files_unmodified, data_added) =
        if let Some(stats) = &res.stats {
            (
                stats.total_files_processed,
                stats.total_bytes_processed,
                stats.files_new,
                stats.files_changed,
                stats.files_unmodified,
                stats.data_added,
            )
        } else {
            (
                scan_total_files.max(files.len() as u64),
                scan_total_bytes,
                files.len() as u64,
                0,
                0,
                0,
            )
        };

    let largest_excluded_folders = compute_largest_excluded_folders(backup, ctx.home_dir, 10);

    Ok(BackupDryRunResult {
        total_files,
        total_bytes,
        files_new,
        files_changed,
        files_unmodified,
        data_added,
        files,
        largest_excluded_folders,
        cancelled: false,
        transparent_command,
    })
}

/// Builds a [`ResticCommandSpec`] for `restic forget` scoped to `backup.tag()`.
/// Never passes `--unsafe-allow-remove-all`.
/// Always passes `--no-lock` when `dry_run` / `no_lock` is requested.
pub fn build_forget_command_spec(
    backup: &Backup,
    destination: &Destination,
    dry_run: bool,
    no_lock: bool,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
) -> Result<ResticCommandSpec, BackupRunError> {
    backup.retention.validate()?;

    let mut args = vec![
        "forget".to_string(),
        "--tag".to_string(),
        backup.tag(),
        "--group-by".to_string(),
        "tags".to_string(),
    ];
    args.extend(backup.retention.to_forget_args());
    if dry_run {
        args.push("--dry-run".to_string());
    }
    if no_lock {
        args.push("--no-lock".to_string());
    }

    let mut spec =
        ResticCommandSpec::for_destination(&resolved.binary, kestic_binary, destination, args);
    spec.low_priority = !dry_run;
    Ok(spec)
}

/// Single snapshot verdict (keep or remove) in `restic forget --dry-run --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetPreviewSnapshot {
    pub snapshot_id: String,
    pub short_id: String,
    pub time: String,
    pub hostname: String,
    pub paths: Vec<String>,
    /// `"keep"` or `"remove"`.
    pub action: String,
    /// Matched reasons from restic (`Reasons`), e.g. `"daily snapshot"`.
    pub reasons: Vec<String>,
}

/// Parsed preview of `restic forget --dry-run --no-lock --json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetPreviewResult {
    pub backup_id: String,
    pub snapshots: Vec<ForgetPreviewSnapshot>,
    pub keep_count: usize,
    pub remove_count: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawForgetReason {
    snapshot: Option<ResticSnapshot>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    matches: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawForgetGroup {
    #[serde(default, deserialize_with = "deserialize_null_default")]
    keep: Vec<ResticSnapshot>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    remove: Vec<ResticSnapshot>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    reasons: Vec<RawForgetReason>,
}

fn forget_preview_snapshot(
    snap: ResticSnapshot,
    action: &str,
    reasons: Vec<String>,
) -> ForgetPreviewSnapshot {
    let short_id = snap
        .short_id
        .unwrap_or_else(|| snap.id.chars().take(8).collect());
    ForgetPreviewSnapshot {
        snapshot_id: snap.id,
        short_id,
        time: snap.time.to_string(),
        hostname: snap.hostname.unwrap_or_default(),
        paths: snap.paths,
        action: action.to_string(),
        reasons,
    }
}

/// Parses JSON array lines emitted by `restic forget --json` (tolerant parsing).
pub fn parse_forget_json_output(
    backup_id: &BackupId,
    log_lines: &[String],
) -> Result<ForgetPreviewResult, serde_json::Error> {
    let mut result = ForgetPreviewResult {
        backup_id: backup_id.as_str().to_string(),
        snapshots: Vec::new(),
        keep_count: 0,
        remove_count: 0,
    };

    for line in log_lines {
        let trimmed = line.trim();
        if !trimmed.starts_with('[') {
            continue;
        }
        let groups: Vec<RawForgetGroup> = serde_json::from_str(trimmed)?;
        for group in groups {
            let mut reasons_by_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for r in group.reasons {
                if let Some(snap) = r.snapshot {
                    reasons_by_id.insert(snap.id, r.matches);
                }
            }

            for snap in group.keep {
                let reasons = reasons_by_id.remove(&snap.id).unwrap_or_default();
                result.keep_count += 1;
                result
                    .snapshots
                    .push(forget_preview_snapshot(snap, "keep", reasons));
            }

            for snap in group.remove {
                result.remove_count += 1;
                result
                    .snapshots
                    .push(forget_preview_snapshot(snap, "remove", Vec::new()));
            }
        }
        break;
    }

    // Sort newest first using parsed RFC 3339 timestamps (handling differing offsets/precision).
    result.snapshots.sort_by(|a, b| {
        let ta = OffsetDateTime::parse(&a.time, &Rfc3339).ok();
        let tb = OffsetDateTime::parse(&b.time, &Rfc3339).ok();
        match (ta, tb) {
            (Some(pa), Some(pb)) => pb.cmp(&pa),
            _ => b.time.cmp(&a.time),
        }
        .then_with(|| a.snapshot_id.cmp(&b.snapshot_id))
    });
    Ok(result)
}

/// Runs `restic forget --dry-run --no-lock` with explicit cancellation handle.
pub fn execute_forget_dry_run_with(
    backup: &Backup,
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    cancel: Option<&CancelHandle>,
    extra_env: &[(&str, &str)],
) -> Result<ForgetPreviewResult, BackupRunError> {
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;
    let spec = build_forget_command_spec(
        backup,
        destination,
        true,
        true,
        runtime.resolved,
        runtime.kestic_binary,
    )?
    .with_cache_dir(runtime.cache_dir);

    let res = runtime.run(&spec, destination, cancel, extra_env, |_| {})?;

    if res.presentation.status == RunStatus::Cancelled {
        return Err(BackupRunError::Cancelled);
    }

    check_command_succeeded(&res)?;

    Ok(parse_forget_json_output(&backup.id, &res.log_lines)?)
}

/// Runs real `restic forget` for `backup`.
pub fn execute_forget(
    backup: &Backup,
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    cancel: Option<&CancelHandle>,
    extra_env: &[(&str, &str)],
) -> Result<ResticExecutionResult, BackupRunError> {
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;
    let spec = build_forget_command_spec(
        backup,
        destination,
        false,
        false,
        runtime.resolved,
        runtime.kestic_binary,
    )?
    .with_cache_dir(runtime.cache_dir);

    let res = runtime.run(&spec, destination, cancel, extra_env, |_| {})?;
    Ok(res)
}

/// Helper distinguishing Exit 11 (exclusive lock contention during `forget`).
pub fn is_forget_lock_failure(res: &ResticExecutionResult) -> bool {
    res.exit_code == 11 || res.presentation.kind == ResticFailureKind::RepositoryLocked
}

/// Lists all snapshots in `destination` belonging strictly to `backup_id` (`kestic:<backup-id>`).
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

    check_command_succeeded(&res)?;

    // `restic snapshots --json` outputs a JSON array line.
    for line in &res.log_lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            return Ok(serde_json::from_str(trimmed)?);
        }
    }
    Ok(Vec::new())
}
