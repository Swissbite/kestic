// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Snapshot lists for the restore area: the snapshots of one backup and all snapshots of a
//! destination, sorted into backups, unassigned and orphaned snapshots.

use super::error::{KesticServiceErr, backup_run_error_kind, map_provider_err, parse_backup_id};
use super::state::DaemonCore;
use crate::dbus_api::{
    BackupSnapshotCountDto, BackupSnapshotsDto, DestinationSnapshotsDto, OrphanedBackupDto,
    PreviousDestinationSnapshotsDto, SnapshotSummaryDto,
};
use kestic_core::backup::{
    SnapshotListAccess, list_destination_snapshots, list_snapshots_for_backup,
};
use kestic_core::clock::Clock;
use kestic_core::model::{BackupId, Destination, DestinationId};
use kestic_core::provider::{ResolvedRestic, resolve_and_check};
use kestic_core::restic_gate::ResticBinaryGate;
use kestic_core::secrets::SecretStore;
use kestic_core::snapshot::{ResticSnapshot, sort_newest_first};
use kestic_core::snapshot_list::sort_destination_snapshots;
use kestic_core::sync::LockUnpoisoned;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Restic binary and secrets needed to list snapshots.
struct ListDeps<'a> {
    resolved: &'a ResolvedRestic,
    kestic_binary: &'a Path,
    secret_store: &'a dyn SecretStore,
    clock: &'a dyn Clock,
    /// IDs of the configured backups.
    configured: &'a [&'a str],
}

impl ListDeps<'_> {
    fn access<'a>(&'a self, destination: &'a Destination) -> SnapshotListAccess<'a> {
        SnapshotListAccess {
            destination,
            resolved: self.resolved,
            kestic_binary: self.kestic_binary,
            secret_store: self.secret_store,
            clock: self.clock,
            extra_env: &[],
        }
    }

    fn backup_snapshots(
        &self,
        b_id: &BackupId,
        destination: &Destination,
    ) -> Result<Vec<SnapshotSummaryDto>, kestic_core::backup::BackupRunError> {
        let mut snaps = list_snapshots_for_backup(
            b_id,
            destination,
            self.resolved,
            self.kestic_binary,
            self.secret_store,
            self.clock,
            &[],
        )?;
        // Like the destination list: neither a metadata snapshot nor one tagged with more than
        // one backup ID belongs to it.
        snaps.retain(|s| s.assigned_backup(self.configured) == Some(b_id.as_str()));
        sort_newest_first(&mut snaps);
        Ok(to_dtos(snaps))
    }
}

fn to_dtos(snaps: Vec<ResticSnapshot>) -> Vec<SnapshotSummaryDto> {
    snaps.into_iter().map(SnapshotSummaryDto::from).collect()
}

fn collect_backup_snapshots(
    b_id: &BackupId,
    current_dest: &Destination,
    other_dests: &[Destination],
    deps: &ListDeps<'_>,
) -> BackupSnapshotsDto {
    let (current_snapshots, current_error, current_error_kind) =
        match deps.backup_snapshots(b_id, current_dest) {
            Ok(snaps) => (snaps, None, None),
            Err(err) => (
                Vec::new(),
                Some(err.to_string()),
                Some(backup_run_error_kind(&err)),
            ),
        };

    // Earlier `kestic:<backup-id>` snapshots on the other configured destinations.
    let previous_destinations = other_dests
        .iter()
        .filter_map(|other| {
            let snaps = deps.backup_snapshots(b_id, other).ok()?;
            (!snaps.is_empty()).then(|| PreviousDestinationSnapshotsDto {
                destination_id: other.id.as_str().to_string(),
                destination_name: other.name.clone(),
                snapshots: snaps,
            })
        })
        .collect();

    BackupSnapshotsDto {
        backup_id: b_id.as_str().to_string(),
        current_destination_name: current_dest.name.clone(),
        current_snapshots,
        current_error,
        current_error_kind,
        previous_destinations,
    }
}

pub(crate) fn list_backup_snapshots_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<BackupSnapshotsDto, KesticServiceErr> {
    // Held for the whole call, so the app-managed restic is not replaced between its restic
    // processes.
    let mut run_lease = ResticBinaryGate::global().read();
    let b_id = parse_backup_id(backup_id)?;
    let (current_dest, other_dests, configured, deps) = {
        let guard = shared.lock_unpoisoned();
        let current_dest = guard.backup_destination(&b_id)?;
        let other_dests: Vec<Destination> = guard
            .config
            .destinations
            .iter()
            .filter(|d| d.id != current_dest.id)
            .cloned()
            .collect();
        let configured: Vec<String> = guard
            .config
            .backups
            .iter()
            .map(|b| b.id.as_str().to_string())
            .collect();
        (current_dest, other_dests, configured, guard.restic_deps())
    };
    let configured: Vec<&str> = configured.iter().map(String::as_str).collect();

    let resolved = resolve_and_check(&deps.provider).map_err(map_provider_err)?;
    run_lease.bind(&resolved.binary);
    let list_deps = ListDeps {
        resolved: &resolved,
        kestic_binary: &deps.kestic_binary,
        secret_store: deps.secret_store.as_ref(),
        clock: deps.clock.as_ref(),
        configured: &configured,
    };
    Ok(collect_backup_snapshots(
        &b_id,
        &current_dest,
        &other_dests,
        &list_deps,
    ))
}

pub(crate) fn list_destination_snapshots_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    destination_id: &str,
) -> Result<DestinationSnapshotsDto, KesticServiceErr> {
    // Held for the whole call, so the app-managed restic is not replaced between its restic
    // processes.
    let mut run_lease = ResticBinaryGate::global().read();
    let d_id = DestinationId::parse(destination_id)
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    let (dest, backups, deps) = {
        let guard = shared.lock_unpoisoned();
        let dest = guard
            .config
            .destination(&d_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;
        let backups: Vec<(String, String)> = guard
            .config
            .backups
            .iter()
            .map(|b| (b.id.as_str().to_string(), b.name.clone()))
            .collect();
        (dest, backups, guard.restic_deps())
    };

    let resolved = resolve_and_check(&deps.provider).map_err(map_provider_err)?;
    run_lease.bind(&resolved.binary);
    let list_deps = ListDeps {
        resolved: &resolved,
        kestic_binary: &deps.kestic_binary,
        secret_store: deps.secret_store.as_ref(),
        clock: deps.clock.as_ref(),
        configured: &[],
    };
    let mut dto = DestinationSnapshotsDto {
        destination_id: dest.id.as_str().to_string(),
        error_kind: None,
        error_detail: None,
        backups: Vec::new(),
        unassigned: Vec::new(),
        orphaned: Vec::new(),
        described: Vec::new(),
    };
    match list_destination_snapshots(&list_deps.access(&dest)) {
        Ok(snaps) => fill_destination_snapshots(&mut dto, snaps, &backups),
        Err(err) => {
            dto.error_kind = Some(backup_run_error_kind(&err));
            dto.error_detail = Some(err.to_string());
        }
    }
    Ok(dto)
}

/// Sorts `snaps` into `dto`; `backups` are the configured backups as (ID, name).
fn fill_destination_snapshots(
    dto: &mut DestinationSnapshotsDto,
    snaps: Vec<ResticSnapshot>,
    backups: &[(String, String)],
) {
    let ids: Vec<&str> = backups.iter().map(|(id, _)| id.as_str()).collect();
    let sorted = sort_destination_snapshots(snaps, &ids);
    dto.backups = sorted
        .backups
        .into_iter()
        .zip(backups)
        .map(|((id, snaps), (_, name))| BackupSnapshotCountDto {
            backup_id: id,
            backup_name: name.clone(),
            count: snaps.len() as u64,
            latest_time: snaps.first().map(|s| s.time.clone()),
        })
        .collect();
    dto.unassigned = to_dtos(sorted.unassigned);
    let group = |(id, snaps): (String, Vec<ResticSnapshot>)| OrphanedBackupDto {
        backup_id: id,
        count: snaps.len() as u64,
        latest_time: snaps.first().map(|s| s.time.clone()),
        latest_hostname: snaps.first().and_then(|s| s.hostname.clone()),
        snapshots: to_dtos(snaps),
    };
    dto.orphaned = sorted.orphaned.into_iter().map(group).collect();
    dto.described = sorted.described.into_iter().map(group).collect();
}
