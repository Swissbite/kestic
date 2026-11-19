// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Synchronous and asynchronous dry-run backup previews, retention previews, and prune previews.

use super::error::{
    KesticServiceErr, localized_backup_run_error, map_destination_err, map_provider_err,
};
use super::state::{DaemonCore, DryRunEntry, RetentionPreviewEntry, probe_restic_and_secrets_raw};
use crate::dbus_api::{BackupDryRunStatusDto, RetentionPreviewStatusDto};
use crate::i18n::i18n;
use kestic_core::backup::{
    BackupExecutionContext, BackupRunError, ForgetPreviewResult, execute_backup_dry_run_with,
    execute_forget_dry_run, execute_forget_dry_run_with,
};
use kestic_core::maintenance::{PrunePreviewSummary, execute_prune_dry_run};
use kestic_core::model::{BackupId, DestinationId, ExcludeRules, RetentionPolicy, RunId};
use kestic_core::mount::current_home_dir;
use kestic_core::provider::resolve_and_check;
use kestic_core::restic::{CancelHandle, ResticRuntime};
use std::sync::{Arc, Mutex};
use std::thread;

impl DaemonCore {
    pub fn get_backup_dry_run_status(
        &self,
        preview_id: &str,
    ) -> Result<BackupDryRunStatusDto, KesticServiceErr> {
        let entry = self
            .dry_runs
            .get(preview_id)
            .ok_or_else(|| KesticServiceErr::NotFound(preview_id.to_string()))?;
        Ok(BackupDryRunStatusDto {
            preview_id: preview_id.to_string(),
            state: entry.state.clone(),
            result: entry.result.clone(),
            error: entry.error.clone(),
        })
    }

    pub fn cancel_backup_dry_run(&self, preview_id: &str) -> Result<(), KesticServiceErr> {
        let entry = self
            .dry_runs
            .get(preview_id)
            .ok_or_else(|| KesticServiceErr::NotFound(preview_id.to_string()))?;
        entry.cancel.cancel();
        Ok(())
    }

    pub fn get_retention_preview_status(
        &self,
        preview_id: &str,
    ) -> Result<RetentionPreviewStatusDto, KesticServiceErr> {
        let entry = self
            .retention_previews
            .get(preview_id)
            .ok_or_else(|| KesticServiceErr::NotFound(preview_id.to_string()))?;
        Ok(RetentionPreviewStatusDto {
            preview_id: preview_id.to_string(),
            state: entry.state.clone(),
            result: entry.result.clone(),
            error: entry.error.clone(),
        })
    }

    pub fn cancel_retention_preview(&self, preview_id: &str) -> Result<(), KesticServiceErr> {
        let entry = self
            .retention_previews
            .get(preview_id)
            .ok_or_else(|| KesticServiceErr::NotFound(preview_id.to_string()))?;
        entry.cancel.cancel();
        Ok(())
    }
}

/// Starts an asynchronous dry-run preview (`restic backup --dry-run -vv`, `M2-S04`).
pub fn start_backup_dry_run_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
    excludes_json: &str,
) -> Result<String, KesticServiceErr> {
    let b_id =
        BackupId::parse(backup_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

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

    let (
        backup,
        dest,
        provider,
        state_dir,
        kestic_binary,
        secret_store,
        clock,
        mount_inspector,
        preview_id,
        cancel,
    ) = {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let mut backup = guard
            .config
            .backup(&b_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        if let Some(ex) = override_excludes {
            backup.excludes = ex;
        }
        let dest = guard
            .config
            .destination(&backup.destination_id)
            .cloned()
            .ok_or_else(|| {
                KesticServiceErr::NotFound(backup.destination_id.as_str().to_string())
            })?;

        let preview_id = RunId::generate()
            .map(|r| r.as_str().to_string())
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        let cancel = CancelHandle::new();

        if guard.dry_runs.len() > 16 {
            guard.dry_runs.retain(|_, v| v.state == "running");
        }
        guard.dry_runs.insert(
            preview_id.clone(),
            DryRunEntry {
                state: "running".to_string(),
                result: None,
                error: None,
                cancel: cancel.clone(),
            },
        );

        (
            backup,
            dest,
            guard.config.restic_provider.clone(),
            guard.state_dir.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
            Arc::clone(&guard.mount_inspector),
            preview_id,
            cancel,
        )
    };

    let (_, secret_status, _) = probe_restic_and_secrets_raw(&provider, secret_store.as_ref());
    if !secret_status.available {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        guard.dry_runs.remove(&preview_id);
        return Err(KesticServiceErr::SecretStoreLocked(
            secret_status
                .message
                .unwrap_or_else(|| i18n("Password store is locked").into()),
        ));
    }
    let resolved = match resolve_and_check(&provider) {
        Ok(r) => r,
        Err(err) => {
            let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
            guard.dry_runs.remove(&preview_id);
            return Err(map_provider_err(err));
        }
    };

    let pid_ret = preview_id.clone();
    let shared_worker = Arc::clone(shared);
    thread::spawn(move || {
        let home = current_home_dir();
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &kestic_binary,
            secret_store: secret_store.as_ref(),
            clock: clock.as_ref(),
            cache_dir: None,
        };
        let dry_ctx = BackupExecutionContext {
            state_dir: &state_dir,
            home_dir: home.as_deref(),
            inspector: mount_inspector.as_ref(),
            cancel: Some(&cancel),
            extra_env: &[],
        };
        let res = execute_backup_dry_run_with(&backup, &dest, &runtime, &dry_ctx, |_| {});
        let mut guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = guard.dry_runs.get_mut(&preview_id) {
            match res {
                Ok(dry) => {
                    entry.state = if dry.cancelled {
                        "cancelled".to_string()
                    } else {
                        "done".to_string()
                    };
                    entry.result = Some(dry);
                }
                Err(err) => {
                    let msg = localized_backup_run_error(&err);
                    entry.state = "error".to_string();
                    entry.error = Some(msg);
                }
            }
        }
    });

    Ok(pid_ret)
}

/// Starts an asynchronous retention preview (`restic forget --dry-run --no-lock --json`, `M2-S06`).
pub fn start_retention_preview_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
    retention_json: &str,
) -> Result<String, KesticServiceErr> {
    let b_id =
        BackupId::parse(backup_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    let retention: RetentionPolicy = serde_json::from_str(retention_json)
        .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid retention JSON: {e}")))?;
    retention
        .validate()
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

    let (mut backup, dest, provider, kestic_binary, secret_store, clock, preview_id, cancel) = {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let backup = guard
            .config
            .backup(&b_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let dest = guard
            .config
            .destination(&backup.destination_id)
            .cloned()
            .ok_or_else(|| {
                KesticServiceErr::NotFound(backup.destination_id.as_str().to_string())
            })?;

        let preview_id = RunId::generate()
            .map(|r| r.as_str().to_string())
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        let cancel = CancelHandle::new();

        if guard.retention_previews.len() > 16 {
            guard.retention_previews.retain(|_, v| v.state == "running");
        }
        guard.retention_previews.insert(
            preview_id.clone(),
            RetentionPreviewEntry {
                state: "running".to_string(),
                result: None,
                error: None,
                cancel: cancel.clone(),
            },
        );

        (
            backup,
            dest,
            guard.config.restic_provider.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
            preview_id,
            cancel,
        )
    };
    backup.retention = retention;

    let (_, secret_status, _) = probe_restic_and_secrets_raw(&provider, secret_store.as_ref());
    if !secret_status.available {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        guard.retention_previews.remove(&preview_id);
        return Err(KesticServiceErr::SecretStoreLocked(
            secret_status
                .message
                .unwrap_or_else(|| i18n("Password store is locked").into()),
        ));
    }
    let resolved = match resolve_and_check(&provider) {
        Ok(r) => r,
        Err(err) => {
            let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
            guard.retention_previews.remove(&preview_id);
            return Err(map_provider_err(err));
        }
    };

    let pid_ret = preview_id.clone();
    let shared_worker = Arc::clone(shared);
    thread::spawn(move || {
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &kestic_binary,
            secret_store: secret_store.as_ref(),
            clock: clock.as_ref(),
            cache_dir: None,
        };
        let res = execute_forget_dry_run_with(&backup, &dest, &runtime, Some(&cancel), &[]);
        let mut guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = guard.retention_previews.get_mut(&preview_id) {
            match res {
                Ok(preview) => {
                    entry.state = "done".to_string();
                    entry.result = Some(preview);
                }
                Err(BackupRunError::Cancelled) => {
                    entry.state = "cancelled".to_string();
                }
                Err(err) => {
                    let msg = match err {
                        BackupRunError::Destination(d) => map_destination_err(d).to_string(),
                        BackupRunError::InvalidModel(m) => m.to_string(),
                        other => localized_backup_run_error(&other),
                    };
                    entry.state = "error".to_string();
                    entry.error = Some(msg);
                }
            }
        }
    });

    Ok(pid_ret)
}

pub(crate) fn preview_retention_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
    retention_json: &str,
) -> Result<ForgetPreviewResult, KesticServiceErr> {
    let b_id =
        BackupId::parse(backup_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    let retention: RetentionPolicy = serde_json::from_str(retention_json)
        .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid retention JSON: {e}")))?;
    retention
        .validate()
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

    let (mut backup, dest, provider, kestic_binary, secret_store, clock) = {
        let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let backup = guard
            .config
            .backup(&b_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let dest = guard
            .config
            .destination(&backup.destination_id)
            .cloned()
            .ok_or_else(|| {
                KesticServiceErr::NotFound(backup.destination_id.as_str().to_string())
            })?;
        (
            backup,
            dest,
            guard.config.restic_provider.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
        )
    };
    backup.retention = retention;

    let probe = probe_restic_and_secrets_raw(&provider, secret_store.as_ref());
    if !probe.1.available {
        let msg = probe
            .1
            .message
            .clone()
            .unwrap_or_else(|| i18n("Password store is locked").into());
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        guard.record_probe(probe);
        return Err(KesticServiceErr::SecretStoreLocked(msg));
    }
    let resolved = resolve_and_check(&provider).map_err(map_provider_err)?;
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &kestic_binary,
        secret_store: secret_store.as_ref(),
        clock: clock.as_ref(),
        cache_dir: None,
    };
    execute_forget_dry_run(&backup, &dest, &runtime, &[]).map_err(|err| match err {
        BackupRunError::Destination(d) => map_destination_err(d),
        BackupRunError::InvalidModel(m) => KesticServiceErr::InvalidInput(m.to_string()),
        other => KesticServiceErr::Internal(other.to_string()),
    })
}

pub(crate) fn preview_destination_maintenance_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    destination_id: &str,
) -> Result<PrunePreviewSummary, KesticServiceErr> {
    let d_id = DestinationId::parse(destination_id)
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    let (dest, provider, kestic_binary, secret_store, clock) = {
        let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let dest = guard
            .config
            .destination(&d_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;
        (
            dest,
            guard.config.restic_provider.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
        )
    };

    let probe = probe_restic_and_secrets_raw(&provider, secret_store.as_ref());
    if !probe.1.available {
        let msg = probe
            .1
            .message
            .clone()
            .unwrap_or_else(|| i18n("Password store is locked").into());
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        guard.record_probe(probe);
        return Err(KesticServiceErr::SecretStoreLocked(msg));
    }
    let resolved = resolve_and_check(&provider).map_err(map_provider_err)?;
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &kestic_binary,
        secret_store: secret_store.as_ref(),
        clock: clock.as_ref(),
        cache_dir: None,
    };
    execute_prune_dry_run(&dest, &runtime, None, &[]).map_err(|err| match err {
        BackupRunError::Destination(d) => map_destination_err(d),
        BackupRunError::InvalidModel(m) => KesticServiceErr::InvalidInput(m.to_string()),
        other => KesticServiceErr::Internal(other.to_string()),
    })
}
