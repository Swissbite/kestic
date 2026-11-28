// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The cancel button of the Plasma progress view (`org.kde.JobViewV3.cancelRequested`) is never
//! lost: the watcher listens from the moment it is started until it is stopped.

mod common;

use common::fakes::FakeJobViewHandle;
use common::run_isolated_test;
use kestic_daemon::job_view::{CAPABILITY_KILLABLE, JobViewHandle};
use kestic_test_support::wait::try_poll_every;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

const JOB_VIEW_DEST: &str = "org.kde.JobViewServer.CancelWatchTest";
const TRIALS: usize = 30;

/// Starts a watcher, waits `delay`, presses cancel once and reports whether the callback ran.
fn cancel_arrives(view: &JobViewHandle, fake: &FakeJobViewHandle, delay: Duration) -> bool {
    let calls = Arc::new(AtomicUsize::new(0));
    let guard = {
        let calls = Arc::clone(&calls);
        view.start_cancel_watcher(move || {
            calls.fetch_add(1, Ordering::SeqCst);
        })
    };
    // quiet window: the watcher runs `delay` without a cancel, then gets one at that moment
    thread::sleep(delay);
    fake.emit_cancel_requested();
    // A lost cancel shows as no call: the wait ends without one after 1 s.
    try_poll_every(Duration::from_secs(1), Duration::from_millis(5), || {
        (calls.load(Ordering::SeqCst) > 0).then_some(())
    });
    guard.stop();
    calls.load(Ordering::SeqCst) == 1
}

/// Cancel pressed (a) right after the watcher started and (b) at any moment while it runs
/// always reaches the callback.
#[test]
fn jobview_cancel_is_never_lost() {
    run_isolated_test(
        "jobview_cancel_is_never_lost",
        "KESTIC_JOBVIEW_CANCEL_WATCH_INNER",
        |_root| {
            let fake = FakeJobViewHandle::start(JOB_VIEW_DEST);
            let view = JobViewHandle::request(
                JOB_VIEW_DEST,
                "net.daester.Kestic",
                "Kestic",
                CAPABILITY_KILLABLE,
            )
            .expect("requestView");

            // (a) right after start.
            let lost_at_start = (0..TRIALS)
                .filter(|_| !cancel_arrives(&view, &fake, Duration::ZERO))
                .count();
            assert_eq!(
                lost_at_start, 0,
                "(a) cancel pressed right after the watcher started was lost {lost_at_start} of \
                 {TRIALS} times"
            );

            // (b) at spread-out moments while the watcher runs.
            let lost_while_running = (0..TRIALS)
                .filter(|i| {
                    let delay = Duration::from_millis(150 + (*i as u64 * 7) % 130);
                    !cancel_arrives(&view, &fake, delay)
                })
                .count();
            assert_eq!(
                lost_while_running, 0,
                "(b) cancel pressed while the watcher ran was lost {lost_while_running} of \
                 {TRIALS} times"
            );
            view.terminate(0, "").expect("terminate");
        },
    );
}
