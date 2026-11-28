// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `daemon_client` (kept in their own file, NFR-12 file size).

use super::*;
use kestic_core::model::*;
use time::macros::datetime;

fn sample_backup() -> Backup {
    let dest = Destination::new(
        "Vault",
        DestinationKind::Local {
            path: "/srv/vault".into(),
        },
    )
    .expect("dest");
    Backup::new("Documents", Source::folder("/home/u/docs"), dest.id).expect("backup")
}

fn progress_of(run: &Run, percent_done: f64) -> ActiveRunProgressDto {
    ActiveRunProgressDto {
        run_id: run.id.as_str().to_string(),
        kind: Default::default(),
        backup_id: None,
        backup_name: String::new(),
        phase: Default::default(),
        percent_done,
        bytes_done: 0,
        total_bytes: 0,
        files_done: 0,
        speed_bps: 0,
        seconds_remaining: 0,
    }
}

/// M4-S05, a restore beside a backup (AC-M4-05-9): a state refresh, whose overall progress is
/// the backup's, (a) keeps the progress of the running restore as it was, (b) adds the
/// backup's, and (c) drops the progress of a run that ended.
#[test]
fn m4_s05_restore_progress_survives_a_state_refresh_beside_a_backup() {
    let clock = kestic_core::clock::FakeClock::new(datetime!(2026-11-23 12:00:00 UTC));
    let backup_run = |status| {
        let mut run = Run::new_backup(&clock, &sample_backup()).expect("run");
        run.status = status;
        run
    };
    let mut restore = Run::new_restore(&clock, sample_backup().destination_id, None).expect("run");
    restore.status = RunStatus::Running;
    let running_backup = backup_run(RunStatus::Running);
    let ended = backup_run(RunStatus::Ok);
    let mut progress = HashMap::from([
        (restore.id.as_str().to_string(), progress_of(&restore, 40.0)),
        (ended.id.as_str().to_string(), progress_of(&ended, 100.0)),
    ]);

    retain_active_progress(
        &mut progress,
        &[],
        &[running_backup.clone(), restore.clone(), ended.clone()],
        Some(&progress_of(&running_backup, 5.0)),
    );

    assert_eq!(
        progress.get(restore.id.as_str()).map(|p| p.percent_done),
        Some(40.0),
        "(a)"
    );
    assert_eq!(
        progress
            .get(running_backup.id.as_str())
            .map(|p| p.percent_done),
        Some(5.0),
        "(b)"
    );
    assert!(!progress.contains_key(ended.id.as_str()), "(c)");
}
