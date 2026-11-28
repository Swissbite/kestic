// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared harness for the offscreen GUI smoke tests: an isolated XDG tree, a private session bus,
//! a real `kestic daemon` and the `kestic --smoke-m1 <scenario>` runner that captures a PNG.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub use crate::bus::PrivateDbusGuard;
use crate::keyring::{HeadlessKeyring, KeyringDirs};
use crate::process::KillOnDrop;
use crate::wait::wait_until_within;
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
    _child: KillOnDrop,
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

        let mut child = KillOnDrop::spawn(&mut cmd, "kestic daemon");
        wait_until_within(Duration::from_secs(8), "kestic daemon ready file", || {
            if let Some(status) = child.try_wait().expect("try_wait daemon") {
                let mut err_buf = String::new();
                if let Some(mut se) = child.stderr.take() {
                    use std::io::Read;
                    let _ = se.read_to_string(&mut err_buf);
                }
                panic!("kestic daemon exited early ({status}): {err_buf}");
            }
            ready_file.is_file()
        });
        Self { _child: child }
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
        let screenshot = fresh_screenshot_path(screenshot_rel);
        let output = self
            .smoke_command(scenario, layout, lang, &screenshot, extra_args, extra_env)
            .output()
            .expect("run kestic --smoke-m1");
        let outcome = SmokeOutcome {
            success: output.status.success(),
            status: output.status.to_string(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        };
        outcome.assert_passed(&format!("{scenario} ({layout}/{lang})"), &screenshot);
    }

    /// Like [`Self::run_with_env`] for a scene in which the GUI starts the daemon itself. That
    /// daemon outlives the GUI and inherits its output, so the output goes to files (waiting for
    /// the end of a pipe would wait for the daemon) and the daemon's own lines are not counted.
    pub fn run_starting_daemon(
        &self,
        scenario: &str,
        layout: &str,
        lang: &str,
        screenshot_rel: &str,
        extra_args: &[&str],
        extra_env: &[(&str, &str)],
    ) {
        let screenshot = fresh_screenshot_path(screenshot_rel);
        let out_path = self
            .iso
            .root
            .join(format!("smoke-{scenario}-{lang}.stdout"));
        let err_path = self
            .iso
            .root
            .join(format!("smoke-{scenario}-{lang}.stderr"));
        let status = self
            .smoke_command(scenario, layout, lang, &screenshot, extra_args, extra_env)
            .stdout(fs::File::create(&out_path).expect("create stdout file"))
            .stderr(fs::File::create(&err_path).expect("create stderr file"))
            .status()
            .expect("run kestic --smoke-m1");
        let gui_lines = |path: &Path| -> String {
            fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.starts_with("kestic-daemon"))
                .map(|l| format!("{l}\n"))
                .collect()
        };
        let outcome = SmokeOutcome {
            success: status.success(),
            status: status.to_string(),
            stdout: gui_lines(&out_path),
            stderr: gui_lines(&err_path),
        };
        outcome.assert_passed(&format!("{scenario} ({layout}/{lang})"), &screenshot);
    }

    fn smoke_command(
        &self,
        scenario: &str,
        layout: &str,
        lang: &str,
        screenshot: &Path,
        extra_args: &[&str],
        extra_env: &[(&str, &str)],
    ) -> Command {
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
        .arg(screenshot)
        .args(extra_args);
        self.iso.apply_env(&mut cmd, self.dbus_addr, lang);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd
    }
}

/// The absolute screenshot path below the workspace, with its folder made and an old file gone.
fn fresh_screenshot_path(screenshot_rel: &str) -> PathBuf {
    let screenshot = workspace_root().join(screenshot_rel);
    if let Some(parent) = screenshot.parent() {
        fs::create_dir_all(parent).expect("create screenshot dir");
    }
    if screenshot.exists() {
        fs::remove_file(&screenshot).expect("remove stale screenshot");
    }
    screenshot
}

/// What one smoke run of the GUI left behind.
struct SmokeOutcome {
    success: bool,
    status: String,
    stdout: String,
    stderr: String,
}

impl SmokeOutcome {
    /// The run ended successfully, without QML/Qt warnings, and wrote a real PNG.
    fn assert_passed(&self, what: &str, screenshot: &Path) {
        assert!(
            self.success,
            "smoke `{what}` failed (status={}):\nstdout:\n{}\nstderr:\n{}",
            self.status, self.stdout, self.stderr
        );
        assert!(
            self.stderr.trim().is_empty(),
            "smoke `{what}` emitted QML/Qt warnings:\n{}",
            self.stderr
        );

        let png = fs::read(screenshot).expect("read captured PNG screenshot");
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

/// Runs `body` in a child process of the current test binary (`test_name --exact`) whose `HOME`
/// and `XDG_*` directories lie in a fresh [`IsolatedEnv`], so the test never reads the real home
/// folder; `inner_env` marks the child, which gets the root of the tree.
pub fn run_with_temp_home(test_name: &str, inner_env: &str, body: impl FnOnce(&Path)) {
    if std::env::var_os(inner_env).as_deref() == Some("1".as_ref()) {
        let root = std::env::var_os("KESTIC_TEST_ROOT").expect("KESTIC_TEST_ROOT set");
        body(Path::new(&root));
        return;
    }
    let dirs = IsolatedEnv::create(inner_env, None);
    let out = Command::new(std::env::current_exe().expect("current test executable"))
        .args([test_name, "--exact"])
        .env(inner_env, "1")
        .env("KESTIC_TEST_ROOT", &dirs.root)
        .env("HOME", &dirs.home)
        .env("XDG_CONFIG_HOME", &dirs.cfg)
        .env("XDG_DATA_HOME", &dirs.data)
        .env("XDG_CACHE_HOME", &dirs.cache)
        .env("XDG_STATE_HOME", &dirs.state)
        .env("XDG_RUNTIME_DIR", &dirs.run)
        .output()
        .expect("run the test in a temp home");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "test `{test_name}` in a temp home failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
