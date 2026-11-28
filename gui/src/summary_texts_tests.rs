// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `summary_texts` (kept in their own file, NFR-12 file size).

use super::*;
use crate::time_texts::{format_datetime_display, format_day_group, format_time_short};
use time::macros::datetime;

#[test]
fn utc_and_offset_same_instant_format_identically_in_local_time() {
    let ts_utc = datetime!(2026-11-16 23:30:00 UTC);
    let ts_plus1 = datetime!(2026-11-17 00:30:00 +01:00);
    let ts_minus5 = datetime!(2026-11-16 18:30:00 -05:00);
    assert_eq!(ts_utc.unix_timestamp(), ts_plus1.unix_timestamp());
    assert_eq!(ts_utc.unix_timestamp(), ts_minus5.unix_timestamp());

    let disp_utc = format_datetime_display(ts_utc);
    let disp_plus1 = format_datetime_display(ts_plus1);
    let disp_minus5 = format_datetime_display(ts_minus5);
    assert_eq!(disp_utc, disp_plus1);
    assert_eq!(disp_utc, disp_minus5);
    assert_eq!(format_time_short(ts_utc), &disp_plus1[11..16]);
    assert_eq!(format_time_short(ts_plus1), &disp_plus1[11..16]);
    assert_eq!(format_time_short(ts_minus5), &disp_plus1[11..16]);
    assert_eq!(
        format_day_group(ts_utc, datetime!(2026-11-17 12:00:00 UTC)),
        format_day_group(ts_plus1, datetime!(2026-11-17 12:00:00 UTC))
    );
}

/// Count texts use the singular for one: (a) one excluded folder, (b) one expert rule, and (c)
/// two of each stay plural.
#[test]
fn finding2_exclude_summary_uses_singular_for_one() {
    let rules = |folders: &[&str], lines: &[&str]| ExcludeRules {
        folders: folders.iter().map(ToString::to_string).collect(),
        lines: lines.iter().map(ToString::to_string).collect(),
        ..ExcludeRules::default()
    };
    assert_eq!(
        format_exclude_summary(&rules(&["$HOME/a"], &[])),
        "1 folder",
        "(a)"
    );
    assert_eq!(
        format_exclude_summary(&rules(&[], &["**/*.tmp"])),
        "1 expert rule",
        "(b)"
    );
    assert_eq!(
        format_exclude_summary(&rules(&["$HOME/a", "$HOME/b"], &["**/*.tmp", "**/*.bak"])),
        "2 folders · 2 expert rules",
        "(c)"
    );
}

/// Relative times use the singular for one: (a) minute, (b) hour, (c) day; (d) more
/// than one stays plural.
#[test]
fn finding2_relative_time_uses_singular_for_one() {
    let now = datetime!(2026-11-21 16:00:00 UTC);
    let ago = |secs: i64| format_relative_time(now - time::Duration::seconds(secs), now);
    assert_eq!(ago(60), "1 minute ago", "(a)");
    assert_eq!(ago(3600), "1 hour ago", "(b)");
    assert_eq!(ago(86_400), "1 day ago", "(c)");
    assert_eq!(ago(2 * 86_400), "2 days ago", "(d)");
}

/// «Every n days» uses the singular for an interval of one day.
#[test]
fn finding2_every_n_days_uses_singular_for_one() {
    let schedule = |interval_days| Schedule::EveryNDays {
        interval_days,
        start_date: "2026-11-21".to_string(),
        hour: 12,
        minute: 0,
    };
    assert_eq!(
        format_schedule_summary(&schedule(1)),
        "Every 1 day from 2026-11-21 at 12:00"
    );
    assert_eq!(
        format_schedule_summary(&schedule(3)),
        "Every 3 days from 2026-11-21 at 12:00"
    );
}
