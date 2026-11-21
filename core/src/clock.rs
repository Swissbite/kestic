// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Injectable clock abstraction.
//!
//! All time-dependent logic in `core` and `daemon` must obtain wall-clock
//! timestamps via [`Clock`] rather than calling `SystemTime::now()` directly.

use std::sync::RwLock;
use std::time::{Duration, SystemTime};
use time::OffsetDateTime;

/// Wall-clock source for timestamps across `core` and `daemon`.
pub trait Clock: Send + Sync {
    /// Returns the current UTC timestamp.
    fn now_utc(&self) -> OffsetDateTime;

    /// Returns the current UTC timestamp as whole seconds since the Unix epoch.
    fn now_unix_secs(&self) -> i64 {
        self.now_utc().unix_timestamp()
    }
}

/// Whole milliseconds from `start` to `end`: zero if `end` is earlier (clock set back),
/// `u64::MAX` if the span does not fit.
pub fn elapsed_ms(start: OffsetDateTime, end: OffsetDateTime) -> u64 {
    let ms = (end - start).whole_milliseconds().max(0);
    u64::try_from(ms).unwrap_or(u64::MAX)
}

/// Production [`Clock`] backed by [`SystemTime::now`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> OffsetDateTime {
        OffsetDateTime::from(SystemTime::now())
    }
}

/// Deterministic [`Clock`] for unit and integration tests.
#[derive(Debug)]
pub struct FakeClock {
    now: RwLock<OffsetDateTime>,
}

impl FakeClock {
    /// Creates a fake clock initialized to `initial` (in UTC).
    pub fn new(initial: OffsetDateTime) -> Self {
        Self {
            now: RwLock::new(initial.to_offset(time::UtcOffset::UTC)),
        }
    }

    /// Sets the clock to an explicit timestamp.
    pub fn set(&self, timestamp: OffsetDateTime) {
        let mut guard = self
            .now
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = timestamp.to_offset(time::UtcOffset::UTC);
    }

    /// Advances the clock by `delta`.
    pub fn advance(&self, delta: Duration) {
        let signed = time::Duration::try_from(delta)
            .expect("FakeClock::advance duration must fit into time::Duration");
        let mut guard = self
            .now
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard += signed;
    }
}

impl Clock for FakeClock {
    fn now_utc(&self) -> OffsetDateTime {
        *self
            .now
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn fake_clock_advances_and_resets_cleanly() {
        let clock = FakeClock::new(datetime!(2026-12-01 12:00:00 UTC));
        assert_eq!(clock.now_utc(), datetime!(2026-12-01 12:00:00 UTC));

        clock.advance(Duration::from_hours(2));
        assert_eq!(clock.now_utc(), datetime!(2026-12-01 14:00:00 UTC));

        clock.set(datetime!(2026-12-02 09:30:00 UTC));
        assert_eq!(clock.now_utc(), datetime!(2026-12-02 09:30:00 UTC));
    }

    #[test]
    fn elapsed_ms_is_zero_when_the_clock_went_backwards() {
        let start = datetime!(2026-12-01 12:00:00 UTC);
        let end = datetime!(2026-12-01 12:00:01.250 UTC);
        assert_eq!(elapsed_ms(start, end), 1250);
        assert_eq!(elapsed_ms(end, start), 0);
    }

    #[test]
    fn system_clock_returns_plausible_utc_timestamp() {
        let clock = SystemClock;
        assert!(clock.now_utc().year() >= 2026);
    }
}
