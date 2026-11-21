// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::execute_backup;

use kestic_core::backup::{
    build_backup_command_spec, exclude_file_path, list_snapshots_for_backup,
};
use kestic_core::model::{Backup, Destination, ExcludeRules, RunStatus, Source};
use kestic_core::restic::{CancelHandle, ResticRuntime};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use time::macros::datetime;

fn verify_snapshots_ls_and_transparent_shell(
    label: &str,
    backup_a: &Backup,
    backup_b: &Backup,
    dest: &Destination,
    repo_dir: &Path,
    state_dir: &Path,
    runtime: &ResticRuntime<'_>,
) {
    let cache_dir = runtime.cache_dir.expect("cache dir");
    let cache_str = cache_dir.to_string_lossy().into_owned();
    let snaps_a = list_snapshots_for_backup(
        &backup_a.id,
        dest,
        runtime.resolved,
        runtime.kestic_binary,
        runtime.secret_store,
        runtime.clock,
        &[("RESTIC_CACHE_DIR", cache_str.as_str())],
    )
    .expect("list snaps a");
    let snaps_b = list_snapshots_for_backup(
        &backup_b.id,
        dest,
        runtime.resolved,
        runtime.kestic_binary,
        runtime.secret_store,
        runtime.clock,
        &[("RESTIC_CACHE_DIR", cache_str.as_str())],
    )
    .expect("list snaps b");

    assert_eq!(snaps_a.len(), 2, "{label}: backup A snapshots");
    assert!(
        snaps_a
            .iter()
            .all(|s| s.tags.iter().any(|t| t == &backup_a.tag()))
    );
    assert_eq!(snaps_b.len(), 1, "{label}: backup B snapshots");
    assert!(
        snaps_b
            .iter()
            .all(|s| s.tags.iter().any(|t| t == &backup_b.tag()))
    );

    // AC-M1-07-3: Verify `skip.tmp` is absent from snapshot A via `restic ls`.
    let snap_id = snaps_a[0].short_id.as_deref().unwrap_or(&snaps_a[0].id);
    let ls_out = Command::new(&runtime.resolved.binary)
        .args([
            "-r",
            &repo_dir.to_string_lossy(),
            "ls",
            "--no-lock",
            snap_id,
        ])
        .env("RESTIC_PASSWORD", "backup-test-pw")
        .env("RESTIC_CACHE_DIR", cache_dir)
        .output()
        .expect("restic ls");
    assert!(ls_out.status.success());
    let listing = String::from_utf8_lossy(&ls_out.stdout);
    assert!(listing.contains("keep.txt"), "{label}: listing:\n{listing}");
    assert!(
        !listing.contains("skip.tmp"),
        "{label}: excluded file present in snapshot:\n{listing}"
    );

    // AC-M1-14-3: Transparent shell command runs directly in `sh` with same exit 0.
    let cmd_spec = build_backup_command_spec(
        backup_a,
        dest,
        state_dir,
        runtime.resolved,
        runtime.kestic_binary,
    )
    .expect("build spec");
    let mut sh_spec = cmd_spec.clone();
    sh_spec.password_command_override = Some(format!(
        "{} secret get {}",
        runtime.kestic_binary.display(),
        dest.id
    ));
    let shell_line = sh_spec.to_transparent_shell_command();
    let sh_out = Command::new("sh")
        .args(["-c", &shell_line])
        .env("RESTIC_CACHE_DIR", cache_dir)
        .output()
        .expect("run transparent shell cmd");
    assert!(
        sh_out.status.success(),
        "{label}: transparent shell command failed: {shell_line}\nstderr: {}",
        String::from_utf8_lossy(&sh_out.stderr)
    );
}

#[test]
fn ac_m1_07_1_to_07_4_backup_tags_group_by_excludes_and_isolation() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        let src_a = dir.path().join("src-a");
        let src_b = dir.path().join("src-b");
        fs::create_dir_all(&src_a).expect("mkdir src_a");
        fs::create_dir_all(&src_b).expect("mkdir src_b");

        fs::write(src_a.join("keep.txt"), b"hello kestic").expect("write keep");
        fs::write(src_a.join("skip.tmp"), b"ignored content").expect("write skip");
        fs::write(src_b.join("other.txt"), b"backup b file").expect("write other");

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-12-01 11:00:00 UTC));
        let runtime = rig.runtime();

        let dest = rig.init_local("Local", &repo_dir, "backup-test-pw");

        let mut backup_a =
            Backup::new("Docs A", Source::folder(&src_a), dest.id.clone()).expect("backup a");
        backup_a.excludes = ExcludeRules::from_text("*.tmp");

        let backup_b =
            Backup::new("Docs B", Source::folder(&src_b), dest.id.clone()).expect("backup b");

        // Run backup A with host1 (`AC-M1-07-1`, `AC-M1-07-3`).
        let res_a1 = execute_backup(
            &backup_a,
            &dest,
            &state_dir,
            &runtime,
            None,
            &[("RESTIC_HOST", "host-one")],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: backup A1 failed: {e}"));
        assert_eq!(res_a1.exit_code, 0, "{label}");
        assert_eq!(res_a1.presentation.status, RunStatus::Ok);

        // AC-M1-07-3: Exclude file persisted at `$XDG_STATE_HOME/kestic/excludes/<id>.txt`.
        let excl_path = exclude_file_path(&state_dir, &backup_a.id);
        assert!(excl_path.is_file());
        assert_eq!(
            fs::read_to_string(&excl_path).expect("read excl"),
            "*.tmp\n"
        );

        // Second run of backup A under DIFFERENT `RESTIC_HOST` (`AC-M1-07-2`: `--group-by tags`).
        fs::write(src_a.join("keep2.txt"), b"second snapshot").expect("write keep2");
        let res_a2 = execute_backup(
            &backup_a,
            &dest,
            &state_dir,
            &runtime,
            None,
            &[("RESTIC_HOST", "host-two")],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: backup A2 failed: {e}"));
        assert_eq!(res_a2.exit_code, 0, "{label}");
        let joined_logs = res_a2.log_lines.join("\n");
        assert!(
            !joined_logs.contains("no parent snapshot found"),
            "{label}: `--group-by tags` must find parent across host change:\n{joined_logs}"
        );

        // Run backup B once.
        let res_b = execute_backup(
            &backup_b,
            &dest,
            &state_dir,
            &runtime,
            None,
            &[("RESTIC_HOST", "host-one")],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: backup B failed: {e}"));
        assert_eq!(res_b.exit_code, 0, "{label}");

        verify_snapshots_ls_and_transparent_shell(
            label, &backup_a, &backup_b, &dest, &repo_dir, &state_dir, &runtime,
        );
    }
}

#[test]
fn ac_m1_10_1_and_10_3_cancel_sends_sigterm_and_releases_restic_lock() {
    let (_label, restic_bin) = common::restic_test_binaries()
        .into_iter()
        .next()
        .expect("restic binary");

    let dir = TempDir::new().expect("tempdir");
    let cache_dir = common::restic_cache_dir(dir.path());
    let repo_dir = dir.path().join("repo");
    let state_dir = dir.path().join("state");
    let src_dir = dir.path().join("large-src");
    fs::create_dir_all(&src_dir).expect("mkdir src");

    // Create enough incompressible random-ish data (40 MB) or many files so backup runs > 0.3s cleanly.
    let mut chunk = vec![0u8; 1024 * 1024];
    for (idx, b) in chunk.iter_mut().enumerate() {
        *b = ((idx.wrapping_mul(31) ^ (idx >> 5)) & 0xff) as u8;
    }
    for i in 0..48 {
        chunk[0] = i as u8;
        fs::write(src_dir.join(format!("blob_{i:02}.bin")), &chunk).expect("write blob");
    }

    let rig = common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-12-01 11:30:00 UTC));
    let runtime = rig.runtime();

    let dest = rig.init_local("Local", &repo_dir, "cancel-test-pw");

    let backup = Backup::new("Cancel Me", Source::folder(&src_dir), dest.id.clone()).expect("bk");
    let cancel = Arc::new(CancelHandle::new());
    let cancel_trigger = Arc::clone(&cancel);

    let trigger_handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(120));
        cancel_trigger.cancel();
    });

    let res = execute_backup(
        &backup,
        &dest,
        &state_dir,
        &runtime,
        Some(&cancel),
        &[],
        |_| {},
    )
    .expect("execute backup");
    trigger_handle.join().expect("join cancel thread");

    assert!(res.cancelled, "expected cancelled=true");
    assert_eq!(res.presentation.status, RunStatus::Cancelled);

    // AC-M1-10-3: No stale restic locks left behind after SIGTERM.
    let locks_out = Command::new(&rig.resolved.binary)
        .args([
            "-r",
            &repo_dir.to_string_lossy(),
            "list",
            "locks",
            "--no-lock",
        ])
        .env("RESTIC_PASSWORD", "cancel-test-pw")
        .env("RESTIC_CACHE_DIR", &cache_dir)
        .output()
        .expect("list locks");
    assert!(locks_out.status.success());
    let locks = String::from_utf8_lossy(&locks_out.stdout);
    assert!(
        locks.trim().is_empty(),
        "expected 0 locks after SIGTERM cancel, found:\n{locks}"
    );
}
