// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{PROGRESS_EVERY, ProgressPacer};
use std::time::{Duration, Instant};

/// Progress wakes the window at most every 150 ms; what comes sooner waits for the rest of the
/// interval (`AC-M6-33-1` (b): the window stays usable while a backup reports progress).
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
