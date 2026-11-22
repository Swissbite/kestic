// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Snapshot lists of the restore area for [`super::qobject::KesticApp`]. The daemon calls run
//! in a background thread; `poll_restore` hands the results to QML.

use super::qobject;
use crate::state_cache::{daemon_unavailable_text, dbus_error_text, make_client, shared_state};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{BackupSnapshotsDto, DestinationSnapshotsDto};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};

/// Snapshot lists loaded for the restore area.
#[derive(Default)]
pub struct RestoreCache {
    /// All snapshots per destination ID, or the error text of the call.
    pub destinations: HashMap<String, Result<DestinationSnapshotsDto, String>>,
    /// The backup whose snapshots are loaded, and the result.
    pub backup_snapshots: Option<(String, Result<BackupSnapshotsDto, String>)>,
    overview_generation: u64,
    snapshots_generation: u64,
    overview_done: bool,
    snapshots_done: bool,
}

pub fn restore_cache() -> &'static Mutex<RestoreCache> {
    static CACHE: OnceLock<Mutex<RestoreCache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

fn configured_destination_ids() -> Vec<String> {
    let guard = shared_state().lock_unpoisoned();
    guard.state.as_ref().map_or_else(Vec::new, |s| {
        s.config
            .destinations
            .iter()
            .map(|d| d.id.as_str().to_string())
            .collect()
    })
}

fn fetch_destination(dest_id: &str) -> Result<DestinationSnapshotsDto, String> {
    let client = make_client().map_err(|_| daemon_unavailable_text())?;
    client
        .list_destination_snapshots(dest_id)
        .map_err(|e| dbus_error_text(&e))
}

fn fetch_backup(backup_id: &str) -> Result<BackupSnapshotsDto, String> {
    let client = make_client().map_err(|_| daemon_unavailable_text())?;
    client
        .list_backup_snapshots(backup_id)
        .map_err(|e| dbus_error_text(&e))
}

impl qobject::KesticApp {
    /// Loads the snapshots of every destination for the backup list of the restore area.
    pub fn load_restore_overview(mut self: Pin<&mut Self>) {
        let dest_ids = configured_destination_ids();
        let generation = {
            let mut cache = restore_cache().lock_unpoisoned();
            cache.overview_generation += 1;
            cache.overview_done = false;
            cache.destinations.clear();
            cache.overview_generation
        };
        self.as_mut().set_restore_overview_loading(true);
        self.as_mut().restore_overview_updated();
        std::thread::spawn(move || {
            let results: HashMap<_, _> = dest_ids
                .into_iter()
                .map(|id| {
                    let result = fetch_destination(&id);
                    (id, result)
                })
                .collect();
            let mut cache = restore_cache().lock_unpoisoned();
            if cache.overview_generation == generation {
                cache.destinations = results;
                cache.overview_done = true;
            }
        });
    }

    /// Loads the snapshots of `backup_id` for the snapshot list of the restore area.
    pub fn load_restore_snapshots(mut self: Pin<&mut Self>, backup_id: &cxx_qt_lib::QString) {
        let backup_id = backup_id.to_string();
        let generation = {
            let mut cache = restore_cache().lock_unpoisoned();
            cache.snapshots_generation += 1;
            cache.snapshots_done = false;
            cache.backup_snapshots = None;
            cache.snapshots_generation
        };
        self.as_mut().set_restore_snapshots_loading(true);
        self.as_mut()
            .set_restore_snapshots_error(cxx_qt_lib::QString::from(""));
        self.as_mut().restore_snapshots_updated();
        std::thread::spawn(move || {
            let result = fetch_backup(&backup_id);
            let mut cache = restore_cache().lock_unpoisoned();
            if cache.snapshots_generation == generation {
                cache.backup_snapshots = Some((backup_id, result));
                cache.snapshots_done = true;
            }
        });
    }

    /// Hands finished snapshot lists to QML (called by `poll_events`).
    pub(super) fn poll_restore(mut self: Pin<&mut Self>) {
        let (overview_done, snapshots_error) = {
            let mut cache = restore_cache().lock_unpoisoned();
            let overview_done = std::mem::take(&mut cache.overview_done);
            let snapshots_error = if std::mem::take(&mut cache.snapshots_done) {
                Some(match &cache.backup_snapshots {
                    Some((_, Ok(dto))) => dto.current_error.clone().unwrap_or_default(),
                    Some((_, Err(err))) => err.clone(),
                    None => String::new(),
                })
            } else {
                None
            };
            (overview_done, snapshots_error)
        };
        if overview_done {
            self.as_mut().set_restore_overview_loading(false);
            self.as_mut().restore_overview_updated();
        }
        if let Some(error) = snapshots_error {
            self.as_mut().set_restore_snapshots_loading(false);
            self.as_mut()
                .set_restore_snapshots_error(cxx_qt_lib::QString::from(&error));
            self.as_mut().restore_snapshots_updated();
        }
    }
}
