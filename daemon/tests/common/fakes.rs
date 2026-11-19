// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reusable D-Bus system fakes on isolated session buses (`AC-M1-Q-1`, `M3-S03`, `M3-S04`,
//! `M3-S09`, `M3-S12`, `M3-S13`):
//! - `org.freedesktop.UPower` + `DisplayDevice`
//! - `org.freedesktop.NetworkManager` + `Active` + `Settings`
//! - `org.freedesktop.login1` (`Inhibit` delay pipe FD + `PrepareForSleep`)
//! - `org.freedesktop.UDisks2` (`ObjectManager` `InterfacesAdded` / `InterfacesRemoved`)
//! - `org.kde.JobViewServerV2` / `org.kde.JobViewV3` + `org.freedesktop.portal.Inhibit`

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
};
use zbus::object_server::SignalEmitter;
use zbus::{Connection, Fd, OwnedObjectPath, OwnedVariant, Variant, block_on, interface};

// --- UPower Fake ------------------------------------------------------------------------------

#[derive(Default)]
pub struct FakeUPowerState {
    pub on_battery: bool,
    pub percentage: f64,
}

pub struct FakeUPower {
    state: Arc<Mutex<FakeUPowerState>>,
}

#[interface(name = "org.freedesktop.UPower")]
impl FakeUPower {
    #[zbus(property, name = "OnBattery")]
    fn on_battery(&self) -> bool {
        self.state.lock().expect("lock upower").on_battery
    }
}

pub struct FakeUPowerDevice {
    state: Arc<Mutex<FakeUPowerState>>,
}

#[interface(name = "org.freedesktop.UPower.Device")]
impl FakeUPowerDevice {
    #[zbus(property, name = "Percentage")]
    fn percentage(&self) -> f64 {
        self.state.lock().expect("lock upower").percentage
    }
}

pub struct FakeUPowerHandle {
    conn: Connection,
    state: Arc<Mutex<FakeUPowerState>>,
}

impl FakeUPowerHandle {
    pub fn start(on_battery: bool, percentage: f64) -> Self {
        let state = Arc::new(Mutex::new(FakeUPowerState {
            on_battery,
            percentage,
        }));
        let root = FakeUPower {
            state: Arc::clone(&state),
        };
        let dev = FakeUPowerDevice {
            state: Arc::clone(&state),
        };
        let conn = block_on(async move {
            zbus::connection::Builder::session()
                .name("org.freedesktop.UPower")
                .serve_at("/org/freedesktop/UPower", root)
                .serve_at("/org/freedesktop/UPower/devices/DisplayDevice", dev)
                .build()
                .await
                .expect("build fake UPower")
        });
        Self { conn, state }
    }

    pub fn set_on_battery(&self, on_battery: bool, percentage: f64) {
        {
            let mut s = self.state.lock().expect("lock upower");
            s.on_battery = on_battery;
            s.percentage = percentage;
        }
        let conn = self.conn.clone();
        block_on(async move {
            let iface_ref = conn
                .object_server()
                .interface::<_, FakeUPower>("/org/freedesktop/UPower")
                .await
                .expect("upower iface");
            let mut changed = HashMap::new();
            let val = Variant::from(on_battery);
            changed.insert("OnBattery", val);
            let _ = iface_ref
                .signal_emitter()
                .emit(
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &("org.freedesktop.UPower", changed, Vec::<&str>::new()),
                )
                .await;
        });
    }
}

// --- NetworkManager Fake ----------------------------------------------------------------------

#[derive(Default)]
pub struct FakeNmState {
    pub connectivity: u32,
    pub state: u32,
    pub metered: u32,
    pub active_connections: Vec<OwnedObjectPath>,
    pub known_ids: Vec<String>,
}

pub struct FakeNetworkManager {
    state: Arc<Mutex<FakeNmState>>,
}

#[interface(name = "org.freedesktop.NetworkManager")]
impl FakeNetworkManager {
    #[zbus(property, name = "Connectivity")]
    fn connectivity(&self) -> u32 {
        self.state.lock().expect("lock nm").connectivity
    }

    #[zbus(property, name = "State")]
    fn state(&self) -> u32 {
        self.state.lock().expect("lock nm").state
    }

    #[zbus(property, name = "Metered")]
    fn metered(&self) -> u32 {
        self.state.lock().expect("lock nm").metered
    }

    #[zbus(property, name = "ActiveConnections")]
    fn active_connections(&self) -> Vec<OwnedObjectPath> {
        self.state
            .lock()
            .expect("lock nm")
            .active_connections
            .clone()
    }

    #[zbus(signal, name = "StateChanged")]
    async fn nm_state_changed(emitter: &SignalEmitter<'_>, state: u32) -> zbus::Result<()>;
}

pub struct FakeNmActiveConn {
    id: Arc<Mutex<String>>,
}

#[interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl FakeNmActiveConn {
    #[zbus(property, name = "Id")]
    fn id(&self) -> String {
        self.id.lock().expect("lock nm active id").clone()
    }
}

pub struct FakeNmSettings {
    paths: Vec<OwnedObjectPath>,
}

#[interface(name = "org.freedesktop.NetworkManager.Settings")]
impl FakeNmSettings {
    #[zbus(name = "ListConnections")]
    fn list_connections(&self) -> Vec<OwnedObjectPath> {
        self.paths.clone()
    }
}

pub struct FakeNmSavedConn {
    id: String,
    conn_type: &'static str,
}

#[interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl FakeNmSavedConn {
    #[zbus(name = "GetSettings")]
    fn get_settings(&self) -> HashMap<String, HashMap<String, OwnedVariant>> {
        let mut conn_map = HashMap::new();
        conn_map.insert(
            "id".to_string(),
            Variant::from(self.id.as_str())
                .try_to_owned()
                .expect("owned"),
        );
        conn_map.insert(
            "type".to_string(),
            Variant::from(self.conn_type).try_to_owned().expect("owned"),
        );
        let mut root = HashMap::new();
        root.insert("connection".to_string(), conn_map);
        root
    }
}

/// Saved wired profile the fake NetworkManager always lists (never an allowed network).
pub const FAKE_NM_WIRED_ID: &str = "Kabelgebundene Verbindung 1";
/// Loopback profile the fake NetworkManager always lists (never an allowed network).
pub const FAKE_NM_LOOPBACK_ID: &str = "lo";

pub struct FakeNetworkManagerHandle {
    conn: Connection,
    state: Arc<Mutex<FakeNmState>>,
    active_id: Arc<Mutex<String>>,
    active_path: OwnedObjectPath,
}

impl FakeNetworkManagerHandle {
    pub fn start(
        connected: bool,
        metered: bool,
        active_network: Option<&str>,
        known_networks: &[&str],
    ) -> Self {
        let active_path =
            OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/ActiveConnection/1")
                .expect("active path");
        let active_id_str = active_network.unwrap_or("").to_string();
        let active_connections = if active_network.is_some() {
            vec![active_path.clone()]
        } else {
            Vec::new()
        };

        let state = Arc::new(Mutex::new(FakeNmState {
            connectivity: if connected { 4 } else { 1 },
            state: if connected { 70 } else { 20 },
            metered: if metered { 1 } else { 2 },
            active_connections,
            known_ids: known_networks.iter().map(|s| s.to_string()).collect(),
        }));
        let active_id = Arc::new(Mutex::new(active_id_str));

        let nm_iface = FakeNetworkManager {
            state: Arc::clone(&state),
        };
        let active_iface = FakeNmActiveConn {
            id: Arc::clone(&active_id),
        };

        // Known Wi-Fi networks plus the non-Wi-Fi profiles NetworkManager always lists
        // (wired, loopback): only the Wi-Fi ones may be offered as allowed networks.
        let saved: Vec<(String, &'static str)> = known_networks
            .iter()
            .map(|s| (s.to_string(), "802-11-wireless"))
            .chain([
                (FAKE_NM_WIRED_ID.to_string(), "802-3-ethernet"),
                (FAKE_NM_LOOPBACK_ID.to_string(), "loopback"),
            ])
            .collect();
        let mut saved_paths = Vec::new();
        for (idx, _) in saved.iter().enumerate() {
            let p = OwnedObjectPath::try_from(format!(
                "/org/freedesktop/NetworkManager/Settings/{idx}"
            ))
            .expect("saved path");
            saved_paths.push(p);
        }
        let settings_iface = FakeNmSettings {
            paths: saved_paths.clone(),
        };
        let known_owned: Vec<(OwnedObjectPath, (String, &'static str))> =
            saved_paths.into_iter().zip(saved).collect();
        let ap_clone = active_path.clone();

        let conn = block_on(async move {
            let mut builder = zbus::connection::Builder::session()
                .name("org.freedesktop.NetworkManager")
                .serve_at("/org/freedesktop/NetworkManager", nm_iface)
                .serve_at(ap_clone, active_iface)
                .serve_at("/org/freedesktop/NetworkManager/Settings", settings_iface);
            for (path, (id, conn_type)) in known_owned {
                builder = builder.serve_at(path, FakeNmSavedConn { id, conn_type });
            }
            builder.build().await.expect("build fake NetworkManager")
        });

        Self {
            conn,
            state,
            active_id,
            active_path,
        }
    }

    pub fn update(&self, connected: bool, metered: bool, active_network: Option<&str>) {
        let new_state = if connected { 70 } else { 20 };
        let new_conn = if connected { 4 } else { 1 };
        let new_metered = if metered { 1 } else { 2 };
        {
            let mut s = self.state.lock().expect("lock nm");
            s.connectivity = new_conn;
            s.state = new_state;
            s.metered = new_metered;
            s.active_connections = if active_network.is_some() {
                vec![self.active_path.clone()]
            } else {
                Vec::new()
            };
        }
        *self.active_id.lock().expect("lock nm active id") =
            active_network.unwrap_or("").to_string();

        let conn = self.conn.clone();
        block_on(async move {
            let iface_ref = conn
                .object_server()
                .interface::<_, FakeNetworkManager>("/org/freedesktop/NetworkManager")
                .await
                .expect("nm iface");
            let _ =
                FakeNetworkManager::nm_state_changed(iface_ref.signal_emitter(), new_state).await;
        });
    }
}

// --- systemd-logind Fake ----------------------------------------------------------------------

struct LogindRecorder {
    inhibit_calls: Mutex<Vec<(String, String, String, String)>>,
    peer_sockets: Mutex<Vec<UnixStream>>,
}

struct FakeLogindManager {
    recorder: Arc<LogindRecorder>,
}

#[interface(name = "org.freedesktop.login1.Manager")]
impl FakeLogindManager {
    #[zbus(name = "Inhibit")]
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::fdo::Result<Fd<'static>> {
        self.recorder.inhibit_calls.lock().expect("lock").push((
            what.to_string(),
            who.to_string(),
            why.to_string(),
            mode.to_string(),
        ));
        let (peer_end, client_end) =
            UnixStream::pair().map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let _ = peer_end.set_nonblocking(true);
        self.recorder
            .peer_sockets
            .lock()
            .expect("lock")
            .push(peer_end);
        let owned: std::os::unix::io::OwnedFd = client_end.into();
        Ok(Fd::from(owned))
    }

    #[zbus(signal, name = "PrepareForSleep")]
    async fn prepare_for_sleep(emitter: &SignalEmitter<'_>, start: bool) -> zbus::Result<()>;
}

pub struct FakeLogindHandle {
    conn: Connection,
    recorder: Arc<LogindRecorder>,
}

impl FakeLogindHandle {
    pub fn start() -> Self {
        let recorder = Arc::new(LogindRecorder {
            inhibit_calls: Mutex::new(Vec::new()),
            peer_sockets: Mutex::new(Vec::new()),
        });
        let iface = FakeLogindManager {
            recorder: Arc::clone(&recorder),
        };
        let conn = block_on(async move {
            zbus::connection::Builder::session()
                .name("org.freedesktop.login1")
                .serve_at("/org/freedesktop/login1", iface)
                .build()
                .await
                .expect("build fake logind")
        });
        Self { conn, recorder }
    }

    pub fn inhibit_call_count(&self) -> usize {
        self.recorder.inhibit_calls.lock().expect("lock").len()
    }

    /// Returns every recorded `Inhibit(what, who, why, mode)` call in arrival order.
    pub fn inhibit_calls(&self) -> Vec<(String, String, String, String)> {
        self.recorder.inhibit_calls.lock().expect("lock").clone()
    }

    /// Returns how many previously acquired inhibitor FDs are still held open by clients.
    pub fn open_inhibitor_count(&self) -> usize {
        let mut peers = self.recorder.peer_sockets.lock().expect("lock");
        let mut open = 0usize;
        let mut buf = [0u8; 8];
        peers.retain_mut(|sock| match sock.read(&mut buf) {
            Ok(0) => false, // client closed FD -> EOF
            Ok(_) => {
                open += 1;
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                open += 1;
                true
            }
            Err(_) => false,
        });
        open
    }

    pub fn emit_prepare_for_sleep(&self, start: bool) {
        let conn = self.conn.clone();
        block_on(async move {
            let iface_ref = conn
                .object_server()
                .interface::<_, FakeLogindManager>("/org/freedesktop/login1")
                .await
                .expect("logind iface");
            let _ = FakeLogindManager::prepare_for_sleep(iface_ref.signal_emitter(), start).await;
        });
    }
}

// --- UDisks2 Fake -----------------------------------------------------------------------------

struct FakeUDisks2Manager;

#[interface(name = "org.freedesktop.DBus.ObjectManager")]
impl FakeUDisks2Manager {
    #[zbus(signal, name = "InterfacesAdded")]
    async fn interfaces_added(
        emitter: &SignalEmitter<'_>,
        object_path: OwnedObjectPath,
        interfaces_and_properties: HashMap<String, HashMap<String, OwnedVariant>>,
    ) -> zbus::Result<()>;
}

pub struct FakeUDisks2Handle {
    conn: Connection,
}

impl FakeUDisks2Handle {
    pub fn start() -> Self {
        let conn = block_on(async move {
            zbus::connection::Builder::session()
                .name("org.freedesktop.UDisks2")
                .serve_at("/org/freedesktop/UDisks2", FakeUDisks2Manager)
                .build()
                .await
                .expect("build fake UDisks2")
        });
        Self { conn }
    }

    pub fn emit_mount_added(&self, device_name: &str, uuid: &str, mount_point: &str) {
        let conn = self.conn.clone();
        let path = OwnedObjectPath::try_from(format!(
            "/org/freedesktop/UDisks2/block_devices/{device_name}"
        ))
        .expect("block device path");
        let uuid_owned = uuid.to_string();
        let mp_bytes: Vec<u8> = mount_point
            .as_bytes()
            .iter()
            .copied()
            .chain(std::iter::once(0))
            .collect();
        block_on(async move {
            let iface_ref = conn
                .object_server()
                .interface::<_, FakeUDisks2Manager>("/org/freedesktop/UDisks2")
                .await
                .expect("udisks2 iface");
            let mut block_props = HashMap::new();
            block_props.insert(
                "IdUUID".to_string(),
                Variant::from(uuid_owned.as_str())
                    .try_to_owned()
                    .expect("owned"),
            );
            let mut fs_props = HashMap::new();
            let mp_array = vec![mp_bytes];
            fs_props.insert(
                "MountPoints".to_string(),
                Variant::from(mp_array).try_to_owned().expect("owned"),
            );
            let mut ifaces = HashMap::new();
            ifaces.insert("org.freedesktop.UDisks2.Block".to_string(), block_props);
            ifaces.insert("org.freedesktop.UDisks2.Filesystem".to_string(), fs_props);
            let _ = FakeUDisks2Manager::interfaces_added(iface_ref.signal_emitter(), path, ifaces)
                .await;
        });
    }
}

// --- JobViewServerV2 & JobViewV3 Fake ---------------------------------------------------------

#[derive(Default)]
pub struct FakeJobViewRecorder {
    pub request_count: AtomicU32,
    pub desktop_entry: Mutex<String>,
    pub capabilities: AtomicI32,
    pub updates: Mutex<Vec<(u32, String)>>,
    pub terminated: AtomicBool,
    pub terminate_code: AtomicU32,
    /// Every `terminate(error_code, error_message, ..)` call in arrival order.
    pub terminations: Mutex<Vec<(u32, String)>>,
}

struct FakeJobViewServerV2 {
    recorder: Arc<FakeJobViewRecorder>,
    view_path: OwnedObjectPath,
}

#[interface(name = "org.kde.JobViewServerV2")]
impl FakeJobViewServerV2 {
    #[zbus(name = "requestView")]
    fn request_view(
        &self,
        desktop_entry: &str,
        capabilities: i32,
        _hints: HashMap<String, OwnedVariant>,
    ) -> OwnedObjectPath {
        self.recorder.request_count.fetch_add(1, Ordering::SeqCst);
        *self.recorder.desktop_entry.lock().expect("lock") = desktop_entry.to_string();
        self.recorder
            .capabilities
            .store(capabilities, Ordering::SeqCst);
        self.view_path.clone()
    }
}

struct FakeJobViewV3 {
    recorder: Arc<FakeJobViewRecorder>,
}

#[interface(name = "org.kde.JobViewV3")]
impl FakeJobViewV3 {
    #[zbus(name = "update")]
    fn update(&self, properties: HashMap<String, OwnedVariant>) {
        let pct = properties
            .get("percent")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let msg = properties
            .get("infoMessage")
            .and_then(|v| <&str>::try_from(v).ok())
            .unwrap_or("")
            .to_string();
        self.recorder.updates.lock().expect("lock").push((pct, msg));
    }

    #[zbus(name = "terminate")]
    fn terminate(
        &self,
        error_code: u32,
        error_message: &str,
        _hints: HashMap<String, OwnedVariant>,
    ) {
        self.recorder
            .terminations
            .lock()
            .expect("lock")
            .push((error_code, error_message.to_string()));
        self.recorder
            .terminate_code
            .store(error_code, Ordering::SeqCst);
        self.recorder.terminated.store(true, Ordering::SeqCst);
    }

    #[zbus(signal, name = "cancelRequested")]
    async fn cancel_requested(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

pub struct FakeJobViewHandle {
    conn: Connection,
    view_path: OwnedObjectPath,
    pub recorder: Arc<FakeJobViewRecorder>,
}

impl FakeJobViewHandle {
    pub fn start(bus_name: &str) -> Self {
        let recorder = Arc::new(FakeJobViewRecorder::default());
        let view_path = OwnedObjectPath::try_from("/JobViewServer/JobView_1").expect("view path");
        let server_iface = FakeJobViewServerV2 {
            recorder: Arc::clone(&recorder),
            view_path: view_path.clone(),
        };
        let view_iface = FakeJobViewV3 {
            recorder: Arc::clone(&recorder),
        };
        let bus_owned = bus_name.to_string();
        let vp_clone = view_path.clone();
        let conn = block_on(async move {
            zbus::connection::Builder::session()
                .name(bus_owned)
                .serve_at("/JobViewServer", server_iface)
                .serve_at(vp_clone, view_iface)
                .build()
                .await
                .expect("build fake JobViewServer")
        });
        Self {
            conn,
            view_path,
            recorder,
        }
    }

    pub fn emit_cancel_requested(&self) {
        let conn = self.conn.clone();
        let vp = self.view_path.clone();
        block_on(async move {
            let iface_ref = conn
                .object_server()
                .interface::<_, FakeJobViewV3>(vp.as_str())
                .await
                .expect("job view iface");
            let _ = FakeJobViewV3::cancel_requested(iface_ref.signal_emitter()).await;
        });
    }
}

// --- Portal Inhibit Fake ----------------------------------------------------------------------

#[derive(Default)]
pub struct FakePortalRecorder {
    pub inhibit_count: AtomicU32,
    pub last_flags: AtomicU32,
    pub closed_count: AtomicU32,
}

struct FakePortalInhibit {
    recorder: Arc<FakePortalRecorder>,
    req_path: OwnedObjectPath,
}

#[interface(name = "org.freedesktop.portal.Inhibit")]
impl FakePortalInhibit {
    fn inhibit(
        &self,
        _window: &str,
        flags: u32,
        _options: HashMap<String, OwnedVariant>,
    ) -> OwnedObjectPath {
        self.recorder.inhibit_count.fetch_add(1, Ordering::SeqCst);
        self.recorder.last_flags.store(flags, Ordering::SeqCst);
        self.req_path.clone()
    }
}

struct FakePortalRequest {
    recorder: Arc<FakePortalRecorder>,
}

#[interface(name = "org.freedesktop.portal.Request")]
impl FakePortalRequest {
    fn close(&self) {
        self.recorder.closed_count.fetch_add(1, Ordering::SeqCst);
    }
}

pub struct FakePortalInhibitHandle {
    _conn: Connection,
    pub recorder: Arc<FakePortalRecorder>,
}

impl FakePortalInhibitHandle {
    pub fn start() -> Self {
        let recorder = Arc::new(FakePortalRecorder::default());
        let req_path =
            OwnedObjectPath::try_from("/org/freedesktop/portal/desktop/request/kestic/1")
                .expect("req path");
        let inhibit = FakePortalInhibit {
            recorder: Arc::clone(&recorder),
            req_path: req_path.clone(),
        };
        let request = FakePortalRequest {
            recorder: Arc::clone(&recorder),
        };
        let conn = block_on(async move {
            zbus::connection::Builder::session()
                .name("org.freedesktop.portal.Desktop")
                .serve_at("/org/freedesktop/portal/desktop", inhibit)
                .serve_at(req_path, request)
                .build()
                .await
                .expect("build fake portal inhibit")
        });
        Self {
            _conn: conn,
            recorder,
        }
    }
}
