// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Progress of a backup does not jump (`docs/spec/restic-integration.md` «Fortschritt beim
//! Sichern»): no percentage while restic still finds files, afterwards a percentage that never
//! falls. Fixtures are recorded sequences of `status` lines, anonymised and shortened.

mod common;

use kestic_core::backup_progress::{BackupProgress, BackupProgressTracker};
use kestic_core::restic::{ResticStatusMessage, ResticStreamEvent, parse_json_line};
use kestic_i18n::codes::RunProgressPhase;
use kestic_test_support::repo_template::copy_template_repo;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn status_lines(ndjson: &str) -> Vec<ResticStatusMessage> {
    ndjson
        .lines()
        .filter_map(|line| match parse_json_line(line) {
            Ok(ResticStreamEvent::Status(st)) => Some(st),
            _ => None,
        })
        .collect()
}

fn fixture(name: &str) -> Vec<ResticStatusMessage> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    status_lines(&fs::read_to_string(&path).expect("read fixture"))
}

fn track(lines: &[ResticStatusMessage]) -> Vec<BackupProgress> {
    let mut tracker = BackupProgressTracker::new();
    lines.iter().map(|st| tracker.update(st, 1)).collect()
}

/// Largest drop of a percentage between two consecutive lines (0 when it never falls).
fn largest_drop(percents: impl Iterator<Item = f64>) -> f64 {
    let mut last = f64::MIN;
    let mut drop = 0.0_f64;
    for p in percents {
        drop = drop.max(last - p);
        last = p;
    }
    drop
}

#[test]
fn progress_stays_in_scan_until_seconds_remaining_then_shows_percent_without_falling() {
    let lines = fixture("backup_progress_search_then_count.ndjson");
    let first_remaining = lines
        .iter()
        .position(|st| st.seconds_remaining.is_some_and(|s| s > 0))
        .expect("fixture has seconds_remaining");
    // The raw restic percentage jumps back in this fixture, otherwise the test proves nothing.
    assert!(
        largest_drop(lines.iter().map(|st| st.percent_done)) > 0.05,
        "fixture must contain a falling percent_done"
    );

    let shown = track(&lines);

    // (a) Until seconds_remaining arrives: scanning, no percentage, no total, no speed.
    for (i, p) in shown[..first_remaining].iter().enumerate() {
        assert_eq!(p.phase, RunProgressPhase::Scanning, "line {i}: phase");
        assert_eq!(p.percent_done, 0.0, "line {i}: percent while scanning");
        assert_eq!(p.total_bytes, 0, "line {i}: total while scanning");
        assert_eq!(p.speed_bps, 0, "line {i}: speed while scanning");
    }
    // (b) The amount looked through is shown while scanning.
    assert_eq!(
        shown[first_remaining - 1].bytes_done,
        lines[first_remaining - 1].bytes_done.expect("bytes_done"),
        "bytes looked through while scanning"
    );
    assert_eq!(
        shown[first_remaining - 1].files_done,
        lines[first_remaining - 1].files_done.expect("files_done"),
        "files looked through while scanning"
    );
    // (c) From then on: scan finished up to the end, also when seconds_remaining is missing
    // again in the last second.
    for (i, p) in shown.iter().enumerate().skip(first_remaining) {
        assert_eq!(p.phase, RunProgressPhase::ScanFinished, "line {i}: phase");
    }
    assert!(
        lines
            .last()
            .is_some_and(|st| st.seconds_remaining.is_none()),
        "fixture ends without seconds_remaining"
    );
    // (d) The shown percentage never falls, also when restic's value falls after the scan.
    assert!(
        largest_drop(lines[first_remaining..].iter().map(|st| st.percent_done)) > 0.0,
        "fixture must contain a falling percent_done after the scan"
    );
    assert_eq!(
        largest_drop(shown.iter().map(|p| p.percent_done)),
        0.0,
        "shown percentage fell"
    );
    // (e) The percentage after the scan follows restic and ends at 100 %.
    assert!(
        shown[first_remaining].percent_done > 80.0,
        "first percentage after the scan: {}",
        shown[first_remaining].percent_done
    );
    assert_eq!(
        shown.last().map(|p| p.percent_done),
        Some(100.0),
        "final percentage"
    );
    // (f) The time left is shown when restic sends it.
    assert_eq!(
        shown[first_remaining].seconds_remaining,
        Some(12),
        "time left after the scan"
    );
}

#[test]
fn fast_run_whose_search_never_ends_before_the_end_shows_no_percentage() {
    let lines = fixture("backup_progress_fast_incremental.ndjson");
    assert!(
        lines.iter().all(|st| st.seconds_remaining.is_none()),
        "fixture never sends seconds_remaining"
    );
    // restic's own percentage jumps back in this fixture, otherwise the test proves nothing.
    assert!(
        largest_drop(lines.iter().map(|st| st.percent_done)) > 0.05,
        "fixture must contain a falling percent_done"
    );

    let shown = track(&lines);

    // (a) The whole run stays in scanning.
    assert!(
        shown.iter().all(|p| p.phase == RunProgressPhase::Scanning),
        "every line is scanning"
    );
    // (b) No percentage at any point, although restic's own value jumps between 33 % and 100 %.
    assert!(
        shown.iter().all(|p| p.percent_done == 0.0),
        "no percentage in a fast run"
    );
    // (c) The amount looked through grows up to the final amount.
    assert_eq!(
        shown.last().map(|p| p.bytes_done),
        lines.last().and_then(|st| st.bytes_done),
        "final amount looked through"
    );
    assert_eq!(
        shown.last().map(|p| p.files_done),
        lines.last().and_then(|st| st.files_done),
        "final number of files looked through"
    );
}

/// Real restic output of 0.18.0 and the current version: the phase only changes with
/// `seconds_remaining`, and the shown percentage never falls. Whether a run this small ever gets
/// `seconds_remaining` depends on the machine, so both outcomes are accepted.
#[test]
fn real_restic_backup_progress_follows_seconds_remaining_and_never_falls() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path().join("repo");
        let src = tmp.path().join("src");
        let cache = tmp.path().join("cache");
        fs::create_dir_all(&src).expect("src dir");
        // Incompressible data, large enough that restic sends status lines before it ends.
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        for i in 0..6 {
            let data: Vec<u8> = (0..4_000_000)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed.to_le_bytes()[3]
                })
                .collect();
            fs::write(src.join(format!("file-{i}.bin")), data).expect("write source file");
        }
        // The repository is only the object of the backup: a prepared copy, not a fresh init.
        let test_repo = copy_template_repo(&restic_bin, &repo);
        let restic = |args: &[&str]| {
            Command::new(&restic_bin)
                .arg("-r")
                .arg(&repo)
                .args(args)
                .env("RESTIC_PASSWORD", &test_repo.password)
                .env("RESTIC_CACHE_DIR", &cache)
                .output()
                .expect("run restic")
        };
        let out = restic(&["backup", "--json", src.to_str().expect("utf-8 path")]);
        assert!(
            out.status.success(),
            "{label}: backup failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let lines = status_lines(&String::from_utf8_lossy(&out.stdout));
        let shown = track(&lines);
        let first_remaining = lines
            .iter()
            .position(|st| st.seconds_remaining.is_some_and(|s| s > 0))
            .unwrap_or(lines.len());

        // (a) restic sends status lines we can parse.
        assert!(!lines.is_empty(), "{label}: no status lines");
        // (b) Before seconds_remaining: scanning without percentage.
        for p in &shown[..first_remaining] {
            assert_eq!(p.phase, RunProgressPhase::Scanning, "{label}: phase");
            assert_eq!(p.percent_done, 0.0, "{label}: percent while scanning");
        }
        // (c) From seconds_remaining on: scan finished.
        for p in &shown[first_remaining..] {
            assert_eq!(p.phase, RunProgressPhase::ScanFinished, "{label}: phase");
        }
        // (d) The shown percentage never falls.
        assert_eq!(
            largest_drop(shown.iter().map(|p| p.percent_done)),
            0.0,
            "{label}: shown percentage fell"
        );
        // (e) The amount looked through is visible.
        assert!(
            shown.iter().any(|p| p.bytes_done > 0),
            "{label}: no amount looked through"
        );
    }
}
