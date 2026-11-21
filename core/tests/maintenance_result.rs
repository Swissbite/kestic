// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Result of a maintenance run for its run detail page: parsing the output of `forget --json`,
//! the text output of `prune` and `check --json` (fixtures recorded with restic 0.18.0 and
//! 0.19.1), the completed steps and storage in the run history.

use kestic_core::backup::parse_forget_removed_count;
use kestic_core::clock::FakeClock;
use kestic_core::history::RunHistory;
use kestic_core::maintenance::{parse_check_summary, parse_prune_freed_bytes};
use kestic_core::model::{
    Backup, CheckSummary, Destination, DestinationKind, MaintenanceForget, MaintenanceOutcome,
    MaintenanceStats, Run, RunStats, RunStatus, Source,
};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use time::macros::datetime;

const RESTIC_VERSIONS: [&str; 2] = ["0.18.0", "0.19.1"];

fn fixture_lines(name: &str) -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .lines()
        .map(ToOwned::to_owned)
        .collect()
}

/// The `forget --json` output of both restic versions yields (a) the number of removed
/// snapshots; (b) output without a JSON result (e.g. an error text) and (c) an unreadable
/// result line yield none.
#[test]
fn finding1_forget_json_yields_removed_snapshot_count() {
    for version in RESTIC_VERSIONS {
        let lines = fixture_lines(&format!("forget_remove_two_restic_{version}.json"));
        assert_eq!(
            parse_forget_removed_count(&lines),
            Some(2),
            "{version}: (a) two snapshots removed"
        );
    }
    let error_text = vec!["Fatal: unable to create lock in backend".to_string()];
    assert_eq!(
        parse_forget_removed_count(&error_text),
        None,
        "(b) no result without JSON"
    );
    let cut_off = vec![r#"[{"tags":["kestic:x"],"remove":[{"id":"#.to_string()];
    assert_eq!(
        parse_forget_removed_count(&cut_off),
        None,
        "(c) no result from a cut-off line"
    );
}

/// The freed space shown in the run detail page: the `total prune:` line of the real prune output of (a) restic
/// 0.18.0 and (b) 0.19.1 yields its size in bytes (decimals cut off); sizes in (c) bytes and
/// (d) GiB are converted; (e) without the line and (f) with an unreadable size there is no
/// number and no error.
#[test]
fn freed_space_from_prune_text_line() {
    let lines = |text: &str| vec![text.to_string()];
    assert_eq!(
        parse_prune_freed_bytes(&fixture_lines("prune_restic_0.18.0.txt")),
        Some(1297),
        "(a) 1.267 KiB"
    );
    assert_eq!(
        parse_prune_freed_bytes(&fixture_lines("prune_restic_0.19.1.txt")),
        Some(1_003_857),
        "(b) 980.330 KiB"
    );
    assert_eq!(
        parse_prune_freed_bytes(&lines("total prune:           3 blobs / 512 B")),
        Some(512),
        "(c) bytes"
    );
    assert_eq!(
        parse_prune_freed_bytes(&lines("total prune:      9 blobs / 2.500 GiB")),
        Some(2_684_354_560),
        "(d) GiB"
    );
    assert_eq!(
        parse_prune_freed_bytes(&lines("remaining:  5 blobs / 586.701 KiB")),
        None,
        "(e) no total prune line"
    );
    for unreadable in [
        "total prune: 4 blobs / 1.2 XB",
        "total prune: 4 blobs / many KiB",
        "total prune: 4 blobs 1.267 KiB",
        "total prune: 4 blobs / -1 KiB",
        "total prune: 4 blobs / 99999999999999999999999 TiB",
        "total prune: 4 blobs / 1.000 PiB",
    ] {
        assert_eq!(
            parse_prune_freed_bytes(&lines(unreadable)),
            None,
            "(f) {unreadable}"
        );
    }
}

/// The `check --json` summary of both restic versions is parsed (a) with its error count; (b) a
/// backup summary, which shares `message_type: "summary"`, is not taken for it; (c) a summary
/// with errors keeps the count.
#[test]
fn finding1_check_json_summary_is_parsed_and_not_confused_with_backup_summary() {
    for version in RESTIC_VERSIONS {
        let lines = fixture_lines(&format!("check_summary_restic_{version}.json"));
        assert_eq!(
            parse_check_summary(&lines),
            Some(CheckSummary { num_errors: 0 }),
            "{version}: (a) check summary without errors"
        );
    }
    let backup_summary = vec![
        r#"{"message_type":"summary","files_new":3,"files_changed":0,"data_added":120}"#
            .to_string(),
    ];
    assert_eq!(
        parse_check_summary(&backup_summary),
        None,
        "(b) a backup summary is no check result"
    );
    let with_errors = vec![
        "using temporary cache in /tmp/restic-check-cache-1".to_string(),
        r#"{"message_type":"summary","num_errors":2,"broken_packs":["ab"],"suggest_repair_index":true,"suggest_prune":false}"#
            .to_string(),
    ];
    assert_eq!(
        parse_check_summary(&with_errors),
        Some(CheckSummary { num_errors: 2 }),
        "(c) error count kept"
    );
}

/// The status line names the completed steps: (a) both, (b) only `prune`, (c) only
/// `check`, (d) only pending `forget`s.
#[test]
fn finding1_maintenance_outcome_names_completed_steps() {
    let stats = |pruned, checked| MaintenanceStats {
        pruned,
        checked,
        ..MaintenanceStats::default()
    };
    assert_eq!(
        stats(true, true).outcome(),
        MaintenanceOutcome::CleanedUpAndVerified,
        "(a)"
    );
    assert_eq!(
        stats(true, false).outcome(),
        MaintenanceOutcome::CleanedUp,
        "(b)"
    );
    assert_eq!(
        stats(false, true).outcome(),
        MaintenanceOutcome::Verified,
        "(c)"
    );
    assert_eq!(
        stats(false, false).outcome(),
        MaintenanceOutcome::RetentionOnly,
        "(d)"
    );
}

/// The run history keeps (a) the result of a maintenance run and (b) the file
/// statistics of a backup run in the same column, read back by run kind; (c) a maintenance run
/// recorded without a result stays without one.
#[test]
fn finding1_history_keeps_maintenance_result_and_backup_stats() {
    let dir = TempDir::new().expect("tempdir");
    let clock = FakeClock::new(datetime!(2026-11-21 15:25:00 UTC));
    let dest = Destination::new(
        "Vault",
        DestinationKind::Local {
            path: "/srv/vault".into(),
        },
    )
    .expect("dest");
    let backup =
        Backup::new("Documents", Source::folder("/home/u/docs"), dest.id.clone()).expect("backup");
    let mut history = RunHistory::open(&dir.path().join("state")).expect("history");

    let stats = MaintenanceStats {
        forgets: vec![MaintenanceForget {
            backup_id: backup.id.clone(),
            removed_snapshots: Some(3),
        }],
        pruned: true,
        freed_bytes: Some(1_262_485),
        checked: true,
        check: Some(CheckSummary::default()),
    };
    let mut maintenance = Run::new_maintenance(&clock, dest.id.clone()).expect("run");
    maintenance.maintenance = Some(stats.clone());
    clock.advance(Duration::from_secs(6));
    maintenance.finish(&clock, RunStatus::Ok, Some(0));
    history
        .upsert_run(&maintenance, &[])
        .expect("store maintenance");

    let mut backup_run = Run::new_backup(&clock, &backup).expect("run");
    backup_run.stats = Some(RunStats {
        files_new: 4,
        ..RunStats::default()
    });
    backup_run.finish(&clock, RunStatus::Ok, Some(0));
    history.upsert_run(&backup_run, &[]).expect("store backup");

    let mut bare = Run::new_maintenance(&clock, dest.id.clone()).expect("run");
    bare.finish(&clock, RunStatus::Ok, Some(0));
    history
        .upsert_run(&bare, &[])
        .expect("store bare maintenance");

    let read = |run: &Run| history.get_run(&run.id).expect("read").expect("present");
    let maintenance_back = read(&maintenance);
    assert_eq!(
        maintenance_back.maintenance,
        Some(stats),
        "(a) maintenance result"
    );
    assert_eq!(maintenance_back.stats, None, "(a) no file statistics");
    let backup_back = read(&backup_run);
    assert_eq!(
        backup_back.stats.map(|s| s.files_new),
        Some(4),
        "(b) backup statistics"
    );
    assert_eq!(backup_back.maintenance, None, "(b) no maintenance result");
    assert_eq!(read(&bare).maintenance, None, "(c) no result recorded");
}
