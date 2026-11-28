// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Dates and times as the GUI shows them: in the local time zone, relative to the daemon's
//! clock where one is known.

use crate::i18n::i18n;
use crate::state_cache::shared_state;
use kestic_core::clock::{Clock, SystemClock};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::DaemonStateDto;
use time::{Date, OffsetDateTime};

/// `ts` in the local time zone; unchanged when the shifted time is out of range (a time at the
/// very end of year 9999, as a foreign snapshot may carry).
pub(crate) fn to_local(ts: OffsetDateTime) -> OffsetDateTime {
    let offset_secs = crate::qt_bridge::ffi::local_offset_seconds(ts.unix_timestamp());
    let offset = time::UtcOffset::from_whole_seconds(offset_secs).unwrap_or(ts.offset());
    to_offset_or_keep(ts, offset)
}

/// `ts` at `offset`, or `ts` unchanged when that leaves the representable range.
pub(crate) fn to_offset_or_keep(ts: OffsetDateTime, offset: time::UtcOffset) -> OffsetDateTime {
    ts.checked_to_offset(offset).unwrap_or(ts)
}

/// `date` as `YYYY-MM-DD`.
pub(crate) fn iso_date(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month() as u8,
        date.day()
    )
}

pub fn format_day_group(ts: OffsetDateTime, now: OffsetDateTime) -> String {
    let date = to_local(ts).date();
    let today = to_local(now).date();
    if date == today {
        return i18n("Today").to_string();
    }
    if let Some(yesterday) = today.previous_day()
        && date == yesterday
    {
        return i18n("Yesterday").to_string();
    }
    iso_date(date)
}

pub fn format_time_short(ts: OffsetDateTime) -> String {
    let local = to_local(ts);
    format!("{:02}:{:02}", local.hour(), local.minute())
}

pub fn format_datetime_display(ts: OffsetDateTime) -> String {
    let local = to_local(ts);
    format!(
        "{} {:02}:{:02}",
        iso_date(local.date()),
        local.hour(),
        local.minute()
    )
}

pub fn current_now() -> OffsetDateTime {
    SystemClock.now_utc()
}

pub fn daemon_now(state: &DaemonStateDto) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(state.overall.now_unix).unwrap_or_else(|_| current_now())
}

pub fn cached_daemon_now() -> OffsetDateTime {
    let guard = shared_state().lock_unpoisoned();
    guard.state.as_ref().map_or_else(current_now, daemon_now)
}

pub fn cached_today_iso_date() -> String {
    iso_date(to_local(cached_daemon_now()).date())
}
