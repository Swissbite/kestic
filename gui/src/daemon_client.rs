// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The GUI's connection to the daemon: the session-bus connection, the client, starting the
//! daemon when it does not run, and fetching its state into the cache.

use crate::daemon_events;
use crate::i18n::i18n;
use crate::state_cache::shared_state;
use kestic_core::model::{Run, RunStatus};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{
    ActiveRunProgressDto, BackupSummaryDto, KESTIC_BUS_NAME, KesticClient,
};
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

/// The GUI's one session-bus connection: every daemon call and the signal listener use it.
static GUI_CONNECTION: Mutex<Option<zbus::Connection>> = Mutex::new(None);

/// The GUI's session-bus connection; opened on first use and kept for the whole session. A
/// failed attempt is not cached, so the next call tries again.
pub fn gui_connection() -> zbus::Result<zbus::Connection> {
    let mut slot = GUI_CONNECTION.lock_unpoisoned();
    if let Some(conn) = slot.as_ref() {
        return Ok(conn.clone());
    }
    let conn = zbus::block_on(zbus::Connection::session())?;
    *slot = Some(conn.clone());
    Ok(conn)
}

/// Forgets the GUI's connection after it was lost; the next call opens a new one.
pub fn drop_gui_connection() {
    *GUI_CONNECTION.lock_unpoisoned() = None;
}

/// A daemon client on the GUI's connection.
pub fn make_client() -> Result<KesticClient, String> {
    let conn = gui_connection().map_err(|e| e.to_string())?;
    let guard = shared_state().lock_unpoisoned();
    Ok(KesticClient::from_connection(
        conn,
        &guard.launch.bus_name,
        &guard.launch.object_path,
    ))
}

/// Starts the background threads that follow the daemon and has its state loaded in the
/// background. A daemon that is not running yet is started in the background too, so the window
/// shows at once.
pub fn initialize_and_sync() {
    daemon_events::start();
    let no_autostart = shared_state().lock_unpoisoned().launch.no_autostart_daemon;
    let owned = make_client().is_ok_and(|c| c.is_bus_owned().unwrap_or(false));
    if owned {
        // The refresher fetches the first state; meanwhile the window shows that it loads.
        daemon_events::request_refresh();
    } else if no_autostart {
        // Nothing to start: this only reports that the daemon is missing.
        ensure_daemon_and_refresh();
    } else {
        daemon_events::start_daemon_in_background();
    }
}

/// Ensures the daemon is running (spawning `kestic daemon` without `--autostart` when needed,
/// and waiting for it) and loads its state. It waits, so the Qt thread calls it only when the
/// daemon already runs.
pub fn ensure_daemon_and_refresh() {
    let launch = {
        let guard = shared_state().lock_unpoisoned();
        guard.launch.clone()
    };

    let client = match make_client() {
        Ok(c) => c,
        Err(err) => {
            let mut guard = shared_state().lock_unpoisoned();
            guard.daemon_reachable = false;
            guard.daemon_error = Some(err);
            guard.first_fetch_pending = false;
            guard.state_fetched = true;
            return;
        }
    };

    let mut owned = client.is_bus_owned().unwrap_or(false);
    if !owned && !launch.no_autostart_daemon {
        let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("kestic"));
        let mut cmd = Command::new(&exe);
        cmd.arg("daemon");
        if launch.bus_name != KESTIC_BUS_NAME {
            cmd.arg("--bus-name").arg(&launch.bus_name);
        }
        // stderr is inherited so the daemon's lifecycle messages reach the session journal.
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());

        match cmd.spawn() {
            Ok(mut child) => owned = wait_for_daemon(&client, &mut child),
            Err(err) => {
                let mut guard = shared_state().lock_unpoisoned();
                guard.daemon_reachable = false;
                guard.daemon_error = Some(err.to_string());
                guard.first_fetch_pending = false;
                guard.state_fetched = true;
                return;
            }
        }
    }

    if !owned {
        let mut guard = shared_state().lock_unpoisoned();
        guard.daemon_reachable = false;
        guard.daemon_error = Some(i18n("The background service did not start.").to_string());
        guard.first_fetch_pending = false;
        guard.state_fetched = true;
        return;
    }

    refresh_state_now();
}

/// Longest wait for a started daemon to take its bus name; a slow disk or a busy machine may
/// need seconds.
const DAEMON_START_LIMIT: Duration = Duration::from_secs(60);

/// Waits until the started daemon owns its bus name (`true`), or until it exited or the limit
/// passed (`false`). A daemon that ended because another one took the name in the meantime
/// counts as started.
fn wait_for_daemon(client: &KesticClient, child: &mut std::process::Child) -> bool {
    let deadline = Instant::now() + DAEMON_START_LIMIT;
    loop {
        if client.is_bus_owned().unwrap_or(false) {
            return true;
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            return client.is_bus_owned().unwrap_or(false);
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Drops the progress of runs that ended. Still active are the run of each backup and every run
/// the state lists as running: a restore or a maintenance also runs beside a backup, and only
/// one run is the overall progress. That one is added when its progress has not arrived yet.
fn retain_active_progress(
    progress: &mut HashMap<String, ActiveRunProgressDto>,
    backups: &[BackupSummaryDto],
    recent_runs: &[Run],
    overall: Option<&ActiveRunProgressDto>,
) {
    let active_ids: HashSet<&str> = backups
        .iter()
        .filter_map(|b| b.active_run.as_ref())
        .chain(
            recent_runs
                .iter()
                .filter(|r| r.status == RunStatus::Running),
        )
        .map(|r| r.id.as_str())
        .collect();
    progress.retain(|id, _| active_ids.contains(id.as_str()));
    if let Some(p) = overall {
        progress
            .entry(p.run_id.clone())
            .or_insert_with(|| p.clone());
    }
}

pub fn refresh_state_now() {
    daemon_events::start();
    let client = match make_client() {
        Ok(c) => c,
        Err(err) => {
            let mut guard = shared_state().lock_unpoisoned();
            guard.daemon_reachable = false;
            guard.daemon_error = Some(err);
            guard.first_fetch_pending = false;
            guard.state_fetched = true;
            return;
        }
    };

    match client.get_state() {
        Ok(state) => {
            let (became_reachable, listening) = {
                let mut guard = shared_state().lock_unpoisoned();
                let was_reachable = guard.daemon_reachable;
                guard.daemon_reachable = true;
                guard.daemon_error = None;
                retain_active_progress(
                    &mut guard.active_progress,
                    &state.backups,
                    &state.recent_runs,
                    state.overall.active_run_progress.as_ref(),
                );
                guard.state = Some(state);
                guard.first_fetch_pending = false;
                guard.state_fetched = true;
                (!was_reachable, guard.listening)
            };
            // A daemon that (re)appeared learns that this GUI is open; before the listener
            // receives signals, the listener registers the GUI itself.
            if became_reachable && listening {
                let _ = client.register_gui();
            }
        }
        Err(err) => {
            {
                let mut guard = shared_state().lock_unpoisoned();
                guard.daemon_reachable = false;
                guard.daemon_error = Some(err.to_string());
                guard.first_fetch_pending = false;
                guard.state_fetched = true;
            }
            daemon_events::retry_while_unreachable();
        }
    }
}

#[cfg(test)]
#[path = "daemon_client_tests.rs"]
mod tests;
