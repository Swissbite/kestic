// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Daemon calls the window makes on behalf of the user, off the Qt thread: one worker thread
//! makes them in the order they were asked for, and each result goes back to the Qt thread.
//! Calls that may wait for the user (a password prompt) run on a thread of their own, so they
//! do not hold back the others.

use super::{Finish, qobject};
use crate::daemon_events::request_refresh;
use crate::state_cache::{
    daemon_unavailable_text, dbus_error_text, make_client, refresh_state_now,
};
use cxx_qt::Threading;
use cxx_qt_lib::QString;
use kestic_daemon::dbus_api::{KesticClient, KesticDbusError};
use std::pin::Pin;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send>;

/// The worker thread's queue; the thread starts with the first call.
static WORKER: OnceLock<Sender<Job>> = OnceLock::new();
/// Calls asked for and not made yet, on the worker or on a thread of their own.
static PENDING: AtomicUsize = AtomicUsize::new(0);

fn worker() -> &'static Sender<Job> {
    WORKER.get_or_init(|| {
        let (jobs, queue) = mpsc::channel::<Job>();
        thread::spawn(move || {
            for job in queue {
                job();
            }
        });
        jobs
    })
}

/// Counts `job` as pending until it has run.
fn counted(job: Job) -> Job {
    PENDING.fetch_add(1, Ordering::SeqCst);
    Box::new(move || {
        job();
        PENDING.fetch_sub(1, Ordering::SeqCst);
    })
}

/// Waits until the calls asked for are made, at most `limit`: a window that closes still tells
/// the daemon so (`WindowActive(false)`) before the process ends.
pub fn finish_pending_calls(limit: Duration) {
    let deadline = Instant::now() + limit;
    while PENDING.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
}

/// How a call is made.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Lane {
    /// On the worker, after the calls asked for before it.
    InOrder,
    /// On a thread of its own: the call may wait for the user.
    Alone,
}

impl qobject::KesticApp {
    /// Runs `call` with a client of the daemon on the worker thread, then `then` with its result
    /// on the Qt thread; `Err` means there is no connection to the bus. Calls run one
    /// after the other, in the order the user made them.
    pub(super) fn call_daemon<T: Send + 'static>(
        self: Pin<&mut Self>,
        call: impl FnOnce(&KesticClient) -> T + Send + 'static,
        then: impl FnOnce(Pin<&mut Self>, Result<T, String>) + Send + 'static,
    ) {
        self.call_daemon_with(Lane::InOrder, call, |_| false, then);
    }

    /// Like [`Self::call_daemon`], on `lane`; when `fetch_state(&result)` holds, the state is
    /// fetched anew before `then` runs, so `then` reads the state as it is after the call.
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
            Lane::InOrder => {
                if worker().send(job).is_err() {
                    PENDING.fetch_sub(1, Ordering::SeqCst);
                    eprintln!("kestic: the thread for daemon calls has ended");
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
