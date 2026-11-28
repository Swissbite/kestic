// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S05 per-destination queue and maintenance runs (`AC-M3-05-1`, `-3`, `-4`, `-5`, `-6`) and
//! the recorded result of a maintenance run (including `issue-42-AC2`, `-AC4`).

mod common;

use common::m3::{
    LocalBackupFixture, M3Rig, ResticVariant, StdFakes, backup_summary, connect_local_repo,
    dest_summary, hold_repo_lock, latest_variant, restic_repo_cmd, restic_variants, settle,
    wait_for_state,
};
use common::run_isolated_test;
use kestic_core::maintenance::PRUNE_JSON_MIN_VERSION;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, DestinationKind, MaintenanceForget, RunKind,
    RunStatus, Schedule, Source,
};
use kestic_core::provider::check_binary_version;
use kestic_core::retention::{KeepCount, RetentionPolicy};
use kestic_daemon::dbus_api::DaemonStateDto;
use kestic_test_support::tools::write_executable;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use time::OffsetDateTime;
use time::macros::datetime;

/// Friday 2026-12-04 11:59:30 Europe/Zurich, before the first-Sunday maintenance slot.
const FRIDAY: OffsetDateTime = datetime!(2026-12-04 10:59:30 UTC);
/// Sunday 2026-12-06 12:00:05 Europe/Zurich: `prune` (every Sunday) and `check` (first Sunday) due.
const SUNDAY_1200: OffsetDateTime = datetime!(2026-12-06 11:00:05 UTC);
/// Sunday 2026-12-06 12:40 Europe/Zurich.
const SUNDAY_1240: OffsetDateTime = datetime!(2026-12-06 11:40:00 UTC);
/// 2026-12-01 12:00:05 Europe/Zurich.
const TUESDAY_1200: OffsetDateTime = datetime!(2026-12-01 11:00:05 UTC);

/// Adds a backup (given schedule) to an existing destination through the daemon API.
fn add_backup_on(
    rig: &M3Rig,
    root: &Path,
    dest_id: &DestinationId,
    name: &str,
    schedule: Schedule,
) -> (BackupId, String) {
    let src = root.join(rig.variant.sub("src")).join(name);
    fs::create_dir_all(&src).expect("mkdir src");
    fs::write(src.join("f.txt"), format!("data {name}")).expect("write src");
    let mut backup = Backup::new(name, Source::folder(&src), dest_id.clone()).expect("backup");
    backup.schedule = schedule;
    let id_str = rig.daemon.client.add_backup(&backup).expect("add backup");
    (BackupId::parse(&id_str).expect("id"), id_str)
}

/// Adds an empty local destination (maintenance only) and waits one tick for its baseline.
fn add_local_dest(rig: &M3Rig, root: &Path, name: &str) -> Destination {
    let repo = root.join(rig.variant.sub("repo")).join(name);
    fs::create_dir_all(&repo).expect("mkdir repo");
    let dest = Destination::new(name, DestinationKind::Local { path: repo }).expect("dest");
    connect_local_repo(&rig.daemon.client, &rig.variant.bin, &dest);
    std::thread::sleep(Duration::from_millis(1300));
    dest
}

fn maintenance_done(state: &DaemonStateDto, dest_id: &DestinationId) -> bool {
    let d = dest_summary(state, dest_id);
    d.last_prune_at_unix.is_some() && d.last_check_at_unix.is_some() && !d.maintenance_running
}

/// AC-M3-05-1 (§5.1 queue): backups A and B on the same destination, both due 12:00 ->
/// (a) they run one after the other, (b) B is `ausstehend` (queue) while A runs.
#[test]
fn ac_m3_05_1_backups_on_same_destination_run_sequentially_second_is_queued() {
    run_isolated_test(
        "ac_m3_05_1_backups_on_same_destination_run_sequentially_second_is_queued",
        "KESTIC_M3_05_1_INNER",
        |root| {
            let _fakes = StdFakes::start();
            for variant in restic_variants() {
                let rig = M3Rig::start(
                    root,
                    &variant,
                    "M3Q1",
                    TUESDAY_1200 - time::Duration::seconds(35),
                    |_| {},
                );
                let client = &rig.daemon.client;
                let fx = rig.add_backup("Alpha", |_| {});
                let (beta_id, _) = add_backup_on(
                    &rig,
                    root,
                    &fx.dest.id,
                    "Beta",
                    Schedule::Daily {
                        hour: 12,
                        minute: 0,
                    },
                );
                let alpha_id = fx.backup.id;
                rig.wrapper.hold("backup");

                rig.clock.set(TUESDAY_1200);
                let running = wait_for_state(client, Duration::from_secs(15), "one running", |s| {
                    backup_summary(s, &alpha_id).active_run.is_some()
                        || backup_summary(s, &beta_id).active_run.is_some()
                });
                let (first, second) = if backup_summary(&running, &alpha_id).active_run.is_some() {
                    (&alpha_id, &beta_id)
                } else {
                    (&beta_id, &alpha_id)
                };
                // (b) the other backup waits in the destination queue.
                let queued =
                    wait_for_state(client, Duration::from_secs(10), "second queued", |s| {
                        backup_summary(s, second)
                            .pending_primary_code
                            .map(|c| c.as_str())
                            == Some("queue")
                    });
                assert!(
                    backup_summary(&queued, first).active_run.is_some(),
                    "{}: (b) first backup is still running while the second is queued",
                    variant.label
                );
                assert_eq!(
                    backup_summary(&queued, second).active_run,
                    None,
                    "{}: (b) second backup has not started",
                    variant.label
                );
                settle();
                assert_eq!(
                    rig.wrapper.of('S', "backup").len(),
                    1,
                    "{}: (a) only one restic backup process while the first holds the destination",
                    variant.label
                );

                // (a) after the first one ends the second runs: start/end never interleave.
                rig.wrapper.release("backup");
                wait_for_state(client, Duration::from_secs(30), "both Ok", |s| {
                    [&alpha_id, &beta_id].iter().all(|id| {
                        backup_summary(s, id)
                            .last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Ok)
                    })
                });
                let kinds: String = rig
                    .wrapper
                    .entries()
                    .iter()
                    .filter(|e| e.sub == "backup" && (e.kind == 'S' || e.kind == 'E'))
                    .map(|e| e.kind)
                    .collect();
                assert_eq!(
                    kinds, "SESE",
                    "{}: (a) sequential, no overlap",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-05-3 (FR-45): (a) a due maintenance run starts only after the running backup ended, and
/// (b) no backup starts on the destination while maintenance runs.
#[test]
fn ac_m3_05_3_maintenance_waits_for_backup_and_blocks_backups_on_destination() {
    run_isolated_test(
        "ac_m3_05_3_maintenance_waits_for_backup_and_blocks_backups_on_destination",
        "KESTIC_M3_05_3_INNER",
        |root| {
            let _fakes = StdFakes::start();
            for variant in restic_variants() {
                let rig = M3Rig::start(
                    root,
                    &variant,
                    "M3Q3",
                    datetime!(2026-12-06 10:59:30 UTC),
                    |_| {},
                );
                let client = &rig.daemon.client;
                let alpha = rig.add_backup("Alpha", |_| {});
                let (beta_id, _) = add_backup_on(
                    &rig,
                    root,
                    &alpha.dest.id,
                    "Beta",
                    Schedule::Daily {
                        hour: 12,
                        minute: 30,
                    },
                );
                let dest_id = alpha.dest.id.clone();
                std::thread::sleep(Duration::from_millis(1300));
                rig.wrapper.hold("backup");
                rig.wrapper.hold("prune");

                // Alpha (12:00) and the maintenance (Sunday 12:00) are due together; Alpha runs first.
                rig.clock.set(SUNDAY_1200);
                wait_for_state(client, Duration::from_secs(15), "alpha running", |s| {
                    backup_summary(s, &alpha.backup.id).active_run.is_some()
                });
                settle();
                // (a) no maintenance process while the backup runs.
                assert_eq!(
                    rig.wrapper.of('S', "prune").len() + rig.wrapper.of('S', "check").len(),
                    0,
                    "{}: (a) maintenance must wait for the running backup",
                    variant.label
                );
                let during_backup = client.get_state().expect("GetState");
                assert!(
                    !dest_summary(&during_backup, &dest_id).maintenance_running,
                    "{}: (a) maintenance not running yet",
                    variant.label
                );

                // Backup ends -> maintenance starts (its `prune` is held by the wrapper).
                rig.wrapper.release("backup");
                rig.wrapper
                    .wait_for('S', "prune", 1, Duration::from_secs(30));
                // Ordering by log position, not by wall-clock timestamps (CI clocks may step).
                let log = rig.wrapper.entries();
                let backup_end_idx = log
                    .iter()
                    .rposition(|e| e.kind == 'E' && e.sub == "backup")
                    .expect("backup end");
                let prune_start_idx = log
                    .iter()
                    .position(|e| e.kind == 'S' && e.sub == "prune")
                    .expect("prune start");
                assert!(
                    prune_start_idx > backup_end_idx,
                    "{}: (a) maintenance started after the backup ended; calls:\n{}",
                    variant.label,
                    log.iter()
                        .map(|e| format!("{} {} {} {}", e.kind, e.at_ms, e.sub, e.detail))
                        .collect::<Vec<_>>()
                        .join("\n")
                );

                // (b) Beta becomes due (12:30) while maintenance runs: it must queue, not start.
                rig.clock.set(SUNDAY_1240);
                wait_for_state(client, Duration::from_secs(10), "beta queued", |s| {
                    backup_summary(s, &beta_id)
                        .pending_primary_code
                        .map(|c| c.as_str())
                        == Some("queue")
                });
                settle();
                let during_maintenance = client.get_state().expect("GetState");
                assert!(
                    dest_summary(&during_maintenance, &dest_id).maintenance_running,
                    "{}: (b) maintenance is running",
                    variant.label
                );
                assert_eq!(
                    rig.wrapper.of('S', "backup").len(),
                    1,
                    "{}: (b) no backup process starts during maintenance",
                    variant.label
                );

                // After maintenance Beta runs.
                rig.wrapper.release("prune");
                wait_for_state(client, Duration::from_secs(40), "beta Ok", |s| {
                    backup_summary(s, &beta_id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });
                // Ordering by log position, not by wall-clock timestamps (CI clocks may step).
                let log = rig.wrapper.entries();
                let call_log: Vec<String> = log
                    .iter()
                    .map(|e| format!("{} {} {} {}", e.kind, e.at_ms, e.sub, e.detail))
                    .collect();
                let maintenance_end_idx = log
                    .iter()
                    .rposition(|e| e.kind == 'E' && (e.sub == "prune" || e.sub == "check"))
                    .expect("maintenance end");
                let beta_start_idx = log
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.kind == 'S' && e.sub == "backup")
                    .map(|(i, _)| i)
                    .nth(1)
                    .expect("beta start");
                assert!(
                    beta_start_idx > maintenance_end_idx,
                    "{}: (b) backup started after maintenance ended; calls:\n{}",
                    variant.label,
                    call_log.join("\n")
                );
                rig.daemon.stop();
            }
        },
    );
}

/// Backup «Alpha» (manual only, `configure` applied) whose last run ended `Warnung` with an open
/// `forget`: a first backup succeeds, a second one runs under a foreign non-exclusive lock, so
/// its backup exits 0 and its `forget` exits 11. The foreign backup then ends cleanly and removes
/// its lock (no stale lock).
fn backup_with_open_forget(
    rig: &M3Rig,
    root: &Path,
    configure: impl FnOnce(&mut Backup),
) -> LocalBackupFixture {
    let client = &rig.daemon.client;
    let alpha = rig.add_backup("Alpha", |b| {
        b.schedule = Schedule::ManualOnly;
        configure(b);
    });
    let alpha_id = alpha.backup.id.clone();
    client.start_backup(&alpha.backup_id_str).expect("seed run");
    wait_for_state(client, Duration::from_secs(30), "seed Ok", |s| {
        backup_summary(s, &alpha_id)
            .last_run
            .as_ref()
            .is_some_and(|r| r.status == RunStatus::Ok)
    });
    // The second run backs up other content, so forgetting the seed snapshot leaves data to prune.
    fs::write(
        alpha.src_dir.join("data.txt"),
        "payload Alpha, second version",
    )
    .expect("change src");

    let mut cmd = restic_repo_cmd(
        &rig.variant.bin,
        &alpha.repo_dir,
        &alpha.password,
        &root.join("restic-cache"),
    );
    cmd.args(["backup", "--stdin", "--stdin-filename", "hold.txt"]);
    let mut holder = hold_repo_lock(cmd, &alpha.repo_dir);
    std::thread::sleep(Duration::from_millis(30));
    client
        .start_backup(&alpha.backup_id_str)
        .expect("run under lock");
    let warned = wait_for_state(client, Duration::from_secs(30), "Warning", |s| {
        backup_summary(s, &alpha_id)
            .last_run
            .as_ref()
            .is_some_and(|r| r.status == RunStatus::Warning)
    });
    assert!(
        backup_summary(&warned, &alpha_id).pending_forget,
        "{}: precondition: forget is open",
        rig.variant.label
    );
    if let Some(mut stdin) = holder.0.stdin.take() {
        let _ = stdin.write_all(b"done");
    }
    holder.0.wait().expect("foreign backup finishes");
    alpha
}

/// AC-M3-05-4: backup A with an open `forget` (last run `Warnung`, Exit 11) -> when the
/// maintenance run starts, (a) first `forget --tag kestic:<A> --group-by tags`, (b) then `prune`.
#[test]
fn ac_m3_05_4_maintenance_runs_open_forget_before_prune() {
    run_isolated_test(
        "ac_m3_05_4_maintenance_runs_open_forget_before_prune",
        "KESTIC_M3_05_4_INNER",
        |root| {
            let _fakes = StdFakes::start();
            for variant in restic_variants() {
                let rig = M3Rig::start(root, &variant, "M3Q4", FRIDAY, |_| {});
                let client = &rig.daemon.client;
                // Keeping only the last snapshot gives the retention one to remove, so its
                // `forget <id>` needs the exclusive lock the foreign backup holds.
                let alpha = backup_with_open_forget(&rig, root, |b| {
                    b.retention = RetentionPolicy {
                        keep_last: Some(KeepCount::Count(1)),
                        ..RetentionPolicy::empty()
                    };
                });
                let alpha_id = alpha.backup.id.clone();
                let seen = rig.wrapper.entries().len();

                // Sunday 12:00: maintenance starts.
                rig.clock.set(SUNDAY_1200);
                let done =
                    wait_for_state(client, Duration::from_secs(40), "maintenance done", |s| {
                        maintenance_done(s, &alpha.dest.id)
                    });
                assert_eq!(
                    dest_summary(&done, &alpha.dest.id).pending_forget_count,
                    0,
                    "{}: open forget was worked off",
                    variant.label
                );
                // Ordering by log position, not by wall-clock timestamps (CI clocks may step).
                let log: Vec<_> = rig.wrapper.entries().into_iter().skip(seen).collect();
                let call_log: Vec<String> = log
                    .iter()
                    .map(|e| format!("{} {} {} {}", e.kind, e.at_ms, e.sub, e.detail))
                    .collect();
                let calls: Vec<_> = log
                    .iter()
                    .filter(|e| e.kind == 'S' && (e.sub == "forget" || e.sub == "prune"))
                    .cloned()
                    .collect();
                let forget_pos = calls
                    .iter()
                    .position(|e| e.sub == "forget")
                    .expect("maintenance runs forget");
                let prune_pos = calls
                    .iter()
                    .position(|e| e.sub == "prune")
                    .expect("maintenance runs prune");
                let forget = &calls[forget_pos];
                // (a) `forget --tag kestic:<A> --group-by tags` comes first.
                assert!(
                    forget
                        .detail
                        .contains(&format!("--tag kestic:{}", alpha_id.as_str())),
                    "{}: (a) forget is limited to the tag of backup A: {}",
                    variant.label,
                    forget.detail
                );
                assert!(
                    forget.detail.contains("--group-by tags"),
                    "{}: (a) forget groups by tags: {}",
                    variant.label,
                    forget.detail
                );
                // (b) `prune` runs after it.
                assert!(
                    forget_pos < prune_pos,
                    "{}: (b) prune after forget: {calls:?}",
                    variant.label
                );
                let forget_start_idx = log
                    .iter()
                    .position(|e| e.kind == 'S' && e.sub == "forget")
                    .expect("forget start");
                let forget_end_idx = log
                    .iter()
                    .rposition(|e| e.kind == 'E' && e.sub == "forget")
                    .filter(|end| *end > forget_start_idx);
                let prune_start_idx = log
                    .iter()
                    .position(|e| e.kind == 'S' && e.sub == "prune")
                    .expect("prune start");
                assert!(
                    forget_end_idx.is_some_and(|end| prune_start_idx > end),
                    "{}: (b) prune starts only after forget ended; calls:\n{}",
                    variant.label,
                    call_log.join("\n")
                );
                rig.daemon.stop();
            }
        },
    );
}

/// A maintenance run records its result for the run detail page: (a) the
/// snapshots its pending `forget` removed per backup, (b) a successful `prune`, (c) a successful
/// `check` and (d) the parsed `check` summary without errors, (e) `GetRunDetail` delivers it to
/// the GUI, and (f) the space prune freed is read from its output (issue-42-AC4: text with
/// restic 0.18.0, JSON from restic 0.20.0; the run removes data, so it is more than zero).
#[test]
fn finding1_maintenance_run_records_removed_snapshots_prune_and_check() {
    run_isolated_test(
        "finding1_maintenance_run_records_removed_snapshots_prune_and_check",
        "KESTIC_FINDING1_MAINT_STATS_INNER",
        |root| {
            let _fakes = StdFakes::start();
            for variant in restic_variants() {
                let rig = M3Rig::start(root, &variant, "F1", FRIDAY, |_| {});
                let client = &rig.daemon.client;
                let alpha = backup_with_open_forget(&rig, root, |b| {
                    b.retention = RetentionPolicy {
                        keep_last: Some(KeepCount::Count(1)),
                        ..RetentionPolicy::empty()
                    };
                });
                let alpha_id = alpha.backup.id.clone();

                rig.clock.set(SUNDAY_1200);
                let done =
                    wait_for_state(client, Duration::from_secs(40), "maintenance done", |s| {
                        maintenance_done(s, &alpha.dest.id)
                    });
                let run = dest_summary(&done, &alpha.dest.id)
                    .last_maintenance_run
                    .clone()
                    .expect("last maintenance run");
                assert_eq!(run.status, RunStatus::Ok, "{}: run ok", variant.label);
                let stats = run
                    .maintenance
                    .clone()
                    .expect("maintenance result recorded");
                assert_eq!(
                    stats.forgets,
                    vec![MaintenanceForget {
                        backup_id: alpha_id,
                        removed_snapshots: Some(1),
                    }],
                    "{}: (a) removed snapshots per backup",
                    variant.label
                );
                assert!(stats.pruned, "{}: (b) prune recorded", variant.label);
                assert!(
                    stats.freed_bytes.is_some_and(|bytes| bytes > 0),
                    "{}: (f) freed space read from the prune output: {:?}",
                    variant.label,
                    stats.freed_bytes
                );
                assert!(stats.checked, "{}: (c) check recorded", variant.label);
                assert_eq!(
                    stats.check.as_ref().map(|c| c.num_errors),
                    Some(0),
                    "{}: (d) check summary without errors",
                    variant.label
                );
                let stored = client
                    .get_run_detail(run.id.as_str())
                    .expect("GetRunDetail")
                    .run;
                assert_eq!(
                    stored.maintenance,
                    Some(stats),
                    "{}: (e) GetRunDetail delivers the result",
                    variant.label
                );
                let prune_args = stored
                    .commands
                    .iter()
                    .find(|c| c.args.iter().any(|a| a == "prune"))
                    .map(|c| c.args.clone())
                    .expect("prune command recorded");
                let json_expected = check_binary_version(&variant.bin)
                    .expect("restic version")
                    .version
                    >= PRUNE_JSON_MIN_VERSION;
                assert_eq!(
                    prune_args.iter().any(|a| a == "--json"),
                    json_expected,
                    "{}: (f) prune output is JSON exactly from restic 0.20.0: {prune_args:?}",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// issue-42-AC2: a `prune` whose JSON result cannot be read (latest restic behind a wrapper that
/// replaces the output with a `summary` without a number) leaves (a) the run `ok`, (b) `prune`
/// recorded and (c) no freed space.
#[test]
fn issue_42_ac2_unreadable_prune_result_keeps_run_ok() {
    run_isolated_test(
        "issue_42_ac2_unreadable_prune_result_keeps_run_ok",
        "KESTIC_ISSUE_42_AC2_INNER",
        |root| {
            let _fakes = StdFakes::start();
            let latest = latest_variant();
            let wrapper_dir = root.join("unreadable-prune");
            fs::create_dir_all(&wrapper_dir).expect("mkdir wrapper dir");
            let wrapper = wrapper_dir.join("restic");
            write_executable(
                &wrapper,
                &format!(
                    r#"#!/bin/sh
real='{real}'
prune=""; dry=""
for a in "$@"; do
  case "$a" in
    prune) prune=1;;
    --dry-run) dry=1;;
  esac
done
if [ -n "$prune" ] && [ -z "$dry" ]; then
  "$real" "$@" >/dev/null || exit $?
  echo '{{"message_type":"summary","bytes":{{"remove_total":"unreadable"}}}}'
  exit 0
fi
exec "$real" "$@"
"#,
                    real = latest.bin.display()
                ),
            );
            let variant = ResticVariant {
                index: latest.index,
                label: "restic-latest, unreadable prune result",
                bin: wrapper,
            };
            let rig = M3Rig::start(root, &variant, "I42", FRIDAY, |_| {});
            let alpha = rig.add_backup("Alpha", |b| b.schedule = Schedule::ManualOnly);
            rig.clock.set(SUNDAY_1200);
            let done = wait_for_state(
                &rig.daemon.client,
                Duration::from_secs(40),
                "maintenance done",
                |s| maintenance_done(s, &alpha.dest.id),
            );
            let run = dest_summary(&done, &alpha.dest.id)
                .last_maintenance_run
                .clone()
                .expect("last maintenance run");
            assert_eq!(run.status, RunStatus::Ok, "(a) run ok");
            let stats = run.maintenance.expect("maintenance result recorded");
            assert!(stats.pruned, "(b) prune recorded");
            assert_eq!(stats.freed_bytes, None, "(c) no freed space");
            rig.daemon.stop();
        },
    );
}

/// AC-M3-05-5 (negative): a due maintenance run is held back (a) on battery and (b-neg) on a
/// metered connection, and (b) starts as soon as AC power and an unmetered connection are present.
#[test]
fn ac_m3_05_5_maintenance_deferred_on_battery_or_metered_connection() {
    run_isolated_test(
        "ac_m3_05_5_maintenance_deferred_on_battery_or_metered_connection",
        "KESTIC_M3_05_5_INNER",
        |root| {
            let fakes = StdFakes::start();
            for variant in restic_variants() {
                fakes.upower.set_on_battery(true, 80.0);
                fakes.nm.update(true, false, Some("Heim"));
                let rig = M3Rig::start(root, &variant, "M3Q5", FRIDAY, |_| {});
                let client = &rig.daemon.client;
                let dest = add_local_dest(&rig, root, "Z");
                let repo_calls_before = rig.wrapper.started_repo_count();

                // On battery: the due maintenance does not start.
                rig.clock.set(SUNDAY_1200);
                settle();
                let on_battery = client.get_state().expect("GetState");
                let d = dest_summary(&on_battery, &dest.id);
                assert!(
                    !d.maintenance_running,
                    "{}: (a) no maintenance on battery: not running",
                    variant.label
                );
                assert!(
                    d.last_prune_at_unix.is_none(),
                    "{}: (a) no maintenance on battery: no prune recorded",
                    variant.label
                );
                assert_eq!(
                    rig.wrapper.started_repo_count(),
                    repo_calls_before,
                    "{}: (a) no restic process on battery: {:?}",
                    variant.label,
                    rig.wrapper.entries()
                );
                assert_eq!(
                    d.maintenance_pending_reason.map(|c| c.as_str()),
                    Some("on_battery"),
                    "{}: (a) maintenance visibly waits (ausstehend) for AC power",
                    variant.label
                );

                // AC power but metered connection: still deferred.
                fakes.upower.set_on_battery(false, 100.0);
                fakes.nm.update(true, true, Some("Heim"));
                settle();
                let metered = client.get_state().expect("GetState");
                let d = dest_summary(&metered, &dest.id);
                assert!(
                    !d.maintenance_running,
                    "{}: (a) no maintenance on a metered connection: not running",
                    variant.label
                );
                assert!(
                    d.last_prune_at_unix.is_none(),
                    "{}: (a) no maintenance on a metered connection: no prune recorded",
                    variant.label
                );
                assert_eq!(
                    rig.wrapper.started_repo_count(),
                    repo_calls_before,
                    "{}: (a) no restic process on a metered connection",
                    variant.label
                );
                assert_eq!(
                    d.maintenance_pending_reason.map(|c| c.as_str()),
                    Some("metered"),
                    "{}: (a) maintenance visibly waits (ausstehend) for an unmetered connection",
                    variant.label
                );

                // AC power and unmetered: starts.
                fakes.nm.update(true, false, Some("Heim"));
                let done = wait_for_state(client, Duration::from_secs(40), "maintenance", |s| {
                    maintenance_done(s, &dest.id)
                });
                assert!(
                    dest_summary(&done, &dest.id).last_prune_at_unix.is_some(),
                    "{}: (b) prune ran after power + unmetered",
                    variant.label
                );
                assert_eq!(
                    dest_summary(&done, &dest.id).maintenance_pending_reason,
                    None,
                    "{}: (b) no waiting reason once maintenance ran",
                    variant.label
                );
                assert!(
                    !rig.wrapper.of('S', "prune").is_empty(),
                    "{}: (b) restic prune started",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-05-6 (FR-11): a due `check` runs (a) without `--read-data` and (b) is recorded as a
/// maintenance run in the destination's history.
#[test]
fn ac_m3_05_6_check_without_read_data_is_recorded_as_maintenance_run() {
    run_isolated_test(
        "ac_m3_05_6_check_without_read_data_is_recorded_as_maintenance_run",
        "KESTIC_M3_05_6_INNER",
        |root| {
            let _fakes = StdFakes::start();
            for variant in restic_variants() {
                let rig = M3Rig::start(root, &variant, "M3Q6", FRIDAY, |_| {});
                let client = &rig.daemon.client;
                let dest = add_local_dest(&rig, root, "Z");

                rig.clock.set(SUNDAY_1200);
                let done = wait_for_state(client, Duration::from_secs(40), "maintenance", |s| {
                    maintenance_done(s, &dest.id)
                });
                let checks = rig.wrapper.of('S', "check");
                assert_eq!(checks.len(), 1, "{}: (a) one check call", variant.label);
                assert!(
                    !checks[0].detail.contains("--read-data"),
                    "{}: (a) check without --read-data: {}",
                    variant.label,
                    checks[0].detail
                );
                let run = dest_summary(&done, &dest.id)
                    .last_maintenance_run
                    .clone()
                    .expect("last maintenance run");
                assert_eq!(run.kind, RunKind::Maintenance, "{}: (b)", variant.label);
                assert_eq!(run.status, RunStatus::Ok, "{}: (b)", variant.label);
                assert_eq!(run.destination_id, dest.id, "{}: (b)", variant.label);
                assert!(
                    done.recent_runs
                        .iter()
                        .any(|r| r.id == run.id && r.kind == RunKind::Maintenance),
                    "{}: (b) maintenance run listed in the history",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-05-8 (NFR-08): with a running backup on the same destination (non-exclusive lock)
/// the `prune` preview (a) returns a result instead of Exit 11 and (b) uses `--no-lock`.
#[test]
fn ac_m3_05_8_prune_preview_under_non_exclusive_lock_uses_no_lock() {
    run_isolated_test(
        "ac_m3_05_8_prune_preview_under_non_exclusive_lock_uses_no_lock",
        "KESTIC_M3_05_8_INNER",
        |root| {
            let _fakes = StdFakes::start();
            for variant in restic_variants() {
                let rig = M3Rig::start(root, &variant, "M3Q8", FRIDAY, |_| {});
                let client = &rig.daemon.client;
                let fx = rig.add_backup("Docs", |_| {});
                client.start_backup(&fx.backup_id_str).expect("seed run");
                wait_for_state(client, Duration::from_secs(30), "seed Ok", |s| {
                    backup_summary(s, &fx.backup.id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });

                let mut cmd = restic_repo_cmd(
                    &variant.bin,
                    &fx.repo_dir,
                    &fx.password,
                    &root.join("restic-cache"),
                );
                cmd.args(["backup", "--stdin", "--stdin-filename", "concurrent.txt"]);
                let mut holder = hold_repo_lock(cmd, &fx.repo_dir);
                let preview = client
                    .preview_destination_maintenance(fx.dest.id.as_str())
                    .expect("(a) the preview returns a result instead of failing with Exit 11");
                assert!(
                    preview.transparent_command.contains("--no-lock"),
                    "{}: (b) preview command uses --no-lock: {}",
                    variant.label,
                    preview.transparent_command
                );
                assert!(
                    preview.transparent_command.contains("--dry-run"),
                    "{}: (b) preview is a dry run: {}",
                    variant.label,
                    preview.transparent_command
                );
                if let Some(mut stdin) = holder.0.stdin.take() {
                    let _ = stdin.write_all(b"ok");
                }
                holder.0.wait().expect("foreign backup finishes");
                rig.daemon.stop();
            }
        },
    );
}
