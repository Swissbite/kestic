// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Local times in Europe/Zurich, the time zone of all scheduler tests.

use kestic_core::scheduler::{TimezoneSpec, resolve_local_slot};
use time::OffsetDateTime;

pub fn zurich() -> TimezoneSpec {
    TimezoneSpec::Named("Europe/Zurich".to_string())
}

/// The instant of a local date and time in Europe/Zurich.
pub fn zdt(year: i16, month: i8, day: i8, hour: u8, minute: u8) -> OffsetDateTime {
    let date = jiff::civil::Date::new(year, month, day).expect("valid date");
    resolve_local_slot(date, hour, minute, &zurich().resolve()).expect("valid local time")
}
