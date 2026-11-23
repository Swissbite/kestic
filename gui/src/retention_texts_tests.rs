// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `retention_texts`: reasons, times and summary of the retention preview.

use super::*;
use kestic_core::backup::ForgetPreviewSnapshot;
use time::macros::datetime;

fn snapshot(short_id: &str, time: &str, action: &str, reasons: &[&str]) -> ForgetPreviewSnapshot {
    ForgetPreviewSnapshot {
        snapshot_id: format!("{short_id}00000000"),
        short_id: short_id.to_string(),
        time: time.to_string(),
        hostname: "laptop".to_string(),
        paths: vec!["/home/u/docs".to_string()],
        action: action.to_string(),
        reasons: reasons.iter().map(|r| (*r).to_string()).collect(),
    }
}

/// AC-M2-06-2: the reason of a kept snapshot is named in the GUI language, not as restic's
/// English text: (a) `--keep-last`, (b) a period rule, (c) a period rule with a duration,
/// (d) `--keep-within`, (e) the oldest snapshot of a rule; (f) an unknown reason stays as
/// restic writes it.
#[test]
fn ac_m2_06_2_forget_reasons_are_translated() {
    assert_eq!(
        forget_reason_text("last snapshot"),
        "latest snapshot",
        "(a)"
    );
    assert_eq!(
        forget_reason_text("daily snapshot"),
        "daily snapshot",
        "(b)"
    );
    assert_eq!(
        forget_reason_text("monthly within 1y"),
        "monthly within 1y",
        "(c)"
    );
    assert_eq!(forget_reason_text("within 3d"), "all within 3d", "(d)");
    assert_eq!(
        forget_reason_text("oldest yearly snapshot"),
        "yearly snapshot (oldest)",
        "(e)"
    );
    assert_eq!(forget_reason_text("has tags"), "has tags", "(f)");
}

/// AC-M2-06-2: the preview page gets (a) a summary of kept and removed snapshots, (b) the time
/// of each snapshot as local date and time instead of restic's timestamp and (c) its reasons
/// joined; (d) without removals the summary says that nothing would be removed.
#[test]
fn ac_m2_06_2_retention_preview_view_has_summary_times_and_reasons() {
    let result = ForgetPreviewResult {
        backup_id: "b1".to_string(),
        snapshots: vec![
            snapshot(
                "983dd427",
                "2026-11-23T05:21:25.940293187Z",
                "keep",
                &["last snapshot", "daily snapshot"],
            ),
            snapshot("f00bdf7c", "2026-11-22T05:21:25.54754729Z", "remove", &[]),
            snapshot("605b3cca", "2026-11-21T05:21:25.186644429Z", "remove", &[]),
        ],
        keep_count: 1,
        remove_count: 2,
    };
    let view = retention_preview_view(&result);
    assert_eq!(
        view.summary, "1 snapshot kept · 2 snapshots would be removed",
        "(a)"
    );
    assert_eq!(
        view.snapshots[0].time_text,
        format_datetime_display(datetime!(2026-11-23 05:21:25.940293187 UTC)),
        "(b)"
    );
    assert_eq!(
        view.snapshots[0].reasons_text, "latest snapshot, daily snapshot",
        "(c)"
    );
    assert_eq!(
        view.snapshots[1].reasons_text, "",
        "(c) nothing for a removal"
    );

    let kept_only = ForgetPreviewResult {
        snapshots: vec![result.snapshots[0].clone()],
        keep_count: 1,
        remove_count: 0,
        ..result
    };
    assert_eq!(
        retention_preview_view(&kept_only).summary,
        "No snapshots would be removed (1 kept).",
        "(d)"
    );
}

/// AC-M2-06-2: the time of a snapshot is shown in local time whatever offset restic writes
/// (`TZ` of the backing-up machine); a time that cannot be read is shown as restic wrote it.
#[test]
fn ac_m2_06_2_retention_preview_times_with_offsets_and_unreadable_times() {
    let result = ForgetPreviewResult {
        backup_id: "b1".to_string(),
        snapshots: vec![
            snapshot(
                "a1b2c3d4",
                "2026-11-23T10:51:25.5+05:45",
                "keep",
                &["last snapshot"],
            ),
            snapshot("e5f6a7b8", "2026-11-23T05:06:25.5Z", "remove", &[]),
            snapshot("c9d0e1f2", "garbage", "remove", &[]),
        ],
        keep_count: 1,
        remove_count: 2,
    };
    let view = retention_preview_view(&result);
    assert_eq!(
        view.snapshots[0].time_text, view.snapshots[1].time_text,
        "same moment, other offset"
    );
    assert_eq!(view.snapshots[2].time_text, "garbage", "unreadable time");
}
