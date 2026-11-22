// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of the texts of the snapshot browser.

use super::super::snapshot_query::format_count;
use super::*;

fn set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| (*p).to_string()).collect()
}

#[test]
fn selection_text_names_entries_here_and_elsewhere() {
    let here_only = set(&["/home/u/a", "/home/u/b"]);
    assert_eq!(selection_text(&here_only, "/home/u"), "2 items selected");
    let one_other = set(&["/home/u/a", "/home/u/b", "/home/u/Pictures/c"]);
    assert_eq!(
        selection_text(&one_other, "/home/u"),
        "3 items selected (2 here, 1 in ‹Pictures›)"
    );
    let two_others = set(&["/home/u/a", "/x/b", "/y/c"]);
    assert_eq!(
        selection_text(&two_others, "/home/u"),
        "3 items selected (1 here, 2 in other folders)"
    );
}

#[test]
fn count_text_names_total_or_matches() {
    // Counts carry the digit grouping of the test's locale, like KI18n plurals.
    let thousand = format_count(1000);
    assert_eq!(count_text(1000, 1000, false), format!("{thousand} entries"));
    assert_eq!(count_text(1000, 37, true), format!("37 of {thousand}"));
}

#[test]
fn size_column_shows_files_only() {
    let mut e = LsEntryDto {
        name: "a".into(),
        path: "/a".into(),
        entry_type: "file".into(),
        size: Some(48_000),
        mtime: Some("2026-10-12T10:00:00Z".into()),
        uid: None,
        hard_links: None,
    };
    assert_eq!(size_text(&e), "48 KB");
    e.entry_type = "dir".into();
    e.size = None;
    assert_eq!(size_text(&e), "–");
}

#[test]
fn the_last_representable_time_shifted_east_stays_in_utc() {
    let far = OffsetDateTime::parse("9999-12-31T23:59:59Z", &Rfc3339).expect("parse");
    let east = time::UtcOffset::from_hms(1, 0, 0).expect("offset");
    assert_eq!(crate::state_cache::to_offset_or_keep(far, east), far);
}
