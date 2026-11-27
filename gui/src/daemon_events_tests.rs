// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    PROGRESS_EVERY, PROGRESS_WAKER, ProgressPacer, ProgressWaker, WAKE_REQUESTS, note_signal,
};
use crate::state_cache::shared_state;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH};
use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

/// Progress wakes the window at most every 150 ms; what comes sooner waits for the rest of the
/// interval.
#[test]
fn progress_wakes_the_window_at_most_every_interval() {
    let start = Instant::now();
    let mut pacer = ProgressPacer::default();
    assert_eq!(pacer.admit(start), None, "the first progress wakes at once");
    assert_eq!(
        pacer.admit(start + Duration::from_millis(40)),
        Some(PROGRESS_EVERY - Duration::from_millis(40)),
        "progress 40 ms later waits for the rest of the interval"
    );
    assert_eq!(
        pacer.admit(start + PROGRESS_EVERY),
        None,
        "progress one interval later wakes again"
    );
}

/// A burst of progress (100 signals in 100 ms) wakes the window once at once and once more
/// after the interval, which delivers the last value; later progress wakes it at once again.
#[test]
fn a_burst_of_progress_gives_one_immediate_and_one_delayed_wake_up() {
    let waker = ProgressWaker::new();
    let start = Instant::now();
    let wakes = Cell::new(0);
    let delayed = RefCell::new(Vec::new());
    for ms in 0..100 {
        waker.on_progress(
            start + Duration::from_millis(ms),
            || wakes.set(wakes.get() + 1),
            |wait| delayed.borrow_mut().push(wait),
        );
    }
    assert_eq!(wakes.get(), 1, "one wake-up at once");
    assert_eq!(
        delayed.borrow().len(),
        1,
        "one delayed wake-up for the rest"
    );
    waker.delayed_due(start + PROGRESS_EVERY, || wakes.set(wakes.get() + 1));
    assert_eq!(
        wakes.get(),
        2,
        "the delayed wake-up delivers the last value"
    );
    waker.on_progress(
        start + PROGRESS_EVERY * 3,
        || wakes.set(wakes.get() + 1),
        |wait| delayed.borrow_mut().push(wait),
    );
    assert_eq!(
        wakes.get(),
        3,
        "progress after a quiet interval wakes at once"
    );
    assert_eq!(delayed.borrow().len(), 1, "no further delayed wake-up");
}

/// A `RunProgress` signal of the daemon with `percent_done`.
fn run_progress(percent_done: u32) -> zbus::Message {
    let progress = format!(
        r#"{{"run_id":"run-paced","backup_name":"Documents","percent_done":{percent_done}.0,"bytes_done":1,"total_bytes":4,"speed_bps":1,"seconds_remaining":3}}"#
    );
    zbus::Message::signal(KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1, "RunProgress")
        .build(&(progress.as_str(),))
        .expect("signal body")
}

/// Two `RunProgress` signals of the daemon in a row: both are kept for the window, the first
/// wakes it once, the second only through the delayed wake-up of the pacer, which then
/// delivers the last value.
#[test]
fn run_progress_signals_wake_the_window_through_the_pacer() {
    let wakes_before = WAKE_REQUESTS.load(Ordering::SeqCst);
    note_signal(run_progress(25));
    let first = PROGRESS_WAKER.pacer.lock_unpoisoned().last;
    assert!(first.is_some(), "the first progress went through the pacer");
    assert_eq!(
        WAKE_REQUESTS.load(Ordering::SeqCst) - wakes_before,
        1,
        "the first progress wakes the window once"
    );

    note_signal(run_progress(50));
    assert!(
        PROGRESS_WAKER.scheduled.load(Ordering::SeqCst),
        "the second progress waits for a delayed wake-up"
    );
    assert_eq!(
        WAKE_REQUESTS.load(Ordering::SeqCst) - wakes_before,
        1,
        "the second progress does not wake the window at once"
    );
    let kept = shared_state()
        .lock_unpoisoned()
        .active_progress
        .get("run-paced")
        .map(|p| p.percent_done);
    assert_eq!(kept, Some(50.0), "the last progress is kept for the window");

    let deadline = Instant::now() + Duration::from_secs(2);
    while PROGRESS_WAKER.scheduled.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the delayed wake-up runs");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        PROGRESS_WAKER.pacer.lock_unpoisoned().last > first,
        "the delayed wake-up is noted by the pacer"
    );
    assert_eq!(
        WAKE_REQUESTS.load(Ordering::SeqCst) - wakes_before,
        2,
        "the delayed wake-up wakes the window once more"
    );
}
