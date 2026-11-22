// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Snapshot lists of the restore area for [`super::qobject::KesticApp`]. The daemon calls run
//! in a background thread; `poll_restore` hands the results to QML.

use super::qobject;
use crate::state_cache::{daemon_unavailable_text, dbus_error_text, make_client, shared_state, tr};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{BackupSnapshotsDto, DestinationSnapshotsDto};
use kestic_i18n::codes::ErrorKind;
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
    /// Destinations of the current overview load that have not answered yet.
    overview_pending: usize,
    /// A destination answered since the last poll.
    overview_changed: bool,
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
    /// Loads the snapshots of every destination for the backup list of the restore area. Each
    /// destination is asked in its own thread, so a slow or unreachable one does not hold back
    /// the counts of the others.
    pub fn load_restore_overview(mut self: Pin<&mut Self>) {
        let dest_ids = configured_destination_ids();
        let generation = {
            let mut cache = restore_cache().lock_unpoisoned();
            cache.overview_generation += 1;
            cache.overview_pending = dest_ids.len();
            cache.overview_done = dest_ids.is_empty();
            cache.overview_changed = false;
            cache.destinations.clear();
            cache.overview_generation
        };
        self.as_mut()
            .set_restore_overview_loading(!dest_ids.is_empty());
        self.as_mut().restore_overview_updated();
        for id in dest_ids {
            std::thread::spawn(move || {
                let result = fetch_destination(&id);
                let mut cache = restore_cache().lock_unpoisoned();
                if cache.overview_generation == generation {
                    cache.destinations.insert(id, result);
                    cache.overview_pending -= 1;
                    cache.overview_changed = true;
                    cache.overview_done = cache.overview_pending == 0;
                }
            });
        }
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
        let (overview_changed, overview_done, snapshots_done) = {
            let mut cache = restore_cache().lock_unpoisoned();
            (
                std::mem::take(&mut cache.overview_changed),
                std::mem::take(&mut cache.overview_done),
                std::mem::take(&mut cache.snapshots_done),
            )
        };
        if overview_done {
            self.as_mut().set_restore_overview_loading(false);
        }
        if overview_changed || overview_done {
            self.as_mut().restore_overview_updated();
        }
        if snapshots_done {
            self.as_mut().set_restore_snapshots_loading(false);
        }
        // The destination's error code may arrive after the snapshot list: refresh the text.
        if snapshots_done || overview_changed {
            let error = snapshots_error_text();
            self.as_mut()
                .set_restore_snapshots_error(cxx_qt_lib::QString::from(&error));
        }
        if snapshots_done {
            self.as_mut().restore_snapshots_updated();
        }
    }
}

/// Error line of the snapshot list: the text of the destination's error code when the
/// destination list names one, otherwise the raw detail.
fn snapshots_error_text() -> String {
    let (backup_id, raw) = match &restore_cache().lock_unpoisoned().backup_snapshots {
        Some((id, Ok(dto))) => match &dto.current_error {
            Some(raw) => (id.clone(), raw.clone()),
            None => return String::new(),
        },
        Some((_, Err(err))) => return err.clone(),
        None => return String::new(),
    };
    let dest_id = {
        let guard = shared_state().lock_unpoisoned();
        guard.state.as_ref().and_then(|s| {
            s.backups
                .iter()
                .find(|b| b.backup.id.as_str() == backup_id)
                .map(|b| b.backup.destination_id.as_str().to_string())
        })
    };
    let kind = dest_id.and_then(|id| {
        restore_cache()
            .lock_unpoisoned()
            .destinations
            .get(&id)
            .and_then(|r| r.as_ref().ok())
            .and_then(|d| d.error_kind)
    });
    match kind {
        Some(kind) if kind != ErrorKind::Unknown => tr(&kind.message(None, false)),
        _ => raw,
    }
}
