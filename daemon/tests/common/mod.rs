// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(dead_code)]

pub mod fakes;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
use zbus::{Connection, OwnedVariant, Structure, Variant, block_on, interface};

/// Returns the Cargo workspace root directory.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("daemon crate has workspace root parent")
        .to_path_buf()
}

/// Locates the compiled `kestic` binary (`CARGO_BIN_EXE_kestic`, `target/debug/kestic`, or `target/release/kestic`).
pub fn find_kestic_binary() -> PathBuf {
    if let Some(p) = env::var_os("CARGO_BIN_EXE_kestic").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(p);
        if path.is_file() {
            return path;
        }
    }
    let target_dir = env::var_os("CARGO_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"));
    for profile in ["debug", "release"] {
        let candidate = target_dir.join(profile).join("kestic");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "kestic binary not found under {}; build it with `cargo build --bin kestic`",
        target_dir.display()
    );
}

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

/// Resolves `restic 0.18.0` and latest (`0.19.1` / `restic-latest` / `$PATH`) test binaries (`AC-M1-Q-2`).
pub fn restic_test_binaries() -> Vec<(&'static str, PathBuf)> {
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

/// Guard removing a temporary test directory tree on `Drop`.
pub struct TempDirGuard(pub PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Isolated XDG + HOME directories (`home`, `cfg`, `data`, `cache`, `state`, `run` chmod `0700`).
pub struct IsolatedDirs {
    pub root: PathBuf,
    pub home: PathBuf,
    pub cfg: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub run: PathBuf,
    pub secret_file: PathBuf,
}

impl IsolatedDirs {
    pub fn create(prefix: &str) -> Self {
        let nonce = kestic_core::model::RunId::generate()
            .map(|r| r.as_str().to_string())
            .unwrap_or_else(|_| "00000000".to_string());
        let root = env::temp_dir().join(format!("kestic-{prefix}-{}-{nonce}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let home = root.join("home");
        let cfg = root.join("cfg");
        let data = root.join("data");
        let cache = root.join("cache");
        let state = root.join("state");
        let run = root.join("run");
        for dir in [&home, &cfg, &data, &cache, &state, &run] {
            fs::create_dir_all(dir).expect("create isolated dir");
        }
        fs::set_permissions(&run, fs::Permissions::from_mode(0o700))
            .expect("chmod 0700 XDG_RUNTIME_DIR");

        let secret_file = root.join("secrets.hex");
        Self {
            root,
            home,
            cfg,
            data,
            cache,
            state,
            run,
            secret_file,
        }
    }

    pub fn config_toml_path(&self) -> PathBuf {
        self.cfg.join("kestic").join("config.toml")
    }

    pub fn state_kestic_dir(&self) -> PathBuf {
        self.state.join("kestic")
    }
}

impl Drop for IsolatedDirs {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Private `dbus-daemon --session --fork` instance killed automatically on `Drop`.
pub struct PrivateDbusGuard {
    pub address: String,
    pub pid: u32,
}

impl PrivateDbusGuard {
    pub fn start() -> Self {
        let out = Command::new("dbus-daemon")
            .args(["--session", "--fork", "--print-address=1", "--print-pid=1"])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn private dbus-daemon");
        assert!(
            out.status.success(),
            "dbus-daemon failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).expect("utf8 dbus-daemon output");
        let mut lines = stdout.lines();
        let address = lines
            .next()
            .expect("dbus-daemon address line")
            .trim()
            .to_string();
        let pid: u32 = lines
            .next()
            .expect("dbus-daemon pid line")
            .trim()
            .parse()
            .expect("parse dbus-daemon pid");
        Self { address, pid }
    }
}

impl Drop for PrivateDbusGuard {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Re-execs the current test (`test_name --exact`) inside an isolated subprocess with its own
/// private `dbus-daemon`, isolated XDG directories, `KESTIC_SECRET_FILE`, and resolved restic tools.
///
/// This guarantees zero process-global environment races across parallel Cargo test threads (`AC-M1-Q-1`).
pub fn run_isolated_test(test_name: &str, inner_env: &str, body: impl FnOnce(&Path)) {
    if env::var_os(inner_env).as_deref() == Some("1".as_ref()) {
        let root = PathBuf::from(env::var_os("KESTIC_TEST_ROOT").expect("KESTIC_TEST_ROOT set"));
        body(&root);
        return;
    }

    let dirs = IsolatedDirs::create(inner_env);
    let dbus = PrivateDbusGuard::start();
    let tools_dir = dev_tools_bin_dir();
    let restic_bins = restic_test_binaries();
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
        .env("XDG_STATE_HOME", &dirs.state)
        .env("XDG_RUNTIME_DIR", &dirs.run)
        .env("KESTIC_SECRET_FILE", &dirs.secret_file)
        .env("KESTIC_TEST_ROOT", &dirs.root)
        .env("KESTIC_TOOLS_DIR", &tools_dir)
        .env("KESTIC_RESTIC_0_18", &restic_bins[0].1)
        .env("KESTIC_RESTIC_LATEST", &restic_bins[1].1)
        .env("KESTIC_REST_SERVER", &rest_server)
        .env("KESTIC_TEST_FAST_INIT", "1")
        .env("LC_ALL", "C.UTF-8")
        .env("LANG", "C.UTF-8")
        .env("LANGUAGE", "en");

    let out = cmd.output().expect("run isolated test child");
    assert!(
        out.status.success(),
        "isolated test `{test_name}` failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Allocates a free localhost TCP port for test servers or guaranteed-closed preflight probes.
pub fn free_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

/// Running local `rest-server` instance on a localhost TCP port (`--no-auth` or `--htpasswd-file`).
pub struct RestServerGuard {
    pub url: String,
    pub base_url: String,
    pub port: u16,
    pub data_dir: PathBuf,
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

        let base_url;
        let url;
        if let Some((user, pass)) = auth {
            let htpasswd = data_dir.join(".htpasswd");
            let entry = build_htpasswd_entry(user, pass);
            fs::write(&htpasswd, entry).expect("write .htpasswd");
            cmd.args(["--htpasswd-file", &htpasswd.to_string_lossy()]);
            base_url = format!("http://{addr}/{user}");
            url = base_url.clone();
        } else {
            cmd.arg("--no-auth");
            base_url = format!("http://{addr}");
            url = base_url.clone();
        }

        let child = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rest-server");

        let mut guard = Self {
            url,
            base_url,
            port,
            data_dir: data_dir.to_path_buf(),
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
    pub item_type: Option<String>,
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
    let item_type = props.get("type").and_then(|v| unwrap_variant_str(v));

    if id != 0 {
        out.push(TrayMenuEntry {
            id,
            label,
            enabled,
            item_type,
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
    let mut item_type = None;

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
                        if let Some(t_val) = unwrap_variant_str(v) {
                            item_type = Some(t_val);
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
            item_type,
        });
    }

    if let Variant::Array(children) = &fields[2] {
        for child in children.iter() {
            collect_menu_variant(child, out);
        }
    }
}

/// Polls until `ready_file` appears on disk.
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
    RunProgress {
        run_id: String,
        percent_done: f64,
        bytes_done: u64,
        total_bytes: u64,
        speed_bps: u64,
        seconds_remaining: u64,
    },
    ManualRunBlocked {
        backup_id: String,
        reason: String,
    },
    ShowWindowRequested,
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
                    let member = header.member().map(|m| m.as_str()).unwrap_or("");
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
                            .deserialize::<(String, f64, u64, u64, u64, u64)>()
                            .ok()
                            .map(
                                |(
                                    run_id,
                                    percent_done,
                                    bytes_done,
                                    total_bytes,
                                    speed_bps,
                                    seconds_remaining,
                                )| {
                                    CapturedKesticSignal::RunProgress {
                                        run_id,
                                        percent_done,
                                        bytes_done,
                                        total_bytes,
                                        speed_bps,
                                        seconds_remaining,
                                    }
                                },
                            ),
                        "ManualRunBlocked" => {
                            msg.body().deserialize::<(String, String)>().ok().map(
                                |(backup_id, reason)| CapturedKesticSignal::ManualRunBlocked {
                                    backup_id,
                                    reason,
                                },
                            )
                        }
                        "ShowWindowRequested" => Some(CapturedKesticSignal::ShowWindowRequested),
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
