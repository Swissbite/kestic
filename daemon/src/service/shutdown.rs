// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Ending the daemon on `SIGTERM` (systemd stops the autostart unit at logout): nothing new
//! starts, every restic gets `SIGTERM` once and may release its lock, and what is still running
//! after one shared deadline is killed with every process it started. Each run ends as
//! interrupted in the history before the process exits.

use super::state::{DaemonCore, Interruption};
use kestic_core::child_registry::ChildRegistry;
use kestic_core::sync::LockUnpoisoned;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long restic may take after `SIGTERM` to clean up, for all processes together.
pub(super) const STOP_DEADLINE: Duration = Duration::from_secs(10);
/// How long the workers may take after `SIGKILL` to record their runs.
const REAP_GRACE: Duration = Duration::from_millis(1500);
const POLL: Duration = Duration::from_millis(25);

/// Sets the returned flag on `SIGTERM` instead of ending the process, so the daemon can stop its
/// runs first. `SIGINT` and `SIGHUP` keep their default.
pub fn install_stop_signal() -> io::Result<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&flag))?;
    ChildRegistry::global().stop_on(Arc::clone(&flag));
    Ok(flag)
}

/// Whether `SIGTERM` has arrived.
pub(super) fn stop_requested(signal: Option<&Arc<AtomicBool>>) -> bool {
    signal.is_some_and(|flag| flag.load(Ordering::SeqCst))
}

/// Stops every run and child process after `SIGTERM`, which arrived at `signal_at`, and records
/// the runs that have not ended as interrupted.
pub(super) fn stop_after_signal(core: &Arc<Mutex<DaemonCore>>, signal_at: Instant) {
    let registry = ChildRegistry::global();
    begin_stop(core, registry);
    let deadline = signal_at + STOP_DEADLINE;
    if !wait_until_idle(core, registry, deadline) {
        let killed = registry.kill_remaining();
        eprintln!(
            "kestic-daemon: restic did not end within {} s after SIGTERM; killed {killed} process(es)",
            STOP_DEADLINE.as_secs()
        );
        wait_until_idle(core, registry, Instant::now() + REAP_GRACE);
    }
    let guard = core.lock_unpoisoned();
    match guard
        .history
        .mark_interrupted_on_startup(guard.clock.as_ref())
    {
        Ok(0) => {}
        Ok(n) => eprintln!("kestic-daemon: marked {n} unfinished run(s) as interrupted"),
        Err(err) => {
            eprintln!("kestic-daemon: failed to mark unfinished runs as interrupted: {err}")
        }
    }
}

/// Refuses new runs and child processes and sends `SIGTERM` to every running one. The queue
/// ends with the daemon: its backups lose their waiting entry and stay due.
fn begin_stop(core: &Arc<Mutex<DaemonCore>>, registry: &ChildRegistry) {
    {
        let mut guard = core.lock_unpoisoned();
        guard.shutdown_flag.store(true, Ordering::SeqCst);
        for entry in guard.active_runs.values_mut() {
            entry.interruption = Some(Interruption::Stop);
        }
        guard.cancel_any_active_run();
        guard.cancel_running_restic_installs();
        let queued: Vec<_> = guard.queued_backups.drain(..).collect();
        for backup_id in &queued {
            if let Err(err) = guard.history.delete_pending_runs_for_backup(backup_id) {
                eprintln!("kestic-daemon: failed to drop the queue entry of {backup_id}: {err}");
            }
        }
    }
    registry.close();
}

/// Waits until no run and no child process is left, at most until `deadline`.
fn wait_until_idle(
    core: &Arc<Mutex<DaemonCore>>,
    registry: &ChildRegistry,
    deadline: Instant,
) -> bool {
    loop {
        let idle = registry.live_count() == 0 && core.lock_unpoisoned().active_runs.is_empty();
        if idle {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        thread::sleep(POLL.min(deadline - now));
    }
}
