// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{PROGRESS_EVERY, PROGRESS_WAKER, ProgressPacer, ProgressWaker, note_signal};
use crate::state_cache::shared_state;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH};
use std::cell::{Cell, RefCell};
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

/// A `RunProgress` signal from the daemon is kept for the window and paced, not woken
/// directly.
#[test]
fn run_progress_signal_goes_through_the_pacer() {
    let progress = r#"{"run_id":"run-paced","backup_name":"Documents","percent_done":25.0,"bytes_done":1,"total_bytes":4,"speed_bps":1,"seconds_remaining":3}"#;
    let msg = zbus::Message::signal(KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1, "RunProgress")
        .build(&(progress,))
        .expect("signal body");
    note_signal(msg);
    assert!(
        shared_state()
            .lock_unpoisoned()
            .active_progress
            .contains_key("run-paced"),
        "the progress is kept for the window"
    );
    assert!(
        PROGRESS_WAKER.pacer.lock_unpoisoned().last.is_some(),
        "the progress went through the pacer"
    );
}
