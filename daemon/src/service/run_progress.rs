// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The progress of a running backup or restore: stored on its active run, shown in the tray and
//! sent as `RunProgress`.

use super::state::DaemonCore;
use crate::dbus_api::ActiveRunProgressDto;
use crate::dbus_server::emit_run_progress;
use kestic_core::model::RunId;
use kestic_core::sync::LockUnpoisoned;
use std::sync::Mutex;
use zbus::Connection;

/// Lets `update` write the progress of the active run `run_id`, pushes the tray state and, once
/// the lock is released, sends `RunProgress`. Returns the progress, `None` when the run is no
/// longer active.
pub(super) fn publish_run_progress(
    shared: &Mutex<DaemonCore>,
    run_id: &RunId,
    conn: Option<&Connection>,
    update: impl FnOnce(&mut ActiveRunProgressDto),
) -> Option<ActiveRunProgressDto> {
    let progress = {
        let mut guard = shared.lock_unpoisoned();
        let progress = guard.active_runs.get_mut(run_id).map(|entry| {
            update(&mut entry.progress);
            entry.progress.clone()
        });
        guard.push_tray_state();
        progress
    };
    if let (Some(conn), Some(progress)) = (conn, &progress) {
        emit_run_progress(conn, progress);
    }
    progress
}
