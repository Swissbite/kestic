// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Isolated integration tests for M0-S08:
//! - AC-M0-08-1 / AC-M0-08-2: Direct `zbus 6.3.0` `org.freedesktop.secrets` roundtrip
//!   against headless `gnome-keyring-daemon` in `dbus-run-session`, plus `FileSecretStore`
//!   fallback behind `SecretStore` trait and locked-collection prompt handling.

use kestic_core::secrets::{FileSecretStore, SecretError, SecretStore, SecretValue};
use kestic_daemon::secrets::SecretServiceStore;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use zbus::{Connection, OwnedObjectPath, OwnedVariant, block_on, interface};

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

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("daemon crate has workspace root parent")
        .to_path_buf()
}

fn run_isolated_dbus_test(test_name: &str, env_key: &str) {
    static NEXT_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    // Keep tempdir path short: Linux sun_path (UNIX domain sockets for dbus / gnome-keyring
    // control socket) is limited to 107 bytes.
    let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = std::env::temp_dir().join(format!("kestic-s08-{}-{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    for sub in ["home", "cfg", "data", "cache", "state", "run", "keyring"] {
        fs::create_dir_all(temp.join(sub)).expect("create isolated XDG dir");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temp.join("run"), fs::Permissions::from_mode(0o700))
            .expect("chmod 0700 XDG_RUNTIME_DIR");
        fs::set_permissions(temp.join("keyring"), fs::Permissions::from_mode(0o700))
            .expect("chmod 0700 keyring dir");
    }
    let _guard = TempDirGuard(temp.clone());

    let current_exe = std::env::current_exe().expect("current test executable");
    let out = Command::new("dbus-run-session")
        .arg("--")
        .arg(current_exe)
        .arg(test_name)
        .arg("--exact")
        .env_remove("GNOME_KEYRING_CONTROL")
        .env_remove("SSH_AUTH_SOCK")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env(env_key, "1")
        .env("HOME", temp.join("home"))
        .env("XDG_CONFIG_HOME", temp.join("cfg"))
        .env("XDG_DATA_HOME", temp.join("data"))
        .env("XDG_CACHE_HOME", temp.join("cache"))
        .env("XDG_STATE_HOME", temp.join("state"))
        .env("XDG_RUNTIME_DIR", temp.join("run"))
        .env("KESTIC_M0_S08_KEYRING_DIR", temp.join("keyring"))
        .output()
        .expect("run test inside dbus-run-session");

    assert!(
        out.status.success(),
        "isolated dbus-run-session test `{test_name}` failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn wait_for_dbus_owner(child: &mut Child, name: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("try_wait gnome-keyring-daemon") {
            let mut out = String::new();
            let mut err = String::new();
            if let Some(mut s) = child.stdout.take() {
                use std::io::Read;
                let _ = s.read_to_string(&mut out);
            }
            if let Some(mut s) = child.stderr.take() {
                use std::io::Read;
                let _ = s.read_to_string(&mut err);
            }
            panic!(
                "gnome-keyring-daemon exited early with {status:?}\nstdout: {out}\nstderr: {err}"
            );
        }
        let has_owner = block_on(async {
            let conn = Connection::session().await?;
            let proxy = zbus::fdo::DBusProxy::new(&conn).await?;
            proxy
                .name_has_owner(name.try_into().expect("valid bus name"))
                .await
        });
        match has_owner {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => eprintln!("NameHasOwner check error: {err}"),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let mut out = String::new();
            let mut err = String::new();
            if let Some(mut s) = child.stdout.take() {
                use std::io::Read;
                let _ = s.read_to_string(&mut out);
            }
            if let Some(mut s) = child.stderr.take() {
                use std::io::Read;
                let _ = s.read_to_string(&mut err);
            }
            panic!("timed out waiting for D-Bus owner `{name}`\nstdout: {out}\nstderr: {err}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// AC-M0-08-2: Verifies headless `gnome-keyring-daemon` in `dbus-run-session` unlocks with empty
/// stdin password and performs a full `SecretStore` set/get/replace/delete roundtrip without hangs.
#[test]
fn ac_m0_08_2_headless_gnome_keyring_secret_roundtrip_in_dbus_run_session() {
    const INNER_ENV: &str = "KESTIC_M0_S08_KEYRING_INNER";
    if std::env::var_os(INNER_ENV).as_deref() == Some("1".as_ref()) {
        let keyring_dir =
            PathBuf::from(std::env::var_os("KESTIC_M0_S08_KEYRING_DIR").expect("keyring dir"));

        let mut child = Command::new("gnome-keyring-daemon")
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

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(b"\n")
                .expect("write empty unlock passphrase");
            drop(stdin);
        }
        wait_for_dbus_owner(
            &mut child,
            "org.freedesktop.secrets",
            Duration::from_secs(5),
        );
        let _daemon_guard = ChildGuard(child);

        let store: &dyn SecretStore = &SecretServiceStore::default();
        let initial = SecretValue::from_str_secret("kestic-repo-pass-01-äöü");
        let updated = SecretValue::from_str_secret("kestic-repo-pass-02-replaced");

        store
            .set_secret("nas-prod", "repo-password", &initial)
            .expect("write secret to headless gnome-keyring");

        let read_back = store
            .get_secret("nas-prod", "repo-password")
            .expect("read secret from headless gnome-keyring");
        assert_eq!(read_back.as_ref(), Some(&initial));

        // Overwrite existing entry (replace = true)
        store
            .set_secret("nas-prod", "repo-password", &updated)
            .expect("replace secret in headless gnome-keyring");
        let read_updated = store
            .get_secret("nas-prod", "repo-password")
            .expect("read replaced secret");
        assert_eq!(read_updated.as_ref(), Some(&updated));

        store
            .delete_secret("nas-prod", "repo-password")
            .expect("delete secret from headless gnome-keyring");
        let after_delete = store
            .get_secret("nas-prod", "repo-password")
            .expect("read deleted secret");
        assert_eq!(after_delete, None);
        return;
    }

    run_isolated_dbus_test(
        "ac_m0_08_2_headless_gnome_keyring_secret_roundtrip_in_dbus_run_session",
        INNER_ENV,
    );
}

/// AC-M0-08-2: Verifies the `FileSecretStore` fallback behind `dyn SecretStore` works in an
/// isolated tempdir (`0600` permissions) without any D-Bus daemon dependency.
#[test]
fn ac_m0_08_2_file_secret_store_fallback_roundtrip() {
    let root = workspace_root();
    let temp = root.join(format!(
        "target/test-tmp-m0-s08-file-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).expect("create temp dir");
    let _guard = TempDirGuard(temp.clone());

    let store_path = temp.join("secrets.hex");
    let fallback = FileSecretStore::new(&store_path);
    let store: &dyn SecretStore = &fallback;

    let secret = SecretValue::from_str_secret("fallback-secret-token");
    store
        .set_secret("s3-archive", "backend-secret-key", &secret)
        .expect("write fallback secret");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&store_path)
            .expect("stat fallback secret file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "fallback secret file must be 0600");
    }

    assert_eq!(
        store
            .get_secret("s3-archive", "backend-secret-key")
            .expect("read fallback secret"),
        Some(secret)
    );

    store
        .delete_secret("s3-archive", "backend-secret-key")
        .expect("delete fallback secret");
    assert_eq!(
        store
            .get_secret("s3-archive", "backend-secret-key")
            .expect("read deleted fallback secret"),
        None
    );
}

struct FakeLockedSecretService;

#[interface(name = "org.freedesktop.Secret.Service")]
impl FakeLockedSecretService {
    #[zbus(name = "OpenSession")]
    fn open_session(
        &self,
        _algorithm: &str,
        _input: OwnedVariant,
    ) -> (OwnedVariant, OwnedObjectPath) {
        (
            OwnedVariant::from(zbus::Str::from("")),
            OwnedObjectPath::try_from("/org/freedesktop/secrets/session/s1").expect("path"),
        )
    }

    #[zbus(name = "ReadAlias")]
    fn read_alias(&self, _name: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/org/freedesktop/secrets/collection/locked").expect("path")
    }

    #[zbus(name = "Unlock")]
    fn unlock(&self, _objects: Vec<OwnedObjectPath>) -> (Vec<OwnedObjectPath>, OwnedObjectPath) {
        (
            Vec::new(),
            OwnedObjectPath::try_from("/org/freedesktop/secrets/prompt/p1").expect("prompt"),
        )
    }

    #[zbus(name = "SearchItems")]
    fn search_items(
        &self,
        _attributes: HashMap<String, String>,
    ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
        (Vec::new(), Vec::new())
    }
}

struct FakeLockedCollection;

#[interface(name = "org.freedesktop.Secret.Collection")]
impl FakeLockedCollection {
    #[zbus(property, name = "Locked")]
    fn locked(&self) -> bool {
        true
    }
}

struct FakeSecretSession;

#[interface(name = "org.freedesktop.Secret.Session")]
impl FakeSecretSession {
    #[zbus(name = "Close")]
    fn close(&self) {}
}

/// Verifies `SecretServiceStore` detects locked Secret Service collections requiring an
/// interactive prompt instead of hanging indefinitely (ADR-0004 / KWallet without PAM risk).
#[test]
fn secret_service_reports_locked_collection_prompt_without_hanging() {
    const INNER_ENV: &str = "KESTIC_M0_S08_LOCKED_INNER";
    if std::env::var_os(INNER_ENV).as_deref() == Some("1".as_ref()) {
        let _conn = block_on(async {
            zbus::connection::Builder::session()
                .name("net.daester.Kestic.FakeLockedSecrets")
                .serve_at("/org/freedesktop/secrets", FakeLockedSecretService)
                .serve_at(
                    "/org/freedesktop/secrets/collection/locked",
                    FakeLockedCollection,
                )
                .serve_at("/org/freedesktop/secrets/session/s1", FakeSecretSession)
                .build()
                .await
                .expect("build fake locked secret service")
        });

        let store = SecretServiceStore::new("net.daester.Kestic.FakeLockedSecrets", "default");
        let err = store
            .set_secret("nas", "repo-password", &SecretValue::from_str_secret("pw"))
            .expect_err("locked collection requiring prompt must return error");
        assert!(
            matches!(err, SecretError::LockedOrPromptRequired(_)),
            "expected LockedOrPromptRequired, got {err:?}"
        );
        return;
    }

    run_isolated_dbus_test(
        "secret_service_reports_locked_collection_prompt_without_hanging",
        INNER_ENV,
    );
}
