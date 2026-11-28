// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Polling waits with one deadline rule for every test: poll a condition at a fixed interval
//! until it holds, and fail (or return `None`) once the timeout has passed.

use std::fmt::Debug;
use std::thread;
use std::time::{Duration, Instant};

/// Interval between two polls of the waits in this module.
pub const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Timeout of [`wait_until`].
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Calls `poll` every `interval` until it returns `Some`, and returns that value; `None` once
/// `timeout` has passed. `poll` runs at least once, and once more at the deadline.
pub fn try_poll_every<T>(
    timeout: Duration,
    interval: Duration,
    mut poll: impl FnMut() -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = poll() {
            return Some(value);
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        thread::sleep(interval.min(deadline - now));
    }
}

/// [`try_poll_every`] with [`POLL_INTERVAL`].
pub fn try_poll_within<T>(timeout: Duration, poll: impl FnMut() -> Option<T>) -> Option<T> {
    try_poll_every(timeout, POLL_INTERVAL, poll)
}

/// Calls `poll` until it returns `Some` and returns that value; fails naming `what` after
/// `timeout`.
pub fn poll_within<T>(timeout: Duration, what: &str, poll: impl FnMut() -> Option<T>) -> T {
    try_poll_within(timeout, poll).unwrap_or_else(|| panic!("timed out waiting for: {what}"))
}

/// Polls `pred` until it holds; fails naming `what` after `timeout`.
pub fn wait_until_within(timeout: Duration, what: &str, mut pred: impl FnMut() -> bool) {
    poll_within(timeout, what, || pred().then_some(()));
}

/// [`wait_until_within`] with [`DEFAULT_TIMEOUT`].
pub fn wait_until(what: &str, pred: impl FnMut() -> bool) {
    wait_until_within(DEFAULT_TIMEOUT, what, pred);
}

/// Fetches a value with `fetch` until `done` accepts it, and returns it; fails after `timeout`
/// naming `what` and showing the last value fetched.
pub fn wait_for_value<T: Debug>(
    timeout: Duration,
    what: &str,
    mut fetch: impl FnMut() -> T,
    mut done: impl FnMut(&T) -> bool,
) -> T {
    let mut last = None;
    let found = try_poll_within(timeout, || {
        let value = fetch();
        if done(&value) {
            return Some(value);
        }
        last = Some(value);
        None
    });
    found.unwrap_or_else(|| panic!("timed out waiting for: {what}; last value: {last:?}"))
}
