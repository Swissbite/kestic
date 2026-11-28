// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Comprehensive offscreen GUI smoke suite for Milestone M2 (`M2-S01`–`M2-S11`).
//!
//! Each scenario runs completely isolated with its own temporary XDG directories,
//! private `dbus-daemon` with a headless keyring, real `kestic daemon`, and
//! `kestic --smoke-m1 m2-...`, capturing PNG screenshots into
//! `target/smoke-artifacts/m2-*.png` and verifying persisted config, exclude
//! files, and real restic repository state.

mod common;

use common::execute_backup;
use kestic_test_support::config::load_config;

use kestic_test_support::gui::SmokeFixture;

use kestic_core::backup::exclude_file_path;
use kestic_core::clock::FakeClock;
use kestic_core::config::{AppConfig, save_config};
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, Destination, ExcludeRules, KeepCount, ResticProvider, RetentionPolicy, Run, RunStatus,
    Source,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use time::macros::datetime;

/// Copies a prepared local repository (own generated password) to `repo_dir`, connects it and
/// stores its password in the fixture's keyring. Returns the destination and the password.
fn local_destination(fx: &SmokeFixture, name: &str, repo_dir: &Path) -> (Destination, String) {
    let (dest, password) =
        common::ResticRig::new(&fx.iso.root, &fx.restic, datetime!(2026-11-16 12:00:00 UTC))
            .local_repo(name, repo_dir);
    fx.keyring.store_secret(dest.id.as_str(), "repo", &password);
    (dest, password)
}

fn count_repo_snapshots(restic: &Path, repo_dir: &Path, password: &str, cache_dir: &Path) -> usize {
    let out = Command::new(restic)
        .args(["-r", &repo_dir.to_string_lossy(), "snapshots", "--json"])
        .env("RESTIC_PASSWORD", password)
        .env("RESTIC_CACHE_DIR", cache_dir)
        .output()
        .expect("run restic snapshots --json");
    assert!(
        out.status.success(),
        "restic snapshots failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let snaps: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("parse snapshots JSON");
    snaps.as_array().expect("snapshots JSON array").len()
}

/// The exclude rules editor in simple and expert mode with the lint.
/// Also AC-M6-34-2 (c) and AC-M6-34-4 (b): the editor, whose fields go through `core`, behaves
/// as before, and the smoke fails on any QML warning.
#[test]
fn m2_gui_smoke_excludes_simple_expert_and_lint_wide_narrow_en_de() {
    let fx = SmokeFixture::new("excludes", common::latest_restic());
    let (restic, iso) = (&fx.restic, &fx.iso);
    let repo_dir = iso.root.join("repo-excludes");
    let src_dir = iso.root.join("source-excludes");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("keep.txt"), b"keep").expect("write keep");

    let (dest, _repo_pw) = local_destination(&fx, "Local Vault", &repo_dir);
    let mut backup =
        Backup::new("Documents", Source::folder(&src_dir), dest.id.clone()).expect("bk");
    let initial_excludes = ExcludeRules {
        folders: vec!["docs/a".to_string(), "docs/b".to_string()],
        lines: vec![
            "*.tmp".to_string(),
            "dir".to_string(),
            "!dir/inner".to_string(),
        ],
        ..ExcludeRules::default()
    };
    backup.excludes = initial_excludes;

    let initial_cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: restic.clone(),
        },
        expert_mode: false,
        destinations: vec![dest],
        backups: vec![backup.clone()],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &initial_cfg).expect("save initial excludes config");

    let ctx = fx.ctx();

    let assert_persisted_excludes = |label: &str| {
        let saved = load_config(&iso.config_path()).expect("reload saved config");
        assert!(
            saved.expert_mode,
            "[{label}] expected expert_mode == true after confirming Expert Mode"
        );
        let lines = &saved.backups[0].excludes.lines;
        assert!(
            lines.iter().any(|l| l == "dir/*"),
            "[{label}] expected quick-fix 'dir/*' in saved excludes: {lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l == "dir"),
            "[{label}] expected 'dir' to be replaced by 'dir/*': {lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l == "draft-preserved"),
            "[{label}] failed invalid save must NOT persist 'draft-preserved': {lines:?}"
        );

        let excl_path = exclude_file_path(&iso.state_kestic_dir(), &backup.id);
        let excl_contents = fs::read_to_string(&excl_path).expect("read regenerated exclude file");
        assert!(
            excl_contents.contains("dir/*"),
            "[{label}] regenerated exclude file missing 'dir/*': {excl_contents}"
        );
        assert!(
            !excl_contents.contains("draft-preserved"),
            "[{label}] exclude file must NOT contain 'draft-preserved': {excl_contents}"
        );
    };

    // 1. Wide / EN
    {
        let _daemon = fx.daemon("en");
        ctx.run(
            "m2-excludes-simple-expert-lint",
            "wide",
            "en",
            "target/smoke-artifacts/m2-excludes-wide-en.png",
            &[],
        );
    }
    assert_persisted_excludes("wide/en");

    // 2. Reset config.toml to expert_mode=false + initial excludes and run Wide / DE
    save_config(&iso.config_path(), &initial_cfg).expect("reset config for wide/de");
    {
        let _daemon = fx.daemon("de");
        ctx.run(
            "m2-excludes-simple-expert-lint",
            "wide",
            "de",
            "target/smoke-artifacts/m2-excludes-wide-de.png",
            &[],
        );
    }
    assert_persisted_excludes("wide/de");

    // 3. Reset config.toml to expert_mode=false + initial excludes and run Narrow / EN
    save_config(&iso.config_path(), &initial_cfg).expect("reset config for narrow/en");
    {
        let _daemon = fx.daemon("en");
        ctx.run(
            "m2-excludes-simple-expert-lint",
            "narrow",
            "en",
            "target/smoke-artifacts/m2-excludes-narrow-en.png",
            &[],
        );
    }
    assert_persisted_excludes("narrow/en");
}

#[test]
fn m2_gui_smoke_dry_run_preview_real_restic() {
    let fx = SmokeFixture::new("dry-run", common::latest_restic());
    let (restic, iso) = (&fx.restic, &fx.iso);
    let repo_dir = iso.root.join("repo-dry-run");
    let src_dir = iso.root.join("src-dry-run");
    fs::create_dir_all(src_dir.join("sub")).expect("mkdir src/sub");
    fs::write(src_dir.join("notes.txt"), b"Hello dry-run preview!").expect("write notes.txt");
    fs::write(src_dir.join("sub/report.md"), b"# Report\nDry run").expect("write report.md");

    let (dest, repo_pw) = local_destination(&fx, "DryRun Vault", &repo_dir);
    let backup = Backup::new("Docs", Source::folder(&src_dir), dest.id.clone()).expect("bk");

    let cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: restic.clone(),
        },
        destinations: vec![dest],
        backups: vec![backup],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save dry-run config");

    let _daemon = fx.daemon("en");

    fx.ctx().run(
        "m2-dry-run-preview",
        "wide",
        "en",
        "target/smoke-artifacts/m2-dry-run-preview-en.png",
        &[],
    );

    assert_eq!(
        count_repo_snapshots(
            restic,
            &repo_dir,
            &repo_pw,
            &common::restic_cache_dir(&iso.root)
        ),
        0,
        "dry-run preview must NOT create any real restic snapshot"
    );
}

/// The retention editor with the preview before saving.
/// Also AC-M6-34-2 (c) and AC-M6-34-4 (b): the editor, whose fields go through `core`, behaves
/// as before, and the smoke fails on any QML warning.
#[test]
fn m2_gui_smoke_retention_editor_preview_confirm_en_de_narrow() {
    let fx = SmokeFixture::new("retention", common::latest_restic());
    let (restic, iso) = (&fx.restic, &fx.iso);
    let cache_dir = common::restic_cache_dir(&iso.root);
    let repo_dir = iso.root.join("repo-retention");
    let src_dir = iso.root.join("src-retention");
    fs::create_dir_all(&src_dir).expect("mkdir src");

    let (dest, repo_pw) = local_destination(&fx, "Retention Vault", &repo_dir);
    let mut backup =
        Backup::new("Projects", Source::folder(&src_dir), dest.id.clone()).expect("bk");
    backup.retention = RetentionPolicy::standard();

    let rig = common::ResticRig::new(&iso.root, restic, datetime!(2026-11-14 12:00:00 UTC));
    let runtime = rig.runtime();

    for (idx, ts) in [
        datetime!(2026-11-14 12:00:00 UTC),
        datetime!(2026-11-15 12:00:00 UTC),
        datetime!(2026-11-16 12:00:00 UTC),
    ]
    .into_iter()
    .enumerate()
    {
        fs::write(
            src_dir.join(format!("file-{idx}.txt")),
            format!("snapshot payload {idx}\n"),
        )
        .expect("write snapshot file");
        rig.clock.set(ts);
        let res = execute_backup(
            &backup,
            &dest,
            &iso.state_kestic_dir(),
            &runtime,
            None,
            &[],
            |_| {},
        )
        .expect("seed snapshot");
        assert_eq!(res.exit_code, 0);
    }
    assert_eq!(
        count_repo_snapshots(restic, &repo_dir, &repo_pw, &cache_dir),
        3,
        "expected 3 initial snapshots in repo"
    );

    let initial_cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: restic.clone(),
        },
        destinations: vec![dest],
        backups: vec![backup],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &initial_cfg).expect("save retention config");

    let ctx = fx.ctx();

    let assert_retention_saved_and_intact = |label: &str| {
        let saved = load_config(&iso.config_path()).expect("reload retention config");
        assert_eq!(
            saved.backups[0].retention.keep_last,
            Some(KeepCount::Count(5)),
            "[{label}] expected keep_last = Some(Count(5)) persisted in config.toml"
        );
        assert_eq!(
            count_repo_snapshots(restic, &repo_dir, &repo_pw, &cache_dir),
            3,
            "[{label}] retention preview/save must NOT delete snapshots from the repo"
        );
    };

    // 1. Wide / EN
    {
        let _daemon = fx.daemon("en");
        ctx.run(
            "m2-retention-editor-preview-confirm",
            "wide",
            "en",
            "target/smoke-artifacts/m2-retention-wide-en.png",
            &[],
        );
    }
    assert_retention_saved_and_intact("wide/en");

    // 2. Reset config to RetentionPolicy::standard() and run Wide / DE
    save_config(&iso.config_path(), &initial_cfg).expect("reset config for wide/de");
    {
        let _daemon = fx.daemon("de");
        ctx.run(
            "m2-retention-editor-preview-confirm",
            "wide",
            "de",
            "target/smoke-artifacts/m2-retention-wide-de.png",
            &[],
        );
    }
    assert_retention_saved_and_intact("wide/de");

    // 3. Reset config to RetentionPolicy::standard() and run Narrow / EN
    save_config(&iso.config_path(), &initial_cfg).expect("reset config for narrow/en");
    {
        let _daemon = fx.daemon("en");
        ctx.run(
            "m2-retention-editor-preview-confirm",
            "narrow",
            "en",
            "target/smoke-artifacts/m2-retention-narrow-en.png",
            &[],
        );
    }
    assert_retention_saved_and_intact("narrow/en");
}

#[test]
fn m2_gui_smoke_keyboard_navigation() {
    let fx = SmokeFixture::new("kbd-nav", common::latest_restic());
    let (restic, iso) = (&fx.restic, &fx.iso);
    let repo_dir = iso.root.join("repo-kbd");
    let src_dir = iso.root.join("src-kbd");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("doc.txt"), b"keyboard nav").expect("write doc.txt");

    let (dest, _repo_pw) = local_destination(&fx, "Kbd Vault", &repo_dir);
    let backup = Backup::new("Documents", Source::folder(&src_dir), dest.id.clone()).expect("bk");

    let cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: restic.clone(),
        },
        expert_mode: true,
        destinations: vec![dest],
        backups: vec![backup],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save kbd nav config");

    let _daemon = fx.daemon("en");

    fx.ctx().run(
        "m2-keyboard-nav",
        "wide",
        "en",
        "target/smoke-artifacts/m2-keyboard-nav-en.png",
        &[],
    );
}

#[test]
fn m2_gui_smoke_exit3_unreadable_and_add_exclude() {
    let fx = SmokeFixture::new("exit3", common::latest_restic());
    let (restic, iso) = (&fx.restic, &fx.iso);
    let repo_dir = iso.root.join("repo-exit3");
    let src_dir = iso.root.join("src-exit3");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("normal.txt"), b"ok").expect("write normal.txt");
    let secret_file = src_dir.join("unreadable_secret.key");
    fs::write(&secret_file, b"secret").expect("write unreadable_secret.key");
    let secret_file_str = secret_file.to_string_lossy().into_owned();

    let (dest, _repo_pw) = local_destination(&fx, "Exit3 Vault", &repo_dir);
    let backup = Backup::new("Documents", Source::folder(&src_dir), dest.id.clone()).expect("bk");

    // Preseed RunHistory with an Exit-3 Warning run whose log contains the restic JSON error
    // for `secret_file_str`, causing DaemonService to expose `unreadable_files` and
    // `error_action = Some("add_to_excludes")`.
    let clock = FakeClock::new(datetime!(2026-11-16 14:00:00 UTC));
    let mut run = Run::new_backup(&clock, &backup).expect("run");
    let log_dir = iso.state_kestic_dir().join("logs");
    fs::create_dir_all(&log_dir).expect("mkdir logs");
    let log_path = log_dir.join(format!("{}.log", run.id));
    let err_line = serde_json::json!({
        "message_type": "error",
        "message": "open unreadable_secret.key: permission denied",
        "during": "archival",
        "item": secret_file_str,
    })
    .to_string();
    fs::write(&log_path, format!("{err_line}\n")).expect("write exit 3 log");
    run.log_path = Some(log_path);
    run.finish(&clock, RunStatus::Warning, Some(3));

    let mut history = RunHistory::open(&iso.state_kestic_dir()).expect("open history");
    history.upsert_run(&run, &[]).expect("upsert warning run");

    let cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: restic.clone(),
        },
        destinations: vec![dest],
        backups: vec![backup.clone()],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save exit3 config");

    let _daemon = fx.daemon("en");

    fx.ctx().run(
        "m2-exit3-unreadable-and-add-exclude",
        "wide",
        "en",
        "target/smoke-artifacts/m2-exit3-run-detail-en.png",
        &[],
    );

    let saved = load_config(&iso.config_path()).expect("reload exit3 config");
    assert!(
        saved.backups[0].excludes.lines.contains(&secret_file_str),
        "expected unreadable file '{secret_file_str}' in saved excludes.lines: {:?}",
        saved.backups[0].excludes.lines
    );

    let excl_path = exclude_file_path(&iso.state_kestic_dir(), &backup.id);
    let excl_contents = fs::read_to_string(&excl_path).expect("read exclude file");
    assert!(
        excl_contents.contains(&secret_file_str),
        "expected unreadable file '{secret_file_str}' in {}: {excl_contents}",
        excl_path.display()
    );
}

#[test]
fn m2_gui_smoke_drive_unmounted_nested_and_full_config_en_de_narrow() {
    let fx = SmokeFixture::new("full-config", common::latest_restic());
    let (restic, iso) = (&fx.restic, &fx.iso);
    let repo_dir = iso.root.join("repo-full");
    let src_dir = iso.root.join("src-home");
    let bilder_dir = src_dir.join("Bilder");
    fs::create_dir_all(&bilder_dir).expect("mkdir Bilder");
    fs::write(bilder_dir.join("photo.jpg"), b"img").expect("write photo.jpg");

    let (dest, repo_pw) = local_destination(&fx, "NAS Backup", &repo_dir);

    // Backup 0: Drive pointing to an unmounted path not present in /proc/self/mountinfo
    let nonce = kestic_core::model::RunId::generate()
        .map_or_else(|_| "0".into(), |r| r.as_str().to_string());
    let missing_drive_path = PathBuf::from(format!(
        "/tmp/kestic-missing-drive-mount-{}-{nonce}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&missing_drive_path);
    let drive_backup = Backup::new(
        "External Drive",
        Source::drive(&missing_drive_path),
        dest.id.clone(),
    )
    .expect("drive backup");

    // Backup 1: Nested source `src_dir.join("Bilder")` on same destination with `*.tmp` rule
    let mut nested_backup = Backup::new("Pictures", Source::folder(&bilder_dir), dest.id.clone())
        .expect("nested backup");
    nested_backup.excludes = ExcludeRules {
        lines: vec!["*.tmp".to_string()],
        ..ExcludeRules::default()
    };

    // Backup 2: Parent source `src_dir` on the SAME destination (triggers nested warning on Backup 1)
    let parent_backup =
        Backup::new("Home Data", Source::folder(&src_dir), dest.id.clone()).expect("parent backup");

    let cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: restic.clone(),
        },
        destinations: vec![dest],
        backups: vec![drive_backup, nested_backup, parent_backup],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save full-config fixture");

    let ctx = fx.ctx();
    let smoke_env = [("KESTIC_SMOKE_REPO_PASSWORD", repo_pw.as_str())];

    // 1. Wide / EN
    {
        let _daemon = fx.daemon("en");
        ctx.run_with_env(
            "m2-drive-unmounted-nested-and-full-config",
            "wide",
            "en",
            "target/smoke-artifacts/m2-full-config-wide-en.png",
            &[],
            &smoke_env,
        );
    }

    // 2. Wide / DE
    {
        let _daemon = fx.daemon("de");
        ctx.run_with_env(
            "m2-drive-unmounted-nested-and-full-config",
            "wide",
            "de",
            "target/smoke-artifacts/m2-full-config-wide-de.png",
            &[],
            &smoke_env,
        );
    }

    // 3. Narrow / EN
    {
        let _daemon = fx.daemon("en");
        ctx.run_with_env(
            "m2-drive-unmounted-nested-and-full-config",
            "narrow",
            "en",
            "target/smoke-artifacts/m2-full-config-narrow-en.png",
            &[],
            &smoke_env,
        );
    }
    // 4. + 5. Mixed languages: the DTOs carry only codes, so the GUI shows its own language
    // whatever language the daemon runs in. The smoke
    // checks the unavailable reason («drive not mounted» / «Laufwerk nicht eingehängt») in
    // the GUI language.
    for (daemon_lang, gui_lang) in [("de", "en"), ("en", "de")] {
        let _daemon = fx.daemon(daemon_lang);
        ctx.run_with_env(
            "m2-drive-unmounted-nested-and-full-config",
            "wide",
            gui_lang,
            &format!(
                "target/smoke-artifacts/m2-full-config-wide-{gui_lang}-daemon-{daemon_lang}.png"
            ),
            &[],
            &smoke_env,
        );
    }
}
