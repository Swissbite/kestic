// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Integration tests for M0-S10: Portal-Inhibit (F9) and replaced-binary detection (E11).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
use zbus::{OwnedObjectPath, OwnedVariant, block_on, interface};

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("daemon crate has workspace root parent")
        .to_path_buf()
}

fn find_kestic_binary(workspace_root: &Path) -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"));
    for profile in ["debug", "release"] {
        let candidate = target_dir.join(profile).join("kestic");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("kestic binary not found under {}", target_dir.display());
}

/// AC-M0-10-2 (E11): Spawns a binary copy inside a tempdir, replaces it atomically via `rename`
/// (mimicking `dpkg`), and verifies the running process detects the replacement cleanly.
#[test]
fn ac_m0_10_2_daemon_detects_replaced_binary_after_rename() {
    let root = workspace_root();
    let source_bin = find_kestic_binary(&root);

    let temp = root.join(format!("target/test-tmp-m0-s10-bin-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).expect("create tempdir");
    let _guard = TempDirGuard(temp.clone());

    let installed_bin = temp.join("kestic");
    let staged_bin = temp.join("kestic.dpkg-new");
    let ready_file = temp.join("ready");
    let detected_file = temp.join("detected");

    fs::copy(&source_bin, &installed_bin).expect("copy binary into tempdir");
    fs::copy(&source_bin, &staged_bin).expect("stage replacement binary");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&installed_bin, fs::Permissions::from_mode(0o755))
            .expect("chmod 0755 installed_bin");
        fs::set_permissions(&staged_bin, fs::Permissions::from_mode(0o755))
            .expect("chmod 0755 staged_bin");
    }

    let child = Command::new(&installed_bin)
        .args(["spike-watch-exe", "--ready-file"])
        .arg(&ready_file)
        .arg("--detected-file")
        .arg(&detected_file)
        .args(["--timeout-ms", "5000"])
        .env("HOME", temp.join("home"))
        .env("XDG_CONFIG_HOME", temp.join("cfg"))
        .env("XDG_DATA_HOME", temp.join("data"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn copied kestic binary");

    let ready_deadline = Instant::now() + Duration::from_secs(20);
    while !ready_file.exists() {
        assert!(
            Instant::now() < ready_deadline,
            "child did not signal readiness before timeout"
        );
        thread::sleep(Duration::from_millis(10));
    }

    // Atomic replacement via rename, identical to dpkg package upgrade.
    fs::rename(&staged_bin, &installed_bin).expect("atomic rename replacement");

    let output = child.wait_with_output().expect("wait for child exit");
    assert!(
        output.status.success(),
        "binary watch process failed (status={:?})\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let reason = fs::read_to_string(&detected_file).expect("read detected reason");
    let reason = reason.trim();
    assert!(
        matches!(reason, "proc_self_exe_deleted" | "inode_changed"),
        "unexpected binary replacement reason: {reason:?}"
    );
}

struct FakePortalInhibit {
    last_flags: Arc<AtomicU32>,
    last_reason: Arc<std::sync::Mutex<String>>,
    request_path: OwnedObjectPath,
}

#[interface(name = "org.freedesktop.portal.Inhibit")]
impl FakePortalInhibit {
    fn inhibit(
        &self,
        _window: &str,
        flags: u32,
        options: HashMap<String, OwnedVariant>,
    ) -> OwnedObjectPath {
        self.last_flags.store(flags, Ordering::SeqCst);
        if let Some(reason) = options.get("reason").and_then(|v| <&str>::try_from(v).ok()) {
            *self.last_reason.lock().expect("lock") = reason.to_string();
        }
        self.request_path.clone()
    }
}

struct FakePortalRequest {
    closed: Arc<AtomicBool>,
}

#[interface(name = "org.freedesktop.portal.Request")]
impl FakePortalRequest {
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

/// AC-M0-10-1 (F9, D-Bus protocol check): Verifies Portal-Inhibit acquires Idle + Suspend
/// (`flags = 12`) with reason and calls `org.freedesktop.portal.Request.Close` on release
/// inside an isolated `dbus-run-session`.
#[test]
fn ac_m0_10_1_portal_inhibit_roundtrip_in_isolated_dbus_session() {
    if std::env::var_os("KESTIC_M0_S10_INNER_DBUS").as_deref() == Some("1".as_ref()) {
        let last_flags = Arc::new(AtomicU32::new(0));
        let last_reason = Arc::new(std::sync::Mutex::new(String::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let req_path = OwnedObjectPath::try_from("/org/freedesktop/portal/desktop/request/test/1")
            .expect("valid request path");

        let fake_inhibit = FakePortalInhibit {
            last_flags: Arc::clone(&last_flags),
            last_reason: Arc::clone(&last_reason),
            request_path: req_path.clone(),
        };
        let fake_request = FakePortalRequest {
            closed: Arc::clone(&closed),
        };

        let _server_conn = block_on(async move {
            zbus::connection::Builder::session()
                .name("net.daester.Kestic.FakePortal")
                .serve_at("/org/freedesktop/portal/desktop", fake_inhibit)
                .serve_at(req_path, fake_request)
                .build()
                .await
                .expect("build fake portal connection")
        });

        let guard = kestic_daemon::inhibit::PortalInhibitGuard::acquire(
            "net.daester.Kestic.FakePortal",
            "Kestic backup active",
            kestic_daemon::inhibit::INHIBIT_IDLE_AND_SUSPEND,
        )
        .expect("acquire portal inhibit");

        assert_eq!(
            last_flags.load(Ordering::SeqCst),
            12,
            "Portal Inhibit must request Suspend(4) | Idle(8) = 12"
        );
        assert_eq!(
            last_reason.lock().expect("lock").as_str(),
            "Kestic backup active"
        );
        assert!(!closed.load(Ordering::SeqCst));

        guard.release().expect("release portal inhibit");
        assert!(
            closed.load(Ordering::SeqCst),
            "releasing PortalInhibitGuard must call Request.Close()"
        );
        return;
    }

    let root = workspace_root();
    let temp = root.join(format!(
        "target/test-tmp-m0-s10-dbus-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    for sub in ["home", "cfg", "data", "cache", "state", "run"] {
        fs::create_dir_all(temp.join(sub)).expect("create isolated XDG dir");
    }
    let _guard = TempDirGuard(temp.clone());

    let current_exe = std::env::current_exe().expect("current test executable");
    let out = Command::new("dbus-run-session")
        .arg(format!(
            "--config-file={}",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/private-session-bus.xml"
            )
        ))
        .arg("--")
        .arg(current_exe)
        .arg("ac_m0_10_1_portal_inhibit_roundtrip_in_isolated_dbus_session")
        .arg("--exact")
        .env("KESTIC_M0_S10_INNER_DBUS", "1")
        .env("HOME", temp.join("home"))
        .env("XDG_CONFIG_HOME", temp.join("cfg"))
        .env("XDG_DATA_HOME", temp.join("data"))
        .env("XDG_CACHE_HOME", temp.join("cache"))
        .env("XDG_STATE_HOME", temp.join("state"))
        .env("XDG_RUNTIME_DIR", temp.join("run"))
        .output()
        .expect("run test inside dbus-run-session");

    assert!(
        out.status.success(),
        "isolated dbus-run-session test failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
