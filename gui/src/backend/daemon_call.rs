// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Daemon calls the window makes on behalf of the user, off the Qt thread: one worker thread
//! makes them in the order they were asked for, and each result goes back to the Qt thread.

use super::{Finish, qobject};
use crate::daemon_events::request_refresh;
use crate::state_cache::{dbus_error_text, make_client};
use cxx_qt::Threading;
use cxx_qt_lib::QString;
use kestic_daemon::dbus_api::{KesticClient, KesticDbusError};
use std::pin::Pin;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Sender};
use std::thread;

type Job = Box<dyn FnOnce() + Send>;

/// The worker thread's queue; the thread starts with the first call.
static WORKER: OnceLock<Sender<Job>> = OnceLock::new();

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

impl qobject::KesticApp {
    /// Runs `call` with a client of the daemon on the worker thread, then `then` with its result
    /// on the Qt thread; `Err` is the text of a missing connection to the bus. Calls run one
    /// after the other, in the order the user made them.
    pub(super) fn call_daemon<T: Send + 'static>(
        self: Pin<&mut Self>,
        call: impl FnOnce(&KesticClient) -> T + Send + 'static,
        then: impl FnOnce(Pin<&mut Self>, Result<T, String>) + Send + 'static,
    ) {
        let qt = self.qt_thread();
        let job: Job = Box::new(move || {
            let result = make_client().map(|client| call(&client));
            // Fails only when the window is gone; then nobody waits for the result.
            let _ = qt.queue(move |app| then(app, result));
        });
        if worker().send(job).is_err() {
            eprintln!("kestic: the thread for daemon calls has ended");
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
            Err(no_bus) => no_bus,
        };
        self.as_mut().show_action_error(&failure);
        if how == Finish::NotifyOnError {
            self.as_mut().passive_notification(QString::from(&failure));
        }
        None
    }
}
