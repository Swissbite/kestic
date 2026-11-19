// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! In-memory write-through caches for per-backup runtime state, per-destination maintenance
//! state, and the global pause, so the 1 s scheduler tick never reads SQLite (the daemon is the
//! sole writer of `history.db`, `F14`).

use super::state::DaemonCore;
use kestic_core::history::{
    BackupRuntimeState, DestinationMaintenanceState, HistoryError, PauseState,
};
use kestic_core::model::{BackupId, DestinationId, RunKind, RunStatus};

impl DaemonCore {
    /// Loads cached state rows for all configured backups/destinations and the global pause.
    pub(super) fn load_state_caches(&mut self) {
        self.backup_state_cache.clear();
        self.dest_state_cache.clear();
        for backup in &self.config.backups {
            if let Ok(rs) = self.history.get_backup_runtime_state(&backup.id) {
                self.backup_state_cache.insert(backup.id.clone(), rs);
            }
        }
        for dest in &self.config.destinations {
            if let Ok(ds) = self.history.get_destination_maintenance_state(&dest.id) {
                self.dest_state_cache.insert(dest.id.clone(), ds);
            }
        }
        self.global_pause_cache = self.history.get_global_pause().ok().flatten();
    }

    /// Returns the runtime state of `backup_id` (cache; falls back to SQLite on a cache miss).
    pub(super) fn backup_runtime_state(&self, backup_id: &BackupId) -> BackupRuntimeState {
        match self.backup_state_cache.get(backup_id) {
            Some(rs) => rs.clone(),
            None => self
                .history
                .get_backup_runtime_state(backup_id)
                .unwrap_or_default(),
        }
    }

    /// Like [`Self::backup_runtime_state`], but populates the cache on a miss.
    pub(super) fn load_backup_runtime_state(&mut self, backup_id: &BackupId) -> BackupRuntimeState {
        if let Some(rs) = self.backup_state_cache.get(backup_id) {
            return rs.clone();
        }
        let rs = self
            .history
            .get_backup_runtime_state(backup_id)
            .unwrap_or_default();
        self.backup_state_cache
            .insert(backup_id.clone(), rs.clone());
        rs
    }

    /// Persists `rs` for `backup_id` (write-through; skipped when unchanged).
    pub(super) fn store_backup_runtime_state(
        &mut self,
        backup_id: &BackupId,
        rs: &BackupRuntimeState,
    ) -> Result<(), HistoryError> {
        if self.backup_state_cache.get(backup_id) == Some(rs) {
            return Ok(());
        }
        self.history.upsert_backup_runtime_state(backup_id, rs)?;
        self.backup_state_cache
            .insert(backup_id.clone(), rs.clone());
        Ok(())
    }

    /// Returns whether `backup_id` has a pending `forget` («forget offen»).
    pub(super) fn is_pending_forget_cached(&self, backup_id: &BackupId) -> bool {
        match self.backup_state_cache.get(backup_id) {
            Some(rs) => rs.pending_forget,
            None => self.history.is_pending_forget(backup_id).unwrap_or(false),
        }
    }

    /// Sets or clears the persistent `pending_forget` flag (write-through).
    pub(super) fn set_pending_forget_cached(
        &mut self,
        backup_id: &BackupId,
        pending: bool,
    ) -> Result<(), HistoryError> {
        self.history.set_pending_forget(backup_id, pending)?;
        let mut rs = self.load_backup_runtime_state(backup_id);
        rs.pending_forget = pending;
        self.backup_state_cache.insert(backup_id.clone(), rs);
        Ok(())
    }

    /// Removes the cached and persisted state row of a deleted backup.
    pub(super) fn clear_backup_state_cached(
        &mut self,
        backup_id: &BackupId,
    ) -> Result<(), HistoryError> {
        self.backup_state_cache.remove(backup_id);
        self.history.clear_backup_state(backup_id)
    }

    /// Returns the maintenance state of `dest_id` (cache; falls back to SQLite on a cache miss).
    pub(super) fn destination_maintenance_state(
        &self,
        dest_id: &DestinationId,
    ) -> DestinationMaintenanceState {
        match self.dest_state_cache.get(dest_id) {
            Some(ds) => ds.clone(),
            None => self
                .history
                .get_destination_maintenance_state(dest_id)
                .unwrap_or_default(),
        }
    }

    /// Like [`Self::destination_maintenance_state`], but populates the cache on a miss.
    pub(super) fn load_destination_maintenance_state(
        &mut self,
        dest_id: &DestinationId,
    ) -> DestinationMaintenanceState {
        if let Some(ds) = self.dest_state_cache.get(dest_id) {
            return ds.clone();
        }
        let ds = self
            .history
            .get_destination_maintenance_state(dest_id)
            .unwrap_or_default();
        self.dest_state_cache.insert(dest_id.clone(), ds.clone());
        ds
    }

    /// Persists `ds` for `dest_id` (write-through; skipped when unchanged).
    pub(super) fn store_destination_maintenance_state(
        &mut self,
        dest_id: &DestinationId,
        ds: &DestinationMaintenanceState,
    ) -> Result<(), HistoryError> {
        if self.dest_state_cache.get(dest_id) == Some(ds) {
            return Ok(());
        }
        self.history
            .upsert_destination_maintenance_state(dest_id, ds)?;
        self.dest_state_cache.insert(dest_id.clone(), ds.clone());
        Ok(())
    }

    /// Removes the cached and persisted state row of a deleted destination.
    pub(super) fn clear_destination_state_cached(
        &mut self,
        dest_id: &DestinationId,
    ) -> Result<(), HistoryError> {
        self.dest_state_cache.remove(dest_id);
        self.history.clear_destination_state(dest_id)
    }

    /// Returns the cached global pause (`FR-13a`).
    pub(super) fn global_pause_cached(&self) -> Option<PauseState> {
        self.global_pause_cache.clone()
    }

    /// Persists the global pause (write-through).
    pub(super) fn set_global_pause_cached(
        &mut self,
        pause: Option<&PauseState>,
    ) -> Result<(), HistoryError> {
        self.history.set_global_pause(pause)?;
        self.global_pause_cache = pause.cloned();
        Ok(())
    }

    /// Whether the run cache holds a `Pending` backup entry for `backup_id`.
    pub(super) fn has_pending_backup_run_cached(&self, backup_id: &BackupId) -> bool {
        self.runs_cache.iter().any(|r| {
            r.backup_id() == Some(backup_id)
                && matches!(r.kind, RunKind::Backup { .. })
                && r.status == RunStatus::Pending
        })
    }
}
