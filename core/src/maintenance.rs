// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination maintenance (`prune`, `prune --dry-run --no-lock`, `check`) and stale-lock
//! inspection/unlock.

use crate::backup::{BackupRunError, check_command_succeeded};
use crate::destination::{
    DEFAULT_CONNECT_TIMEOUT, DestinationError, preflight_check_destination,
    preflight_check_with_restic, verify_sftp_host_key,
};
use crate::model::{CheckSummary, Destination, RunStatus};
use crate::provider::ResolvedRestic;
use crate::restic::{
    CancelHandle, ResticCommandSpec, ResticExecutionResult, ResticFailureKind, ResticRuntime,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// First restic version whose `prune` writes its result as JSON with `--json`.
pub const PRUNE_JSON_MIN_VERSION: Version = Version::new(0, 20, 0);

/// Builds a [`ResticCommandSpec`] for `restic prune` or `restic prune --dry-run --no-lock`.
///
/// The real `prune` asks for JSON output when restic writes it (see
/// [`PRUNE_JSON_MIN_VERSION`]); older versions and the dry run print text only.
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
    spec.json_output = !dry_run && resolved.version >= PRUNE_JSON_MIN_VERSION;
    spec.low_priority = !dry_run;
    spec
}

/// Builds a [`ResticCommandSpec`] for `restic check` without `--read-data`.
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

/// Parsed preview output of `restic prune --dry-run --no-lock`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrunePreviewSummary {
    pub delete_packs: Option<u64>,
    pub keep_packs: Option<u64>,
    pub freed_summary: Option<String>,
    pub transparent_command: String,
}

/// Parses human-readable `restic prune --dry-run` output tolerantly.
pub fn parse_prune_dry_run_output(
    lines: &[String],
    transparent_command: String,
) -> PrunePreviewSummary {
    let mut summary = PrunePreviewSummary {
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

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawCheckSummary {
    message_type: String,
    num_errors: Option<u64>,
}

/// Finds the `summary` message of `restic check --json` in its output lines.
///
/// Backup summaries share `message_type: "summary"`; only a line with `num_errors` counts.
pub fn parse_check_summary(lines: &[String]) -> Option<CheckSummary> {
    lines.iter().rev().find_map(|line| {
        let raw: RawCheckSummary = serde_json::from_str(line.trim()).ok()?;
        if raw.message_type != "summary" {
            return None;
        }
        Some(CheckSummary {
            num_errors: raw.num_errors?,
        })
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawPruneSummary {
    message_type: String,
    bytes: Option<RawPruneBytes>,
}

#[derive(Debug, Deserialize)]
struct RawPruneBytes {
    remove_total: u64,
}

/// Bytes a real `restic prune` freed: `bytes.remove_total` of its JSON `summary` message, or the
/// size in the text line `total prune: <n> blobs / <size>` of restic versions without JSON output.
/// `None` when neither is present or readable.
pub fn parse_prune_freed_bytes(lines: &[String]) -> Option<u64> {
    parse_prune_json_freed_bytes(lines).or_else(|| parse_prune_text_freed_bytes(lines))
}

fn parse_prune_json_freed_bytes(lines: &[String]) -> Option<u64> {
    lines.iter().rev().find_map(|line| {
        let raw: RawPruneSummary = serde_json::from_str(line.trim()).ok()?;
        if raw.message_type != "summary" {
            return None;
        }
        Some(raw.bytes?.remove_total)
    })
}

fn parse_prune_text_freed_bytes(lines: &[String]) -> Option<u64> {
    let rest = lines
        .iter()
        .find_map(|l| l.trim().strip_prefix("total prune:"))?;
    parse_restic_size(rest.split_once('/')?.1.trim())
}

/// A size as restic prints it (`"512 B"`, `"1.204 MiB"`) in bytes; decimals are cut off.
fn parse_restic_size(text: &str) -> Option<u64> {
    let (number, unit) = text.split_once(' ')?;
    let shift: u32 = match unit.trim() {
        "B" => 0,
        "KiB" => 10,
        "MiB" => 20,
        "GiB" => 30,
        "TiB" => 40,
        _ => return None,
    };
    let (int, frac) = number.split_once('.').unwrap_or((number, ""));
    let digits_only = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if int.is_empty() || frac.len() > 9 || !digits_only(int) || !digits_only(frac) {
        return None;
    }
    let mantissa: u128 = format!("{int}{frac}").parse().ok()?;
    let scale = 10u128.pow(u32::try_from(frac.len()).ok()?);
    let bytes = mantissa.checked_mul(1u128 << shift)? / scale;
    u64::try_from(bytes).ok()
}

/// Executes `restic prune --dry-run --no-lock` on `destination`.
pub fn execute_prune_dry_run(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    cancel: Option<&CancelHandle>,
    extra_env: &[(&str, &str)],
) -> Result<PrunePreviewSummary, BackupRunError> {
    preflight_check_with_restic(destination, runtime)?;
    let spec = build_prune_command_spec(
        destination,
        true,
        true,
        runtime.resolved,
        runtime.kestic_binary,
    )
    .with_cache_dir(runtime.cache_dir);
    let transparent_command = spec.to_transparent_shell_command();

    let res = runtime.run(&spec, destination, cancel, extra_env, |_| {})?;

    if res.presentation.status == RunStatus::Cancelled {
        return Err(BackupRunError::Cancelled);
    }
    check_command_succeeded(&res)?;

    Ok(parse_prune_dry_run_output(
        &res.log_lines,
        transparent_command,
    ))
}

/// Executes real `restic prune` on `destination`.
pub fn execute_prune(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    cancel: Option<&CancelHandle>,
    extra_env: &[(&str, &str)],
) -> Result<ResticExecutionResult, BackupRunError> {
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;
    verify_sftp_host_key(destination)?;
    let spec = build_prune_command_spec(
        destination,
        false,
        false,
        runtime.resolved,
        runtime.kestic_binary,
    )
    .with_cache_dir(runtime.cache_dir);
    Ok(runtime.run(&spec, destination, cancel, extra_env, |_| {})?)
}

/// Executes `restic check` (without `--read-data`) on `destination`.
pub fn execute_check(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    cancel: Option<&CancelHandle>,
    extra_env: &[(&str, &str)],
) -> Result<ResticExecutionResult, BackupRunError> {
    preflight_check_destination(destination, DEFAULT_CONNECT_TIMEOUT)?;
    verify_sftp_host_key(destination)?;
    let spec = build_check_command_spec(destination, runtime.resolved, runtime.kestic_binary)
        .with_cache_dir(runtime.cache_dir);
    Ok(runtime.run(&spec, destination, cancel, extra_env, |_| {})?)
}

/// Lock metadata returned by `restic cat lock <id> --no-lock`.
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

/// Result of classifying repository locks against local host & running PIDs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LockClassification {
    pub stale_lock_ids: Vec<String>,
    pub active_local_locks: Vec<RepoLockInfo>,
    pub foreign_host_locks: Vec<RepoLockInfo>,
    pub has_stale_local_lock: bool,
}

/// Classifies repository locks:
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
    preflight_check_with_restic(destination, runtime)?;
    let mut list_spec = ResticCommandSpec::for_destination(
        &runtime.resolved.binary,
        runtime.kestic_binary,
        destination,
        vec![
            "list".to_string(),
            "locks".to_string(),
            "--no-lock".to_string(),
        ],
    )
    .with_cache_dir(runtime.cache_dir);
    list_spec.json_output = false;
    list_spec.low_priority = false;

    let list_res = runtime.run(&list_spec, destination, None, extra_env, |_| {})?;
    check_command_succeeded(&list_res)?;

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
        )
        .with_cache_dir(runtime.cache_dir);
        cat_spec.json_output = false;
        cat_spec.low_priority = false;

        let cat_res = runtime.run(&cat_spec, destination, None, extra_env, |_| {})?;
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

/// Inspects locks on `destination` using explicit `local_hostname` and `is_pid_alive` predicate.
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

/// Inspects locks on `destination` using the real local hostname and `/proc` PID check.
pub fn detect_stale_locks(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    extra_env: &[(&str, &str)],
) -> Result<LockClassification, BackupRunError> {
    let host = current_local_hostname();
    detect_stale_locks_with(destination, runtime, &host, is_local_pid_alive, extra_env)
}

/// Executes `restic unlock` on `destination` ONLY if lock inspection confirms stale local locks
/// exist and neither foreign-host nor active local locks are present.
pub fn execute_unlock_with<F>(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    local_hostname: &str,
    is_pid_alive: F,
    extra_env: &[(&str, &str)],
) -> Result<ResticExecutionResult, BackupRunError>
where
    F: Fn(u32) -> bool,
{
    let cls = detect_stale_locks_with(
        destination,
        runtime,
        local_hostname,
        is_pid_alive,
        extra_env,
    )?;
    if !cls.foreign_host_locks.is_empty() {
        return Err(BackupRunError::Destination(
            DestinationError::CommandFailed {
                exit_code: 11,
                message: format!(
                    "refusing unlock: foreign host locks present ({})",
                    cls.foreign_host_locks
                        .iter()
                        .map(|l| l.hostname.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                kind: ResticFailureKind::RepositoryLocked,
            },
        ));
    }
    if !cls.active_local_locks.is_empty() {
        return Err(BackupRunError::Destination(
            DestinationError::CommandFailed {
                exit_code: 11,
                message: "refusing unlock: active local process lock present".to_string(),
                kind: ResticFailureKind::RepositoryLocked,
            },
        ));
    }
    if !cls.has_stale_local_lock {
        return Err(BackupRunError::Destination(
            DestinationError::CommandFailed {
                exit_code: 11,
                message: "no stale local lock found".to_string(),
                kind: ResticFailureKind::RepositoryLocked,
            },
        ));
    }

    let spec =
        ResticCommandSpec::unlock(&runtime.resolved.binary, runtime.kestic_binary, destination)
            .with_cache_dir(runtime.cache_dir);
    Ok(runtime.run(&spec, destination, None, extra_env, |_| {})?)
}

/// Executes `restic unlock` on `destination` using the real local hostname and `/proc` PID check.
pub fn execute_unlock(
    destination: &Destination,
    runtime: &ResticRuntime<'_>,
    extra_env: &[(&str, &str)],
) -> Result<ResticExecutionResult, BackupRunError> {
    let host = current_local_hostname();
    execute_unlock_with(destination, runtime, &host, is_local_pid_alive, extra_env)
}
