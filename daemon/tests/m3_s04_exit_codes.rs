// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S04 restic exit codes: lock (Exit 11) as transient failure with backoff, `forget` Exit 11 as
//! warning, and wrong password (Exit 12) as permanent error (`AC-M3-04-4`, `-5`, `-8`).

mod common;

use common::m3::{
    LocalBackupFixture, M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, StdFakes,
    add_local_backup, backup_summary, dest_summary, hold_repo_lock, notifications_for,
    restic_repo_cmd, restic_variants, runtime_state, send_signal, settle, spawn_m3_daemon,
    wait_for_backoff_attempt, wait_for_notifications, wait_for_state, write_incompressible_dataset,
};
use common::run_isolated_test;
use kestic_core::clock::FakeClock;
use kestic_core::model::RunStatus;
use kestic_core::restic::SECRET_KIND_REPO;
use kestic_core::secrets::{SecretStore, SecretValue};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use time::macros::datetime;

/// 2026-12-01 11:59:30 Europe/Zurich.
const BEFORE_SLOT: OffsetDateTime = datetime!(2026-12-01 10:59:30 UTC);
/// 2026-12-01 12:00:05 Europe/Zurich.
const SLOT_1: OffsetDateTime = datetime!(2026-12-01 11:00:05 UTC);

struct LockRig {
    daemon: M3Daemon,
    clock: Arc<FakeClock>,
    wrapper: ResticWrapper,
    fixture: LocalBackupFixture,
    variant: ResticVariant,
    root: std::path::PathBuf,
}

fn start_lock_rig(root: &Path, variant: &ResticVariant, base: &str, mib: usize) -> LockRig {
    let sub = variant.sub(base);
    let wrapper = ResticWrapper::create(&root.join(&sub).join("wrapper"), &variant.bin);
    let clock = Arc::new(FakeClock::new(BEFORE_SLOT));
    let bus = variant.bus_name(base);
    let daemon = spawn_m3_daemon(
        root,
        M3DaemonOpts::new(&sub, &bus, &wrapper.path, Arc::clone(&clock) as _),
    );
    let fixture = add_local_backup(&daemon.client, root, &sub, "Docs", |_| {});
    write_incompressible_dataset(&fixture.src_dir, mib, 1024 * 1024);
    LockRig {
        daemon,
        clock,
        wrapper,
        fixture,
        variant: variant.clone(),
        root: root.to_path_buf(),
    }
}

impl LockRig {
    /// Seeds a first snapshot with a manual run (restic `backup` exit 0, `forget` exit 0).
    fn seed_snapshot(&self) {
        self.daemon
            .client
            .start_backup(&self.fixture.backup_id_str)
            .expect("seed run");
        wait_for_state(
            &self.daemon.client,
            Duration::from_secs(30),
            "seed Ok",
            |s| {
                backup_summary(s, &self.fixture.backup.id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            },
        );
    }

    fn password(&self) -> String {
        format!("pw-{}", self.fixture.backup.name)
    }

    fn restic_cmd(&self) -> Command {
        restic_repo_cmd(
            &self.variant.bin,
            &self.fixture.repo_dir,
            &self.password(),
            &self.root.join("restic-cache"),
        )
    }
}

/// AC-M3-04-4: another process holds the exclusive lock (running `restic check`); the
/// scheduled `backup` ends with Exit 11 -> (a) `ausstehend` (backoff after failure), (b) retries
/// after 5/15/60 min counted from the end of the last attempt, (c) no immediate notification.
#[test]
fn ac_m3_04_4_exclusive_lock_exit_11_backoff_5_15_60_without_notification() {
    run_isolated_test(
        "ac_m3_04_4_exclusive_lock_exit_11_backoff_5_15_60_without_notification",
        "KESTIC_M3_04_4_INNER",
        |root| {
            let fakes = StdFakes::start();
            for variant in restic_variants() {
                let rig = start_lock_rig(root, &variant, "M3E4", 16);
                let client = &rig.daemon.client;
                let bid = &rig.fixture.backup.id;
                rig.seed_snapshot();
                fakes.notif.clear();

                // Exclusive lock: `restic check --read-data` frozen with SIGSTOP while it holds it.
                let mut cmd = rig.restic_cmd();
                cmd.args(["check", "--read-data"]);
                let mut check = hold_repo_lock(cmd, &rig.fixture.repo_dir);
                send_signal(check.0.id(), "STOP");
                assert!(
                    check.0.try_wait().expect("try_wait").is_none(),
                    "{}: lock holder must still be alive",
                    variant.label
                );
                let backups_before = rig.wrapper.of('E', "backup").len();

                // First attempt at the 12:00 slot.
                rig.clock.set(SLOT_1);
                let rs1 = wait_for_backoff_attempt(
                    &rig.daemon.state_dir,
                    bid,
                    1,
                    Duration::from_secs(15),
                );
                let state = client.get_state().expect("GetState");
                // (a) `ausstehend` with the backoff reason, the backup ended with Exit 11.
                assert_eq!(
                    backup_summary(&state, bid)
                        .pending_primary_code
                        .map(|c| c.as_str()),
                    Some("backoff"),
                    "{}: (a)",
                    variant.label
                );
                let exits: Vec<String> = rig
                    .wrapper
                    .of('E', "backup")
                    .into_iter()
                    .skip(backups_before)
                    .map(|e| e.detail)
                    .collect();
                assert_eq!(
                    exits,
                    vec!["11".to_string()],
                    "{}: (a) backup Exit 11",
                    variant.label
                );
                // (b) retry after 5 min measured from the end of the attempt (clock 12:00:05).
                assert_eq!(
                    rs1.backoff_until,
                    Some(datetime!(2026-12-01 11:05:05 UTC)),
                    "{}: (b) first retry +5 min",
                    variant.label
                );

                rig.clock.set(datetime!(2026-12-01 11:05:05 UTC));
                let rs2 = wait_for_backoff_attempt(
                    &rig.daemon.state_dir,
                    bid,
                    2,
                    Duration::from_secs(15),
                );
                assert_eq!(
                    rs2.backoff_until,
                    Some(datetime!(2026-12-01 11:20:05 UTC)),
                    "{}: (b) second retry +15 min",
                    variant.label
                );

                rig.clock.set(datetime!(2026-12-01 11:20:05 UTC));
                let rs3 = wait_for_backoff_attempt(
                    &rig.daemon.state_dir,
                    bid,
                    3,
                    Duration::from_secs(15),
                );
                assert_eq!(
                    rs3.backoff_until,
                    Some(datetime!(2026-12-01 12:20:05 UTC)),
                    "{}: (b) third retry +60 min",
                    variant.label
                );

                rig.clock.set(datetime!(2026-12-01 12:20:05 UTC));
                let rs4 = wait_for_backoff_attempt(
                    &rig.daemon.state_dir,
                    bid,
                    4,
                    Duration::from_secs(15),
                );
                assert_eq!(
                    rs4.backoff_until, None,
                    "{}: (b) retries exhausted after 5/15/60",
                    variant.label
                );
                let exit_codes: Vec<String> = rig
                    .wrapper
                    .of('E', "backup")
                    .into_iter()
                    .skip(backups_before)
                    .map(|e| e.detail)
                    .collect();
                assert_eq!(
                    exit_codes,
                    vec!["11"; 4],
                    "{}: (b) one initial attempt plus three retries, all Exit 11",
                    variant.label
                );

                // (c) no immediate failure notification during the whole series.
                settle();
                assert_eq!(
                    notifications_for(&fakes.notif, "BackupFailed").len(),
                    0,
                    "{}: (c) no BackupFailed notification: {:?}",
                    variant.label,
                    fakes.notif.snapshot()
                );
                drop(check);
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-04-5 (negative): wrong repository password -> Exit 12 -> (a) `Fehler`, (b) immediate
/// notification, (c) no retry.
#[test]
fn ac_m3_04_5_wrong_password_exit_12_is_permanent_error_without_retry() {
    run_isolated_test(
        "ac_m3_04_5_wrong_password_exit_12_is_permanent_error_without_retry",
        "KESTIC_M3_04_5_INNER",
        |root| {
            let fakes = StdFakes::start();
            for variant in restic_variants() {
                let rig = start_lock_rig(root, &variant, "M3E5", 1);
                let client = &rig.daemon.client;
                let bid = &rig.fixture.backup.id;
                common::isolated_secret_store()
                    .set_secret(
                        rig.fixture.dest.id.as_str(),
                        SECRET_KIND_REPO,
                        &SecretValue::from_str_secret("completely-wrong-password"),
                    )
                    .expect("overwrite stored password");
                fakes.notif.clear();

                rig.clock.set(SLOT_1);
                let state = wait_for_state(client, Duration::from_secs(20), "Exit 12", |s| {
                    backup_summary(s, bid).has_error
                });
                // (a) `Fehler` with Exit 12.
                let last = backup_summary(&state, bid)
                    .last_run
                    .clone()
                    .expect("last run");
                assert_eq!(last.status, RunStatus::Error, "{}: (a)", variant.label);
                assert_eq!(last.exit_code, Some(12), "{}: (a)", variant.label);
                assert!(
                    dest_summary(&state, &rig.fixture.dest.id).password_rejected,
                    "{}: (a) password marked as rejected",
                    variant.label
                );
                // (b) immediate notification.
                let failed =
                    wait_for_notifications(&fakes.notif, "BackupFailed", 1, Duration::from_secs(5));
                assert_eq!(
                    failed.len(),
                    1,
                    "{}: (b) exactly one BackupFailed",
                    variant.label
                );

                // (c) no retry: jump past every backoff window (5/15/60 min) without a new slot.
                rig.clock.set(datetime!(2026-12-01 12:30:00 UTC));
                settle();
                assert_eq!(
                    rig.wrapper.of('S', "backup").len(),
                    1,
                    "{}: (c) backup was never retried",
                    variant.label
                );
                let rs = runtime_state(&rig.daemon.state_dir, bid);
                assert_eq!(
                    rs.backoff_attempt, 0,
                    "{}: (c) no backoff scheduled",
                    variant.label
                );
                assert_eq!(rs.backoff_until, None, "{}: (c)", variant.label);
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-04-8: a running `restic backup --stdin` holds a non-exclusive lock ->
/// (a) `backup` Exit 0, (b) `forget` Exit 11, (c) run `Warnung` with "forget offen", (d) no backoff.
#[test]
fn ac_m3_04_8_forget_exit_11_under_non_exclusive_lock_is_warning_without_backoff() {
    run_isolated_test(
        "ac_m3_04_8_forget_exit_11_under_non_exclusive_lock_is_warning_without_backoff",
        "KESTIC_M3_04_8_INNER",
        |root| {
            let _fakes = StdFakes::start();
            for variant in restic_variants() {
                let rig = start_lock_rig(root, &variant, "M3E8", 1);
                let client = &rig.daemon.client;
                let bid = &rig.fixture.backup.id;
                rig.seed_snapshot();
                let backups_before = rig.wrapper.of('E', "backup").len();
                let forgets_before = rig.wrapper.of('E', "forget").len();

                let mut cmd = rig.restic_cmd();
                cmd.args(["backup", "--stdin", "--stdin-filename", "hold.txt"]);
                let mut holder = hold_repo_lock(cmd, &rig.fixture.repo_dir);
                std::thread::sleep(Duration::from_millis(30));

                client
                    .start_backup(&rig.fixture.backup_id_str)
                    .expect("start under non-exclusive lock");
                let state = wait_for_state(client, Duration::from_secs(30), "Warning", |s| {
                    backup_summary(s, bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Warning)
                });
                let backup_exits: Vec<String> = rig
                    .wrapper
                    .of('E', "backup")
                    .into_iter()
                    .skip(backups_before)
                    .map(|e| e.detail)
                    .collect();
                let forget_exits: Vec<String> = rig
                    .wrapper
                    .of('E', "forget")
                    .into_iter()
                    .skip(forgets_before)
                    .map(|e| e.detail)
                    .collect();
                assert_eq!(
                    backup_exits,
                    vec!["0".to_string()],
                    "{}: (a)",
                    variant.label
                );
                assert_eq!(
                    forget_exits,
                    vec!["11".to_string()],
                    "{}: (b)",
                    variant.label
                );
                let summary = backup_summary(&state, bid);
                assert!(
                    summary.pending_forget,
                    "{}: (c) forget is marked open",
                    variant.label
                );
                let message = summary
                    .last_run
                    .as_ref()
                    .and_then(|r| r.summary_message.clone())
                    .unwrap_or_default();
                assert!(
                    message.contains("retention runs later"),
                    "{}: (c) warning says the retention (forget) is still open: {message:?}",
                    variant.label
                );
                let rs = runtime_state(&rig.daemon.state_dir, bid);
                assert_eq!(
                    rs.backoff_attempt, 0,
                    "{}: (d) no backoff attempt",
                    variant.label
                );
                assert_eq!(rs.backoff_until, None, "{}: (d) no backoff", variant.label);
                assert_eq!(
                    summary.pending_primary_code, None,
                    "{}: (d) not pending after the warning",
                    variant.label
                );
                assert_eq!(
                    dest_summary(&state, &rig.fixture.dest.id).stale_lock_count,
                    0,
                    "{}: live lock is not stale",
                    variant.label
                );

                if let Some(mut stdin) = holder.0.stdin.take() {
                    let _ = stdin.write_all(b"done");
                }
                drop(holder);
                rig.daemon.stop();
            }
        },
    );
}
