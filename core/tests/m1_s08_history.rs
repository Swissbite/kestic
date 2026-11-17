// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use kestic_core::clock::{Clock, FakeClock};
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, CommandRecord, Destination, DestinationKind, Run, RunStats, RunStatus, Source,
};
use std::fs;
use std::time::Duration;
use tempfile::TempDir;
use time::macros::datetime;

#[test]
fn ac_m1_08_1_08_2_08_3_sqlite_history_redacted_logs_and_commands() {
    let dir = TempDir::new().expect("tempdir");
    let state_dir = dir.path().join("state");
    let clock = FakeClock::new(datetime!(2026-12-01 09:00:00 UTC));

    let dest = Destination::new(
        "REST",
        DestinationKind::Rest {
            url: "https://example.invalid/repo".to_string(),
            username: Some("alice".to_string()),
        },
    )
    .expect("dest");
    let backup = Backup::new("Home", Source::folder("/home/alice"), dest.id.clone()).expect("bk");

    let mut run = Run::new_backup(&clock, &backup).expect("run");
    let repo_pw = "UltraSecretRepoPassword!";
    let rest_pw = "UltraSecretRestPassword!";

    clock.advance(Duration::from_secs(42));
    run.finish(&clock, RunStatus::Ok, Some(0));
    run.summary_message = Some(format!("Done without leaking {repo_pw}"));
    run.stats = Some(RunStats {
        files_new: 5,
        total_files_processed: 10,
        snapshot_id: Some("a1b2c3d4".to_string()),
        ..RunStats::default()
    });
    run.commands.push(CommandRecord {
        executable: "/usr/bin/restic".to_string(),
        args: vec![
            "-r".to_string(),
            dest.kind.repository_uri(),
            "backup".to_string(),
            "/home/alice".to_string(),
        ],
        env_keys: vec![
            "RESTIC_PASSWORD_COMMAND".to_string(),
            "RESTIC_REST_USERNAME".to_string(),
            "RESTIC_REST_PASSWORD".to_string(),
        ],
        started_at: datetime!(2026-12-01 09:00:00 UTC),
        duration_ms: 42_000,
        exit_code: 0,
    });

    {
        let mut history = RunHistory::open(&state_dir).expect("open history");
        let log_path = history
            .write_redacted_log(
                &run.id,
                &[
                    format!("debug using {repo_pw} and {rest_pw}"),
                    "snapshot a1b2c3d4 saved".to_string(),
                ],
                &[repo_pw, rest_pw],
            )
            .expect("write log");
        run.log_path = Some(log_path);
        history
            .upsert_run(&run, &[repo_pw, rest_pw])
            .expect("upsert run");
    }

    // Reopen SQLite after simulated daemon restart (`AC-M1-08-1`).
    let reopened = RunHistory::open(&state_dir).expect("reopen history");
    let runs = reopened.list_runs().expect("list runs");
    assert_eq!(runs.len(), 1);
    let loaded = &runs[0];
    assert_eq!(loaded.id, run.id);
    assert_eq!(loaded.status, RunStatus::Ok);
    assert_eq!(loaded.exit_code, Some(0));
    assert_eq!(loaded.duration_ms, Some(42_000));
    assert_eq!(loaded.commands.len(), 1);
    assert_eq!(loaded.commands[0].executable, "/usr/bin/restic");
    assert_eq!(loaded.commands[0].exit_code, 0);
    assert_eq!(loaded.commands[0].duration_ms, 42_000);

    // AC-M1-08-2: Zero secret leaks in summary or log file.
    assert!(!loaded.summary_message.as_deref().unwrap().contains(repo_pw));
    let log_text =
        fs::read_to_string(loaded.log_path.as_ref().expect("log path")).expect("read log file");
    assert!(!log_text.contains(repo_pw));
    assert!(!log_text.contains(rest_pw));
    assert!(log_text.contains("***REDACTED***"));
    assert!(log_text.contains("snapshot a1b2c3d4 saved"));
}

#[test]
fn ac_m1_08_4_log_retention_deletes_only_logs_older_than_90_days_and_outside_last_100() {
    let dir = TempDir::new().expect("tempdir");
    let state_dir = dir.path().join("state");
    let clock = FakeClock::new(datetime!(2026-12-31 12:00:00 UTC));

    let dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: "/tmp/repo".into(),
        },
    )
    .expect("dest");
    let backup = Backup::new("Home", Source::folder("/home/alice"), dest.id).expect("bk");

    let mut history = RunHistory::open(&state_dir).expect("open");

    // Create 120 runs:
    // - 30 runs older than 90 days (e.g., 120..=91 days before 2026-12-31)
    // - 90 runs younger than 90 days (e.g., 89..=0 days before 2026-12-31)
    // Newest 100 runs = 90 younger + 10 newest of the 30 old runs.
    // Remaining 20 oldest runs are BOTH > 90 days old AND outside newest 100 -> deleted!
    let now = clock.now_utc();
    let mut created_ids = Vec::new();
    for age_days in (1..=30).map(|i| 90 + i) {
        let ts = now - time::Duration::days(age_days);
        clock.set(ts);
        let mut run = Run::new_backup(&clock, &backup).expect("old run");
        run.finish(&clock, RunStatus::Ok, Some(0));
        let log_path = history
            .write_redacted_log(&run.id, &[format!("log age {age_days}d")], &[])
            .expect("log");
        run.log_path = Some(log_path);
        history.upsert_run(&run, &[]).expect("upsert");
        created_ids.push((age_days, run.id));
    }
    for age_days in 0..90 {
        let ts = now - time::Duration::days(age_days);
        clock.set(ts);
        let mut run = Run::new_backup(&clock, &backup).expect("recent run");
        run.finish(&clock, RunStatus::Ok, Some(0));
        let log_path = history
            .write_redacted_log(&run.id, &[format!("log age {age_days}d")], &[])
            .expect("log");
        run.log_path = Some(log_path);
        history.upsert_run(&run, &[]).expect("upsert");
        created_ids.push((age_days, run.id));
    }

    clock.set(now);
    let deleted = history
        .prune_logs_for_backup(&backup.id, &clock)
        .expect("prune");
    assert_eq!(deleted, 20, "expected exactly 20 old logs deleted");

    let remaining_files = fs::read_dir(state_dir.join("logs"))
        .expect("read logs dir")
        .count();
    assert_eq!(remaining_files, 100, "expected exactly 100 logs retained");
}

#[test]
fn ac_m1_10_2_running_runs_become_interrupted_on_startup() {
    let dir = TempDir::new().expect("tempdir");
    let state_dir = dir.path().join("state");
    let clock = FakeClock::new(datetime!(2026-12-01 08:00:00 UTC));

    let dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: "/tmp/repo".into(),
        },
    )
    .expect("dest");
    let backup = Backup::new("Home", Source::folder("/home/alice"), dest.id).expect("bk");

    let run = Run::new_backup(&clock, &backup).expect("running run");
    assert_eq!(run.status, RunStatus::Running);

    {
        let mut history = RunHistory::open(&state_dir).expect("open");
        history.upsert_run(&run, &[]).expect("save running");
    }

    clock.advance(Duration::from_secs(300));
    let history = RunHistory::open(&state_dir).expect("reopen");
    let updated = history
        .mark_interrupted_on_startup(&clock)
        .expect("mark interrupted");
    assert_eq!(updated, 1);

    let loaded = history.get_run(&run.id).expect("get").expect("exists");
    assert_eq!(loaded.status, RunStatus::Interrupted);
    assert_eq!(loaded.finished_at, Some(datetime!(2026-12-01 08:05:00 UTC)));
}
