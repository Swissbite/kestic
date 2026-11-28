// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Core daemon state (`DaemonCore`), configuration, availability/run cache, and state/tray DTO builders.

use super::error::{KesticServiceErr, map_provider_err};
use super::previews::PreviewEntry;
use super::restic_status::restic_status_with;
use super::restore_runner::{PendingRestore, RestoreClaim};
use super::run_errors::unreadable_items_from_lines;
use super::snapshot_queries::SharedResult;
use super::supervision::SupervisionConfig;
use crate::dbus_api::{
    ActiveRunProgressDto, BlockingStateDto, DaemonStateDto, DestinationAvailabilityDto,
    KESTIC_BUS_NAME, ResticStatusDto, SecretStoreStatusDto,
};
use crate::i18n::tr;
use crate::job_view::DEFAULT_JOB_VIEW_DEST;
use crate::monitors::{MonitorBusConfig, read_system_environment};
use crate::secrets::SecretServiceStore;
use crate::tray::{KesticTrayState, TrayHandle, TrayStatus};
use kestic_core::backup::{BackupDryRunResult, ForgetPreviewResult, default_state_dir};
use kestic_core::clock::{Clock, SystemClock};
use kestic_core::config::{AppConfig, load_optional_config, save_config};
use kestic_core::history::{
    BackupRuntimeState, DestinationMaintenanceState, HistoryError, PauseState, RunHistory,
    last_successful_backup_at,
};
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, ResticProvider, Run, RunId, RunKind, RunStatus,
};
use kestic_core::mount::{MountInspector, SourceCheckResult, SystemMountInspector};
use kestic_core::provider::{ResolvedRestic, ResticLocations, ResticProviderError};
use kestic_core::query_list::{QueryLimits, QueryStore};
use kestic_core::restic::{CancelHandle, ResticRuntime};
use kestic_core::scheduler::{
    SystemEnvironment, TimezoneSpec, UnmetPrecondition, format_local_hhmm, format_local_slot_hint,
};
use kestic_core::secrets::{SecretError, SecretStore};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::{BlockingKind, PendingReasonCode};
use kestic_i18n::messages::{interrupted_by_sleep_message, interrupted_by_stop_message};
use kestic_i18n::messages::{pause_message, unmet_conditions_message};
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use time::OffsetDateTime;
use zbus::Connection;

/// Cache TTL for destination/source availability checks.
pub const AVAILABILITY_CACHE_TTL_SECS: i64 = 60;

/// Cooldown before scheduled runs re-probe a locked password store or unusable restic
/// (`restic version` + Secret Service), so a blocking state is not re-probed on every tick.
pub const BLOCKING_PROBE_RETRY_SECS: i64 = 60;

/// Probe result tuple cached for the state DTO.
pub(super) type ProbeTuple = (
    ResticStatusDto,
    SecretStoreStatusDto,
    Option<BlockingStateDto>,
);

/// What a restic call needs from [`DaemonCore`], cloned out so the lock is released while the
/// call runs.
pub(super) struct ResticDeps {
    pub(super) provider: ResticProvider,
    pub(super) kestic_binary: PathBuf,
    pub(super) secret_store: Arc<dyn SecretStore + Send + Sync>,
    pub(super) clock: Arc<dyn Clock>,
}

impl ResticDeps {
    /// Runtime for the resolved restic binary, without a cache directory.
    pub(super) fn runtime<'a>(&'a self, resolved: &'a ResolvedRestic) -> ResticRuntime<'a> {
        ResticRuntime {
            resolved,
            kestic_binary: &self.kestic_binary,
            secret_store: self.secret_store.as_ref(),
            clock: self.clock.as_ref(),
            cache_dir: None,
        }
    }
}

/// Optional status override for D-Bus tests covering later daemon states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulatedOverallStatus {
    Overdue,
    Paused,
    Waiting,
    PausedAndWaiting,
    MaintenanceRunning,
    RestoreRunning,
}

/// Configuration for starting the Kestic daemon.
#[derive(Clone)]
pub struct DaemonRunConfig {
    pub bus_name: String,
    pub assume_sni_available: bool,
    pub tray_state: KesticTrayState,
    pub ready_file: Option<PathBuf>,
    pub duration: Option<Duration>,
    pub config_path: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub kestic_binary: Option<PathBuf>,
    pub clock: Arc<dyn Clock>,
    pub secret_store: Option<Arc<dyn SecretStore + Send + Sync>>,
    pub mount_inspector: Option<Arc<dyn MountInspector>>,
    pub suppress_ui_spawn: bool,
    pub enable_job_view: bool,
    pub job_view_dest: String,
    pub enable_portal_inhibit: bool,
    pub simulated_status: Option<SimulatedOverallStatus>,
    pub timezone: TimezoneSpec,
    pub monitor_bus: MonitorBusConfig,
    pub enable_system_monitors: bool,
    pub enable_scheduler: bool,
    pub enable_binary_watch: bool,
    pub binary_watch_args: Option<Vec<String>>,
    /// `PATH` searched for the programs of the destination types (`ListDestinationTypes`);
    /// `None` uses the daemon's own `PATH`.
    pub program_search_path: Option<OsString>,
    /// Memory limits for the lists of snapshot queries.
    pub query_limits: QueryLimits,
    /// Set on `SIGTERM` ([`super::install_stop_signal`]); the daemon then stops its runs and
    /// ends. `None` without a handler.
    pub stop_signal: Option<Arc<AtomicBool>>,
}

impl Default for DaemonRunConfig {
    fn default() -> Self {
        Self {
            bus_name: KESTIC_BUS_NAME.to_string(),
            assume_sni_available: true,
            tray_state: KesticTrayState {
                status: TrayStatus::Passive,
                ..Default::default()
            },
            ready_file: None,
            duration: None,
            config_path: None,
            state_dir: None,
            kestic_binary: None,
            clock: Arc::new(SystemClock),
            secret_store: None,
            mount_inspector: None,
            suppress_ui_spawn: false,
            enable_job_view: true,
            job_view_dest: DEFAULT_JOB_VIEW_DEST.to_string(),
            enable_portal_inhibit: true,
            simulated_status: None,
            timezone: TimezoneSpec::System,
            monitor_bus: MonitorBusConfig::default(),
            enable_system_monitors: false,
            enable_scheduler: false,
            enable_binary_watch: false,
            binary_watch_args: None,
            program_search_path: None,
            query_limits: QueryLimits::default(),
            stop_signal: None,
        }
    }
}

/// Outcome of running [`super::run_daemon`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonOutcome {
    /// Another daemon already owned the D-Bus name; this instance exited cleanly.
    AlreadyRunning,
    /// This instance acquired the D-Bus lock and ran until requested to stop.
    RanAndStopped,
}

/// The detail of a request refused because the daemon is stopping.
pub(super) const STOPPING: &str = "the daemon is stopping";

/// Why the daemon stopped a run: it ends as `Interrupted`, not as cancelled by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Interruption {
    /// The system goes to sleep.
    Sleep,
    /// The daemon got `SIGTERM` (logout, shutdown).
    Stop,
}

impl Interruption {
    /// The run's summary, in the daemon's language.
    pub(super) fn summary(self) -> String {
        match self {
            Self::Sleep => tr(&interrupted_by_sleep_message()),
            Self::Stop => tr(&interrupted_by_stop_message()),
        }
    }
}

pub(super) struct ActiveRunEntry {
    pub(super) run: Run,
    pub(super) backup_id: Option<BackupId>,
    pub(super) backup_name: String,
    pub(super) progress: ActiveRunProgressDto,
    pub(super) cancel: CancelHandle,
    /// Why the run was stopped by the daemon, not the user (it ends as `Interrupted`).
    pub(super) interruption: Option<Interruption>,
    /// The run holds the exclusive repository lock (a backup's `forget`).
    pub(super) exclusive: bool,
}

impl ActiveRunEntry {
    pub(super) fn is_restore(&self) -> bool {
        matches!(self.run.kind, RunKind::Restore { .. })
    }
}

/// Core stateful daemon engine shared between D-Bus interface methods, tray actions, and unit tests.
pub struct DaemonCore {
    pub(super) config_path: PathBuf,
    pub(super) state_dir: PathBuf,
    pub(super) kestic_binary: PathBuf,
    pub(super) clock: Arc<dyn Clock>,
    pub(crate) secret_store: Arc<dyn SecretStore + Send + Sync>,
    pub(super) mount_inspector: Arc<dyn MountInspector>,
    pub(super) config: AppConfig,
    pub(super) has_config_file: bool,
    pub(super) config_load_error: Option<String>,
    pub(super) history: RunHistory,
    pub(super) runs_cache: Vec<Run>,
    pub(super) last_availability_check: Option<OffsetDateTime>,
    pub(super) dest_availability: HashMap<DestinationId, DestinationAvailabilityDto>,
    pub(super) source_availability: HashMap<BackupId, SourceCheckResult>,
    pub(super) availability_probe_count: u64,
    pub(super) availability_probe_in_flight: bool,
    /// The background check of the destinations only restic can reach is running.
    pub(super) restic_availability_in_flight: bool,
    pub(super) availability_probe_started_at: Option<OffsetDateTime>,
    pub(super) availability_probe_generation: u64,
    pub(super) source_probe_in_flight: bool,
    pub(super) source_probe_started_at: Option<OffsetDateTime>,
    pub(super) source_probe_generation: u64,
    pub(super) cached_probe: ProbeTuple,
    pub(super) blocking_probe_retry_at: Option<OffsetDateTime>,
    pub(super) backup_state_cache: HashMap<BackupId, BackupRuntimeState>,
    pub(super) dest_state_cache: HashMap<DestinationId, DestinationMaintenanceState>,
    pub(super) global_pause_cache: Option<PauseState>,
    /// Destinations with a scheduled start/maintenance dispatched to a worker thread
    /// (value: the backup being started, `None` for maintenance).
    pub(super) scheduled_dispatch: HashMap<DestinationId, Option<BackupId>>,
    /// Unreadable items of finished backup runs, parsed from their logs outside the
    /// `DaemonCore` lock (startup / run worker), so state building never reads log files.
    pub(super) run_unreadable_cache: HashMap<RunId, Vec<String>>,
    pub(super) active_runs: HashMap<RunId, ActiveRunEntry>,
    pub(super) claiming_destinations: HashSet<DestinationId>,
    pub(super) queued_backups: VecDeque<BackupId>,
    /// Restores waiting for maintenance of their destination to end.
    pub(super) pending_restores: Vec<PendingRestore>,
    /// Every accepted restore until its run ends (starting, waiting or running): its
    /// destination and resolved target folder.
    pub(super) restore_claims: HashMap<RunId, RestoreClaim>,
    pub(super) sleep_in_progress: bool,
    pub(super) dry_runs: HashMap<String, PreviewEntry<BackupDryRunResult>>,
    pub(super) retention_previews: HashMap<String, PreviewEntry<ForgetPreviewResult>>,
    /// Installations of the restic managed by Kestic by `install_id`.
    pub(super) restic_installs: super::restic_install::InstallMap,
    /// Removal of what an interrupted installation of an earlier daemon left behind.
    pub(super) install_leftovers: super::restic_install::LeftoverSweep,
    /// SFTP destinations being added or moved to another server right now.
    pub(super) sftp_setups: super::host_keys::SftpSetups,
    pub(super) snapshot_queries: QueryStore<SharedResult>,
    pub(super) window_active: bool,
    pub(super) window_page: String,
    pub(crate) registered_gui_sender: Option<String>,
    pub(super) simulated_status: Option<SimulatedOverallStatus>,
    pub(super) icon_theme_path: String,
    pub(super) enable_job_view: bool,
    pub(super) job_view_dest: String,
    pub(super) enable_portal_inhibit: bool,
    pub(super) suppress_ui_spawn: bool,
    pub(super) timezone: TimezoneSpec,
    pub(crate) monitor_bus: MonitorBusConfig,
    pub(crate) program_search_path: Option<OsString>,
    pub(super) enable_system_monitors: bool,
    pub(super) system_env: SystemEnvironment,
    pub(super) dest_stale_locks: HashMap<DestinationId, usize>,
    pub(super) sleep_interrupted_backups: HashSet<BackupId>,
    pub(crate) conn: Option<Connection>,
    pub(crate) tray: Option<Arc<TrayHandle>>,
    pub(super) shutdown_flag: Arc<AtomicBool>,
    /// The flag of the `SIGTERM` handler, if one is installed.
    pub(super) stop_signal: Option<Arc<AtomicBool>>,
    pub(super) quit_tx: Option<mpsc::Sender<()>>,
}

impl DaemonCore {
    /// Initializes daemon state, opens SQLite `RunHistory`, recovers stuck `Running` runs as
    /// `Interrupted`, initializes baselines, and loads `config.toml` if present.
    pub fn new(run_config: &DaemonRunConfig) -> Result<Self, String> {
        let config_path = match &run_config.config_path {
            Some(p) => p.clone(),
            None => kestic_core::config::default_config_path().map_err(|e| e.to_string())?,
        };
        let state_dir = match &run_config.state_dir {
            Some(d) => d.clone(),
            None => default_state_dir().map_err(|e| e.to_string())?,
        };
        let kestic_binary = match &run_config.kestic_binary {
            Some(p) => p.clone(),
            None => env::current_exe().unwrap_or_else(|_| PathBuf::from("kestic")),
        };

        let secret_store: Arc<dyn SecretStore + Send + Sync> = match &run_config.secret_store {
            Some(store) => Arc::clone(store),
            None => Arc::new(SecretServiceStore::default()),
        };

        let mount_inspector: Arc<dyn MountInspector> = run_config
            .mount_inspector
            .clone()
            .unwrap_or_else(|| Arc::new(SystemMountInspector));

        let history = RunHistory::open(&state_dir).map_err(|e| e.to_string())?;
        // any runs left in `running` state from a hard crash/kill become `interrupted`.
        if let Err(err) = history.mark_interrupted_on_startup(run_config.clock.as_ref()) {
            eprintln!("kestic-daemon: failed to mark interrupted runs on startup: {err}");
        }
        let runs_cache = history.list_runs().unwrap_or_default();

        let (config, has_config_file, config_load_error) = match load_optional_config(&config_path)
        {
            Ok(Some(cfg)) => (cfg, true, None),
            Ok(None) => (AppConfig::default(), false, None),
            Err(err) => (
                AppConfig::default(),
                config_path.exists(),
                Some(err.to_string()),
            ),
        };

        let run_unreadable_cache = load_run_unreadable_cache(&runs_cache, &config);

        let system_env = if run_config.enable_system_monitors {
            read_system_environment(&run_config.monitor_bus)
        } else {
            SystemEnvironment::desktop_connected()
        };

        let cached_probe =
            probe_restic_and_secrets_raw(&config.restic_provider, secret_store.as_ref());

        let mut core = Self {
            config_path,
            state_dir,
            kestic_binary,
            clock: Arc::clone(&run_config.clock),
            secret_store,
            mount_inspector,
            config,
            has_config_file,
            config_load_error,
            history,
            runs_cache,
            last_availability_check: None,
            dest_availability: HashMap::new(),
            source_availability: HashMap::new(),
            availability_probe_count: 0,
            availability_probe_in_flight: false,
            restic_availability_in_flight: false,
            availability_probe_started_at: None,
            availability_probe_generation: 0,
            source_probe_in_flight: false,
            source_probe_started_at: None,
            source_probe_generation: 0,
            cached_probe,
            blocking_probe_retry_at: None,
            backup_state_cache: HashMap::new(),
            dest_state_cache: HashMap::new(),
            global_pause_cache: None,
            scheduled_dispatch: HashMap::new(),
            run_unreadable_cache,
            active_runs: HashMap::new(),
            claiming_destinations: HashSet::new(),
            queued_backups: VecDeque::new(),
            pending_restores: Vec::new(),
            restore_claims: HashMap::new(),
            sleep_in_progress: false,
            dry_runs: HashMap::new(),
            retention_previews: HashMap::new(),
            restic_installs: HashMap::new(),
            install_leftovers: super::restic_install::LeftoverSweep::default(),
            sftp_setups: super::host_keys::SftpSetups::default(),
            snapshot_queries: QueryStore::new(run_config.query_limits),
            window_active: false,
            window_page: String::new(),
            registered_gui_sender: None,
            simulated_status: run_config.simulated_status,
            icon_theme_path: run_config.tray_state.icon_theme_path.clone(),
            enable_job_view: run_config.enable_job_view,
            job_view_dest: run_config.job_view_dest.clone(),
            enable_portal_inhibit: run_config.enable_portal_inhibit,
            suppress_ui_spawn: run_config.suppress_ui_spawn,
            timezone: run_config.timezone.clone(),
            monitor_bus: run_config.monitor_bus.clone(),
            program_search_path: run_config.program_search_path.clone(),
            enable_system_monitors: run_config.enable_system_monitors,
            system_env,
            dest_stale_locks: HashMap::new(),
            sleep_interrupted_backups: HashSet::new(),
            conn: None,
            tray: None,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            stop_signal: run_config.stop_signal.clone(),
            quit_tx: None,
        };
        core.load_state_caches();
        core.ensure_baselines();
        core.refresh_availability_if_needed(true);
        Ok(core)
    }

    /// The destination the backup with `id` writes to.
    pub(super) fn backup_destination(
        &self,
        id: &BackupId,
    ) -> Result<Destination, KesticServiceErr> {
        let backup = self
            .config
            .backup(id)
            .ok_or_else(|| KesticServiceErr::NotFound(id.as_str().to_string()))?;
        self.config
            .destination(&backup.destination_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(backup.destination_id.as_str().to_string()))
    }

    /// The backup with `id` and the destination it writes to.
    pub(super) fn backup_with_destination(
        &self,
        id: &BackupId,
    ) -> Result<(Backup, Destination), KesticServiceErr> {
        let dest = self.backup_destination(id)?;
        let backup = self
            .config
            .backup(id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(id.as_str().to_string()))?;
        Ok((backup, dest))
    }

    /// The `PATH` the daemon starts restic with: the configured one, else its own.
    pub(crate) fn program_search_path(&self) -> Option<OsString> {
        self.program_search_path
            .clone()
            .or_else(|| std::env::var_os("PATH"))
    }

    pub(super) fn restic_deps(&self) -> ResticDeps {
        ResticDeps {
            provider: self.config.restic_provider.clone(),
            kestic_binary: self.kestic_binary.clone(),
            secret_store: Arc::clone(&self.secret_store),
            clock: Arc::clone(&self.clock),
        }
    }

    pub(super) fn supervision_config(&self) -> SupervisionConfig {
        SupervisionConfig {
            enable_job_view: self.enable_job_view,
            job_view_dest: self.job_view_dest.clone(),
            enable_portal_inhibit: self.enable_portal_inhibit,
            monitor_bus: self.monitor_bus.clone(),
            enable_system_monitors: self.enable_system_monitors,
        }
    }

    pub fn availability_probe_count(&self) -> u64 {
        self.availability_probe_count
    }

    pub fn history(&self) -> &RunHistory {
        &self.history
    }

    /// Whether a backup or maintenance run holds or claims `dest_id`. Restores keep their own
    /// accounting: they run beside backups ([`Self::has_active_restore`]).
    pub(super) fn is_destination_busy(&self, dest_id: &DestinationId) -> bool {
        self.claiming_destinations.contains(dest_id)
            || self
                .active_runs
                .values()
                .any(|e| &e.run.destination_id == dest_id && !e.is_restore())
    }

    /// Whether a restore runs on `dest_id`; maintenance waits for it.
    pub(super) fn has_active_restore(&self, dest_id: &DestinationId) -> bool {
        self.active_runs
            .values()
            .any(|e| &e.run.destination_id == dest_id && e.is_restore())
    }

    /// Whether a run on `dest_id` holds the exclusive lock (maintenance, or a backup in its
    /// `forget`); a restore waits for it.
    pub(super) fn has_exclusive_run(&self, dest_id: &DestinationId) -> bool {
        self.active_runs.values().any(|e| {
            &e.run.destination_id == dest_id
                && (e.exclusive || matches!(e.run.kind, RunKind::Maintenance))
        })
    }

    /// Marks run `run_id` as holding the exclusive lock unless a restore runs on `dest_id`;
    /// returns whether it may go on.
    pub(super) fn claim_exclusive_phase(
        &mut self,
        run_id: &RunId,
        dest_id: &DestinationId,
    ) -> bool {
        if self.has_active_restore(dest_id) {
            return false;
        }
        if let Some(entry) = self.active_runs.get_mut(run_id) {
            entry.exclusive = true;
        }
        true
    }

    pub(super) fn upsert_run_cached(
        &mut self,
        run: &Run,
        secrets: &[&str],
    ) -> Result<(), HistoryError> {
        self.history.upsert_run(run, secrets)?;
        if let Some(existing) = self
            .runs_cache
            .iter_mut()
            .find(|r| r.id == run.id && r.started_at == run.started_at)
        {
            *existing = run.clone();
            return Ok(());
        }
        self.runs_cache.retain(|r| r.id != run.id);
        let insert_idx = self
            .runs_cache
            .iter()
            .position(|r| r.started_at <= run.started_at)
            .unwrap_or(self.runs_cache.len());
        self.runs_cache.insert(insert_idx, run.clone());
        Ok(())
    }

    pub(super) fn delete_pending_runs_cached(
        &mut self,
        backup_id: &BackupId,
    ) -> Result<(), HistoryError> {
        if !self.has_pending_backup_run_cached(backup_id) {
            return Ok(());
        }
        self.history.delete_pending_runs_for_backup(backup_id)?;
        self.runs_cache
            .retain(|r| !(r.is_backup_run_of(backup_id) && r.status == RunStatus::Pending));
        Ok(())
    }

    pub(super) fn last_successful_backup_at_cached(
        &self,
        backup_id: &BackupId,
    ) -> Option<OffsetDateTime> {
        last_successful_backup_at(&self.runs_cache, backup_id)
    }

    pub(super) fn ensure_baselines(&mut self) {
        let now = self.clock.now_utc();
        let backup_ids: Vec<BackupId> = self.config.backups.iter().map(|b| b.id.clone()).collect();
        for id in &backup_ids {
            let mut rs = self.load_backup_runtime_state(id);
            if rs.baseline_at.is_none() {
                rs.baseline_at = Some(now);
                let _ = self.store_backup_runtime_state(id, &rs);
            }
        }
        let dest_ids: Vec<DestinationId> = self
            .config
            .destinations
            .iter()
            .map(|d| d.id.clone())
            .collect();
        for id in &dest_ids {
            let mut ds = self.load_destination_maintenance_state(id);
            if ds.baseline_at.is_none() {
                ds.baseline_at = Some(now);
                let _ = self.store_destination_maintenance_state(id, &ds);
            }
        }
    }

    /// Stores a fresh probe result; a blocking result arms the scheduled re-probe cooldown
    /// ([`BLOCKING_PROBE_RETRY_SECS`]), a clean result clears it.
    pub(super) fn record_probe(&mut self, probe: ProbeTuple) {
        self.blocking_probe_retry_at = if probe.2.is_some() {
            Some(self.clock.now_utc() + time::Duration::seconds(BLOCKING_PROBE_RETRY_SECS))
        } else {
            None
        };
        self.cached_probe = probe;
    }

    /// Whether scheduled starts must wait because the last probe found a blocking state
    /// (locked password store / unusable restic) and its re-probe cooldown is still running.
    pub(super) fn scheduled_probe_blocked(&self, now: OffsetDateTime) -> bool {
        self.cached_probe.2.is_some() && self.blocking_probe_retry_at.is_some_and(|t| now < t)
    }

    pub(super) fn format_pause_label(
        &self,
        paused_since: OffsetDateTime,
        paused_until: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> String {
        let since_str = format_local_hhmm(paused_since, &self.timezone);
        let until_str =
            paused_until.map(|until| format_local_slot_hint(until, now, &self.timezone));
        tr(&pause_message(&since_str, until_str.as_deref()))
    }

    /// Daemon-language «Condition not met: …» for the unmet preconditions (notifications, run
    /// history of skipped backups).
    pub(super) fn localized_unmet_conditions(&self, items: &[UnmetPrecondition]) -> String {
        let parts: Vec<String> = items
            .iter()
            .map(|item| {
                let allowed = match item {
                    UnmetPrecondition::AllowedNetworkMismatch { allowed } => allowed.as_slice(),
                    _ => &[],
                };
                tr(&PendingReasonCode::from(item)
                    .condition_message(self.system_env.battery_percent, allowed))
            })
            .collect();
        tr(&unmet_conditions_message(&parts.join(", ")))
    }

    /// Daemon-language «Waiting: …» line of an unmet precondition (run history).
    pub(super) fn localized_pending_reason(
        &self,
        item: &UnmetPrecondition,
        backoff_until: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> String {
        let hint = backoff_until.map_or_default(|u| format_local_slot_hint(u, now, &self.timezone));
        tr(&PendingReasonCode::from(item).waiting_message(&hint))
    }

    pub(super) fn persist_config_without_probe(
        &mut self,
        next: AppConfig,
    ) -> Result<(), KesticServiceErr> {
        if let Some(ref load_err) = self.config_load_error {
            return Err(KesticServiceErr::InvalidInput(format!(
                "refusing to overwrite incompatible config.toml: {load_err}"
            )));
        }
        save_config(&self.config_path, &next).map_err(super::error::map_config_err)?;
        self.config = next;
        self.has_config_file = true;
        self.last_availability_check = None;
        Ok(())
    }

    pub(super) fn persist_config(&mut self, next: AppConfig) -> Result<(), KesticServiceErr> {
        self.persist_config_without_probe(next)
    }

    pub(crate) fn set_window_active_state(&mut self, active: bool, page: &str) {
        self.window_active = active;
        self.window_page = page.to_string();
    }

    pub fn set_expert_mode(&mut self, enabled: bool) -> Result<(), KesticServiceErr> {
        let mut next = self.config.clone();
        next.expert_mode = enabled;
        self.persist_config(next)
    }

    /// Cancels the active run `run_id`, or a restore that still waits for maintenance (it ends
    /// as cancelled without starting).
    pub fn cancel_run(&mut self, run_id: &str) -> Result<(), KesticServiceErr> {
        let id = RunId::parse(run_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        if let Some(entry) = self.active_runs.get(&id) {
            entry.cancel.cancel();
            return Ok(());
        }
        if self.cancel_pending_restore(&id) {
            return Ok(());
        }
        Err(KesticServiceErr::NotFound(run_id.to_string()))
    }

    pub fn cancel_any_active_run(&self) -> bool {
        let mut any = false;
        for entry in self.active_runs.values() {
            entry.cancel.cancel();
            any = true;
        }
        any
    }

    /// Whether the daemon is stopping: nothing new starts. Also true from the moment `SIGTERM`
    /// arrives, before the event loop notices it.
    pub(super) fn stopping(&self) -> bool {
        self.shutdown_flag.load(Ordering::SeqCst) || self.signal_received()
    }

    /// Whether `SIGTERM` has arrived.
    pub(super) fn signal_received(&self) -> bool {
        self.stop_signal
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
    }

    pub fn request_shutdown(&self) {
        self.cancel_any_active_run();
        self.shutdown_flag.store(true, Ordering::SeqCst);
        if let Some(tx) = &self.quit_tx {
            let _ = tx.send(());
        }
    }
}

/// Parses the unreadable items of the last finished run of every configured backup from its
/// log file (startup only, before the `DaemonCore` is shared).
fn load_run_unreadable_cache(runs: &[Run], config: &AppConfig) -> HashMap<RunId, Vec<String>> {
    let mut cache = HashMap::new();
    for backup in &config.backups {
        let last = runs.iter().find(|r| {
            r.is_backup_run_of(&backup.id)
                && !matches!(r.status, RunStatus::Pending | RunStatus::Running)
        });
        if let Some(run) = last
            && run.status == RunStatus::Warning
            && let Some(path) = &run.log_path
            && std::fs::metadata(path).is_ok_and(|m| m.is_file())
            && let Ok(content) = std::fs::read_to_string(path)
        {
            cache.insert(run.id.clone(), unreadable_items_from_lines(content.lines()));
        }
    }
    cache
}

pub(super) fn probe_restic_and_secrets_raw(
    provider: &ResticProvider,
    secret_store: &dyn SecretStore,
) -> ProbeTuple {
    probe_restic_and_secrets_resolved(provider, secret_store).0
}

/// Probes restic and the password store once, returning the probe tuple and, if usable,
/// the resolved restic binary.
pub(super) fn probe_restic_and_secrets_resolved(
    provider: &ResticProvider,
    secret_store: &dyn SecretStore,
) -> (ProbeTuple, Result<ResolvedRestic, ResticProviderError>) {
    probe_restic_and_secrets_with(provider, &ResticLocations::from_env(), secret_store)
}

/// [`probe_restic_and_secrets_resolved`] with the system and the app-managed restic at
/// `locations`.
pub(super) fn probe_restic_and_secrets_with(
    provider: &ResticProvider,
    locations: &ResticLocations,
    secret_store: &dyn SecretStore,
) -> (ProbeTuple, Result<ResolvedRestic, ResticProviderError>) {
    let (restic_status, resolved_res) = restic_status_with(provider, locations);

    let secret_status = match secret_store.check_availability() {
        Ok(()) => SecretStoreStatusDto {
            available: true,
            locked: false,
            detail: None,
        },
        Err(SecretError::LockedOrPromptRequired(msg)) => SecretStoreStatusDto {
            available: false,
            locked: true,
            detail: Some(msg),
        },
        Err(err) => SecretStoreStatusDto {
            available: false,
            locked: false,
            detail: Some(err.to_string()),
        },
    };

    let blocking = blocking_state(&restic_status, &secret_status);

    ((restic_status, secret_status, blocking), resolved_res)
}

/// The blocking state: a locked password store first, then an unusable restic.
fn blocking_state(
    restic_status: &ResticStatusDto,
    secret_status: &SecretStoreStatusDto,
) -> Option<BlockingStateDto> {
    let kind = if !secret_status.available {
        BlockingKind::SecretStoreLocked
    } else if !restic_status.valid {
        BlockingKind::ResticUnavailable
    } else {
        return None;
    };
    Some(BlockingStateDto { kind })
}

/// Checks the system and the app-managed restic again and stores the new restic status; the
/// password store keeps its last status.
pub(super) fn refresh_restic_probe_unlocked(shared: &Arc<Mutex<DaemonCore>>) {
    let provider = shared.lock_unpoisoned().config.restic_provider.clone();
    let (restic_status, _) = restic_status_with(&provider, &ResticLocations::from_env());
    let mut guard = shared.lock_unpoisoned();
    // A source set meanwhile was checked by its own call.
    if guard.config.restic_provider != provider {
        return;
    }
    let secret_status = guard.cached_probe.1.clone();
    let blocking = blocking_state(&restic_status, &secret_status);
    guard.record_probe((restic_status, secret_status, blocking));
}

/// Error for a locked password store; `detail` is the reason the probe reported.
pub(super) fn secret_store_locked(detail: Option<String>) -> KesticServiceErr {
    KesticServiceErr::SecretStoreLocked(
        detail.unwrap_or_else(|| "password store is locked".to_string()),
    )
}

/// Probes the password store and restic (one `restic version`) before a call that needs both.
///
/// Returns the probe for caching and the resolved restic, or the error to report: a locked
/// store takes precedence over a restic problem.
pub(super) fn probe_for_run(
    provider: &ResticProvider,
    secret_store: &dyn SecretStore,
) -> (ProbeTuple, Result<ResolvedRestic, KesticServiceErr>) {
    let (probe, resolved) = probe_restic_and_secrets_resolved(provider, secret_store);
    let result = match resolved {
        Ok(resolved) if probe.1.available => Ok(resolved),
        Err(err) if probe.1.available => Err(map_provider_err(&err)),
        _ => Err(secret_store_locked(probe.1.detail.clone())),
    };
    (probe, result)
}

/// Resolves the restic binary for a preview run; on failure the probe is cached for the
/// scheduler's re-probe cooldown.
pub(super) fn resolve_restic_recording_probe(
    shared: &Arc<Mutex<DaemonCore>>,
    deps: &ResticDeps,
) -> Result<ResolvedRestic, KesticServiceErr> {
    let (probe, resolved) = probe_for_run(&deps.provider, deps.secret_store.as_ref());
    resolved.inspect_err(|_| shared.lock_unpoisoned().record_probe(probe))
}

/// Probes the password store and restic before a backup/maintenance run (one `restic version`).
///
/// On failure the probe is cached (arming the scheduled re-probe cooldown), the destination
/// claim is released, and the matching error is returned.
pub(super) fn probe_before_run(
    shared: &Arc<Mutex<DaemonCore>>,
    provider: &ResticProvider,
    secret_store: &dyn SecretStore,
    dest_id: &DestinationId,
    claim_guard: &mut super::availability::DestinationClaimGuard,
) -> Result<(ProbeTuple, ResolvedRestic), KesticServiceErr> {
    let (probe, result) = probe_for_run(provider, secret_store);
    match result {
        Ok(resolved) => Ok((probe, resolved)),
        Err(err) => {
            let mut guard = shared.lock_unpoisoned();
            guard.record_probe(probe);
            guard.release_claim(dest_id, claim_guard);
            Err(err)
        }
    }
}

pub(crate) fn refresh_probe_unlocked(shared: &Arc<Mutex<DaemonCore>>) {
    let (provider, secret_store) = {
        let guard = shared.lock_unpoisoned();
        (
            guard.config.restic_provider.clone(),
            Arc::clone(&guard.secret_store),
        )
    };
    let probe = probe_restic_and_secrets_raw(&provider, secret_store.as_ref());
    let mut guard = shared.lock_unpoisoned();
    guard.record_probe(probe);
}

pub(crate) fn set_restic_provider_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    provider_json: &str,
) -> Result<
    (
        ResticStatusDto,
        Option<Connection>,
        DaemonStateDto,
        KesticTrayState,
    ),
    KesticServiceErr,
> {
    let provider: ResticProvider = serde_json::from_str(provider_json)
        .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid provider JSON: {e}")))?;
    let secret_store = {
        let guard = shared.lock_unpoisoned();
        Arc::clone(&guard.secret_store)
    };
    let locations = ResticLocations::from_env();
    let (probe, resolved) =
        probe_restic_and_secrets_with(&provider, &locations, secret_store.as_ref());
    let not_installed = || {
        KesticServiceErr::InvalidInput("the restic managed by Kestic is not installed".to_string())
    };
    let binary = match resolved {
        Ok(resolved) => resolved.binary,
        Err(_) if provider == ResticProvider::AppManaged && probe.0.app_managed.is_none() => {
            return Err(not_installed());
        }
        Err(err) => return Err(map_provider_err(&err)),
    };
    let mut restic_status = probe.0.clone();

    let mut guard = shared.lock_unpoisoned();
    // `RemoveAppManagedRestic` deletes under this lock, so a binary removed since the check
    // above is gone by now; a link to it then points nowhere.
    if !binary.exists() {
        return Err(if provider == ResticProvider::AppManaged {
            not_installed()
        } else {
            KesticServiceErr::ResticUnavailable(format!("{} was removed", binary.display()))
        });
    }
    let mut next = guard.config.clone();
    next.restic_provider = provider;
    guard.persist_config(next)?;
    guard.record_probe(probe);
    restic_status.install_id = guard.running_restic_install();
    let (state_dto, tray_state) = guard.build_state_and_tray();
    Ok((restic_status, guard.conn.clone(), state_dto, tray_state))
}
