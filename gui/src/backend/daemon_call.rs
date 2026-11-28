// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Daemon calls the window makes on behalf of the user, off the Qt thread. Each call runs on a
//! lane: calls about the same thing run in the order the user made them, calls about different
//! things do not wait for each other. Each result goes back to the Qt thread.

use super::{Finish, qobject};
use crate::daemon_client::{make_client, refresh_state_now};
use crate::daemon_events::request_refresh;
use crate::i18n::{daemon_unavailable_text, dbus_error_text};
use cxx_qt::{CxxQtThread, Threading};
use cxx_qt_lib::QString;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{KesticClient, KesticDbusError};
use std::collections::{BTreeMap, BTreeSet};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send>;

/// One worker thread per lane key, each started with the first call on its lane.
static WORKERS: Mutex<BTreeMap<String, Sender<Job>>> = Mutex::new(BTreeMap::new());
/// Calls asked for and not made yet.
static PENDING: AtomicUsize = AtomicUsize::new(0);
/// The id of the next call QML waits for; 0 is never used (it means «failed before the call»).
static NEXT_REQUEST: AtomicI32 = AtomicI32::new(1);

/// Requests whose asker has gone (`forgetDaemonCall`): what their answer started is undone.
static FORGOTTEN: Mutex<BTreeSet<i32>> = Mutex::new(BTreeSet::new());

/// Notes that nobody waits for `request` any more.
pub(super) fn forget_request(request: i32) {
    FORGOTTEN.lock_unpoisoned().insert(request);
}

/// Whether nobody waits for `request` any more; forgets the note.
pub(super) fn take_forgotten(request: i32) -> bool {
    FORGOTTEN.lock_unpoisoned().remove(&request)
}

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

/// A new worker thread for one lane.
fn start_worker() -> Sender<Job> {
    let (jobs, queue) = mpsc::channel::<Job>();
    thread::spawn(move || {
        for job in queue {
            job();
        }
    });
    jobs
}

/// Queues `job` on the worker of `key`. A worker that has ended (a call on it panicked) is
/// replaced, so the lane keeps working; the job comes back only if the new worker fails too.
fn send_in_order(key: &str, job: Job) -> Result<(), Job> {
    let mut workers = WORKERS.lock_unpoisoned();
    let worker = workers.entry(key.to_string()).or_insert_with(start_worker);
    match worker.send(job) {
        Ok(()) => Ok(()),
        Err(returned) => {
            let fresh = start_worker();
            let sent = fresh.send(returned.0).map_err(|again| again.0);
            workers.insert(key.to_string(), fresh);
            sent
        }
    }
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
/// them; calls on different lanes do not wait for each other. A call the daemon may answer only
/// after a while (it checks destinations first or waits for the user) has a lane of its own, so
/// the user's quick calls (pause, resume, settings, the window) never wait for it.
pub(crate) enum Lane {
    /// After the earlier calls with the same key.
    After(String),
    /// On a thread of its own, after nothing: cancelling must never wait.
    Alone,
}

impl Lane {
    /// The window's own state (`WindowActive`).
    pub(crate) fn window() -> Self {
        Self::After("window".into())
    }

    /// The quick calls about all backups (pause, resume).
    pub(crate) fn all_backups() -> Self {
        Self::After("all-backups".into())
    }

    /// The quick calls about the backup `id` (pause, resume).
    pub(crate) fn backup(id: &str) -> Self {
        Self::After(format!("backup:{id}"))
    }

    /// The settings.
    pub(crate) fn settings() -> Self {
        Self::After("settings".into())
    }

    /// A call the daemon may answer only after a while, `what` naming the kind of call and its
    /// target: only calls of the same kind about the same thing wait for each other.
    pub(crate) fn slow(what: &str) -> Self {
        Self::After(format!("slow:{what}"))
    }
}

/// Runs `call` with a client of the daemon on `lane`, off the Qt thread, then `then` with its
/// result on the Qt thread of the object `qt` belongs to; `Err` means there is no connection to
/// the bus. When `fetch_state(&result)` holds, the state is fetched anew before `then` runs.
pub(crate) fn call_daemon_on<O, T>(
    qt: &CxxQtThread<O>,
    lane: Lane,
    call: impl FnOnce(&KesticClient) -> T + Send + 'static,
    fetch_state: impl FnOnce(&T) -> bool + Send + 'static,
    then: impl FnOnce(Pin<&mut O>, Result<T, String>) + Send + 'static,
) where
    O: Threading + 'static,
    T: Send + 'static,
{
    let qt = qt.clone();
    run_on(
        lane,
        Box::new(move || {
            let result = make_client().map(|client| call(&client));
            if result.as_ref().is_ok_and(fetch_state) {
                refresh_state_now();
            }
            // Fails only when the object is gone; then nobody waits for the result.
            let _ = qt.queue(move |object| then(object, result));
        }),
    );
}

/// Runs `call` with a client of the daemon on `lane`, off the Qt thread; nobody waits for its
/// result (a cancel whose effect the next status shows).
pub(crate) fn send_to_daemon(lane: Lane, call: impl FnOnce(&KesticClient) + Send + 'static) {
    run_on(
        lane,
        Box::new(move || {
            if let Ok(client) = make_client() {
                call(&client);
            }
        }),
    );
}

/// Runs `job` on `lane`, counted as pending until it has run.
fn run_on(lane: Lane, job: Job) {
    let job = counted(job);
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
        call_daemon_on(&self.qt_thread(), lane, call, fetch_state, then);
    }

    /// Like [`Self::call_daemon`] for an edit whose result QML waits for: returns the id of the
    /// request at once; `then` handles the result on the Qt thread and says whether it succeeded
    /// and what to hand to QML, which then gets `daemonCallDone(id, ok, result)`. The state is
    /// fetched anew once the call reached the daemon, so QML reads the edit's effect.
    pub(super) fn request_daemon<T: Send + 'static>(
        self: Pin<&mut Self>,
        lane: Lane,
        call: impl FnOnce(&KesticClient) -> T + Send + 'static,
        then: impl FnOnce(Pin<&mut Self>, Result<T, String>) -> (bool, String) + Send + 'static,
    ) -> i32 {
        self.request_daemon_with(lane, call, |_| true, then)
    }

    /// Like [`Self::request_daemon`], with the state fetched anew before `then` only when
    /// `fetch_state(&result)` holds (see [`Self::call_daemon_with`]); a list that changes nothing
    /// passes `|_| false`.
    pub(super) fn request_daemon_with<T: Send + 'static>(
        self: Pin<&mut Self>,
        lane: Lane,
        call: impl FnOnce(&KesticClient) -> T + Send + 'static,
        fetch_state: impl FnOnce(&T) -> bool + Send + 'static,
        then: impl FnOnce(Pin<&mut Self>, Result<T, String>) -> (bool, String) + Send + 'static,
    ) -> i32 {
        let request = NEXT_REQUEST.fetch_add(1, Ordering::SeqCst);
        let fetched = Arc::new(AtomicBool::new(false));
        let fetched_in_worker = Arc::clone(&fetched);
        let fetch_state = move |result: &T| {
            let fetch = fetch_state(result);
            fetched_in_worker.store(fetch, Ordering::SeqCst);
            fetch
        };
        self.call_daemon_with(lane, call, fetch_state, move |mut app, result| {
            let (ok, answer) = then(app.as_mut(), result);
            // The window shows the state that was fetched with the answer before QML reads it.
            if fetched.load(Ordering::SeqCst) {
                app.as_mut().apply_cached_state();
            }
            take_forgotten(request);
            app.daemon_call_done(request, ok, QString::from(&answer));
        });
        request
    }

    /// Nobody waits for `request` any more (its page closed); a preview its answer started is
    /// cancelled.
    pub fn forget_daemon_call(&self, request: i32) {
        if request > 0 {
            forget_request(request);
        }
    }

    /// Like [`Self::finish_daemon_action`] for a call that first checks the user's input off the
    /// Qt thread: `Ok(Err(text))` is input the call refused before it reached the daemon.
    pub(super) fn finish_checked<T, E>(
        mut self: Pin<&mut Self>,
        result: Result<Result<Result<T, E>, String>, String>,
        how: Finish,
    ) -> Option<T>
    where
        E: Clone + Into<KesticDbusError>,
    {
        match result {
            Ok(Ok(reply)) => self.finish_daemon_action(Ok(reply), how),
            Ok(Err(refused)) => {
                self.as_mut().show_action_error(&refused);
                None
            }
            Err(no_bus) => self.finish_daemon_action(Err::<Result<T, E>, String>(no_bus), how),
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
