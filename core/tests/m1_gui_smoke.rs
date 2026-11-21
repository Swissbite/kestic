// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Comprehensive offscreen GUI smoke suite for Milestone M1 (`M1-S13`–`M1-S23`, `NFR-08`).
//!
//! Each scenario runs completely isolated with its own `TempDir` XDG directories,
//! private `dbus-daemon` with a headless keyring, real `kestic daemon`,
//! and `kestic --smoke-m1 <scenario>`, capturing PNG screenshots into
//! `target/smoke-artifacts/m1-*.png` for visual inspection.

mod common;

use common::execute_backup;

use kestic_test_support::gui::{
    DaemonProcessGuard, IsolatedEnv, PrivateDbusGuard, SmokeContext, SmokeFixture,
    find_kestic_binary,
};

use kestic_core::config::{AppConfig, save_config};
use kestic_core::history::RunHistory;
use kestic_core::model::{Backup, Destination, ResticProvider, Run, RunStatus, Source};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use time::macros::datetime;

fn setup_populated_fixture(fx: &SmokeFixture) -> (Destination, Backup, Run, PathBuf) {
    let (iso, restic) = (&fx.iso, fx.restic.as_path());
    let repo_dir = iso.root.join("repo-primary");
    let src_dir = iso.root.join("source-docs");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("notes.txt"), b"Hello Kestic M1!").expect("write sample doc");

    let rig = common::ResticRig::new(&iso.root, restic, datetime!(2026-11-16 12:00:00 UTC));
    let runtime = rig.runtime();

    let dest = rig.init_local("Local Vault", &repo_dir, "m1-smoke-pw");
    fx.keyring
        .store_secret(dest.id.as_str(), "repo", "m1-smoke-pw");

    let backup = Backup::new("Documents", Source::folder(&src_dir), dest.id.clone()).expect("bk");
    let res = execute_backup(
        &backup,
        &dest,
        &iso.state_kestic_dir(),
        &runtime,
        None,
        &[],
        |_| {},
    )
    .expect("execute backup");
    assert_eq!(res.exit_code, 0);

    let mut run = Run::new_backup(&rig.clock, &backup).expect("run");
    run.log_path = Some(iso.state_kestic_dir().join("logs").join("sample-run.log"));
    if let Some(ref lp) = run.log_path {
        fs::create_dir_all(lp.parent().expect("log parent")).expect("mkdir logs");
        fs::write(lp, "[info] Snapshot created successfully.\n").expect("write log");
    }
    run.stats = res.stats;
    run.commands.push(res.command_record);
    run.finish(&rig.clock, RunStatus::Ok, Some(0));

    let mut history = RunHistory::open(&iso.state_kestic_dir()).expect("open history");
    history.upsert_run(&run, &[]).expect("upsert run");

    let cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: restic.to_path_buf(),
        },
        expert_mode: false,
        destinations: vec![dest.clone()],
        backups: vec![backup.clone()],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save config");
    (dest, backup, run, repo_dir)
}

#[test]
fn m1_gui_smoke_welcome_wide_narrow_en_and_de() {
    let fx = SmokeFixture::new("welcome", common::latest_restic());
    let _daemon = fx.daemon("en");

    let ctx = fx.ctx();

    ctx.run(
        "welcome",
        "wide",
        "en",
        "target/smoke-artifacts/m1-welcome-wide-en.png",
        &[],
    );
    ctx.run(
        "welcome",
        "narrow",
        "en",
        "target/smoke-artifacts/m1-welcome-narrow-en.png",
        &[],
    );
    ctx.run(
        "welcome",
        "narrow",
        "de",
        "target/smoke-artifacts/m1-welcome-narrow-de.png",
        &[],
    );
    ctx.run(
        "destinations-empty",
        "wide",
        "de",
        "target/smoke-artifacts/m1-destinations-empty-de.png",
        &[],
    );
}

#[test]
fn m1_gui_smoke_blocking_states_and_settings() {
    let kestic = find_kestic_binary();
    let restic = common::latest_restic();

    // 1. Daemon unreachable (--no-autostart, no daemon running)
    {
        let fx = SmokeFixture::new("blk-daemon", common::latest_restic());
        fx.ctx().run(
            "blocking-daemon_unreachable",
            "wide",
            "en",
            "target/smoke-artifacts/m1-blocking-daemon.png",
            &["--no-autostart"],
        );
    }

    // 2. Secret store locked (no org.freedesktop.secrets on the private bus)
    {
        let iso = IsolatedEnv::create("blk-secret", Some(&restic));
        let dbus = PrivateDbusGuard::start();
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, "en");
        SmokeContext {
            kestic_bin: &kestic,
            iso: &iso,
            dbus_addr: &dbus.address,
        }
        .run(
            "blocking-secret_store_locked",
            "wide",
            "en",
            "target/smoke-artifacts/m1-blocking-secret.png",
            &[],
        );
    }

    // 3. restic unavailable (invalid custom restic path in config) + Settings dialog
    {
        let iso = IsolatedEnv::create("blk-restic", None);
        let cfg = AppConfig {
            restic_provider: ResticProvider::Custom {
                path: PathBuf::from("/nonexistent/kestic-missing-restic"),
            },
            ..AppConfig::default()
        };
        save_config(&iso.config_path(), &cfg).expect("save invalid restic cfg");
        let dbus = PrivateDbusGuard::start();
        let _keyring = iso.start_keyring(&dbus.address);
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, "en");

        let ctx = SmokeContext {
            kestic_bin: &kestic,
            iso: &iso,
            dbus_addr: &dbus.address,
        };
        ctx.run(
            "blocking-restic_unavailable",
            "wide",
            "en",
            "target/smoke-artifacts/m1-blocking-restic.png",
            &[],
        );
        ctx.run(
            "settings",
            "wide",
            "en",
            "target/smoke-artifacts/m1-settings.png",
            &[],
        );
    }
}

#[test]
fn m1_gui_smoke_populated_workflows_show_cmd_history_wizard_quit() {
    let fx = SmokeFixture::new("populated", common::latest_restic());
    setup_populated_fixture(&fx);

    let _daemon = fx.daemon("en");

    let ctx = fx.ctx();
    ctx.run(
        "overview",
        "wide",
        "en",
        "target/smoke-artifacts/m1-overview.png",
        &[],
    );
    ctx.run(
        "backups-detail-show-cmd",
        "wide",
        "en",
        "target/smoke-artifacts/m1-backups-show-cmd.png",
        &[],
    );
    ctx.run(
        "destinations-reassign",
        "wide",
        "en",
        "target/smoke-artifacts/m1-destinations-reassign.png",
        &[],
    );
    ctx.run(
        "history-run-detail",
        "wide",
        "en",
        "target/smoke-artifacts/m1-history-run-detail.png",
        &[],
    );
    ctx.run(
        "wizard-validation",
        "wide",
        "en",
        "target/smoke-artifacts/m1-wizard-validation.png",
        &[],
    );
    ctx.run(
        "quit-dialog",
        "wide",
        "en",
        "target/smoke-artifacts/m1-quit-dialog.png",
        &["--confirm-quit"],
    );
}

#[test]
fn m1_gui_smoke_snapshots_previous_after_reassign() {
    // Previous destination snapshots after reassigning backup
    let fx = SmokeFixture::new("snap-prev", common::latest_restic());
    let (restic, iso) = (&fx.restic, &fx.iso);
    let (dest1, mut backup, _run, _) = setup_populated_fixture(&fx);

    let rig = common::ResticRig::new(&iso.root, restic, datetime!(2026-11-16 12:30:00 UTC));
    let dest2 = rig.init_local(
        "Archive Drive",
        &iso.root.join("repo-secondary"),
        "m1-smoke-pw2",
    );
    fx.keyring
        .store_secret(dest2.id.as_str(), "repo", "m1-smoke-pw2");

    backup.destination_id = dest2.id.clone();
    let cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: restic.clone(),
        },
        destinations: vec![dest1, dest2],
        backups: vec![backup],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save reassigned cfg");

    let _daemon = fx.daemon("en");
    fx.ctx().run(
        "snapshots-previous",
        "wide",
        "en",
        "target/smoke-artifacts/m1-snapshots-previous.png",
        &[],
    );
}

#[test]
fn m1_gui_smoke_manual_run_blocked_when_source_missing() {
    // ManualRunBlocked inline banner when source directory disappears
    let fx = SmokeFixture::new("run-blocked", common::latest_restic());
    let (restic, iso) = (&fx.restic, &fx.iso);
    let repo_dir = iso.root.join("repo");
    let missing_src = iso.root.join("missing-source-folder");
    fs::create_dir_all(&missing_src).expect("mkdir tmp src");

    let rig = common::ResticRig::new(&iso.root, restic, datetime!(2026-11-16 13:00:00 UTC));
    let dest = rig.init_local("Local", &repo_dir, "m1-smoke-pw");
    fx.keyring
        .store_secret(dest.id.as_str(), "repo", "m1-smoke-pw");
    let backup =
        Backup::new("MissingSrc", Source::folder(&missing_src), dest.id.clone()).expect("bk");
    fs::remove_dir_all(&missing_src).expect("remove source dir so manual start is blocked");

    let cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: restic.clone(),
        },
        destinations: vec![dest],
        backups: vec![backup],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save cfg");

    let _daemon = fx.daemon("en");
    fx.ctx().run(
        "manual-run-blocked",
        "wide",
        "en",
        "target/smoke-artifacts/m1-manual-run-blocked.png",
        &[],
    );
}

#[test]
fn m1_gui_e2e_create_destination_and_backup_with_real_restic() {
    let fx = SmokeFixture::new("e2e-wizard", common::latest_restic());
    let (restic, iso) = (&fx.restic, &fx.iso);

    let repo_dir = iso.root.join("e2e-repo");
    let src_dir = iso.root.join("e2e-source");
    fs::create_dir_all(&repo_dir).expect("mkdir repo_dir");
    fs::create_dir_all(&src_dir).expect("mkdir src_dir");
    fs::write(src_dir.join("hello.txt"), b"hello from kestic e2e\n").expect("write test file");

    let _daemon = fx.daemon("en");

    let repo_str = repo_dir.to_string_lossy().into_owned();
    let src_str = src_dir.to_string_lossy().into_owned();
    fx.ctx().run_with_env(
        "e2e-wizard-backup",
        "wide",
        "en",
        "target/smoke-artifacts/m1-e2e-wizard-backup.png",
        &[],
        &[
            ("KESTIC_SMOKE_REPO_DIR", &repo_str),
            ("KESTIC_SMOKE_SRC_DIR", &src_str),
        ],
    );

    let cfg = common::load_config(&iso.config_path()).expect("load saved config");
    assert_eq!(cfg.destinations.len(), 1, "expected 1 saved destination");
    assert_eq!(cfg.destinations[0].name, "E2E Local Repo");
    assert_eq!(cfg.backups.len(), 1, "expected 1 saved backup");

    let cache_dir = common::restic_cache_dir(&iso.root);
    let snap_out = Command::new(restic)
        .args(["-r", &repo_str, "snapshots", "--json"])
        .env("RESTIC_PASSWORD", "e2e-test-pw")
        .env("RESTIC_CACHE_DIR", &cache_dir)
        .output()
        .expect("run real restic snapshots --json");
    assert!(
        snap_out.status.success(),
        "restic snapshots failed: {}",
        String::from_utf8_lossy(&snap_out.stderr)
    );
    let snaps: serde_json::Value =
        serde_json::from_slice(&snap_out.stdout).expect("parse restic snapshots JSON");
    let snap_arr = snaps.as_array().expect("snapshots JSON array");
    assert_eq!(
        snap_arr.len(),
        1,
        "expected exactly 1 real restic snapshot on disk, got {snap_arr:?}"
    );
}

#[test]
fn m1_gui_e2e_setup_first_backup_with_real_restic() {
    let fx = SmokeFixture::new("e2e-first-backup", common::latest_restic());
    let (restic, iso) = (&fx.restic, &fx.iso);

    let repo_dir = iso.root.join("first-repo");
    let src_dir = iso.root.join("first-source");
    fs::create_dir_all(&repo_dir).expect("mkdir repo_dir");
    fs::create_dir_all(&src_dir).expect("mkdir src_dir");
    fs::write(src_dir.join("first.txt"), b"hello first backup\n").expect("write test file");

    let _daemon = fx.daemon("en");

    let repo_str = repo_dir.to_string_lossy().into_owned();
    let src_str = src_dir.to_string_lossy().into_owned();
    fx.ctx().run_with_env(
        "e2e-first-backup",
        "wide",
        "en",
        "target/smoke-artifacts/m1-e2e-first-backup.png",
        &[],
        &[
            ("KESTIC_SMOKE_REPO_DIR", &repo_str),
            ("KESTIC_SMOKE_SRC_DIR", &src_str),
        ],
    );

    let cfg = common::load_config(&iso.config_path()).expect("load saved config");
    assert_eq!(cfg.destinations.len(), 1, "expected 1 saved destination");
    assert_eq!(cfg.destinations[0].name, "First Local Repo");
    assert_eq!(cfg.backups.len(), 1, "expected 1 saved backup");

    let cache_dir = common::restic_cache_dir(&iso.root);
    let snap_out = Command::new(restic)
        .args(["-r", &repo_str, "snapshots", "--json"])
        .env("RESTIC_PASSWORD", "e2e-first-pw")
        .env("RESTIC_CACHE_DIR", &cache_dir)
        .output()
        .expect("run real restic snapshots --json");
    assert!(
        snap_out.status.success(),
        "restic snapshots failed: {}",
        String::from_utf8_lossy(&snap_out.stderr)
    );
    let snaps: serde_json::Value =
        serde_json::from_slice(&snap_out.stdout).expect("parse restic snapshots JSON");
    let snap_arr = snaps.as_array().expect("snapshots JSON array");
    assert_eq!(
        snap_arr.len(),
        1,
        "expected exactly 1 real restic snapshot on disk, got {snap_arr:?}"
    );
}

#[test]
fn m1_gui_e2e_destination_edit_reinit_and_nested_wizard_single_column() {
    let fx = SmokeFixture::new("e2e-edit-reinit", common::latest_restic());

    let (_dest, _backup, _run, repo_dir) = setup_populated_fixture(&fx);

    // Delete repository directory contents and recreate it empty (simulating Dolphin / rm + mkdir).
    fs::remove_dir_all(&repo_dir).expect("remove repo_dir");
    fs::create_dir_all(&repo_dir).expect("recreate empty repo_dir");
    assert!(
        !repo_dir.join("config").exists(),
        "repo config must be absent before UI edit re-init"
    );

    let _daemon = fx.daemon("en");

    fx.ctx().run(
        "e2e-edit-reinit-and-nested-wizard",
        "wide",
        "en",
        "target/smoke-artifacts/m1-e2e-edit-reinit-nested.png",
        &[],
    );

    assert!(
        repo_dir.join("config").is_file(),
        "saving DestinationEditPage after switching to Create New Repository must initialize real restic repo"
    );
}

#[test]
fn m1_gui_smoke_keyboard_navigation() {
    let fx = SmokeFixture::new("kbd-nav", common::latest_restic());
    setup_populated_fixture(&fx);

    let _daemon = fx.daemon("en");

    fx.ctx().run(
        "keyboard-navigation",
        "wide",
        "en",
        "target/smoke-artifacts/m1-keyboard-navigation.png",
        &[],
    );
}
