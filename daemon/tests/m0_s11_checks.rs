// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Isolated D-Bus integration tests for M0-S11:
//! - AC-M0-11-2: `org.kde.JobViewServerV2` / `org.kde.JobViewV3` progress + Cancel
//! - AC-M0-11-3: `kestic-symbolic` SVG `ColorScheme-Text` + custom `IconThemePath` on SNI
//! - AC-M0-11-4: `notify-rust` `org.freedesktop.Notifications` with `desktop-entry` + event hint
//! - AC-M0-11-5: `org.kde.StatusNotifierItem` `Status` transitions (`Passive` <-> `Active` <-> `NeedsAttention`)

use kestic_daemon::job_view::{CAPABILITY_KILLABLE, JobProgress, JobViewHandle};
use kestic_daemon::notifications::{KESTIC_DESKTOP_ENTRY, KesticEvent, send_event_notification};
use kestic_daemon::tray::{KesticTrayState, TrayHandle, TrayStatus};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
};
use std::time::{Duration, Instant};
use zbus::object_server::SignalEmitter;
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

fn run_isolated_dbus_test(test_name: &str, env_key: &str) {
    let root = workspace_root();
    let temp = root.join(format!(
        "target/test-tmp-m0-s11-{}-{}",
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
struct JobViewRecorder {
    desktop_entry: Mutex<String>,
    capabilities: AtomicI32,
    immediate: AtomicBool,
    last_percent: AtomicU32,
    last_processed_bytes: Mutex<u64>,
    last_info_message: Mutex<String>,
    terminated: AtomicBool,
    terminate_code: AtomicU32,
}

struct FakeJobViewServerV2 {
    recorder: Arc<JobViewRecorder>,
    view_path: OwnedObjectPath,
}

#[interface(name = "org.kde.JobViewServerV2")]
impl FakeJobViewServerV2 {
    #[zbus(name = "requestView")]
    fn request_view(
        &self,
        desktop_entry: &str,
        capabilities: i32,
        hints: HashMap<String, OwnedVariant>,
    ) -> OwnedObjectPath {
        *self.recorder.desktop_entry.lock().expect("lock") = desktop_entry.to_string();
        self.recorder
            .capabilities
            .store(capabilities, Ordering::SeqCst);
        if let Some(imm) = hints.get("immediate").and_then(|v| bool::try_from(v).ok()) {
            self.recorder.immediate.store(imm, Ordering::SeqCst);
        }
        self.view_path.clone()
    }
}

struct FakeJobViewV3 {
    recorder: Arc<JobViewRecorder>,
}

#[interface(name = "org.kde.JobViewV3")]
impl FakeJobViewV3 {
    #[zbus(name = "update")]
    fn update(&self, properties: HashMap<String, OwnedVariant>) {
        if let Some(pct) = properties
            .get("percent")
            .and_then(|v| u32::try_from(v).ok())
        {
            self.recorder.last_percent.store(pct, Ordering::SeqCst);
        }
        if let Some(bytes) = properties
            .get("processedBytes")
            .and_then(|v| u64::try_from(v).ok())
        {
            *self.recorder.last_processed_bytes.lock().expect("lock") = bytes;
        }
        if let Some(msg) = properties
            .get("infoMessage")
            .and_then(|v| <&str>::try_from(v).ok())
        {
            *self.recorder.last_info_message.lock().expect("lock") = msg.to_string();
        }
    }

    #[zbus(name = "terminate")]
    fn terminate(
        &self,
        error_code: u32,
        _error_message: &str,
        _hints: HashMap<String, OwnedVariant>,
    ) {
        self.recorder
            .terminate_code
            .store(error_code, Ordering::SeqCst);
        self.recorder.terminated.store(true, Ordering::SeqCst);
    }

    #[zbus(signal, name = "cancelRequested")]
    async fn cancel_requested(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// AC-M0-11-2 (FR-53, U5): Verifies `org.kde.JobViewServerV2.requestView` -> `org.kde.JobViewV3.update`,
/// `cancelRequested` signal handling, and `terminate` over D-Bus in an isolated `dbus-run-session`.
#[test]
fn ac_m0_11_2_job_view_server_v2_progress_and_cancel_in_isolated_dbus() {
    const INNER_ENV: &str = "KESTIC_M0_S11_JOBVIEW_INNER";
    if std::env::var_os(INNER_ENV).as_deref() == Some("1".as_ref()) {
        let recorder = Arc::new(JobViewRecorder::default());
        let view_path =
            OwnedObjectPath::try_from("/JobViewServer/JobView_1").expect("valid view path");

        let server_iface = FakeJobViewServerV2 {
            recorder: Arc::clone(&recorder),
            view_path: view_path.clone(),
        };
        let view_iface = FakeJobViewV3 {
            recorder: Arc::clone(&recorder),
        };

        let server_conn = block_on(async move {
            zbus::connection::Builder::session()
                .name("net.daester.Kestic.FakeJobViewServer")
                .serve_at("/JobViewServer", server_iface)
                .serve_at(view_path, view_iface)
                .build()
                .await
                .expect("build fake job view server connection")
        });

        let handle = JobViewHandle::request(
            "net.daester.Kestic.FakeJobViewServer",
            KESTIC_DESKTOP_ENTRY,
            "Backing up Personal folder",
            CAPABILITY_KILLABLE,
        )
        .expect("request job view");

        assert_eq!(handle.view_path(), "/JobViewServer/JobView_1");
        assert_eq!(
            recorder.desktop_entry.lock().expect("lock").as_str(),
            KESTIC_DESKTOP_ENTRY
        );
        assert_eq!(recorder.capabilities.load(Ordering::SeqCst), 1);
        assert!(recorder.immediate.load(Ordering::SeqCst));

        handle
            .update(&JobProgress {
                percent: 42,
                processed_bytes: 420_000_000,
                total_bytes: 1_000_000_000,
                speed_bytes_per_sec: 7_000_000,
                info_message: "Personal folder: 42 % · about 4 minutes left".to_string(),
                elapsed_ms: 12_000,
            })
            .expect("send progress update");

        // Allow fire-and-forget no-reply update to arrive at server task.
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(recorder.last_percent.load(Ordering::SeqCst), 42);
        assert_eq!(
            *recorder.last_processed_bytes.lock().expect("lock"),
            420_000_000
        );
        assert_eq!(
            recorder.last_info_message.lock().expect("lock").as_str(),
            "Personal folder: 42 % · about 4 minutes left"
        );

        let wait_conn = server_conn.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            block_on(async move {
                let iface_ref = wait_conn
                    .object_server()
                    .interface::<_, FakeJobViewV3>("/JobViewServer/JobView_1")
                    .await
                    .expect("get FakeJobViewV3 interface");
                FakeJobViewV3::cancel_requested(iface_ref.signal_emitter())
                    .await
                    .expect("emit cancelRequested");
            });
        });

        let canceled = handle
            .wait_cancel_requested(Duration::from_secs(2))
            .expect("listen for cancelRequested");
        assert!(canceled, "JobViewHandle must observe cancelRequested");

        handle.terminate(0, "").expect("terminate job view");
        std::thread::sleep(Duration::from_millis(40));
        assert!(recorder.terminated.load(Ordering::SeqCst));
        assert_eq!(recorder.terminate_code.load(Ordering::SeqCst), 0);
        return;
    }

    run_isolated_dbus_test(
        "ac_m0_11_2_job_view_server_v2_progress_and_cancel_in_isolated_dbus",
        INNER_ENV,
    );
}

#[derive(Default)]
struct NotificationCapture {
    app_name: Mutex<String>,
    summary: Mutex<String>,
    desktop_entry: Mutex<String>,
    event_id: Mutex<String>,
    urgency: Mutex<Option<u8>>,
    calls: AtomicU32,
}

struct FakeNotificationsServer {
    capture: Arc<NotificationCapture>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl FakeNotificationsServer {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        _replaces_id: u32,
        _app_icon: &str,
        summary: &str,
        _body: &str,
        _actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        _expire_timeout: i32,
    ) -> u32 {
        *self.capture.app_name.lock().expect("lock") = app_name.to_string();
        *self.capture.summary.lock().expect("lock") = summary.to_string();
        if let Some(entry) = hints
            .get("desktop-entry")
            .and_then(|v| <&str>::try_from(v).ok())
        {
            *self.capture.desktop_entry.lock().expect("lock") = entry.to_string();
        }
        if let Some(ev) = hints
            .get("x-kde-eventId")
            .and_then(|v| <&str>::try_from(v).ok())
        {
            *self.capture.event_id.lock().expect("lock") = ev.to_string();
        }
        if let Some(urg) = hints.get("urgency").and_then(|v| u8::try_from(v).ok()) {
            *self.capture.urgency.lock().expect("lock") = Some(urg);
        }
        self.capture.calls.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".to_string(), "actions".to_string()]
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

/// AC-M0-11-4 (U6, U7): Verifies `notify-rust 4.19.0` sends `desktop-entry`, `x-kde-eventId`,
/// and `Urgency::Normal` over `org.freedesktop.Notifications` inside `dbus-run-session`.
#[test]
fn ac_m0_11_4_notify_rust_sends_desktop_entry_and_event_hint_in_isolated_dbus() {
    const INNER_ENV: &str = "KESTIC_M0_S11_NOTIFY_INNER";
    if std::env::var_os(INNER_ENV).as_deref() == Some("1".as_ref()) {
        let capture = Arc::new(NotificationCapture::default());
        let iface = FakeNotificationsServer {
            capture: Arc::clone(&capture),
        };

        let _conn = block_on(async move {
            zbus::connection::Builder::session()
                .name("org.freedesktop.Notifications")
                .serve_at("/org/freedesktop/Notifications", iface)
                .build()
                .await
                .expect("build fake notifications server")
        });

        let id = send_event_notification(
            KesticEvent::BackupFailed,
            "Projects backup failed",
            "S3 rejected the saved password.",
        )
        .expect("send notification via notify-rust");
        assert_eq!(id, 1);
        assert_eq!(capture.calls.load(Ordering::SeqCst), 1);
        assert_eq!(capture.app_name.lock().expect("lock").as_str(), "Kestic");
        assert_eq!(
            capture.desktop_entry.lock().expect("lock").as_str(),
            KESTIC_DESKTOP_ENTRY
        );
        assert_eq!(
            capture.event_id.lock().expect("lock").as_str(),
            "BackupFailed"
        );
        assert_eq!(*capture.urgency.lock().expect("lock"), Some(1));
        return;
    }

    run_isolated_dbus_test(
        "ac_m0_11_4_notify_rust_sends_desktop_entry_and_event_hint_in_isolated_dbus",
        INNER_ENV,
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

fn read_sni_string_prop(conn: &zbus::Connection, dest: &str, prop: &str) -> String {
    block_on(async {
        let reply = conn
            .call_method(
                Some(dest),
                "/StatusNotifierItem",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.kde.StatusNotifierItem", prop),
            )
            .await
            .expect("call org.freedesktop.DBus.Properties.Get on StatusNotifierItem");
        let val: OwnedVariant = reply.body().deserialize().expect("deserialize SNI variant");
        <&str>::try_from(&val)
            .expect("SNI property must be string")
            .to_string()
    })
}

/// AC-M0-11-3 & AC-M0-11-5 (K1): Verifies `kestic-symbolic.svg` uses Breeze `ColorScheme-Text`
/// CSS recoloring classes, `IconThemePath` is exported over `org.kde.StatusNotifierItem`, and
/// `Status` dynamically transitions `Passive` -> `Active` -> `NeedsAttention` -> `Passive`.
#[test]
fn ac_m0_11_3_and_11_5_sni_icon_theme_path_and_status_transitions() {
    const INNER_ENV: &str = "KESTIC_M0_S11_SNI_STATUS_INNER";
    if std::env::var_os(INNER_ENV).as_deref() == Some("1".as_ref()) {
        let icons_dir = workspace_root().join("packaging/icons");
        let svg_path = icons_dir.join("hicolor/scalable/status/kestic-symbolic.svg");
        let svg = fs::read_to_string(&svg_path).expect("read kestic-symbolic.svg");
        assert!(
            svg.contains(".ColorScheme-Text") && svg.contains("currentColor"),
            "kestic-symbolic.svg must use Breeze ColorScheme-Text + currentColor"
        );

        let watcher = FakeSniWatcher::default();
        let watcher_conn = block_on(async {
            zbus::connection::Builder::session()
                .name("org.kde.StatusNotifierWatcher")
                .serve_at("/StatusNotifierWatcher", watcher)
                .build()
                .await
                .expect("build fake StatusNotifierWatcher")
        });

        let icons_str = icons_dir.to_string_lossy().into_owned();
        let tray = TrayHandle::spawn(
            KesticTrayState {
                status: TrayStatus::Passive,
                icon_theme_path: icons_str.clone(),
                ..Default::default()
            },
            false,
            None,
        )
        .expect("spawn ksni tray");

        let deadline = Instant::now() + Duration::from_secs(5);
        let sni_service = loop {
            let items = block_on(async {
                let iface = watcher_conn
                    .object_server()
                    .interface::<_, FakeSniWatcher>("/StatusNotifierWatcher")
                    .await
                    .expect("get FakeSniWatcher");
                iface.get().await.registered_status_notifier_items()
            });
            if let Some(first) = items.into_iter().next() {
                break first;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for ksni to register StatusNotifierItem"
            );
            std::thread::sleep(Duration::from_millis(20));
        };

        let client_conn =
            block_on(zbus::Connection::session()).expect("open session D-Bus connection");
        assert_eq!(
            read_sni_string_prop(&client_conn, &sni_service, "IconName"),
            "kestic-symbolic"
        );
        assert_eq!(
            read_sni_string_prop(&client_conn, &sni_service, "IconThemePath"),
            icons_str
        );
        assert_eq!(
            read_sni_string_prop(&client_conn, &sni_service, "Status"),
            "Passive"
        );

        tray.update_state(TrayStatus::Active, "media-playback-start", "Backing up");
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(
            read_sni_string_prop(&client_conn, &sni_service, "Status"),
            "Active"
        );
        assert_eq!(
            read_sni_string_prop(&client_conn, &sni_service, "OverlayIconName"),
            "media-playback-start"
        );

        tray.update_state(TrayStatus::NeedsAttention, "dialog-error", "Backup failed");
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(
            read_sni_string_prop(&client_conn, &sni_service, "Status"),
            "NeedsAttention"
        );

        tray.update_state(TrayStatus::Passive, "", "Backups ready");
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(
            read_sni_string_prop(&client_conn, &sni_service, "Status"),
            "Passive"
        );

        tray.shutdown();
        return;
    }

    run_isolated_dbus_test(
        "ac_m0_11_3_and_11_5_sni_icon_theme_path_and_status_transitions",
        INNER_ENV,
    );
}
