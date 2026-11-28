// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S17 over D-Bus against real restic (0.18.0 and latest): the retention of a backup run
//! shows both restic calls, the plan and the `forget <id…>`, in the run detail («Show Command»)
//! and in the run history.

mod common;

use common::m3::{
    M3DaemonOpts, add_folder_backup, add_local_destination, daemon_paths, restic_variants,
    spawn_m3_daemon, wait_for_state,
};
use common::run_isolated_test;
use kestic_core::history::RunHistory;
use kestic_core::model::{
    BackupId, CommandRecord, KeepCount, RetentionPolicy, Run, RunId, RunStatus, Schedule,
};
use kestic_daemon::dbus_api::KesticClient;
use kestic_test_support::tools::assert_has_flag_pair;
use std::fs;
use std::time::Duration;

/// Starts a backup run and waits until it has ended.
fn run_to_end(client: &KesticClient, backup_id: &BackupId) -> Run {
    let run_id = client
        .start_backup(backup_id.as_str())
        .expect("StartBackup");
    let state = wait_for_state(client, Duration::from_secs(30), "run ended", |s| {
        s.recent_runs.iter().any(|r| {
            r.id.as_str() == run_id
                && matches!(
                    r.status,
                    RunStatus::Ok | RunStatus::Warning | RunStatus::Error
                )
        })
    });
    state
        .recent_runs
        .into_iter()
        .find(|r| r.id.as_str() == run_id)
        .expect("ended run")
}

/// The arguments after the subcommand `forget`.
fn after_forget(cmd: &CommandRecord) -> Vec<String> {
    let pos = cmd
        .args
        .iter()
        .position(|a| a == "forget")
        .unwrap_or_else(|| panic!("no forget in {:?}", cmd.args));
    cmd.args[pos + 1..].to_vec()
}

/// AC-M6-17-4: after a backup run whose retention removed a snapshot, the run detail and the
/// run history (a) show both calls, `forget --dry-run --json …` and `forget <id…>`, each with
/// its start time and exit code, and (b) no shown command contains the repository password.
#[test]
fn ac_m6_17_4_run_detail_shows_plan_and_forget() {
    run_isolated_test(
        "ac_m6_17_4_run_detail_shows_plan_and_forget",
        "KESTIC_M6_17_4_INNER",
        |root| {
            for variant in restic_variants() {
                let label = variant.label;
                let sub = variant.sub("M6R4");
                let bus = variant.bus_name("M6R4");
                let daemon =
                    spawn_m3_daemon(root, M3DaemonOpts::on_request(&sub, &bus, &variant.bin));
                let client = &daemon.client;
                let (dest, password) = add_local_destination(
                    client,
                    &variant.bin,
                    &root.join(&sub).join("repo"),
                    "Repo",
                    |_| {},
                );
                let src_dir = root.join(&sub).join("src");
                let backup = add_folder_backup(client, &src_dir, &dest.id, "Docs", |b| {
                    b.schedule = Schedule::ManualOnly;
                    b.retention = RetentionPolicy {
                        keep_last: Some(KeepCount::Count(1)),
                        ..RetentionPolicy::empty()
                    };
                });

                let first = run_to_end(client, &backup.id);
                assert_eq!(first.status, RunStatus::Ok, "{label}: first run {first:?}");
                let first_snapshot = first
                    .stats
                    .as_ref()
                    .and_then(|s| s.snapshot_id.clone())
                    .expect("first snapshot ID");
                fs::write(src_dir.join("data.txt"), "second").expect("change source");
                let second = run_to_end(client, &backup.id);
                assert_eq!(
                    second.status,
                    RunStatus::Ok,
                    "{label}: second run {second:?}"
                );

                let detail = client
                    .get_run_detail(second.id.as_str())
                    .expect("GetRunDetail");
                let commands = &detail.run.commands;
                assert_eq!(
                    commands.len(),
                    3,
                    "{label}: (a) backup, plan and forget: {commands:?}"
                );
                let (plan, forget) = (&commands[1], &commands[2]);
                let plan_args = after_forget(plan);
                assert!(
                    plan_args.iter().any(|a| a == "--dry-run")
                        && plan.args.iter().any(|a| a == "--json"),
                    "{label}: (a) the plan is `forget --dry-run --json`: {:?}",
                    plan.args
                );
                assert_has_flag_pair(&plan_args, "--tag", &backup.tag());
                assert_eq!(
                    after_forget(forget),
                    vec![first_snapshot.clone()],
                    "{label}: (a) `forget <id…>` names the removed snapshot"
                );
                let started = detail.run.started_at;
                let finished = detail.run.finished_at.expect("finished run");
                for cmd in [plan, forget] {
                    assert!(
                        cmd.started_at >= started && cmd.started_at <= finished,
                        "{label}: (a) start time within the run: {cmd:?}"
                    );
                    assert_eq!(cmd.exit_code, 0, "{label}: (a) exit code: {cmd:?}");
                }
                assert!(
                    plan.started_at <= forget.started_at,
                    "{label}: (a) the plan starts first"
                );
                let shown = &detail.transparent_commands;
                assert_eq!(
                    shown.len(),
                    3,
                    "{label}: (a) three shown commands: {shown:?}"
                );
                assert!(
                    shown[1].contains(" forget ") && shown[1].contains("--dry-run"),
                    "{label}: (a) the plan is shown: {}",
                    shown[1]
                );
                assert!(
                    shown[2].contains(" forget ") && shown[2].contains(&first_snapshot),
                    "{label}: (a) `forget <id…>` is shown: {}",
                    shown[2]
                );
                for text in shown
                    .iter()
                    .cloned()
                    .chain(commands.iter().flat_map(|c| c.args.clone()))
                {
                    assert!(
                        !text.contains(&password),
                        "{label}: (b) no repository password in a shown command"
                    );
                }

                let run_id: RunId = second.id;
                daemon.stop();
                let (_, state_dir) = daemon_paths(root, &sub);
                let stored = RunHistory::open(&state_dir)
                    .expect("open run history")
                    .get_run(&run_id)
                    .expect("read run history")
                    .expect("run in history");
                assert_eq!(
                    &stored.commands, commands,
                    "{label}: (a) the history holds the same calls with time and exit code"
                );
            }
        },
    );
}
