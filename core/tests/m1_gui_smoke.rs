// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Comprehensive offscreen GUI smoke suite for Milestone M1 (`M1-S13`–`M1-S23`, `NFR-08`).
//!
//! Each scenario runs completely isolated with its own `TempDir` XDG directories,
//! private `dbus-daemon`, optional `KESTIC_SECRET_FILE`, real `kestic daemon`,
//! and `kestic --smoke-m1 <scenario>`, capturing PNG screenshots into
//! `target/smoke-artifacts/m1-*.png` for visual inspection.

mod common;

use kestic_core::backup::execute_backup;
use kestic_core::clock::FakeClock;
use kestic_core::config::{AppConfig, save_config};
use kestic_core::destination::{DestinationSetupMode, setup_destination};
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, Destination, DestinationKind, ResticProvider, Run, RunStatus, Source,
};
use kestic_core::provider::check_binary_version;
use kestic_core::restic::ResticRuntime;
use kestic_core::secrets::{FileSecretStore, SecretStore, SecretValue};
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
            "kestic-m1-gui-{tag}-{}-{nonce}",
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
        let conf_path =
            env::temp_dir().join(format!("kestic-dbus-{}-{nonce}.conf", std::process::id()));
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
        self.run_with_env(scenario, layout, lang, screenshot_rel, extra_args, &[]);
    }

    fn run_with_env(
        &self,
        scenario: &str,
        layout: &str,
        lang: &str,
        screenshot_rel: &str,
        extra_args: &[&str],
        extra_env: &[(&str, &str)],
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

        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        if self.use_secret_file {
            cmd.env("KESTIC_SECRET_FILE", &self.iso.secret_file);
        } else {
            cmd.env_remove("KESTIC_SECRET_FILE");
        }

        let output = cmd.output().expect("run kestic --smoke-m1");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "M1 smoke `{scenario}` ({layout}/{lang}) failed (status={:?}):\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            stderr
        );
        assert!(
            stderr.trim().is_empty(),
            "M1 smoke `{scenario}` ({layout}/{lang}) emitted QML/Qt warnings:\n{stderr}"
        );

        let png = fs::read(&screenshot).expect("read captured M1 PNG screenshot");
        assert!(
            png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 2048,
            "M1 screenshot {} must be a valid non-trivial PNG (got {} bytes)",
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

fn setup_populated_fixture(
    iso: &IsolatedEnv,
    restic: &Path,
) -> (Destination, Backup, Run, PathBuf) {
    let repo_dir = iso.root.join("repo-primary");
    let src_dir = iso.root.join("source-docs");
    fs::create_dir_all(&repo_dir).expect("mkdir repo");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("notes.txt"), b"Hello Kestic M1!").expect("write sample doc");

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
        "Local Vault",
        DestinationKind::Local {
            path: repo_dir.clone(),
        },
    )
    .expect("dest");
    setup_destination(
        &dest,
        DestinationSetupMode::InitNew,
        &SecretValue::from_str_secret("m1-smoke-pw"),
        None,
        &runtime,
    )
    .expect("init repo");

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

    let mut run = Run::new_backup(&clock, &backup).expect("run");
    run.log_path = Some(iso.state_kestic_dir().join("logs").join("sample-run.log"));
    if let Some(ref lp) = run.log_path {
        fs::create_dir_all(lp.parent().expect("log parent")).expect("mkdir logs");
        fs::write(lp, "[info] Snapshot created successfully.\n").expect("write log");
    }
    run.stats = res.stats;
    run.commands.push(res.command_record);
    run.finish(&clock, RunStatus::Ok, Some(0));

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
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("welcome", Some(&restic));
    FileSecretStore::new(&iso.secret_file)
        .set_secret(
            "kestic_health_probe",
            "probe",
            &SecretValue::from_str_secret("ok"),
        )
        .expect("seed probe");
    let dbus = PrivateDbusGuard::start();
    let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");

    let ctx = SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    };

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
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();

    // 1. Daemon unreachable (--no-autostart, no daemon running)
    {
        let iso = IsolatedEnv::create("blk-daemon", Some(&restic));
        let dbus = PrivateDbusGuard::start();
        SmokeContext {
            kestic_bin: &kestic,
            iso: &iso,
            dbus_addr: &dbus.address,
            use_secret_file: true,
        }
        .run(
            "blocking-daemon_unreachable",
            "wide",
            "en",
            "target/smoke-artifacts/m1-blocking-daemon.png",
            &["--no-autostart"],
        );
    }

    // 2. Secret store locked (no KESTIC_SECRET_FILE and no org.freedesktop.secrets on private bus)
    {
        let iso = IsolatedEnv::create("blk-secret", Some(&restic));
        let dbus = PrivateDbusGuard::start();
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, false, "en");
        SmokeContext {
            kestic_bin: &kestic,
            iso: &iso,
            dbus_addr: &dbus.address,
            use_secret_file: false,
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
        FileSecretStore::new(&iso.secret_file)
            .set_secret(
                "kestic_health_probe",
                "probe",
                &SecretValue::from_str_secret("ok"),
            )
            .expect("seed probe");
        let cfg = AppConfig {
            restic_provider: ResticProvider::Custom {
                path: PathBuf::from("/nonexistent/kestic-missing-restic"),
            },
            ..AppConfig::default()
        };
        save_config(&iso.config_path(), &cfg).expect("save invalid restic cfg");
        let dbus = PrivateDbusGuard::start();
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");

        let ctx = SmokeContext {
            kestic_bin: &kestic,
            iso: &iso,
            dbus_addr: &dbus.address,
            use_secret_file: true,
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
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("populated", Some(&restic));
    setup_populated_fixture(&iso, &restic);

    let dbus = PrivateDbusGuard::start();
    let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");

    let ctx = SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    };
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
fn m1_gui_smoke_snapshots_previous_and_manual_run_blocked() {
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();

    // 1. Previous destination snapshots after reassigning backup
    {
        let iso = IsolatedEnv::create("snap-prev", Some(&restic));
        let (dest1, mut backup, _run, _) = setup_populated_fixture(&iso, &restic);

        let repo2 = iso.root.join("repo-secondary");
        fs::create_dir_all(&repo2).expect("mkdir repo2");
        let dest2 = Destination::new(
            "Archive Drive",
            DestinationKind::Local {
                path: repo2.clone(),
            },
        )
        .expect("dest2");
        let store = FileSecretStore::new(&iso.secret_file);
        let helper = common::write_secret_get_helper(&iso.root, &iso.secret_file);
        let resolved = check_binary_version(&restic).expect("resolve restic");
        let clock = FakeClock::new(datetime!(2026-11-16 12:30:00 UTC));
        let cache_dir = common::restic_cache_dir(&iso.root);
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
            cache_dir: Some(&cache_dir),
        };
        setup_destination(
            &dest2,
            DestinationSetupMode::InitNew,
            &SecretValue::from_str_secret("m1-smoke-pw2"),
            None,
            &runtime,
        )
        .expect("init repo2");

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

        let dbus = PrivateDbusGuard::start();
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");
        SmokeContext {
            kestic_bin: &kestic,
            iso: &iso,
            dbus_addr: &dbus.address,
            use_secret_file: true,
        }
        .run(
            "snapshots-previous",
            "wide",
            "en",
            "target/smoke-artifacts/m1-snapshots-previous.png",
            &[],
        );
    }

    // 2. ManualRunBlocked inline banner when source directory disappears
    {
        let iso = IsolatedEnv::create("run-blocked", Some(&restic));
        let repo_dir = iso.root.join("repo");
        let missing_src = iso.root.join("missing-source-folder");
        fs::create_dir_all(&repo_dir).expect("mkdir repo");
        fs::create_dir_all(&missing_src).expect("mkdir tmp src");

        let store = FileSecretStore::new(&iso.secret_file);
        let helper = common::write_secret_get_helper(&iso.root, &iso.secret_file);
        let resolved = check_binary_version(&restic).expect("resolve restic");
        let clock = FakeClock::new(datetime!(2026-11-16 13:00:00 UTC));
        let dest =
            Destination::new("Local", DestinationKind::Local { path: repo_dir }).expect("dest");
        let cache_dir = common::restic_cache_dir(&iso.root);
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
            cache_dir: Some(&cache_dir),
        };
        setup_destination(
            &dest,
            DestinationSetupMode::InitNew,
            &SecretValue::from_str_secret("m1-smoke-pw"),
            None,
            &runtime,
        )
        .expect("init repo");
        let backup =
            Backup::new("MissingSrc", Source::folder(&missing_src), dest.id.clone()).expect("bk");
        fs::remove_dir_all(&missing_src).expect("remove source dir so manual start is blocked");

        let cfg = AppConfig {
            restic_provider: ResticProvider::Custom { path: restic },
            destinations: vec![dest],
            backups: vec![backup],
            ..AppConfig::default()
        };
        save_config(&iso.config_path(), &cfg).expect("save cfg");

        let dbus = PrivateDbusGuard::start();
        let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");
        SmokeContext {
            kestic_bin: &kestic,
            iso: &iso,
            dbus_addr: &dbus.address,
            use_secret_file: true,
        }
        .run(
            "manual-run-blocked",
            "wide",
            "en",
            "target/smoke-artifacts/m1-manual-run-blocked.png",
            &[],
        );
    }
}

#[test]
fn m1_gui_e2e_create_destination_and_backup_with_real_restic() {
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("e2e-wizard", Some(&restic));
    FileSecretStore::new(&iso.secret_file)
        .set_secret(
            "kestic_health_probe",
            "probe",
            &SecretValue::from_str_secret("ok"),
        )
        .expect("seed probe");

    let repo_dir = iso.root.join("e2e-repo");
    let src_dir = iso.root.join("e2e-source");
    fs::create_dir_all(&repo_dir).expect("mkdir repo_dir");
    fs::create_dir_all(&src_dir).expect("mkdir src_dir");
    fs::write(src_dir.join("hello.txt"), b"hello from kestic e2e\n").expect("write test file");

    let dbus = PrivateDbusGuard::start();
    let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");

    let repo_str = repo_dir.to_string_lossy().into_owned();
    let src_str = src_dir.to_string_lossy().into_owned();
    SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    }
    .run_with_env(
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

    let cfg = kestic_core::config::load_config(&iso.config_path()).expect("load saved config");
    assert_eq!(cfg.destinations.len(), 1, "expected 1 saved destination");
    assert_eq!(cfg.destinations[0].name, "E2E Local Repo");
    assert_eq!(cfg.backups.len(), 1, "expected 1 saved backup");

    let cache_dir = common::restic_cache_dir(&iso.root);
    let snap_out = Command::new(&restic)
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
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("e2e-first-backup", Some(&restic));
    FileSecretStore::new(&iso.secret_file)
        .set_secret(
            "kestic_health_probe",
            "probe",
            &SecretValue::from_str_secret("ok"),
        )
        .expect("seed probe");

    let repo_dir = iso.root.join("first-repo");
    let src_dir = iso.root.join("first-source");
    fs::create_dir_all(&repo_dir).expect("mkdir repo_dir");
    fs::create_dir_all(&src_dir).expect("mkdir src_dir");
    fs::write(src_dir.join("first.txt"), b"hello first backup\n").expect("write test file");

    let dbus = PrivateDbusGuard::start();
    let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");

    let repo_str = repo_dir.to_string_lossy().into_owned();
    let src_str = src_dir.to_string_lossy().into_owned();
    SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    }
    .run_with_env(
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

    let cfg = kestic_core::config::load_config(&iso.config_path()).expect("load saved config");
    assert_eq!(cfg.destinations.len(), 1, "expected 1 saved destination");
    assert_eq!(cfg.destinations[0].name, "First Local Repo");
    assert_eq!(cfg.backups.len(), 1, "expected 1 saved backup");

    let cache_dir = common::restic_cache_dir(&iso.root);
    let snap_out = Command::new(&restic)
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
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("e2e-edit-reinit", Some(&restic));
    FileSecretStore::new(&iso.secret_file)
        .set_secret(
            "kestic_health_probe",
            "probe",
            &SecretValue::from_str_secret("ok"),
        )
        .expect("seed probe");

    let (_dest, _backup, _run, repo_dir) = setup_populated_fixture(&iso, &restic);

    // Delete repository directory contents and recreate it empty (simulating Dolphin / rm + mkdir).
    fs::remove_dir_all(&repo_dir).expect("remove repo_dir");
    fs::create_dir_all(&repo_dir).expect("recreate empty repo_dir");
    assert!(
        !repo_dir.join("config").exists(),
        "repo config must be absent before UI edit re-init"
    );

    let dbus = PrivateDbusGuard::start();
    let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");

    SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    }
    .run(
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
    let kestic = find_kestic_binary(workspace_root());
    let restic = restic_bin();
    let iso = IsolatedEnv::create("kbd-nav", Some(&restic));
    setup_populated_fixture(&iso, &restic);

    let dbus = PrivateDbusGuard::start();
    let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, "en");

    SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    }
    .run(
        "keyboard-navigation",
        "wide",
        "en",
        "target/smoke-artifacts/m1-keyboard-navigation.png",
        &[],
    );
}
