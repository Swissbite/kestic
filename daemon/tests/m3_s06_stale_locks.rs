// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S06 stale lock detection and `unlock` offer through the daemon (`AC-M3-06-1`, `AC-M3-06-2`).
//! The foreign-host case (`AC-M3-06-3`) is covered against a real repository in
//! `core/tests/m3_s04_s05_s06_restic.rs`.

mod common;

use common::revision::UpdateNow;

use common::m3::{
    LocalBackupFixture, M3Rig, ResticVariant, StdFakes, backup_summary, dest_summary,
    hold_repo_lock, restic_repo_cmd, restic_variants, send_signal, wait_for_state,
    write_incompressible_dataset,
};
use common::run_isolated_test;
use kestic_core::model::{KeepCount, RetentionPolicy, RunStatus};
use kestic_test_support::tools::find_lock_file;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use time::macros::datetime;

fn start_rig(root: &Path, variant: &ResticVariant, base: &str) -> (M3Rig, LocalBackupFixture) {
    let rig = M3Rig::start(
        root,
        variant,
        base,
        datetime!(2026-12-01 10:59:30 UTC),
        |_| {},
    );
    let fixture = rig.add_backup("Docs", |_| {});
    write_incompressible_dataset(&fixture.src_dir, 16, 1024 * 1024);
    (rig, fixture)
}

/// Starts a manual run and waits for a *new* run that ends `Ok`.
fn manual_run_ok(rig: &M3Rig, fixture: &LocalBackupFixture) {
    let bid = &fixture.backup.id;
    let previous = backup_summary(&rig.daemon.client.get_state().expect("GetState"), bid)
        .last_run
        .as_ref()
        .map(|r| r.id.clone());
    rig.daemon
        .client
        .start_backup(&fixture.backup_id_str)
        .expect("manual run");
    wait_for_state(
        &rig.daemon.client,
        Duration::from_secs(30),
        "manual Ok",
        |s| {
            let b = backup_summary(s, bid);
            b.active_run.is_none()
                && b.last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok && Some(&r.id) != previous.as_ref())
        },
    );
}

fn check_cmd(rig: &M3Rig, fixture: &LocalBackupFixture, extra: &[&str]) -> std::process::Command {
    let mut cmd = restic_repo_cmd(
        &rig.variant.bin,
        &fixture.repo_dir,
        &fixture.password,
        &Path::new(&std::env::var("KESTIC_TEST_ROOT").expect("root")).join("restic-cache"),
    );
    cmd.args(extra);
    cmd
}

/// AC-M3-06-1 (FR-19): a stale lock of the own host (SIGKILL of an exclusive `restic check`)
/// (a) makes `backup` end with Exit 11, (b) is recognised and `unlock` is offered, (c) the run
/// proceeds after unlocking; (d) a SIGTERMed check leaves no lock and triggers no offer.
#[test]
fn ac_m3_06_1_stale_own_host_lock_is_recognised_and_unlock_offered() {
    run_isolated_test(
        "ac_m3_06_1_stale_own_host_lock_is_recognised_and_unlock_offered",
        "KESTIC_M3_06_1_INNER",
        |root| {
            let _fakes = StdFakes::start();
            for variant in restic_variants() {
                let (rig, fx) = start_rig(root, &variant, "M3L1");
                let client = &rig.daemon.client;
                let bid = &fx.backup.id;
                let dest_id = &fx.dest.id;
                let locks = fx.repo_dir.join("locks");
                manual_run_ok(&rig, &fx);

                // (d) SIGTERM: restic releases its exclusive lock, no stale lock, no offer.
                let mut term = hold_repo_lock(
                    check_cmd(&rig, &fx, &["check", "--read-data"]),
                    &fx.repo_dir,
                );
                send_signal(term.id(), "TERM");
                term.wait().expect("check ends after SIGTERM");
                assert!(
                    find_lock_file(&locks).is_none(),
                    "{}: (d) SIGTERM leaves no lock behind",
                    variant.label
                );
                manual_run_ok(&rig, &fx);
                let after_term = client.get_state().expect("GetState");
                assert_eq!(
                    dest_summary(&after_term, dest_id).stale_lock_count,
                    0,
                    "{}: (d) no unlock offer after SIGTERM",
                    variant.label
                );

                // SIGKILL of an exclusive check leaves a stale lock of this host.
                let mut killed = hold_repo_lock(
                    check_cmd(&rig, &fx, &["check", "--read-data"]),
                    &fx.repo_dir,
                );
                killed.kill().expect("SIGKILL check");
                killed.wait().expect("reap check");
                assert!(
                    find_lock_file(&locks).is_some(),
                    "{}: stale exclusive lock remains",
                    variant.label
                );

                // Scheduled run at 12:00 hits the lock.
                let exits_before = rig.wrapper.of('E', "backup").len();
                rig.clock.set(datetime!(2026-12-01 11:00:05 UTC));
                let state =
                    wait_for_state(client, Duration::from_secs(20), "stale lock offered", |s| {
                        dest_summary(s, dest_id).has_stale_lock
                    });
                let exits: Vec<String> = rig
                    .wrapper
                    .of('E', "backup")
                    .into_iter()
                    .skip(exits_before)
                    .map(|e| e.detail)
                    .collect();
                assert_eq!(
                    exits,
                    vec!["11".to_string()],
                    "{}: (a) backup Exit 11",
                    variant.label
                );
                assert!(
                    dest_summary(&state, dest_id).stale_lock_count >= 1,
                    "{}: (b) stale lock recognised, unlock offered",
                    variant.label
                );

                // (c) unlock, then the run succeeds.
                let removed = client
                    .unlock_destination(dest_id.as_str())
                    .expect("UnlockDestination");
                assert!(removed >= 1, "{}: (c) a lock was removed", variant.label);
                rig.clock.set(datetime!(2026-12-01 11:05:05 UTC));
                wait_for_state(client, Duration::from_secs(20), "run after unlock", |s| {
                    let b = backup_summary(s, bid);
                    b.pending_primary_code.is_none()
                        && b.last_run.as_ref().is_some_and(|r| {
                            r.status == RunStatus::Ok
                                && r.started_at >= datetime!(2026-12-01 11:05:05 UTC)
                        })
                });
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-06-2 (FR-19, negative): the lock of a running backup (other process alive) is not
/// stale: (a) no unlock offer and an explicit `UnlockDestination` is refused.
#[test]
fn ac_m3_06_2_lock_of_running_backup_is_not_stale_and_not_unlocked() {
    run_isolated_test(
        "ac_m3_06_2_lock_of_running_backup_is_not_stale_and_not_unlocked",
        "KESTIC_M3_06_2_INNER",
        |root| {
            let _fakes = StdFakes::start();
            for variant in restic_variants() {
                let (rig, fx) = start_rig(root, &variant, "M3L2");
                let client = &rig.daemon.client;
                let dest_id = &fx.dest.id;
                // Keeping only the last snapshot gives the retention one to remove, so its
                // `forget <id>` needs the exclusive lock the foreign backup holds.
                let mut backup = fx.backup.clone();
                backup.retention = RetentionPolicy {
                    keep_last: Some(KeepCount::Count(1)),
                    ..RetentionPolicy::empty()
                };
                client
                    .update_backup_now(&backup)
                    .expect("keep the last snapshot");
                manual_run_ok(&rig, &fx);

                let cmd = check_cmd(
                    &rig,
                    &fx,
                    &["backup", "--stdin", "--stdin-filename", "hold.txt"],
                );
                let mut holder = hold_repo_lock(cmd, &fx.repo_dir);
                std::thread::sleep(Duration::from_millis(30));
                client
                    .start_backup(&fx.backup_id_str)
                    .expect("run under the foreign backup lock");
                let state = wait_for_state(client, Duration::from_secs(30), "warning", |s| {
                    backup_summary(s, &fx.backup.id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Warning)
                });
                assert!(
                    !dest_summary(&state, dest_id).has_stale_lock,
                    "{}: (a) live lock is not offered for unlock",
                    variant.label
                );
                assert_eq!(
                    dest_summary(&state, dest_id).stale_lock_count,
                    0,
                    "{}: (a)",
                    variant.label
                );
                assert!(
                    client.unlock_destination(dest_id.as_str()).is_err(),
                    "{}: (a) unlock of a live lock is refused",
                    variant.label
                );
                assert!(
                    find_lock_file(&fx.repo_dir.join("locks")).is_some(),
                    "{}: (a) live lock untouched",
                    variant.label
                );
                if let Some(mut stdin) = holder.stdin.take() {
                    let _ = stdin.write_all(b"done");
                }
                holder.wait().expect("foreign backup finishes");
                rig.daemon.stop();
            }
        },
    );
}
