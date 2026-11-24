// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Synchronous and asynchronous dry-run backup previews, retention previews, and prune previews.

use super::error::{KesticServiceErr, backup_run_error_kind, map_backup_run_err, parse_backup_id};
use super::state::{DaemonCore, ResticDeps, probe_for_run, resolve_restic_recording_probe};
use crate::dbus_api::{BackupDryRunStatusDto, PreviewStatusDto, RetentionPreviewStatusDto};
use kestic_core::backup::{
    BackupDryRunResult, BackupExecutionContext, BackupRunError, ForgetPreviewResult,
    execute_backup_dry_run_with, execute_forget_dry_run_with,
};
use kestic_core::maintenance::{PrunePreviewSummary, execute_prune_dry_run};
use kestic_core::model::{DestinationId, ExcludeRules, RetentionPolicy, RunId};
use kestic_core::mount::current_home_dir;
use kestic_core::restic::{CancelHandle, ResticRuntime};
use kestic_core::restic_gate::ResticBinaryGate;
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::ErrorKind;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

/// State of an asynchronous preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PreviewState {
    Running,
    Done,
    Cancelled,
    Error,
}

impl PreviewState {
    /// The `state` string of the status DTO.
    fn wire_name(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
        }
    }
}

/// An asynchronous preview (dry run or retention preview) and its outcome; `T` is the result of
/// the finished preview.
pub(super) struct PreviewEntry<T> {
    state: PreviewState,
    result: Option<T>,
    /// Error kind and raw detail while the state is `Error`.
    error: Option<(ErrorKind, String)>,
    cancel: CancelHandle,
}

impl<T> PreviewEntry<T> {
    /// Records the outcome of the worker; `is_cancelled` tells whether an `Ok` result is that of
    /// a cancelled preview.
    fn finish(&mut self, res: Result<T, BackupRunError>, is_cancelled: impl FnOnce(&T) -> bool) {
        match res {
            Ok(result) => {
                self.state = if is_cancelled(&result) {
                    PreviewState::Cancelled
                } else {
                    PreviewState::Done
                };
                self.result = Some(result);
            }
            Err(err) => {
                self.state = PreviewState::Error;
                self.error = Some((backup_run_error_kind(&err), err.to_string()));
            }
        }
    }
}

type PreviewMap<T> = HashMap<String, PreviewEntry<T>>;

fn preview_status<T: Clone>(
    previews: &PreviewMap<T>,
    preview_id: &str,
) -> Result<PreviewStatusDto<T>, KesticServiceErr> {
    let entry = previews
        .get(preview_id)
        .ok_or_else(|| KesticServiceErr::NotFound(preview_id.to_string()))?;
    Ok(PreviewStatusDto {
        preview_id: preview_id.to_string(),
        state: entry.state.wire_name().to_string(),
        result: entry.result.clone(),
        error_kind: entry.error.as_ref().map(|(kind, _)| *kind),
        error_detail: entry.error.as_ref().map(|(_, detail)| detail.clone()),
    })
}

fn cancel_preview<T>(previews: &PreviewMap<T>, preview_id: &str) -> Result<(), KesticServiceErr> {
    previews
        .get(preview_id)
        .ok_or_else(|| KesticServiceErr::NotFound(preview_id.to_string()))?
        .cancel
        .cancel();
    Ok(())
}

impl DaemonCore {
    pub fn get_backup_dry_run_status(
        &self,
        preview_id: &str,
    ) -> Result<BackupDryRunStatusDto, KesticServiceErr> {
        preview_status(&self.dry_runs, preview_id)
    }

    pub fn cancel_backup_dry_run(&self, preview_id: &str) -> Result<(), KesticServiceErr> {
        cancel_preview(&self.dry_runs, preview_id)
    }

    pub fn get_retention_preview_status(
        &self,
        preview_id: &str,
    ) -> Result<RetentionPreviewStatusDto, KesticServiceErr> {
        preview_status(&self.retention_previews, preview_id)
    }

    pub fn cancel_retention_preview(&self, preview_id: &str) -> Result<(), KesticServiceErr> {
        cancel_preview(&self.retention_previews, preview_id)
    }
}

/// Registers a running preview in `previews`, dropping finished ones once there are many.
fn register_preview<T>(
    previews: &mut PreviewMap<T>,
) -> Result<(String, CancelHandle), KesticServiceErr> {
    let preview_id = RunId::generate()
        .map(|r| r.as_str().to_string())
        .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
    let cancel = CancelHandle::new();
    if previews.len() > 16 {
        previews.retain(|_, v| v.state == PreviewState::Running);
    }
    previews.insert(
        preview_id.clone(),
        PreviewEntry {
            state: PreviewState::Running,
            result: None,
            error: None,
            cancel: cancel.clone(),
        },
    );
    Ok((preview_id, cancel))
}

/// A registered preview and what its worker needs.
struct PreviewLaunch<T, W> {
    preview_id: String,
    deps: ResticDeps,
    cancel: CancelHandle,
    /// Runs restic and returns the preview result.
    work: W,
    /// Records the worker's outcome in the preview entry.
    finish: fn(&mut PreviewEntry<T>, Result<T, BackupRunError>),
}

/// Resolves restic for a registered preview and runs its work on a worker thread. The preview is
/// withdrawn again when restic cannot be resolved.
fn launch_preview<T, W>(
    shared: &Arc<Mutex<DaemonCore>>,
    previews: fn(&mut DaemonCore) -> &mut PreviewMap<T>,
    launch: PreviewLaunch<T, W>,
) -> Result<String, KesticServiceErr>
where
    T: Send + 'static,
    W: FnOnce(&ResticRuntime<'_>, &CancelHandle) -> Result<T, BackupRunError> + Send + 'static,
{
    let PreviewLaunch {
        preview_id,
        deps,
        cancel,
        work,
        finish,
    } = launch;
    // Held until the preview has ended, so the app-managed restic is not replaced between its
    // restic processes.
    let run_lease = ResticBinaryGate::global().read();
    let resolved = match probe_for_run(&deps.provider, deps.secret_store.as_ref()).1 {
        Ok(resolved) => resolved,
        Err(err) => {
            previews(&mut shared.lock_unpoisoned()).remove(&preview_id);
            return Err(err);
        }
    };

    let id_for_caller = preview_id.clone();
    let shared_worker = Arc::clone(shared);
    thread::spawn(move || {
        let _run_lease = run_lease;
        let res = work(&deps.runtime(&resolved), &cancel);
        let mut guard = shared_worker.lock_unpoisoned();
        if let Some(entry) = previews(&mut guard).get_mut(&preview_id) {
            finish(entry, res);
        }
    });
    Ok(id_for_caller)
}

/// Starts an asynchronous dry-run preview (`restic backup --dry-run -vv`).
pub fn start_backup_dry_run_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
    excludes_json: &str,
) -> Result<String, KesticServiceErr> {
    let b_id = parse_backup_id(backup_id)?;

    let override_excludes = if excludes_json.trim().is_empty() {
        None
    } else {
        let mut ex: ExcludeRules = serde_json::from_str(excludes_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid excludes JSON: {e}")))?;
        ex.normalize_folders(current_home_dir().as_deref());
        ex.validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        Some(ex)
    };

    let (backup, dest, deps, state_dir, mount_inspector, preview_id, cancel) = {
        let mut guard = shared.lock_unpoisoned();
        let (mut backup, dest) = guard.backup_with_destination(&b_id)?;
        if let Some(ex) = override_excludes {
            backup.excludes = ex;
        }
        let (preview_id, cancel) = register_preview(&mut guard.dry_runs)?;
        (
            backup,
            dest,
            guard.restic_deps(),
            guard.state_dir.clone(),
            Arc::clone(&guard.mount_inspector),
            preview_id,
            cancel,
        )
    };

    let work = move |runtime: &ResticRuntime<'_>, cancel: &CancelHandle| {
        let home = current_home_dir();
        let dry_ctx = BackupExecutionContext {
            state_dir: &state_dir,
            home_dir: home.as_deref(),
            inspector: mount_inspector.as_ref(),
            cancel: Some(cancel),
            extra_env: &[],
        };
        execute_backup_dry_run_with(&backup, &dest, runtime, &dry_ctx, |_| {})
    };
    launch_preview(
        shared,
        |core| &mut core.dry_runs,
        PreviewLaunch {
            preview_id,
            deps,
            cancel,
            work,
            finish: |entry, res| entry.finish(res, |dry: &BackupDryRunResult| dry.cancelled),
        },
    )
}

/// Starts an asynchronous retention preview (`restic forget --dry-run --no-lock --json`).
pub fn start_retention_preview_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
    retention_json: &str,
) -> Result<String, KesticServiceErr> {
    let b_id = parse_backup_id(backup_id)?;
    let retention = parse_retention(retention_json)?;

    let (mut backup, dest, deps, preview_id, cancel) = {
        let mut guard = shared.lock_unpoisoned();
        let (backup, dest) = guard.backup_with_destination(&b_id)?;
        let (preview_id, cancel) = register_preview(&mut guard.retention_previews)?;
        (backup, dest, guard.restic_deps(), preview_id, cancel)
    };
    backup.retention = retention;

    let work = move |runtime: &ResticRuntime<'_>, cancel: &CancelHandle| {
        execute_forget_dry_run_with(&backup, &dest, runtime, Some(cancel), &[])
    };
    launch_preview(
        shared,
        |core| &mut core.retention_previews,
        PreviewLaunch {
            preview_id,
            deps,
            cancel,
            work,
            finish: |entry, res| match res {
                Err(BackupRunError::Cancelled) => entry.state = PreviewState::Cancelled,
                res => entry.finish(res, |_: &ForgetPreviewResult| false),
            },
        },
    )
}

fn parse_retention(retention_json: &str) -> Result<RetentionPolicy, KesticServiceErr> {
    let retention: RetentionPolicy = serde_json::from_str(retention_json)
        .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid retention JSON: {e}")))?;
    retention
        .validate()
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    Ok(retention)
}

pub(crate) fn preview_destination_maintenance_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    destination_id: &str,
) -> Result<PrunePreviewSummary, KesticServiceErr> {
    let d_id = DestinationId::parse(destination_id)
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    let (dest, deps) = {
        let guard = shared.lock_unpoisoned();
        let dest = guard
            .config
            .destination(&d_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;
        (dest, guard.restic_deps())
    };

    // Held for the whole call, so the app-managed restic is not replaced between its restic
    // processes.
    let _run_lease = ResticBinaryGate::global().read();
    let resolved = resolve_restic_recording_probe(shared, &deps)?;
    execute_prune_dry_run(&dest, &deps.runtime(&resolved), None, &[]).map_err(map_backup_run_err)
}
