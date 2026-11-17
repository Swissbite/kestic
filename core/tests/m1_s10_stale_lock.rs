// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

mod common;

use kestic_core::backup::execute_backup;
use kestic_core::clock::FakeClock;
use kestic_core::destination::{DestinationSetupMode, setup_destination};
use kestic_core::model::{Backup, Destination, DestinationKind, RunStatus, Source};
use kestic_core::provider::check_binary_version;
use kestic_core::restic::{
    RecoveryAction, ResticCommandSpec, ResticFailureKind, ResticRuntime, run_restic,
};
use kestic_core::secrets::{FileSecretStore, SecretValue};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use time::macros::datetime;

fn find_lock_file(locks_dir: &Path) -> Option<std::path::PathBuf> {
    let entries = fs::read_dir(locks_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()))
            && fs::metadata(&path).is_ok_and(|m| m.len() > 0)
        {
            return Some(path);
        }
    }
    None
}

#[test]
fn ac_m1_10_4_stale_exclusive_lock_blocks_backup_with_exit_11() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&repo_dir).expect("mkdir repo");
        fs::create_dir_all(&src_dir).expect("mkdir src");

        // Create 24 MB of incompressible data so `restic check --read-data` holds its exclusive
        // lock long enough to be observed and SIGKILLed deterministically.
        let mut chunk = vec![0u8; 1024 * 1024];
        for (idx, b) in chunk.iter_mut().enumerate() {
            *b = ((idx.wrapping_mul(31) ^ (idx >> 5)) & 0xff) as u8;
        }
        for i in 0..24 {
            chunk[0] = i as u8;
            fs::write(src_dir.join(format!("blob_{i:02}.bin")), &chunk).expect("write blob");
        }

        let secrets_hex = dir.path().join("secrets.hex");
        let store = FileSecretStore::new(&secrets_hex);
        let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
        let resolved = check_binary_version(&restic_bin).expect("resolved restic");
        let clock = FakeClock::new(datetime!(2026-12-01 12:00:00 UTC));
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
        };

        let dest = Destination::new(
            "Local",
            DestinationKind::Local {
                path: repo_dir.clone(),
            },
        )
        .expect("dest");
        let pw = SecretValue::from_str_secret("stale-lock-test-pw");
        setup_destination(&dest, DestinationSetupMode::InitNew, &pw, None, &runtime)
            .unwrap_or_else(|e| panic!("{label}: init failed: {e}"));

        let backup =
            Backup::new("Stale Lock Test", Source::folder(&src_dir), dest.id.clone()).expect("bk");

        // Initial backup populates packs so `check --read-data` does real work under exclusive lock.
        let init_bk = execute_backup(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
            .unwrap_or_else(|e| panic!("{label}: initial backup failed: {e}"));
        assert_eq!(init_bk.exit_code, 0, "{label}");

        // Spawn `restic check --read-data` (acquires an exclusive lock) and SIGKILL it once
        // the lock file is written, leaving a genuine encrypted stale exclusive lock.
        let mut check_child = Command::new(&resolved.binary)
            .args(["-r", &repo_dir.to_string_lossy(), "check", "--read-data"])
            .env("RESTIC_PASSWORD", "stale-lock-test-pw")
            .env("RESTIC_HOST", "stale-other-host")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn restic check --read-data");

        let locks_dir = repo_dir.join("locks");
        let deadline = Instant::now() + Duration::from_secs(10);
        let lock_file = loop {
            if let Some(found) = find_lock_file(&locks_dir) {
                break found;
            }
            if let Some(status) = check_child.try_wait().expect("try_wait") {
                panic!(
                    "{label}: restic check exited prematurely with {status} before lock was observed"
                );
            }
            assert!(
                Instant::now() < deadline,
                "{label}: timed out waiting for exclusive lock file in {}",
                locks_dir.display()
            );
            thread::sleep(Duration::from_millis(2));
        };

        // Brief pause to ensure the lock file write is flushed, then SIGKILL.
        thread::sleep(Duration::from_millis(10));
        check_child.kill().expect("SIGKILL restic check");
        let _ = check_child.wait();
        assert!(
            lock_file.exists(),
            "{label}: stale exclusive lock file must remain after SIGKILL"
        );

        // AC-M1-10-4: Subsequent backup must fail with exit 11 + RepositoryLocked + UnlockRepository.
        let blocked_res = execute_backup(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
            .unwrap_or_else(|e| panic!("{label}: blocked backup execution failed: {e}"));

        assert_eq!(
            blocked_res.exit_code, 11,
            "{label}: expected exit code 11 on stale exclusive lock, got {} (logs: {:?})",
            blocked_res.exit_code, blocked_res.log_lines
        );
        assert_eq!(
            blocked_res.presentation.kind,
            ResticFailureKind::RepositoryLocked,
            "{label}"
        );
        assert_eq!(
            blocked_res.presentation.action,
            Some(RecoveryAction::UnlockRepository),
            "{label}"
        );

        // Unlock via `ResticCommandSpec::unlock`.
        let unlock_spec = ResticCommandSpec::unlock(&resolved.binary, &helper, &dest);
        let unlock_res = run_restic(&unlock_spec, &dest, Some(&store), &clock, None, &[], |_| {})
            .unwrap_or_else(|e| panic!("{label}: unlock runner failed: {e}"));
        assert_eq!(
            unlock_res.exit_code, 0,
            "{label}: restic unlock failed: {:?}",
            unlock_res.log_lines
        );

        // Subsequent backup succeeds cleanly.
        let recovered_res = execute_backup(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
            .unwrap_or_else(|e| panic!("{label}: post-unlock backup failed: {e}"));
        assert_eq!(recovered_res.exit_code, 0, "{label}");
        assert_eq!(recovered_res.presentation.status, RunStatus::Ok, "{label}");
    }
}
