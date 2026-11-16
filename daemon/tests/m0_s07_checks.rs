// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Isolated D-Bus integration tests for M0-S07:
//! - AC-M0-07-1: Single-instance D-Bus name lock (second daemon detects taken name and exits 0)
//! - AC-M0-07-2: Daemon registers StatusNotifierItem via `ksni` and sends 1 `Notify` call via `notify-rust`

use kestic_daemon::service::{DaemonOutcome, DaemonRunConfig, run_daemon};
use kestic_daemon::single_instance::KESTIC_BUS_NAME;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Mutex,
    atomic::{AtomicU32, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
use zbus::{OwnedVariant, block_on, interface};

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

fn run_isolated_dbus_test(test_name: &str, env_key: &str) {
    let root = workspace_root();
    let temp = root.join(format!(
        "target/test-tmp-m0-s07-{}-{}",
        test_name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    for sub in ["home", "cfg", "data", "cache", "state", "run"] {
        fs::create_dir_all(temp.join(sub)).expect("create isolated XDG dir");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temp.join("run"), fs::Permissions::from_mode(0o700))
            .expect("chmod 0700 XDG_RUNTIME_DIR");
    }
    let _guard = TempDirGuard(temp.clone());

    let current_exe = std::env::current_exe().expect("current test executable");
    let out = Command::new("dbus-run-session")
        .arg("--")
        .arg(current_exe)
        .arg(test_name)
        .arg("--exact")
        .env(env_key, "1")
        .env("HOME", temp.join("home"))
        .env("XDG_CONFIG_HOME", temp.join("cfg"))
        .env("XDG_DATA_HOME", temp.join("data"))
        .env("XDG_CACHE_HOME", temp.join("cache"))
        .env("XDG_STATE_HOME", temp.join("state"))
        .env("XDG_RUNTIME_DIR", temp.join("run"))
        .env("KESTIC_M0_S07_TMP", &temp)
        .output()
        .expect("run test inside dbus-run-session");

    assert!(
        out.status.success(),
        "isolated dbus-run-session test `{test_name}` failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[derive(Default)]
struct FakeSniWatcher {
    registered_items: Mutex<Vec<String>>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl FakeSniWatcher {
    #[zbus(name = "RegisterStatusNotifierItem")]
    fn register_status_notifier_item(&self, service: &str) {
        self.registered_items
            .lock()
            .expect("lock")
            .push(service.to_string());
    }

    #[zbus(name = "RegisterStatusNotifierHost")]
    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property, name = "RegisteredStatusNotifierItems")]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.registered_items.lock().expect("lock").clone()
    }

    #[zbus(property, name = "IsStatusNotifierHostRegistered")]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property, name = "ProtocolVersion")]
    fn protocol_version(&self) -> i32 {
        0
    }
}

#[derive(Default)]
struct FakeNotifications {
    calls: AtomicU32,
    last_desktop_entry: Mutex<String>,
    last_summary: Mutex<String>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl FakeNotifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        _app_name: &str,
        _replaces_id: u32,
        _app_icon: &str,
        summary: &str,
        _body: &str,
        _actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        _expire_timeout: i32,
    ) -> u32 {
        *self.last_summary.lock().expect("lock") = summary.to_string();
        if let Some(entry) = hints
            .get("desktop-entry")
            .and_then(|v| <&str>::try_from(v).ok())
        {
            *self.last_desktop_entry.lock().expect("lock") = entry.to_string();
        }
        self.calls.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".to_string()]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "FakePlasmaNotifications".to_string(),
            "KesticTest".to_string(),
            "1.0".to_string(),
            "1.2".to_string(),
        )
    }
}

/// AC-M0-07-1 (F14): Given a running `kestic daemon` holding `net.daester.Kestic` in
/// `dbus-run-session`, when a second `kestic daemon` starts, it detects the taken
/// D-Bus name and exits cleanly with code 0.
#[test]
fn ac_m0_07_1_second_daemon_detects_taken_dbus_name_and_exits_zero() {
    const INNER_ENV: &str = "KESTIC_M0_S07_SINGLE_INNER";
    if std::env::var_os(INNER_ENV).as_deref() == Some("1".as_ref()) {
        let root = workspace_root();
        let kestic = find_kestic_binary(&root);
        let tmp = PathBuf::from(std::env::var_os("KESTIC_M0_S07_TMP").expect("tmp env"));
        let ready_file = tmp.join("daemon1.ready");

        let mut first = Command::new(&kestic)
            .args(["daemon", "--duration-ms", "5000", "--ready-file"])
            .arg(&ready_file)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn first kestic daemon");

        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready_file.exists() {
            assert!(
                Instant::now() < deadline,
                "first daemon did not signal readiness before timeout"
            );
            thread::sleep(Duration::from_millis(20));
        }

        let second = Command::new(&kestic)
            .args(["daemon", "--duration-ms", "5000"])
            .output()
            .expect("run second kestic daemon");

        let _ = first.wait();

        assert!(
            second.status.success(),
            "second daemon must exit 0 when D-Bus name is taken, got {:?}\nstdout: {}\nstderr: {}",
            second.status.code(),
            String::from_utf8_lossy(&second.stdout),
            String::from_utf8_lossy(&second.stderr)
        );
        let stdout = String::from_utf8_lossy(&second.stdout);
        assert!(
            stdout.contains("already running"),
            "second daemon should report existing instance cleanly, got: {stdout:?}"
        );
        return;
    }

    run_isolated_dbus_test(
        "ac_m0_07_1_second_daemon_detects_taken_dbus_name_and_exits_zero",
        INNER_ENV,
    );
}

/// AC-M0-07-2: Given the running daemon prototype, when it sends a test notification,
/// the fake `org.freedesktop.Notifications` server receives exactly one `Notify` call
/// and `org.kde.StatusNotifierWatcher` registers a StatusNotifierItem.
#[test]
fn ac_m0_07_2_daemon_registers_sni_and_sends_single_notify() {
    const INNER_ENV: &str = "KESTIC_M0_S07_TRAY_NOTIFY_INNER";
    if std::env::var_os(INNER_ENV).as_deref() == Some("1".as_ref()) {
        let watcher = FakeSniWatcher::default();
        let watcher_items_conn = block_on(async {
            zbus::connection::Builder::session()
                .name("org.kde.StatusNotifierWatcher")
                .serve_at("/StatusNotifierWatcher", watcher)
                .build()
                .await
                .expect("build fake StatusNotifierWatcher")
        });

        let notif = FakeNotifications::default();
        let notif_conn = block_on(async {
            zbus::connection::Builder::session()
                .name("org.freedesktop.Notifications")
                .serve_at("/org/freedesktop/Notifications", notif)
                .build()
                .await
                .expect("build fake org.freedesktop.Notifications")
        });

        let outcome = run_daemon(DaemonRunConfig {
            bus_name: KESTIC_BUS_NAME.to_string(),
            assume_sni_available: false,
            send_test_notification: true,
            duration: Some(Duration::from_millis(80)),
            ..Default::default()
        })
        .expect("run daemon prototype");
        assert_eq!(outcome, DaemonOutcome::RanAndStopped);

        let registered = block_on(async {
            let iface_ref = watcher_items_conn
                .object_server()
                .interface::<_, FakeSniWatcher>("/StatusNotifierWatcher")
                .await
                .expect("get FakeSniWatcher interface");
            iface_ref.get().await.registered_status_notifier_items()
        });
        assert_eq!(
            registered.len(),
            1,
            "expected exactly 1 StatusNotifierItem registered, got {registered:?}"
        );
        assert!(
            registered[0].starts_with("org.kde.StatusNotifierItem-"),
            "unexpected SNI service name: {:?}",
            registered[0]
        );

        let (calls, summary, desktop_entry) = block_on(async {
            let iface_ref = notif_conn
                .object_server()
                .interface::<_, FakeNotifications>("/org/freedesktop/Notifications")
                .await
                .expect("get FakeNotifications interface");
            let guard = iface_ref.get().await;
            (
                guard.calls.load(Ordering::SeqCst),
                guard.last_summary.lock().expect("lock").clone(),
                guard.last_desktop_entry.lock().expect("lock").clone(),
            )
        });
        assert_eq!(calls, 1, "expected exactly 1 Notify call");
        assert_eq!(desktop_entry, "net.daester.Kestic");
        assert!(!summary.is_empty());
        return;
    }

    run_isolated_dbus_test(
        "ac_m0_07_2_daemon_registers_sni_and_sends_single_notify",
        INNER_ENV,
    );
}
