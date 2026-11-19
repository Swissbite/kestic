// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(dead_code)]

use std::env;
use std::path::PathBuf;

/// Resolves the `kestic-dev/bin` directory populated by `scripts/bootstrap.sh`.
pub fn dev_tools_bin_dir() -> PathBuf {
    if let Some(dir) = env::var_os("KESTIC_TOOLS_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(data) = env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(data).join("kestic-dev").join("bin");
    }
    let home = env::var_os("HOME").expect("HOME must be set for integration tests");
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("kestic-dev")
        .join("bin")
}

/// Returns `(label, binary_path)` pairs for both restic 0.18.0 and latest (`AC-M1-Q-2`).
///
/// Each path is an isolation wrapper (see [`isolated_restic`]): restic child processes always run
/// with `HOME` and all `XDG_*` directories inside `target/tmp`, never in the real user profile.
pub fn restic_test_binaries() -> Vec<(&'static str, PathBuf)> {
    let dir = dev_tools_bin_dir();
    let v018 = env::var_os("KESTIC_RESTIC_0_18")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("restic-0.18.0"));
    let vlatest = env::var_os("KESTIC_RESTIC_LATEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("restic-latest"));

    assert!(
        v018.exists(),
        "restic-0.18.0 missing at {} (run scripts/bootstrap.sh)",
        v018.display()
    );
    assert!(
        vlatest.exists(),
        "restic-latest missing at {} (run scripts/bootstrap.sh)",
        vlatest.display()
    );

    vec![
        ("restic-0.18.0", isolated_restic(&v018, "restic-0.18.0")),
        ("restic-latest", isolated_restic(&vlatest, "restic-latest")),
    ]
}

/// Writes (once, atomically) a shell wrapper that points `HOME`, `XDG_CONFIG_HOME`,
/// `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` and `XDG_RUNTIME_DIR` at a temp tree below
/// `CARGO_TARGET_TMPDIR` and then `exec`s the real restic binary (same PID, signals unchanged).
fn isolated_restic(real: &std::path::Path, tag: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("kestic-restic-isolation");
    let home = root.join("home");
    let run = root.join("run");
    for sub in ["home", "cfg", "data", "cache", "state", "run"] {
        std::fs::create_dir_all(root.join(sub)).expect("mkdir restic isolation dir");
    }
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700))
        .expect("chmod 0700 XDG_RUNTIME_DIR");

    // Override a variable only while it still has the value of the test process (or is unset):
    // tests that hand restic an explicit `HOME` (portable `$HOME` exclude tests) keep theirs.
    let mut script = String::from("#!/bin/sh\n");
    let vars: [(&str, PathBuf); 6] = [
        ("HOME", home),
        ("XDG_CONFIG_HOME", root.join("cfg")),
        ("XDG_DATA_HOME", root.join("data")),
        ("XDG_CACHE_HOME", root.join("cache")),
        ("XDG_STATE_HOME", root.join("state")),
        ("XDG_RUNTIME_DIR", run),
    ];
    for (name, isolated) in &vars {
        let inherited = env::var(name).unwrap_or_default();
        script.push_str(&format!(
            "case \"${{{name}:-}}\" in '{inherited}'|'') export {name}='{isolated}' ;; esac\n",
            isolated = isolated.display()
        ));
    }
    script.push_str(&format!("exec '{}' \"$@\"\n", real.display()));
    let wrapper = root.join(format!("{tag}-isolated.sh"));
    if std::fs::read_to_string(&wrapper).is_ok_and(|existing| existing == script) {
        return wrapper;
    }
    let tmp = root.join(format!("{tag}-isolated.sh.{}", std::process::id()));
    std::fs::write(&tmp, &script).expect("write restic isolation wrapper");
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
        .expect("chmod restic isolation wrapper");
    std::fs::rename(&tmp, &wrapper).expect("install restic isolation wrapper");
    wrapper
}

/// Returns the `rest-server` test binary installed by `scripts/bootstrap.sh`.
pub fn rest_server_binary() -> PathBuf {
    let bin = env::var_os("KESTIC_REST_SERVER")
        .map(PathBuf::from)
        .unwrap_or_else(|| dev_tools_bin_dir().join("rest-server"));
    assert!(
        bin.exists(),
        "rest-server missing at {} (run scripts/bootstrap.sh)",
        bin.display()
    );
    bin
}

/// Running local `rest-server` instance on a random loopback port with `.htpasswd` auth.
pub struct RestServerGuard {
    pub url: String,
    child: std::process::Child,
}

impl Drop for RestServerGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts a local `rest-server` in `data_dir` on a free ephemeral port (`alice` / `secret`).
pub fn start_rest_server_with_auth(data_dir: &std::path::Path) -> RestServerGuard {
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    std::fs::create_dir_all(data_dir).expect("mkdir rest-server data");
    // SHA-1 htpasswd entry for user `alice`, password `secret`.
    let htpasswd = data_dir.join(".htpasswd");
    std::fs::write(&htpasswd, "alice:{SHA}5en6G6MezRroT3XKqkdPOmY/BfQ=\n")
        .expect("write .htpasswd");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let addr = format!("127.0.0.1:{port}");
    let child = Command::new(rest_server_binary())
        .args([
            "--path",
            &data_dir.to_string_lossy(),
            "--htpasswd-file",
            &htpasswd.to_string_lossy(),
            "--listen",
            &addr,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rest-server");

    let mut guard = RestServerGuard {
        url: format!("http://{addr}/alice"),
        child,
    };

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(status) = guard.child.try_wait().expect("try_wait rest-server") {
            panic!("rest-server exited prematurely: {status}");
        }
        if let Ok(mut stream) = TcpStream::connect_timeout(
            &addr.parse().expect("socket addr"),
            Duration::from_millis(100),
        ) {
            let mut buf = [0u8; 1];
            let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
            let _ = stream.read(&mut buf);
            return guard;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    panic!("rest-server did not start listening on {addr} in time");
}

/// Allocates a guaranteed-closed loopback TCP port for connection-refused preflight tests.
pub fn unused_loopback_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

/// Writes an executable shell script emulating `kestic secret get <dest-id> [kind]` backed by
/// a `FileSecretStore` hex file, allowing standalone `cargo test -p kestic-core` runs.
pub fn write_secret_get_helper(
    dir: &std::path::Path,
    secrets_hex_path: &std::path::Path,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let script_path = dir.join("kestic-helper");
    let script = format!(
        r#"#!/usr/bin/env python3
import sys, pathlib
if len(sys.argv) < 3 or sys.argv[1] != "secret" or sys.argv[2] != "get":
    sys.exit(2)
target = sys.argv[3] if len(sys.argv) > 3 else ""
kind = sys.argv[4] if len(sys.argv) > 4 else "repo"
p = pathlib.Path({path:?})
if not p.exists():
    sys.exit(1)
for line in p.read_text(encoding="utf-8").splitlines():
    if "=" not in line:
        continue
    key, hex_val = line.strip().split("=", 1)
    if key == target + ":" + kind:
        sys.stdout.write(bytes.fromhex(hex_val).decode("utf-8"))
        sys.exit(0)
sys.exit(1)
"#,
        path = secrets_hex_path.to_string_lossy()
    );
    std::fs::write(&script_path, script).expect("write helper");
    let mut perms = std::fs::metadata(&script_path).expect("meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script_path, perms).expect("chmod");
    script_path
}

/// Returns an isolated per-test `RESTIC_CACHE_DIR` inside `root` so tests never write to `~/.cache/restic`.
pub fn restic_cache_dir(root: &std::path::Path) -> PathBuf {
    let dir = root.join("restic-cache");
    std::fs::create_dir_all(&dir).expect("mkdir restic-cache");
    dir
}
