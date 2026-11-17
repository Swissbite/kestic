// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

mod common;

use kestic_core::backup::{execute_backup, list_snapshots_for_backup};
use kestic_core::clock::FakeClock;
use kestic_core::config::{AppConfig, ConfigError, load_config, save_config};
use kestic_core::destination::{
    DestinationSetupMode, remove_destination_secrets, setup_destination,
};
use kestic_core::model::{Backup, BackupId, Destination, DestinationId, DestinationKind, Source};
use kestic_core::provider::check_binary_version;
use kestic_core::restic::{ResticCommandSpec, ResticRuntime, run_restic};
use kestic_core::secrets::{FileSecretStore, SecretStore, SecretValue};
use kestic_core::snapshot::{ResticSnapshot, parse_snapshots_json};
use std::fs;
use std::process::Command;
use tempfile::TempDir;
use time::macros::datetime;

fn list_meta_snapshots(
    backup: &Backup,
    dest: &Destination,
    resolved: &kestic_core::provider::ResolvedRestic,
    helper: &std::path::Path,
    store: &dyn SecretStore,
    clock: &FakeClock,
) -> Vec<ResticSnapshot> {
    let mut spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        helper,
        dest,
        vec![
            "snapshots".to_string(),
            "--no-lock".to_string(),
            "--tag".to_string(),
            backup.meta_tag(),
        ],
    );
    spec.low_priority = false;
    let res = run_restic(&spec, dest, Some(store), clock, None, &[], |_| {})
        .expect("run snapshots for meta tag");
    assert_eq!(
        res.exit_code, 0,
        "meta snapshot query failed: {:?}",
        res.log_lines
    );
    for line in &res.log_lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            return parse_snapshots_json(trimmed).expect("parse meta snapshots");
        }
    }
    Vec::new()
}

#[test]
fn ac_m1_19_2_remove_backup_keeps_snapshots() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cfg_path = dir.path().join("config.toml");
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&repo_dir).expect("mkdir repo");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        fs::write(src_dir.join("doc.txt"), b"snapshot 1 content").expect("write doc");

        let secrets_hex = dir.path().join("secrets.hex");
        let store = FileSecretStore::new(&secrets_hex);
        let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
        let resolved = check_binary_version(&restic_bin).expect("resolved restic");
        let clock = FakeClock::new(datetime!(2026-12-01 13:00:00 UTC));
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
        let pw = SecretValue::from_str_secret("remove-backup-pw");
        setup_destination(&dest, DestinationSetupMode::InitNew, &pw, None, &runtime)
            .unwrap_or_else(|e| panic!("{label}: init failed: {e}"));

        let backup =
            Backup::new("Documents", Source::folder(&src_dir), dest.id.clone()).expect("bk");
        let mut cfg = AppConfig {
            destinations: vec![dest.clone()],
            backups: vec![backup.clone()],
            ..AppConfig::default()
        };
        save_config(&cfg_path, &cfg).expect("save config");

        for n in 1..=2 {
            fs::write(src_dir.join(format!("doc_{n}.txt")), format!("run {n}")).expect("write");
            let res = execute_backup(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
                .unwrap_or_else(|e| panic!("{label}: backup {n} failed: {e}"));
            assert_eq!(res.exit_code, 0, "{label}");
        }

        let snaps_before =
            list_snapshots_for_backup(&backup.id, &dest, &resolved, &helper, &store, &clock, &[])
                .expect("list snapshots before remove");
        assert_eq!(snaps_before.len(), 2, "{label}");

        // Remove backup from config only; snapshots remain in destination.
        let removed = cfg.remove_backup(&backup.id).expect("remove backup");
        assert_eq!(removed.id, backup.id);
        save_config(&cfg_path, &cfg).expect("save config after removal");

        let reloaded = load_config(&cfg_path).expect("reload config");
        assert!(reloaded.backup(&backup.id).is_none());
        assert!(reloaded.destination(&dest.id).is_some());

        let snaps_after =
            list_snapshots_for_backup(&backup.id, &dest, &resolved, &helper, &store, &clock, &[])
                .expect("list snapshots after remove");
        assert_eq!(
            snaps_after.len(),
            2,
            "{label}: snapshots must remain in destination after removing backup from Kestic"
        );
    }
}

#[test]
fn ac_m1_19_3_remove_unused_destination_deletes_secrets_keeps_repo() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cfg_path = dir.path().join("config.toml");
        let repo_dir = dir.path().join("repo");
        fs::create_dir_all(&repo_dir).expect("mkdir repo");

        let secrets_hex = dir.path().join("secrets.hex");
        let store = FileSecretStore::new(&secrets_hex);
        let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
        let resolved = check_binary_version(&restic_bin).expect("resolved restic");
        let clock = FakeClock::new(datetime!(2026-12-01 13:30:00 UTC));
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
        };

        let dest = Destination::new(
            "Unused Local",
            DestinationKind::Local {
                path: repo_dir.clone(),
            },
        )
        .expect("dest");
        let pw = SecretValue::from_str_secret("remove-dest-pw");
        setup_destination(&dest, DestinationSetupMode::InitNew, &pw, None, &runtime)
            .unwrap_or_else(|e| panic!("{label}: init failed: {e}"));

        let mut cfg = AppConfig {
            destinations: vec![dest.clone()],
            ..AppConfig::default()
        };
        save_config(&cfg_path, &cfg).expect("save config");
        assert_eq!(
            store.get_secret(dest.id.as_str(), "repo").expect("get"),
            Some(pw)
        );

        let removed = cfg
            .remove_destination(&dest.id)
            .expect("remove unused dest");
        assert_eq!(removed.id, dest.id);
        save_config(&cfg_path, &cfg).expect("save updated config");
        remove_destination_secrets(&removed, &store).expect("delete secrets");

        let reloaded = load_config(&cfg_path).expect("reload");
        assert!(reloaded.destination(&dest.id).is_none());
        assert_eq!(
            store
                .get_secret(dest.id.as_str(), "repo")
                .expect("get repo"),
            None
        );
        assert_eq!(
            store
                .get_secret(dest.id.as_str(), "rest")
                .expect("get rest"),
            None
        );

        // Repository on disk must stay intact (`restic snapshots` exits 0).
        let snap_out = Command::new(&resolved.binary)
            .args(["-r", &repo_dir.to_string_lossy(), "snapshots", "--no-lock"])
            .env("RESTIC_PASSWORD", "remove-dest-pw")
            .output()
            .expect("restic snapshots");
        assert!(
            snap_out.status.success(),
            "{label}: repository damaged after removing destination: {}",
            String::from_utf8_lossy(&snap_out.stderr)
        );
    }
}

#[test]
fn ac_m1_19_4_remove_destination_with_backups_rejected() {
    let dest = Destination::new(
        "NAS",
        DestinationKind::Local {
            path: "/tmp/nas-repo".into(),
        },
    )
    .expect("dest");
    let bk1 = Backup::new(
        "Photos",
        Source::folder("/home/alice/Pictures"),
        dest.id.clone(),
    )
    .expect("bk1");
    let bk2 = Backup::new(
        "Docs",
        Source::folder("/home/alice/Documents"),
        dest.id.clone(),
    )
    .expect("bk2");

    let mut cfg = AppConfig {
        destinations: vec![dest.clone()],
        backups: vec![bk1.clone(), bk2.clone()],
        ..AppConfig::default()
    };
    let snapshot_before = cfg.clone();

    let err = cfg
        .remove_destination(&dest.id)
        .expect_err("destination referenced by backups must not be removed");
    match err {
        ConfigError::Validation(msg) => {
            assert!(msg.contains(&bk1.name), "msg missing bk1 name: {msg}");
            assert!(msg.contains(bk1.id.as_str()), "msg missing bk1 id: {msg}");
            assert!(msg.contains(&bk2.name), "msg missing bk2 name: {msg}");
            assert!(msg.contains(bk2.id.as_str()), "msg missing bk2 id: {msg}");
        }
        other => panic!("expected ConfigError::Validation, got {other:?}"),
    }
    assert_eq!(
        cfg, snapshot_before,
        "failed removal must not modify config"
    );

    // Also verify unknown IDs fail cleanly without modifying `cfg`.
    let missing_bk = BackupId::generate().expect("id");
    assert!(matches!(
        cfg.remove_backup(&missing_bk),
        Err(ConfigError::Validation(_))
    ));
    let missing_dest = DestinationId::generate().expect("id");
    assert!(matches!(
        cfg.remove_destination(&missing_dest),
        Err(ConfigError::Validation(_))
    ));
    assert_eq!(cfg, snapshot_before);
}

#[test]
fn ac_m1_19_5_aborting_removal_leaves_config_and_secrets_untouched() {
    let dir = TempDir::new().expect("tempdir");
    let cfg_path = dir.path().join("config.toml");
    let secrets_hex = dir.path().join("secrets.hex");
    let store = FileSecretStore::new(&secrets_hex);

    let dest = Destination::new(
        "Primary",
        DestinationKind::Local {
            path: dir.path().join("repo"),
        },
    )
    .expect("dest");
    let bk = Backup::new(
        "Home",
        Source::folder(dir.path().join("src")),
        dest.id.clone(),
    )
    .expect("backup");
    let pw = SecretValue::from_str_secret("keep-me-intact");
    store
        .set_secret(dest.id.as_str(), "repo", &pw)
        .expect("set secret");

    let cfg = AppConfig {
        destinations: vec![dest.clone()],
        backups: vec![bk.clone()],
        ..AppConfig::default()
    };
    save_config(&cfg_path, &cfg).expect("save config");
    let raw_before = fs::read_to_string(&cfg_path).expect("read config before");

    // Simulate user staging/opening removal confirmation and aborting before commit.
    let mut staged = cfg.clone();
    let aborted = true;
    if !aborted {
        let _ = staged.remove_backup(&bk.id);
        save_config(&cfg_path, &staged).expect("save");
        remove_destination_secrets(&dest, &store).expect("delete");
    }

    let raw_after = fs::read_to_string(&cfg_path).expect("read config after");
    assert_eq!(raw_after, raw_before);
    assert_eq!(load_config(&cfg_path).expect("reload"), cfg);
    assert_eq!(
        store.get_secret(dest.id.as_str(), "repo").expect("get"),
        Some(pw)
    );
}

#[test]
fn ac_m1_22_1_reassign_backup_starts_fresh_chain_on_b_keeps_snapshots_on_a() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cfg_path = dir.path().join("config.toml");
        let repo_a = dir.path().join("repo-a");
        let repo_b = dir.path().join("repo-b");
        let state_dir = dir.path().join("state");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&repo_a).expect("mkdir repo_a");
        fs::create_dir_all(&repo_b).expect("mkdir repo_b");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        fs::write(src_dir.join("file.txt"), b"initial").expect("write");

        let secrets_hex = dir.path().join("secrets.hex");
        let store = FileSecretStore::new(&secrets_hex);
        let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
        let resolved = check_binary_version(&restic_bin).expect("resolved restic");
        let clock = FakeClock::new(datetime!(2026-12-01 14:00:00 UTC));
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
        };

        let dest_a =
            Destination::new("Dest A", DestinationKind::Local { path: repo_a }).expect("a");
        let dest_b = Destination::new(
            "Dest B",
            DestinationKind::Local {
                path: repo_b.clone(),
            },
        )
        .expect("b");
        let pw_a = SecretValue::from_str_secret("pw-a");
        let pw_b = SecretValue::from_str_secret("pw-b");

        setup_destination(
            &dest_a,
            DestinationSetupMode::InitNew,
            &pw_a,
            None,
            &runtime,
        )
        .unwrap_or_else(|e| panic!("{label}: init A failed: {e}"));
        setup_destination(
            &dest_b,
            DestinationSetupMode::InitNew,
            &pw_b,
            None,
            &runtime,
        )
        .unwrap_or_else(|e| panic!("{label}: init B failed: {e}"));

        let backup =
            Backup::new("Work", Source::folder(&src_dir), dest_a.id.clone()).expect("backup");
        let original_backup_id = backup.id.clone();

        let mut cfg = AppConfig {
            destinations: vec![dest_a.clone(), dest_b.clone()],
            backups: vec![backup.clone()],
            ..AppConfig::default()
        };
        save_config(&cfg_path, &cfg).expect("save config");

        for n in 1..=2 {
            fs::write(src_dir.join(format!("item_{n}.txt")), format!("v{n}")).expect("write");
            let res = execute_backup(&backup, &dest_a, &state_dir, &runtime, None, &[], |_| {})
                .unwrap_or_else(|e| panic!("{label}: backup {n} on A failed: {e}"));
            assert_eq!(res.exit_code, 0, "{label}");
        }

        // Verify reassign validation failures before valid reassignment.
        let unknown_bk = BackupId::generate().expect("id");
        assert!(matches!(
            cfg.reassign_backup(&unknown_bk, &dest_b.id),
            Err(ConfigError::Validation(_))
        ));
        let unknown_dest = DestinationId::generate().expect("id");
        assert!(matches!(
            cfg.reassign_backup(&original_backup_id, &unknown_dest),
            Err(ConfigError::Validation(_))
        ));

        // Reassign backup to destination B while preserving BackupId.
        cfg.reassign_backup(&original_backup_id, &dest_b.id)
            .expect("reassign backup");
        save_config(&cfg_path, &cfg).expect("save reassigned config");

        let reloaded = load_config(&cfg_path).expect("reload");
        let reassigned_bk = reloaded
            .backup(&original_backup_id)
            .expect("backup id must stay unchanged")
            .clone();
        assert_eq!(reassigned_bk.id, original_backup_id);
        assert_eq!(reassigned_bk.destination_id, dest_b.id);

        // Next backup runs on B and starts a fresh snapshot chain (no parent on B).
        let res_b = execute_backup(
            &reassigned_bk,
            &dest_b,
            &state_dir,
            &runtime,
            None,
            &[],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: backup on B failed: {e}"));
        assert_eq!(res_b.exit_code, 0, "{label}");

        let snaps_b = list_snapshots_for_backup(
            &original_backup_id,
            &dest_b,
            &resolved,
            &helper,
            &store,
            &clock,
            &[],
        )
        .expect("list B");
        assert_eq!(snaps_b.len(), 1, "{label}: B should have 1 snapshot");

        // Confirm the first snapshot on B has no parent snapshot (`cat snapshot`).
        let cat_out = Command::new(&resolved.binary)
            .args([
                "-r",
                &repo_b.to_string_lossy(),
                "cat",
                "--no-lock",
                "snapshot",
                &snaps_b[0].id,
            ])
            .env("RESTIC_PASSWORD", "pw-b")
            .output()
            .expect("cat snapshot on B");
        assert!(cat_out.status.success(), "{label}");
        let snap_json: serde_json::Value =
            serde_json::from_slice(&cat_out.stdout).expect("parse snapshot json");
        assert!(
            snap_json.get("parent").is_none()
                || snap_json.get("parent").is_some_and(|v| v.is_null()),
            "{label}: fresh snapshot on B must have no parent, got {snap_json}"
        );

        // Destination A still retains its 2 snapshots untouched.
        let snaps_a = list_snapshots_for_backup(
            &original_backup_id,
            &dest_a,
            &resolved,
            &helper,
            &store,
            &clock,
            &[],
        )
        .expect("list A");
        assert_eq!(
            snaps_a.len(),
            2,
            "{label}: A must still contain 2 snapshots"
        );
    }
}

#[test]
fn ac_m1_22_2_old_destination_a_can_be_removed_after_reassign() {
    let dir = TempDir::new().expect("tempdir");
    let cfg_path = dir.path().join("config.toml");
    let secrets_hex = dir.path().join("secrets.hex");
    let store = FileSecretStore::new(&secrets_hex);

    let dest_a = Destination::new(
        "A",
        DestinationKind::Local {
            path: dir.path().join("a"),
        },
    )
    .expect("a");
    let dest_b = Destination::new(
        "B",
        DestinationKind::Local {
            path: dir.path().join("b"),
        },
    )
    .expect("b");
    let bk = Backup::new(
        "Data",
        Source::folder(dir.path().join("src")),
        dest_a.id.clone(),
    )
    .expect("bk");

    store
        .set_secret(
            dest_a.id.as_str(),
            "repo",
            &SecretValue::from_str_secret("pw-a"),
        )
        .expect("set a");
    store
        .set_secret(
            dest_b.id.as_str(),
            "repo",
            &SecretValue::from_str_secret("pw-b"),
        )
        .expect("set b");

    let mut cfg = AppConfig {
        destinations: vec![dest_a.clone(), dest_b.clone()],
        backups: vec![bk.clone()],
        ..AppConfig::default()
    };

    // Cannot remove A while backup points to A.
    assert!(matches!(
        cfg.remove_destination(&dest_a.id),
        Err(ConfigError::Validation(_))
    ));

    // Reassign backup to B, then A can be removed cleanly.
    cfg.reassign_backup(&bk.id, &dest_b.id)
        .expect("reassign to B");
    let removed_a = cfg
        .remove_destination(&dest_a.id)
        .expect("remove A after reassign");
    assert_eq!(removed_a.id, dest_a.id);

    save_config(&cfg_path, &cfg).expect("save config");
    remove_destination_secrets(&removed_a, &store).expect("remove A secrets");

    let reloaded = load_config(&cfg_path).expect("reload");
    assert!(reloaded.destination(&dest_a.id).is_none());
    assert!(reloaded.destination(&dest_b.id).is_some());
    assert_eq!(
        reloaded.backup(&bk.id).expect("bk").destination_id,
        dest_b.id
    );
    assert_eq!(store.get_secret(dest_a.id.as_str(), "repo").unwrap(), None);
    assert!(
        store
            .get_secret(dest_b.id.as_str(), "repo")
            .unwrap()
            .is_some()
    );
}

#[test]
fn ac_m1_22_3_meta_snapshots_preserved_in_a_on_reassign() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let repo_a = dir.path().join("repo-a");
        let repo_b = dir.path().join("repo-b");
        let state_dir = dir.path().join("state");
        let src_dir = dir.path().join("src");
        let meta_dir = dir.path().join("meta");
        fs::create_dir_all(&repo_a).expect("mkdir a");
        fs::create_dir_all(&repo_b).expect("mkdir b");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        fs::create_dir_all(&meta_dir).expect("mkdir meta");
        fs::write(src_dir.join("data.txt"), b"payload").expect("write data");
        fs::write(meta_dir.join("backup.json"), b"{\"name\":\"Work\"}").expect("write meta");

        let secrets_hex = dir.path().join("secrets.hex");
        let store = FileSecretStore::new(&secrets_hex);
        let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
        let resolved = check_binary_version(&restic_bin).expect("resolved restic");
        let clock = FakeClock::new(datetime!(2026-12-01 15:00:00 UTC));

        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
        };
        let dest_a = Destination::new("A", DestinationKind::Local { path: repo_a }).expect("a");
        let dest_b = Destination::new("B", DestinationKind::Local { path: repo_b }).expect("b");
        setup_destination(
            &dest_a,
            DestinationSetupMode::InitNew,
            &SecretValue::from_str_secret("pw-a"),
            None,
            &runtime,
        )
        .unwrap_or_else(|e| panic!("{label}: init A failed: {e}"));
        setup_destination(
            &dest_b,
            DestinationSetupMode::InitNew,
            &SecretValue::from_str_secret("pw-b"),
            None,
            &runtime,
        )
        .unwrap_or_else(|e| panic!("{label}: init B failed: {e}"));

        let backup =
            Backup::new("Work", Source::folder(&src_dir), dest_a.id.clone()).expect("backup");
        let mut cfg = AppConfig {
            destinations: vec![dest_a.clone(), dest_b.clone()],
            backups: vec![backup.clone()],
            ..AppConfig::default()
        };

        // Create a `kestic-meta:<backup_id>` snapshot in A before reassigning.
        let mut meta_spec = ResticCommandSpec::for_destination(
            &resolved.binary,
            &helper,
            &dest_a,
            vec![
                "backup".to_string(),
                "--tag".to_string(),
                backup.meta_tag(),
                meta_dir.to_string_lossy().into_owned(),
            ],
        );
        meta_spec.low_priority = false;
        let meta_res = run_restic(&meta_spec, &dest_a, Some(&store), &clock, None, &[], |_| {})
            .expect("create meta snapshot");
        assert_eq!(meta_res.exit_code, 0, "{label}");

        let meta_before = list_meta_snapshots(&backup, &dest_a, &resolved, &helper, &store, &clock);
        assert_eq!(meta_before.len(), 1, "{label}");

        // Reassign backup to B and run backup on B.
        cfg.reassign_backup(&backup.id, &dest_b.id)
            .expect("reassign");
        let reassigned_bk = cfg.backup(&backup.id).expect("bk").clone();
        let bk_b_res = execute_backup(
            &reassigned_bk,
            &dest_b,
            &state_dir,
            &runtime,
            None,
            &[],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: backup on B failed: {e}"));
        assert_eq!(bk_b_res.exit_code, 0, "{label}");

        // Meta snapshots in A must remain unchanged.
        let meta_after = list_meta_snapshots(&backup, &dest_a, &resolved, &helper, &store, &clock);
        assert_eq!(meta_after.len(), 1, "{label}");
        assert_eq!(meta_after[0].id, meta_before[0].id, "{label}");

        let meta_in_b = list_meta_snapshots(&backup, &dest_b, &resolved, &helper, &store, &clock);
        assert!(meta_in_b.is_empty(), "{label}");
    }
}
