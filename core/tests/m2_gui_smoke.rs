// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Comprehensive offscreen GUI smoke suite for Milestone M2 (`M2-S01`–`M2-S11`).
//!
//! Each scenario runs completely isolated with its own temporary XDG directories,
//! private `dbus-daemon`, `KESTIC_SECRET_FILE`, real `kestic daemon`, and
//! `kestic --smoke-m1 m2-...`, capturing PNG screenshots into
//! `target/smoke-artifacts/m2-*.png` and verifying persisted config, exclude
//! files, and real restic repository state.

mod common;

use kestic_core::backup::{exclude_file_path, execute_backup};
use kestic_core::clock::FakeClock;
use kestic_core::config::{AppConfig, load_config, save_config};
use kestic_core::destination::{DestinationSetupMode, setup_destination};
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, Destination, DestinationKind, ExcludeRules, KeepCount, ResticProvider, RetentionPolicy,
    Run, RunStatus, Source,
};
use kestic_core::provider::check_binary_version;
use kestic_core::restic::ResticRuntime;
use kestic_core::secrets::{FileSecretStore, SecretValue};
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use time::macros::datetime;

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has workspace root parent")
}

fn find_target_dir(root: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
}

fn find_kestic_binary(root: &Path) -> PathBuf {
    let target_dir = find_target_dir(root);
    for profile in ["debug", "release"] {
        let candidate = target_dir.join(profile).join("kestic");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("kestic binary not found under {}", target_dir.display());
}

struct IsolatedEnv {
    root: PathBuf,
    home: PathBuf,
    cfg: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    state: PathBuf,
    run: PathBuf,
    bin_dir: PathBuf,
    secret_file: PathBuf,
}

impl IsolatedEnv {
    fn create(tag: &str, restic_bin: Option<&Path>) -> Self {
        let nonce = kestic_core::model::RunId::generate()
            .map(|r| r.as_str().to_string())
            .unwrap_or_else(|_| "0".into());
        let root = env::temp_dir().join(format!(
            "kestic-m2-gui-{tag}-{}-{nonce}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let cfg = root.join("cfg");
        let data = root.join("data");
        let cache = root.join("cache");
        let state = root.join("state");
        let run = root.join("run");
        let bin_dir = root.join("bin");
        for d in [&home, &cfg, &data, &cache, &state, &run, &bin_dir] {
            fs::create_dir_all(d).expect("create isolated dir");
        }
        fs::create_dir_all(cfg.join("kestic")).expect("create config dir");
        fs::create_dir_all(state.join("kestic")).expect("create state dir");
        fs::set_permissions(&run, fs::Permissions::from_mode(0o700)).expect("chmod 0700 run dir");

        if let Some(restic) = restic_bin {
            std::os::unix::fs::symlink(restic, bin_dir.join("restic")).expect("symlink restic");
        }

        // Copy compiled German translation catalog if present so EN/DE smoke both work.
        let built_mo = find_target_dir(workspace_root()).join("locale/de/LC_MESSAGES/kestic.mo");
        if built_mo.is_file() {
            let mo_dest_dir = data.join("locale/de/LC_MESSAGES");
            fs::create_dir_all(&mo_dest_dir).expect("mkdir locale");
            fs::copy(&built_mo, mo_dest_dir.join("kestic.mo")).expect("copy kestic.mo");
        }

        let secret_file = root.join("secrets.hex");
        Self {
            root,
            home,
            cfg,
            data,
            cache,
            state,
            run,
            bin_dir,
            secret_file,
        }
    }

    fn config_path(&self) -> PathBuf {
        self.cfg.join("kestic").join("config.toml")
    }

    fn state_kestic_dir(&self) -> PathBuf {
        self.state.join("kestic")
    }

    fn path_env(&self) -> String {
        let base = env::var("PATH").unwrap_or_default();
        format!("{}:{base}", self.bin_dir.display())
    }
}

impl Drop for IsolatedEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct PrivateDbusGuard {
    address: String,
    pid: u32,
    conf_path: PathBuf,
}

impl PrivateDbusGuard {
    fn start() -> Self {
        let nonce = kestic_core::model::RunId::generate()
            .map(|r| r.as_str().to_string())
            .unwrap_or_else(|_| "0".into());
        let conf_path = env::temp_dir().join(format!(
            "kestic-m2-dbus-{}-{nonce}.conf",
            std::process::id()
        ));
        let conf = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <keep_umask/>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;
        fs::write(&conf_path, conf).expect("write private dbus config");

        let out = Command::new("dbus-daemon")
            .arg("--config-file")
            .arg(&conf_path)
            .args(["--fork", "--print-address=1", "--print-pid=1"])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn private dbus-daemon");
        assert!(
            out.status.success(),
            "dbus-daemon failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).expect("utf8 dbus output");
        let mut lines = stdout.lines();
        let address = lines.next().expect("dbus address").trim().to_string();
        let pid = lines
            .next()
            .expect("dbus pid")
            .trim()
            .parse::<u32>()
            .expect("parse dbus pid");
        Self {
            address,
            pid,
            conf_path,
        }
    }
}

impl Drop for PrivateDbusGuard {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::remove_file(&self.conf_path);
    }
}

struct DaemonProcessGuard {
    child: Child,
}

impl DaemonProcessGuard {
    fn spawn(
        kestic_bin: &Path,
        iso: &IsolatedEnv,
        dbus_addr: &str,
        use_secret_file: bool,
        lang: &str,
    ) -> Self {
        let ready_file = iso.root.join("daemon.ready");
        let _ = fs::remove_file(&ready_file);
        let mut cmd = Command::new(kestic_bin);
        cmd.args([
            "daemon",
            "--no-job-view",
            "--no-portal-inhibit",
            "--suppress-ui-spawn",
            "--config-path",
        ])
        .arg(iso.config_path())
        .arg("--state-dir")
        .arg(iso.state_kestic_dir())
        .arg("--ready-file")
        .arg(&ready_file)
        .env("HOME", &iso.home)
        .env("XDG_CONFIG_HOME", &iso.cfg)
        .env("XDG_DATA_HOME", &iso.data)
        .env("XDG_CACHE_HOME", &iso.cache)
        .env("XDG_STATE_HOME", &iso.state)
        .env("XDG_RUNTIME_DIR", &iso.run)
        .env("DBUS_SESSION_BUS_ADDRESS", dbus_addr)
        .env("PATH", iso.path_env())
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", "en_US.UTF-8")
        .env("LANGUAGE", lang)
        .env("QT_QPA_PLATFORM", "offscreen")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

        if use_secret_file {
            cmd.env("KESTIC_SECRET_FILE", &iso.secret_file);
        } else {
            cmd.env_remove("KESTIC_SECRET_FILE");
        }

        let mut child = cmd.spawn().expect("spawn kestic daemon");
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            if ready_file.is_file() {
                return Self { child };
            }
            if let Some(status) = child.try_wait().expect("try_wait daemon") {
                let mut err_buf = String::new();
                if let Some(mut se) = child.stderr.take() {
                    use std::io::Read;
                    let _ = se.read_to_string(&mut err_buf);
                }
                panic!("kestic daemon exited early ({status}): {err_buf}");
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("timed out waiting for kestic daemon ready file");
    }
}

impl Drop for DaemonProcessGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct SmokeContext<'a> {
    kestic_bin: &'a Path,
    iso: &'a IsolatedEnv,
    dbus_addr: &'a str,
    use_secret_file: bool,
}

impl<'a> SmokeContext<'a> {
    fn run(
        &self,
        scenario: &str,
        layout: &str,
        lang: &str,
        screenshot_rel: &str,
        extra_args: &[&str],
    ) {
        let root = workspace_root();
        let screenshot = root.join(screenshot_rel);
        if let Some(parent) = screenshot.parent() {
            fs::create_dir_all(parent).expect("create screenshot dir");
        }
        if screenshot.exists() {
            fs::remove_file(&screenshot).expect("remove stale screenshot");
        }

        let mut cmd = Command::new(self.kestic_bin);
        cmd.args([
            "--smoke-m1",
            scenario,
            "--smoke-layout",
            layout,
            "--smoke-lang",
            lang,
            "--screenshot",
        ])
        .arg(&screenshot)
        .args(extra_args)
        .env("HOME", &self.iso.home)
        .env("XDG_CONFIG_HOME", &self.iso.cfg)
        .env("XDG_DATA_HOME", &self.iso.data)
        .env("XDG_CACHE_HOME", &self.iso.cache)
        .env("XDG_STATE_HOME", &self.iso.state)
        .env("XDG_RUNTIME_DIR", &self.iso.run)
        .env("DBUS_SESSION_BUS_ADDRESS", self.dbus_addr)
        .env("PATH", self.iso.path_env())
        .env("QT_QPA_PLATFORM", "offscreen")
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", "en_US.UTF-8")
        .env("LANGUAGE", lang);

        if self.use_secret_file {
            cmd.env("KESTIC_SECRET_FILE", &self.iso.secret_file);
        } else {
            cmd.env_remove("KESTIC_SECRET_FILE");
        }

        let output = cmd.output().expect("run kestic --smoke-m1");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "M2 smoke `{scenario}` ({layout}/{lang}) failed (status={}):\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            stderr
        );
        assert!(
            stderr.trim().is_empty(),
            "M2 smoke `{scenario}` ({layout}/{lang}) emitted QML/Qt warnings:\n{stderr}"
        );

        let png = fs::read(&screenshot).expect("read captured M2 PNG screenshot");
        assert!(
            png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 2048,
            "M2 screenshot {} must be a valid non-trivial PNG (got {} bytes)",
            screenshot.display(),
            png.len()
        );
    }
}

fn restic_bin() -> PathBuf {
    let (_, path) = common::restic_test_binaries()
        .into_iter()
        .last()
        .expect("restic binary");
    path
}

fn init_local_destination(
    iso: &IsolatedEnv,
    restic: &Path,
    name: &str,
    repo_dir: &Path,
    password: &str,
) -> Destination {
    fs::create_dir_all(repo_dir).expect("mkdir repo");
    let store = FileSecretStore::new(&iso.secret_file);
    let helper = common::write_secret_get_helper(&iso.root, &iso.secret_file);
    let resolved = check_binary_version(restic).expect("resolve restic");
    let clock = FakeClock::new(datetime!(2026-11-16 12:00:00 UTC));
    let cache_dir = common::restic_cache_dir(&iso.root);
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
        cache_dir: Some(&cache_dir),
    };

    let dest = Destination::new(
        name,
        DestinationKind::Local {
            path: repo_dir.to_path_buf(),
        },
    )
    .expect("dest");
    setup_destination(
        &dest,
        DestinationSetupMode::InitNew,
        &SecretValue::from_str_secret(password),
        None,
        &runtime,
    )
    .expect("init repo");
    dest
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

#[test]
fn m2_gui_smoke_excludes_simple_expert_and_lint_wide_narrow_en_de() {
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("excludes", Some(&restic));
    let repo_dir = iso.root.join("repo-excludes");
    let src_dir = iso.root.join("source-excludes");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("keep.txt"), b"keep").expect("write keep");

    let dest = init_local_destination(&iso, &restic, "Local Vault", &repo_dir, "m2-smoke-pw");
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
    backup.excludes = initial_excludes.clone();

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

    let dbus = PrivateDbusGuard::start();
    let ctx = SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    };

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
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");
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
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "de");
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
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");
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
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("dry-run", Some(&restic));
    let repo_dir = iso.root.join("repo-dry-run");
    let src_dir = iso.root.join("src-dry-run");
    fs::create_dir_all(src_dir.join("sub")).expect("mkdir src/sub");
    fs::write(src_dir.join("notes.txt"), b"Hello dry-run preview!").expect("write notes.txt");
    fs::write(src_dir.join("sub/report.md"), b"# Report\nDry run").expect("write report.md");

    let dest = init_local_destination(&iso, &restic, "DryRun Vault", &repo_dir, "m2-smoke-pw");
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

    let dbus = PrivateDbusGuard::start();
    let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");

    SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    }
    .run(
        "m2-dry-run-preview",
        "wide",
        "en",
        "target/smoke-artifacts/m2-dry-run-preview-en.png",
        &[],
    );

    assert_eq!(
        count_repo_snapshots(
            &restic,
            &repo_dir,
            "m2-smoke-pw",
            &common::restic_cache_dir(&iso.root)
        ),
        0,
        "dry-run preview must NOT create any real restic snapshot"
    );
}

#[test]
fn m2_gui_smoke_retention_editor_preview_confirm_en_de_narrow() {
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("retention", Some(&restic));
    let cache_dir = common::restic_cache_dir(&iso.root);
    let repo_dir = iso.root.join("repo-retention");
    let src_dir = iso.root.join("src-retention");
    fs::create_dir_all(&src_dir).expect("mkdir src");

    let dest = init_local_destination(&iso, &restic, "Retention Vault", &repo_dir, "m2-smoke-pw");
    let mut backup =
        Backup::new("Projects", Source::folder(&src_dir), dest.id.clone()).expect("bk");
    backup.retention = RetentionPolicy::standard();

    let store = FileSecretStore::new(&iso.secret_file);
    let helper = common::write_secret_get_helper(&iso.root, &iso.secret_file);
    let resolved = check_binary_version(&restic).expect("resolve restic");

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
        let clock = FakeClock::new(ts);
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
            cache_dir: Some(&cache_dir),
        };
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
        count_repo_snapshots(&restic, &repo_dir, "m2-smoke-pw", &cache_dir),
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

    let dbus = PrivateDbusGuard::start();
    let ctx = SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    };

    let assert_retention_saved_and_intact = |label: &str| {
        let saved = load_config(&iso.config_path()).expect("reload retention config");
        assert_eq!(
            saved.backups[0].retention.keep_last,
            Some(KeepCount::Count(5)),
            "[{label}] expected keep_last = Some(Count(5)) persisted in config.toml"
        );
        assert_eq!(
            count_repo_snapshots(&restic, &repo_dir, "m2-smoke-pw", &cache_dir),
            3,
            "[{label}] retention preview/save must NOT delete snapshots from the repo"
        );
    };

    // 1. Wide / EN
    {
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");
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
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "de");
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
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");
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
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("kbd-nav", Some(&restic));
    let repo_dir = iso.root.join("repo-kbd");
    let src_dir = iso.root.join("src-kbd");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("doc.txt"), b"keyboard nav").expect("write doc.txt");

    let dest = init_local_destination(&iso, &restic, "Kbd Vault", &repo_dir, "m2-smoke-pw");
    let backup = Backup::new("Documents", Source::folder(&src_dir), dest.id.clone()).expect("bk");

    let cfg = AppConfig {
        restic_provider: ResticProvider::Custom { path: restic },
        expert_mode: true,
        destinations: vec![dest],
        backups: vec![backup],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save kbd nav config");

    let dbus = PrivateDbusGuard::start();
    let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");

    SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    }
    .run(
        "m2-keyboard-nav",
        "wide",
        "en",
        "target/smoke-artifacts/m2-keyboard-nav-en.png",
        &[],
    );
}

#[test]
fn m2_gui_smoke_exit3_unreadable_and_add_exclude() {
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("exit3", Some(&restic));
    let repo_dir = iso.root.join("repo-exit3");
    let src_dir = iso.root.join("src-exit3");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("normal.txt"), b"ok").expect("write normal.txt");
    let secret_file = src_dir.join("unreadable_secret.key");
    fs::write(&secret_file, b"secret").expect("write unreadable_secret.key");
    let secret_file_str = secret_file.to_string_lossy().into_owned();

    let dest = init_local_destination(&iso, &restic, "Exit3 Vault", &repo_dir, "m2-smoke-pw");
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
        restic_provider: ResticProvider::Custom { path: restic },
        destinations: vec![dest],
        backups: vec![backup.clone()],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save exit3 config");

    let dbus = PrivateDbusGuard::start();
    let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");

    SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    }
    .run(
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
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("full-config", Some(&restic));
    let repo_dir = iso.root.join("repo-full");
    let src_dir = iso.root.join("src-home");
    let bilder_dir = src_dir.join("Bilder");
    fs::create_dir_all(&bilder_dir).expect("mkdir Bilder");
    fs::write(bilder_dir.join("photo.jpg"), b"img").expect("write photo.jpg");

    let dest = init_local_destination(&iso, &restic, "NAS Backup", &repo_dir, "m2-smoke-pw");

    // Backup 0: Drive pointing to an unmounted path not present in /proc/self/mountinfo
    let nonce = kestic_core::model::RunId::generate()
        .map(|r| r.as_str().to_string())
        .unwrap_or_else(|_| "0".into());
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
        restic_provider: ResticProvider::Custom { path: restic },
        destinations: vec![dest],
        backups: vec![drive_backup, nested_backup, parent_backup],
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save full-config fixture");

    let dbus = PrivateDbusGuard::start();
    let ctx = SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    };

    // 1. Wide / EN
    {
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");
        ctx.run(
            "m2-drive-unmounted-nested-and-full-config",
            "wide",
            "en",
            "target/smoke-artifacts/m2-full-config-wide-en.png",
            &[],
        );
    }

    // 2. Wide / DE
    {
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "de");
        ctx.run(
            "m2-drive-unmounted-nested-and-full-config",
            "wide",
            "de",
            "target/smoke-artifacts/m2-full-config-wide-de.png",
            &[],
        );
    }

    // 3. Narrow / EN
    {
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");
        ctx.run(
            "m2-drive-unmounted-nested-and-full-config",
            "narrow",
            "en",
            "target/smoke-artifacts/m2-full-config-narrow-en.png",
            &[],
        );
    }
    // 4. + 5. Mixed languages (variant A+, decision 2026-11-20): the DTOs carry only codes,
    // so the GUI shows its own language whatever language the daemon runs in. The smoke
    // checks the unavailable reason («drive not mounted» / «Laufwerk nicht eingehängt») in
    // the GUI language.
    for (daemon_lang, gui_lang) in [("de", "en"), ("en", "de")] {
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, daemon_lang);
        ctx.run(
            "m2-drive-unmounted-nested-and-full-config",
            "wide",
            gui_lang,
            &format!(
                "target/smoke-artifacts/m2-full-config-wide-{gui_lang}-daemon-{daemon_lang}.png"
            ),
            &[],
        );
    }
}
