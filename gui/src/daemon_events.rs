// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Keeps the window in step with the daemon without a timer on the Qt thread. One thread
//! receives the daemon's signals for the whole session, one fetches the daemon state when it
//! changed, and both wake the Qt thread when the window has something to show.

use crate::backend::qobject::KesticApp;
use crate::state_cache::{
    daemon_unavailable_text, drop_gui_connection, ensure_daemon_and_refresh, gui_connection,
    refresh_state_now, shared_state,
};
use cxx_qt::CxxQtThread;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{ActiveRunProgressDto, KESTIC_INTERFACE_V1, KesticClient};
use kestic_i18n::codes::BlockedReason;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, Once, PoisonError};
use std::thread;
use std::time::{Duration, Instant};
use zbus::export::ordered_stream::{self, OrderedStreamExt};

/// How often an unreachable daemon is asked again for its state.
const UNREACHABLE_RETRY: Duration = Duration::from_secs(2);
/// Pause before a lost session-bus connection is opened again.
const RECONNECT_AFTER: Duration = Duration::from_secs(2);

/// The Qt thread of the window's `KesticApp`, once it exists.
static GUI_THREAD: Mutex<Option<CxxQtThread<KesticApp>>> = Mutex::new(None);
/// A wake-up is queued on the Qt thread and has not run yet.
static WAKE_QUEUED: AtomicBool = AtomicBool::new(false);
/// How often the window was asked to wake up, so tests can count the wake-ups of a signal.
static WAKE_REQUESTS: AtomicUsize = AtomicUsize::new(0);
/// Someone asked for the daemon state since the last fetch.
static REFRESH_WANTED: Mutex<bool> = Mutex::new(false);
static REFRESH_CHANGED: Condvar = Condvar::new();
static STARTED: Once = Once::new();

/// Lets the background threads reach the window's object on the Qt thread.
pub fn attach_gui_thread(thread: CxxQtThread<KesticApp>) {
    *GUI_THREAD.lock_unpoisoned() = Some(thread);
    wake_gui();
}

/// Asks the Qt thread to take over what the background noted (`KesticApp::poll_events`).
/// Requests made before it runs are served by one call.
pub fn wake_gui() {
    WAKE_REQUESTS.fetch_add(1, Ordering::SeqCst);
    if WAKE_QUEUED.swap(true, Ordering::SeqCst) {
        return;
    }
    let queued = GUI_THREAD.lock_unpoisoned().as_ref().is_some_and(|t| {
        t.queue(|app| {
            WAKE_QUEUED.store(false, Ordering::SeqCst);
            app.poll_events();
        })
        .is_ok()
    });
    if !queued {
        WAKE_QUEUED.store(false, Ordering::SeqCst);
    }
}

/// Asks for the daemon state; the refresher fetches it off the Qt thread and wakes the window.
/// Must not be called while the shared GUI state is locked.
pub fn request_refresh() {
    *REFRESH_WANTED.lock_unpoisoned() = true;
    REFRESH_CHANGED.notify_one();
}

/// Starts the signal listener and the refresher (once per process).
pub fn start() {
    STARTED.call_once(|| {
        let (bus_name, object_path) = {
            let guard = shared_state().lock_unpoisoned();
            (
                guard.launch.bus_name.clone(),
                guard.launch.object_path.clone(),
            )
        };
        thread::spawn(move || session_loop(&bus_name, &object_path));
        thread::spawn(refresh_loop);
    });
}

/// Starts the daemon and waits for it in the background; meanwhile the window says that the
/// background service is starting. Does nothing while a start is under way.
pub fn start_daemon_in_background() {
    {
        let mut guard = shared_state().lock_unpoisoned();
        if guard.daemon_starting {
            return;
        }
        guard.daemon_starting = true;
        guard.state_fetched = true;
    }
    wake_gui();
    thread::spawn(|| {
        ensure_daemon_and_refresh();
        {
            let mut guard = shared_state().lock_unpoisoned();
            guard.daemon_starting = false;
            guard.state_fetched = true;
        }
        wake_gui();
    });
}

/// Fetches the daemon state whenever it is asked for, and every two seconds while the daemon is
/// unreachable.
fn refresh_loop() {
    loop {
        {
            let mut wanted = REFRESH_WANTED.lock_unpoisoned();
            while !*wanted {
                if shared_state().lock_unpoisoned().daemon_reachable {
                    wanted = REFRESH_CHANGED
                        .wait(wanted)
                        .unwrap_or_else(PoisonError::into_inner);
                } else {
                    let (guard, waited) = REFRESH_CHANGED
                        .wait_timeout(wanted, UNREACHABLE_RETRY)
                        .unwrap_or_else(PoisonError::into_inner);
                    wanted = guard;
                    if waited.timed_out() {
                        break;
                    }
                }
            }
            *wanted = false;
        }
        refresh_state_now();
        {
            let mut guard = shared_state().lock_unpoisoned();
            if let Some(blocked) = guard.blocked_after_refresh.take() {
                guard.pending_blocked = Some(blocked);
            }
        }
        wake_gui();
    }
}

/// Receives the daemon's signals on the GUI's connection. When the connection ends (the session
/// bus went away) or cannot receive signals, the window shows the daemon as unreachable and a
/// new connection is opened after a pause.
fn session_loop(bus_name: &str, object_path: &str) {
    loop {
        if let Ok(conn) = gui_connection() {
            zbus::block_on(listen(conn, bus_name, object_path));
        }
        drop_gui_connection();
        shared_state().lock_unpoisoned().listening = false;
        mark_unreachable();
        thread::sleep(RECONNECT_AFTER);
    }
}

/// The daemon is gone: the window says so, and the refresher asks again every two seconds.
fn mark_unreachable() {
    {
        let mut guard = shared_state().lock_unpoisoned();
        guard.daemon_reachable = false;
        guard.daemon_error = Some(daemon_unavailable_text());
        guard.state_fetched = true;
    }
    retry_while_unreachable();
    wake_gui();
}

/// Lets the refresher know that the daemon is unreachable, so it asks again every two seconds.
/// Notified under the refresher's lock, so the refresher cannot miss it between its check of
/// the reachability and its wait.
pub fn retry_while_unreachable() {
    let _wanted = REFRESH_WANTED.lock_unpoisoned();
    REFRESH_CHANGED.notify_one();
}

/// Registers the GUI with the daemon and notes the daemon's signals and the changes of its bus
/// name until the stream ends.
async fn listen(conn: zbus::Connection, bus_name: &str, object_path: &str) {
    let Ok(kestic_rule) = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(KESTIC_INTERFACE_V1)
        .path(object_path)
        .build()
    else {
        return;
    };
    let Ok(owner_rule) = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .interface("org.freedesktop.DBus")
        .member("NameOwnerChanged")
        .arg(0, bus_name)
        .build()
    else {
        return;
    };
    let Ok(signals) = zbus::MessageStream::for_match_rule(kestic_rule, &conn, None).await else {
        return;
    };
    let Ok(owners) = zbus::MessageStream::for_match_rule(owner_rule, &conn, None).await else {
        return;
    };
    shared_state().lock_unpoisoned().listening = true;
    let client = KesticClient::from_connection(conn, bus_name, object_path);
    register(&client).await;
    request_refresh();
    let mut messages = ordered_stream::join(signals, owners);
    while let Some(msg_res) = messages.next().await {
        if let Ok(msg) = msg_res
            && !note_owner_change(&client, &msg).await
        {
            note_signal(msg);
        }
    }
}

/// Tells the daemon that this GUI is open.
async fn register(client: &KesticClient) {
    if let Ok(daemon) = client.proxy().await {
        let _ = daemon.register_gui().await;
    }
}

/// Handles a change of the daemon's bus name; `false` for any other message. A new daemon
/// learns that this GUI is open and its state is fetched; a daemon that went away is shown as
/// unreachable at once.
async fn note_owner_change(client: &KesticClient, msg: &zbus::Message) -> bool {
    // Only the bus itself sends this interface: the match rule for it names the bus as sender,
    // and the rule for the daemon's signals admits its own interface only.
    let header = msg.header();
    let from_bus = header
        .interface()
        .is_some_and(|i| i.as_str() == "org.freedesktop.DBus")
        && header
            .member()
            .is_some_and(|m| m.as_str() == "NameOwnerChanged");
    if !from_bus {
        return false;
    }
    if let Ok((_, _, new_owner)) = msg.body().deserialize::<(String, String, String)>() {
        if new_owner.is_empty() {
            mark_unreachable();
        } else {
            register(client).await;
            request_refresh();
        }
    }
    true
}

/// Notes one daemon signal in the shared state and wakes the window.
fn note_signal(msg: zbus::Message) {
    use kestic_daemon::dbus_api::proxy::{
        ManualRunBlocked, QuitConfirmationRequested, RunProgress, RunUpdated, ShowAboutRequested,
        ShowRestoreRequested, ShowWindowRequested, StatusChanged,
    };
    if StatusChanged::from_message(msg.clone()).is_some()
        || RunUpdated::from_message(msg.clone()).is_some()
    {
        request_refresh();
        return;
    }
    if let Some(signal) = RunProgress::from_message(msg.clone()) {
        if let Ok(args) = signal.args()
            && let Ok(progress) = serde_json::from_str::<ActiveRunProgressDto>(args.progress_json)
        {
            note_progress(progress);
            wake_gui_for_progress();
        }
        return;
    } else if let Some(signal) = ManualRunBlocked::from_message(msg.clone()) {
        // A reason code this GUI does not know is ignored, never guessed.
        if let Ok(args) = signal.args()
            && let Some(reason) = BlockedReason::parse(args.reason)
        {
            shared_state().lock_unpoisoned().blocked_after_refresh =
                Some((args.backup_id.to_string(), reason));
            request_refresh();
        }
    } else if ShowWindowRequested::from_message(msg.clone()).is_some() {
        shared_state().lock_unpoisoned().pending_show_window = true;
    } else if ShowAboutRequested::from_message(msg.clone()).is_some() {
        shared_state().lock_unpoisoned().pending_show_about = true;
    } else if ShowRestoreRequested::from_message(msg.clone()).is_some() {
        shared_state().lock_unpoisoned().pending_show_restore = true;
    } else if let Some(signal) = QuitConfirmationRequested::from_message(msg)
        && let Ok(args) = signal.args()
    {
        shared_state().lock_unpoisoned().pending_quit =
            Some((args.has_running_run, args.running_backup_name.to_string()));
    }
    wake_gui();
}

/// How often progress alone may wake the window: restic reports progress many times a second,
/// and each wake-up updates every model on the Qt thread.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);

/// Spaces the wake-ups for progress at least [`PROGRESS_EVERY`] apart.
#[derive(Default)]
struct ProgressPacer {
    last: Option<Instant>,
}

impl ProgressPacer {
    /// `None` when the window may be woken at `now` (noted as the last wake-up), otherwise how
    /// long until it may.
    fn admit(&mut self, now: Instant) -> Option<Duration> {
        let since = self.last.map(|last| now.saturating_duration_since(last));
        match since {
            Some(since) if since < PROGRESS_EVERY => Some(PROGRESS_EVERY - since),
            _ => {
                self.last = Some(now);
                None
            }
        }
    }
}

/// Turns progress into wake-ups of the window, at most one every [`PROGRESS_EVERY`].
struct ProgressWaker {
    pacer: Mutex<ProgressPacer>,
    /// A delayed wake-up for progress is on its way.
    scheduled: AtomicBool,
}

impl ProgressWaker {
    const fn new() -> Self {
        Self {
            pacer: Mutex::new(ProgressPacer { last: None }),
            scheduled: AtomicBool::new(false),
        }
    }

    /// Wakes the window at once, or, when the last wake-up is too recent, has `spawn_after`
    /// deliver one delayed wake-up; a delayed one already on its way covers this progress too,
    /// so the last value always reaches the window.
    fn on_progress(&self, now: Instant, wake: impl Fn(), spawn_after: impl FnOnce(Duration)) {
        let wait = self.pacer.lock_unpoisoned().admit(now);
        match wait {
            None => wake(),
            Some(wait) => {
                if !self.scheduled.swap(true, Ordering::SeqCst) {
                    spawn_after(wait);
                }
            }
        }
    }

    /// The delayed wake-up is due.
    fn delayed_due(&self, now: Instant, wake: impl Fn()) {
        self.scheduled.store(false, Ordering::SeqCst);
        self.pacer.lock_unpoisoned().last = Some(now);
        wake();
    }
}

static PROGRESS_WAKER: ProgressWaker = ProgressWaker::new();

/// Wakes the window for new progress, at most every [`PROGRESS_EVERY`].
fn wake_gui_for_progress() {
    PROGRESS_WAKER.on_progress(Instant::now(), wake_gui, |wait| {
        thread::spawn(move || {
            thread::sleep(wait);
            PROGRESS_WAKER.delayed_due(Instant::now(), wake_gui);
        });
    });
}

/// Keeps the progress of a running backup for the window.
fn note_progress(progress: ActiveRunProgressDto) {
    let mut guard = shared_state().lock_unpoisoned();
    if let Some(active) = guard
        .state
        .as_mut()
        .and_then(|s| s.overall.active_run_progress.as_mut())
        .filter(|a| a.run_id == progress.run_id)
    {
        *active = progress.clone();
    }
    guard
        .active_progress
        .insert(progress.run_id.clone(), progress);
    guard.progress_dirty = true;
}

#[cfg(test)]
#[path = "daemon_events_tests.rs"]
mod tests;
