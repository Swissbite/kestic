// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination/source availability probing, background probe workers, queue draining, and repository unlock.

use super::backup_runner::start_backup_async;
use super::error::{KesticServiceErr, map_destination_err};
use super::state::{AVAILABILITY_CACHE_TTL_SECS, DaemonCore, probe_for_run};
use crate::dbus_api::{AvailabilitySnapshotDto, BackupAvailabilityDto, DestinationAvailabilityDto};
use crate::dbus_server::emit_status_changed;
use kestic_core::backup::BackupRunError;
use kestic_core::destination::{
    DEFAULT_CONNECT_TIMEOUT, preflight_check_destination, preflight_check_with_restic,
};
use kestic_core::maintenance::{LockClassification, detect_stale_locks, execute_unlock};
use kestic_core::model::{Backup, Destination, DestinationId};
use kestic_core::mount::{
    MountInspector, SourceCheckResult, check_source_status_with, current_home_dir,
};
use kestic_core::provider::{ResolvedRestic, resolve_and_check};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::BlockedReason;
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

impl DaemonCore {
    /// Refreshes availability if `force == true` or > 60 s elapsed since last check.
    pub fn refresh_availability_if_needed(&mut self, force: bool) -> AvailabilitySnapshotDto {
        let now = self.clock.now_utc();
        let should_run = force
            || match self.last_availability_check {
                None => true,
                Some(prev) => (now - prev) > time::Duration::seconds(AVAILABILITY_CACHE_TTL_SECS),
            };

        if should_run {
            let now_unix = now.unix_timestamp();
            // A destination that only restic can check is left to the background refresh: restic
            // may take up to 15 s, too long while the daemon state is locked.
            for dest in self
                .config
                .destinations
                .iter()
                .filter(|d| !d.kind.needs_restic_preflight())
            {
                let res = preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT);
                self.availability_probe_count += 1;
                let dto = DestinationAvailabilityDto::from_probe(&dest.id, &res, now_unix);
                self.dest_availability.insert(dest.id.clone(), dto);
            }

            for backup in &self.config.backups {
                let status = check_source_status_with(
                    backup,
                    current_home_dir().as_deref(),
                    self.mount_inspector.as_ref(),
                );
                self.source_availability.insert(backup.id.clone(), status);
            }
            self.last_availability_check = Some(now);
        }

        self.current_availability_snapshot()
    }

    pub(super) fn current_availability_snapshot(&self) -> AvailabilitySnapshotDto {
        let backups = self
            .config
            .backups
            .iter()
            .map(|b| self.availability_for_backup(b))
            .collect();
        let destinations = self
            .config
            .destinations
            .iter()
            .map(|d| self.availability_for_destination(d))
            .collect();
        AvailabilitySnapshotDto {
            checked_at_unix: self.last_availability_check.map(|t| t.unix_timestamp()),
            backups,
            destinations,
        }
    }

    pub(super) fn availability_for_destination(
        &self,
        dest: &Destination,
    ) -> DestinationAvailabilityDto {
        self.dest_availability
            .get(&dest.id)
            .cloned()
            .unwrap_or_else(|| DestinationAvailabilityDto {
                destination_id: dest.id.as_str().to_string(),
                reachable: true,
                detail: None,
                checked_at_unix: self.clock.now_unix_secs(),
            })
    }

    pub(super) fn availability_for_backup(&self, backup: &Backup) -> BackupAvailabilityDto {
        let source_status = self
            .source_availability
            .get(&backup.id)
            .cloned()
            .unwrap_or(SourceCheckResult::Available);
        let source_available = source_status.is_ok();
        let dest_available = self
            .dest_availability
            .get(&backup.destination_id)
            .is_none_or(|d| d.reachable);

        if !source_available {
            BackupAvailabilityDto {
                backup_id: backup.id.as_str().to_string(),
                source_available: false,
                destination_available: dest_available,
                available: false,
                blocked_reason: Some(BlockedReason::SourceUnavailable),
                source_status: source_status.status_code(),
            }
        } else if !dest_available {
            BackupAvailabilityDto {
                backup_id: backup.id.as_str().to_string(),
                source_available: true,
                destination_available: false,
                available: false,
                blocked_reason: Some(BlockedReason::DestinationUnreachable),
                source_status: None,
            }
        } else {
            BackupAvailabilityDto {
                backup_id: backup.id.as_str().to_string(),
                source_available: true,
                destination_available: true,
                available: true,
                blocked_reason: None,
                source_status: None,
            }
        }
    }
}

pub(super) struct DestinationClaimGuard {
    shared: Arc<Mutex<DaemonCore>>,
    dest_id: DestinationId,
    armed: bool,
}

impl DestinationClaimGuard {
    pub(super) fn new(shared: Arc<Mutex<DaemonCore>>, dest_id: DestinationId) -> Self {
        Self {
            shared,
            dest_id,
            armed: true,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl DaemonCore {
    /// Claims `dest` for a run that is about to start; the claim ends with `release_claim`
    /// or when the returned guard drops.
    pub(super) fn claim_destination(
        &mut self,
        shared: &Arc<Mutex<DaemonCore>>,
        dest: &Destination,
        is_scheduled: bool,
    ) -> Result<DestinationClaimGuard, KesticServiceErr> {
        if is_scheduled && self.sleep_in_progress {
            return Err(KesticServiceErr::SleepInProgress);
        }
        if self.is_destination_busy(&dest.id) {
            return Err(KesticServiceErr::AlreadyRunning(format!(
                "another run is already active on destination '{}'",
                dest.name
            )));
        }
        self.claiming_destinations.insert(dest.id.clone());
        Ok(DestinationClaimGuard::new(
            Arc::clone(shared),
            dest.id.clone(),
        ))
    }
}

impl Drop for DestinationClaimGuard {
    fn drop(&mut self) {
        if self.armed {
            let mut guard = self.shared.lock_unpoisoned();
            guard.claiming_destinations.remove(&self.dest_id);
        }
    }
}

pub(crate) fn refresh_availability_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    force: bool,
) -> AvailabilitySnapshotDto {
    let (should_run, destinations, backups, mount_inspector, clock, current_snapshot, deps) = {
        let guard = shared.lock_unpoisoned();
        let now = guard.clock.now_utc();
        let should_run = force
            || match guard.last_availability_check {
                None => true,
                Some(prev) => (now - prev) > time::Duration::seconds(AVAILABILITY_CACHE_TTL_SECS),
            };
        (
            should_run,
            guard.config.destinations.clone(),
            guard.config.backups.clone(),
            Arc::clone(&guard.mount_inspector),
            Arc::clone(&guard.clock),
            guard.current_availability_snapshot(),
            guard.restic_deps(),
        )
    };

    if !should_run {
        return current_snapshot;
    }

    let now = clock.now_utc();
    let now_unix = now.unix_timestamp();
    let home = current_home_dir();
    let mut dest_updates = Vec::with_capacity(destinations.len());
    // Resolved only when a destination needs restic for its check; without a usable restic such
    // a destination keeps its last status.
    let mut resolved: Option<Option<ResolvedRestic>> = None;
    for dest in &destinations {
        let res = if dest.kind.needs_restic_preflight() {
            let restic = resolved.get_or_insert_with(|| resolve_and_check(&deps.provider).ok());
            let Some(restic) = restic.as_ref() else {
                continue;
            };
            preflight_check_with_restic(dest, &deps.runtime(restic))
        } else {
            preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)
        };
        let dto = DestinationAvailabilityDto::from_probe(&dest.id, &res, now_unix);
        dest_updates.push((dest.id.clone(), dto));
    }

    let mut source_updates = Vec::with_capacity(backups.len());
    for backup in &backups {
        let status = check_source_status_bounded(backup, home.as_deref(), &mount_inspector);
        source_updates.push((backup.id.clone(), status));
    }

    let mut guard = shared.lock_unpoisoned();
    let dest_count = dest_updates.len() as u64;
    for (id, dto) in dest_updates {
        guard.dest_availability.insert(id, dto);
    }
    for (id, status) in source_updates {
        guard.source_availability.insert(id, status);
    }
    guard.availability_probe_count += dest_count;
    guard.last_availability_check = Some(now);
    guard.current_availability_snapshot()
}

/// A background availability probe running longer than this is treated as hung (e.g. a local
/// destination on a stale NFS mount blocking `stat`) and no longer blocks new probes.
pub(super) const AVAILABILITY_PROBE_STALE_SECS: i64 = 120;

/// Upper bound for one source check (`stat` / mountinfo) on a start path; a source that does
/// not answer in time (hung mount) counts as unavailable instead of blocking forever.
pub(super) const SOURCE_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// [`check_source_status_with`] bounded by [`SOURCE_CHECK_TIMEOUT`] (the check runs on a helper
/// thread; on timeout the source is reported unavailable and the helper is left behind).
pub(super) fn check_source_status_bounded(
    backup: &Backup,
    home: Option<&Path>,
    inspector: &Arc<dyn MountInspector>,
) -> SourceCheckResult {
    let (tx, rx) = mpsc::channel();
    let backup_bg = backup.clone();
    let home_bg = home.map(Path::to_path_buf);
    let inspector_bg = Arc::clone(inspector);
    thread::spawn(move || {
        let status =
            check_source_status_with(&backup_bg, home_bg.as_deref(), inspector_bg.as_ref());
        // The receiver is gone after a timeout; nothing left to report then.
        let _ = tx.send(status);
    });
    rx.recv_timeout(SOURCE_CHECK_TIMEOUT).unwrap_or_else(|_| {
        SourceCheckResult::PathMissing(backup.source.path.display().to_string())
    })
}

pub(crate) fn trigger_background_availability_probe(shared: Arc<Mutex<DaemonCore>>) {
    let generation = {
        let mut guard = shared.lock_unpoisoned();
        let now = guard.clock.now_utc();
        let ttl_elapsed = match guard.last_availability_check {
            None => true,
            Some(prev) => (now - prev) > time::Duration::seconds(AVAILABILITY_CACHE_TTL_SECS),
        };
        let fresh_in_flight = guard.availability_probe_in_flight
            && guard
                .availability_probe_started_at
                .is_some_and(|t| now - t < time::Duration::seconds(AVAILABILITY_PROBE_STALE_SECS));
        if !ttl_elapsed || fresh_in_flight {
            return;
        }
        guard.availability_probe_in_flight = true;
        guard.availability_probe_started_at = Some(now);
        guard.availability_probe_generation = guard.availability_probe_generation.wrapping_add(1);
        guard.availability_probe_generation
    };

    thread::spawn(move || {
        let _ = refresh_availability_unlocked(&shared, false);
        let (conn_opt, state_dto, tray_opt, tray_state) = {
            let mut guard = shared.lock_unpoisoned();
            if guard.availability_probe_generation == generation {
                guard.availability_probe_in_flight = false;
                guard.availability_probe_started_at = None;
            }
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (
                guard.conn.clone(),
                state_dto,
                guard.tray.clone(),
                tray_state,
            )
        };
        if let Some(tray) = tray_opt {
            tray.set_full_state(tray_state);
        }
        if let Some(conn) = &conn_opt {
            emit_status_changed(conn, &state_dto);
        }
    });
}

/// A background source probe running longer than this is treated as hung (e.g. a stale NFS
/// mount blocking `stat`) and replaced by a fresh probe; the late result is discarded.
pub(super) const SOURCE_PROBE_STALE_SECS: i64 = 120;

pub(super) fn trigger_background_source_probe(shared: Arc<Mutex<DaemonCore>>) {
    let (backups, mount_inspector, generation) = {
        let mut guard = shared.lock_unpoisoned();
        let now = guard.clock.now_utc();
        let fresh_in_flight = guard.source_probe_in_flight
            && guard
                .source_probe_started_at
                .is_some_and(|t| now - t < time::Duration::seconds(SOURCE_PROBE_STALE_SECS));
        if fresh_in_flight {
            return;
        }
        guard.source_probe_in_flight = true;
        guard.source_probe_started_at = Some(now);
        guard.source_probe_generation = guard.source_probe_generation.wrapping_add(1);
        (
            guard.config.backups.clone(),
            Arc::clone(&guard.mount_inspector),
            guard.source_probe_generation,
        )
    };

    thread::spawn(move || {
        let home = current_home_dir();
        let mut updates = Vec::with_capacity(backups.len());
        for backup in &backups {
            let status =
                check_source_status_with(backup, home.as_deref(), mount_inspector.as_ref());
            updates.push((backup.id.clone(), status));
        }

        let changed_conn = {
            let mut guard = shared.lock_unpoisoned();
            if guard.source_probe_generation != generation {
                // Superseded after being considered hung: a newer probe owns the state.
                return;
            }
            guard.source_probe_in_flight = false;
            guard.source_probe_started_at = None;
            let mut any_changed = false;
            for (bid, status) in updates {
                if guard.source_availability.get(&bid) != Some(&status) {
                    any_changed = true;
                }
                guard.source_availability.insert(bid, status);
            }
            if any_changed {
                let state_dto = guard.push_tray_state();
                guard.conn.clone().map(|c| (c, state_dto))
            } else {
                None
            }
        };
        if let Some((conn, state_dto)) = changed_conn {
            emit_status_changed(&conn, &state_dto);
        }
    });
}

pub(crate) fn drain_queued_backups_async(shared: &Arc<Mutex<DaemonCore>>) {
    loop {
        let next_backup_id = {
            let mut guard = shared.lock_unpoisoned();
            if guard.sleep_in_progress || guard.queued_backups.is_empty() {
                None
            } else {
                let mut picked_idx = None;
                for (idx, bid) in guard.queued_backups.iter().enumerate() {
                    if let Some(backup) = guard.config.backup(bid) {
                        if !guard.is_destination_busy(&backup.destination_id) {
                            picked_idx = Some(idx);
                            break;
                        }
                    } else {
                        picked_idx = Some(idx);
                        break;
                    }
                }
                picked_idx.and_then(|idx| guard.queued_backups.remove(idx))
            }
        };

        let Some(bid) = next_backup_id else {
            break;
        };

        let still_configured = {
            let guard = shared.lock_unpoisoned();
            guard.config.backup(&bid).is_some()
        };
        if !still_configured {
            continue;
        }

        if let Err(KesticServiceErr::AlreadyRunning(_)) = start_backup_async(shared, bid.as_str()) {
            // Lost the race for the destination (another start claimed it after the pick):
            // keep the backup queued; the finishing run drains the queue again.
            let mut guard = shared.lock_unpoisoned();
            if !guard.queued_backups.contains(&bid) && guard.config.backup(&bid).is_some() {
                guard.queued_backups.push_front(bid);
            }
            break;
        }
    }
}

/// Unlocks stale local locks on a destination repository without holding `DaemonCore` lock.
pub fn unlock_destination_async(
    shared: &Arc<Mutex<DaemonCore>>,
    destination_id: &str,
) -> Result<u32, KesticServiceErr> {
    let d_id = DestinationId::parse(destination_id)
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

    let (dest, deps, mut claim_guard) = {
        let mut guard = shared.lock_unpoisoned();
        let dest = guard
            .config
            .destination(&d_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;

        if guard.is_destination_busy(&dest.id) {
            return Err(KesticServiceErr::AlreadyRunning(format!(
                "another run is already active on destination '{}'",
                dest.name
            )));
        }

        guard.claiming_destinations.insert(d_id.clone());
        (
            dest,
            guard.restic_deps(),
            DestinationClaimGuard::new(Arc::clone(shared), d_id.clone()),
        )
    };

    let resolved_bin = probe_for_run(&deps.provider, deps.secret_store.as_ref()).1?;
    let runtime = deps.runtime(&resolved_bin);

    let classification = detect_stale_locks(&dest, &runtime, &[]).map_err(|err| match err {
        BackupRunError::Destination(d) => map_destination_err(d),
        other => KesticServiceErr::Internal(other.to_string()),
    })?;

    if !classification.active_local_locks.is_empty()
        || !classification.foreign_host_locks.is_empty()
    {
        release_unlock_claim(shared, &d_id, &mut claim_guard);
        return Err(KesticServiceErr::LockInUse(
            "repository lock belongs to an active or remote process".to_string(),
        ));
    }

    if !classification.has_stale_local_lock {
        release_unlock_claim(shared, &d_id, &mut claim_guard);
        return Ok(0);
    }

    let stale_count = u32::try_from(classification.stale_lock_ids.len()).unwrap_or(u32::MAX);
    let res = execute_unlock(&dest, &runtime, &[]).map_err(|err| match err {
        BackupRunError::Destination(d) => map_destination_err(d),
        other => KesticServiceErr::Internal(other.to_string()),
    })?;
    if res.exit_code != 0 {
        return Err(KesticServiceErr::Internal(
            res.presentation.detail_or_summary(),
        ));
    }

    release_unlock_claim(shared, &d_id, &mut claim_guard);
    Ok(stale_count)
}

fn release_unlock_claim(
    shared: &Arc<Mutex<DaemonCore>>,
    d_id: &DestinationId,
    claim_guard: &mut DestinationClaimGuard,
) {
    {
        let mut guard = shared.lock_unpoisoned();
        guard.dest_stale_locks.insert(d_id.clone(), 0);
        guard.release_claim(d_id, claim_guard);
    }
    drain_queued_backups_async(shared);
}

pub(super) fn unlockable_stale_lock_count(cls: &LockClassification) -> usize {
    if cls.has_stale_local_lock
        && cls.foreign_host_locks.is_empty()
        && cls.active_local_locks.is_empty()
    {
        cls.stale_lock_ids.len()
    } else {
        0
    }
}
