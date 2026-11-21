// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared harness for the offscreen GUI smoke tests: an isolated XDG tree, a private session bus,
//! a real `kestic daemon` and the `kestic --smoke-m1 <scenario>` runner that captures a PNG.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub use crate::bus::PrivateDbusGuard;
use crate::keyring::{HeadlessKeyring, KeyringDirs};
pub use crate::workspace::{find_kestic_binary, workspace_root};

/// Isolated `HOME` and `XDG_*` tree plus a `PATH` directory that may hold a `restic` symlink.
pub struct IsolatedEnv {
    _tmp: Option<tempfile::TempDir>,
    pub root: PathBuf,
    pub home: PathBuf,
    pub cfg: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub run: PathBuf,
    bin_dir: PathBuf,
}

impl IsolatedEnv {
    pub fn create(tag: &str, restic_bin: Option<&Path>) -> Self {
        let tmp = crate::tools::short_tempdir(&format!("kestic-gui-{tag}"));
        let root = tmp.path().to_path_buf();
        Self::build(Some(tmp), root, restic_bin)
    }

    /// Wraps an already isolated tree at `root` (as `run_isolated_test` hands it over) without
    /// owning its lifetime.
    pub fn adopt(root: &Path) -> Self {
        Self::build(None, root.to_path_buf(), None)
    }

    fn build(tmp: Option<tempfile::TempDir>, root: PathBuf, restic_bin: Option<&Path>) -> Self {
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

        crate::tools::install_german_catalog(&data);

        Self {
            _tmp: tmp,
            root,
            home,
            cfg,
            data,
            cache,
            state,
            run,
            bin_dir,
        }
    }

    pub fn config_path(&self) -> PathBuf {
        self.cfg.join("kestic").join("config.toml")
    }

    pub fn state_kestic_dir(&self) -> PathBuf {
        self.state.join("kestic")
    }

    fn path_env(&self) -> String {
        let base = env::var("PATH").unwrap_or_default();
        format!("{}:{base}", self.bin_dir.display())
    }

    /// Starts an unlocked headless keyring on the bus at `dbus_addr`, keeping its state inside
    /// this tree.
    pub fn start_keyring(&self, dbus_addr: &str) -> HeadlessKeyring {
        HeadlessKeyring::start(
            dbus_addr,
            &KeyringDirs {
                home: &self.home,
                data_home: &self.data,
                runtime_dir: &self.run,
            },
        )
    }

    /// Applies the isolation environment shared by the daemon and the GUI process.
    fn apply_env(&self, cmd: &mut Command, dbus_addr: &str, lang: &str) {
        cmd.env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.cfg)
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_RUNTIME_DIR", &self.run)
            .env("DBUS_SESSION_BUS_ADDRESS", dbus_addr)
            .env("PATH", self.path_env())
            .env_remove("LC_ALL")
            .env_remove("LC_MESSAGES")
            .env("LANG", "en_US.UTF-8")
            .env("LANGUAGE", lang)
            .env("QT_QPA_PLATFORM", "offscreen");
    }
}

/// A running `kestic daemon`, killed on drop.
pub struct DaemonProcessGuard {
    child: Child,
}

impl DaemonProcessGuard {
    pub fn spawn(kestic_bin: &Path, iso: &IsolatedEnv, dbus_addr: &str, lang: &str) -> Self {
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
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
        iso.apply_env(&mut cmd, dbus_addr, lang);

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

/// Runs `kestic --smoke-m1 <scenario>` against an isolated tree and checks the outcome.
pub struct SmokeContext<'a> {
    pub kestic_bin: &'a Path,
    pub iso: &'a IsolatedEnv,
    pub dbus_addr: &'a str,
}

impl SmokeContext<'_> {
    pub fn run(
        &self,
        scenario: &str,
        layout: &str,
        lang: &str,
        screenshot_rel: &str,
        extra_args: &[&str],
    ) {
        self.run_with_env(scenario, layout, lang, screenshot_rel, extra_args, &[]);
    }

    pub fn run_with_env(
        &self,
        scenario: &str,
        layout: &str,
        lang: &str,
        screenshot_rel: &str,
        extra_args: &[&str],
        extra_env: &[(&str, &str)],
    ) {
        let screenshot = workspace_root().join(screenshot_rel);
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
        .args(extra_args);
        self.iso.apply_env(&mut cmd, self.dbus_addr, lang);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        let output = cmd.output().expect("run kestic --smoke-m1");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "smoke `{scenario}` ({layout}/{lang}) failed (status={}):\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            stderr
        );
        assert!(
            stderr.trim().is_empty(),
            "smoke `{scenario}` ({layout}/{lang}) emitted QML/Qt warnings:\n{stderr}"
        );

        let png = fs::read(&screenshot).expect("read captured PNG screenshot");
        assert!(
            png.starts_with(b"\x89PNG\r\n\x1a\n"),
            "screenshot {} must be a PNG",
            screenshot.display()
        );
        assert!(
            png.len() > 2048,
            "screenshot {} must be non-trivial (got {} bytes)",
            screenshot.display(),
            png.len()
        );
    }
}

/// Everything a smoke scenario needs before it seeds its own fixture: the `kestic` and latest
/// `restic` binaries, an isolated tree with `restic` on its `PATH`, and a private session bus
/// with an unlocked headless keyring.
pub struct SmokeFixture {
    pub kestic: PathBuf,
    pub restic: PathBuf,
    pub keyring: HeadlessKeyring,
    pub dbus: PrivateDbusGuard,
    pub iso: IsolatedEnv,
}

impl SmokeFixture {
    pub fn new(tag: &str, restic: PathBuf) -> Self {
        let iso = IsolatedEnv::create(tag, Some(&restic));
        let dbus = PrivateDbusGuard::start();
        let keyring = iso.start_keyring(&dbus.address);
        Self {
            kestic: find_kestic_binary(),
            restic,
            iso,
            keyring,
            dbus,
        }
    }

    /// Starts a daemon with the given UI language.
    pub fn daemon(&self, lang: &str) -> DaemonProcessGuard {
        DaemonProcessGuard::spawn(&self.kestic, &self.iso, &self.dbus.address, lang)
    }

    pub fn ctx(&self) -> SmokeContext<'_> {
        SmokeContext {
            kestic_bin: &self.kestic,
            iso: &self.iso,
            dbus_addr: &self.dbus.address,
        }
    }
}
