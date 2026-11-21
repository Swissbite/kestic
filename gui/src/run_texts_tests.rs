// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `run_texts`: run detail page of a maintenance run.

use super::*;
use kestic_core::clock::FakeClock;
use kestic_core::model::{
    Backup, CheckSummary, Destination, DestinationKind, MaintenanceForget, Source,
};
use std::time::Duration;
use time::macros::datetime;

fn sample_backup() -> Backup {
    let dest = Destination::new(
        "NAS",
        DestinationKind::Local {
            path: "/srv/nas".into(),
        },
    )
    .expect("dest");
    Backup::new("Documents", Source::folder("/home/u/docs"), dest.id).expect("backup")
}

fn maintenance_run(stats: Option<MaintenanceStats>) -> Run {
    let clock = FakeClock::new(datetime!(2026-11-21 15:25:00 UTC));
    let mut run = Run::new_maintenance(&clock, sample_backup().destination_id).expect("run");
    run.maintenance = stats;
    clock.advance(Duration::from_secs(6));
    run.finish(&clock, RunStatus::Ok, Some(0));
    run
}

fn full_stats(backup_id: &BackupId) -> MaintenanceStats {
    MaintenanceStats {
        forgets: vec![MaintenanceForget {
            backup_id: backup_id.clone(),
            removed_snapshots: Some(2),
        }],
        pruned: true,
        checked: true,
        check: Some(CheckSummary::default()),
    }
}

/// The page title of a maintenance run reads «Maintenance on …»; a backup run keeps
/// «Run on …».
#[test]
fn finding1_maintenance_title_names_the_run_kind() {
    let backup = sample_backup();
    let run = maintenance_run(None);
    let started = format_datetime_display(run.started_at);
    assert_eq!(run_detail_title(&run), format!("Maintenance on {started}"));
    let clock = FakeClock::new(datetime!(2026-11-21 15:25:00 UTC));
    let backup_run = Run::new_backup(&clock, &backup).expect("run");
    assert_eq!(run_detail_title(&backup_run), format!("Run on {started}"));
}

/// The status of a successful maintenance run names its steps: (a) both, (b) only
/// clean up, (c) only verify, (d) neither or no recorded result: «Completed».
#[test]
fn finding1_maintenance_status_names_completed_steps() {
    let with = |pruned, checked| {
        run_status(&maintenance_run(Some(MaintenanceStats {
            pruned,
            checked,
            ..MaintenanceStats::default()
        })))
        .0
    };
    assert_eq!(with(true, true), "Cleaned up and verified", "(a)");
    assert_eq!(with(true, false), "Cleaned up", "(b)");
    assert_eq!(with(false, true), "Verified", "(c)");
    assert_eq!(with(false, false), "Completed", "(d) only forget");
    assert_eq!(
        run_status(&maintenance_run(None)).0,
        "Completed",
        "(d) old run"
    );
}

/// Below the status a maintenance run shows its destination, not «… added».
#[test]
fn finding1_maintenance_subtitle_is_the_destination() {
    let run = maintenance_run(None);
    assert_eq!(run_detail_subtitle(&run, None, "NAS"), "NAS");
}

/// The «Result» tab lists (a) the removed snapshots per backup and (b) the check
/// result; (c) one removed snapshot uses the singular; (d) without forget it says that no
/// snapshot was removed; (e) a deleted backup is named as deleted.
#[test]
fn finding1_maintenance_result_lists_removed_snapshots_and_check() {
    let backup = sample_backup();
    let names = |id: &BackupId| (id == &backup.id).then(|| backup.name.clone());
    let mut stats = full_stats(&backup.id);
    let text = run_result_summary(&maintenance_run(Some(stats.clone())), &names);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("Documents: 2 snapshots removed"),
        "(a)"
    );
    assert_eq!(
        lines.get(1).copied(),
        Some("Verify: no errors found"),
        "(b)"
    );

    stats.forgets[0].removed_snapshots = Some(1);
    let single = run_result_summary(&maintenance_run(Some(stats.clone())), &names);
    assert!(
        single.starts_with("Documents: 1 snapshot removed\n"),
        "(c) singular: {single}"
    );

    stats.forgets.clear();
    let none = run_result_summary(&maintenance_run(Some(stats.clone())), &names);
    assert!(none.starts_with("No snapshots removed."), "(d) {none}");

    stats.forgets.push(MaintenanceForget {
        backup_id: BackupId::generate().expect("id"),
        removed_snapshots: Some(3),
    });
    let deleted = run_result_summary(&maintenance_run(Some(stats)), &names);
    assert!(
        deleted.starts_with("Deleted backup: 3 snapshots removed"),
        "(e) {deleted}"
    );
}

/// A check that found errors names their number (singular for one).
#[test]
fn finding1_maintenance_result_counts_check_errors() {
    let backup = sample_backup();
    let mut stats = full_stats(&backup.id);
    stats.checked = false;
    stats.check = Some(CheckSummary { num_errors: 1 });
    let text = run_result_summary(&maintenance_run(Some(stats)), &|_| None);
    assert!(text.ends_with("Verify: 1 error found"), "{text}");
}

/// The «Result» tab claims nothing it does not know: (a) a forget without a readable count names
/// no number; (b) after a failed forget (run `Warning`) it does not say that no snapshot was
/// removed; (c) a run that is still active shows no result yet.
#[test]
fn finding1_maintenance_result_claims_nothing_unknown() {
    let backup = sample_backup();
    let names = |id: &BackupId| (id == &backup.id).then(|| backup.name.clone());
    let mut stats = full_stats(&backup.id);
    stats.forgets[0].removed_snapshots = None;
    let unknown = run_result_summary(&maintenance_run(Some(stats.clone())), &names);
    assert!(
        unknown.starts_with("Documents: snapshots removed\n"),
        "(a) {unknown}"
    );

    stats.forgets.clear();
    let mut warned = maintenance_run(Some(stats));
    warned.status = RunStatus::Warning;
    assert_eq!(
        run_result_summary(&warned, &names),
        "Verify: no errors found",
        "(b)"
    );

    let clock = FakeClock::new(datetime!(2026-11-21 15:25:00 UTC));
    let active = Run::new_maintenance(&clock, backup.destination_id.clone()).expect("run");
    assert_eq!(run_result_summary(&active, &names), "", "(c)");
}
