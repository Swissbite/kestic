// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

mod common;

use common::execute_backup;

use kestic_test_support::tools::find_lock_file;

use kestic_core::backup::{build_backup_command_spec, build_forget_command_spec};
use kestic_core::maintenance::{
    RepoLockInfo, build_check_command_spec, build_prune_command_spec, classify_locks,
    detect_stale_locks, detect_stale_locks_with, execute_check, execute_prune,
    execute_prune_dry_run, execute_unlock, execute_unlock_with,
};
use kestic_core::model::{Backup, Destination, DestinationKind, RunStatus, Source};
use kestic_core::provider::check_binary_version;
use kestic_core::restic::{ResticCommandSpec, ResticRuntime};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use time::macros::datetime;

fn populate_incompressible_blobs(dir: &Path, count_mb: usize) {
    let mut chunk = vec![0u8; 1024 * 1024];
    for (idx, b) in chunk.iter_mut().enumerate() {
        *b = ((idx.wrapping_mul(31) ^ (idx >> 5)) & 0xff) as u8;
    }
    for i in 0..count_mb {
        chunk[0] = i as u8;
        fs::write(dir.join(format!("blob_{i:02}.bin")), &chunk).expect("write blob");
    }
}

#[test]
fn ac_m3_04_9_no_command_spec_contains_retry_lock() {
    let dir = TempDir::new().expect("tempdir");
    let dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: dir.path().join("repo"),
        },
    )
    .expect("dest");
    let backup = Backup::new(
        "Docs",
        Source::folder(dir.path().join("src")),
        dest.id.clone(),
    )
    .expect("backup");
    let (_, restic_bin) = common::restic_test_binaries()
        .into_iter()
        .next()
        .expect("bin");
    let resolved = check_binary_version(&restic_bin).expect("resolved");
    let helper = Path::new("/usr/bin/kestic");
    let state_dir = dir.path().join("state");

    let specs = vec![
        build_backup_command_spec(&backup, &dest, &state_dir, &resolved, helper).expect("bk spec"),
        build_forget_command_spec(&backup, &dest, false, false, &resolved, helper).expect("forget"),
        build_forget_command_spec(&backup, &dest, true, true, &resolved, helper)
            .expect("forget dry"),
        build_prune_command_spec(&dest, false, false, &resolved, helper),
        build_prune_command_spec(&dest, true, true, &resolved, helper),
        build_check_command_spec(&dest, &resolved, helper),
        ResticCommandSpec::unlock(&resolved.binary, helper, &dest),
    ];

    for spec in specs {
        let argv = spec.restic_argv();
        assert!(
            !argv.iter().any(|a| a.contains("--retry-lock")),
            "unexpected --retry-lock in argv: {argv:?}"
        );
    }
}

#[test]
fn ac_m3_05_6_check_runs_without_read_data_and_succeeds() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        fs::write(src_dir.join("hello.txt"), b"hello check").expect("write");

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-12-06 12:00:00 UTC));
        let runtime = rig.runtime();

        let (dest, _repo_pw) = rig.local_repo("Local", &repo_dir);
        let backup = Backup::new("Check", Source::folder(&src_dir), dest.id.clone()).expect("bk");
        let bk_res = execute_backup(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
            .expect("backup");
        assert_eq!(bk_res.exit_code, 0, "{label}");

        let spec = build_check_command_spec(&dest, &rig.resolved, &rig.helper);
        assert!(
            !spec.restic_argv().iter().any(|a| a == "--read-data"),
            "{label}: check must not pass --read-data"
        );

        let check_res = execute_check(&dest, &runtime, None, &[]).expect("check");
        assert_eq!(check_res.exit_code, 0, "{label}: check failed");
        assert_eq!(check_res.presentation.status, RunStatus::Ok, "{label}");
    }
}

#[test]
fn ac_m3_05_7_and_05_8_prune_dry_run_uses_no_lock_during_active_backup() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        fs::write(src_dir.join("data.txt"), b"prune preview data").expect("write");

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-12-06 12:00:00 UTC));
        let runtime = rig.runtime();

        let (dest, repo_pw) = rig.local_repo("Local", &repo_dir);
        let backup = Backup::new("Prune", Source::folder(&src_dir), dest.id.clone()).expect("bk");
        assert_eq!(
            execute_backup(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
                .expect("bk")
                .exit_code,
            0
        );

        // Hold a non-exclusive lock via `restic backup --stdin`.
        let mut stdin_child = Command::new(&rig.resolved.binary)
            .args([
                "-r",
                &repo_dir.to_string_lossy(),
                "backup",
                "--stdin",
                "--stdin-filename",
                "lock-hold.txt",
            ])
            .env("RESTIC_PASSWORD", &repo_pw)
            .env("RESTIC_CACHE_DIR", &cache_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn stdin backup");

        let locks_dir = repo_dir.join("locks");
        let deadline = Instant::now() + Duration::from_secs(10);
        while find_lock_file(&locks_dir).is_none() {
            assert!(Instant::now() < deadline, "{label}: lock never appeared");
            thread::sleep(Duration::from_millis(25));
        }

        // AC-M3-05-8: prune --dry-run --no-lock succeeds while non-exclusive lock is held.
        let preview = execute_prune_dry_run(&dest, &runtime, None, &[])
            .unwrap_or_else(|e| panic!("{label}: prune dry-run failed under lock: {e}"));
        assert!(
            preview.transparent_command.contains("--dry-run"),
            "{label}: missing --dry-run: {}",
            preview.transparent_command
        );
        assert!(
            preview.transparent_command.contains("--no-lock"),
            "{label}: missing --no-lock: {}",
            preview.transparent_command
        );

        // Finish stdin backup cleanly, then run real prune.
        if let Some(mut stdin) = stdin_child.stdin.take() {
            let _ = stdin.write_all(b"done");
        }
        let status = stdin_child.wait().expect("wait stdin backup");
        assert!(status.success(), "{label}");

        let prune_res = execute_prune(&dest, &runtime, None, &[]).expect("real prune");
        assert_eq!(prune_res.exit_code, 0, "{label}");
    }
}

fn wait_for_lock_file(locks_dir: &Path, label: &str, msg: &str) -> std::path::PathBuf {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(lock) = find_lock_file(locks_dir) {
            return lock;
        }
        assert!(Instant::now() < deadline, "{label}: {msg}");
        thread::sleep(Duration::from_millis(25));
    }
}

fn verify_active_local_lock_refusal(
    label: &str,
    dest: &Destination,
    repo_dir: &Path,
    repo_pw: &str,
    cache_dir: &Path,
    runtime: &ResticRuntime<'_>,
) {
    let locks_dir = repo_dir.join("locks");
    let mut active_child = Command::new(&runtime.resolved.binary)
        .args([
            "-r",
            &repo_dir.to_string_lossy(),
            "backup",
            "--stdin",
            "--stdin-filename",
            "active.txt",
        ])
        .env("RESTIC_PASSWORD", repo_pw)
        .env("RESTIC_CACHE_DIR", cache_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn active backup");
    wait_for_lock_file(&locks_dir, label, "active lock never appeared");

    let active_cls = detect_stale_locks(dest, runtime, &[]).expect("active detect");
    assert!(
        !active_cls.has_stale_local_lock,
        "{label}: live process lock must not be stale"
    );
    assert_eq!(active_cls.active_local_locks.len(), 1, "{label}");
    assert!(
        execute_unlock(dest, runtime, &[]).is_err(),
        "{label}: active local lock unlock must be refused"
    );

    if let Some(mut stdin) = active_child.stdin.take() {
        let _ = stdin.write_all(b"done");
    }
    let _ = active_child.wait();
}

#[test]
fn ac_m3_06_1_to_06_3_stale_lock_detection_sigterm_active_and_foreign_host() {
    // Unit check for foreign hostname (`AC-M3-06-3`).
    let foreign = RepoLockInfo {
        lock_id: "a".repeat(64),
        time: None,
        exclusive: true,
        hostname: "other-laptop".to_string(),
        username: Some("alice".to_string()),
        pid: Some(999_999),
    };
    let cls_unit = classify_locks(&[foreign], "this-laptop", |_| false);
    assert!(!cls_unit.has_stale_local_lock);
    assert_eq!(cls_unit.foreign_host_locks.len(), 1);

    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        populate_incompressible_blobs(&src_dir, 24);

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-12-06 12:00:00 UTC));
        let runtime = rig.runtime();

        let (dest, repo_pw) = rig.local_repo("Local", &repo_dir);
        let backup = Backup::new("Lock", Source::folder(&src_dir), dest.id.clone()).expect("bk");
        assert_eq!(
            execute_backup(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
                .expect("init bk")
                .exit_code,
            0
        );

        let locks_dir = repo_dir.join("locks");

        // 1. SIGTERM on exclusive `restic check --read-data` cleans up its lock -> no unlock offer (`AC-M3-06-1`).
        let mut sigterm_child = Command::new(&rig.resolved.binary)
            .args(["-r", &repo_dir.to_string_lossy(), "check", "--read-data"])
            .env("RESTIC_PASSWORD", &repo_pw)
            .env("RESTIC_CACHE_DIR", &cache_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn check");
        wait_for_lock_file(&locks_dir, label, "lock never appeared");
        let _ = Command::new("kill")
            .args(["-TERM", &sigterm_child.id().to_string()])
            .status();
        let _ = sigterm_child.wait();
        let after_term = detect_stale_locks(&dest, &runtime, &[]).expect("inspect after term");
        assert!(
            !after_term.has_stale_local_lock,
            "{label}: SIGTERM should leave no stale lock"
        );

        // 2. SIGKILL on exclusive `restic check --read-data` leaves stale local lock (`AC-M3-06-1`).
        let mut sigkill_child = Command::new(&rig.resolved.binary)
            .args(["-r", &repo_dir.to_string_lossy(), "check", "--read-data"])
            .env("RESTIC_PASSWORD", &repo_pw)
            .env("RESTIC_CACHE_DIR", &cache_dir)
            .env("RESTIC_HOST", "ignored-by-restic-lock")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn check for kill");
        wait_for_lock_file(&locks_dir, label, "lock never appeared");
        let _ = sigkill_child.kill();
        let _ = sigkill_child.wait();

        // Backup hits Exit 11.
        let blocked_bk = execute_backup(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
            .expect("blocked bk");
        assert_eq!(blocked_bk.exit_code, 11, "{label}");

        // Same host + dead PID -> stale lock detected (`AC-M3-06-1`).
        let stale = detect_stale_locks(&dest, &runtime, &[]).expect("detect stale");
        assert!(stale.has_stale_local_lock, "{label}: expected stale lock");

        // Foreign host check against the same real lock -> NOT offered AND execute_unlock_with refuses leaving lock intact (`AC-M3-06-3`).
        let foreign_check =
            detect_stale_locks_with(&dest, &runtime, "completely-different-host", |_| false, &[])
                .expect("foreign check");
        assert!(
            !foreign_check.has_stale_local_lock,
            "{label}: foreign lock must not offer unlock"
        );
        assert_eq!(foreign_check.foreign_host_locks.len(), 1, "{label}");
        let lock_file_before_refusal =
            find_lock_file(&locks_dir).expect("lock file present before foreign unlock refusal");
        assert!(
            execute_unlock_with(&dest, &runtime, "completely-different-host", |_| false, &[])
                .is_err(),
            "{label}: foreign lock unlock must be refused"
        );
        assert!(
            lock_file_before_refusal.exists(),
            "{label}: foreign lock file must remain untouched"
        );

        // Unlock clears the stale lock and backup succeeds (`AC-M3-06-1`).
        let unlock_res = execute_unlock(&dest, &runtime, &[]).expect("unlock");
        assert_eq!(unlock_res.exit_code, 0, "{label}");
        let ok_bk = execute_backup(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
            .expect("post unlock bk");
        assert_eq!(ok_bk.exit_code, 0, "{label}");

        // 3. Active local `restic backup --stdin` lock -> NOT stale AND unlock refused (`AC-M3-06-2`).
        verify_active_local_lock_refusal(label, &dest, &repo_dir, &repo_pw, &cache_dir, &runtime);
    }
}
