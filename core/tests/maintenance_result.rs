// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Result of a maintenance run for its run detail page: parsing the output of `forget --json`,
//! of `prune` (text up to restic 0.19, JSON from 0.20) and of `check --json` (fixtures recorded
//! with restic 0.18.0, 0.19.1 and, for `prune`, 0.20.0), the completed steps and storage in the
//! run history.

use kestic_core::backup::parse_forget_json_output;
use kestic_core::clock::FakeClock;
use kestic_core::history::RunHistory;
use kestic_core::maintenance::{
    build_prune_command_spec, parse_check_summary, parse_prune_freed_bytes,
};
use kestic_core::model::{
    Backup, CheckSummary, Destination, DestinationKind, MaintenanceForget, MaintenanceOutcome,
    MaintenanceStats, Run, RunStats, RunStatus, Source,
};
use kestic_core::provider::ResolvedRestic;
use semver::Version;
use std::fs;
use std::path::{Path, PathBuf};
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

/// The remove list of the retention plan (`forget --json` output of both restic versions, tag
/// set to a configured backup) is what a maintenance run counts as removed: (a) the two
/// snapshots; (b) output without a JSON result (e.g. an error text) yields none, and (c) a
/// cut-off result line is an error, not an empty list.
#[test]
fn finding1_forget_json_yields_removed_snapshot_count() {
    let backup = Backup::new(
        "Fixture",
        Source::folder("/tmp/kestic-fixture-src"),
        Destination::new(
            "Fixture",
            DestinationKind::Local {
                path: PathBuf::from("/tmp/kestic-fixture-repo"),
            },
        )
        .expect("destination")
        .id,
    )
    .expect("backup");
    for version in RESTIC_VERSIONS {
        let lines: Vec<String> = fixture_lines(&format!("forget_remove_two_restic_{version}.json"))
            .into_iter()
            .map(|l| l.replace("kestic:bk_fixture", &backup.tag()))
            .collect();
        let plan = parse_forget_json_output(&backup.id, &lines).expect("parse plan");
        assert_eq!(
            plan.removal_ids().len(),
            2,
            "{version}: (a) two snapshots removed"
        );
    }
    let error_text = vec!["Fatal: unable to create lock in backend".to_string()];
    assert!(
        parse_forget_json_output(&backup.id, &error_text)
            .expect("no JSON line")
            .removal_ids()
            .is_empty(),
        "(b) no result without JSON"
    );
    let cut_off = vec![r#"[{"tags":["kestic:x"],"remove":[{"id":"#.to_string()];
    assert!(
        parse_forget_json_output(&backup.id, &cut_off).is_err(),
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

fn prune_spec_argv(version: Version, dry_run: bool) -> Vec<String> {
    let dest = Destination::new(
        "Vault",
        DestinationKind::Local {
            path: "/srv/vault".into(),
        },
    )
    .expect("dest");
    let resolved = ResolvedRestic {
        binary: PathBuf::from("/usr/bin/restic"),
        version,
    };
    build_prune_command_spec(
        &dest,
        dry_run,
        dry_run,
        &resolved,
        Path::new("/usr/bin/kestic"),
    )
    .restic_argv()
}

/// issue-42-AC1: the real `prune` (a) of restic 0.19.1 keeps its arguments without `--json`;
/// (b) of restic 0.20.0 carries `--json`, otherwise unchanged; (c) the dry-run preview of 0.20.0
/// stays without `--json`.
#[test]
fn issue_42_ac1_real_prune_carries_json_from_restic_0_20() {
    assert_eq!(
        prune_spec_argv(Version::new(0, 19, 1), false),
        ["-r", "/srv/vault", "prune"],
        "(a) 0.19.1 unchanged"
    );
    assert_eq!(
        prune_spec_argv(Version::new(0, 20, 0), false),
        ["-r", "/srv/vault", "--json", "prune"],
        "(b) 0.20.0 with --json"
    );
    assert_eq!(
        prune_spec_argv(Version::new(0, 20, 0), true),
        ["-r", "/srv/vault", "prune", "--dry-run", "--no-lock"],
        "(c) dry run without --json"
    );
}

/// issue-42-AC2: from the JSON output of restic 0.20.0 the freed bytes are (a) `bytes.remove_total`
/// of the `summary` message, (b) also with unknown fields; (c) without a summary, (d) with a
/// summary without `bytes` and (e) with an unreadable `remove_total` there is no number.
#[test]
fn issue_42_ac2_freed_bytes_from_prune_json() {
    let lines = |text: &str| vec![text.to_string()];
    assert_eq!(
        parse_prune_freed_bytes(&lines(
            r#"{"message_type":"summary","bytes":{"remove":10,"repack_remove":5,"remove_total":15}}"#
        )),
        Some(15),
        "(a) remove_total"
    );
    assert_eq!(
        parse_prune_freed_bytes(&lines(
            r#"{"message_type":"summary","future":[1],"bytes":{"remove_total":42,"novel":{"x":1}}}"#
        )),
        Some(42),
        "(b) unknown fields ignored"
    );
    assert_eq!(
        parse_prune_freed_bytes(&lines(
            r#"{"message_type":"exit_error","code":1,"message":"Fatal: repository is locked"}"#
        )),
        None,
        "(c) no summary"
    );
    assert_eq!(
        parse_prune_freed_bytes(&lines(
            r#"{"message_type":"summary","files_new":3,"data_added":120}"#
        )),
        None,
        "(d) summary without bytes"
    );
    assert_eq!(
        parse_prune_freed_bytes(&lines(
            r#"{"message_type":"summary","bytes":{"remove_total":"many"}}"#
        )),
        None,
        "(e) unreadable remove_total"
    );
}

/// issue-42-AC3: the recorded real `prune` output yields the freed bytes for (a) restic 0.18.0
/// (text), (b) 0.19.1 (text) and (c) 0.20.0 (JSON, a run that also repacks).
#[test]
fn issue_42_ac3_freed_bytes_from_prune_fixtures_of_all_test_versions() {
    assert_eq!(
        parse_prune_freed_bytes(&fixture_lines("prune_restic_0.18.0.txt")),
        Some(1297),
        "(a) 0.18.0: 1.267 KiB"
    );
    assert_eq!(
        parse_prune_freed_bytes(&fixture_lines("prune_restic_0.19.1.txt")),
        Some(1_003_857),
        "(b) 0.19.1: 980.330 KiB"
    );
    assert_eq!(
        parse_prune_freed_bytes(&fixture_lines("prune_restic_0.20.0.json")),
        Some(400_983),
        "(c) 0.20.0: bytes.remove_total"
    );
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
