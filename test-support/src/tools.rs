// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Test tools shared by the integration tests: the dev-tools directory, a local `rest-server`, free
//! loopback ports, restic lock-file lookup, restic binary resolution and argument-list assertions.

use std::env;
use std::fs;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Resolves the `kestic-dev/bin` directory populated by `scripts/bootstrap.sh`.
pub fn dev_tools_bin_dir() -> PathBuf {
    if let Some(dir) = env::var_os("KESTIC_TOOLS_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(home) = env::var_os("KESTIC_HOST_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("kestic-dev")
            .join("bin");
    }
    if let Some(data) = env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        let candidate = PathBuf::from(data).join("kestic-dev").join("bin");
        if candidate.is_dir() {
            return candidate;
        }
    }
    let home = env::var_os("HOME").expect("HOME must be set for integration tests");
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("kestic-dev")
        .join("bin")
}

/// Resolves `(label, path)` of restic 0.18.0 and of the latest restic (`AC-M1-Q-2`): the
/// `KESTIC_RESTIC_0_18` / `KESTIC_RESTIC_LATEST` overrides, the dev-tools directory
/// (`restic-latest`, `restic-0.19.1`), `/usr/local/bin/restic`, then `$PATH`.
pub fn resolve_restic_binaries() -> Vec<(&'static str, PathBuf)> {
    let dir = dev_tools_bin_dir();
    let v018 = env::var_os("KESTIC_RESTIC_0_18")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("restic-0.18.0"));

    let vlatest = env::var_os("KESTIC_RESTIC_LATEST")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| {
            let p = dir.join("restic-latest");
            p.is_file().then_some(p)
        })
        .or_else(|| {
            let p = dir.join("restic-0.19.1");
            p.is_file().then_some(p)
        })
        .or_else(|| {
            let p = PathBuf::from("/usr/local/bin/restic");
            p.is_file().then_some(p)
        })
        .or_else(|| find_in_path("restic"))
        .expect("latest restic binary (0.19.1 / restic-latest / PATH) must exist");

    assert!(
        v018.is_file(),
        "restic-0.18.0 missing at {} (run scripts/bootstrap.sh)",
        v018.display()
    );

    vec![("restic-0.18.0", v018), ("restic-latest", vlatest)]
}

/// Temporary directory directly below `/tmp`: short enough for Unix socket paths (108 bytes),
/// removed when dropped.
pub fn short_tempdir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("{prefix}-"))
        .tempdir_in("/tmp")
        .expect("create temp dir below /tmp")
}

/// Writes `script` to `path` and makes it executable (`0755`).
pub fn write_executable(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, script).expect("write executable script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod 0755");
}

/// Installs `<dir>/<name>.sh`, a restic wrapper that runs the shell command `on_call` for every
/// invocation except `version` and then execs `real_restic`; returns the wrapper path.
fn install_restic_spy(dir: &Path, name: &str, real_restic: &Path, on_call: &str) -> PathBuf {
    let script = dir.join(format!("{name}.sh"));
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\nif [ \"$1\" != \"version\" ]; then\n  {on_call}\nfi\nexec \"{}\" \"$@\"\n",
            real_restic.display()
        ),
    );
    script
}

/// Restic wrapper that creates a marker file on every non-`version` invocation; returns
/// `(wrapper, marker)` (the marker exists once restic was invoked).
pub fn install_marker_spy(dir: &Path, name: &str, real_restic: &Path) -> (PathBuf, PathBuf) {
    let marker = dir.join(format!("{name}.marker"));
    let script = install_restic_spy(
        dir,
        name,
        real_restic,
        &format!("touch \"{}\"", marker.display()),
    );
    (script, marker)
}

/// Restic wrapper that appends the arguments of every non-`version` invocation as one line to a
/// log; returns `(wrapper, log)`.
pub fn install_args_spy(dir: &Path, name: &str, real_restic: &Path) -> (PathBuf, PathBuf) {
    let log = dir.join(format!("{name}.log"));
    let script = install_restic_spy(
        dir,
        name,
        real_restic,
        &format!("echo \"$*\" >> \"{}\"", log.display()),
    );
    (script, log)
}

fn find_in_path(exe: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Resolves `rest-server` binary installed by `scripts/bootstrap.sh`.
pub fn rest_server_binary() -> PathBuf {
    let bin = env::var_os("KESTIC_REST_SERVER")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dev_tools_bin_dir().join("rest-server"));
    assert!(
        bin.is_file(),
        "rest-server missing at {} (run scripts/bootstrap.sh)",
        bin.display()
    );
    bin
}

/// Asserts that `args` contains `flag` immediately followed by `value`.
pub fn assert_has_flag_pair(args: &[String], flag: &str, value: &str) {
    assert!(
        args.windows(2).any(|w| w[0] == flag && w[1] == value),
        "missing `{flag} {value}` in {args:?}"
    );
}

/// Allocates a free localhost TCP port for test servers or guaranteed-closed preflight probes.
pub fn free_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

/// Path of the compiled German catalog (`target/locale/de/LC_MESSAGES/kestic.mo`).
pub fn built_german_catalog() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has workspace root parent");
    env::var_os("CARGO_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
        .join("locale/de/LC_MESSAGES/kestic.mo")
}

/// Copies the compiled German catalog, if built, to `<data_dir>/locale/de/LC_MESSAGES/` so the
/// EN and DE runs of a process using `XDG_DATA_HOME=<data_dir>` both find it.
pub fn install_german_catalog(data_dir: &Path) {
    let built = built_german_catalog();
    if built.is_file() {
        let dest_dir = data_dir.join("locale/de/LC_MESSAGES");
        fs::create_dir_all(&dest_dir).expect("mkdir locale dir");
        fs::copy(&built, dest_dir.join("kestic.mo")).expect("copy kestic.mo");
    }
}

/// Finds a non-empty restic lock file (64 hex chars) below `locks_dir`.
pub fn find_lock_file(locks_dir: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(locks_dir).ok()?.flatten() {
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

/// Running local `rest-server` instance on a localhost TCP port (`--no-auth` or `--htpasswd-file`).
pub struct RestServerGuard {
    pub url: String,
    pub port: u16,
    child: Option<Child>,
}

impl RestServerGuard {
    /// Starts `rest-server` on a random free port with `.htpasswd` authentication (`alice` / `secret`).
    pub fn start_with_auth(data_dir: &Path) -> Self {
        let port = free_loopback_port();
        Self::start_on_port(data_dir, port, Some(("alice", "secret")))
    }

    /// Starts `rest-server` on a random free port with `--no-auth`.
    pub fn start_no_auth(data_dir: &Path) -> Self {
        let port = free_loopback_port();
        Self::start_on_port(data_dir, port, None)
    }

    /// Starts `rest-server` on an explicit port with optional `(username, password)` auth.
    pub fn start_on_port(data_dir: &Path, port: u16, auth: Option<(&str, &str)>) -> Self {
        fs::create_dir_all(data_dir).expect("mkdir rest-server data");
        let addr = format!("127.0.0.1:{port}");
        let mut cmd = Command::new(rest_server_binary());
        cmd.args(["--path", &data_dir.to_string_lossy(), "--listen", &addr]);

        let url = if let Some((user, pass)) = auth {
            let htpasswd = data_dir.join(".htpasswd");
            let entry = build_htpasswd_entry(user, pass);
            fs::write(&htpasswd, entry).expect("write .htpasswd");
            cmd.args(["--htpasswd-file", &htpasswd.to_string_lossy()]);
            format!("http://{addr}/{user}")
        } else {
            cmd.arg("--no-auth");
            format!("http://{addr}")
        };

        let child = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rest-server");

        let mut guard = Self {
            url,
            port,
            child: Some(child),
        };
        guard.wait_ready();
        guard
    }

    fn wait_ready(&mut self) {
        let addr = format!("127.0.0.1:{}", self.port);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(ref mut child) = self.child
                && let Some(status) = child.try_wait().expect("try_wait rest-server")
            {
                panic!("rest-server exited prematurely: {status}");
            }
            if let Ok(mut stream) = TcpStream::connect_timeout(
                &addr.parse().expect("socket addr"),
                Duration::from_millis(100),
            ) {
                let mut buf = [0u8; 1];
                let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
                let _ = stream.read(&mut buf);
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("rest-server did not start listening on {addr} in time");
    }

    /// Stops `rest-server` immediately (keeps `data_dir` and `.htpasswd` intact for restarts).
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for RestServerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn build_htpasswd_entry(user: &str, pass: &str) -> String {
    if find_in_path("htpasswd").is_some()
        && let Ok(out) = Command::new("htpasswd").args(["-nbB", user, pass]).output()
        && out.status.success()
    {
        return String::from_utf8_lossy(&out.stdout).into_owned();
    }

    // Fallback: Apache `{SHA}` entry generated with `openssl` (`rest-server` supports `{SHA}`).
    let mut child = Command::new("openssl")
        .args(["dgst", "-sha1", "-binary"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn openssl for htpasswd SHA1");
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(pass.as_bytes()).expect("write password");
    }
    let digest = child.wait_with_output().expect("openssl sha1");
    assert!(digest.status.success(), "openssl sha1 failed");

    let mut b64_child = Command::new("openssl")
        .args(["base64", "-A"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn openssl base64");
    if let Some(mut stdin) = b64_child.stdin.take() {
        use std::io::Write;
        stdin.write_all(&digest.stdout).expect("write sha1 digest");
    }
    let b64 = b64_child.wait_with_output().expect("openssl base64");
    assert!(b64.status.success(), "openssl base64 failed");
    let hash = String::from_utf8_lossy(&b64.stdout).trim().to_string();
    format!("{user}:{{SHA}}{hash}\n")
}
