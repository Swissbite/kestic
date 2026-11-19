// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination maintenance (`prune`, `prune --dry-run --no-lock`, `check`) and stale-lock
//! inspection/unlock (`FR-11`, `FR-19`, `FR-45`, `NFR-08`, `P16`, `P33`, `P41`, `M3-S05`, `M3-S06`).

use crate::backup::BackupRunError;
use crate::destination::{DEFAULT_CONNECT_TIMEOUT, DestinationError, preflight_check_destination};
use crate::model::{Destination, RunStatus};
use crate::provider::ResolvedRestic;
use crate::restic::{
    CancelHandle, ResticCommandSpec, ResticExecutionResult, ResticRuntime, run_restic,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Builds a [`ResticCommandSpec`] for `restic prune` or `restic prune --dry-run --no-lock`
/// (`AC-M3-05-7`, `AC-M3-05-8`, `P33`, `P41`).
///
/// Note: `restic prune` does NOT support `--json` (`restic-integration.md`), so `json_output = false`.
pub fn build_prune_command_spec(
    destination: &Destination,
    dry_run: bool,
    no_lock: bool,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
) -> ResticCommandSpec {
    let mut args = vec!["prune".to_string()];
    if dry_run {
        args.push("--dry-run".to_string());
    }
    if no_lock {
        args.push("--no-lock".to_string());
    }
    let mut spec =
        ResticCommandSpec::for_destination(&resolved.binary, kestic_binary, destination, args);
    spec.json_output = false;
    spec.low_priority = !dry_run;
    spec
}

/// Builds a [`ResticCommandSpec`] for `restic check` without `--read-data` (`AC-M3-05-6`, `FR-11`).
pub fn build_check_command_spec(
    destination: &Destination,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
) -> ResticCommandSpec {
    ResticCommandSpec::for_destination(
        &resolved.binary,
        kestic_binary,
        destination,
        vec!["check".to_string()],
    )
}

/// Parsed preview output of `restic prune --dry-run --no-lock` (`AC-M3-05-7`, `AC-M3-05-8`, `P10`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrunePreviewSummary {
    pub raw_lines: Vec<String>,
    pub delete_packs: Option<u64>,
    pub keep_packs: Option<u64>,
    pub freed_summary: Option<String>,
    pub transparent_command: String,
}

/// Parses human-readable `restic prune --dry-run` output tolerantly (`NFR-10`).
pub fn parse_prune_dry_run_output(
    lines: &[String],
    transparent_command: String,
) -> PrunePreviewSummary {
    let mut summary = PrunePreviewSummary {
        raw_lines: lines.to_vec(),
        delete_packs: None,
        keep_packs: None,
        freed_summary: None,
        transparent_command,
    };

    for line in lines {
        let lower = line.trim().to_ascii_lowercase();
        if (lower.starts_with("to delete:")
            || lower.contains("will remove")
            || lower.contains("unused size"))
            && summary.freed_summary.is_none()
        {
            summary.freed_summary = Some(line.trim().to_string());
        }
        if lower.starts_with("to delete:") && summary.delete_packs.is_none() {
            summary.delete_packs = extract_pack_count(&lower);
        }
        if lower.starts_with("to keep:") && summary.keep_packs.is_none() {
            summary.keep_packs = extract_pack_count(&lower);
        }
    }

    summary
}

fn extract_pack_count(lower: &str) -> Option<u64> {
    // Examples: "to delete: 12 packs (3.4 MiB)" or "to delete: 0 blobs / 0 B"
    for token in lower.split_whitespace() {
        let clean = token.trim_matches(|c: char| !c.is_ascii_digit());
        if !clean.is_empty()
            && let Ok(n) = clean.parse::<u64>()
        {
            return Some(n);
        }
    }
    None
}

/// Executes `restic prune --dry-run --no-lock` on `destination` (`AC-M3-05-7`, `AC-M3-05-8`, `P41`).
pub fn execute_prune_dry_run(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    cancel: Option<&CancelHandle>,
    extra_env: &[(&str, &str)],
) -> Result<PrunePreviewSummary, BackupRunError> {
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;
    let spec = build_prune_command_spec(
        destination,
        true,
        true,
        runtime.resolved,
        runtime.kestic_binary,
    );
    let transparent_command = spec.to_transparent_shell_command();

    let res = run_restic(
        &spec,
        destination,
        Some(runtime.secret_store),
        runtime.clock,
        cancel,
        extra_env,
        |_| {},
    )?;

    if res.presentation.status == RunStatus::Cancelled {
        return Err(BackupRunError::Cancelled);
    }
    if res.presentation.status == RunStatus::Error {
        return Err(BackupRunError::Destination(
            DestinationError::CommandFailed {
                exit_code: res.exit_code,
                message: res.presentation.detail_or_summary(),
            },
        ));
    }

    Ok(parse_prune_dry_run_output(
        &res.log_lines,
        transparent_command,
    ))
}

/// Executes real `restic prune` on `destination` (`FR-45`, `M3-S05`).
pub fn execute_prune(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    cancel: Option<&CancelHandle>,
    extra_env: &[(&str, &str)],
) -> Result<ResticExecutionResult, BackupRunError> {
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;
    let spec = build_prune_command_spec(
        destination,
        false,
        false,
        runtime.resolved,
        runtime.kestic_binary,
    );
    Ok(run_restic(
        &spec,
        destination,
        Some(runtime.secret_store),
        runtime.clock,
        cancel,
        extra_env,
        |_| {},
    )?)
}

/// Executes `restic check` (without `--read-data`) on `destination` (`AC-M3-05-6`, `FR-11`).
pub fn execute_check(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    cancel: Option<&CancelHandle>,
    extra_env: &[(&str, &str)],
) -> Result<ResticExecutionResult, BackupRunError> {
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;
    let spec = build_check_command_spec(destination, runtime.resolved, runtime.kestic_binary);
    Ok(run_restic(
        &spec,
        destination,
        Some(runtime.secret_store),
        runtime.clock,
        cancel,
        extra_env,
        |_| {},
    )?)
}

/// Lock metadata returned by `restic cat lock <id> --no-lock` (`NFR-10`, `FR-19`, `P16`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoLockInfo {
    #[serde(skip_deserializing)]
    pub lock_id: String,
    pub time: Option<String>,
    pub exclusive: bool,
    pub hostname: String,
    pub username: Option<String>,
    pub pid: Option<u32>,
}

/// Result of classifying repository locks against local host & running PIDs (`AC-M3-06-1..3`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LockClassification {
    pub stale_lock_ids: Vec<String>,
    pub active_local_locks: Vec<RepoLockInfo>,
    pub foreign_host_locks: Vec<RepoLockInfo>,
    pub has_stale_local_lock: bool,
}

/// Classifies repository locks (`FR-19`, `P16`):
/// Only locks created by `local_hostname` whose recorded local PID is no longer alive are stale.
pub fn classify_locks<F>(
    locks: &[RepoLockInfo],
    local_hostname: &str,
    is_pid_alive: F,
) -> LockClassification
where
    F: Fn(u32) -> bool,
{
    let mut out = LockClassification::default();
    for lock in locks {
        if lock.hostname.trim().is_empty() || lock.hostname != local_hostname {
            out.foreign_host_locks.push(lock.clone());
            continue;
        }
        match lock.pid {
            Some(pid) if !is_pid_alive(pid) => {
                out.stale_lock_ids.push(lock.lock_id.clone());
            }
            _ => {
                out.active_local_locks.push(lock.clone());
            }
        }
    }
    out.has_stale_local_lock = !out.stale_lock_ids.is_empty();
    out
}

/// Reads the local machine hostname (`uname` / `/proc/sys/kernel/hostname`) without unsafe code.
pub fn current_local_hostname() -> String {
    if let Ok(raw) = fs::read_to_string("/proc/sys/kernel/hostname") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(raw) = fs::read_to_string("/etc/hostname") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    std::env::var("HOSTNAME").unwrap_or_default()
}

/// Returns whether `pid` currently exists in `/proc` (excluding zombie `Z` processes if readable).
pub fn is_local_pid_alive(pid: u32) -> bool {
    let proc_dir = Path::new("/proc").join(pid.to_string());
    if !proc_dir.exists() {
        return false;
    }
    if let Ok(stat) = fs::read_to_string(proc_dir.join("stat"))
        && let Some(rparen) = stat.rfind(')')
        && let Some(state_char) = stat[rparen + 1..].trim_start().chars().next()
        && state_char == 'Z'
    {
        return false;
    }
    true
}

/// Lists and reads all lock objects currently present in `destination` using `--no-lock`.
pub fn list_and_inspect_locks(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    extra_env: &[(&str, &str)],
) -> Result<Vec<RepoLockInfo>, BackupRunError> {
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;
    let mut list_spec = ResticCommandSpec::for_destination(
        &runtime.resolved.binary,
        runtime.kestic_binary,
        destination,
        vec![
            "list".to_string(),
            "locks".to_string(),
            "--no-lock".to_string(),
        ],
    );
    list_spec.json_output = false;
    list_spec.low_priority = false;

    let list_res = run_restic(
        &list_spec,
        destination,
        Some(runtime.secret_store),
        runtime.clock,
        None,
        extra_env,
        |_| {},
    )?;
    if list_res.presentation.status == RunStatus::Error {
        return Err(BackupRunError::Destination(
            DestinationError::CommandFailed {
                exit_code: list_res.exit_code,
                message: list_res.presentation.detail_or_summary(),
            },
        ));
    }

    let lock_ids: Vec<String> = list_res
        .log_lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| l.len() == 64 && l.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(ToOwned::to_owned)
        .collect();

    let mut locks = Vec::with_capacity(lock_ids.len());
    for lock_id in lock_ids {
        let mut cat_spec = ResticCommandSpec::for_destination(
            &runtime.resolved.binary,
            runtime.kestic_binary,
            destination,
            vec![
                "cat".to_string(),
                "lock".to_string(),
                lock_id.clone(),
                "--no-lock".to_string(),
            ],
        );
        cat_spec.json_output = false;
        cat_spec.low_priority = false;

        let cat_res = run_restic(
            &cat_spec,
            destination,
            Some(runtime.secret_store),
            runtime.clock,
            None,
            extra_env,
            |_| {},
        )?;
        if cat_res.exit_code == 0 {
            let joined = cat_res.log_lines.join("\n");
            if let Some(start) = joined.find('{')
                && let Ok(mut info) = serde_json::from_str::<RepoLockInfo>(&joined[start..])
            {
                info.lock_id = lock_id;
                locks.push(info);
            }
        }
    }

    Ok(locks)
}

/// Inspects locks on `destination` using explicit `local_hostname` and `is_pid_alive` predicate (`AC-M3-06-1..3`).
pub fn detect_stale_locks_with<F>(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    local_hostname: &str,
    is_pid_alive: F,
    extra_env: &[(&str, &str)],
) -> Result<LockClassification, BackupRunError>
where
    F: Fn(u32) -> bool,
{
    let locks = list_and_inspect_locks(destination, runtime, extra_env)?;
    Ok(classify_locks(&locks, local_hostname, is_pid_alive))
}

/// Inspects locks on `destination` using the real local hostname and `/proc` PID check (`FR-19`, `P16`).
pub fn detect_stale_locks(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    extra_env: &[(&str, &str)],
) -> Result<LockClassification, BackupRunError> {
    let host = current_local_hostname();
    detect_stale_locks_with(destination, runtime, &host, is_local_pid_alive, extra_env)
}

/// Executes `restic unlock` on `destination` (`FR-19`, `AC-M3-06-1`).
pub fn execute_unlock(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    extra_env: &[(&str, &str)],
) -> Result<ResticExecutionResult, BackupRunError> {
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;
    let spec =
        ResticCommandSpec::unlock(&runtime.resolved.binary, runtime.kestic_binary, destination);
    Ok(run_restic(
        &spec,
        destination,
        Some(runtime.secret_store),
        runtime.clock,
        None,
        extra_env,
        |_| {},
    )?)
}
