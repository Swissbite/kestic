// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Sorts the snapshots of one destination into backups, orphaned and unassigned snapshots.

use crate::snapshot::{ResticSnapshot, sort_newest_first};
use std::collections::{BTreeMap, BTreeSet};

/// Snapshots of one destination, each list newest first. Metadata snapshots are left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DestinationSnapshots {
    /// Per configured backup (in the order given to [`sort_destination_snapshots`]).
    pub backups: Vec<(String, Vec<ResticSnapshot>)>,
    /// Snapshots without a `kestic:` tag.
    pub unassigned: Vec<ResticSnapshot>,
    /// Per backup ID that is neither configured nor described by a metadata snapshot of the
    /// destination, sorted by ID.
    pub orphaned: Vec<(String, Vec<ResticSnapshot>)>,
    /// Per backup ID that is not configured but has a metadata snapshot, sorted by ID; such a
    /// backup can still be taken over, so its snapshots are not orphaned.
    pub described: Vec<(String, Vec<ResticSnapshot>)>,
}

/// Sorts all snapshots of a destination. The order of the checks is: metadata snapshot (left
/// out), configured backup, orphaned, unassigned; a snapshot's other tags do not change it.
pub fn sort_destination_snapshots(
    snapshots: Vec<ResticSnapshot>,
    configured_backup_ids: &[&str],
) -> DestinationSnapshots {
    let described_ids: BTreeSet<String> = snapshots
        .iter()
        .flat_map(|s| s.meta_backup_ids().map(str::to_string))
        .collect();
    let mut by_backup: BTreeMap<String, Vec<ResticSnapshot>> = BTreeMap::new();
    let mut orphaned: BTreeMap<String, Vec<ResticSnapshot>> = BTreeMap::new();
    let mut described: BTreeMap<String, Vec<ResticSnapshot>> = BTreeMap::new();
    let mut unassigned = Vec::new();

    for snap in snapshots {
        if snap.is_meta() {
            continue;
        }
        let configured = snap
            .backup_ids()
            .find(|id| configured_backup_ids.contains(id))
            .map(str::to_string);
        if let Some(id) = configured {
            by_backup.entry(id).or_default().push(snap);
            continue;
        }
        let Some(id) = snap.backup_ids().next().map(str::to_string) else {
            unassigned.push(snap);
            continue;
        };
        if described_ids.contains(&id) {
            described.entry(id).or_default().push(snap);
        } else {
            orphaned.entry(id).or_default().push(snap);
        }
    }

    let backups = configured_backup_ids
        .iter()
        .map(|id| {
            let mut snaps = by_backup.remove(*id).unwrap_or_default();
            sort_newest_first(&mut snaps);
            ((*id).to_string(), snaps)
        })
        .collect();
    sort_newest_first(&mut unassigned);
    DestinationSnapshots {
        backups,
        unassigned,
        orphaned: sorted_groups(orphaned),
        described: sorted_groups(described),
    }
}

fn sorted_groups(
    groups: BTreeMap<String, Vec<ResticSnapshot>>,
) -> Vec<(String, Vec<ResticSnapshot>)> {
    groups
        .into_iter()
        .map(|(id, mut snaps)| {
            sort_newest_first(&mut snaps);
            (id, snaps)
        })
        .collect()
}
