// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! System D-Bus monitors and inhibitors (`UPower`, `NetworkManager`, `systemd-logind`, `UDisks2`)
//! with injectable bus mode (`system` vs isolated `session` fakes for `AC-M1-Q-1`) and safe
//! headless fallbacks (`M3-S03`, `M3-S04`, `M3-S09`, `M3-S13`).

use kestic_core::scheduler::SystemEnvironment;
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Poll, Waker};
use std::thread;
use std::time::Duration;
use zbus::export::futures_core::Stream;
use zbus::{
    Connection, MatchRule, MessageStream, OwnedFd, OwnedObjectPath, OwnedVariant, Proxy, block_on,
};

pub const DEFAULT_UPOWER_DEST: &str = "org.freedesktop.UPower";
pub const DEFAULT_NM_DEST: &str = "org.freedesktop.NetworkManager";
pub const DEFAULT_LOGIND_DEST: &str = "org.freedesktop.login1";
pub const DEFAULT_UDISKS2_DEST: &str = "org.freedesktop.UDisks2";

/// NetworkManager connection type of Wi-Fi profiles: only these are offered as allowed networks.
const NM_WIFI_CONNECTION_TYPE: &str = "802-11-wireless";

/// Configures which bus and service names the system monitors target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorBusConfig {
    /// When `true`, connects to the D-Bus **session** bus (for isolated fake tests).
    /// When `false` (production), connects to the D-Bus **system** bus.
    pub use_session_bus: bool,
    pub upower_dest: String,
    pub nm_dest: String,
    pub logind_dest: String,
    pub udisks2_dest: String,
    /// Enables spawning the background D-Bus signal listener thread.
    pub enable_watcher: bool,
}

impl Default for MonitorBusConfig {
    fn default() -> Self {
        Self {
            use_session_bus: false,
            upower_dest: DEFAULT_UPOWER_DEST.to_string(),
            nm_dest: DEFAULT_NM_DEST.to_string(),
            logind_dest: DEFAULT_LOGIND_DEST.to_string(),
            udisks2_dest: DEFAULT_UDISKS2_DEST.to_string(),
            enable_watcher: true,
        }
    }
}

/// Asynchronous events emitted by the system D-Bus monitor thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemMonitorEvent {
    EnvironmentChanged(SystemEnvironment),
    MountChanged,
    PrepareForSleep(bool),
}

async fn connect_target_bus(cfg: &MonitorBusConfig) -> zbus::Result<Connection> {
    let res = if cfg.use_session_bus {
        Connection::session().await
    } else {
        Connection::system().await
    };
    if let Err(ref err) = res {
        let bus_kind = if cfg.use_session_bus {
            "session"
        } else {
            "system"
        };
        eprintln!(
            "kestic-daemon: failed to connect to {bus_kind} D-Bus for system monitors: {err}"
        );
    }
    res
}

async fn name_has_owner(conn: &Connection, dest: &str) -> bool {
    let dbus = match zbus::fdo::DBusProxy::new(conn).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("kestic-daemon: failed to create DBusProxy checking `{dest}`: {err}");
            return false;
        }
    };
    let Ok(bus_name) = zbus::names::BusName::try_from(dest) else {
        eprintln!("kestic-daemon: invalid D-Bus name `{dest}`");
        return false;
    };
    match dbus.name_has_owner(bus_name).await {
        Ok(owned) => owned,
        Err(err) => {
            eprintln!("kestic-daemon: NameHasOwner failed for `{dest}`: {err}");
            false
        }
    }
}

async fn get_prop_val(
    conn: &Connection,
    dest: &str,
    path: &str,
    iface: &str,
    prop: &str,
) -> Option<OwnedVariant> {
    let proxy = match Proxy::new(conn, dest, path, "org.freedesktop.DBus.Properties").await {
        Ok(p) => p,
        Err(err) => {
            eprintln!(
                "kestic-daemon: failed to create Properties proxy for `{dest}` `{path}`: {err}"
            );
            return None;
        }
    };
    match proxy.call("Get", &(iface, prop)).await {
        Ok(v) => Some(v),
        Err(err) => {
            eprintln!("kestic-daemon: Properties.Get({iface}.{prop}) failed on `{dest}`: {err}");
            None
        }
    }
}

/// Reads current UPower & NetworkManager state asynchronously.
///
/// Falls back to [`SystemEnvironment::desktop_connected()`] when services are absent
/// (e.g., headless containers or isolated M1/M2 test buses without system fakes).
pub async fn read_system_environment_async(
    conn: &Connection,
    cfg: &MonitorBusConfig,
) -> SystemEnvironment {
    let mut env = SystemEnvironment::desktop_connected();

    if name_has_owner(conn, &cfg.upower_dest).await {
        if let Some(val) = get_prop_val(
            conn,
            &cfg.upower_dest,
            "/org/freedesktop/UPower",
            "org.freedesktop.UPower",
            "OnBattery",
        )
        .await
            && let Ok(on_bat) = bool::try_from(&val)
        {
            env.on_battery = on_bat;
        }
        if let Some(val) = get_prop_val(
            conn,
            &cfg.upower_dest,
            "/org/freedesktop/UPower/devices/DisplayDevice",
            "org.freedesktop.UPower.Device",
            "Percentage",
        )
        .await
            && let Ok(pct) = f64::try_from(&val)
        {
            env.battery_percent = Some(pct.round().clamp(0.0, 100.0) as u8);
        }
    } else if !cfg.use_session_bus {
        eprintln!(
            "kestic-daemon: UPower service `{}` is not owned on system bus; assuming AC power",
            cfg.upower_dest
        );
    }

    if name_has_owner(conn, &cfg.nm_dest).await {
        let connectivity = get_prop_val(
            conn,
            &cfg.nm_dest,
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "Connectivity",
        )
        .await
        .and_then(|v| u32::try_from(&v).ok());

        let state = get_prop_val(
            conn,
            &cfg.nm_dest,
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "State",
        )
        .await
        .and_then(|v| u32::try_from(&v).ok());

        env.network_connected = match (connectivity, state) {
            (Some(c), _) if c >= 1 => c >= 3,
            (_, Some(s)) => s >= 60,
            _ => true,
        };

        if let Some(val) = get_prop_val(
            conn,
            &cfg.nm_dest,
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "Metered",
        )
        .await
            && let Ok(metered) = u32::try_from(&val)
        {
            // NM_METERED_YES = 1, NM_METERED_GUESS_YES = 3
            env.network_metered = metered == 1 || metered == 3;
        }

        if let Some(val) = get_prop_val(
            conn,
            &cfg.nm_dest,
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "ActiveConnections",
        )
        .await
            && let Ok(paths) = Vec::<OwnedObjectPath>::try_from(val)
        {
            let mut active = Vec::new();
            for path in paths {
                if let Some(id_val) = get_prop_val(
                    conn,
                    &cfg.nm_dest,
                    path.as_str(),
                    "org.freedesktop.NetworkManager.Connection.Active",
                    "Id",
                )
                .await
                    && let Ok(id_str) = <&str>::try_from(&id_val)
                    && !id_str.is_empty()
                {
                    active.push(id_str.to_string());
                }
            }
            env.active_networks = active;
        }
    } else if !cfg.use_session_bus {
        eprintln!(
            "kestic-daemon: NetworkManager service `{}` is not owned on system bus; assuming online unmetered",
            cfg.nm_dest
        );
    }

    env
}

/// Synchronously reads [`SystemEnvironment`] according to `cfg`.
pub fn read_system_environment(cfg: &MonitorBusConfig) -> SystemEnvironment {
    block_on(async move {
        let Ok(conn) = connect_target_bus(cfg).await else {
            return SystemEnvironment::desktop_connected();
        };
        read_system_environment_async(&conn, cfg).await
    })
}

/// Queries NetworkManager for known saved connection IDs (`§7.4`, `M3-S14`).
pub fn list_known_networks(cfg: &MonitorBusConfig) -> Vec<String> {
    block_on(async move {
        let Ok(conn) = connect_target_bus(cfg).await else {
            return Vec::new();
        };
        if !name_has_owner(&conn, &cfg.nm_dest).await {
            return Vec::new();
        }

        let mut names = BTreeSet::new();

        if let Ok(settings_proxy) = Proxy::new(
            &conn,
            cfg.nm_dest.as_str(),
            "/org/freedesktop/NetworkManager/Settings",
            "org.freedesktop.NetworkManager.Settings",
        )
        .await
            && let Ok(conns) = settings_proxy
                .call::<_, _, Vec<OwnedObjectPath>>("ListConnections", &())
                .await
        {
            for path in conns {
                if let Ok(c_proxy) = Proxy::new(
                    &conn,
                    cfg.nm_dest.as_str(),
                    path.as_str(),
                    "org.freedesktop.NetworkManager.Settings.Connection",
                )
                .await
                    && let Ok(settings) = c_proxy
                        .call::<_, _, HashMap<String, HashMap<String, OwnedVariant>>>(
                            "GetSettings",
                            &(),
                        )
                        .await
                    && let Some(conn_section) = settings.get("connection")
                    && conn_section
                        .get("type")
                        .and_then(|t| <&str>::try_from(t).ok())
                        .is_some_and(|t| t == NM_WIFI_CONNECTION_TYPE)
                    && let Some(id_val) = conn_section.get("id")
                    && let Ok(id_str) = <&str>::try_from(id_val)
                    && !id_str.trim().is_empty()
                {
                    names.insert(id_str.trim().to_string());
                }
            }
        }

        names.into_iter().collect()
    })
}

/// Active `systemd-logind` sleep `delay` inhibitor (`M3-S09`, `AC-M3-09-1..2`).
///
/// Retains **both** the `zbus::Connection` and `OwnedFd` for its entire lifetime so logind
/// keeps the inhibitor active until [`LogindSleepInhibitor::release`] or drop.
pub struct LogindSleepInhibitor {
    _conn: Connection,
    fd: Option<OwnedFd>,
}

impl LogindSleepInhibitor {
    /// Acquires `Inhibit("sleep", "Kestic", ..., "delay")` if logind is available on the target bus.
    pub fn acquire(cfg: &MonitorBusConfig) -> Option<Self> {
        let cfg = cfg.clone();
        block_on(async move {
            let conn = connect_target_bus(&cfg).await.ok()?;
            if !name_has_owner(&conn, &cfg.logind_dest).await {
                if !cfg.use_session_bus {
                    eprintln!(
                        "kestic-daemon: logind service `{}` not owned; sleep inhibitor unavailable",
                        cfg.logind_dest
                    );
                }
                return None;
            }
            let proxy = match Proxy::new(
                &conn,
                cfg.logind_dest.as_str(),
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
            )
            .await
            {
                Ok(p) => p,
                Err(err) => {
                    eprintln!("kestic-daemon: failed to create logind Manager proxy: {err}");
                    return None;
                }
            };
            let fd: OwnedFd = match proxy
                .call(
                    "Inhibit",
                    &(
                        "sleep",
                        "Kestic",
                        "Interrupt active backup cleanly before sleep",
                        "delay",
                    ),
                )
                .await
            {
                Ok(fd) => fd,
                Err(err) => {
                    eprintln!("kestic-daemon: logind Inhibit('sleep') failed: {err}");
                    return None;
                }
            };
            Some(Self {
                _conn: conn,
                fd: Some(fd),
            })
        })
    }

    /// Drops the open inhibitor FD immediately so `systemd-logind` can proceed to sleep.
    pub fn release(&mut self) {
        self.fd.take();
    }

    /// Returns whether the inhibitor FD is currently held open.
    pub fn is_held(&self) -> bool {
        self.fd.is_some()
    }
}

fn build_signal_rule<'a>(
    iface: &'a str,
    member: Option<&'a str>,
    path_ns: Option<&'a str>,
) -> Option<MatchRule<'a>> {
    let mut b = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(iface);
    if let Some(m) = member {
        b = b.member(m);
    }
    if let Some(p) = path_ns {
        b = b.path_namespace(p);
    }
    b.build().ok()
}

/// Longest time [`SystemMonitorHandle::stop`] waits for the listener thread to end.
pub(crate) const MONITOR_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Cooperative stop request that reaches a `block_on` waiting in any await point.
#[derive(Clone, Default)]
pub(crate) struct StopSignal {
    inner: Arc<StopInner>,
}

#[derive(Default)]
struct StopInner {
    stopped: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl StopSignal {
    /// Requests the stop and wakes the task currently waiting in [`StopSignal::until_stopped`].
    pub(crate) fn stop(&self) {
        self.inner.stopped.store(true, Ordering::SeqCst);
        let waker = self
            .inner
            .waker
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(w) = waker {
            w.wake();
        }
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.inner.stopped.load(Ordering::SeqCst)
    }

    /// Awaits `fut`, or returns `None` as soon as a stop is requested, whichever comes first.
    pub(crate) async fn until_stopped<F: Future>(&self, fut: F) -> Option<F::Output> {
        let mut fut = std::pin::pin!(fut);
        std::future::poll_fn(|cx| {
            if self.is_stopped() {
                return Poll::Ready(None);
            }
            *self.inner.waker.lock().unwrap_or_else(|p| p.into_inner()) = Some(cx.waker().clone());
            if self.is_stopped() {
                return Poll::Ready(None);
            }
            fut.as_mut().poll(cx).map(Some)
        })
        .await
    }
}

/// Signals the end of a thread when dropped, also when the thread unwinds.
struct DoneOnDrop(mpsc::Sender<()>);

impl Drop for DoneOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// Background D-Bus listener for UPower, NetworkManager, logind, and UDisks2 signals.
pub struct SystemMonitorHandle {
    stop: StopSignal,
    done_rx: mpsc::Receiver<()>,
    thread: Option<thread::JoinHandle<()>>,
}

impl SystemMonitorHandle {
    /// Starts the D-Bus signal monitor thread and delivers [`SystemMonitorEvent`]s on `event_tx`.
    /// Optional `wakeup_tx` is pinged whenever an event is pushed so the daemon loop wakes immediately.
    pub fn start(
        cfg: MonitorBusConfig,
        event_tx: mpsc::Sender<SystemMonitorEvent>,
        wakeup_tx: Option<mpsc::Sender<()>>,
    ) -> Self {
        let stop = StopSignal::default();
        let stop_bg = stop.clone();
        Self::spawn(stop, move || {
            block_on(run_monitor_loop(cfg, stop_bg, event_tx, wakeup_tx));
        })
    }

    /// Runs `body` on the listener thread; `stop` must be the signal `body` observes.
    pub(crate) fn spawn(stop: StopSignal, body: impl FnOnce() + Send + 'static) -> Self {
        let (done_tx, done_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _done = DoneOnDrop(done_tx);
            body();
        });
        Self {
            stop,
            done_rx,
            thread: Some(handle),
        }
    }

    /// Stops the monitor thread; never waits longer than [`MONITOR_STOP_TIMEOUT`], so a listener
    /// stuck in a D-Bus call cannot keep the daemon from shutting down.
    pub fn stop(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.stop.stop();
        let Some(handle) = self.thread.take() else {
            return;
        };
        match self.done_rx.recv_timeout(MONITOR_STOP_TIMEOUT) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = handle.join();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                eprintln!(
                    "kestic-daemon: system monitor thread did not stop within {} s; leaving it behind",
                    MONITOR_STOP_TIMEOUT.as_secs()
                );
            }
        }
    }
}

fn monitor_match_rules() -> Vec<MatchRule<'static>> {
    [
        build_signal_rule(
            "org.freedesktop.DBus.Properties",
            Some("PropertiesChanged"),
            Some("/org/freedesktop/UPower"),
        ),
        build_signal_rule(
            "org.freedesktop.DBus.Properties",
            Some("PropertiesChanged"),
            Some("/org/freedesktop/NetworkManager"),
        ),
        build_signal_rule("org.freedesktop.NetworkManager", Some("StateChanged"), None),
        build_signal_rule(
            "org.freedesktop.login1.Manager",
            Some("PrepareForSleep"),
            None,
        ),
        build_signal_rule(
            "org.freedesktop.DBus.ObjectManager",
            None,
            Some("/org/freedesktop/UDisks2"),
        ),
        build_signal_rule(
            "org.freedesktop.DBus.Properties",
            Some("PropertiesChanged"),
            Some("/org/freedesktop/UDisks2"),
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// What a received signal means for the daemon.
enum SignalKind {
    /// Forwarded at once (`PrepareForSleep`, mount changes).
    Event(SystemMonitorEvent),
    /// Power or network changed: re-read the environment once the burst is drained.
    EnvironmentChanged,
    Other,
}

fn classify_signal(msg: &zbus::Message) -> SignalKind {
    let hdr = msg.header();
    let iface = hdr.interface().map(|i| i.as_str()).unwrap_or("");
    let member = hdr.member().map(|m| m.as_str()).unwrap_or("");
    let path = hdr.path().map(|p| p.as_str()).unwrap_or("");

    if iface == "org.freedesktop.login1.Manager" && member == "PrepareForSleep" {
        return msg
            .body()
            .deserialize::<bool>()
            .map_or(SignalKind::Other, |start| {
                SignalKind::Event(SystemMonitorEvent::PrepareForSleep(start))
            });
    }
    if path.starts_with("/org/freedesktop/UDisks2")
        && (iface == "org.freedesktop.DBus.ObjectManager"
            || iface == "org.freedesktop.DBus.Properties")
    {
        return SignalKind::Event(SystemMonitorEvent::MountChanged);
    }
    if path.starts_with("/org/freedesktop/UPower")
        || path.starts_with("/org/freedesktop/NetworkManager")
        || iface == "org.freedesktop.NetworkManager"
    {
        return SignalKind::EnvironmentChanged;
    }
    SignalKind::Other
}

/// Waits for the next message of `stream` (`None` once it ended).
async fn next_message(stream: &mut MessageStream) -> Option<zbus::Result<zbus::Message>> {
    std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await
}

/// The next message of `stream` if one is ready right now, without waiting.
async fn ready_message(stream: &mut MessageStream) -> Option<zbus::Result<zbus::Message>> {
    std::future::poll_fn(|cx| match Pin::new(&mut *stream).poll_next(cx) {
        Poll::Ready(item) => Poll::Ready(item),
        Poll::Pending => Poll::Ready(None),
    })
    .await
}

/// Subscribes `conn` to the monitored signals; `false` once a stop is requested.
async fn add_match_rules(conn: &Connection, stop: &StopSignal) -> bool {
    match stop.until_stopped(zbus::fdo::DBusProxy::new(conn)).await {
        None => return false,
        Some(Ok(dbus_proxy)) => {
            for rule in monitor_match_rules() {
                match stop.until_stopped(dbus_proxy.add_match_rule(rule)).await {
                    None => return false,
                    Some(Err(err)) => eprintln!(
                        "kestic-daemon: failed to register system monitor match rule: {err}"
                    ),
                    Some(Ok(())) => {}
                }
            }
        }
        Some(Err(err)) => eprintln!(
            "kestic-daemon: failed to create DBusProxy for system monitor match rules: {err}"
        ),
    }
    true
}

/// Listener loop of the monitor thread; every await point ends as soon as `stop` is requested.
///
/// Signals arrive on one connection, environment reads use a second one: a reply never waits
/// behind a full signal queue (zbus stops reading a connection whose queue is full, which
/// deadlocked the listener during the burst of signals when the lid closes). Each burst is
/// drained first, forwarding sleep and mount events at once, and the environment is then read
/// once instead of once per signal.
async fn run_monitor_loop(
    cfg: MonitorBusConfig,
    stop: StopSignal,
    event_tx: mpsc::Sender<SystemMonitorEvent>,
    wakeup_tx: Option<mpsc::Sender<()>>,
) {
    let Some(Ok(conn)) = stop.until_stopped(connect_target_bus(&cfg)).await else {
        return;
    };
    let Some(Ok(env_conn)) = stop.until_stopped(connect_target_bus(&cfg)).await else {
        return;
    };
    if !add_match_rules(&conn, &stop).await {
        return;
    }
    let send = |ev: SystemMonitorEvent| {
        let _ = event_tx.send(ev);
        if let Some(ref wtx) = wakeup_tx {
            let _ = wtx.send(());
        }
    };

    let mut stream = MessageStream::from(&conn);
    while let Some(Some(first)) = stop.until_stopped(next_message(&mut stream)).await {
        let mut env_changed = false;
        let mut item = Some(first);
        while let Some(msg_res) = item {
            if let Ok(msg) = msg_res {
                match classify_signal(&msg) {
                    SignalKind::Event(ev) => send(ev),
                    SignalKind::EnvironmentChanged => env_changed = true,
                    SignalKind::Other => {}
                }
            }
            item = match stop.until_stopped(ready_message(&mut stream)).await {
                Some(next) => next,
                None => return,
            };
        }
        if env_changed {
            let Some(env) = stop
                .until_stopped(read_system_environment_async(&env_conn, &cfg))
                .await
            else {
                return;
            };
            send(SystemMonitorEvent::EnvironmentChanged(env));
        }
    }
}

impl Drop for SystemMonitorHandle {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// M3 manual acceptance (2026-11-19): a stop request must end a listener waiting in an
    /// await that never completes (e.g. a D-Bus reply that never arrives).
    #[test]
    fn m3_acceptance_stop_signal_interrupts_await_that_never_completes() {
        let stop = StopSignal::default();
        let stop_bg = stop.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let res = block_on(stop_bg.until_stopped(std::future::pending::<()>()));
            let _ = tx.send(res);
        });
        thread::sleep(Duration::from_millis(100));

        stop.stop();

        let res = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("until_stopped must return after stop()");
        assert_eq!(res, None);
    }

    /// M3 manual acceptance (2026-11-19): `stop()` may not wait forever for a listener thread
    /// that does not react, or the daemon hangs in shutdown with tray and D-Bus still alive.
    #[test]
    fn m3_acceptance_stop_returns_within_timeout_when_thread_ignores_stop() {
        let release = Arc::new(AtomicBool::new(false));
        let release_bg = Arc::clone(&release);
        let handle = SystemMonitorHandle::spawn(StopSignal::default(), move || {
            while !release_bg.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(20));
            }
        });

        let started = Instant::now();
        handle.stop();
        let waited = started.elapsed();
        release.store(true, Ordering::SeqCst);

        assert!(
            waited < MONITOR_STOP_TIMEOUT + Duration::from_secs(1),
            "stop() waited {waited:?}"
        );
    }

    /// The bounded wait does not cut short a thread that ends on its own after the stop.
    #[test]
    fn m3_acceptance_stop_joins_thread_that_reacts_to_stop() {
        let stop = StopSignal::default();
        let stop_bg = stop.clone();
        let ended = Arc::new(AtomicBool::new(false));
        let ended_bg = Arc::clone(&ended);
        let handle = SystemMonitorHandle::spawn(stop, move || {
            block_on(stop_bg.until_stopped(std::future::pending::<()>()));
            ended_bg.store(true, Ordering::SeqCst);
        });

        handle.stop();

        assert!(ended.load(Ordering::SeqCst));
    }
}
