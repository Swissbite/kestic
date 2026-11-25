// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(dead_code)]

pub mod fakes;
pub mod m3;
pub mod m4;
pub mod m4_restore;
pub mod po_translator;

pub use kestic_test_support::bus::PrivateDbusGuard;
pub use kestic_test_support::workspace::find_kestic_binary;
use kestic_test_support::{gui, tools};
use tools::{dev_tools_bin_dir, rest_server_binary};

use kestic_daemon::secrets::SecretServiceStore;
use kestic_daemon::service::DaemonRunConfig;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
use zbus::{Connection, OwnedVariant, Structure, Variant, block_on, interface};

/// Connects a D-Bus client to the daemon under test on the current (private) session bus.
pub fn connect_default_client() -> zbus::Result<kestic_daemon::dbus_api::KesticClient> {
    kestic_daemon::dbus_api::KesticClient::connect(
        kestic_daemon::dbus_api::KESTIC_BUS_NAME,
        kestic_daemon::dbus_api::KESTIC_OBJECT_PATH,
    )
}

/// Loads an existing `config.toml` through the production loader (`load_optional_config`).
pub fn load_config(
    path: &Path,
) -> Result<kestic_core::config::AppConfig, kestic_core::config::ConfigError> {
    kestic_core::config::load_optional_config(path)?.ok_or_else(|| {
        kestic_core::config::ConfigError::Io(std::io::Error::from(std::io::ErrorKind::NotFound))
    })
}

/// Resolves restic 0.18.0 and latest test binaries (`AC-M1-Q-2`).
pub fn restic_test_binaries() -> Vec<(&'static str, PathBuf)> {
    tools::resolve_restic_binaries()
}

/// Re-execs the current test (`test_name --exact`) inside an isolated subprocess with its own
/// private `dbus-daemon`, an unlocked headless keyring on it, isolated XDG directories and
/// resolved restic tools.
///
/// This guarantees zero process-global environment races across parallel Cargo test threads (`AC-M1-Q-1`).
pub fn run_isolated_test(test_name: &str, inner_env: &str, body: impl FnOnce(&Path)) {
    run_isolated(test_name, inner_env, false, body);
}

/// Name of the folder below the test root that [`run_isolated_test_with_system_programs`]
/// puts first in `PATH`.
pub const SYSTEM_PROGRAMS_DIR: &str = "system-programs";

/// [`run_isolated_test`] with `<root>/system-programs` first in the child's `PATH`, so programs
/// written there (a system restic of another version) are the ones the daemon finds.
pub fn run_isolated_test_with_system_programs(
    test_name: &str,
    inner_env: &str,
    body: impl FnOnce(&Path),
) {
    run_isolated(test_name, inner_env, true, body);
}

fn run_isolated(
    test_name: &str,
    inner_env: &str,
    system_programs_first: bool,
    body: impl FnOnce(&Path),
) {
    if env::var_os(inner_env).as_deref() == Some("1".as_ref()) {
        let root = PathBuf::from(env::var_os("KESTIC_TEST_ROOT").expect("KESTIC_TEST_ROOT set"));
        body(&root);
        return;
    }

    let dirs = gui::IsolatedEnv::create(inner_env, None);
    let dbus = PrivateDbusGuard::start();
    let _keyring = dirs.start_keyring(&dbus.address);
    let tools_dir = dev_tools_bin_dir();
    let restic_bins = restic_test_binaries();
    let restic_bin = |label: &str| {
        restic_bins
            .iter()
            .find(|(l, _)| *l == label)
            .unwrap_or_else(|| panic!("restic binary `{label}` resolved"))
            .1
            .clone()
    };
    let rest_server = rest_server_binary();

    let current_exe = env::current_exe().expect("current test executable");
    let mut cmd = Command::new(&current_exe);
    cmd.arg(test_name)
        .arg("--exact")
        .env_remove("GNOME_KEYRING_CONTROL")
        .env_remove("SSH_AUTH_SOCK")
        .env(inner_env, "1")
        .env("DBUS_SESSION_BUS_ADDRESS", &dbus.address)
        .env("HOME", &dirs.home)
        .env("XDG_CONFIG_HOME", &dirs.cfg)
        .env("XDG_DATA_HOME", &dirs.data)
        .env("XDG_CACHE_HOME", &dirs.cache)
        .env("RESTIC_CACHE_DIR", &dirs.cache)
        .env("XDG_STATE_HOME", &dirs.state)
        .env("XDG_RUNTIME_DIR", &dirs.run)
        .env(ISOLATED_BUS_ENV, &dbus.address)
        .env("KESTIC_TEST_ROOT", &dirs.root)
        .env("KESTIC_TOOLS_DIR", &tools_dir)
        .env("KESTIC_RESTIC_0_18", restic_bin("restic-0.18.0"))
        .env("KESTIC_RESTIC_LATEST", restic_bin("restic-latest"))
        .env("KESTIC_REST_SERVER", &rest_server)
        .env("LC_ALL", "C.UTF-8")
        .env("LANG", "C.UTF-8")
        .env("LANGUAGE", "en");
    if system_programs_first {
        let programs = dirs.root.join(SYSTEM_PROGRAMS_DIR);
        std::fs::create_dir_all(&programs).expect("mkdir system programs");
        let mut path = vec![programs];
        path.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
        cmd.env("PATH", env::join_paths(path).expect("PATH"));
    }

    let out = cmd.output().expect("run isolated test child");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "isolated test `{test_name}` failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("1 passed"),
        "isolated test `{test_name}` did not match/execute exactly 1 test:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// Private session bus address of the isolated test, as handed to the re-exec'd child.
const ISOLATED_BUS_ENV: &str = "KESTIC_TEST_ISOLATED_BUS";

/// The Secret Service store of the isolated test: the headless keyring [`run_isolated_test`]
/// started on the private session bus. Panics anywhere else, so no test reaches the user's
/// real wallet.
pub fn isolated_secret_store() -> SecretServiceStore {
    let isolated_bus = env::var(ISOLATED_BUS_ENV)
        .expect("isolated_secret_store() is only available inside run_isolated_test");
    assert_eq!(
        env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref(),
        Some(isolated_bus.as_str()),
        "the session bus must be the private bus of the isolated test"
    );
    SecretServiceStore::default()
}

/// Asks the in-process daemon behind `client` to quit, asserts a clean shutdown and waits until
/// it released the default bus name.
pub fn stop_daemon(
    client: &kestic_daemon::dbus_api::KesticClient,
    handle: thread::JoinHandle<Result<kestic_daemon::service::DaemonOutcome, String>>,
) {
    client.confirm_quit().expect("confirm quit");
    assert_eq!(
        handle.join().expect("join daemon"),
        Ok(kestic_daemon::service::DaemonOutcome::RanAndStopped)
    );
    wait_for_bus_name_released(
        client.connection(),
        kestic_daemon::dbus_api::KESTIC_BUS_NAME,
        Duration::from_secs(3),
    );
}

/// Daemon configuration inside the isolated test tree `root`: config and state below `root`, the
/// isolated test keyring, the real `kestic` binary, a ready file `<root>/daemon.ready`, and no
/// JobView, portal inhibit or UI spawn.
pub fn sandbox_run_config(root: &Path) -> DaemonRunConfig {
    DaemonRunConfig {
        ready_file: Some(root.join("daemon.ready")),
        config_path: Some(root.join("cfg").join("kestic").join("config.toml")),
        state_dir: Some(root.join("state").join("kestic")),
        kestic_binary: Some(find_kestic_binary()),
        secret_store: Some(Arc::new(isolated_secret_store())),
        suppress_ui_spawn: true,
        enable_job_view: false,
        enable_portal_inhibit: false,
        ..Default::default()
    }
}

/// Captured call to `org.freedesktop.Notifications.Notify`.
#[derive(Debug, Clone)]
pub struct CapturedNotification {
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub desktop_entry: Option<String>,
    pub event_id: Option<String>,
    pub urgency: Option<u8>,
}

/// Thread-safe store of captured `Notify` calls.
#[derive(Default)]
pub struct NotificationCapture {
    pub calls: Mutex<Vec<CapturedNotification>>,
    counter: AtomicU32,
}

impl NotificationCapture {
    pub fn snapshot(&self) -> Vec<CapturedNotification> {
        self.calls.lock().expect("lock notifications").clone()
    }

    pub fn clear(&self) {
        self.calls.lock().expect("lock notifications").clear();
    }
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
        body: &str,
        _actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        _expire_timeout: i32,
    ) -> u32 {
        let desktop_entry = hints
            .get("desktop-entry")
            .and_then(|v| <&str>::try_from(v).ok())
            .map(ToOwned::to_owned);
        let event_id = hints
            .get("x-kde-eventId")
            .and_then(|v| <&str>::try_from(v).ok())
            .map(ToOwned::to_owned);
        let urgency = hints.get("urgency").and_then(|v| u8::try_from(v).ok());

        self.capture
            .calls
            .lock()
            .expect("lock notifications")
            .push(CapturedNotification {
                app_name: app_name.to_string(),
                summary: summary.to_string(),
                body: body.to_string(),
                desktop_entry,
                event_id,
                urgency,
            });
        self.capture.counter.fetch_add(1, Ordering::SeqCst) + 1
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

/// Starts `org.freedesktop.Notifications` fake on the current session bus.
pub fn start_fake_notifications() -> (Connection, Arc<NotificationCapture>) {
    let capture = Arc::new(NotificationCapture::default());
    let iface = FakeNotificationsServer {
        capture: Arc::clone(&capture),
    };
    let conn = block_on(async move {
        zbus::connection::Builder::session()
            .name("org.freedesktop.Notifications")
            .serve_at("/org/freedesktop/Notifications", iface)
            .build()
            .await
            .expect("build fake org.freedesktop.Notifications")
    });
    (conn, capture)
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
            .expect("lock sni items")
            .push(service.to_string());
    }

    #[zbus(name = "RegisterStatusNotifierHost")]
    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property, name = "RegisteredStatusNotifierItems")]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.registered_items
            .lock()
            .expect("lock sni items")
            .clone()
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

/// Starts `org.kde.StatusNotifierWatcher` fake on the current session bus.
pub fn start_fake_sni_watcher() -> Connection {
    let watcher = FakeSniWatcher::default();
    block_on(async move {
        zbus::connection::Builder::session()
            .name("org.kde.StatusNotifierWatcher")
            .serve_at("/StatusNotifierWatcher", watcher)
            .build()
            .await
            .expect("build fake StatusNotifierWatcher")
    })
}

/// Waits until at least one `StatusNotifierItem` is registered on `FakeSniWatcher` and returns its bus name.
pub fn wait_for_sni_service(watcher_conn: &Connection, timeout: Duration) -> String {
    wait_for_sni_service_at(watcher_conn, 0, timeout)
}

/// Waits until `registered_status_notifier_items().len() > index` and returns that item's bus name.
pub fn wait_for_sni_service_at(
    watcher_conn: &Connection,
    index: usize,
    timeout: Duration,
) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let items = block_on(async {
            let iface = watcher_conn
                .object_server()
                .interface::<_, FakeSniWatcher>("/StatusNotifierWatcher")
                .await
                .expect("get FakeSniWatcher interface");
            iface.get().await.registered_status_notifier_items()
        });
        if let Some(item) = items.into_iter().nth(index) {
            return item;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for StatusNotifierItem registration at index {index}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// Reads a string property from `org.kde.StatusNotifierItem` on `sni_service`.
pub fn read_sni_string_prop(conn: &Connection, sni_service: &str, prop: &str) -> String {
    block_on(async {
        let reply = conn
            .call_method(
                Some(sni_service),
                "/StatusNotifierItem",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.kde.StatusNotifierItem", prop),
            )
            .await
            .expect("call Properties.Get on StatusNotifierItem");
        let val: OwnedVariant = reply.body().deserialize().expect("deserialize SNI variant");
        <&str>::try_from(&val)
            .expect("SNI property must be string")
            .to_string()
    })
}

type SniToolTipTuple = (String, Vec<(i32, i32, Vec<u8>)>, String, String);
type DbusMenuLayoutNode = (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>);

/// Reads the `ToolTip` `(title, description)` from `org.kde.StatusNotifierItem`.
pub fn read_sni_tooltip(conn: &Connection, sni_service: &str) -> (String, String) {
    block_on(async {
        let reply = conn
            .call_method(
                Some(sni_service),
                "/StatusNotifierItem",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.kde.StatusNotifierItem", "ToolTip"),
            )
            .await
            .expect("call Properties.Get(ToolTip) on StatusNotifierItem");
        let val: OwnedVariant = reply
            .body()
            .deserialize()
            .expect("deserialize ToolTip variant");
        let (_icon_name, _pixmap, title, description): SniToolTipTuple =
            (&*val).try_into().expect("decode SNI ToolTip tuple");
        (title, description)
    })
}

/// Flattened `com.canonical.dbusmenu` item from `GetLayout`.
#[derive(Debug, Clone)]
pub struct TrayMenuEntry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    /// dbusmenu `type`: `separator`, or empty for a standard item.
    pub kind: String,
}

/// Calls `com.canonical.dbusmenu.AboutToShow(0)` followed by `GetLayout(0, -1, [])` on `/MenuBar`.
pub fn fetch_dbusmenu_items(conn: &Connection, sni_service: &str) -> Vec<TrayMenuEntry> {
    block_on(async {
        let menu_path = "/MenuBar";
        let _about: bool = conn
            .call_method(
                Some(sni_service),
                menu_path,
                Some("com.canonical.dbusmenu"),
                "AboutToShow",
                &(0i32,),
            )
            .await
            .expect("call com.canonical.dbusmenu.AboutToShow")
            .body()
            .deserialize()
            .expect("deserialize AboutToShow");

        let reply = conn
            .call_method(
                Some(sni_service),
                menu_path,
                Some("com.canonical.dbusmenu"),
                "GetLayout",
                &(0i32, -1i32, Vec::<&str>::new()),
            )
            .await
            .expect("call com.canonical.dbusmenu.GetLayout");

        let (_revision, root): (u32, DbusMenuLayoutNode) =
            reply.body().deserialize().expect("deserialize GetLayout");
        let mut out = Vec::new();
        collect_menu_tuple(root.0, &root.1, &root.2, &mut out);
        out
    })
}

/// Triggers `com.canonical.dbusmenu.Event(item_id, "clicked", 0, 0)` on `/MenuBar`.
pub fn click_dbusmenu_item(conn: &Connection, sni_service: &str, item_id: i32) {
    block_on(async {
        conn.call_method(
            Some(sni_service),
            "/MenuBar",
            Some("com.canonical.dbusmenu"),
            "Event",
            &(item_id, "clicked", Variant::from(0i32), 0u32),
        )
        .await
        .expect("call com.canonical.dbusmenu.Event");
    });
}

fn unwrap_variant_str(v: &Variant<'_>) -> Option<String> {
    match v {
        Variant::Variant(inner) => unwrap_variant_str(inner),
        _ => <&str>::try_from(v).ok().map(ToOwned::to_owned),
    }
}

fn unwrap_variant_bool(v: &Variant<'_>) -> Option<bool> {
    match v {
        Variant::Variant(inner) => unwrap_variant_bool(inner),
        _ => bool::try_from(v).ok(),
    }
}

fn collect_menu_tuple(
    id: i32,
    props: &HashMap<String, OwnedVariant>,
    children: &[OwnedVariant],
    out: &mut Vec<TrayMenuEntry>,
) {
    let label = props
        .get("label")
        .and_then(|v| unwrap_variant_str(v))
        .unwrap_or_default();
    let enabled = props
        .get("enabled")
        .and_then(|v| unwrap_variant_bool(v))
        .unwrap_or(true);
    let kind = props
        .get("type")
        .and_then(|v| unwrap_variant_str(v))
        .unwrap_or_default();

    if id != 0 {
        out.push(TrayMenuEntry {
            id,
            label,
            enabled,
            kind,
        });
    }

    for child in children {
        collect_menu_variant(child, out);
    }
}

fn collect_menu_variant(node: &Variant<'_>, out: &mut Vec<TrayMenuEntry>) {
    let inner = match node {
        Variant::Variant(v) => &**v,
        other => other,
    };
    let Ok(s) = <&Structure>::try_from(inner) else {
        return;
    };
    let fields = s.fields();
    if fields.len() < 3 {
        return;
    }

    let id = i32::try_from(&fields[0]).unwrap_or(0);
    let mut label = String::new();
    let mut enabled = true;
    let mut kind = String::new();

    if let Variant::Dict(dict) = &fields[1] {
        for (k, v) in dict.iter() {
            if let Ok(key) = <&str>::try_from(k) {
                match key {
                    "label" => {
                        if let Some(s_val) = unwrap_variant_str(v) {
                            label = s_val;
                        }
                    }
                    "enabled" => {
                        if let Some(b_val) = unwrap_variant_bool(v) {
                            enabled = b_val;
                        }
                    }
                    "type" => {
                        if let Some(s_val) = unwrap_variant_str(v) {
                            kind = s_val;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if id != 0 {
        out.push(TrayMenuEntry {
            id,
            label,
            enabled,
            kind,
        });
    }

    if let Variant::Array(children) = &fields[2] {
        for child in children.iter() {
            collect_menu_variant(child, out);
        }
    }
}

/// Polls until `ready_file` appears on disk.
/// Upper bound for a test daemon to write its ready file. Generous because
/// startup (SQLite with fsync) can queue behind other tests' disk writeback
/// under full `just verify` load; waits return as soon as the file exists.
pub const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(20);

pub fn wait_for_file(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for file {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// Polls `org.freedesktop.DBus.NameHasOwner` until `bus_name` is no longer owned.
pub fn wait_for_bus_name_released(conn: &Connection, bus_name: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let owned = block_on(async {
            conn.call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "NameHasOwner",
                &(bus_name,),
            )
            .await
            .ok()?
            .body()
            .deserialize::<bool>()
            .ok()
        })
        .unwrap_or(false);
        if !owned {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for D-Bus name `{bus_name}` to be released"
        );
        thread::sleep(Duration::from_millis(15));
    }
}

/// Captured D-Bus signal from `net.daester.Kestic1`.
#[derive(Debug, Clone)]
pub enum CapturedKesticSignal {
    StatusChanged(String),
    RunUpdated(String),
    RunProgress(kestic_daemon::dbus_api::ActiveRunProgressDto),
    ManualRunBlocked {
        backup_id: String,
        reason: String,
    },
    ShowWindowRequested,
    ShowAboutRequested,
    ShowRestoreRequested,
    QuitConfirmationRequested {
        has_running_run: bool,
        running_backup_name: String,
    },
}

/// Subscribes synchronously to all signals on `net.daester.Kestic1` before actions trigger,
/// collecting them in arrival order on a background thread.
pub struct KesticSignalCollector {
    signals: Arc<Mutex<Vec<CapturedKesticSignal>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    stop_waker: Arc<Mutex<Option<std::task::Waker>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl KesticSignalCollector {
    pub fn start(bus_name: &str, object_path: &str, interface_name: &str) -> Self {
        use zbus::export::ordered_stream::OrderedStreamExt;

        let bus_owned = bus_name.to_string();
        let path_owned = object_path.to_string();
        let iface_owned = interface_name.to_string();

        let (tx_ready, rx_ready) = std::sync::mpsc::channel::<()>();
        let signals = Arc::new(Mutex::new(Vec::new()));
        let signals_bg = Arc::clone(&signals);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);
        let stop_waker: Arc<Mutex<Option<std::task::Waker>>> = Arc::new(Mutex::new(None));
        let stop_waker_bg = Arc::clone(&stop_waker);

        let handle = thread::spawn(move || {
            block_on(async move {
                let conn = Connection::session().await.expect("signal session conn");
                let proxy = zbus::Proxy::new(
                    &conn,
                    bus_owned.as_str(),
                    path_owned.as_str(),
                    iface_owned.as_str(),
                )
                .await
                .expect("signal proxy");
                let mut stream = proxy
                    .receive_all_signals()
                    .await
                    .expect("subscribe receive_all_signals");
                let _ = tx_ready.send(());

                while !stop_bg.load(Ordering::SeqCst) {
                    let next_msg = {
                        let mut fut = std::pin::pin!(stream.next());
                        std::future::poll_fn(|cx| {
                            if stop_bg.load(Ordering::SeqCst) {
                                return std::task::Poll::Ready(None);
                            }
                            *stop_waker_bg.lock().expect("lock stop waker") =
                                Some(cx.waker().clone());
                            if stop_bg.load(Ordering::SeqCst) {
                                return std::task::Poll::Ready(None);
                            }
                            fut.as_mut().poll(cx)
                        })
                        .await
                    };

                    let Some(msg) = next_msg else {
                        break;
                    };
                    let header = msg.header();
                    let member = header.member().map_or("", |m| m.as_str());
                    let parsed = match member {
                        "StatusChanged" => msg
                            .body()
                            .deserialize::<String>()
                            .ok()
                            .map(CapturedKesticSignal::StatusChanged),
                        "RunUpdated" => msg
                            .body()
                            .deserialize::<String>()
                            .ok()
                            .map(CapturedKesticSignal::RunUpdated),
                        "RunProgress" => msg
                            .body()
                            .deserialize::<String>()
                            .ok()
                            .and_then(|json| serde_json::from_str(&json).ok())
                            .map(CapturedKesticSignal::RunProgress),
                        "ManualRunBlocked" => {
                            msg.body().deserialize::<(String, String)>().ok().map(
                                |(backup_id, reason)| CapturedKesticSignal::ManualRunBlocked {
                                    backup_id,
                                    reason,
                                },
                            )
                        }
                        "ShowWindowRequested" => Some(CapturedKesticSignal::ShowWindowRequested),
                        "ShowAboutRequested" => Some(CapturedKesticSignal::ShowAboutRequested),
                        "ShowRestoreRequested" => Some(CapturedKesticSignal::ShowRestoreRequested),
                        "QuitConfirmationRequested" => {
                            msg.body().deserialize::<(bool, String)>().ok().map(
                                |(has_running_run, running_backup_name)| {
                                    CapturedKesticSignal::QuitConfirmationRequested {
                                        has_running_run,
                                        running_backup_name,
                                    }
                                },
                            )
                        }
                        _ => None,
                    };
                    if let Some(sig) = parsed {
                        signals_bg.lock().expect("lock signals").push(sig);
                    }
                }
            });
        });

        rx_ready
            .recv_timeout(Duration::from_secs(5))
            .expect("signal collector ready");

        Self {
            signals,
            stop,
            stop_waker,
            handle: Some(handle),
        }
    }

    pub fn snapshot(&self) -> Vec<CapturedKesticSignal> {
        self.signals.lock().expect("lock signals").clone()
    }

    pub fn clear(&self) {
        self.signals.lock().expect("lock signals").clear();
    }

    pub fn wait_until<F>(&self, timeout: Duration, mut pred: F) -> Vec<CapturedKesticSignal>
    where
        F: FnMut(&[CapturedKesticSignal]) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            let snap = self.snapshot();
            if pred(&snap) {
                return snap;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for expected D-Bus signal; captured so far: {snap:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for KesticSignalCollector {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(w) = self.stop_waker.lock().expect("lock stop waker").take() {
            w.wake();
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
