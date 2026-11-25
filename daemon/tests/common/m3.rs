// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared helpers for the M3 automation acceptance tests (`M3-S03`–`M3-S14`): daemon spawning with
//! injectable clock, restic wrapper with call log, state polling, and notification assertions.

use super::fakes::{FakeNetworkManagerHandle, FakeUPowerHandle};
use super::{CapturedNotification, NotificationCapture, find_kestic_binary, wait_for_file};
use kestic_core::clock::{Clock, FakeClock, SystemClock};
use kestic_core::destination::{DestinationSetupMode, setup_destination};
use kestic_core::history::{BackupRuntimeState, RunHistory};
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, DestinationKind, ResticProvider, RunStatus,
    Schedule, Source,
};
use kestic_core::mount::MountInspector;
use kestic_core::provider::check_binary_version;
use kestic_core::query_list::QueryLimits;
use kestic_core::restic::ResticRuntime;
use kestic_core::scheduler::TimezoneSpec;
use kestic_core::secrets::{SecretError, SecretStore, SecretValue};
use kestic_daemon::dbus_api::{
    BackupSummaryDto, DaemonStateDto, DestinationSummaryDto, KESTIC_OBJECT_PATH, KesticClient,
};
use kestic_daemon::monitors::MonitorBusConfig;
use kestic_daemon::secrets::SecretServiceStore;
use kestic_daemon::service::{DaemonOutcome, DaemonRunConfig, run_daemon};
use kestic_test_support::repo_template::copy_template_repo;
use kestic_test_support::tools::{RestServerGuard, find_lock_file};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use time::macros::datetime;

/// One restic binary under test (`AC-M1-Q-2`: 0.18.0 and latest).
#[derive(Debug, Clone)]
pub struct ResticVariant {
    pub index: usize,
    pub label: &'static str,
    pub bin: PathBuf,
}

impl ResticVariant {
    /// Unique session-bus name per variant so sequential daemons never collide.
    pub fn bus_name(&self, base: &str) -> String {
        format!("net.daester.Kestic.{base}V{}", self.index)
    }

    /// Unique sub-directory below the isolated test root per variant.
    pub fn sub(&self, base: &str) -> String {
        format!("{base}-v{}", self.index)
    }
}

/// Both restic versions (`restic 0.18.0` and latest) for tests that execute restic.
pub fn restic_variants() -> Vec<ResticVariant> {
    super::restic_test_binaries()
        .into_iter()
        .enumerate()
        .map(|(index, (label, bin))| ResticVariant { index, label, bin })
        .collect()
}

/// Latest restic only, for tests whose behaviour does not depend on the restic version.
pub fn latest_variant() -> ResticVariant {
    restic_variants()
        .pop()
        .expect("latest restic variant must exist")
}

/// Returns `(config.toml path, state dir)` of the daemon sandbox `sub` below `root`.
pub fn daemon_paths(root: &Path, sub: &str) -> (PathBuf, PathBuf) {
    let base = root.join(sub);
    (
        base.join("cfg").join("kestic").join("config.toml"),
        base.join("state").join("kestic"),
    )
}

/// Options for [`spawn_m3_daemon`].
pub struct M3DaemonOpts<'a> {
    pub sub_name: &'a str,
    pub bus_name: &'a str,
    pub restic_bin: &'a Path,
    pub clock: Arc<dyn Clock>,
    /// Secret store of the daemon; defaults to the isolated test keyring.
    pub secret_store: Option<Arc<dyn SecretStore + Send + Sync>>,
    pub mount_inspector: Option<Arc<dyn MountInspector>>,
    pub timezone: TimezoneSpec,
    pub enable_monitors: bool,
    pub enable_scheduler: bool,
    pub enable_job_view: bool,
    pub enable_portal_inhibit: bool,
    /// Skip the StatusNotifierWatcher check at startup.
    pub assume_sni_available: bool,
    /// Memory limits for the lists of snapshot queries.
    pub query_limits: QueryLimits,
}

impl<'a> M3DaemonOpts<'a> {
    /// Scheduler and system monitors on, JobView and portal off, Europe/Zurich time zone.
    pub fn new(
        sub_name: &'a str,
        bus_name: &'a str,
        restic_bin: &'a Path,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            sub_name,
            bus_name,
            restic_bin,
            clock,
            secret_store: None,
            mount_inspector: None,
            timezone: TimezoneSpec::Named("Europe/Zurich".to_string()),
            enable_monitors: true,
            enable_scheduler: true,
            enable_job_view: false,
            enable_portal_inhibit: false,
            assume_sni_available: true,
            query_limits: QueryLimits::default(),
        }
    }

    /// Real clock, no scheduler and no system monitors: runs start on request only.
    pub fn on_request(sub_name: &'a str, bus_name: &'a str, restic_bin: &'a Path) -> Self {
        Self {
            enable_monitors: false,
            enable_scheduler: false,
            timezone: TimezoneSpec::System,
            ..Self::new(sub_name, bus_name, restic_bin, Arc::new(SystemClock))
        }
    }
}

/// Running in-process daemon thread plus its D-Bus client.
pub struct M3Daemon {
    pub client: KesticClient,
    pub state_dir: PathBuf,
    pub config_path: PathBuf,
    bus_name: String,
    handle: Option<thread::JoinHandle<Result<DaemonOutcome, String>>>,
}

impl M3Daemon {
    /// Confirms quit over D-Bus and asserts a clean daemon shutdown with the bus name released.
    pub fn stop(mut self) {
        self.client.confirm_quit().expect("ConfirmQuit");
        let handle = self.handle.take().expect("daemon handle");
        assert_eq!(
            handle.join().expect("join daemon thread"),
            Ok(DaemonOutcome::RanAndStopped)
        );
        super::wait_for_bus_name_released(
            self.client.connection(),
            &self.bus_name,
            Duration::from_secs(5),
        );
    }
}

/// Starts the daemon in-process (existing `config.toml` and history in the sandbox are honoured).
pub fn spawn_m3_daemon(root: &Path, opts: M3DaemonOpts<'_>) -> M3Daemon {
    let (config_path, state_dir) = daemon_paths(root, opts.sub_name);
    let ready_file = root.join(opts.sub_name).join("daemon.ready");
    fs::create_dir_all(root.join(opts.sub_name)).expect("mkdir daemon sandbox");
    let _ = fs::remove_file(&ready_file);

    let run_cfg = DaemonRunConfig {
        bus_name: opts.bus_name.to_string(),
        assume_sni_available: opts.assume_sni_available,
        ready_file: Some(ready_file.clone()),
        config_path: Some(config_path.clone()),
        state_dir: Some(state_dir.clone()),
        kestic_binary: Some(find_kestic_binary()),
        clock: opts.clock,
        secret_store: Some(
            opts.secret_store
                .unwrap_or_else(|| Arc::new(super::isolated_secret_store())),
        ),
        mount_inspector: opts.mount_inspector,
        suppress_ui_spawn: true,
        enable_job_view: opts.enable_job_view,
        enable_portal_inhibit: opts.enable_portal_inhibit,
        timezone: opts.timezone,
        monitor_bus: MonitorBusConfig {
            use_session_bus: true,
            enable_watcher: opts.enable_monitors,
            ..Default::default()
        },
        enable_system_monitors: opts.enable_monitors,
        enable_scheduler: opts.enable_scheduler,
        query_limits: opts.query_limits,
        ..Default::default()
    };

    let handle = thread::spawn(move || run_daemon(run_cfg));
    wait_for_file(&ready_file, super::DAEMON_READY_TIMEOUT);

    let client =
        KesticClient::connect(opts.bus_name, KESTIC_OBJECT_PATH).expect("connect KesticClient");
    let prov = client
        .set_restic_provider(&ResticProvider::Custom {
            path: opts.restic_bin.to_path_buf(),
        })
        .expect("set restic provider");
    assert!(prov.valid, "restic provider must be valid: {prov:?}");
    M3Daemon {
        client,
        state_dir,
        config_path,
        bus_name: opts.bus_name.to_string(),
        handle: Some(handle),
    }
}

/// A daemon running behind a [`ResticWrapper`] around the variant's restic, with a fake clock.
pub struct M3Rig {
    pub daemon: M3Daemon,
    pub clock: Arc<FakeClock>,
    pub wrapper: ResticWrapper,
    pub variant: ResticVariant,
    pub root: PathBuf,
    /// Sandbox sub-directory name of this variant/base below `root`.
    pub sub: String,
}

impl M3Rig {
    /// Starts the daemon (default options with the scheduler on) at fake time `start`;
    /// `configure` adjusts the options before the spawn.
    pub fn start(
        root: &Path,
        variant: &ResticVariant,
        base: &str,
        start: time::OffsetDateTime,
        configure: impl FnOnce(&mut M3DaemonOpts<'_>),
    ) -> Self {
        Self::start_on(
            root,
            variant,
            &variant.sub(base),
            &variant.bus_name(base),
            start,
            configure,
        )
    }

    /// Like [`Self::start`], but with an explicit sandbox sub-directory `sub` below `root` and
    /// session-bus name `bus` (for example the real bus name a GUI process connects to).
    pub fn start_on(
        root: &Path,
        variant: &ResticVariant,
        sub: &str,
        bus: &str,
        start: time::OffsetDateTime,
        configure: impl FnOnce(&mut M3DaemonOpts<'_>),
    ) -> Self {
        let wrapper = ResticWrapper::create(&root.join(sub).join("wrapper"), &variant.bin);
        let clock = Arc::new(FakeClock::new(start));
        let mut opts = M3DaemonOpts::new(sub, bus, &wrapper.path, Arc::clone(&clock) as _);
        configure(&mut opts);
        let daemon = spawn_m3_daemon(root, opts);
        Self {
            daemon,
            clock,
            wrapper,
            variant: variant.clone(),
            root: root.to_path_buf(),
            sub: sub.to_string(),
        }
    }

    /// Adds a local destination `name` below the sandbox of this rig ([`add_local_destination`]).
    pub fn add_destination(
        &self,
        name: &str,
        configure: impl FnOnce(&mut Destination),
    ) -> Destination {
        let repo_dir = self.root.join(&self.sub).join(format!("repo-{name}"));
        add_local_destination(
            &self.daemon.client,
            &self.variant.bin,
            &repo_dir,
            name,
            configure,
        )
        .0
    }

    /// Adds a daily 12:00 backup `name` with a one-file source to `dest` ([`add_folder_backup`]).
    pub fn add_backup_to(
        &self,
        dest: &Destination,
        name: &str,
        configure: impl FnOnce(&mut Backup),
    ) -> Backup {
        let src_dir = self.root.join(&self.sub).join(format!("src-{name}"));
        add_folder_backup(&self.daemon.client, &src_dir, &dest.id, name, configure)
    }

    /// Adds a local destination and a daily 12:00 backup `name` ([`add_local_backup`]).
    pub fn add_backup(
        &self,
        name: &str,
        configure: impl FnOnce(&mut Backup),
    ) -> LocalBackupFixture {
        add_local_backup(
            &self.daemon.client,
            &self.variant.bin,
            &self.root,
            &self.sub,
            name,
            configure,
        )
    }
}

/// Polls `GetState` until `pred` holds; fails with a state dump after `timeout`.
pub fn wait_for_state<F>(
    client: &KesticClient,
    timeout: Duration,
    label: &str,
    mut pred: F,
) -> DaemonStateDto
where
    F: FnMut(&DaemonStateDto) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let state = client.get_state().expect("GetState");
        if pred(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for condition ({label}); last overall={:?}\n  blocking={:?}, \
             secret store available={} locked={}, restic={:?}\n  backups={:?}",
            state.overall,
            state.blocking,
            state.secret_store_status.available,
            state.secret_store_status.locked,
            state.restic_status,
            state
                .backups
                .iter()
                .map(|b| (
                    &b.backup.name,
                    &b.pending_primary_code,
                    b.is_paused,
                    b.is_overdue,
                    b.has_error,
                    b.active_run.as_ref().map(|r| &r.status),
                    b.last_run.as_ref().map(|r| (&r.status, &r.summary_message)),
                ))
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn backup_summary<'a>(state: &'a DaemonStateDto, id: &BackupId) -> &'a BackupSummaryDto {
    state
        .backups
        .iter()
        .find(|b| &b.backup.id == id)
        .expect("backup summary present")
}

pub fn dest_summary<'a>(
    state: &'a DaemonStateDto,
    dest_id: &DestinationId,
) -> &'a DestinationSummaryDto {
    state
        .destinations
        .iter()
        .find(|d| &d.destination.id == dest_id)
        .expect("destination summary present")
}

/// Asserts that a D-Bus call failed with the given `net.daester.Kestic.Error.*` name.
pub fn assert_dbus_error_name(err: &zbus::Error, expected_name: &str) {
    match err {
        zbus::Error::MethodError(name, _, _) => assert_eq!(
            name.as_str(),
            expected_name,
            "unexpected D-Bus error: {err:?}"
        ),
        other => panic!("expected D-Bus MethodError `{expected_name}`, got {other:?}"),
    }
}

/// The isolated test keyring behind a switch: while locked, reads, writes and availability
/// probes fail with `LockedOrPromptRequired`; `unlock_interactive` unlocks it. Counts the
/// availability probes.
pub struct LockableSecretStore {
    inner: SecretServiceStore,
    pub locked: Arc<AtomicBool>,
    pub availability_checks: Arc<AtomicUsize>,
}

impl LockableSecretStore {
    pub fn new(locked: bool) -> Self {
        Self {
            inner: super::isolated_secret_store(),
            locked: Arc::new(AtomicBool::new(locked)),
            availability_checks: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn fail_if_locked(&self) -> Result<(), SecretError> {
        if self.locked.load(Ordering::SeqCst) {
            return Err(SecretError::LockedOrPromptRequired(
                "locked for test".to_string(),
            ));
        }
        Ok(())
    }
}

impl SecretStore for LockableSecretStore {
    fn set_secret(
        &self,
        target_id: &str,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError> {
        self.fail_if_locked()?;
        self.inner.set_secret(target_id, kind, value)
    }

    fn get_secret(&self, target_id: &str, kind: &str) -> Result<Option<SecretValue>, SecretError> {
        self.fail_if_locked()?;
        self.inner.get_secret(target_id, kind)
    }

    fn delete_secret(&self, target_id: &str, kind: &str) -> Result<(), SecretError> {
        self.inner.delete_secret(target_id, kind)
    }

    fn check_availability(&self) -> Result<(), SecretError> {
        self.availability_checks.fetch_add(1, Ordering::SeqCst);
        self.fail_if_locked()?;
        self.inner.check_availability()
    }

    fn unlock_interactive(&self) -> Result<(), SecretError> {
        self.locked.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// Notifications and StatusNotifierWatcher fakes on the session bus.
pub struct DesktopFakes {
    pub notif: Arc<NotificationCapture>,
    pub sni_conn: zbus::Connection,
    _notif_conn: zbus::Connection,
}

impl DesktopFakes {
    pub fn start() -> Self {
        let (notif_conn, notif) = super::start_fake_notifications();
        Self {
            notif,
            sni_conn: super::start_fake_sni_watcher(),
            _notif_conn: notif_conn,
        }
    }
}

/// Power and network state of [`StdFakes::start_with`]; the default is AC power at 100 % and an
/// unmetered connection to the known network `Heim`.
pub struct FakeSetup<'a> {
    pub on_battery: bool,
    pub percentage: f64,
    pub connected: bool,
    pub metered: bool,
    pub active_network: Option<&'a str>,
    pub known_networks: &'a [&'a str],
}

impl Default for FakeSetup<'_> {
    fn default() -> Self {
        Self {
            on_battery: false,
            percentage: 100.0,
            connected: true,
            metered: false,
            active_network: Some("Heim"),
            known_networks: &["Heim"],
        }
    }
}

/// The fakes every M3 daemon test needs on the session bus: notifications, a
/// StatusNotifierWatcher, UPower (AC power) and NetworkManager (connected to `Heim`).
pub struct StdFakes {
    pub upower: FakeUPowerHandle,
    pub nm: FakeNetworkManagerHandle,
    pub notif: Arc<NotificationCapture>,
    pub sni_conn: zbus::Connection,
    _notif_conn: zbus::Connection,
}

impl StdFakes {
    /// On AC power at 100 %.
    pub fn start() -> Self {
        Self::start_with(FakeSetup::default())
    }

    pub fn start_with_power(on_battery: bool, percentage: f64) -> Self {
        Self::start_with(FakeSetup {
            on_battery,
            percentage,
            ..FakeSetup::default()
        })
    }

    pub fn start_with(setup: FakeSetup<'_>) -> Self {
        let desktop = DesktopFakes::start();
        Self {
            upower: FakeUPowerHandle::start(setup.on_battery, setup.percentage),
            nm: FakeNetworkManagerHandle::start(
                setup.connected,
                setup.metered,
                setup.active_network,
                setup.known_networks,
            ),
            notif: desktop.notif,
            sni_conn: desktop.sni_conn,
            _notif_conn: desktop._notif_conn,
        }
    }
}

/// Polls `pred` every 25 ms until it holds; fails with `what` after `timeout`.
pub fn wait_until_within(timeout: Duration, what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !pred() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        thread::sleep(Duration::from_millis(25));
    }
}

/// [`wait_until_within`] with the default timeout of 10 s.
pub fn wait_until(what: &str, pred: impl FnMut() -> bool) {
    wait_until_within(Duration::from_secs(10), what, pred);
}

/// Copies a prepared repository to `repo_dir` and connects it with the real `restic_bin`
/// (`ConnectExisting`), storing its generated password in the isolated test keyring (the
/// default secret store of [`spawn_m3_daemon`]). Returns the destination and its password.
pub fn seed_local_destination(
    root: &Path,
    restic_bin: &Path,
    name: &str,
    repo_dir: &Path,
) -> (Destination, String) {
    let repo = copy_template_repo(restic_bin, repo_dir);
    let dest = Destination::new(
        name,
        DestinationKind::Local {
            path: repo_dir.to_path_buf(),
        },
    )
    .expect("destination");
    let resolved = check_binary_version(restic_bin).expect("resolve restic");
    let store = super::isolated_secret_store();
    let kestic = find_kestic_binary();
    let cache = root.join("seed-restic-cache");
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &kestic,
        secret_store: &store,
        clock: &clock,
        cache_dir: Some(&cache),
    };
    setup_destination(
        &dest,
        DestinationSetupMode::ConnectExisting,
        &SecretValue::from_str_secret(&repo.password),
        None,
        &runtime,
    )
    .expect("connect repo");
    (dest, repo.password)
}

/// Number of runs of the backup that actually executed (neither pending nor still running).
pub fn executed_runs(state: &DaemonStateDto, bid: &BackupId) -> usize {
    state
        .recent_runs
        .iter()
        .filter(|r| r.backup_id() == Some(bid))
        .filter(|r| !matches!(r.status, RunStatus::Pending))
        .count()
}

/// Number of runs in the history below `state_dir` that satisfy `pred`.
pub fn count_runs(
    state_dir: &Path,
    mut pred: impl FnMut(&kestic_core::model::Run) -> bool,
) -> usize {
    RunHistory::open(state_dir)
        .expect("open history")
        .list_runs()
        .expect("list runs")
        .iter()
        .filter(|r| pred(r))
        .count()
}

/// Number of scheduler ticks (1 s each) to wait so an expected absence is meaningful.
const SETTLE: Duration = Duration::from_millis(2300);

/// Sleeps long enough for at least two scheduler ticks plus asynchronous worker completion.
pub fn settle() {
    thread::sleep(SETTLE);
}

/// All captured notifications carrying the KNotification event id `event`.
pub fn notifications_for(capture: &NotificationCapture, event: &str) -> Vec<CapturedNotification> {
    capture
        .snapshot()
        .into_iter()
        .filter(|n| n.event_id.as_deref() == Some(event))
        .collect()
}

/// Waits until at least `min` notifications with `event` were captured and returns them all.
pub fn wait_for_notifications(
    capture: &NotificationCapture,
    event: &str,
    min: usize,
    timeout: Duration,
) -> Vec<CapturedNotification> {
    let deadline = Instant::now() + timeout;
    loop {
        let items = notifications_for(capture, event);
        if items.len() >= min {
            return items;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {min} `{event}` notification(s); captured: {:?}",
            capture.snapshot()
        );
        thread::sleep(Duration::from_millis(40));
    }
}

/// Writes `num_files` incompressible blobs of `bytes_per_file` bytes below `dir`.
pub fn write_incompressible_dataset(dir: &Path, num_files: usize, bytes_per_file: usize) {
    fs::create_dir_all(dir).expect("mkdir dataset");
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut buf = vec![0u8; bytes_per_file];
    for idx in 0..num_files {
        for chunk in buf.as_chunks_mut::<8>().0 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *chunk = state.to_le_bytes();
        }
        let path = dir.join(format!("blob_{idx:03}.bin"));
        let mut f = fs::File::create(&path).expect("create dataset file");
        f.write_all(&buf).expect("write dataset file");
    }
}

/// What [`add_local_backup`] created: the destination, the backup and their directories.
pub struct LocalBackupFixture {
    pub dest: Destination,
    pub backup: Backup,
    pub backup_id_str: String,
    pub repo_dir: PathBuf,
    pub src_dir: PathBuf,
    /// Generated password of the repository.
    pub password: String,
}

/// Copies a prepared repository (for `restic`'s version) to `repo_dir` and adds it as local
/// destination `name` through the daemon API (`"connect"`); `configure` adjusts the destination
/// before it is added. Returns the destination and its generated password.
pub fn add_local_destination(
    client: &KesticClient,
    restic: &Path,
    repo_dir: &Path,
    name: &str,
    configure: impl FnOnce(&mut Destination),
) -> (Destination, String) {
    let repo = copy_template_repo(restic, repo_dir);
    let mut dest = Destination::new(
        name,
        DestinationKind::Local {
            path: repo_dir.to_path_buf(),
        },
    )
    .expect("dest");
    configure(&mut dest);
    client
        .add_destination(&dest, "connect", &repo.password, "")
        .expect("add destination");
    (dest, repo.password)
}

/// Copies a prepared repository into the directory of the local destination `dest` and connects
/// it through the daemon API (`"connect"`). Returns the generated repository password.
pub fn connect_local_repo(client: &KesticClient, restic: &Path, dest: &Destination) -> String {
    let DestinationKind::Local { path } = &dest.kind else {
        panic!("connect_local_repo needs a local destination");
    };
    let repo = copy_template_repo(restic, path);
    client
        .add_destination(dest, "connect", &repo.password, "")
        .expect("add local destination");
    repo.password
}

/// Copies a prepared repository into the directory behind `server` and connects it as REST
/// destination `dest` through the daemon API (`"connect"`, with the server password when the
/// server has authentication). Returns the generated repository password.
pub fn connect_rest_repo(
    client: &KesticClient,
    restic: &Path,
    server: &RestServerGuard,
    dest: &Destination,
) -> String {
    let repo = copy_template_repo(restic, &server.repo_dir());
    let backend_password = server.auth().map_or("", |(_, password)| password);
    client
        .add_destination(dest, "connect", &repo.password, backend_password)
        .expect("add REST destination");
    repo.password
}

/// Creates a folder backup `name` (daily 12:00) on destination `dest_id` through the daemon API;
/// `src_dir` gets a one-file source. The returned backup carries the id assigned by the daemon.
pub fn add_folder_backup(
    client: &KesticClient,
    src_dir: &Path,
    dest_id: &DestinationId,
    name: &str,
    configure: impl FnOnce(&mut Backup),
) -> Backup {
    fs::create_dir_all(src_dir).expect("mkdir src");
    fs::write(src_dir.join("data.txt"), format!("payload {name}")).expect("write src");
    let mut backup = Backup::new(name, Source::folder(src_dir), dest_id.clone()).expect("backup");
    backup.schedule = Schedule::Daily {
        hour: 12,
        minute: 0,
    };
    configure(&mut backup);
    let backup_id_str = client.add_backup(&backup).expect("add backup");
    backup.id = BackupId::parse(&backup_id_str).expect("backup id");
    backup
}

/// Creates a folder backup `name` (daily 12:00) with a one-file source and a local destination
/// (a copy of the prepared repository, [`add_local_destination`]) through the daemon API.
pub fn add_local_backup(
    client: &KesticClient,
    restic: &Path,
    root: &Path,
    sub: &str,
    name: &str,
    configure: impl FnOnce(&mut Backup),
) -> LocalBackupFixture {
    let repo_dir = root.join(sub).join(format!("repo-{name}"));
    let src_dir = root.join(sub).join(format!("src-{name}"));
    let (dest, password) =
        add_local_destination(client, restic, &repo_dir, &format!("Dest {name}"), |_| {});
    let backup = add_folder_backup(client, &src_dir, &dest.id, name, configure);
    LocalBackupFixture {
        dest,
        backup_id_str: backup.id.as_str().to_string(),
        backup,
        repo_dir,
        src_dir,
        password,
    }
}

/// One line of the restic wrapper call log.
#[derive(Debug, Clone)]
pub struct WrapperEntry {
    /// `S` = process start, `E` = process end (with exit code in `detail`), `T` = SIGTERM received,
    /// `L` = stopped while holding its lock (with the number of lock files in `detail`).
    pub kind: char,
    pub at_ms: u128,
    pub sub: String,
    pub detail: String,
}

/// Shell wrapper around the real restic binary that records start, end, and SIGTERM per call,
/// can hold `backup` runs until released (progress `25 %` is emitted while held) and can make a
/// subcommand fail with a given exit code and stderr.
pub struct ResticWrapper {
    pub path: PathBuf,
    pub log: PathBuf,
    /// Prefix of the per-subcommand hold markers (`<hold>.<subcommand>`).
    pub hold: PathBuf,
    /// Prefix of the per-subcommand failure markers (`<fail>.<subcommand>`).
    fail: PathBuf,
}

impl ResticWrapper {
    pub fn create(dir: &Path, real_restic: &Path) -> Self {
        fs::create_dir_all(dir).expect("mkdir wrapper dir");
        let path = dir.join("restic-wrapper.sh");
        let log = dir.join("restic-wrapper.log");
        let hold = dir.join("restic-wrapper.hold");
        let script = format!(
            r#"#!/bin/sh
REAL='{real}'
LOG='{log}'
HOLD='{hold}'
FAIL='{fail}'
now() {{ date +%s%3N; }}
sub=""
for a in "$@"; do
  case "$a" in
    backup|forget|prune|check|unlock|init|snapshots|stats|list|key|version|cat|ls|find|restore|mount|diff|copy|rewrite|tag|cache)
      sub="$a"; break;;
  esac
done
echo "S $(now) $sub $*" >> "$LOG"
if [ -n "$sub" ] && [ -f "$FAIL.$sub" ]; then
  code=$(cat "$FAIL.$sub")
  cat "$FAIL.$sub.stderr" >&2
  echo "E $(now) $sub $code" >> "$LOG"
  exit "$code"
fi
# A held call prints the repeated output, else the one-off output, else (backup) one progress line;
# a SIGTERM while it prints or waits is logged like one during the real call. A call held with
# its lock starts the real restic instead (below).
if [ -n "$sub" ] && [ -f "$HOLD.$sub" ] && [ ! -f "$HOLD.$sub.locked" ]; then
  trap 'echo "T $(now) $sub SIGTERM" >> "$LOG"; echo "E $(now) $sub 143" >> "$LOG"; exit 143' TERM INT
  if [ -f "$HOLD.$sub.repeat" ]; then
    while [ -f "$HOLD.$sub" ] && cat "$HOLD.$sub.repeat" 2>/dev/null; do sleep 0.05; done
  elif [ -f "$HOLD.$sub.status" ]; then
    cat "$HOLD.$sub.status"
  elif [ "$sub" = "backup" ]; then
    printf '{{"message_type":"status","percent_done":0.25,"files_done":3,"bytes_done":1048576,"total_bytes":4194304,"seconds_elapsed":1,"seconds_remaining":30}}\n'
  fi
  while [ -f "$HOLD.$sub" ]; do sleep 0.1; done
  trap - TERM INT
fi
# Held with its lock: once the real restic created a lock file in the folder named by the marker,
# it is stopped (logged as `L` with the number of lock files) until the hold ends, 60 s at most.
# restic writes a lock under a temporary name first; only the final, hex-named file counts.
locks=""
lock_files() {{ ls "$locks" 2>/dev/null | grep -E '^[0-9a-f]{{64}}$'; }}
if [ -n "$sub" ] && [ -f "$HOLD.$sub.locked" ]; then
  locks=$(cat "$HOLD.$sub.locked")
  before=$(lock_files)
fi
"$REAL" "$@" &
pid=$!
stop=""
trap 'echo "T $(now) $sub SIGTERM" >> "$LOG"; stop=1; kill -TERM $pid 2>/dev/null; kill -CONT $pid 2>/dev/null' TERM INT
if [ -n "$locks" ]; then
  i=0
  while [ -z "$stop" ] && [ $i -lt 2000 ] && [ "$(lock_files)" = "$before" ]; do sleep 0.005; i=$((i+1)); done
  [ -z "$stop" ] && kill -STOP $pid 2>/dev/null
  echo "L $(now) $sub $(lock_files | wc -l)" >> "$LOG"
  i=0
  while [ -z "$stop" ] && [ -f "$HOLD.$sub" ] && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done
  kill -CONT $pid 2>/dev/null
fi
wait $pid
code=$?
if kill -0 $pid 2>/dev/null; then wait $pid; code=$?; fi
echo "E $(now) $sub $code" >> "$LOG"
exit $code
"#,
            real = real_restic.display(),
            log = log.display(),
            hold = hold.display(),
            fail = dir.join("restic-wrapper.fail").display(),
        );
        super::tools::write_executable(&path, &script);
        fs::write(&log, "").expect("create wrapper log");
        let fail = dir.join("restic-wrapper.fail");
        Self {
            path,
            log,
            hold,
            fail,
        }
    }

    fn fail_marker(&self, sub: &str) -> PathBuf {
        PathBuf::from(format!("{}.{sub}", self.fail.display()))
    }

    /// Makes every subsequent call of restic subcommand `sub` print `stderr` and exit with
    /// `code` without touching the repository, until [`Self::clear_fail`].
    pub fn fail(&self, sub: &str, code: i32, stderr: &str) {
        fs::write(
            PathBuf::from(format!("{}.stderr", self.fail_marker(sub).display())),
            format!("{stderr}\n"),
        )
        .expect("write failure stderr");
        fs::write(self.fail_marker(sub), code.to_string()).expect("write failure marker");
    }

    /// Lets calls of restic subcommand `sub` run normally again.
    pub fn clear_fail(&self, sub: &str) {
        fs::remove_file(self.fail_marker(sub)).expect("remove failure marker");
    }

    fn hold_marker(&self, sub: &str) -> PathBuf {
        PathBuf::from(format!("{}.{sub}", self.hold.display()))
    }

    /// Makes every subsequent call of restic subcommand `sub` block until [`Self::release`]
    /// (a held `backup` additionally reports 25 % progress after the search for files).
    pub fn hold(&self, sub: &str) {
        fs::write(self.hold_marker(sub), b"1").expect("create hold marker");
    }

    /// Like [`Self::hold`], but a held call first prints `ndjson` (restic `--json` lines)
    /// instead of the default progress line.
    pub fn hold_with_output(&self, sub: &str, ndjson: &str) {
        fs::write(
            PathBuf::from(format!("{}.{sub}.status", self.hold.display())),
            ndjson,
        )
        .expect("write held output");
        self.hold(sub);
    }

    /// Like [`Self::hold`], but a held call prints `ndjson` over and over until
    /// [`Self::release`], about 20 times a second like a real restic: progress keeps coming for
    /// as long as the call is held, without a flood of lines and progress signals that a busy
    /// test machine works off only seconds later.
    pub fn hold_with_repeated_output(&self, sub: &str, ndjson: &str) {
        fs::write(
            PathBuf::from(format!("{}.{sub}.repeat", self.hold.display())),
            ndjson,
        )
        .expect("write repeated output");
        self.hold(sub);
    }

    /// Like [`Self::hold`], but the real restic starts and is stopped (`SIGSTOP`) right after
    /// it created its lock file in `locks_dir` (the `locks` folder of the repository), so the
    /// held call keeps its lock until [`Self::release`]. The wrapper logs `L` with the number of
    /// lock files once it is stopped ([`Self::wait_for`] with kind `'L'`).
    pub fn hold_with_lock(&self, sub: &str, locks_dir: &Path) {
        fs::write(
            PathBuf::from(format!("{}.{sub}.locked", self.hold.display())),
            locks_dir.as_os_str().as_encoded_bytes(),
        )
        .expect("write lock hold marker");
        self.hold(sub);
    }

    /// Lets held (and future) calls of restic subcommand `sub` run on and drops the output
    /// set by [`Self::hold_with_output`] or [`Self::hold_with_repeated_output`] and the hold of
    /// [`Self::hold_with_lock`].
    pub fn release(&self, sub: &str) {
        let _ = fs::remove_file(self.hold_marker(sub));
        for kind in ["status", "repeat", "locked"] {
            let _ = fs::remove_file(PathBuf::from(format!(
                "{}.{sub}.{kind}",
                self.hold.display()
            )));
        }
    }

    pub fn entries(&self) -> Vec<WrapperEntry> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut it = line.splitn(4, ' ');
                let kind = it.next()?.chars().next()?;
                let at_ms: u128 = it.next()?.parse().ok()?;
                let sub = it.next()?.to_string();
                let detail = it.next().unwrap_or("").to_string();
                Some(WrapperEntry {
                    kind,
                    at_ms,
                    sub,
                    detail,
                })
            })
            .collect()
    }

    /// Entries of kind `kind` for restic subcommand `sub` in log order.
    pub fn of(&self, kind: char, sub: &str) -> Vec<WrapperEntry> {
        self.entries()
            .into_iter()
            .filter(|e| e.kind == kind && e.sub == sub)
            .collect()
    }

    /// Number of started restic processes that touch the repository (everything except the
    /// repository-independent `restic version` probe).
    pub fn started_repo_count(&self) -> usize {
        self.entries()
            .iter()
            .filter(|e| e.kind == 'S' && e.sub != "version")
            .count()
    }

    /// Waits until at least `min` calls of `kind`/`sub` were logged.
    pub fn wait_for(&self, kind: char, sub: &str, min: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.of(kind, sub).len() < min {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {min} wrapper `{kind} {sub}` entries; log: {:?}",
                self.entries()
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
}

/// Kills and reaps the child on drop (stops stray restic helper processes on test failure).
pub struct DropKillChild(pub std::process::Child);

impl Drop for DropKillChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Reads the persisted scheduling/backoff state of `backup_id` from the daemon's `history.db`.
pub fn runtime_state(state_dir: &Path, backup_id: &BackupId) -> BackupRuntimeState {
    RunHistory::open(state_dir)
        .expect("open history")
        .get_backup_runtime_state(backup_id)
        .expect("backup runtime state")
}

/// Waits until the persisted backoff `attempt` counter of the backup equals `attempt`.
pub fn wait_for_backoff_attempt(
    state_dir: &Path,
    backup_id: &BackupId,
    attempt: u32,
    timeout: Duration,
) -> BackupRuntimeState {
    let deadline = Instant::now() + timeout;
    loop {
        let rs = runtime_state(state_dir, backup_id);
        if rs.backoff_attempt == attempt {
            return rs;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for backoff attempt {attempt}; state: {rs:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// `restic -r <repo>` command with the repository password.
pub fn restic_cmd(bin: &Path, repo: &Path, password: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(bin);
    cmd.arg("-r").arg(repo).env("RESTIC_PASSWORD", password);
    cmd
}

/// `restic -r <repo>` command with password and cache dir from the isolated test environment.
pub fn restic_repo_cmd(
    bin: &Path,
    repo: &Path,
    password: &str,
    cache_dir: &Path,
) -> std::process::Command {
    let mut cmd = restic_cmd(bin, repo, password);
    cmd.env("RESTIC_CACHE_DIR", cache_dir);
    cmd
}

/// Spawns a foreign restic process (stdin piped) and waits until it created a lock in `repo_dir`.
pub fn hold_repo_lock(mut cmd: std::process::Command, repo_dir: &Path) -> DropKillChild {
    use std::process::Stdio;
    let child = DropKillChild(
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn lock holder"),
    );
    let locks = repo_dir.join("locks");
    let deadline = Instant::now() + Duration::from_secs(10);
    while find_lock_file(&locks).is_none() {
        assert!(Instant::now() < deadline, "lock never appeared");
        thread::sleep(Duration::from_millis(1));
    }
    child
}

/// Sends `signal` (e.g. `STOP`, `CONT`, `TERM`) to `pid`.
pub fn send_signal(pid: u32, signal: &str) {
    let status = std::process::Command::new("kill")
        .args([&format!("-{signal}"), &pid.to_string()])
        .status()
        .expect("kill");
    assert!(status.success(), "kill -{signal} {pid}");
}
