// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! The speed of a transfer from the bytes done at two moments. restic's restore `status` lines
//! name no speed, so it follows from how `bytes_restored` grows from one line to the next.

use time::OffsetDateTime;

/// Time constant of the smoothing in nanoseconds (2 s): a lasting change of speed shows within
/// a few seconds, and one status line that comes early or late does not make the value jump.
const SMOOTHING_NANOS: i128 = 2_000_000_000;
const NANOS_PER_SEC: i128 = 1_000_000_000;

/// The smoothed speed of a transfer, from samples of the bytes done so far.
#[derive(Debug, Clone, Default)]
pub struct TransferSpeed {
    last: Option<(OffsetDateTime, u64)>,
    bytes_per_sec: Option<u64>,
}

impl TransferSpeed {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `bytes` done at `now` and returns the smoothed speed in bytes per second;
    /// `None` after the first sample, which has nothing to compare with. A sample at the same
    /// moment as the one before, or before it, changes nothing.
    pub fn update(&mut self, now: OffsetDateTime, bytes: u64) -> Option<u64> {
        if let Some((then, before)) = self.last {
            let nanos = (now - then).whole_nanoseconds();
            if nanos <= 0 {
                return self.bytes_per_sec;
            }
            let sample = i128::from(bytes.saturating_sub(before)) * NANOS_PER_SEC / nanos;
            // Moves the speed towards the sample by the share nanos / (nanos + time constant).
            let speed = self.bytes_per_sec.map_or(sample, |old| {
                let old = i128::from(old);
                old + (sample - old) * nanos / (nanos + SMOOTHING_NANOS)
            });
            self.bytes_per_sec = Some(u64::try_from(speed.max(0)).unwrap_or(u64::MAX));
        }
        self.last = Some((now, bytes));
        self.bytes_per_sec
    }
}
