// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

use kestic_core::secrets::{SecretStore, SecretValue};
use kestic_daemon::secrets::SecretServiceStore;
use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use zbus::{Connection, block_on};

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn kestic_binary() -> PathBuf {
    if let Some(p) = env::var_os("CARGO_BIN_EXE_kestic") {
        return PathBuf::from(p);
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .expect("workspace root")
        .join("target")
        .join("debug")
        .join("kestic")
}

fn wait_for_dbus_owner(child: &mut Child, name: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("try_wait gnome-keyring-daemon") {
            panic!("gnome-keyring-daemon exited early with {status:?}");
        }
        let has_owner = block_on(async {
            let conn = Connection::session().await?;
            let proxy = zbus::fdo::DBusProxy::new(&conn).await?;
            proxy
                .name_has_owner(name.try_into().expect("valid bus name"))
                .await
        });
        if matches!(has_owner, Ok(true)) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for D-Bus owner `{name}`");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn ac_m1_05_1_05_2_05_3_05_4_kestic_secret_get_over_isolated_dbus_keyring_and_restic() {
    let bin = kestic_binary();
    if !bin.exists() {
        panic!(
            "kestic binary not built at {}; run `cargo build --bin kestic`",
            bin.display()
        );
    }

    if env::var_os("KESTIC_M1_S05_CHILD").as_deref() != Some("1".as_ref()) {
        // Short `/tmp` path so gnome-keyring control socket stays under 107-byte sun_path limit.
        let temp = env::temp_dir().join(format!("kestic-s05-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        for sub in [
            "home", "cfg", "data", "cache", "state", "run", "keyring", "repo",
        ] {
            fs::create_dir_all(temp.join(sub)).expect("mkdir");
        }
        fs::set_permissions(temp.join("run"), fs::Permissions::from_mode(0o700)).expect("chmod");
        fs::set_permissions(temp.join("keyring"), fs::Permissions::from_mode(0o700))
            .expect("chmod");
        let _guard = TempDirGuard(temp.clone());

        let restic_exe = env::var_os("KESTIC_RESTIC_LATEST")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("HOME").map(|h| {
                    PathBuf::from(h)
                        .join(".local")
                        .join("share")
                        .join("kestic-dev")
                        .join("bin")
                        .join("restic-latest")
                })
            })
            .filter(|p| p.exists())
            .expect("restic-latest binary must exist");

        let current_exe = env::current_exe().expect("current test exe");
        let out = Command::new("dbus-run-session")
            .arg("--")
            .arg(&current_exe)
            .arg(
                "ac_m1_05_1_05_2_05_3_05_4_kestic_secret_get_over_isolated_dbus_keyring_and_restic",
            )
            .arg("--exact")
            .env_remove("GNOME_KEYRING_CONTROL")
            .env_remove("SSH_AUTH_SOCK")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env("KESTIC_M1_S05_CHILD", "1")
            .env("HOME", temp.join("home"))
            .env("XDG_CONFIG_HOME", temp.join("cfg"))
            .env("XDG_DATA_HOME", temp.join("data"))
            .env("XDG_CACHE_HOME", temp.join("cache"))
            .env("XDG_STATE_HOME", temp.join("state"))
            .env("XDG_RUNTIME_DIR", temp.join("run"))
            .env("KESTIC_M1_S05_KEYRING_DIR", temp.join("keyring"))
            .env("KESTIC_M1_S05_REPO_DIR", temp.join("repo"))
            .env("KESTIC_M1_S05_RESTIC", restic_exe)
            .output()
            .expect("spawn dbus-run-session");

        assert!(
            out.status.success(),
            "isolated dbus-run-session test failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }

    let keyring_dir = PathBuf::from(env::var_os("KESTIC_M1_S05_KEYRING_DIR").expect("keyring dir"));
    let repo_dir = PathBuf::from(env::var_os("KESTIC_M1_S05_REPO_DIR").expect("repo dir"));
    let restic_exe = PathBuf::from(env::var_os("KESTIC_M1_S05_RESTIC").expect("restic bin"));

    let mut keyring_child = Command::new("gnome-keyring-daemon")
        .args([
            "--foreground",
            "--replace",
            "--unlock",
            "--components=secrets",
            "--control-directory",
        ])
        .arg(&keyring_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn headless gnome-keyring-daemon");

    if let Some(mut stdin) = keyring_child.stdin.take() {
        let _ = stdin.write_all(b"\n");
    }
    let mut keyring_guard = ChildGuard(keyring_child);
    wait_for_dbus_owner(
        &mut keyring_guard.0,
        "org.freedesktop.secrets",
        Duration::from_secs(5),
    );

    let store = SecretServiceStore::default();
    let dest_id = "k7m2q9xa";
    let pw = SecretValue::from_str_secret("secret-from-isolated-dbus-kestic!");
    store
        .set_secret(dest_id, "repo", &pw)
        .expect("set secret in isolated keyring");

    // AC-M1-05-2: `kestic secret get <ziel-id>` outputs exact password + Exit 0.
    let ok_out = Command::new(&bin)
        .args(["secret", "get", dest_id])
        .output()
        .expect("run kestic secret get");
    assert_eq!(ok_out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&ok_out.stdout),
        pw.as_str().unwrap()
    );

    // AC-M1-05-2: Unknown `<ziel-id>` -> Exit 1, empty stdout.
    let miss_out = Command::new(&bin)
        .args(["secret", "get", "zzzzzzzz"])
        .output()
        .expect("run kestic secret get missing");
    assert_eq!(miss_out.status.code(), Some(1));
    assert!(miss_out.stdout.is_empty());

    // AC-M1-05-3: Real restic init + snapshots via `RESTIC_PASSWORD_COMMAND`.
    let pw_cmd = format!("{} secret get {dest_id}", bin.display());
    let init_out = Command::new(&restic_exe)
        .args(["-r", &repo_dir.to_string_lossy(), "init"])
        .env_remove("RESTIC_PASSWORD")
        .env("RESTIC_PASSWORD_COMMAND", &pw_cmd)
        .output()
        .expect("restic init");
    assert!(
        init_out.status.success(),
        "restic init failed: {}",
        String::from_utf8_lossy(&init_out.stderr)
    );

    let snap_out = Command::new(&restic_exe)
        .args(["-r", &repo_dir.to_string_lossy(), "snapshots", "--json"])
        .env_remove("RESTIC_PASSWORD")
        .env("RESTIC_PASSWORD_COMMAND", &pw_cmd)
        .output()
        .expect("restic snapshots");
    assert!(snap_out.status.success());
}
