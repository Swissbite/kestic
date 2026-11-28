// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Test tools shared by the integration tests: the dev-tools directory, a local `rest-server`, free
//! loopback ports, restic lock-file lookup, restic binary resolution and argument-list assertions.

use std::env;
use std::fs;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::process::KillOnDrop;
use crate::wait::poll_within;

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
/// (`restic-latest`, `restic-0.20.0`), `/usr/local/bin/restic`, then `$PATH`.
pub fn resolve_restic_binaries() -> Vec<(&'static str, PathBuf)> {
    let dir = dev_tools_bin_dir();
    let v018 = env::var_os("KESTIC_RESTIC_0_18")
        .filter(|v| !v.is_empty())
        .map_or_else(|| dir.join("restic-0.18.0"), PathBuf::from);

    let vlatest = env::var_os("KESTIC_RESTIC_LATEST")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| {
            let p = dir.join("restic-latest");
            p.is_file().then_some(p)
        })
        .or_else(|| {
            let p = dir.join("restic-0.20.0");
            p.is_file().then_some(p)
        })
        .or_else(|| {
            let p = PathBuf::from("/usr/local/bin/restic");
            p.is_file().then_some(p)
        })
        .or_else(|| find_in_path("restic"))
        .expect("latest restic binary (0.20.0 / restic-latest / PATH) must exist");

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
        .map_or_else(|| dev_tools_bin_dir().join("rest-server"), PathBuf::from);
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
        .map_or_else(|| root.join("target"), PathBuf::from)
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
    data_dir: PathBuf,
    auth: Option<(String, String)>,
    child: Option<KillOnDrop>,
}

impl RestServerGuard {
    /// Starts `rest-server` on a random free port with `.htpasswd` authentication: user `alice`
    /// and a generated [`strong_password`](crate::password::strong_password).
    pub fn start_with_auth(data_dir: &Path) -> Self {
        let port = free_loopback_port();
        let password = crate::password::strong_password();
        Self::start_on_port(data_dir, port, Some(("alice", &password)))
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

        let child = KillOnDrop::spawn(
            cmd.stdout(Stdio::null()).stderr(Stdio::null()),
            "rest-server",
        );

        let mut guard = Self {
            url,
            port,
            data_dir: data_dir.to_path_buf(),
            auth: auth.map(|(u, p)| (u.to_string(), p.to_string())),
            child: Some(child),
        };
        guard.wait_ready();
        guard
    }

    fn wait_ready(&mut self) {
        // rest-server accepts and closes the first connection: read it, so its log stays quiet.
        let stream = wait_listening(
            self.child.as_deref_mut(),
            self.port,
            Duration::from_secs(5),
            "rest-server",
            String::new,
        );
        let mut buf = [0u8; 1];
        let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
        let _ = (&stream).read(&mut buf);
    }

    /// `(username, password)` of the server, `None` for `--no-auth`.
    pub fn auth(&self) -> Option<(&str, &str)> {
        self.auth.as_ref().map(|(u, p)| (u.as_str(), p.as_str()))
    }

    /// Password of the authenticated server.
    pub fn password(&self) -> &str {
        self.auth()
            .expect("rest-server started with authentication")
            .1
    }

    /// Directory that holds the repository behind [`Self::url`].
    pub fn repo_dir(&self) -> PathBuf {
        match self.auth() {
            Some((user, _)) => self.data_dir.join(user),
            None => self.data_dir.clone(),
        }
    }

    /// Starts the server again on the same port, data directory and credentials (after
    /// [`Self::stop`] or for a second guard).
    pub fn restart(&self) -> Self {
        Self::start_on_port(&self.data_dir, self.port, self.auth())
    }

    /// Stops `rest-server` immediately (keeps `data_dir` and `.htpasswd` intact for restarts).
    pub fn stop(&mut self) {
        self.child = None;
    }
}

/// Runs `cmd` with `input` on stdin and returns its output; panics with `what` on failure.
fn pipe_through(cmd: &mut Command, input: &[u8], what: &str) -> Vec<u8> {
    use std::io::Write;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {what}: {e}"));
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input)
        .unwrap_or_else(|e| panic!("write to {what}: {e}"));
    let out = child
        .wait_with_output()
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(
        out.status.success(),
        "{what} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// `.htpasswd` line for `user`; the password goes through stdin, never argv.
fn build_htpasswd_entry(user: &str, pass: &str) -> String {
    if find_in_path("htpasswd").is_some() {
        let entry = pipe_through(
            Command::new("htpasswd").args(["-niB", user]),
            pass.as_bytes(),
            "htpasswd",
        );
        return String::from_utf8_lossy(&entry).into_owned();
    }

    // Fallback: Apache `{SHA}` entry generated with `openssl` (`rest-server` supports `{SHA}`).
    let digest = pipe_through(
        Command::new("openssl").args(["dgst", "-sha1", "-binary"]),
        pass.as_bytes(),
        "openssl sha1",
    );
    let b64 = pipe_through(
        Command::new("openssl").args(["base64", "-A"]),
        &digest,
        "openssl base64",
    );
    let hash = String::from_utf8_lossy(&b64).trim().to_string();
    format!("{user}:{{SHA}}{hash}\n")
}

/// Waits until a server started as `child` listens on `127.0.0.1:<port>` and returns the test
/// connection; panics with `what`, its exit status and `log()` when it ends first, and after
/// `limit`.
pub fn wait_listening(
    mut child: Option<&mut std::process::Child>,
    port: u16,
    limit: Duration,
    what: &str,
    log: impl Fn() -> String,
) -> TcpStream {
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().expect("socket addr");
    poll_within(limit, &format!("{what} listening on {addr}"), || {
        if let Some(child) = child.as_deref_mut()
            && let Some(status) = child.try_wait().expect("try_wait")
        {
            panic!("{what} exited prematurely: {status}\n{}", log());
        }
        TcpStream::connect_timeout(&addr, Duration::from_millis(100)).ok()
    })
}
