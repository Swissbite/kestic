// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Daemon calls the window makes on behalf of the user, off the Qt thread. Each call runs on a
//! lane: calls about the same thing run in the order the user made them, calls about different
//! things do not wait for each other. Each result goes back to the Qt thread.

use super::{Finish, qobject};
use crate::daemon_events::request_refresh;
use crate::state_cache::{
    daemon_unavailable_text, dbus_error_text, make_client, refresh_state_now,
};
use cxx_qt::Threading;
use cxx_qt_lib::QString;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{KesticClient, KesticDbusError};
use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send>;

/// One worker thread per lane key, each started with the first call on its lane.
static WORKERS: Mutex<BTreeMap<String, Sender<Job>>> = Mutex::new(BTreeMap::new());
/// Calls asked for and not made yet.
static PENDING: AtomicUsize = AtomicUsize::new(0);

/// Holds one pending call; dropping it (the call ran, panicked or was never sent) releases it.
struct Pending;

impl Drop for Pending {
    fn drop(&mut self) {
        PENDING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Counts `job` as pending until it has run.
fn counted(job: Job) -> Job {
    PENDING.fetch_add(1, Ordering::SeqCst);
    let pending = Pending;
    Box::new(move || {
        let _pending = pending;
        job();
    })
}

/// Queues `job` on the worker of `key`; the job comes back if that worker has ended.
fn send_in_order(key: &str, job: Job) -> Result<(), Job> {
    let mut workers = WORKERS.lock_unpoisoned();
    let worker = workers.entry(key.to_string()).or_insert_with(|| {
        let (jobs, queue) = mpsc::channel::<Job>();
        thread::spawn(move || {
            for job in queue {
                job();
            }
        });
        jobs
    });
    worker.send(job).map_err(|returned| returned.0)
}

/// Waits until the calls asked for are made, at most `limit`: a window that closes still tells
/// the daemon so (`WindowActive(false)`) before the process ends.
pub fn finish_pending_calls(limit: Duration) {
    let deadline = Instant::now() + limit;
    while PENDING.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
}

/// Where a call runs. Calls on the same lane run one after the other, in the order the user made
/// them; calls on different lanes do not wait for each other, so a slow call (a backup whose
/// destination answers late, a password prompt) holds back only the calls about the same thing.
pub(super) enum Lane {
    /// After the earlier calls with the same key.
    After(String),
    /// On a thread of its own, after nothing: cancelling must never wait.
    Alone,
}

impl Lane {
    /// The window's own state (`WindowActive`).
    pub(super) fn window() -> Self {
        Self::After("window".into())
    }

    /// Everything that concerns all backups (pause, resume, start all).
    pub(super) fn all_backups() -> Self {
        Self::After("all-backups".into())
    }

    /// Everything that concerns the backup `id`.
    pub(super) fn backup(id: &str) -> Self {
        Self::After(format!("backup:{id}"))
    }

    /// Everything that concerns the destination `id`.
    pub(super) fn destination(id: &str) -> Self {
        Self::After(format!("destination:{id}"))
    }

    /// Settings and the password store.
    pub(super) fn settings() -> Self {
        Self::After("settings".into())
    }

    /// The availability checks.
    pub(super) fn availability() -> Self {
        Self::After("availability".into())
    }
}

impl qobject::KesticApp {
    /// Runs `call` with a client of the daemon on `lane`, off the Qt thread, then `then` with its
    /// result on the Qt thread; `Err` means there is no connection to the bus.
    pub(super) fn call_daemon<T: Send + 'static>(
        self: Pin<&mut Self>,
        lane: Lane,
        call: impl FnOnce(&KesticClient) -> T + Send + 'static,
        then: impl FnOnce(Pin<&mut Self>, Result<T, String>) + Send + 'static,
    ) {
        self.call_daemon_with(lane, call, |_| false, then);
    }

    /// Like [`Self::call_daemon`]; when `fetch_state(&result)` holds, the state is fetched anew
    /// before `then` runs, so `then` reads the state as it is after the call.
    pub(super) fn call_daemon_with<T: Send + 'static>(
        self: Pin<&mut Self>,
        lane: Lane,
        call: impl FnOnce(&KesticClient) -> T + Send + 'static,
        fetch_state: impl FnOnce(&T) -> bool + Send + 'static,
        then: impl FnOnce(Pin<&mut Self>, Result<T, String>) + Send + 'static,
    ) {
        let qt = self.qt_thread();
        let job = counted(Box::new(move || {
            let result = make_client().map(|client| call(&client));
            if result.as_ref().is_ok_and(fetch_state) {
                refresh_state_now();
            }
            // Fails only when the window is gone; then nobody waits for the result.
            let _ = qt.queue(move |app| then(app, result));
        }));
        match lane {
            Lane::Alone => {
                thread::spawn(job);
            }
            Lane::After(key) => {
                if send_in_order(&key, job).is_err() {
                    eprintln!("kestic: the thread for daemon calls on lane {key} has ended");
                }
            }
        }
    }

    /// Ends a daemon call that `call_daemon` made for the user: on success clears the error line
    /// and has the state fetched in the background, on failure shows the error text.
    pub(super) fn finish_daemon_action<T, E>(
        mut self: Pin<&mut Self>,
        result: Result<Result<T, E>, String>,
        how: Finish,
    ) -> Option<T>
    where
        E: Clone + Into<KesticDbusError>,
    {
        let failure = match result {
            Ok(Ok(value)) => {
                if how == Finish::Saved {
                    self.as_mut().set_has_unsaved_changes(false);
                }
                self.as_mut().show_action_error("");
                request_refresh();
                return Some(value);
            }
            Ok(Err(err)) => dbus_error_text(&err),
            Err(_) => daemon_unavailable_text(),
        };
        self.as_mut().show_action_error(&failure);
        if how == Finish::NotifyOnError {
            self.as_mut().passive_notification(QString::from(&failure));
        }
        None
    }
}
