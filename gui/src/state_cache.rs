// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The GUI's cache of the daemon state and the launch options, shared between the Qt thread
//! and the background threads.

use kestic_core::model::{BackupId, DestinationKind};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{
    ActiveRunProgressDto, BackupSnapshotsDto, BackupSummaryDto, DaemonStateDto, KESTIC_BUS_NAME,
    KESTIC_OBJECT_PATH, RunDetailDto,
};
use kestic_i18n::codes::BlockedReason;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Runtime options passed from `main.rs` CLI parsing.
#[derive(Debug, Clone)]
pub struct GuiLaunchConfig {
    pub bus_name: String,
    pub object_path: String,
    pub no_autostart_daemon: bool,
    pub show_quit_dialog_on_start: bool,
    pub show_about_on_start: bool,
    /// Open the window on the tab «Restore» (`--show-restore`, tray «Restore Files…»).
    pub show_restore_on_start: bool,
}

impl Default for GuiLaunchConfig {
    fn default() -> Self {
        Self {
            bus_name: KESTIC_BUS_NAME.to_string(),
            object_path: KESTIC_OBJECT_PATH.to_string(),
            no_autostart_daemon: false,
            show_quit_dialog_on_start: false,
            show_about_on_start: false,
            show_restore_on_start: false,
        }
    }
}

pub struct SharedGuiState {
    pub launch: GuiLaunchConfig,
    pub daemon_reachable: bool,
    pub daemon_error: Option<String>,
    pub state: Option<DaemonStateDto>,
    pub active_progress: HashMap<String, ActiveRunProgressDto>,
    pub snapshots: Option<BackupSnapshotsDto>,
    pub run_detail: Option<RunDetailDto>,
    /// The cached state or the reachability changed: show it in the window.
    pub state_fetched: bool,
    pub progress_dirty: bool,
    /// The GUI started the daemon and waits for it to take its bus name.
    pub daemon_starting: bool,
    /// The first state of the daemon is still on its way.
    pub first_fetch_pending: bool,
    /// The quit dialog opened before the first state arrived; that state names the running
    /// backup.
    pub quit_dialog_waits_for_state: bool,
    pub pending_blocked: Option<(String, BlockedReason)>,
    /// A blocked manual run whose message waits for the state fetched after it, so the message
    /// names the backup and its unmet conditions as they are now.
    pub blocked_after_refresh: Option<(String, BlockedReason)>,
    pub pending_show_window: bool,
    pub pending_show_about: bool,
    pub pending_show_restore: bool,
    pub pending_quit: Option<(bool, String)>,
    pub maintenance_preview: Option<(String, Result<String, String>)>,
    /// The signal listener receives the daemon's signals (and has registered the GUI).
    pub listening: bool,
}

impl SharedGuiState {
    fn new(launch: GuiLaunchConfig) -> Self {
        Self {
            launch,
            daemon_reachable: false,
            daemon_error: None,
            state: None,
            active_progress: HashMap::new(),
            snapshots: None,
            run_detail: None,
            state_fetched: false,
            progress_dirty: false,
            daemon_starting: false,
            first_fetch_pending: true,
            quit_dialog_waits_for_state: false,
            pending_blocked: None,
            blocked_after_refresh: None,
            pending_show_window: false,
            pending_show_about: false,
            pending_show_restore: false,
            pending_quit: None,
            maintenance_preview: None,
            listening: false,
        }
    }
}

static SHARED_STATE: OnceLock<Arc<Mutex<SharedGuiState>>> = OnceLock::new();

static LAUNCH_CONFIG: OnceLock<Mutex<GuiLaunchConfig>> = OnceLock::new();

pub fn set_launch_config(config: GuiLaunchConfig) {
    let lock = LAUNCH_CONFIG.get_or_init(|| Mutex::new(GuiLaunchConfig::default()));
    if let Ok(mut guard) = lock.lock() {
        *guard = config.clone();
    }
    if let Some(shared) = SHARED_STATE.get()
        && let Ok(mut guard) = shared.lock()
    {
        guard.launch = config;
    }
}

pub fn shared_state() -> &'static Arc<Mutex<SharedGuiState>> {
    SHARED_STATE.get_or_init(|| {
        let initial = LAUNCH_CONFIG
            .get()
            .and_then(|l| l.lock().ok().map(|g| g.clone()))
            .unwrap_or_default();
        Arc::new(Mutex::new(SharedGuiState::new(initial)))
    })
}

/// Kind of the cached destination `dest_id`, if the daemon state knows it.
pub fn cached_destination_kind(dest_id: &str) -> Option<DestinationKind> {
    let guard = shared_state().lock_unpoisoned();
    guard.state.as_ref().and_then(|s| {
        s.destinations
            .iter()
            .find(|d| d.destination.id.as_str() == dest_id.trim())
            .map(|d| d.destination.kind.clone())
    })
}

pub fn lookup_backup_summary<'a>(
    state: &'a DaemonStateDto,
    backup_id: &str,
) -> Option<&'a BackupSummaryDto> {
    BackupId::parse(backup_id)
        .ok()
        .and_then(|id| state.backups.iter().find(|b| b.backup.id == id))
}
