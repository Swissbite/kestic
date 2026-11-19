// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Full Kestic M1 daemon service (`net.daester.Kestic1` D-Bus server, single-writer config/history,
//! async backup worker with cancellation & Plasma JobView/Portal Inhibit, 60s availability cache,
//! SNI tray synchronization, and desktop notifications).

use crate::binary_watch::{BinaryIdentity, BinaryStatus};
use crate::dbus_api::{
    ActiveRunProgressDto, AvailabilitySnapshotDto, BackupAvailabilityDto, BackupDryRunStatusDto,
    BackupSnapshotsDto, BackupSummaryDto, BlockingStateDto, ConnectionTestResultDto,
    DaemonStateDto, DestinationAvailabilityDto, DestinationSummaryDto, ExcludeRulePreviewDto,
    KESTIC_BUS_NAME, KESTIC_OBJECT_PATH, KesticDbusError, OverallStatusDto, OverallStatusKind,
    PreviousDestinationSnapshotsDto, REASON_CONDITION_NOT_MET, REASON_DESTINATION_UNREACHABLE,
    REASON_SOURCE_UNAVAILABLE, ResticStatusDto, RetentionPreviewStatusDto, RunDetailDto,
    SecretStoreStatusDto, SkippedBackupDto, SnapshotSummaryDto, StartAllResultDto,
};
use crate::i18n::{i18n, localized_failure_summary, localized_recovery_action_label};
use crate::inhibit::PortalInhibitGuard;
use crate::job_view::{CAPABILITY_KILLABLE, DEFAULT_JOB_VIEW_DEST, JobProgress, JobViewHandle};
use crate::monitors::{
    LogindSleepInhibitor, MonitorBusConfig, SystemMonitorEvent, SystemMonitorHandle,
    list_known_networks, read_system_environment,
};
use crate::notifications::{
    KESTIC_DESKTOP_ENTRY, KesticEvent, send_configured_notification, send_event_notification,
};
use crate::secrets::SecretServiceStore;
use crate::single_instance::{InstanceAcquire, SingleInstanceLock};
use crate::tray::{
    KesticTrayState, TrayAction, TrayBackupItem, TrayHandle, TrayStatus, compute_tray_presentation,
};
use kestic_core::backup::{
    BackupDryRunResult, BackupExecutionContext, BackupRunError, ForgetPreviewResult,
    default_state_dir, exclude_file_path, execute_backup_dry_run_with,
    execute_backup_with_inspector, execute_forget, execute_forget_dry_run,
    execute_forget_dry_run_with, is_forget_lock_failure, list_snapshots_for_backup,
    write_exclude_file,
};
use kestic_core::clock::{Clock, SystemClock};
use kestic_core::config::{
    AppConfig, ConfigError, default_config_path, load_optional_config, save_config,
};
use kestic_core::destination::{
    DEFAULT_CONNECT_TIMEOUT, DestinationError, DestinationSetupMode, preflight_check_destination,
    remove_destination_secrets, setup_destination, verify_and_update_stored_password,
};
use kestic_core::history::{PauseState, RunHistory};
use kestic_core::maintenance::{
    PrunePreviewSummary, detect_stale_locks, execute_check, execute_prune, execute_prune_dry_run,
    execute_unlock,
};
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, DestinationKind, ExcludeLintWarning,
    ExcludePathTestResult, ExcludeRules, MaintenanceSchedule, NestedSourceConflict,
    NotificationSettings, PendingReason, RetentionPolicy, Run, RunId, RunStatus, Schedule, Source,
    SourceKind, check_nested_sources_for_candidate, evaluate_exclude_path,
    find_nested_source_warnings, lint_exclude_rules, normalize_folder_rule,
};
use kestic_core::mount::{
    MountInspector, SourceCheckResult, SystemMountInspector, check_source_status_with,
    current_home_dir, detect_source_with,
};
use kestic_core::provider::{ResticProviderError, resolve_and_check};
use kestic_core::restic::{
    CancelHandle, ResticCommandSpec, ResticFailureKind, ResticRuntime, ResticStreamEvent,
    SECRET_KIND_REPO, SECRET_KIND_REST, classify_exit, redact_secrets,
};
use kestic_core::scheduler::{
    BackupScheduleEvalInput, DestinationMaintenanceEvalInput, DestinationTickAction, PauseDuration,
    SystemEnvironment, TimezoneSpec, UnmetPrecondition, backup_slots_between,
    collect_pending_preconditions, compute_backoff_until, compute_pause_until,
    evaluate_backup_schedule, evaluate_destination_maintenance, evaluate_run_conditions,
    format_local_hhmm, format_local_slot_hint, is_pause_active, plan_destination_actions,
};
use kestic_core::secrets::{FileSecretStore, SecretError, SecretStore, SecretValue};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

/// Cache TTL for destination/source availability checks (`M1-S23`, `AC-M1-23-3`).
pub const AVAILABILITY_CACHE_TTL_SECS: i64 = 60;

/// Optional status override for D-Bus tests covering states added in M3/M4 (`AC-M1-11-1`).
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
    pub send_test_notification: bool,
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
}

impl Default for DaemonRunConfig {
    fn default() -> Self {
        Self {
            bus_name: KESTIC_BUS_NAME.to_string(),
            assume_sni_available: true,
            send_test_notification: false,
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
        }
    }
}

/// Outcome of running [`run_daemon`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonOutcome {
    /// Another daemon already owned the D-Bus name; this instance exited cleanly (`AC-M0-07-1`, `AC-M1-09-1`).
    AlreadyRunning,
    /// This instance acquired the D-Bus lock and ran until requested to stop.
    RanAndStopped,
}

struct ActiveRunEntry {
    run: Run,
    backup_id: Option<BackupId>,
    backup_name: String,
    progress: ActiveRunProgressDto,
    cancel: CancelHandle,
    sleep_interrupted: bool,
}

struct DryRunEntry {
    state: String,
    result: Option<BackupDryRunResult>,
    error: Option<String>,
    cancel: CancelHandle,
}

struct RetentionPreviewEntry {
    state: String,
    result: Option<ForgetPreviewResult>,
    error: Option<String>,
    cancel: CancelHandle,
}

/// Core stateful daemon engine shared between D-Bus interface methods, tray actions, and unit tests.
pub struct DaemonCore {
    config_path: PathBuf,
    state_dir: PathBuf,
    kestic_binary: PathBuf,
    clock: Arc<dyn Clock>,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    mount_inspector: Arc<dyn MountInspector>,
    config: AppConfig,
    has_config_file: bool,
    config_load_error: Option<String>,
    history: RunHistory,
    last_availability_check: Option<OffsetDateTime>,
    dest_availability: HashMap<DestinationId, DestinationAvailabilityDto>,
    source_availability: HashMap<BackupId, SourceCheckResult>,
    availability_probe_count: u64,
    active_runs: HashMap<RunId, ActiveRunEntry>,
    dry_runs: HashMap<String, DryRunEntry>,
    retention_previews: HashMap<String, RetentionPreviewEntry>,
    window_active: bool,
    window_page: String,
    registered_gui_sender: Option<String>,
    simulated_status: Option<SimulatedOverallStatus>,
    icon_theme_path: String,
    enable_job_view: bool,
    job_view_dest: String,
    enable_portal_inhibit: bool,
    suppress_ui_spawn: bool,
    timezone: TimezoneSpec,
    monitor_bus: MonitorBusConfig,
    enable_system_monitors: bool,
    system_env: SystemEnvironment,
    dest_stale_locks: HashMap<DestinationId, usize>,
    sleep_interrupted_backups: HashSet<BackupId>,
    conn: Option<Connection>,
    tray: Option<Arc<TrayHandle>>,
    shutdown_flag: Arc<AtomicBool>,
    quit_tx: Option<mpsc::Sender<()>>,
}

impl DaemonCore {
    /// Initializes daemon state, opens SQLite `RunHistory`, recovers stuck `Running` runs as
    /// `Interrupted` (`AC-M1-10-2`, `AC-M3-10-3`), initializes baselines, and loads `config.toml` if present.
    pub fn new(run_config: &DaemonRunConfig) -> Result<Self, String> {
        let config_path = match &run_config.config_path {
            Some(p) => p.clone(),
            None => default_config_path().map_err(|e| e.to_string())?,
        };
        let state_dir = match &run_config.state_dir {
            Some(d) => d.clone(),
            None => default_state_dir().map_err(|e| e.to_string())?,
        };
        let kestic_binary = match &run_config.kestic_binary {
            Some(p) => p.clone(),
            None => env::current_exe().unwrap_or_else(|_| PathBuf::from("kestic")),
        };

        let secret_store: Arc<dyn SecretStore + Send + Sync> = if let Some(store) =
            &run_config.secret_store
        {
            Arc::clone(store)
        } else if let Some(file_path) = env::var_os("KESTIC_SECRET_FILE").filter(|v| !v.is_empty())
        {
            Arc::new(FileSecretStore::new(PathBuf::from(file_path)))
        } else {
            Arc::new(SecretServiceStore::default())
        };

        let mount_inspector: Arc<dyn MountInspector> = run_config
            .mount_inspector
            .clone()
            .unwrap_or_else(|| Arc::new(SystemMountInspector));

        let history = RunHistory::open(&state_dir).map_err(|e| e.to_string())?;
        // AC-M1-10-2 / AC-M3-10-3: any runs left in `running` state from a hard crash/kill become `interrupted`.
        if let Err(err) = history.mark_interrupted_on_startup(run_config.clock.as_ref()) {
            eprintln!("kestic-daemon: failed to mark interrupted runs on startup: {err}");
        }

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

        let system_env = if run_config.enable_system_monitors {
            read_system_environment(&run_config.monitor_bus)
        } else {
            SystemEnvironment::desktop_connected()
        };

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
            last_availability_check: None,
            dest_availability: HashMap::new(),
            source_availability: HashMap::new(),
            availability_probe_count: 0,
            active_runs: HashMap::new(),
            dry_runs: HashMap::new(),
            retention_previews: HashMap::new(),
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
            enable_system_monitors: run_config.enable_system_monitors,
            system_env,
            dest_stale_locks: HashMap::new(),
            sleep_interrupted_backups: HashSet::new(),
            conn: None,
            tray: None,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            quit_tx: None,
        };
        core.ensure_baselines();
        core.refresh_availability_if_needed(true);
        Ok(core)
    }

    pub fn availability_probe_count(&self) -> u64 {
        self.availability_probe_count
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn history(&self) -> &RunHistory {
        &self.history
    }

    pub fn system_environment(&self) -> &SystemEnvironment {
        &self.system_env
    }

    pub fn set_system_environment(&mut self, env: SystemEnvironment) {
        self.system_env = env;
    }

    fn ensure_baselines(&self) {
        let now = self.clock.now_utc();
        for backup in &self.config.backups {
            if let Ok(mut rs) = self.history.get_backup_runtime_state(&backup.id)
                && rs.baseline_at.is_none()
            {
                rs.baseline_at = Some(now);
                let _ = self.history.upsert_backup_runtime_state(&backup.id, &rs);
            }
        }
        for dest in &self.config.destinations {
            if let Ok(mut ds) = self.history.get_destination_maintenance_state(&dest.id)
                && ds.baseline_at.is_none()
            {
                ds.baseline_at = Some(now);
                let _ = self
                    .history
                    .upsert_destination_maintenance_state(&dest.id, &ds);
            }
        }
    }

    /// Probes restic source and secret store health (`§6.1`, `M1-S04`, `AC-M1-13-5`).
    fn probe_restic_and_secrets(
        &self,
    ) -> (
        ResticStatusDto,
        SecretStoreStatusDto,
        Option<BlockingStateDto>,
    ) {
        let restic_status = match resolve_and_check(&self.config.restic_provider) {
            Ok(resolved) => ResticStatusDto {
                valid: true,
                binary_path: Some(resolved.binary.to_string_lossy().into_owned()),
                version: Some(resolved.version.to_string()),
                error_kind: None,
                found_version: Some(resolved.version.to_string()),
                message: format!("restic {}", resolved.version),
            },
            Err(ResticProviderError::NotFound { min_version }) => ResticStatusDto {
                valid: false,
                binary_path: None,
                version: None,
                error_kind: Some("not_found".to_string()),
                found_version: None,
                message: i18n("restic not found; at least %1 required").subs(&[&min_version]),
            },
            Err(ResticProviderError::TooOld {
                binary,
                found,
                min_version,
            }) => ResticStatusDto {
                valid: false,
                binary_path: Some(binary.to_string_lossy().into_owned()),
                version: None,
                error_kind: Some("too_old".to_string()),
                found_version: Some(found.to_string()),
                message: i18n("restic %1 is too old; Kestic needs at least %2")
                    .subs(&[&found.to_string(), &min_version]),
            },
            Err(ResticProviderError::InvalidCustomPath { path, reason }) => ResticStatusDto {
                valid: false,
                binary_path: Some(path.to_string_lossy().into_owned()),
                version: None,
                error_kind: Some("invalid_custom_path".to_string()),
                found_version: None,
                message: i18n("Custom restic path is invalid (%1): %2")
                    .subs(&[&path.to_string_lossy(), &reason]),
            },
            Err(ResticProviderError::UnreadableVersion { binary, detail }) => ResticStatusDto {
                valid: false,
                binary_path: Some(binary.to_string_lossy().into_owned()),
                version: None,
                error_kind: Some("unreadable_version".to_string()),
                found_version: None,
                message: i18n("Unable to determine restic version (%1)").subs(&[&detail]),
            },
        };

        let secret_status = match self.secret_store.check_availability() {
            Ok(()) => SecretStoreStatusDto {
                available: true,
                locked: false,
                message: None,
            },
            Err(SecretError::LockedOrPromptRequired(msg)) => SecretStoreStatusDto {
                available: false,
                locked: true,
                message: Some(msg),
            },
            Err(err) => SecretStoreStatusDto {
                available: false,
                locked: false,
                message: Some(err.to_string()),
            },
        };

        let blocking = if !secret_status.available {
            Some(BlockingStateDto {
                kind: "secret_store_locked".to_string(),
                message: i18n(
                    "The system password store (Secret Service / KWallet) is locked. Kestic cannot access destinations.",
                )
                .into(),
            })
        } else if !restic_status.valid {
            Some(BlockingStateDto {
                kind: "restic_unavailable".to_string(),
                message: restic_status.message.clone(),
            })
        } else {
            None
        };

        (restic_status, secret_status, blocking)
    }

    /// Refreshes availability (`M1-S23`) if `force == true` or > 60 s elapsed since last check.
    pub fn refresh_availability_if_needed(&mut self, force: bool) -> AvailabilitySnapshotDto {
        let now = self.clock.now_utc();
        let should_run = force
            || match self.last_availability_check {
                None => true,
                Some(prev) => (now - prev) > time::Duration::seconds(AVAILABILITY_CACHE_TTL_SECS),
            };

        if should_run {
            let now_unix = now.unix_timestamp();
            for dest in &self.config.destinations {
                let res = preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT);
                self.availability_probe_count += 1;
                let dto = match res {
                    Ok(()) => DestinationAvailabilityDto {
                        destination_id: dest.id.as_str().to_string(),
                        reachable: true,
                        detail: None,
                        checked_at_unix: now_unix,
                    },
                    Err(err) => DestinationAvailabilityDto {
                        destination_id: dest.id.as_str().to_string(),
                        reachable: false,
                        detail: Some(err.to_string()),
                        checked_at_unix: now_unix,
                    },
                };
                self.dest_availability.insert(dest.id.clone(), dto);
            }

            for backup in &self.config.backups {
                let status = check_source_status_with(
                    backup,
                    current_home_dir().as_deref(),
                    self.mount_inspector.as_ref(),
                );
                self.source_availability.insert(backup.id.clone(), status);
            }
            self.last_availability_check = Some(now);
        }

        self.current_availability_snapshot()
    }

    fn current_availability_snapshot(&self) -> AvailabilitySnapshotDto {
        let backups = self
            .config
            .backups
            .iter()
            .map(|b| self.availability_for_backup(b))
            .collect();
        let destinations = self
            .config
            .destinations
            .iter()
            .map(|d| self.availability_for_destination(d))
            .collect();
        AvailabilitySnapshotDto {
            checked_at_unix: self.last_availability_check.map(|t| t.unix_timestamp()),
            backups,
            destinations,
        }
    }

    fn availability_for_destination(&self, dest: &Destination) -> DestinationAvailabilityDto {
        self.dest_availability
            .get(&dest.id)
            .cloned()
            .unwrap_or_else(|| DestinationAvailabilityDto {
                destination_id: dest.id.as_str().to_string(),
                reachable: true,
                detail: None,
                checked_at_unix: self.clock.now_unix_secs(),
            })
    }

    fn availability_for_backup(&self, backup: &Backup) -> BackupAvailabilityDto {
        let source_status = self
            .source_availability
            .get(&backup.id)
            .cloned()
            .unwrap_or_else(|| {
                check_source_status_with(
                    backup,
                    current_home_dir().as_deref(),
                    self.mount_inspector.as_ref(),
                )
            });
        let source_available = source_status.is_ok();
        let dest_available = self
            .dest_availability
            .get(&backup.destination_id)
            .map(|d| d.reachable)
            .unwrap_or(true);

        if !source_available {
            BackupAvailabilityDto {
                backup_id: backup.id.as_str().to_string(),
                source_available: false,
                destination_available: dest_available,
                available: false,
                blocked_reason: Some(REASON_SOURCE_UNAVAILABLE.to_string()),
                reason_text: Some(localized_source_check_reason(&source_status)),
            }
        } else if !dest_available {
            BackupAvailabilityDto {
                backup_id: backup.id.as_str().to_string(),
                source_available: true,
                destination_available: false,
                available: false,
                blocked_reason: Some(REASON_DESTINATION_UNREACHABLE.to_string()),
                reason_text: Some(i18n("Destination unreachable").into()),
            }
        } else {
            BackupAvailabilityDto {
                backup_id: backup.id.as_str().to_string(),
                source_available: true,
                destination_available: true,
                available: true,
                blocked_reason: None,
                reason_text: None,
            }
        }
    }

    fn format_pause_label(
        &self,
        paused_since: OffsetDateTime,
        paused_until: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> String {
        let since_str = format_local_hhmm(paused_since, &self.timezone);
        if let Some(until) = paused_until {
            let until_str = format_local_slot_hint(until, now, &self.timezone);
            i18n("Paused since %1 (until %2)").subs(&[&since_str, &until_str])
        } else {
            i18n("Paused since %1").subs(&[&since_str])
        }
    }

    fn format_unmet_condition_item(&self, item: &UnmetPrecondition) -> String {
        match item {
            UnmetPrecondition::OnBattery => {
                if let Some(pct) = self.system_env.battery_percent {
                    i18n("AC power (battery %1%)").subs(&[&pct.to_string()])
                } else {
                    i18n("AC power").into()
                }
            }
            UnmetPrecondition::NoNetwork => i18n("network connection").into(),
            UnmetPrecondition::MeteredNetwork => i18n("unmetered connection").into(),
            UnmetPrecondition::AllowedNetworkMismatch { allowed } => {
                i18n("allowed network (%1)").subs(&[&allowed.join(", ")])
            }
            UnmetPrecondition::SourceUnavailable => i18n("source available").into(),
            UnmetPrecondition::DestinationUnreachable => i18n("destination reachable").into(),
            UnmetPrecondition::Queued => i18n("another run on this destination").into(),
            UnmetPrecondition::Backoff => i18n("retry cooldown").into(),
        }
    }

    fn format_pending_reason_entry(
        &self,
        item: &UnmetPrecondition,
        backoff_until: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> (&'static str, String) {
        match item {
            UnmetPrecondition::SourceUnavailable => (
                "source_unavailable",
                i18n("Waiting: source unavailable").into(),
            ),
            UnmetPrecondition::DestinationUnreachable => (
                "destination_unreachable",
                i18n("Waiting: destination unreachable").into(),
            ),
            UnmetPrecondition::OnBattery => ("on_battery", i18n("Waiting: AC power").into()),
            UnmetPrecondition::NoNetwork => ("no_network", i18n("Waiting: network").into()),
            UnmetPrecondition::MeteredNetwork => {
                ("metered", i18n("Waiting: unmetered connection").into())
            }
            UnmetPrecondition::AllowedNetworkMismatch { .. } => {
                ("wrong_network", i18n("Waiting: allowed network").into())
            }
            UnmetPrecondition::Queued => (
                "queue",
                i18n("Waiting: another run on this destination").into(),
            ),
            UnmetPrecondition::Backoff => {
                let ts = backoff_until
                    .map(|u| format_local_slot_hint(u, now, &self.timezone))
                    .unwrap_or_else(|| "—".to_string());
                ("backoff", i18n("Waiting: next attempt at %1").subs(&[&ts]))
            }
        }
    }

    /// Computes full state snapshot (`DaemonStateDto`) and synchronized `KesticTrayState`.
    pub fn build_state_and_tray(&self) -> (DaemonStateDto, KesticTrayState) {
        let now = self.clock.now_utc();
        let (restic_status, secret_store_status, blocking) = self.probe_restic_and_secrets();
        let all_runs = self.history.list_runs().unwrap_or_default();
        let nested_map = find_nested_source_warnings(&self.config.backups);
        let global_pause_raw = self.history.get_global_pause().ok().flatten();
        let global_paused = is_pause_active(global_pause_raw.as_ref(), now);

        let mut backup_summaries = Vec::with_capacity(self.config.backups.len());
        let mut dest_password_rejected: HashMap<DestinationId, bool> = HashMap::new();
        let mut paused_backup_window: Option<(OffsetDateTime, Option<OffsetDateTime>)> = None;

        for backup in &self.config.backups {
            let dest_name = self
                .config
                .destination(&backup.destination_id)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| backup.destination_id.as_str().to_string());

            let active_run = self
                .active_runs
                .values()
                .find(|entry| entry.backup_id.as_ref() == Some(&backup.id))
                .map(|entry| entry.run.clone());

            let last_run = all_runs
                .iter()
                .find(|r| {
                    r.backup_id() == Some(&backup.id)
                        && !matches!(r.status, RunStatus::Pending | RunStatus::Running)
                })
                .cloned()
                .or_else(|| {
                    all_runs
                        .iter()
                        .find(|r| r.backup_id() == Some(&backup.id))
                        .cloned()
                });

            let pending_forget = self.history.is_pending_forget(&backup.id).unwrap_or(false);
            let mut has_error = false;
            let mut error_action = None;
            let mut error_summary = None;
            let mut unreadable_files = Vec::new();

            if let Some(run) = &last_run {
                match run.status {
                    RunStatus::Error => {
                        has_error = true;
                        let exit_code = run.exit_code.unwrap_or(1);
                        let raw_detail = run.summary_message.as_deref().unwrap_or("");
                        let pres = classify_exit(exit_code, None, Vec::new(), raw_detail);
                        error_summary = Some(localized_failure_summary(&pres.kind));
                        error_action = pres.action.map(|a| action_code(a).to_string());
                        if matches!(pres.kind, ResticFailureKind::WrongPassword) {
                            dest_password_rejected.insert(backup.destination_id.clone(), true);
                        }
                    }
                    RunStatus::Warning => {
                        if let Some(log_path) = &run.log_path
                            && let Ok(content) = fs::read_to_string(log_path)
                        {
                            for line in content.lines() {
                                if let Ok(ResticStreamEvent::Error(err)) =
                                    kestic_core::restic::parse_json_line(line)
                                    && let Some(item) = err.item
                                {
                                    unreadable_files.push(item);
                                }
                            }
                        }
                        if run.exit_code == Some(3) || !unreadable_files.is_empty() {
                            error_summary = Some(localized_failure_summary(
                                &ResticFailureKind::IncompleteSnapshot {
                                    unreadable_items: Vec::new(),
                                },
                            ));
                            error_action = Some("add_to_excludes".to_string());
                        } else if pending_forget {
                            error_summary = Some(i18n("Backed up, retention not executed").into());
                        } else {
                            error_summary = run.summary_message.clone();
                        }
                    }
                    _ => {}
                }
            }

            let availability = self.availability_for_backup(backup);
            let dest_busy = self
                .active_runs
                .values()
                .any(|e| e.run.destination_id == backup.destination_id);
            let rs = self
                .history
                .get_backup_runtime_state(&backup.id)
                .unwrap_or_default();
            let last_success_at = self
                .history
                .last_successful_backup_at(&backup.id)
                .ok()
                .flatten();

            let eval = evaluate_backup_schedule(
                &BackupScheduleEvalInput {
                    backup,
                    runtime_state: &rs,
                    last_success_at,
                    overdue_anchor_at: None,
                    global_pause: global_pause_raw.as_ref(),
                    global_overdue_threshold: self.config.default_overdue_threshold,
                    source_available: availability.source_available,
                    destination_reachable: availability.destination_available,
                    destination_busy: dest_busy,
                    has_permanent_error: has_error,
                },
                &self.system_env,
                now,
                &self.timezone,
            );

            if eval.is_paused
                && let Some(ps) = eval.paused_since
                && paused_backup_window.is_none()
            {
                paused_backup_window = Some((ps, eval.paused_until));
            }

            let interrupted_recovery = active_run.is_none()
                && !eval.is_paused
                && !has_error
                && eval.due_slot.is_none()
                && last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Interrupted);

            let effective_unmet = if active_run.is_some() || eval.is_paused || has_error {
                Vec::new()
            } else if !eval.unmet_preconditions.is_empty() {
                eval.unmet_preconditions.clone()
            } else if interrupted_recovery {
                collect_pending_preconditions(
                    availability.source_available,
                    availability.destination_available,
                    &backup.run_conditions,
                    &self.system_env,
                    dest_busy,
                    eval.in_backoff,
                )
            } else {
                Vec::new()
            };

            let mut pending_primary_code = None;
            let mut pending_primary_reason = None;
            let mut pending_reasons = Vec::with_capacity(effective_unmet.len());
            for (idx, item) in effective_unmet.iter().enumerate() {
                let (code, text) = self.format_pending_reason_entry(item, rs.backoff_until, now);
                if idx == 0 {
                    pending_primary_code = Some(code.to_string());
                    pending_primary_reason = Some(text.clone());
                }
                pending_reasons.push(text);
            }

            let cond_unmet = evaluate_run_conditions(&backup.run_conditions, &self.system_env);
            let has_unmet_run_condition = !cond_unmet.is_empty();
            let unmet_conditions_text = if has_unmet_run_condition {
                let parts: Vec<String> = cond_unmet
                    .iter()
                    .map(|c| self.format_unmet_condition_item(c))
                    .collect();
                Some(i18n("Condition not met: %1").subs(&[&parts.join(", ")]))
            } else {
                None
            };

            let nested_warnings = nested_map.get(&backup.id).cloned().unwrap_or_default();
            let exclude_lint_warnings = lint_exclude_rules(&backup.excludes);

            backup_summaries.push(BackupSummaryDto {
                backup: backup.clone(),
                destination_name: dest_name,
                last_run,
                active_run,
                availability,
                has_error,
                is_overdue: eval.is_overdue,
                error_action,
                error_summary,
                unreadable_files,
                pending_forget,
                nested_warnings,
                exclude_lint_warnings,
                is_paused: eval.is_paused,
                paused_since_unix: eval.paused_since.map(|t| t.unix_timestamp()),
                paused_until_unix: eval.paused_until.map(|t| t.unix_timestamp()),
                next_scheduled_at_unix: eval.next_scheduled_at.map(|t| t.unix_timestamp()),
                pending_primary_reason,
                pending_primary_code,
                pending_reasons,
                missed_slots_count: eval.missed_slots_for_overdue,
                has_unmet_run_condition,
                unmet_conditions_text,
            });
        }

        let mut dest_summaries = Vec::with_capacity(self.config.destinations.len());
        for dest in &self.config.destinations {
            let backup_count = self
                .config
                .backups
                .iter()
                .filter(|b| b.destination_id == dest.id)
                .count();
            let avail = self.availability_for_destination(dest);
            let password_rejected = dest_password_rejected
                .get(&dest.id)
                .copied()
                .unwrap_or(false);
            let stale_lock_count = self.dest_stale_locks.get(&dest.id).copied().unwrap_or(0);
            let has_stale_lock = stale_lock_count > 0;
            let ds = self
                .history
                .get_destination_maintenance_state(&dest.id)
                .unwrap_or_default();
            let pending_forget_count = self
                .config
                .backups
                .iter()
                .filter(|b| {
                    b.destination_id == dest.id
                        && self.history.is_pending_forget(&b.id).unwrap_or(false)
                })
                .count();
            let dest_busy = self
                .active_runs
                .values()
                .any(|e| e.run.destination_id == dest.id);
            let m_eval = evaluate_destination_maintenance(
                &DestinationMaintenanceEvalInput {
                    destination: dest,
                    state: &ds,
                    has_pending_forget: pending_forget_count > 0,
                    global_pause: global_pause_raw.as_ref(),
                    destination_busy: dest_busy,
                    destination_reachable: avail.reachable,
                },
                &self.system_env,
                now,
                &self.timezone,
            );
            let last_maintenance_run = all_runs
                .iter()
                .find(|r| {
                    r.destination_id == dest.id
                        && matches!(r.kind, kestic_core::model::RunKind::Maintenance)
                })
                .cloned();
            let maintenance_running = self.active_runs.values().any(|e| {
                e.run.destination_id == dest.id
                    && matches!(e.run.kind, kestic_core::model::RunKind::Maintenance)
            });
            let maint_failed = last_maintenance_run
                .as_ref()
                .is_some_and(|r| r.status == RunStatus::Error);
            let has_problem =
                !avail.reachable || password_rejected || has_stale_lock || maint_failed;

            dest_summaries.push(DestinationSummaryDto {
                destination: dest.clone(),
                backup_count,
                availability: avail,
                password_rejected,
                has_stale_lock,
                has_problem,
                next_prune_at_unix: m_eval.next_prune_at.map(|t| t.unix_timestamp()),
                next_check_at_unix: m_eval.next_check_at.map(|t| t.unix_timestamp()),
                last_prune_at_unix: ds.last_prune_at.map(|t| t.unix_timestamp()),
                last_check_at_unix: ds.last_check_at.map(|t| t.unix_timestamp()),
                last_maintenance_run,
                maintenance_running,
                pending_forget_count,
                stale_lock_count,
            });
        }

        let global_pause_active = if global_paused {
            global_pause_raw.as_ref()
        } else {
            None
        };
        let overall = self.compute_overall_status(
            &backup_summaries,
            &dest_summaries,
            blocking.as_ref(),
            global_pause_active,
            paused_backup_window,
            now,
        );

        let tray_backups = backup_summaries
            .iter()
            .map(|b| {
                let is_running_dest = self
                    .active_runs
                    .values()
                    .any(|r| r.run.destination_id == b.backup.destination_id);
                let label = if !b.availability.available {
                    let reason = b
                        .availability
                        .reason_text
                        .as_deref()
                        .unwrap_or("unavailable");
                    format!("{} ({reason})", b.backup.name)
                } else {
                    b.backup.name.clone()
                };
                TrayBackupItem {
                    backup_id: b.backup.id.as_str().to_string(),
                    label,
                    enabled: b.availability.available && !is_running_dest,
                }
            })
            .collect();

        let tray_status = match overall.tray_status.as_str() {
            "NeedsAttention" => TrayStatus::NeedsAttention,
            "Active" => TrayStatus::Active,
            _ => TrayStatus::Passive,
        };
        let tray_paused_line = overall
            .paused_label
            .clone()
            .or_else(|| paused_backup_window.map(|(ps, pu)| self.format_pause_label(ps, pu, now)));
        let tray_next_backup_line = overall
            .next_scheduled_backup_name
            .as_ref()
            .zip(overall.next_scheduled_at_unix)
            .and_then(|(name, ts)| {
                OffsetDateTime::from_unix_timestamp(ts).ok().map(|dt| {
                    let when = format_local_slot_hint(dt, now, &self.timezone);
                    i18n("Next backup: %1 (%2)").subs(&[name, &when])
                })
            });

        let tray_state = KesticTrayState {
            status: tray_status,
            icon_name: "kestic-symbolic".to_string(),
            icon_theme_path: self.icon_theme_path.clone(),
            overlay_icon_name: overall.overlay_icon.clone(),
            tooltip_title: "Kestic".to_string(),
            tooltip_description: overall.headline.clone(),
            status_line: overall.headline.clone(),
            running_run: overall.active_run_progress.clone(),
            paused_line: tray_paused_line,
            next_backup_line: tray_next_backup_line,
            global_paused,
            backups: tray_backups,
            blocking_active: blocking.is_some(),
        };

        let recent_runs = all_runs.into_iter().take(200).collect();
        let state = DaemonStateDto {
            daemon_version: crate::daemon_version().to_string(),
            has_config_file: self.has_config_file,
            config_load_error: self.config_load_error.clone(),
            config: self.config.clone(),
            restic_status,
            secret_store_status,
            blocking,
            overall,
            backups: backup_summaries,
            destinations: dest_summaries,
            recent_runs,
        };

        (state, tray_state)
    }

    fn compute_overall_status(
        &self,
        backups: &[BackupSummaryDto],
        destinations: &[DestinationSummaryDto],
        blocking: Option<&BlockingStateDto>,
        global_pause: Option<&PauseState>,
        paused_backup_window: Option<(OffsetDateTime, Option<OffsetDateTime>)>,
        now: OffsetDateTime,
    ) -> OverallStatusDto {
        let active_run_progress = self.active_runs.values().next().map(|e| e.progress.clone());

        if let Some(sim) = self.simulated_status {
            return self.compute_simulated_overall_status(sim, active_run_progress);
        }

        let failed_count = backups.iter().filter(|b| b.has_error).count();
        let overdue_count = backups.iter().filter(|b| b.is_overdue).count();
        let warning_count = backups
            .iter()
            .filter(|b| {
                b.last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Warning)
            })
            .count();
        let running_count = self.active_runs.len();
        let waiting_count = backups
            .iter()
            .filter(|b| {
                b.pending_primary_reason.is_some()
                    || (b.active_run.is_none()
                        && !b.is_paused
                        && !b.has_error
                        && b.last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Pending))
            })
            .count();
        let global_paused = global_pause.is_some();
        let any_paused = global_paused || backups.iter().any(|b| b.is_paused);
        let all_paused = !backups.is_empty() && backups.iter().all(|b| b.is_paused);

        let has_interrupted = backups.iter().any(|b| {
            b.last_run
                .as_ref()
                .is_some_and(|r| r.status == RunStatus::Interrupted)
        });
        let has_cancelled = backups.iter().any(|b| {
            b.last_run
                .as_ref()
                .is_some_and(|r| r.status == RunStatus::Cancelled)
        });
        let has_never_run = backups
            .iter()
            .any(|b| b.active_run.is_none() && b.last_run.is_none());
        let all_manual = !backups.is_empty()
            && backups
                .iter()
                .all(|b| matches!(b.backup.schedule, Schedule::ManualOnly));

        let worst_kind = if blocking.is_some() {
            OverallStatusKind::Blocked
        } else if failed_count > 0 {
            OverallStatusKind::Error
        } else if overdue_count > 0 {
            OverallStatusKind::Overdue
        } else if warning_count > 0 {
            OverallStatusKind::Warning
        } else if running_count > 0 {
            OverallStatusKind::Running
        } else if waiting_count > 0 {
            OverallStatusKind::Pending
        } else if global_paused || all_paused {
            OverallStatusKind::Paused
        } else if has_interrupted {
            OverallStatusKind::Interrupted
        } else if has_cancelled {
            OverallStatusKind::Cancelled
        } else if has_never_run || backups.is_empty() {
            OverallStatusKind::NeverRun
        } else if all_manual {
            OverallStatusKind::ManualOnly
        } else {
            OverallStatusKind::Ok
        };

        let (tray_status, overlay) =
            compute_tray_presentation(worst_kind, running_count > 0, any_paused);

        let paused_since_unix = global_pause
            .map(|p| p.paused_since.unix_timestamp())
            .or_else(|| paused_backup_window.map(|(ps, _)| ps.unix_timestamp()));
        let paused_until_unix = global_pause
            .and_then(|p| p.paused_until.map(|u| u.unix_timestamp()))
            .or_else(|| paused_backup_window.and_then(|(_, pu)| pu.map(|u| u.unix_timestamp())));
        let paused_label = global_pause
            .map(|p| self.format_pause_label(p.paused_since, p.paused_until, now))
            .or_else(|| paused_backup_window.map(|(ps, pu)| self.format_pause_label(ps, pu, now)));

        let earliest_next = backups
            .iter()
            .filter(|b| !b.is_paused)
            .filter_map(|b| {
                b.next_scheduled_at_unix
                    .map(|ts| (ts, b.backup.name.clone()))
            })
            .min_by_key(|(ts, _)| *ts);
        let (next_scheduled_at_unix, next_scheduled_backup_name) = match earliest_next {
            Some((ts, name)) => (Some(ts), Some(name)),
            None => (None, None),
        };

        let headline = if let Some(b) = blocking {
            b.message.clone()
        } else if failed_count > 0 {
            if failed_count == 1 {
                i18n("1 backup failed").into()
            } else {
                i18n("%1 backups failed").subs(&[&failed_count.to_string()])
            }
        } else if overdue_count > 0 {
            if overdue_count == 1 {
                i18n("1 backup overdue").into()
            } else {
                i18n("%1 backups overdue").subs(&[&overdue_count.to_string()])
            }
        } else if warning_count > 0 {
            if warning_count == 1 {
                i18n("1 backup finished with warning").into()
            } else {
                i18n("%1 backups finished with warnings").subs(&[&warning_count.to_string()])
            }
        } else if let Some(active) = &active_run_progress {
            let pct = active.percent_done.round().clamp(0.0, 100.0) as u32;
            i18n("%1: backing up · %2%").subs(&[&active.backup_name, &pct.to_string()])
        } else if waiting_count > 0 {
            if global_paused {
                if waiting_count == 1 {
                    i18n("Schedules paused · 1 backup waiting").into()
                } else {
                    i18n("Schedules paused · %1 backups waiting")
                        .subs(&[&waiting_count.to_string()])
                }
            } else if waiting_count == 1 {
                i18n("1 backup waiting").into()
            } else {
                i18n("%1 of %2 backups waiting")
                    .subs(&[&waiting_count.to_string(), &backups.len().to_string()])
            }
        } else if global_paused || all_paused {
            i18n("Schedules paused").into()
        } else if backups.is_empty() {
            i18n("No backups configured yet").into()
        } else if has_interrupted {
            i18n("Last run interrupted").into()
        } else if has_cancelled {
            i18n("Last run cancelled").into()
        } else if has_never_run {
            i18n("Not backed up yet").into()
        } else {
            i18n("All data backed up").into()
        };

        let subtitle = if let Some(active) = &active_run_progress {
            if failed_count > 0 || overdue_count > 0 {
                let pct = active.percent_done.round().clamp(0.0, 100.0) as u32;
                i18n("Currently running: %1 (%2%)").subs(&[&active.backup_name, &pct.to_string()])
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let backups_have_attention = failed_count > 0 || overdue_count > 0;
        let destinations_have_attention = destinations.iter().any(|d| d.has_problem);

        OverallStatusDto {
            kind: worst_kind,
            tray_status: tray_status.as_str().to_string(),
            overlay_icon: overlay.to_string(),
            headline,
            subtitle,
            failed_count,
            warning_count,
            overdue_count,
            running_count,
            waiting_count,
            paused: global_paused,
            paused_since_unix,
            paused_until_unix,
            paused_label,
            next_scheduled_backup_name,
            next_scheduled_at_unix,
            active_run_progress,
            backups_have_attention,
            destinations_have_attention,
        }
    }

    fn compute_simulated_overall_status(
        &self,
        sim: SimulatedOverallStatus,
        active_run_progress: Option<ActiveRunProgressDto>,
    ) -> OverallStatusDto {
        let (kind, is_running, is_paused, headline) = match sim {
            SimulatedOverallStatus::Overdue => (
                OverallStatusKind::Overdue,
                false,
                false,
                i18n("1 backup overdue").into(),
            ),
            SimulatedOverallStatus::Paused => (
                OverallStatusKind::Paused,
                false,
                true,
                i18n("Schedules paused").into(),
            ),
            SimulatedOverallStatus::Waiting => (
                OverallStatusKind::Pending,
                false,
                false,
                i18n("1 backup waiting").into(),
            ),
            SimulatedOverallStatus::PausedAndWaiting => (
                OverallStatusKind::Pending,
                false,
                true,
                i18n("Schedules paused · 1 backup waiting").into(),
            ),
            SimulatedOverallStatus::MaintenanceRunning => (
                OverallStatusKind::Running,
                true,
                false,
                i18n("Maintenance running").into(),
            ),
            SimulatedOverallStatus::RestoreRunning => (
                OverallStatusKind::Running,
                true,
                false,
                i18n("Restore running").into(),
            ),
        };
        let (tray_status, overlay) = compute_tray_presentation(kind, is_running, is_paused);
        OverallStatusDto {
            kind,
            tray_status: tray_status.as_str().to_string(),
            overlay_icon: overlay.to_string(),
            headline,
            subtitle: String::new(),
            failed_count: 0,
            warning_count: 0,
            overdue_count: usize::from(matches!(sim, SimulatedOverallStatus::Overdue)),
            running_count: usize::from(is_running),
            waiting_count: usize::from(matches!(
                sim,
                SimulatedOverallStatus::Waiting | SimulatedOverallStatus::PausedAndWaiting
            )),
            paused: is_paused,
            paused_since_unix: None,
            paused_until_unix: None,
            paused_label: is_paused.then(|| i18n("Schedules paused").into()),
            next_scheduled_backup_name: None,
            next_scheduled_at_unix: None,
            active_run_progress,
            backups_have_attention: matches!(sim, SimulatedOverallStatus::Overdue),
            destinations_have_attention: false,
        }
    }

    fn persist_config(&mut self, next: AppConfig) -> Result<(), KesticServiceErr> {
        if let Some(ref load_err) = self.config_load_error {
            return Err(KesticServiceErr::InvalidInput(format!(
                "refusing to overwrite incompatible config.toml: {load_err}"
            )));
        }
        save_config(&self.config_path, &next).map_err(map_config_err)?;
        self.config = next;
        self.has_config_file = true;
        self.refresh_availability_if_needed(true);
        Ok(())
    }

    /// `WindowActive(active, page)` (`U17`, `AC-M1-23-3`).
    pub fn set_window_active(&mut self, active: bool, page: &str) {
        self.window_active = active;
        self.window_page = page.to_string();
        if active {
            self.refresh_availability_if_needed(false);
        }
    }

    pub fn add_destination(
        &mut self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> Result<String, KesticServiceErr> {
        let mut dest: Destination = serde_json::from_str(dest_json).map_err(|e| {
            KesticServiceErr::InvalidInput(format!("invalid destination JSON: {e}"))
        })?;
        if self.config.destination(&dest.id).is_some() {
            dest.id =
                DestinationId::generate_unique(self.config.destinations.iter().map(|d| &d.id))
                    .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
            dest.secrets = kestic_core::model::DestinationSecretRefs::for_destination(
                &dest.id,
                dest.kind.uses_rest_credentials(),
            );
        }
        dest.validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        if let Some(existing) = self.config.find_duplicate_destination(&dest.kind, None) {
            return Err(KesticServiceErr::InvalidInput(format!(
                "DUPLICATE_LOCATION:{}",
                existing.name
            )));
        }

        let setup_mode = parse_setup_mode(mode)?;
        if repo_password.is_empty() {
            return Err(KesticServiceErr::InvalidInput(
                "repository password must not be empty".to_string(),
            ));
        }

        let resolved = resolve_and_check(&self.config.restic_provider).map_err(map_provider_err)?;
        let repo_pw = SecretValue::from_str_secret(repo_password);
        let rest_pw =
            (!rest_password.is_empty()).then(|| SecretValue::from_str_secret(rest_password));

        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &self.kestic_binary,
            secret_store: self.secret_store.as_ref(),
            clock: self.clock.as_ref(),
        };
        setup_destination(&dest, setup_mode, &repo_pw, rest_pw.as_ref(), &runtime)
            .map_err(map_destination_err)?;

        let new_id = dest.id.as_str().to_string();
        let mut next = self.config.clone();
        next.destinations.push(dest);
        self.persist_config(next)?;
        self.ensure_baselines();
        Ok(new_id)
    }

    pub fn test_destination_connection(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> ConnectionTestResultDto {
        let dest: Destination = match serde_json::from_str(dest_json) {
            Ok(d) => d,
            Err(e) => {
                return ConnectionTestResultDto {
                    ok: false,
                    error_kind: Some("invalid_input".to_string()),
                    message: e.to_string(),
                };
            }
        };
        if let Err(e) = dest.validate() {
            return ConnectionTestResultDto {
                ok: false,
                error_kind: Some("invalid_input".to_string()),
                message: e.to_string(),
            };
        }

        let is_existing = self.config.destination(&dest.id).is_some();
        let exclude_id = is_existing.then_some(&dest.id);
        if let Some(existing) = self
            .config
            .find_duplicate_destination(&dest.kind, exclude_id)
        {
            return ConnectionTestResultDto {
                ok: false,
                error_kind: Some("duplicate_location".to_string()),
                message: i18n("This location is already used by destination '%1'.")
                    .subs(&[&existing.name]),
            };
        }

        if let Err(err) = preflight_check_destination(&dest, DEFAULT_CONNECT_TIMEOUT) {
            return ConnectionTestResultDto {
                ok: false,
                error_kind: Some("destination_unreachable".to_string()),
                message: err.to_string(),
            };
        }

        let setup_mode = match parse_setup_mode(mode) {
            Ok(m) => m,
            Err(e) => {
                return ConnectionTestResultDto {
                    ok: false,
                    error_kind: Some("invalid_input".to_string()),
                    message: e.to_string(),
                };
            }
        };

        if matches!(setup_mode, DestinationSetupMode::InitNew) {
            if let DestinationKind::Local { path } = &dest.kind {
                if path.join("config").exists() {
                    return ConnectionTestResultDto {
                        ok: false,
                        error_kind: Some("repository_exists".to_string()),
                        message: i18n("This folder already contains a restic repository. Choose 'Connect existing repository' instead.").into(),
                    };
                }
                let nonce = RunId::generate()
                    .map(|r| r.as_str().to_string())
                    .unwrap_or_else(|_| "0".to_string());
                let probe_path = path.join(format!(".kestic-write-probe-{nonce}"));
                match fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&probe_path)
                {
                    Ok(f) => {
                        drop(f);
                        let _ = fs::remove_file(&probe_path);
                    }
                    Err(err) => {
                        return ConnectionTestResultDto {
                            ok: false,
                            error_kind: Some("not_writable".to_string()),
                            message: i18n("Folder is not writable: %1").subs(&[&err.to_string()]),
                        };
                    }
                }
            }
            if is_existing && repo_password.is_empty() {
                match self
                    .secret_store
                    .get_secret(dest.id.as_str(), SECRET_KIND_REPO)
                {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        return ConnectionTestResultDto {
                            ok: false,
                            error_kind: Some("invalid_input".to_string()),
                            message: i18n("No repository password provided or stored.").into(),
                        };
                    }
                    Err(SecretError::LockedOrPromptRequired(msg)) => {
                        return ConnectionTestResultDto {
                            ok: false,
                            error_kind: Some("secret_store_locked".to_string()),
                            message: msg,
                        };
                    }
                    Err(err) => {
                        return ConnectionTestResultDto {
                            ok: false,
                            error_kind: Some("internal".to_string()),
                            message: err.to_string(),
                        };
                    }
                }
            }
            return ConnectionTestResultDto {
                ok: true,
                error_kind: None,
                message: i18n("Reachable").into(),
            };
        }

        // `ConnectExisting`: probe `cat --no-lock config` using explicit password or (for existing
        // destinations) stored SecretStore credentials without mutating stored secrets.
        let effective_repo_pw = if !repo_password.is_empty() {
            Some(SecretValue::from_str_secret(repo_password))
        } else if is_existing {
            match self
                .secret_store
                .get_secret(dest.id.as_str(), SECRET_KIND_REPO)
            {
                Ok(Some(pw)) => Some(pw),
                Ok(None) => {
                    return ConnectionTestResultDto {
                        ok: false,
                        error_kind: Some("invalid_input".to_string()),
                        message: i18n("No repository password provided or stored.").into(),
                    };
                }
                Err(SecretError::LockedOrPromptRequired(msg)) => {
                    return ConnectionTestResultDto {
                        ok: false,
                        error_kind: Some("secret_store_locked".to_string()),
                        message: msg,
                    };
                }
                Err(err) => {
                    return ConnectionTestResultDto {
                        ok: false,
                        error_kind: Some("internal".to_string()),
                        message: err.to_string(),
                    };
                }
            }
        } else {
            None
        };

        if let Some(repo_pw) = effective_repo_pw {
            let effective_rest_pw = if !rest_password.is_empty() {
                Some(SecretValue::from_str_secret(rest_password))
            } else if is_existing && dest.kind.uses_rest_credentials() {
                match self
                    .secret_store
                    .get_secret(dest.id.as_str(), SECRET_KIND_REST)
                {
                    Ok(pw) => pw,
                    Err(SecretError::LockedOrPromptRequired(msg)) => {
                        return ConnectionTestResultDto {
                            ok: false,
                            error_kind: Some("secret_store_locked".to_string()),
                            message: msg,
                        };
                    }
                    Err(err) => {
                        return ConnectionTestResultDto {
                            ok: false,
                            error_kind: Some("internal".to_string()),
                            message: err.to_string(),
                        };
                    }
                }
            } else {
                None
            };

            let resolved = match resolve_and_check(&self.config.restic_provider) {
                Ok(r) => r,
                Err(e) => {
                    return ConnectionTestResultDto {
                        ok: false,
                        error_kind: Some("restic_unavailable".to_string()),
                        message: e.to_string(),
                    };
                }
            };
            let temp_store = kestic_core::secrets::InMemorySecretStore::new();
            let runtime = ResticRuntime {
                resolved: &resolved,
                kestic_binary: &self.kestic_binary,
                secret_store: &temp_store,
                clock: self.clock.as_ref(),
            };
            if let Err(err) = setup_destination(
                &dest,
                DestinationSetupMode::ConnectExisting,
                &repo_pw,
                effective_rest_pw.as_ref(),
                &runtime,
            ) {
                let kind = match err {
                    DestinationError::WrongPassword => "wrong_password",
                    DestinationError::AuthFailed(_) => "auth_failed",
                    DestinationError::RepositoryMissing(_) => "repository_missing",
                    DestinationError::Unreachable(_) => "destination_unreachable",
                    _ => "command_failed",
                };
                return ConnectionTestResultDto {
                    ok: false,
                    error_kind: Some(kind.to_string()),
                    message: err.to_string(),
                };
            }
        }

        ConnectionTestResultDto {
            ok: true,
            error_kind: None,
            message: i18n("Reachable").into(),
        }
    }

    pub fn update_destination(
        &mut self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
        verify_connection: bool,
    ) -> Result<(), KesticServiceErr> {
        let updated: Destination = serde_json::from_str(dest_json).map_err(|e| {
            KesticServiceErr::InvalidInput(format!("invalid destination JSON: {e}"))
        })?;
        updated
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

        let existing_idx = self
            .config
            .destinations
            .iter()
            .position(|d| d.id == updated.id)
            .ok_or_else(|| KesticServiceErr::NotFound(updated.id.as_str().to_string()))?;

        let setup_mode = parse_setup_mode(mode)?;

        if let Some(existing) = self
            .config
            .find_duplicate_destination(&updated.kind, Some(&updated.id))
        {
            return Err(KesticServiceErr::InvalidInput(format!(
                "DUPLICATE_LOCATION:{}",
                existing.name
            )));
        }

        if verify_connection || matches!(setup_mode, DestinationSetupMode::InitNew) {
            let resolved =
                resolve_and_check(&self.config.restic_provider).map_err(map_provider_err)?;
            let effective_repo_pw = if !repo_password.is_empty() {
                SecretValue::from_str_secret(repo_password)
            } else {
                self.secret_store
                    .get_secret(updated.id.as_str(), SECRET_KIND_REPO)
                    .map_err(map_secret_err)?
                    .ok_or_else(|| {
                        KesticServiceErr::InvalidInput(
                            "missing stored repository password".to_string(),
                        )
                    })?
            };

            let effective_rest_pw = if !rest_password.is_empty() {
                Some(SecretValue::from_str_secret(rest_password))
            } else if updated.kind.uses_rest_credentials() {
                self.secret_store
                    .get_secret(updated.id.as_str(), SECRET_KIND_REST)
                    .map_err(map_secret_err)?
            } else {
                None
            };

            // AC-M1-19-6: preflight + setup (`init` or `connect`) must pass before mutating secrets or config.
            let runtime = ResticRuntime {
                resolved: &resolved,
                kestic_binary: &self.kestic_binary,
                secret_store: self.secret_store.as_ref(),
                clock: self.clock.as_ref(),
            };
            setup_destination(
                &updated,
                setup_mode,
                &effective_repo_pw,
                effective_rest_pw.as_ref(),
                &runtime,
            )
            .map_err(map_destination_err)?;
        }

        let mut next = self.config.clone();
        next.destinations[existing_idx] = updated;
        self.persist_config(next)?;
        Ok(())
    }

    pub fn update_destination_password(
        &mut self,
        dest_id: &str,
        new_password: &str,
    ) -> Result<(), KesticServiceErr> {
        let id = DestinationId::parse(dest_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let dest = self
            .config
            .destination(&id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(dest_id.to_string()))?;
        if new_password.is_empty() {
            return Err(KesticServiceErr::InvalidInput(
                "password must not be empty".to_string(),
            ));
        }

        let resolved = resolve_and_check(&self.config.restic_provider).map_err(map_provider_err)?;
        let candidate = SecretValue::from_str_secret(new_password);
        verify_and_update_stored_password(
            &dest,
            &candidate,
            self.secret_store.as_ref(),
            &resolved,
            &self.kestic_binary,
            self.clock.as_ref(),
        )
        .map_err(map_destination_err)?;
        Ok(())
    }

    pub fn remove_destination(&mut self, dest_id: &str) -> Result<(), KesticServiceErr> {
        let id = DestinationId::parse(dest_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let mut next = self.config.clone();
        let removed = next.remove_destination(&id).map_err(|err| match err {
            ConfigError::Validation(msg) if msg.contains("still referenced by") => {
                KesticServiceErr::InUse(msg)
            }
            ConfigError::Validation(msg) if msg.contains("not found") => {
                KesticServiceErr::NotFound(msg)
            }
            other => map_config_err(other),
        })?;

        self.persist_config(next)?;
        remove_destination_secrets(&removed, self.secret_store.as_ref()).map_err(map_secret_err)?;
        self.dest_availability.remove(&id);
        self.dest_stale_locks.remove(&id);
        let _ = self.history.clear_destination_state(&id);
        Ok(())
    }

    pub fn add_backup(&mut self, backup_json: &str) -> Result<String, KesticServiceErr> {
        let mut backup: Backup = serde_json::from_str(backup_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid backup JSON: {e}")))?;
        if self.config.backup(&backup.id).is_some() {
            backup.id = BackupId::generate_unique(self.config.backups.iter().map(|b| &b.id))
                .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        }
        let home = current_home_dir();
        backup.excludes.normalize_folders(home.as_deref());

        let detected = detect_source_with(
            &backup.source.path,
            home.as_deref(),
            self.mount_inspector.as_ref(),
        );
        if backup.source.kind == SourceKind::Folder && detected.kind == SourceKind::Drive {
            backup.source = detected;
        } else if backup.source.kind == SourceKind::Drive
            && backup.source.identity.is_none()
            && detected.kind == SourceKind::Drive
        {
            backup.source.identity = detected.identity;
            if backup.source.drive_label.is_none() {
                backup.source.drive_label = detected.drive_label;
            }
        }

        backup
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

        let new_id = backup.id.as_str().to_string();
        let backup_for_file = backup.clone();
        let mut next = self.config.clone();
        next.backups.push(backup);
        self.persist_config(next)?;
        self.ensure_baselines();
        write_exclude_file(&self.state_dir, &backup_for_file)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        Ok(new_id)
    }

    pub fn update_backup(&mut self, backup_json: &str) -> Result<(), KesticServiceErr> {
        let mut updated: Backup = serde_json::from_str(backup_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid backup JSON: {e}")))?;
        let home = current_home_dir();
        updated.excludes.normalize_folders(home.as_deref());

        let idx = self
            .config
            .backups
            .iter()
            .position(|b| b.id == updated.id)
            .ok_or_else(|| KesticServiceErr::NotFound(updated.id.as_str().to_string()))?;

        let existing = &self.config.backups[idx];
        let old_schedule = existing.schedule.clone();
        let detected = detect_source_with(
            &updated.source.path,
            home.as_deref(),
            self.mount_inspector.as_ref(),
        );
        if updated.source.kind == SourceKind::Folder && detected.kind == SourceKind::Drive {
            updated.source = detected;
        } else if updated.source.kind == SourceKind::Drive
            && (updated.source.identity.is_none() || updated.source.path != existing.source.path)
            && detected.kind == SourceKind::Drive
        {
            updated.source.identity = detected.identity;
            if updated.source.drive_label.is_none() || updated.source.path != existing.source.path {
                updated.source.drive_label = detected.drive_label;
            }
        }

        updated
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

        let updated_for_file = updated.clone();
        let updated_schedule = updated.schedule.clone();
        let mut next = self.config.clone();
        next.backups[idx] = updated;
        self.persist_config(next)?;
        write_exclude_file(&self.state_dir, &updated_for_file)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;

        if old_schedule != updated_schedule {
            let now = self.clock.now_utc();
            let _ = self
                .history
                .delete_pending_runs_for_backup(&updated_for_file.id);
            if let Ok(mut rs) = self.history.get_backup_runtime_state(&updated_for_file.id) {
                if matches!(updated_schedule, Schedule::ManualOnly) {
                    rs.backoff_attempt = 0;
                    rs.backoff_until = None;
                    rs.last_failure_at = None;
                    rs.last_overdue_notified_at = None;
                } else if matches!(old_schedule, Schedule::ManualOnly) {
                    rs.baseline_at = Some(now);
                    rs.overdue_anchor_at = Some(now);
                    rs.last_overdue_notified_at = None;
                }
                let _ = self
                    .history
                    .upsert_backup_runtime_state(&updated_for_file.id, &rs);
            }
        }
        Ok(())
    }

    pub fn rename_backup(
        &mut self,
        backup_id: &str,
        new_name: &str,
    ) -> Result<(), KesticServiceErr> {
        let id = BackupId::parse(backup_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let mut next = self.config.clone();
        let backup = next
            .backups
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        backup
            .rename(new_name)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        self.persist_config(next)?;
        Ok(())
    }

    pub fn reassign_backup(
        &mut self,
        backup_id: &str,
        new_dest_id: &str,
    ) -> Result<(), KesticServiceErr> {
        let b_id = BackupId::parse(backup_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let d_id = DestinationId::parse(new_dest_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let mut next = self.config.clone();
        next.reassign_backup(&b_id, &d_id)
            .map_err(|err| match err {
                ConfigError::Validation(msg) if msg.contains("not found") => {
                    KesticServiceErr::NotFound(msg)
                }
                other => map_config_err(other),
            })?;
        self.persist_config(next)?;
        Ok(())
    }

    pub fn remove_backup(&mut self, backup_id: &str) -> Result<(), KesticServiceErr> {
        let id = BackupId::parse(backup_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let mut next = self.config.clone();
        next.remove_backup(&id).map_err(|err| match err {
            ConfigError::Validation(msg) if msg.contains("not found") => {
                KesticServiceErr::NotFound(msg)
            }
            other => map_config_err(other),
        })?;
        self.persist_config(next)?;
        self.source_availability.remove(&id);
        let _ = fs::remove_file(exclude_file_path(&self.state_dir, &id));
        if let Err(err) = self.history.clear_backup_state(&id) {
            eprintln!("kestic-daemon: failed to clear history state for backup {id}: {err}");
        }
        Ok(())
    }

    pub fn set_restic_provider(
        &mut self,
        provider_json: &str,
    ) -> Result<ResticStatusDto, KesticServiceErr> {
        let provider: kestic_core::model::ResticProvider = serde_json::from_str(provider_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid provider JSON: {e}")))?;
        let _ = resolve_and_check(&provider).map_err(map_provider_err)?;
        let mut next = self.config.clone();
        next.restic_provider = provider;
        self.persist_config(next)?;
        let (restic_status, _, _) = self.probe_restic_and_secrets();
        Ok(restic_status)
    }

    pub fn set_expert_mode(&mut self, enabled: bool) -> Result<(), KesticServiceErr> {
        let mut next = self.config.clone();
        next.expert_mode = enabled;
        self.persist_config(next)
    }

    pub fn get_run_detail(&self, run_id: &str) -> Result<RunDetailDto, KesticServiceErr> {
        let id = RunId::parse(run_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let run = if let Some(active) = self.active_runs.get(&id) {
            active.run.clone()
        } else {
            self.history
                .get_run(&id)
                .map_err(|e| KesticServiceErr::Internal(e.to_string()))?
                .ok_or_else(|| KesticServiceErr::NotFound(run_id.to_string()))?
        };

        let backup_name = run
            .backup_id()
            .and_then(|bid| self.config.backup(bid).map(|b| b.name.clone()));
        let destination_name = self
            .config
            .destination(&run.destination_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| run.destination_id.as_str().to_string());

        let log_lines = match &run.log_path {
            Some(path) => fs::read_to_string(path)
                .map(|s| s.lines().map(ToOwned::to_owned).collect())
                .unwrap_or_default(),
            None => Vec::new(),
        };

        let transparent_commands = if !run.commands.is_empty() {
            run.commands
                .iter()
                .map(|c| {
                    if let Some(dest) = self.config.destination(&run.destination_id) {
                        let sub_args = strip_prefix_flags(&c.args);
                        let mut spec = ResticCommandSpec::for_destination(
                            &c.executable,
                            &self.kestic_binary,
                            dest,
                            sub_args,
                        );
                        spec.low_priority = true;
                        spec.to_transparent_shell_command()
                    } else {
                        format!("{} {}", c.executable, c.args.join(" "))
                    }
                })
                .collect()
        } else if let (Some(bid), Some(dest)) = (
            run.backup_id(),
            self.config.destination(&run.destination_id),
        ) {
            if let (Some(backup), Ok(resolved)) = (
                self.config.backup(bid),
                resolve_and_check(&self.config.restic_provider),
            ) {
                kestic_core::backup::build_backup_command_spec(
                    backup,
                    dest,
                    &self.state_dir,
                    &resolved,
                    &self.kestic_binary,
                )
                .map(|s| vec![s.to_transparent_shell_command()])
                .unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let mut error_summary = None;
        let mut error_action = None;
        let mut error_raw_detail = run.summary_message.clone();
        let mut unreadable_files = Vec::new();

        if matches!(run.status, RunStatus::Error | RunStatus::Warning) {
            for line in &log_lines {
                if let Ok(ResticStreamEvent::Error(err)) =
                    kestic_core::restic::parse_json_line(line)
                    && let Some(item) = err.item
                {
                    unreadable_files.push(item);
                }
            }
            if run.status == RunStatus::Error
                || run.exit_code == Some(3)
                || !unreadable_files.is_empty()
            {
                let pres = classify_exit(
                    run.exit_code.unwrap_or(1),
                    run.summary_message.as_deref(),
                    unreadable_files.clone(),
                    "",
                );
                error_summary = Some(localized_failure_summary(&pres.kind));
                error_action = pres.action.map(|a| action_code(a).to_string());
                if error_raw_detail.is_none() {
                    error_raw_detail = pres.raw_detail;
                }
            } else {
                error_summary = run
                    .summary_message
                    .clone()
                    .or_else(|| Some(i18n("Backed up, retention not executed").into()));
            }
        }

        Ok(RunDetailDto {
            run,
            backup_name,
            destination_name,
            log_lines,
            transparent_commands,
            error_summary,
            error_action,
            error_raw_detail,
            unreadable_files,
        })
    }

    pub fn list_backup_snapshots(
        &self,
        backup_id: &str,
    ) -> Result<BackupSnapshotsDto, KesticServiceErr> {
        let b_id = BackupId::parse(backup_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let backup = self
            .config
            .backup(&b_id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let current_dest = self
            .config
            .destination(&backup.destination_id)
            .ok_or_else(|| {
                KesticServiceErr::NotFound(backup.destination_id.as_str().to_string())
            })?;

        let resolved = resolve_and_check(&self.config.restic_provider).map_err(map_provider_err)?;

        let mut current_error = None;
        let current_snapshots = match list_snapshots_for_backup(
            &b_id,
            current_dest,
            &resolved,
            &self.kestic_binary,
            self.secret_store.as_ref(),
            self.clock.as_ref(),
            &[],
        ) {
            Ok(snaps) => snaps.into_iter().map(snapshot_to_dto).collect(),
            Err(err) => {
                current_error = Some(err.to_string());
                Vec::new()
            }
        };

        // AC-M1-22-4: also scan any other configured destinations for earlier `kestic:<backup-id>` snapshots.
        let mut previous_destinations = Vec::new();
        for other_dest in &self.config.destinations {
            if other_dest.id == current_dest.id {
                continue;
            }
            if let Ok(snaps) = list_snapshots_for_backup(
                &b_id,
                other_dest,
                &resolved,
                &self.kestic_binary,
                self.secret_store.as_ref(),
                self.clock.as_ref(),
                &[],
            ) && !snaps.is_empty()
            {
                previous_destinations.push(PreviousDestinationSnapshotsDto {
                    destination_id: other_dest.id.as_str().to_string(),
                    destination_name: other_dest.name.clone(),
                    snapshots: snaps.into_iter().map(snapshot_to_dto).collect(),
                });
            }
        }

        Ok(BackupSnapshotsDto {
            backup_id: backup_id.to_string(),
            current_destination_id: current_dest.id.as_str().to_string(),
            current_destination_name: current_dest.name.clone(),
            current_snapshots,
            current_error,
            previous_destinations,
        })
    }

    pub fn cancel_run(&self, run_id: &str) -> Result<(), KesticServiceErr> {
        let id = RunId::parse(run_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let entry = self
            .active_runs
            .get(&id)
            .ok_or_else(|| KesticServiceErr::NotFound(run_id.to_string()))?;
        entry.cancel.cancel();
        Ok(())
    }

    pub fn cancel_any_active_run(&self) -> bool {
        let mut any = false;
        for entry in self.active_runs.values() {
            entry.cancel.cancel();
            any = true;
        }
        any
    }

    pub fn request_shutdown(&self) {
        self.cancel_any_active_run();
        self.shutdown_flag.store(true, Ordering::SeqCst);
        if let Some(tx) = &self.quit_tx {
            let _ = tx.send(());
        }
    }

    pub fn detect_source(&self, path: &str) -> Result<Source, KesticServiceErr> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(KesticServiceErr::InvalidInput(
                "source path must not be empty".to_string(),
            ));
        }
        let home = current_home_dir();
        Ok(detect_source_with(
            std::path::Path::new(trimmed),
            home.as_deref(),
            self.mount_inspector.as_ref(),
        ))
    }

    pub fn check_nested_sources(
        &self,
        candidate_id: &str,
        candidate_path: &str,
        destination_id: &str,
    ) -> Result<Vec<NestedSourceConflict>, KesticServiceErr> {
        if candidate_path.trim().is_empty() || destination_id.trim().is_empty() {
            return Ok(Vec::new());
        }
        let d_id = DestinationId::parse(destination_id.trim())
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let c_id = if candidate_id.trim().is_empty() {
            None
        } else {
            Some(
                BackupId::parse(candidate_id.trim())
                    .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?,
            )
        };
        Ok(check_nested_sources_for_candidate(
            c_id.as_ref(),
            std::path::Path::new(candidate_path.trim()),
            &d_id,
            &self.config.backups,
        ))
    }

    pub fn lint_excludes(
        &self,
        excludes_json: &str,
    ) -> Result<Vec<ExcludeLintWarning>, KesticServiceErr> {
        let rules: ExcludeRules = serde_json::from_str(excludes_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid excludes JSON: {e}")))?;
        Ok(lint_exclude_rules(&rules))
    }

    pub fn test_exclude_path(
        &self,
        backup_id: &str,
        excludes_json: &str,
        test_path: &str,
    ) -> Result<ExcludePathTestResult, KesticServiceErr> {
        let b_id = BackupId::parse(backup_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let backup = self
            .config
            .backup(&b_id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let rules: ExcludeRules = serde_json::from_str(excludes_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid excludes JSON: {e}")))?;
        let home = current_home_dir();
        Ok(evaluate_exclude_path(
            &rules,
            &backup.source.path,
            home.as_deref(),
            test_path,
        ))
    }

    pub fn format_exclude_rule_for_item(
        &self,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, KesticServiceErr> {
        let trimmed = item_path.trim();
        if trimmed.is_empty() {
            return Err(KesticServiceErr::InvalidInput(
                "item path must not be empty".to_string(),
            ));
        }
        let is_dir = fs::metadata(trimmed).map(|m| m.is_dir()).unwrap_or(false);
        if is_dir {
            let rule = normalize_folder_rule(trimmed, current_home_dir().as_deref());
            Ok(ExcludeRulePreviewDto {
                is_dir: true,
                rule,
                is_expert_rule: false,
            })
        } else {
            Ok(ExcludeRulePreviewDto {
                is_dir: false,
                rule: trimmed.to_string(),
                is_expert_rule: true,
            })
        }
    }

    pub fn add_exclude_rule_for_item(
        &mut self,
        backup_id: &str,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, KesticServiceErr> {
        let preview = self.format_exclude_rule_for_item(item_path)?;
        let b_id = BackupId::parse(backup_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let idx = self
            .config
            .backups
            .iter()
            .position(|b| b.id == b_id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;

        let mut next = self.config.clone();
        if preview.is_dir {
            if !next.backups[idx].excludes.folders.contains(&preview.rule) {
                next.backups[idx]
                    .excludes
                    .folders
                    .push(preview.rule.clone());
            }
        } else if !next.backups[idx].excludes.lines.contains(&preview.rule) {
            next.backups[idx].excludes.lines.push(preview.rule.clone());
        }
        next.backups[idx]
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let updated_for_file = next.backups[idx].clone();
        self.persist_config(next)?;
        write_exclude_file(&self.state_dir, &updated_for_file)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        Ok(preview)
    }

    pub fn preview_retention(
        &self,
        backup_id: &str,
        retention_json: &str,
    ) -> Result<ForgetPreviewResult, KesticServiceErr> {
        let b_id = BackupId::parse(backup_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let backup = self
            .config
            .backup(&b_id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let dest = self
            .config
            .destination(&backup.destination_id)
            .ok_or_else(|| {
                KesticServiceErr::NotFound(backup.destination_id.as_str().to_string())
            })?;
        let retention: RetentionPolicy = serde_json::from_str(retention_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid retention JSON: {e}")))?;
        retention
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

        let (_, secret_status, _) = self.probe_restic_and_secrets();
        if !secret_status.available {
            return Err(KesticServiceErr::SecretStoreLocked(
                secret_status
                    .message
                    .unwrap_or_else(|| i18n("Password store is locked").into()),
            ));
        }
        let resolved = resolve_and_check(&self.config.restic_provider).map_err(map_provider_err)?;
        let mut temp_backup = backup.clone();
        temp_backup.retention = retention;
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &self.kestic_binary,
            secret_store: self.secret_store.as_ref(),
            clock: self.clock.as_ref(),
        };
        execute_forget_dry_run(&temp_backup, dest, &runtime, &[]).map_err(|err| match err {
            BackupRunError::Destination(d) => map_destination_err(d),
            BackupRunError::InvalidModel(m) => KesticServiceErr::InvalidInput(m.to_string()),
            other => KesticServiceErr::Internal(other.to_string()),
        })
    }

    pub fn get_backup_dry_run_status(
        &self,
        preview_id: &str,
    ) -> Result<BackupDryRunStatusDto, KesticServiceErr> {
        let entry = self
            .dry_runs
            .get(preview_id)
            .ok_or_else(|| KesticServiceErr::NotFound(preview_id.to_string()))?;
        Ok(BackupDryRunStatusDto {
            preview_id: preview_id.to_string(),
            state: entry.state.clone(),
            result: entry.result.clone(),
            error: entry.error.clone(),
        })
    }

    pub fn cancel_backup_dry_run(&self, preview_id: &str) -> Result<(), KesticServiceErr> {
        let entry = self
            .dry_runs
            .get(preview_id)
            .ok_or_else(|| KesticServiceErr::NotFound(preview_id.to_string()))?;
        entry.cancel.cancel();
        Ok(())
    }

    pub fn get_retention_preview_status(
        &self,
        preview_id: &str,
    ) -> Result<RetentionPreviewStatusDto, KesticServiceErr> {
        let entry = self
            .retention_previews
            .get(preview_id)
            .ok_or_else(|| KesticServiceErr::NotFound(preview_id.to_string()))?;
        Ok(RetentionPreviewStatusDto {
            preview_id: preview_id.to_string(),
            state: entry.state.clone(),
            result: entry.result.clone(),
            error: entry.error.clone(),
        })
    }

    pub fn cancel_retention_preview(&self, preview_id: &str) -> Result<(), KesticServiceErr> {
        let entry = self
            .retention_previews
            .get(preview_id)
            .ok_or_else(|| KesticServiceErr::NotFound(preview_id.to_string()))?;
        entry.cancel.cancel();
        Ok(())
    }

    pub fn pause_global(&mut self, duration_json: &str) -> Result<(), KesticServiceErr> {
        let duration: PauseDuration = serde_json::from_str(duration_json).map_err(|e| {
            KesticServiceErr::InvalidInput(format!("invalid pause duration JSON: {e}"))
        })?;
        let now = self.clock.now_utc();
        let schedules: Vec<&Schedule> = self.config.backups.iter().map(|b| &b.schedule).collect();
        let paused_until = compute_pause_until(now, duration, &schedules, &self.timezone);
        let pause = PauseState {
            paused_since: now,
            paused_until,
        };
        self.history
            .set_global_pause(Some(&pause))
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        for backup in &self.config.backups {
            let _ = self.history.delete_pending_runs_for_backup(&backup.id);
        }
        Ok(())
    }

    pub fn resume_global(&mut self) -> Result<(), KesticServiceErr> {
        let now = self.clock.now_utc();
        self.history
            .set_global_pause(None)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        // AC-M3-07-4: reset overdue anchor on resume so paused slots do not count as overdue.
        for backup in &self.config.backups {
            if let Ok(mut rs) = self.history.get_backup_runtime_state(&backup.id)
                && !is_pause_active(rs.pause.as_ref(), now)
            {
                rs.overdue_anchor_at = Some(now);
                rs.last_overdue_notified_at = None;
                let _ = self.history.upsert_backup_runtime_state(&backup.id, &rs);
            }
        }
        Ok(())
    }

    pub fn pause_backup(
        &mut self,
        backup_id: &str,
        duration_json: &str,
    ) -> Result<(), KesticServiceErr> {
        let b_id = BackupId::parse(backup_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let backup = self
            .config
            .backup(&b_id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let duration: PauseDuration = serde_json::from_str(duration_json).map_err(|e| {
            KesticServiceErr::InvalidInput(format!("invalid pause duration JSON: {e}"))
        })?;
        let now = self.clock.now_utc();
        let paused_until = compute_pause_until(now, duration, &[&backup.schedule], &self.timezone);
        let mut rs = self
            .history
            .get_backup_runtime_state(&b_id)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        rs.pause = Some(PauseState {
            paused_since: now,
            paused_until,
        });
        self.history
            .upsert_backup_runtime_state(&b_id, &rs)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        let _ = self.history.delete_pending_runs_for_backup(&b_id);
        Ok(())
    }

    pub fn resume_backup(&mut self, backup_id: &str) -> Result<(), KesticServiceErr> {
        let b_id = BackupId::parse(backup_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        if self.config.backup(&b_id).is_none() {
            return Err(KesticServiceErr::NotFound(backup_id.to_string()));
        }
        let now = self.clock.now_utc();
        let mut rs = self
            .history
            .get_backup_runtime_state(&b_id)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        rs.pause = None;
        // AC-M3-07-4: reset overdue anchor to resume timestamp.
        rs.overdue_anchor_at = Some(now);
        rs.last_overdue_notified_at = None;
        self.history
            .upsert_backup_runtime_state(&b_id, &rs)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        Ok(())
    }

    pub fn unlock_destination(&mut self, destination_id: &str) -> Result<u32, KesticServiceErr> {
        let d_id = DestinationId::parse(destination_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let dest = self
            .config
            .destination(&d_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;

        if self
            .active_runs
            .values()
            .any(|e| e.run.destination_id == dest.id)
        {
            return Err(KesticServiceErr::AlreadyRunning(format!(
                "another run is already active on destination '{}'",
                dest.name
            )));
        }

        let (_, secret_status, _) = self.probe_restic_and_secrets();
        if !secret_status.available {
            return Err(KesticServiceErr::SecretStoreLocked(
                secret_status
                    .message
                    .unwrap_or_else(|| i18n("Password store is locked").into()),
            ));
        }
        let resolved = resolve_and_check(&self.config.restic_provider).map_err(map_provider_err)?;
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &self.kestic_binary,
            secret_store: self.secret_store.as_ref(),
            clock: self.clock.as_ref(),
        };

        let classification = detect_stale_locks(&dest, &runtime, &[]).map_err(|err| match err {
            BackupRunError::Destination(d) => map_destination_err(d),
            other => KesticServiceErr::Internal(other.to_string()),
        })?;

        if !classification.has_stale_local_lock {
            self.dest_stale_locks.insert(d_id, 0);
            if !classification.active_local_locks.is_empty()
                || !classification.foreign_host_locks.is_empty()
            {
                return Err(KesticServiceErr::InvalidInput(
                    i18n("Cannot unlock: repository lock belongs to an active or remote process")
                        .into(),
                ));
            }
            return Ok(0);
        }

        let stale_count = classification.stale_lock_ids.len() as u32;
        let res = execute_unlock(&dest, &runtime, &[]).map_err(|err| match err {
            BackupRunError::Destination(d) => map_destination_err(d),
            other => KesticServiceErr::Internal(other.to_string()),
        })?;
        if res.exit_code != 0 {
            return Err(KesticServiceErr::Internal(
                res.presentation.detail_or_summary(),
            ));
        }
        self.dest_stale_locks.insert(d_id, 0);
        Ok(stale_count)
    }

    pub fn preview_destination_maintenance(
        &self,
        destination_id: &str,
    ) -> Result<PrunePreviewSummary, KesticServiceErr> {
        let d_id = DestinationId::parse(destination_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let dest = self
            .config
            .destination(&d_id)
            .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;

        let (_, secret_status, _) = self.probe_restic_and_secrets();
        if !secret_status.available {
            return Err(KesticServiceErr::SecretStoreLocked(
                secret_status
                    .message
                    .unwrap_or_else(|| i18n("Password store is locked").into()),
            ));
        }
        let resolved = resolve_and_check(&self.config.restic_provider).map_err(map_provider_err)?;
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &self.kestic_binary,
            secret_store: self.secret_store.as_ref(),
            clock: self.clock.as_ref(),
        };
        execute_prune_dry_run(dest, &runtime, None, &[]).map_err(|err| match err {
            BackupRunError::Destination(d) => map_destination_err(d),
            BackupRunError::InvalidModel(m) => KesticServiceErr::InvalidInput(m.to_string()),
            other => KesticServiceErr::Internal(other.to_string()),
        })
    }

    pub fn update_destination_maintenance(
        &mut self,
        destination_id: &str,
        maintenance_json: &str,
    ) -> Result<(), KesticServiceErr> {
        let d_id = DestinationId::parse(destination_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let maintenance: MaintenanceSchedule =
            serde_json::from_str(maintenance_json).map_err(|e| {
                KesticServiceErr::InvalidInput(format!("invalid maintenance JSON: {e}"))
            })?;
        maintenance
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let idx = self
            .config
            .destinations
            .iter()
            .position(|d| d.id == d_id)
            .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;
        let mut next = self.config.clone();
        next.destinations[idx].maintenance = maintenance;
        self.persist_config(next)?;
        self.ensure_baselines();
        Ok(())
    }

    pub fn update_settings(
        &mut self,
        default_overdue_threshold: u32,
        notifications_json: &str,
    ) -> Result<(), KesticServiceErr> {
        if default_overdue_threshold == 0 {
            return Err(KesticServiceErr::InvalidInput(
                "default_overdue_threshold must be >= 1".to_string(),
            ));
        }
        let notifications: NotificationSettings = serde_json::from_str(notifications_json)
            .map_err(|e| {
                KesticServiceErr::InvalidInput(format!("invalid notifications JSON: {e}"))
            })?;
        let mut next = self.config.clone();
        next.default_overdue_threshold = default_overdue_threshold;
        next.notifications = notifications;
        next.validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        self.persist_config(next)?;
        Ok(())
    }

    pub fn list_known_networks(&self) -> Vec<String> {
        list_known_networks(&self.monitor_bus)
    }
}

fn localized_source_check_reason(status: &SourceCheckResult) -> String {
    match status {
        SourceCheckResult::DriveNotMounted => i18n("Source unavailable: drive not mounted").into(),
        SourceCheckResult::DriveMismatch => i18n("Source unavailable: different drive").into(),
        SourceCheckResult::DriveEmpty => i18n("Source unavailable: source empty").into(),
        SourceCheckResult::PathMissing(_) | SourceCheckResult::Available => {
            i18n("Source unavailable").into()
        }
    }
}

fn localized_backup_run_error(err: &BackupRunError) -> String {
    match err {
        BackupRunError::SourceUnavailable(status) => localized_source_check_reason(status),
        BackupRunError::Destination(dest_err) => match dest_err {
            DestinationError::Unreachable(_) => {
                localized_failure_summary(&ResticFailureKind::DestinationUnreachable)
            }
            DestinationError::AuthFailed(_) => {
                localized_failure_summary(&ResticFailureKind::BackendAuthFailed)
            }
            DestinationError::WrongPassword => {
                localized_failure_summary(&ResticFailureKind::WrongPassword)
            }
            DestinationError::RepositoryMissing(_) => {
                localized_failure_summary(&ResticFailureKind::RepositoryMissing)
            }
            DestinationError::CommandFailed { exit_code, message } => {
                let pres = classify_exit(*exit_code, None, Vec::new(), message);
                localized_failure_summary(&pres.kind)
            }
            other => other.to_string(),
        },
        other => other.to_string(),
    }
}

fn parse_setup_mode(raw: &str) -> Result<DestinationSetupMode, KesticServiceErr> {
    match raw {
        "init" => Ok(DestinationSetupMode::InitNew),
        "connect" => Ok(DestinationSetupMode::ConnectExisting),
        other => Err(KesticServiceErr::InvalidInput(format!(
            "invalid setup mode '{other}' (expected 'init' or 'connect')"
        ))),
    }
}

fn strip_prefix_flags(args: &[String]) -> Vec<String> {
    // Drop leading `-r <uri>` and `--json` if present, keeping user/subcommand args clean for
    // `ResticCommandSpec::for_destination` reconstruction.
    let mut i = 0;
    let mut out = Vec::new();
    while i < args.len() {
        let cur = &args[i];
        if (cur == "ionice" && args.get(i + 1).is_some_and(|s| s == "-c"))
            || (cur == "-c" && args.get(i + 1).is_some_and(|s| s == "3"))
            || (cur == "nice" && args.get(i + 1).is_some_and(|s| s == "-n"))
            || (cur == "-n" && args.get(i + 1).is_some_and(|s| s == "10"))
        {
            i += 2;
            continue;
        }
        if cur.ends_with("/restic") || cur == "restic" {
            i += 1;
            continue;
        }
        if cur == "-r" {
            i += 2;
            continue;
        }
        if cur == "--json" {
            i += 1;
            continue;
        }
        out.push(cur.clone());
        i += 1;
    }
    out
}

fn snapshot_to_dto(s: kestic_core::snapshot::ResticSnapshot) -> SnapshotSummaryDto {
    SnapshotSummaryDto {
        short_id: s.short_id.unwrap_or_else(|| s.id.chars().take(8).collect()),
        id: s.id,
        time: s.time,
        hostname: s.hostname.unwrap_or_default(),
        paths: s.paths,
        tags: s.tags,
        total_bytes_processed: s.summary.as_ref().and_then(|x| x.total_bytes_processed),
        total_files_processed: s.summary.as_ref().and_then(|x| x.total_files_processed),
    }
}

fn action_code(action: kestic_core::restic::RecoveryAction) -> &'static str {
    match action {
        kestic_core::restic::RecoveryAction::UpdateStoredPassword => "update_stored_password",
        kestic_core::restic::RecoveryAction::AddToExcludes => "add_to_excludes",
        kestic_core::restic::RecoveryAction::CheckResticSource => "check_restic_source",
        kestic_core::restic::RecoveryAction::UnlockRepository => "unlock_repository",
    }
}

/// Internal service error mapped to `net.daester.Kestic.Error.*` D-Bus errors.
#[derive(Debug)]
pub enum KesticServiceErr {
    InvalidInput(String),
    NotFound(String),
    InUse(String),
    ManualRunBlocked(String),
    DestinationUnreachable(String),
    WrongPassword(String),
    AuthFailed(String),
    SecretStoreLocked(String),
    ResticUnavailable(String),
    AlreadyRunning(String),
    Internal(String),
}

impl std::fmt::Display for KesticServiceErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(m)
            | Self::NotFound(m)
            | Self::InUse(m)
            | Self::ManualRunBlocked(m)
            | Self::DestinationUnreachable(m)
            | Self::WrongPassword(m)
            | Self::AuthFailed(m)
            | Self::SecretStoreLocked(m)
            | Self::ResticUnavailable(m)
            | Self::AlreadyRunning(m)
            | Self::Internal(m) => f.write_str(m),
        }
    }
}

impl From<KesticServiceErr> for KesticDbusError {
    fn from(err: KesticServiceErr) -> Self {
        match err {
            KesticServiceErr::InvalidInput(m) => KesticDbusError::InvalidInput(m),
            KesticServiceErr::NotFound(m) => KesticDbusError::NotFound(m),
            KesticServiceErr::InUse(m) => KesticDbusError::InUse(m),
            KesticServiceErr::ManualRunBlocked(m) => KesticDbusError::ManualRunBlocked(m),
            KesticServiceErr::DestinationUnreachable(m) => {
                KesticDbusError::DestinationUnreachable(m)
            }
            KesticServiceErr::WrongPassword(m) => KesticDbusError::WrongPassword(m),
            KesticServiceErr::AuthFailed(m) => KesticDbusError::AuthFailed(m),
            KesticServiceErr::SecretStoreLocked(m) => KesticDbusError::SecretStoreLocked(m),
            KesticServiceErr::ResticUnavailable(m) => KesticDbusError::ResticUnavailable(m),
            KesticServiceErr::AlreadyRunning(m) => KesticDbusError::AlreadyRunning(m),
            KesticServiceErr::Internal(m) => KesticDbusError::Internal(m),
        }
    }
}

fn map_config_err(err: ConfigError) -> KesticServiceErr {
    match err {
        ConfigError::Validation(_)
        | ConfigError::UnsupportedVersion { .. }
        | ConfigError::MissingVersion => KesticServiceErr::InvalidInput(err.to_string()),
        ConfigError::Model(m) => KesticServiceErr::InvalidInput(m.to_string()),
        ConfigError::Parse(e) => KesticServiceErr::InvalidInput(e.to_string()),
        other => KesticServiceErr::Internal(other.to_string()),
    }
}

fn map_provider_err(err: ResticProviderError) -> KesticServiceErr {
    KesticServiceErr::ResticUnavailable(err.to_string())
}

fn map_secret_err(err: SecretError) -> KesticServiceErr {
    match err {
        SecretError::LockedOrPromptRequired(m) => KesticServiceErr::SecretStoreLocked(m),
        other => KesticServiceErr::Internal(other.to_string()),
    }
}

fn map_destination_err(err: DestinationError) -> KesticServiceErr {
    match err {
        DestinationError::WrongPassword => {
            KesticServiceErr::WrongPassword(i18n("Stored password does not match").into())
        }
        DestinationError::AuthFailed(m) => KesticServiceErr::AuthFailed(m),
        DestinationError::Unreachable(m) => KesticServiceErr::DestinationUnreachable(m),
        DestinationError::Secret(e) => map_secret_err(e),
        other => KesticServiceErr::InvalidInput(other.to_string()),
    }
}

/// Starts a backup manually (enforcing `RunConditions`, `AC-M3-03-7`).
pub fn start_backup_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<String, KesticServiceErr> {
    start_backup_internal(shared, backup_id, false, false)
}

/// Starts a backup manually overriding unmet `RunConditions` for this single run (`FR-13b`, `AC-M3-03-7`).
pub fn start_backup_override_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<String, KesticServiceErr> {
    start_backup_internal(shared, backup_id, true, false)
}

/// Starts a scheduled backup triggered by the M3 scheduler (`M3-S01..S13`).
pub(crate) fn start_scheduled_backup_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<String, KesticServiceErr> {
    start_backup_internal(shared, backup_id, false, true)
}

fn upsert_pending_run_entry(
    guard: &mut DaemonCore,
    backup: &Backup,
    reason: PendingReason,
    summary_message: String,
) {
    let now = guard.clock.now_utc();
    if let Ok(Some(mut existing)) = guard.history.get_pending_backup_run(&backup.id) {
        if existing.pending_reason == Some(reason)
            && existing.summary_message.as_deref() == Some(summary_message.as_str())
        {
            return;
        }
        existing.pending_reason = Some(reason);
        existing.summary_message = Some(summary_message);
        let _ = guard.history.upsert_run(&existing, &[]);
        return;
    }
    if let Ok(mut run) = Run::new_pending_backup(guard.clock.as_ref(), backup, reason) {
        run.started_at = now;
        run.summary_message = Some(summary_message);
        let _ = guard.history.upsert_run(&run, &[]);
    }
}

fn start_backup_internal(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
    allow_condition_override: bool,
    is_scheduled: bool,
) -> Result<String, KesticServiceErr> {
    let b_id =
        BackupId::parse(backup_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

    let (
        backup,
        dest,
        resolved,
        state_dir,
        kestic_binary,
        secret_store,
        clock,
        mount_inspector,
        pending_other_backups,
        run,
        cancel,
        conn_opt,
        tray_opt,
        enable_job_view,
        job_view_dest,
        enable_portal_inhibit,
        monitor_bus,
        enable_system_monitors,
    ) = {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let backup = guard
            .config
            .backup(&b_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let dest = guard
            .config
            .destination(&backup.destination_id)
            .cloned()
            .ok_or_else(|| {
                KesticServiceErr::NotFound(backup.destination_id.as_str().to_string())
            })?;

        if guard
            .active_runs
            .values()
            .any(|e| e.run.destination_id == dest.id)
        {
            return Err(KesticServiceErr::AlreadyRunning(format!(
                "another run is already active on destination '{}'",
                dest.name
            )));
        }

        let (_, secret_status, _) = guard.probe_restic_and_secrets();
        if !secret_status.available {
            return Err(KesticServiceErr::SecretStoreLocked(
                secret_status
                    .message
                    .unwrap_or_else(|| i18n("Password store is locked").into()),
            ));
        }

        let resolved =
            resolve_and_check(&guard.config.restic_provider).map_err(map_provider_err)?;

        // Check source and destination availability right before launch (FR-29, FR-30, P32, M2-S09).
        let source_status = check_source_status_with(
            &backup,
            current_home_dir().as_deref(),
            guard.mount_inspector.as_ref(),
        );
        let source_ok = source_status.is_ok();
        let dest_res = if source_ok {
            preflight_check_destination(&dest, DEFAULT_CONNECT_TIMEOUT)
        } else {
            Ok(())
        };
        let now = guard.clock.now_utc();
        guard
            .source_availability
            .insert(backup.id.clone(), source_status.clone());
        if source_ok {
            guard.availability_probe_count += 1;
            let dto = match &dest_res {
                Ok(()) => DestinationAvailabilityDto {
                    destination_id: dest.id.as_str().to_string(),
                    reachable: true,
                    detail: None,
                    checked_at_unix: now.unix_timestamp(),
                },
                Err(e) => DestinationAvailabilityDto {
                    destination_id: dest.id.as_str().to_string(),
                    reachable: false,
                    detail: Some(e.to_string()),
                    checked_at_unix: now.unix_timestamp(),
                },
            };
            guard.dest_availability.insert(dest.id.clone(), dto);
        }

        if !source_ok || dest_res.is_err() {
            let reason = if !source_ok {
                REASON_SOURCE_UNAVAILABLE
            } else {
                REASON_DESTINATION_UNREACHABLE
            };
            let source_body = if !source_ok {
                localized_source_check_reason(&source_status)
            } else {
                i18n("Destination unreachable").to_string()
            };

            if is_scheduled {
                if !source_ok {
                    upsert_pending_run_entry(
                        &mut guard,
                        &backup,
                        PendingReason::SourceUnavailable,
                        source_body,
                    );
                } else {
                    let mut rs = guard
                        .history
                        .get_backup_runtime_state(&backup.id)
                        .unwrap_or_default();
                    rs.backoff_attempt = rs.backoff_attempt.saturating_add(1);
                    rs.last_failure_at = Some(now);
                    rs.backoff_until = compute_backoff_until(rs.backoff_attempt, now);
                    let _ = guard.history.upsert_backup_runtime_state(&backup.id, &rs);
                    let pending_reason = if rs.backoff_until.is_some() {
                        PendingReason::DestinationUnreachable
                    } else {
                        PendingReason::Backoff
                    };
                    let msg = match rs.backoff_until {
                        Some(until) => {
                            let hint = format_local_slot_hint(until, now, &guard.timezone);
                            i18n("Destination unreachable · next attempt at %1").subs(&[&hint])
                        }
                        None => {
                            i18n("Destination unreachable · waiting for next scheduled run").into()
                        }
                    };
                    upsert_pending_run_entry(&mut guard, &backup, pending_reason, msg);
                }
                let conn_opt = guard.conn.clone();
                let (state_dto, tray_state) = guard.build_state_and_tray();
                if let Some(tray) = &guard.tray {
                    tray.set_full_state(tray_state);
                }
                drop(guard);
                if let Some(conn) = &conn_opt {
                    emit_status_changed(conn, &state_dto);
                }
                return Err(if !source_ok {
                    KesticServiceErr::ManualRunBlocked(reason.to_string())
                } else {
                    KesticServiceErr::DestinationUnreachable(dest_res.unwrap_err().to_string())
                });
            }

            let window_active = guard.window_active;
            let conn_opt = guard.conn.clone();
            let (state_dto, tray_state) = guard.build_state_and_tray();
            if let Some(tray) = &guard.tray {
                tray.set_full_state(tray_state);
            }
            drop(guard);

            if let Some(conn) = &conn_opt {
                emit_manual_run_blocked(conn, &b_id, reason);
                emit_status_changed(conn, &state_dto);
            }
            if !window_active {
                let title = i18n("%1: Backup cannot start").subs(&[&backup.name]);
                let _ =
                    send_event_notification(KesticEvent::ManualRunBlocked, &title, &source_body);
            }
            return Err(KesticServiceErr::ManualRunBlocked(reason.to_string()));
        }

        // AC-M3-03-7 / FR-13b: manual start checks RunConditions unless overridden for this run.
        if !is_scheduled && !allow_condition_override {
            let unmet = evaluate_run_conditions(&backup.run_conditions, &guard.system_env);
            if !unmet.is_empty() {
                let parts: Vec<String> = unmet
                    .iter()
                    .map(|c| guard.format_unmet_condition_item(c))
                    .collect();
                let body = i18n("Condition not met: %1").subs(&[&parts.join(", ")]);
                let window_active = guard.window_active;
                let conn_opt = guard.conn.clone();
                let (state_dto, tray_state) = guard.build_state_and_tray();
                if let Some(tray) = &guard.tray {
                    tray.set_full_state(tray_state);
                }
                drop(guard);

                if let Some(conn) = &conn_opt {
                    emit_manual_run_blocked(conn, &b_id, REASON_CONDITION_NOT_MET);
                    emit_status_changed(conn, &state_dto);
                }
                if !window_active {
                    let title = i18n("%1: Backup cannot start").subs(&[&backup.name]);
                    let _ = send_event_notification(KesticEvent::ManualRunBlocked, &title, &body);
                }
                return Err(KesticServiceErr::ManualRunBlocked(
                    REASON_CONDITION_NOT_MET.to_string(),
                ));
            }
        }

        let _ = guard.history.delete_pending_runs_for_backup(&backup.id);
        guard.sleep_interrupted_backups.remove(&backup.id);

        let pending_ids = guard
            .history
            .list_pending_forget_backup_ids()
            .unwrap_or_default();
        let pending_other_backups: Vec<Backup> = guard
            .config
            .backups
            .iter()
            .filter(|b| {
                b.id != backup.id && b.destination_id == dest.id && pending_ids.contains(&b.id)
            })
            .cloned()
            .collect();

        let run = Run::new_backup(guard.clock.as_ref(), &backup)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        guard
            .history
            .upsert_run(&run, &[])
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;

        let cancel = CancelHandle::new();
        let entry = ActiveRunEntry {
            run: run.clone(),
            backup_id: Some(backup.id.clone()),
            backup_name: backup.name.clone(),
            progress: ActiveRunProgressDto {
                run_id: run.id.as_str().to_string(),
                backup_id: Some(backup.id.as_str().to_string()),
                backup_name: backup.name.clone(),
                percent_done: 0.0,
                bytes_done: 0,
                total_bytes: 0,
                speed_bps: 0,
                seconds_remaining: 0,
            },
            cancel: cancel.clone(),
            sleep_interrupted: false,
        };
        guard.active_runs.insert(run.id.clone(), entry);

        let conn_opt = guard.conn.clone();
        let tray_opt = guard.tray.clone();
        let (state_dto, tray_state) = guard.build_state_and_tray();
        if let Some(tray) = &tray_opt {
            tray.set_full_state(tray_state);
        }
        let params = (
            backup,
            dest,
            resolved,
            guard.state_dir.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
            Arc::clone(&guard.mount_inspector),
            pending_other_backups,
            run,
            cancel,
            conn_opt,
            tray_opt,
            guard.enable_job_view,
            guard.job_view_dest.clone(),
            guard.enable_portal_inhibit,
            guard.monitor_bus.clone(),
            guard.enable_system_monitors,
        );
        drop(guard);

        if let Some(conn) = &params.11 {
            emit_run_updated(conn, &params.9);
            emit_status_changed(conn, &state_dto);
        }
        params
    };

    let run_id_str = run.id.as_str().to_string();
    let shared_worker = Arc::clone(shared);

    thread::spawn(move || {
        let mut _logind_sleep_guard = if enable_system_monitors {
            LogindSleepInhibitor::acquire(&monitor_bus)
        } else {
            None
        };
        let _inhibit_guard = if enable_portal_inhibit {
            PortalInhibitGuard::acquire_default(&format!("Kestic backup: {}", backup.name)).ok()
        } else {
            None
        };

        let job_view = if enable_job_view {
            JobViewHandle::request(
                &job_view_dest,
                KESTIC_DESKTOP_ENTRY,
                &i18n("Backing up %1").subs(&[&backup.name]),
                CAPABILITY_KILLABLE,
            )
            .ok()
        } else {
            None
        };
        let job_view_cancel_guard = job_view.as_ref().map(|jv| {
            let cancel_for_jv = cancel.clone();
            jv.start_cancel_watcher(move || cancel_for_jv.cancel())
        });

        let mut run = run;
        let run_id = run.id.clone();
        let start_unix = clock.now_unix_secs();
        let home = current_home_dir();

        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &kestic_binary,
            secret_store: secret_store.as_ref(),
            clock: clock.as_ref(),
        };
        let exec_ctx = BackupExecutionContext {
            state_dir: &state_dir,
            home_dir: home.as_deref(),
            inspector: mount_inspector.as_ref(),
            cancel: Some(&cancel),
            extra_env: &[],
        };
        let exec_res = execute_backup_with_inspector(&backup, &dest, &runtime, &exec_ctx, |ev| {
            if let ResticStreamEvent::Status(st) = ev {
                let pct = (st.percent_done * 100.0).clamp(0.0, 100.0);
                let bytes_done = st.bytes_done.unwrap_or(0);
                let total_bytes = st.total_bytes.unwrap_or(0);
                let elapsed_secs = (clock.now_unix_secs() - start_unix).max(1) as u64;
                let speed_bps = st
                    .seconds_elapsed
                    .filter(|s| *s > 0)
                    .map(|s| bytes_done / s)
                    .unwrap_or(bytes_done / elapsed_secs);
                let seconds_remaining = st.seconds_remaining.unwrap_or(0);

                if let Some(ref jv) = job_view {
                    let _ = jv.update(&JobProgress {
                        percent: pct.round() as u32,
                        processed_bytes: bytes_done,
                        total_bytes,
                        speed_bytes_per_sec: speed_bps,
                        info_message: i18n("Backing up %1 (%2%)")
                            .subs(&[&backup.name, &(pct.round() as u32).to_string()]),
                        elapsed_ms: elapsed_secs * 1000,
                    });
                }

                let mut guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(entry) = guard.active_runs.get_mut(&run_id) {
                    entry.progress.percent_done = pct;
                    entry.progress.bytes_done = bytes_done;
                    entry.progress.total_bytes = total_bytes;
                    entry.progress.speed_bps = speed_bps;
                    entry.progress.seconds_remaining = seconds_remaining;
                }
                let (_, tray_state) = guard.build_state_and_tray();
                if let Some(tray) = &tray_opt {
                    tray.set_full_state(tray_state);
                }
                drop(guard);

                if let Some(conn) = &conn_opt {
                    emit_run_progress(
                        conn,
                        run_id.as_str(),
                        pct,
                        bytes_done,
                        total_bytes,
                        speed_bps,
                        seconds_remaining,
                    );
                }
            }
        });
        drop(job_view_cancel_guard);

        let sleep_interrupted = {
            let guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
            guard
                .active_runs
                .get(&run_id)
                .is_some_and(|e| e.sleep_interrupted)
        };

        let mut secrets_owned: Vec<SecretValue> = Vec::new();
        if let Ok(Some(s)) = secret_store.get_secret(dest.id.as_str(), SECRET_KIND_REPO) {
            secrets_owned.push(s);
        }
        if dest.kind.uses_rest_credentials()
            && let Ok(Some(s)) = secret_store.get_secret(dest.id.as_str(), SECRET_KIND_REST)
        {
            secrets_owned.push(s);
        }
        let secret_strs: Vec<&str> = secrets_owned
            .iter()
            .filter_map(|s| s.as_str().ok())
            .collect();

        let mut primary_forget_state: Option<bool> = None;
        let mut cleared_other_pending: Vec<BackupId> = Vec::new();
        let mut detected_stale_locks: Option<usize> = None;
        let mut transient_lock_backoff = false;

        let (final_status, exit_code, raw_detail, jv_code, jv_msg) = match exec_res {
            Ok(res) => {
                let mut status = if sleep_interrupted {
                    RunStatus::Interrupted
                } else {
                    res.presentation.status
                };
                let backup_exit = res.exit_code;
                let mut combined_logs = res.log_lines;
                run.commands = vec![res.command_record];
                run.stats = res.stats;

                let mut summary = if sleep_interrupted {
                    Some(i18n("Interrupted by system sleep").into())
                } else {
                    match status {
                        RunStatus::Error | RunStatus::Warning => {
                            let base = localized_failure_summary(&res.presentation.kind);
                            let raw = res
                                .presentation
                                .raw_detail
                                .as_deref()
                                .map(|d| redact_secrets(d, &secret_strs))
                                .unwrap_or_default();
                            if raw.is_empty() {
                                Some(base)
                            } else {
                                Some(format!("{base}: {raw}"))
                            }
                        }
                        _ => Some(localized_failure_summary(&res.presentation.kind)),
                    }
                };

                // AC-M3-06-1..3: Inspect locks whenever `backup` fails with Exit 11.
                if !sleep_interrupted
                    && (backup_exit == 11
                        || matches!(res.presentation.kind, ResticFailureKind::RepositoryLocked))
                {
                    if let Ok(lock_class) = detect_stale_locks(&dest, &runtime, &[]) {
                        detected_stale_locks = Some(if lock_class.has_stale_local_lock {
                            lock_class.stale_lock_ids.len()
                        } else {
                            0
                        });
                    }
                    // AC-M3-04-4: Exit 11 on scheduled backup is transient -> Pending + Backoff.
                    if is_scheduled {
                        transient_lock_backoff = true;
                        status = RunStatus::Pending;
                        run.pending_reason = Some(PendingReason::Backoff);
                    }
                } else if !sleep_interrupted && (backup_exit == 0 || backup_exit == 3) {
                    detected_stale_locks = Some(0);
                }

                // M2-S07: Execute `forget` after backup with Exit 0 or Exit 3; never on Exit 1 / Cancelled / Sleep.
                if !sleep_interrupted
                    && !cancel.is_cancelled()
                    && (backup_exit == 0 || backup_exit == 3)
                {
                    match execute_forget(&backup, &dest, &runtime, Some(&cancel), &[]) {
                        Ok(f_res) => {
                            run.commands.push(f_res.command_record.clone());
                            combined_logs.extend(f_res.log_lines.iter().cloned());
                            if f_res.exit_code == 0 {
                                primary_forget_state = Some(false);
                                // AC-M2-07-4: catch up any other backups on this destination marked pending_forget.
                                for other in &pending_other_backups {
                                    if cancel.is_cancelled() {
                                        break;
                                    }
                                    if let Ok(other_res) =
                                        execute_forget(other, &dest, &runtime, Some(&cancel), &[])
                                    {
                                        run.commands.push(other_res.command_record.clone());
                                        combined_logs.extend(other_res.log_lines.iter().cloned());
                                        if other_res.exit_code == 0 {
                                            cleared_other_pending.push(other.id.clone());
                                        }
                                    }
                                }
                            } else {
                                primary_forget_state = Some(true);
                                status = RunStatus::Warning;
                                if is_forget_lock_failure(&f_res)
                                    && let Ok(lock_class) = detect_stale_locks(&dest, &runtime, &[])
                                {
                                    detected_stale_locks =
                                        Some(if lock_class.has_stale_local_lock {
                                            lock_class.stale_lock_ids.len()
                                        } else {
                                            0
                                        });
                                }
                                if backup_exit == 0 {
                                    summary = Some(if is_forget_lock_failure(&f_res) {
                                        i18n(
                                            "Backed up, destination locked — retention will catch up later",
                                        )
                                        .into()
                                    } else {
                                        i18n("Backed up, retention not executed").into()
                                    });
                                }
                            }
                        }
                        Err(f_err) => {
                            primary_forget_state = Some(true);
                            status = RunStatus::Warning;
                            if backup_exit == 0 {
                                summary = Some(i18n("Backed up, retention not executed").into());
                            }
                            combined_logs.push(redact_secrets(&f_err.to_string(), &secret_strs));
                        }
                    }
                }

                let log_path = {
                    let guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
                    match guard
                        .history
                        .write_redacted_log(&run_id, &combined_logs, &secret_strs)
                    {
                        Ok(path) => Some(path),
                        Err(err) => {
                            eprintln!(
                                "kestic-daemon: failed to write redacted log for run {run_id}: {err}"
                            );
                            None
                        }
                    }
                };
                run.log_path = log_path;
                run.summary_message = summary.clone();
                let jv_err = match status {
                    RunStatus::Ok => (0, String::new()),
                    _ => (1, summary.clone().unwrap_or_default()),
                };
                (status, Some(backup_exit), summary, jv_err.0, jv_err.1)
            }
            Err(err) => {
                if sleep_interrupted {
                    let msg: String = i18n("Interrupted by system sleep").into();
                    run.summary_message = Some(msg.clone());
                    (RunStatus::Interrupted, None, Some(msg.clone()), 1, msg)
                } else {
                    let localized = localized_backup_run_error(&err);
                    let msg = redact_secrets(&localized, &secret_strs);
                    let (status, exit_code) = match err {
                        BackupRunError::SourceUnavailable(_) | BackupRunError::Destination(_) => {
                            (RunStatus::Error, None)
                        }
                        _ => (RunStatus::Error, Some(1)),
                    };
                    run.summary_message = Some(msg.clone());
                    (status, exit_code, Some(msg.clone()), 1, msg)
                }
            }
        };

        run.finish(clock.as_ref(), final_status, exit_code);
        if let Some(jv) = job_view {
            let _ = jv.terminate(jv_code, &jv_msg);
        }

        let (state_dto, notif_settings) = {
            let mut guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
            guard.active_runs.remove(&run_id);
            if let Some(stale_count) = detected_stale_locks {
                guard.dest_stale_locks.insert(dest.id.clone(), stale_count);
            }
            if let Some(pending) = primary_forget_state
                && let Err(err) = guard.history.set_pending_forget(&backup.id, pending)
            {
                eprintln!(
                    "kestic-daemon: failed to update pending_forget for {}: {err}",
                    backup.id
                );
            }
            for other_id in &cleared_other_pending {
                if let Err(err) = guard.history.set_pending_forget(other_id, false) {
                    eprintln!(
                        "kestic-daemon: failed to clear pending_forget for {other_id}: {err}"
                    );
                }
            }

            let finish_now = clock.now_utc();
            if transient_lock_backoff {
                if let Ok(mut rs) = guard.history.get_backup_runtime_state(&backup.id) {
                    rs.backoff_attempt = rs.backoff_attempt.saturating_add(1);
                    rs.last_failure_at = Some(finish_now);
                    rs.backoff_until = compute_backoff_until(rs.backoff_attempt, finish_now);
                    let _ = guard.history.upsert_backup_runtime_state(&backup.id, &rs);
                }
            } else if matches!(final_status, RunStatus::Ok | RunStatus::Warning) {
                if let Ok(mut rs) = guard.history.get_backup_runtime_state(&backup.id) {
                    rs.backoff_attempt = 0;
                    rs.backoff_until = None;
                    rs.last_failure_at = None;
                    rs.last_overdue_notified_at = None;
                    rs.overdue_anchor_at = Some(finish_now);
                    let _ = guard.history.upsert_backup_runtime_state(&backup.id, &rs);
                }
            }

            if let Err(err) = guard.history.upsert_run(&run, &secret_strs) {
                eprintln!(
                    "kestic-daemon: failed to record finished run {run_id} in history: {err}"
                );
            }
            if let Err(err) = guard
                .history
                .prune_logs_for_backup(&backup.id, clock.as_ref())
            {
                eprintln!(
                    "kestic-daemon: failed to prune old logs for backup {}: {err}",
                    backup.id
                );
            }
            let (state_dto, tray_state) = guard.build_state_and_tray();
            if let Some(tray) = &tray_opt {
                tray.set_full_state(tray_state);
            }
            (state_dto, guard.config.notifications.clone())
        };

        if let Some(ref mut inh) = _logind_sleep_guard {
            inh.release();
        }

        // AC-M1-11-4 / AC-M3-08-1 / AC-M3-08-2: Notification ONLY on Error or Warning (never on
        // Ok, Cancelled, Interrupted, or transient Pending), honoring NotificationSettings (F3).
        match final_status {
            RunStatus::Error => {
                let summary = i18n("%1: Backup failed").subs(&[&backup.name]);
                let body = raw_detail.unwrap_or_else(|| {
                    localized_failure_summary(&ResticFailureKind::GenericFailure)
                });
                let _ = send_configured_notification(
                    &notif_settings,
                    KesticEvent::BackupFailed,
                    &summary,
                    &body,
                );
            }
            RunStatus::Warning => {
                let summary = i18n("%1: Backup finished with warning").subs(&[&backup.name]);
                let body = raw_detail.unwrap_or_else(|| {
                    localized_failure_summary(&ResticFailureKind::IncompleteSnapshot {
                        unreadable_items: Vec::new(),
                    })
                });
                let _ = send_configured_notification(
                    &notif_settings,
                    KesticEvent::BackupWarning,
                    &summary,
                    &body,
                );
            }
            _ => {}
        }

        if let Some(conn) = &conn_opt {
            emit_run_updated(conn, &run);
            emit_status_changed(conn, &state_dto);
        }
    });

    Ok(run_id_str)
}

/// Starts an asynchronous dry-run preview (`restic backup --dry-run -vv`, `M2-S04`).
pub fn start_backup_dry_run_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
    excludes_json: &str,
) -> Result<String, KesticServiceErr> {
    let b_id =
        BackupId::parse(backup_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

    let override_excludes = if excludes_json.trim().is_empty() {
        None
    } else {
        let mut ex: ExcludeRules = serde_json::from_str(excludes_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid excludes JSON: {e}")))?;
        ex.normalize_folders(current_home_dir().as_deref());
        ex.validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        Some(ex)
    };

    let (
        backup,
        dest,
        resolved,
        state_dir,
        kestic_binary,
        secret_store,
        clock,
        mount_inspector,
        preview_id,
        cancel,
    ) = {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let mut backup = guard
            .config
            .backup(&b_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        if let Some(ex) = override_excludes {
            backup.excludes = ex;
        }
        let dest = guard
            .config
            .destination(&backup.destination_id)
            .cloned()
            .ok_or_else(|| {
                KesticServiceErr::NotFound(backup.destination_id.as_str().to_string())
            })?;

        let (_, secret_status, _) = guard.probe_restic_and_secrets();
        if !secret_status.available {
            return Err(KesticServiceErr::SecretStoreLocked(
                secret_status
                    .message
                    .unwrap_or_else(|| i18n("Password store is locked").into()),
            ));
        }
        let resolved =
            resolve_and_check(&guard.config.restic_provider).map_err(map_provider_err)?;

        let preview_id = RunId::generate()
            .map(|r| r.as_str().to_string())
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        let cancel = CancelHandle::new();

        if guard.dry_runs.len() > 16 {
            guard.dry_runs.retain(|_, v| v.state == "running");
        }
        guard.dry_runs.insert(
            preview_id.clone(),
            DryRunEntry {
                state: "running".to_string(),
                result: None,
                error: None,
                cancel: cancel.clone(),
            },
        );

        (
            backup,
            dest,
            resolved,
            guard.state_dir.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
            Arc::clone(&guard.mount_inspector),
            preview_id,
            cancel,
        )
    };

    let pid_ret = preview_id.clone();
    let shared_worker = Arc::clone(shared);
    thread::spawn(move || {
        let home = current_home_dir();
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &kestic_binary,
            secret_store: secret_store.as_ref(),
            clock: clock.as_ref(),
        };
        let dry_ctx = BackupExecutionContext {
            state_dir: &state_dir,
            home_dir: home.as_deref(),
            inspector: mount_inspector.as_ref(),
            cancel: Some(&cancel),
            extra_env: &[],
        };
        let res = execute_backup_dry_run_with(&backup, &dest, &runtime, &dry_ctx, |_| {});
        let mut guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = guard.dry_runs.get_mut(&preview_id) {
            match res {
                Ok(dry) => {
                    if dry.cancelled {
                        entry.state = "cancelled".to_string();
                    } else {
                        entry.state = "done".to_string();
                    }
                    entry.result = Some(dry);
                }
                Err(err) => {
                    let msg = localized_backup_run_error(&err);
                    entry.state = "error".to_string();
                    entry.error = Some(msg);
                }
            }
        }
    });

    Ok(pid_ret)
}

/// Starts an asynchronous retention preview (`restic forget --dry-run --no-lock --json`, `M2-S06`).
pub fn start_retention_preview_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
    retention_json: &str,
) -> Result<String, KesticServiceErr> {
    let b_id =
        BackupId::parse(backup_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    let retention: RetentionPolicy = serde_json::from_str(retention_json)
        .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid retention JSON: {e}")))?;
    retention
        .validate()
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

    let (mut backup, dest, resolved, kestic_binary, secret_store, clock, preview_id, cancel) = {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let backup = guard
            .config
            .backup(&b_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let dest = guard
            .config
            .destination(&backup.destination_id)
            .cloned()
            .ok_or_else(|| {
                KesticServiceErr::NotFound(backup.destination_id.as_str().to_string())
            })?;

        let (_, secret_status, _) = guard.probe_restic_and_secrets();
        if !secret_status.available {
            return Err(KesticServiceErr::SecretStoreLocked(
                secret_status
                    .message
                    .unwrap_or_else(|| i18n("Password store is locked").into()),
            ));
        }
        let resolved =
            resolve_and_check(&guard.config.restic_provider).map_err(map_provider_err)?;

        let preview_id = RunId::generate()
            .map(|r| r.as_str().to_string())
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        let cancel = CancelHandle::new();

        if guard.retention_previews.len() > 16 {
            guard.retention_previews.retain(|_, v| v.state == "running");
        }
        guard.retention_previews.insert(
            preview_id.clone(),
            RetentionPreviewEntry {
                state: "running".to_string(),
                result: None,
                error: None,
                cancel: cancel.clone(),
            },
        );

        (
            backup,
            dest,
            resolved,
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
            preview_id,
            cancel,
        )
    };
    backup.retention = retention;

    let pid_ret = preview_id.clone();
    let shared_worker = Arc::clone(shared);
    thread::spawn(move || {
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &kestic_binary,
            secret_store: secret_store.as_ref(),
            clock: clock.as_ref(),
        };
        let res = execute_forget_dry_run_with(&backup, &dest, &runtime, Some(&cancel), &[]);
        let mut guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = guard.retention_previews.get_mut(&preview_id) {
            match res {
                Ok(preview) => {
                    entry.state = "done".to_string();
                    entry.result = Some(preview);
                }
                Err(BackupRunError::Cancelled) => {
                    entry.state = "cancelled".to_string();
                }
                Err(err) => {
                    let msg = match err {
                        BackupRunError::Destination(d) => map_destination_err(d).to_string(),
                        BackupRunError::InvalidModel(m) => m.to_string(),
                        other => localized_backup_run_error(&other),
                    };
                    entry.state = "error".to_string();
                    entry.error = Some(msg);
                }
            }
        }
    });

    Ok(pid_ret)
}

/// Runs catch-up retention (`forget`) for all backups on `destination_id` marked `pending_forget` (`AC-M2-07-4`).
pub fn run_destination_maintenance_async(
    shared: &Arc<Mutex<DaemonCore>>,
    destination_id: &str,
) -> Result<String, KesticServiceErr> {
    run_destination_maintenance_with_options(shared, destination_id, false, false)
}

/// Runs destination maintenance (`pending_forget` -> optional `prune` -> optional `check`, `M3-S05`).
pub fn run_destination_maintenance_with_options(
    shared: &Arc<Mutex<DaemonCore>>,
    destination_id: &str,
    run_prune: bool,
    run_check: bool,
) -> Result<String, KesticServiceErr> {
    let d_id = DestinationId::parse(destination_id)
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

    let (
        dest,
        pending_backups,
        resolved,
        kestic_binary,
        secret_store,
        clock,
        run,
        cancel,
        conn_opt,
        tray_opt,
        enable_job_view,
        job_view_dest,
        enable_portal_inhibit,
        monitor_bus,
        enable_system_monitors,
    ) = {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let dest = guard
            .config
            .destination(&d_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;

        if guard
            .active_runs
            .values()
            .any(|e| e.run.destination_id == dest.id)
        {
            return Err(KesticServiceErr::AlreadyRunning(format!(
                "another run is already active on destination '{}'",
                dest.name
            )));
        }

        let (_, secret_status, _) = guard.probe_restic_and_secrets();
        if !secret_status.available {
            return Err(KesticServiceErr::SecretStoreLocked(
                secret_status
                    .message
                    .unwrap_or_else(|| i18n("Password store is locked").into()),
            ));
        }
        let resolved =
            resolve_and_check(&guard.config.restic_provider).map_err(map_provider_err)?;
        if let Err(err) = preflight_check_destination(&dest, DEFAULT_CONNECT_TIMEOUT) {
            return Err(KesticServiceErr::DestinationUnreachable(err.to_string()));
        }

        let pending_ids = guard
            .history
            .list_pending_forget_backup_ids()
            .unwrap_or_default();
        let pending_backups: Vec<Backup> = guard
            .config
            .backups
            .iter()
            .filter(|b| b.destination_id == d_id && pending_ids.contains(&b.id))
            .cloned()
            .collect();

        let run = Run::new_maintenance(guard.clock.as_ref(), d_id.clone())
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        guard
            .history
            .upsert_run(&run, &[])
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;

        let cancel = CancelHandle::new();
        let entry = ActiveRunEntry {
            run: run.clone(),
            backup_id: None,
            backup_name: dest.name.clone(),
            progress: ActiveRunProgressDto {
                run_id: run.id.as_str().to_string(),
                backup_id: None,
                backup_name: dest.name.clone(),
                percent_done: 0.0,
                bytes_done: 0,
                total_bytes: 0,
                speed_bps: 0,
                seconds_remaining: 0,
            },
            cancel: cancel.clone(),
            sleep_interrupted: false,
        };
        guard.active_runs.insert(run.id.clone(), entry);

        let conn_opt = guard.conn.clone();
        let tray_opt = guard.tray.clone();
        let (state_dto, tray_state) = guard.build_state_and_tray();
        if let Some(tray) = &tray_opt {
            tray.set_full_state(tray_state);
        }
        if let Some(conn) = &conn_opt {
            emit_run_updated(conn, &run);
            emit_status_changed(conn, &state_dto);
        }

        (
            dest,
            pending_backups,
            resolved,
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
            run,
            cancel,
            conn_opt,
            tray_opt,
            guard.enable_job_view,
            guard.job_view_dest.clone(),
            guard.enable_portal_inhibit,
            guard.monitor_bus.clone(),
            guard.enable_system_monitors,
        )
    };

    let run_id_str = run.id.as_str().to_string();
    let shared_worker = Arc::clone(shared);
    thread::spawn(move || {
        let mut _logind_sleep_guard = if enable_system_monitors {
            LogindSleepInhibitor::acquire(&monitor_bus)
        } else {
            None
        };
        let _inhibit_guard = if enable_portal_inhibit {
            PortalInhibitGuard::acquire_default(&format!("Kestic maintenance: {}", dest.name)).ok()
        } else {
            None
        };
        let job_view = if enable_job_view {
            JobViewHandle::request(
                &job_view_dest,
                KESTIC_DESKTOP_ENTRY,
                &i18n("Maintaining %1").subs(&[&dest.name]),
                CAPABILITY_KILLABLE,
            )
            .ok()
        } else {
            None
        };
        let job_view_cancel_guard = job_view.as_ref().map(|jv| {
            let cancel_for_jv = cancel.clone();
            jv.start_cancel_watcher(move || cancel_for_jv.cancel())
        });

        let mut run = run;
        let run_id = run.id.clone();
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &kestic_binary,
            secret_store: secret_store.as_ref(),
            clock: clock.as_ref(),
        };

        let mut secrets_owned: Vec<SecretValue> = Vec::new();
        if let Ok(Some(s)) = secret_store.get_secret(dest.id.as_str(), SECRET_KIND_REPO) {
            secrets_owned.push(s);
        }
        if dest.kind.uses_rest_credentials()
            && let Ok(Some(s)) = secret_store.get_secret(dest.id.as_str(), SECRET_KIND_REST)
        {
            secrets_owned.push(s);
        }
        let secret_strs: Vec<&str> = secrets_owned
            .iter()
            .filter_map(|s| s.as_str().ok())
            .collect();

        let mut cleared_ids = Vec::new();
        let mut forget_failed = false;
        let mut prune_or_check_failed = false;
        let mut prune_succeeded_at: Option<OffsetDateTime> = None;
        let mut check_succeeded_at: Option<OffsetDateTime> = None;
        let mut detected_stale_locks: Option<usize> = None;
        let mut last_exit = 0i32;
        let mut combined_logs = Vec::new();
        let mut failure_detail: Option<String> = None;

        for b in &pending_backups {
            if cancel.is_cancelled() {
                break;
            }
            match execute_forget(b, &dest, &runtime, Some(&cancel), &[]) {
                Ok(res) => {
                    run.commands.push(res.command_record.clone());
                    combined_logs.extend(res.log_lines.iter().cloned());
                    last_exit = res.exit_code;
                    if res.exit_code == 0 {
                        cleared_ids.push(b.id.clone());
                    } else {
                        forget_failed = true;
                        if is_forget_lock_failure(&res)
                            && let Ok(lock_class) = detect_stale_locks(&dest, &runtime, &[])
                        {
                            detected_stale_locks = Some(if lock_class.has_stale_local_lock {
                                lock_class.stale_lock_ids.len()
                            } else {
                                0
                            });
                        }
                    }
                }
                Err(err) => {
                    forget_failed = true;
                    last_exit = 1;
                    let msg = redact_secrets(&err.to_string(), &secret_strs);
                    failure_detail = Some(msg.clone());
                    combined_logs.push(msg);
                }
            }
        }

        if run_prune && !cancel.is_cancelled() {
            match execute_prune(&dest, &runtime, Some(&cancel), &[]) {
                Ok(res) => {
                    run.commands.push(res.command_record.clone());
                    combined_logs.extend(res.log_lines.iter().cloned());
                    last_exit = res.exit_code;
                    if res.exit_code == 0 {
                        prune_succeeded_at = Some(clock.now_utc());
                    } else if !cancel.is_cancelled() {
                        prune_or_check_failed = true;
                        let detail =
                            redact_secrets(&res.presentation.detail_or_summary(), &secret_strs);
                        failure_detail = Some(detail);
                        if res.exit_code == 11
                            && let Ok(lock_class) = detect_stale_locks(&dest, &runtime, &[])
                        {
                            detected_stale_locks = Some(if lock_class.has_stale_local_lock {
                                lock_class.stale_lock_ids.len()
                            } else {
                                0
                            });
                        }
                    }
                }
                Err(err) => {
                    if !cancel.is_cancelled() {
                        prune_or_check_failed = true;
                        last_exit = 1;
                        let msg = redact_secrets(&err.to_string(), &secret_strs);
                        failure_detail = Some(msg.clone());
                        combined_logs.push(msg);
                    }
                }
            }
        }

        if run_check && !cancel.is_cancelled() {
            match execute_check(&dest, &runtime, Some(&cancel), &[]) {
                Ok(res) => {
                    run.commands.push(res.command_record.clone());
                    combined_logs.extend(res.log_lines.iter().cloned());
                    last_exit = res.exit_code;
                    if res.exit_code == 0 {
                        check_succeeded_at = Some(clock.now_utc());
                    } else if !cancel.is_cancelled() {
                        prune_or_check_failed = true;
                        let detail =
                            redact_secrets(&res.presentation.detail_or_summary(), &secret_strs);
                        failure_detail = Some(detail);
                        if res.exit_code == 11
                            && let Ok(lock_class) = detect_stale_locks(&dest, &runtime, &[])
                        {
                            detected_stale_locks = Some(if lock_class.has_stale_local_lock {
                                lock_class.stale_lock_ids.len()
                            } else {
                                0
                            });
                        }
                    }
                }
                Err(err) => {
                    if !cancel.is_cancelled() {
                        prune_or_check_failed = true;
                        last_exit = 1;
                        let msg = redact_secrets(&err.to_string(), &secret_strs);
                        failure_detail = Some(msg.clone());
                        combined_logs.push(msg);
                    }
                }
            }
        }

        drop(job_view_cancel_guard);

        let sleep_interrupted = {
            let guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
            guard
                .active_runs
                .get(&run_id)
                .is_some_and(|e| e.sleep_interrupted)
        };

        let log_path = {
            let guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
            match guard
                .history
                .write_redacted_log(&run_id, &combined_logs, &secret_strs)
            {
                Ok(path) => Some(path),
                Err(err) => {
                    eprintln!(
                        "kestic-daemon: failed to write maintenance log for run {run_id}: {err}"
                    );
                    None
                }
            }
        };
        run.log_path = log_path;

        let status = if sleep_interrupted {
            RunStatus::Interrupted
        } else if cancel.is_cancelled() {
            RunStatus::Cancelled
        } else if prune_or_check_failed {
            RunStatus::Error
        } else if forget_failed {
            RunStatus::Warning
        } else {
            RunStatus::Ok
        };
        if run.summary_message.is_none() {
            run.summary_message = failure_detail.clone();
        }
        run.finish(clock.as_ref(), status, Some(last_exit));

        if let Some(jv) = job_view {
            let jv_code = if status == RunStatus::Ok { 0 } else { 1 };
            let _ = jv.terminate(jv_code, run.summary_message.as_deref().unwrap_or(""));
        }

        let (state_dto, notif_settings) = {
            let mut guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
            guard.active_runs.remove(&run_id);
            if let Some(stale_count) = detected_stale_locks {
                guard.dest_stale_locks.insert(dest.id.clone(), stale_count);
            }
            for id in &cleared_ids {
                if let Err(err) = guard.history.set_pending_forget(id, false) {
                    eprintln!(
                        "kestic-daemon: failed to clear pending_forget for {id} in maintenance: {err}"
                    );
                }
            }
            if (prune_succeeded_at.is_some() || check_succeeded_at.is_some())
                && let Ok(mut ds) = guard.history.get_destination_maintenance_state(&dest.id)
            {
                if let Some(ts) = prune_succeeded_at {
                    ds.last_prune_at = Some(ts);
                }
                if let Some(ts) = check_succeeded_at {
                    ds.last_check_at = Some(ts);
                }
                let _ = guard
                    .history
                    .upsert_destination_maintenance_state(&dest.id, &ds);
            }
            if let Err(err) = guard.history.upsert_run(&run, &secret_strs) {
                eprintln!(
                    "kestic-daemon: failed to record maintenance run {run_id} in history: {err}"
                );
            }
            let (state_dto, tray_state) = guard.build_state_and_tray();
            if let Some(tray) = &tray_opt {
                tray.set_full_state(tray_state);
            }
            (state_dto, guard.config.notifications.clone())
        };

        if let Some(ref mut inh) = _logind_sleep_guard {
            inh.release();
        }

        if status == RunStatus::Error {
            let summary = i18n("%1: Maintenance failed").subs(&[&dest.name]);
            let body = failure_detail
                .unwrap_or_else(|| i18n("Destination maintenance failed").to_string());
            let _ = send_configured_notification(
                &notif_settings,
                KesticEvent::MaintenanceFailed,
                &summary,
                &body,
            );
        }

        if let Some(conn) = &conn_opt {
            emit_run_updated(conn, &run);
            emit_status_changed(conn, &state_dto);
        }
    });

    Ok(run_id_str)
}

struct StartAllEntry {
    backup: Backup,
    has_error: bool,
    error_summary: Option<String>,
    source_status: kestic_core::mount::SourceCheckResult,
    unmet_parts: Vec<String>,
}

pub fn start_all_backups_async(shared: &Arc<Mutex<DaemonCore>>) -> StartAllResultDto {
    let entries: Vec<StartAllEntry> = {
        let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let (state_dto, _) = guard.build_state_and_tray();
        let home = current_home_dir();
        state_dto
            .backups
            .into_iter()
            .map(|s| {
                let source_status = check_source_status_with(
                    &s.backup,
                    home.as_deref(),
                    guard.mount_inspector.as_ref(),
                );
                let unmet_parts: Vec<String> =
                    evaluate_run_conditions(&s.backup.run_conditions, &guard.system_env)
                        .iter()
                        .map(|c| guard.format_unmet_condition_item(c))
                        .collect();
                StartAllEntry {
                    backup: s.backup,
                    has_error: s.has_error,
                    error_summary: s.error_summary,
                    source_status,
                    unmet_parts,
                }
            })
            .collect()
    };

    let mut started_run_ids = Vec::new();
    let mut skipped = Vec::new();
    let mut skipped_history_entries: Vec<(Backup, String, String)> = Vec::new();

    for StartAllEntry {
        backup,
        has_error,
        error_summary,
        source_status,
        unmet_parts,
    } in entries
    {
        let id = backup.id.as_str().to_string();
        let name = backup.name.clone();
        if has_error {
            let reason_text = error_summary
                .unwrap_or_else(|| i18n("Permanent error — fix required before backing up").into());
            skipped.push(SkippedBackupDto {
                backup_id: id,
                backup_name: name,
                reason: "permanent_error".to_string(),
                reason_text: reason_text.clone(),
            });
            skipped_history_entries.push((backup, "permanent_error".to_string(), reason_text));
            continue;
        }
        if !source_status.is_ok() {
            let reason_text = localized_source_check_reason(&source_status);
            skipped.push(SkippedBackupDto {
                backup_id: id,
                backup_name: name,
                reason: REASON_SOURCE_UNAVAILABLE.to_string(),
                reason_text: reason_text.clone(),
            });
            skipped_history_entries.push((
                backup,
                REASON_SOURCE_UNAVAILABLE.to_string(),
                reason_text,
            ));
            continue;
        }
        if !unmet_parts.is_empty() {
            let reason_text = i18n("Condition not met: %1").subs(&[&unmet_parts.join(", ")]);
            skipped.push(SkippedBackupDto {
                backup_id: id,
                backup_name: name,
                reason: REASON_CONDITION_NOT_MET.to_string(),
                reason_text: reason_text.clone(),
            });
            skipped_history_entries.push((
                backup,
                REASON_CONDITION_NOT_MET.to_string(),
                reason_text,
            ));
            continue;
        }
        match start_backup_async(shared, &id) {
            Ok(run_id) => started_run_ids.push(run_id),
            Err(err) => {
                let (reason, reason_text) = match &err {
                    KesticServiceErr::ManualRunBlocked(r) if r == REASON_SOURCE_UNAVAILABLE => {
                        (r.clone(), i18n("Source unavailable").into())
                    }
                    KesticServiceErr::ManualRunBlocked(r) if r == REASON_CONDITION_NOT_MET => {
                        (r.clone(), i18n("Run condition not met").into())
                    }
                    KesticServiceErr::ManualRunBlocked(r) => {
                        (r.clone(), i18n("Destination unreachable").into())
                    }
                    KesticServiceErr::AlreadyRunning(_) => (
                        "already_running".to_string(),
                        i18n("Another run is already running").into(),
                    ),
                    other => ("error".to_string(), other.to_string()),
                };
                skipped_history_entries.push((backup, reason.clone(), reason_text.clone()));
                skipped.push(SkippedBackupDto {
                    backup_id: id,
                    backup_name: name,
                    reason,
                    reason_text,
                });
            }
        }
    }

    // AC-M3-08-6: Record skipped backups from `StartAllBackups` so Verlauf reflects why they were skipped.
    if !skipped_history_entries.is_empty() {
        let (conn_opt, state_dto) = {
            let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
            for (backup, reason, reason_text) in skipped_history_entries {
                let note = i18n("Skipped during 'Back up all': %1").subs(&[&reason_text]);
                match reason.as_str() {
                    REASON_SOURCE_UNAVAILABLE => upsert_pending_run_entry(
                        &mut guard,
                        &backup,
                        PendingReason::SourceUnavailable,
                        note,
                    ),
                    REASON_DESTINATION_UNREACHABLE => upsert_pending_run_entry(
                        &mut guard,
                        &backup,
                        PendingReason::DestinationUnreachable,
                        note,
                    ),
                    REASON_CONDITION_NOT_MET => upsert_pending_run_entry(
                        &mut guard,
                        &backup,
                        PendingReason::ConditionNotMet,
                        note,
                    ),
                    _ => {
                        if let Ok(mut meta_run) =
                            Run::new_meta(guard.clock.as_ref(), backup.destination_id.clone())
                        {
                            meta_run.summary_message =
                                Some(i18n("%1: %2").subs(&[&backup.name, &note]));
                            meta_run.finish(guard.clock.as_ref(), RunStatus::Warning, None);
                            let _ = guard.history.upsert_run(&meta_run, &[]);
                        }
                    }
                }
            }
            let (state_dto, tray_state) = guard.build_state_and_tray();
            if let Some(tray) = &guard.tray {
                tray.set_full_state(tray_state);
            }
            (guard.conn.clone(), state_dto)
        };
        if let Some(conn) = &conn_opt {
            emit_status_changed(conn, &state_dto);
        }
    }

    StartAllResultDto {
        started_run_ids,
        skipped,
    }
}

/// Evaluates scheduled backups & destination maintenance for one tick (`M3-S01..S13`).
pub fn tick_scheduler_async(shared: &Arc<Mutex<DaemonCore>>) {
    let (actions, overdue_notices, conn_opt, state_dto_opt, notif_settings) = {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let now = guard.clock.now_utc();

        // 1. Expire finished global pause and update overdue anchors (`AC-M3-07-3`, `AC-M3-07-4`).
        let mut global_pause = guard.history.get_global_pause().ok().flatten();
        if let Some(ref gp) = global_pause
            && let Some(until) = gp.paused_until
            && now >= until
        {
            let _ = guard.history.set_global_pause(None);
            global_pause = None;
            for backup in &guard.config.backups {
                if let Ok(mut rs) = guard.history.get_backup_runtime_state(&backup.id)
                    && !is_pause_active(rs.pause.as_ref(), now)
                {
                    let anchor = rs.overdue_anchor_at.map_or(until, |a| a.max(until));
                    rs.overdue_anchor_at = Some(anchor);
                    rs.last_overdue_notified_at = None;
                    let _ = guard.history.upsert_backup_runtime_state(&backup.id, &rs);
                }
            }
        }

        // Refresh local source status for scheduled / sleep-interrupted backups (`AC-M3-13-5`).
        let home = current_home_dir();
        let backups_snapshot = guard.config.backups.clone();
        for backup in &backups_snapshot {
            let status =
                check_source_status_with(backup, home.as_deref(), guard.mount_inspector.as_ref());
            guard.source_availability.insert(backup.id.clone(), status);
        }

        let all_runs = guard.history.list_runs().unwrap_or_default();
        let busy_dests: HashSet<DestinationId> = guard
            .active_runs
            .values()
            .map(|e| e.run.destination_id.clone())
            .collect();
        let mut backup_dest_map = HashMap::new();
        let mut evaluations = Vec::with_capacity(backups_snapshot.len());
        let mut overdue_notices: Vec<(String, usize)> = Vec::new();

        for backup in &backups_snapshot {
            backup_dest_map.insert(backup.id.clone(), backup.destination_id.clone());
            let mut rs = guard
                .history
                .get_backup_runtime_state(&backup.id)
                .unwrap_or_default();

            // Expire finished per-backup pause (`AC-M3-07-3`, `AC-M3-07-4`).
            if let Some(ref p) = rs.pause
                && let Some(until) = p.paused_until
                && now >= until
            {
                rs.pause = None;
                let anchor = rs.overdue_anchor_at.map_or(until, |a| a.max(until));
                rs.overdue_anchor_at = Some(anchor);
                rs.last_overdue_notified_at = None;
                let _ = guard.history.upsert_backup_runtime_state(&backup.id, &rs);
            }

            // Reset stale backoff state from an earlier slot once a newer regular slot arrives (`AC-M3-04-1`).
            let due_now_slots = {
                let catchup_anchor = match (
                    guard
                        .history
                        .last_successful_backup_at(&backup.id)
                        .ok()
                        .flatten(),
                    rs.baseline_at,
                ) {
                    (Some(a), Some(b)) => a.max(b),
                    (Some(a), None) => a,
                    (None, Some(b)) => b,
                    (None, None) => now,
                };
                backup_slots_between(&backup.schedule, catchup_anchor, now, &guard.timezone)
            };
            if let (Some(latest_slot), Some(failed_at)) =
                (due_now_slots.last().copied(), rs.last_failure_at)
                && latest_slot > failed_at
                && (rs.backoff_attempt > 0 || rs.backoff_until.is_some())
            {
                rs.backoff_attempt = 0;
                rs.backoff_until = None;
                rs.last_failure_at = None;
                let _ = guard.history.upsert_backup_runtime_state(&backup.id, &rs);
            }

            let backoff_elapsed = rs.backoff_until.is_some_and(|until| now >= until);

            let last_run = all_runs.iter().find(|r| {
                r.backup_id() == Some(&backup.id)
                    && !matches!(r.status, RunStatus::Pending | RunStatus::Running)
            });
            let has_error = last_run.is_some_and(|r| r.status == RunStatus::Error);
            let source_ok = guard
                .source_availability
                .get(&backup.id)
                .map(|s| s.is_ok())
                .unwrap_or(true);
            let cached_dest_ok = guard
                .dest_availability
                .get(&backup.destination_id)
                .map(|d| d.reachable)
                .unwrap_or(true);
            let dest_busy = busy_dests.contains(&backup.destination_id);
            let can_probe_unreachable_dest = !cached_dest_ok
                && !dest_busy
                && !has_error
                && source_ok
                && !due_now_slots.is_empty()
                && !is_pause_active(rs.pause.as_ref(), now)
                && !is_pause_active(global_pause.as_ref(), now)
                && evaluate_run_conditions(&backup.run_conditions, &guard.system_env).is_empty()
                && (backoff_elapsed || (rs.backoff_attempt == 0 && rs.backoff_until.is_none()));
            let dest_ok = cached_dest_ok || can_probe_unreachable_dest;
            let last_success_at = guard
                .history
                .last_successful_backup_at(&backup.id)
                .ok()
                .flatten();

            let mut eval = evaluate_backup_schedule(
                &BackupScheduleEvalInput {
                    backup,
                    runtime_state: &rs,
                    last_success_at,
                    overdue_anchor_at: None,
                    global_pause: global_pause.as_ref(),
                    global_overdue_threshold: guard.config.default_overdue_threshold,
                    source_available: source_ok,
                    destination_reachable: dest_ok,
                    destination_busy: dest_busy,
                    has_permanent_error: has_error,
                },
                &guard.system_env,
                now,
                &guard.timezone,
            );

            // AC-M3-09-3: Allow sleep-interrupted backups (even ManualOnly) to resume after wake.
            if eval.due_slot.is_none()
                && !eval.is_paused
                && !has_error
                && guard.sleep_interrupted_backups.contains(&backup.id)
            {
                let unmet = collect_pending_preconditions(
                    source_ok,
                    dest_ok,
                    &backup.run_conditions,
                    &guard.system_env,
                    dest_busy,
                    eval.in_backoff,
                );
                eval.due_slot = Some(now);
                eval.should_start_now = unmet.is_empty();
                eval.unmet_preconditions = unmet;
            }

            if eval.is_overdue && rs.last_overdue_notified_at.is_none() {
                rs.last_overdue_notified_at = Some(now);
                let _ = guard.history.upsert_backup_runtime_state(&backup.id, &rs);
                overdue_notices.push((backup.name.clone(), eval.missed_slots_for_overdue));
            }

            if !eval.should_start_now
                && !eval.is_paused
                && !has_error
                && !dest_busy
                && !eval.unmet_preconditions.is_empty()
            {
                let primary = &eval.unmet_preconditions[0];
                let (_, msg) = guard.format_pending_reason_entry(primary, rs.backoff_until, now);
                upsert_pending_run_entry(&mut guard, backup, primary.to_pending_reason(), msg);
            } else if eval.due_slot.is_none() || eval.is_paused || has_error {
                let _ = guard.history.delete_pending_runs_for_backup(&backup.id);
            }

            evaluations.push(eval);
        }

        let mut maint_needs = Vec::with_capacity(guard.config.destinations.len());
        for dest in &guard.config.destinations {
            let ds = guard
                .history
                .get_destination_maintenance_state(&dest.id)
                .unwrap_or_default();
            let has_pending_forget = guard.config.backups.iter().any(|b| {
                b.destination_id == dest.id
                    && guard.history.is_pending_forget(&b.id).unwrap_or(false)
            });
            let dest_busy = busy_dests.contains(&dest.id);
            let dest_ok = guard
                .dest_availability
                .get(&dest.id)
                .map(|d| d.reachable)
                .unwrap_or(true);
            maint_needs.push(evaluate_destination_maintenance(
                &DestinationMaintenanceEvalInput {
                    destination: dest,
                    state: &ds,
                    has_pending_forget,
                    global_pause: global_pause.as_ref(),
                    destination_busy: dest_busy,
                    destination_reachable: dest_ok,
                },
                &guard.system_env,
                now,
                &guard.timezone,
            ));
        }

        let actions =
            plan_destination_actions(&evaluations, &maint_needs, &busy_dests, &backup_dest_map);

        let state_dto_opt = if !overdue_notices.is_empty() || !actions.is_empty() {
            let (state_dto, tray_state) = guard.build_state_and_tray();
            if let Some(tray) = &guard.tray {
                tray.set_full_state(tray_state);
            }
            Some(state_dto)
        } else {
            None
        };

        (
            actions,
            overdue_notices,
            guard.conn.clone(),
            state_dto_opt,
            guard.config.notifications.clone(),
        )
    };

    for (name, missed) in overdue_notices {
        let summary = i18n("%1: Backup overdue").subs(&[&name]);
        let body = i18n("%1 scheduled backups missed").subs(&[&missed.to_string()]);
        let _ = send_configured_notification(
            &notif_settings,
            KesticEvent::BackupOverdue,
            &summary,
            &body,
        );
    }

    if let (Some(conn), Some(state_dto)) = (&conn_opt, &state_dto_opt) {
        emit_status_changed(conn, state_dto);
    }

    for action in actions {
        match action {
            DestinationTickAction::StartBackup { backup_id, .. } => {
                let _ = start_scheduled_backup_async(shared, backup_id.as_str());
            }
            DestinationTickAction::StartMaintenance {
                destination_id,
                run_prune,
                run_check,
            } => {
                let _ = run_destination_maintenance_with_options(
                    shared,
                    destination_id.as_str(),
                    run_prune,
                    run_check,
                );
            }
        }
    }
}

struct KesticDbusV1Service {
    core: Arc<Mutex<DaemonCore>>,
}

impl KesticDbusV1Service {
    async fn sync_tray_and_emit(
        &self,
        conn: &Connection,
        state_dto: &DaemonStateDto,
        tray_state: KesticTrayState,
    ) {
        let tray_opt = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.tray.clone()
        };
        if let Some(tray) = tray_opt {
            tray.set_full_state(tray_state);
        }
        emit_status_changed_async(conn, state_dto).await;
    }
}

#[interface(name = "net.daester.Kestic1")]
impl KesticDbusV1Service {
    #[zbus(name = "DaemonVersion")]
    fn daemon_version(&self) -> String {
        crate::daemon_version().to_string()
    }

    #[zbus(name = "GetState")]
    fn get_state(&self) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let (state_dto, _) = guard.build_state_and_tray();
        serde_json::to_string(&state_dto).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "GetRunDetail")]
    fn get_run_detail(&self, run_id: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let detail = guard.get_run_detail(run_id)?;
        serde_json::to_string(&detail).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "ListBackupSnapshots")]
    fn list_backup_snapshots(&self, backup_id: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let snaps = guard.list_backup_snapshots(backup_id)?;
        serde_json::to_string(&snaps).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "TestDestinationConnection")]
    fn test_destination_connection(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let res = guard.test_destination_connection(dest_json, mode, repo_password, rest_password);
        serde_json::to_string(&res).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "AddDestination")]
    async fn add_destination(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> Result<String, KesticDbusError> {
        let (new_id, conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let new_id = guard.add_destination(dest_json, mode, repo_password, rest_password)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (new_id, guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(new_id)
    }

    #[zbus(name = "UpdateDestination")]
    async fn update_destination(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
        verify_connection: bool,
    ) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.update_destination(
                dest_json,
                mode,
                repo_password,
                rest_password,
                verify_connection,
            )?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "UpdateDestinationPassword")]
    async fn update_destination_password(
        &self,
        dest_id: &str,
        new_password: &str,
    ) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.update_destination_password(dest_id, new_password)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "RemoveDestination")]
    async fn remove_destination(&self, dest_id: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.remove_destination(dest_id)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "AddBackup")]
    async fn add_backup(&self, backup_json: &str) -> Result<String, KesticDbusError> {
        let (new_id, conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let new_id = guard.add_backup(backup_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (new_id, guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(new_id)
    }

    #[zbus(name = "UpdateBackup")]
    async fn update_backup(&self, backup_json: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.update_backup(backup_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "RenameBackup")]
    async fn rename_backup(&self, backup_id: &str, new_name: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.rename_backup(backup_id, new_name)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "ReassignBackup")]
    async fn reassign_backup(
        &self,
        backup_id: &str,
        new_dest_id: &str,
    ) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.reassign_backup(backup_id, new_dest_id)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "RemoveBackup")]
    async fn remove_backup(&self, backup_id: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.remove_backup(backup_id)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "SetResticProvider")]
    async fn set_restic_provider(&self, provider_json: &str) -> Result<String, KesticDbusError> {
        let (res, conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let res = guard.set_restic_provider(provider_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (res, guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        serde_json::to_string(&res).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "SetExpertMode")]
    async fn set_expert_mode(&self, enabled: bool) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.set_expert_mode(enabled)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "StartBackup")]
    async fn start_backup(&self, backup_id: &str) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = backup_id.to_string();
        run_blocking_task(move || start_backup_async(&core, &id))
            .await
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "StartAllBackups")]
    async fn start_all_backups(&self) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let res = run_blocking_task(move || start_all_backups_async(&core)).await;
        serde_json::to_string(&res).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "CancelRun")]
    fn cancel_run(&self, run_id: &str) -> Result<(), KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        Ok(guard.cancel_run(run_id)?)
    }

    #[zbus(name = "CheckAvailability")]
    async fn check_availability(&self, force: bool) -> Result<String, KesticDbusError> {
        let (avail, conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let avail = guard.refresh_availability_if_needed(force);
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (avail, guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        serde_json::to_string(&avail).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "WindowActive")]
    async fn window_active(&self, active: bool, page: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.set_window_active(active, page);
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "RegisterGui")]
    fn register_gui(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(), KesticDbusError> {
        if let Some(sender) = header.sender() {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.registered_gui_sender = Some(sender.as_str().to_string());
        }
        Ok(())
    }

    #[zbus(name = "ConfirmQuit")]
    fn confirm_quit(&self) -> Result<(), KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.request_shutdown();
        Ok(())
    }

    #[zbus(name = "UnlockSecretStore")]
    async fn unlock_secret_store(&self) -> Result<bool, KesticDbusError> {
        let secret_store = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            Arc::clone(&guard.secret_store)
        };
        let unlock_res = run_blocking_task(move || secret_store.unlock_interactive()).await;

        let (conn_opt, state_dto, tray_state) = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }

        match unlock_res {
            Ok(()) => Ok(true),
            Err(SecretError::LockedOrPromptRequired(_)) => Ok(false),
            Err(err) => Err(KesticDbusError::SecretStoreLocked(err.to_string())),
        }
    }

    #[zbus(name = "DetectSource")]
    fn detect_source(&self, path: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let src = guard.detect_source(path)?;
        serde_json::to_string(&src).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "CheckNestedSources")]
    fn check_nested_sources(
        &self,
        candidate_id: &str,
        candidate_path: &str,
        destination_id: &str,
    ) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let conflicts = guard.check_nested_sources(candidate_id, candidate_path, destination_id)?;
        serde_json::to_string(&conflicts).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "LintExcludes")]
    fn lint_excludes(&self, excludes_json: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let warnings = guard.lint_excludes(excludes_json)?;
        serde_json::to_string(&warnings).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "TestExcludePath")]
    fn test_exclude_path(
        &self,
        backup_id: &str,
        excludes_json: &str,
        test_path: &str,
    ) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let res = guard.test_exclude_path(backup_id, excludes_json, test_path)?;
        serde_json::to_string(&res).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "PreviewRetention")]
    async fn preview_retention(
        &self,
        backup_id: &str,
        retention_json: &str,
    ) -> Result<String, KesticDbusError> {
        let (mut backup, dest, resolved, kestic_binary, secret_store, clock) = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let b_id = BackupId::parse(backup_id)
                .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
            let backup = guard
                .config
                .backup(&b_id)
                .cloned()
                .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
            let dest = guard
                .config
                .destination(&backup.destination_id)
                .cloned()
                .ok_or_else(|| {
                    KesticServiceErr::NotFound(backup.destination_id.as_str().to_string())
                })?;
            let retention: RetentionPolicy = serde_json::from_str(retention_json).map_err(|e| {
                KesticServiceErr::InvalidInput(format!("invalid retention JSON: {e}"))
            })?;
            retention
                .validate()
                .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
            let (_, secret_status, _) = guard.probe_restic_and_secrets();
            if !secret_status.available {
                return Err(KesticServiceErr::SecretStoreLocked(
                    secret_status
                        .message
                        .unwrap_or_else(|| i18n("Password store is locked").into()),
                )
                .into());
            }
            let resolved =
                resolve_and_check(&guard.config.restic_provider).map_err(map_provider_err)?;
            let mut b = backup;
            b.retention = retention;
            (
                b,
                dest,
                resolved,
                guard.kestic_binary.clone(),
                Arc::clone(&guard.secret_store),
                Arc::clone(&guard.clock),
            )
        };
        let _ = &mut backup;
        let preview = run_blocking_task(move || {
            let runtime = ResticRuntime {
                resolved: &resolved,
                kestic_binary: &kestic_binary,
                secret_store: secret_store.as_ref(),
                clock: clock.as_ref(),
            };
            execute_forget_dry_run(&backup, &dest, &runtime, &[]).map_err(|err| match err {
                BackupRunError::Destination(d) => map_destination_err(d),
                BackupRunError::InvalidModel(m) => KesticServiceErr::InvalidInput(m.to_string()),
                other => KesticServiceErr::Internal(other.to_string()),
            })
        })
        .await
        .map_err(KesticDbusError::from)?;
        serde_json::to_string(&preview).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "StartRetentionPreview")]
    async fn start_retention_preview(
        &self,
        backup_id: &str,
        retention_json: &str,
    ) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = backup_id.to_string();
        let ret = retention_json.to_string();
        run_blocking_task(move || start_retention_preview_async(&core, &id, &ret))
            .await
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "CancelRetentionPreview")]
    fn cancel_retention_preview(&self, preview_id: &str) -> Result<(), KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        Ok(guard.cancel_retention_preview(preview_id)?)
    }

    #[zbus(name = "GetRetentionPreviewStatus")]
    fn get_retention_preview_status(&self, preview_id: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let st = guard.get_retention_preview_status(preview_id)?;
        serde_json::to_string(&st).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "StartBackupDryRun")]
    async fn start_backup_dry_run(
        &self,
        backup_id: &str,
        excludes_json: &str,
    ) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = backup_id.to_string();
        let ex = excludes_json.to_string();
        run_blocking_task(move || start_backup_dry_run_async(&core, &id, &ex))
            .await
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "CancelBackupDryRun")]
    fn cancel_backup_dry_run(&self, preview_id: &str) -> Result<(), KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        Ok(guard.cancel_backup_dry_run(preview_id)?)
    }

    #[zbus(name = "GetBackupDryRunStatus")]
    fn get_backup_dry_run_status(&self, preview_id: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let st = guard.get_backup_dry_run_status(preview_id)?;
        serde_json::to_string(&st).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "FormatExcludeRuleForItem")]
    fn format_exclude_rule_for_item(&self, item_path: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let preview = guard.format_exclude_rule_for_item(item_path)?;
        serde_json::to_string(&preview).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "AddExcludeRuleForItem")]
    async fn add_exclude_rule_for_item(
        &self,
        backup_id: &str,
        item_path: &str,
    ) -> Result<String, KesticDbusError> {
        let (preview, conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let preview = guard.add_exclude_rule_for_item(backup_id, item_path)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (preview, guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        serde_json::to_string(&preview).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "RunDestinationMaintenance")]
    async fn run_destination_maintenance(
        &self,
        destination_id: &str,
    ) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = destination_id.to_string();
        run_blocking_task(move || run_destination_maintenance_async(&core, &id))
            .await
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "PauseGlobal")]
    async fn pause_global(&self, duration_json: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.pause_global(duration_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "ResumeGlobal")]
    async fn resume_global(&self) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.resume_global()?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "PauseBackup")]
    async fn pause_backup(
        &self,
        backup_id: &str,
        duration_json: &str,
    ) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.pause_backup(backup_id, duration_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "ResumeBackup")]
    async fn resume_backup(&self, backup_id: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.resume_backup(backup_id)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "StartBackupOverride")]
    async fn start_backup_override(&self, backup_id: &str) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = backup_id.to_string();
        run_blocking_task(move || start_backup_override_async(&core, &id))
            .await
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "UnlockDestination")]
    async fn unlock_destination(&self, destination_id: &str) -> Result<u32, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = destination_id.to_string();
        let count = run_blocking_task(move || {
            let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
            guard.unlock_destination(&id)
        })
        .await
        .map_err(KesticDbusError::from)?;

        let (conn_opt, state_dto, tray_state) = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(count)
    }

    #[zbus(name = "PreviewDestinationMaintenance")]
    async fn preview_destination_maintenance(
        &self,
        destination_id: &str,
    ) -> Result<String, KesticDbusError> {
        let (dest, resolved, kestic_binary, secret_store, clock) = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let d_id = DestinationId::parse(destination_id)
                .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
            let dest = guard
                .config
                .destination(&d_id)
                .cloned()
                .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;
            let (_, secret_status, _) = guard.probe_restic_and_secrets();
            if !secret_status.available {
                return Err(KesticServiceErr::SecretStoreLocked(
                    secret_status
                        .message
                        .unwrap_or_else(|| i18n("Password store is locked").into()),
                )
                .into());
            }
            let resolved =
                resolve_and_check(&guard.config.restic_provider).map_err(map_provider_err)?;
            (
                dest,
                resolved,
                guard.kestic_binary.clone(),
                Arc::clone(&guard.secret_store),
                Arc::clone(&guard.clock),
            )
        };

        let preview = run_blocking_task(move || {
            let runtime = ResticRuntime {
                resolved: &resolved,
                kestic_binary: &kestic_binary,
                secret_store: secret_store.as_ref(),
                clock: clock.as_ref(),
            };
            execute_prune_dry_run(&dest, &runtime, None, &[]).map_err(|err| match err {
                BackupRunError::Destination(d) => map_destination_err(d),
                BackupRunError::InvalidModel(m) => KesticServiceErr::InvalidInput(m.to_string()),
                other => KesticServiceErr::Internal(other.to_string()),
            })
        })
        .await
        .map_err(KesticDbusError::from)?;
        serde_json::to_string(&preview).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "UpdateDestinationMaintenance")]
    async fn update_destination_maintenance(
        &self,
        destination_id: &str,
        maintenance_json: &str,
    ) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.update_destination_maintenance(destination_id, maintenance_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "UpdateSettings")]
    async fn update_settings(
        &self,
        default_overdue_threshold: u32,
        notifications_json: &str,
    ) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.update_settings(default_overdue_threshold, notifications_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "ListKnownNetworks")]
    async fn list_known_networks(&self) -> Result<String, KesticDbusError> {
        let monitor_bus = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.monitor_bus.clone()
        };
        let networks = run_blocking_task(move || list_known_networks(&monitor_bus)).await;
        serde_json::to_string(&networks).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(signal, name = "StatusChanged")]
    async fn status_changed(
        ctxt: &SignalEmitter<'_>,
        overall_status_json: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal, name = "RunUpdated")]
    async fn run_updated(ctxt: &SignalEmitter<'_>, run_json: &str) -> zbus::Result<()>;

    #[zbus(signal, name = "RunProgress")]
    async fn run_progress(
        ctxt: &SignalEmitter<'_>,
        run_id: &str,
        percent_done: f64,
        bytes_done: u64,
        total_bytes: u64,
        speed_bps: u64,
        seconds_remaining: u64,
    ) -> zbus::Result<()>;

    #[zbus(signal, name = "ManualRunBlocked")]
    async fn manual_run_blocked(
        ctxt: &SignalEmitter<'_>,
        backup_id: &str,
        reason: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal, name = "ShowWindowRequested")]
    async fn show_window_requested(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal, name = "QuitConfirmationRequested")]
    async fn quit_confirmation_requested(
        ctxt: &SignalEmitter<'_>,
        has_running_run: bool,
        running_backup_name: &str,
    ) -> zbus::Result<()>;
}

async fn run_blocking_task<T, F>(f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let state: Arc<Mutex<(Option<T>, Option<std::task::Waker>)>> =
        Arc::new(Mutex::new((None, None)));
    let state_bg = Arc::clone(&state);
    thread::spawn(move || {
        let value = f();
        let waker = {
            let mut guard = state_bg.lock().unwrap_or_else(|p| p.into_inner());
            guard.0 = Some(value);
            guard.1.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    });
    std::future::poll_fn(move |cx| {
        let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(val) = guard.0.take() {
            std::task::Poll::Ready(val)
        } else {
            guard.1 = Some(cx.waker().clone());
            std::task::Poll::Pending
        }
    })
    .await
}

async fn emit_status_changed_async(conn: &Connection, state_dto: &DaemonStateDto) {
    if let Ok(json) = serde_json::to_string(&state_dto.overall)
        && let Ok(emitter) = SignalEmitter::new(conn, KESTIC_OBJECT_PATH)
    {
        let _ = KesticDbusV1Service::status_changed(&emitter, &json).await;
    }
}

fn emit_status_changed(conn: &Connection, state_dto: &DaemonStateDto) {
    if let Ok(json) = serde_json::to_string(&state_dto.overall) {
        let conn = conn.clone();
        thread::spawn(move || {
            let _ = zbus::block_on(async move {
                let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
                KesticDbusV1Service::status_changed(&emitter, &json).await
            });
        });
    }
}

fn emit_run_updated(conn: &Connection, run: &Run) {
    if let Ok(json) = serde_json::to_string(run) {
        let conn = conn.clone();
        thread::spawn(move || {
            let _ = zbus::block_on(async move {
                let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
                KesticDbusV1Service::run_updated(&emitter, &json).await
            });
        });
    }
}

fn emit_run_progress(
    conn: &Connection,
    run_id: &str,
    percent_done: f64,
    bytes_done: u64,
    total_bytes: u64,
    speed_bps: u64,
    seconds_remaining: u64,
) {
    let conn = conn.clone();
    let run_id = run_id.to_string();
    thread::spawn(move || {
        let _ = zbus::block_on(async move {
            let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
            KesticDbusV1Service::run_progress(
                &emitter,
                &run_id,
                percent_done,
                bytes_done,
                total_bytes,
                speed_bps,
                seconds_remaining,
            )
            .await
        });
    });
}

fn emit_manual_run_blocked(conn: &Connection, backup_id: &BackupId, reason: &str) {
    let conn = conn.clone();
    let bid = backup_id.as_str().to_string();
    let reason = reason.to_string();
    thread::spawn(move || {
        let _ = zbus::block_on(async move {
            let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
            KesticDbusV1Service::manual_run_blocked(&emitter, &bid, &reason).await
        });
    });
}

fn emit_show_window_requested(conn: &Connection) {
    let conn = conn.clone();
    thread::spawn(move || {
        let _ = zbus::block_on(async move {
            let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
            KesticDbusV1Service::show_window_requested(&emitter).await
        });
    });
}

fn emit_quit_confirmation_requested(
    conn: &Connection,
    has_running_run: bool,
    running_backup_name: &str,
) {
    let conn = conn.clone();
    let name = running_backup_name.to_string();
    thread::spawn(move || {
        let _ = zbus::block_on(async move {
            let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
            KesticDbusV1Service::quit_confirmation_requested(&emitter, has_running_run, &name).await
        });
    });
}

/// Runs the Kestic daemon service loop (`SingleInstanceLock` + `net.daester.Kestic1` + `TrayHandle`).
pub fn run_daemon(config: DaemonRunConfig) -> Result<DaemonOutcome, String> {
    let core = Arc::new(Mutex::new(DaemonCore::new(&config)?));
    let (quit_tx, quit_rx) = mpsc::channel();
    let (action_tx, action_rx) = mpsc::channel();

    {
        let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
        guard.quit_tx = Some(quit_tx.clone());
    }

    let dbus_service = KesticDbusV1Service {
        core: Arc::clone(&core),
    };
    let lock = match SingleInstanceLock::acquire_with_interface(&config.bus_name, dbus_service)
        .map_err(|err| format!("failed to acquire D-Bus name `{}`: {err}", config.bus_name))?
    {
        InstanceAcquire::Acquired(lock) => lock,
        InstanceAcquire::AlreadyRunning => return Ok(DaemonOutcome::AlreadyRunning),
    };

    let conn = lock.connection().clone();
    let (_, initial_tray_state) = {
        let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
        guard.conn = Some(conn.clone());
        guard.build_state_and_tray()
    };

    let core_for_about = Arc::clone(&core);
    let conn_for_about = conn.clone();
    let about_to_show_hook: Arc<dyn Fn() -> Option<KesticTrayState> + Send + Sync> =
        Arc::new(move || {
            let (state_dto, tray_state) = {
                let mut guard = core_for_about.lock().unwrap_or_else(|p| p.into_inner());
                guard.refresh_availability_if_needed(false);
                guard.build_state_and_tray()
            };
            emit_status_changed(&conn_for_about, &state_dto);
            Some(tray_state)
        });

    let tray = Arc::new(
        TrayHandle::spawn_with_actions(
            initial_tray_state,
            config.assume_sni_available,
            Some(action_tx),
            Some(quit_tx),
            Some(about_to_show_hook),
        )
        .map_err(|err| format!("failed to register StatusNotifierItem tray: {err}"))?,
    );

    {
        let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
        guard.tray = Some(Arc::clone(&tray));
    }

    if config.send_test_notification {
        send_event_notification(
            KesticEvent::BackupWarning,
            &i18n("Kestic daemon test notification").to_string(),
            &i18n("Tray registered and notification channel active.").to_string(),
        )
        .map_err(|err| format!("failed to send test notification: {err}"))?;
    }

    if let Some(ready_path) = &config.ready_file {
        fs::write(ready_path, "ready\n")
            .map_err(|err| format!("failed to write ready file {}: {err}", ready_path.display()))?;
    }

    let mut monitor_rx_and_handle: Option<(
        mpsc::Receiver<SystemMonitorEvent>,
        SystemMonitorHandle,
    )> = if config.enable_system_monitors {
        let (mon_tx, mon_rx) = mpsc::channel();
        let handle = SystemMonitorHandle::start(config.monitor_bus.clone(), mon_tx, None);
        Some((mon_rx, handle))
    } else {
        None
    };

    let bin_identity = if config.enable_binary_watch {
        BinaryIdentity::capture_current().ok()
    } else {
        None
    };

    if config.enable_scheduler {
        tick_scheduler_async(&core);
    }

    let deadline = config.duration.map(|d| Instant::now() + d);
    let mut last_sched_tick = Instant::now();
    let mut last_bin_check = Instant::now();
    let mut last_network_reconnect_probe: Option<time::OffsetDateTime> = None;
    let mut pending_exec_path: Option<PathBuf> = None;

    loop {
        while let Ok(action) = action_rx.try_recv() {
            handle_tray_action(&core, &conn, &config, action);
        }

        if let Some((ref mon_rx, _)) = monitor_rx_and_handle {
            while let Ok(ev) = mon_rx.try_recv() {
                handle_system_monitor_event(
                    &core,
                    &conn,
                    &tray,
                    &config,
                    ev,
                    &mut last_network_reconnect_probe,
                );
            }
        }

        if config.enable_scheduler && last_sched_tick.elapsed() >= Duration::from_secs(1) {
            last_sched_tick = Instant::now();
            tick_scheduler_async(&core);
        }

        if let Some(ref bin_id) = bin_identity
            && last_bin_check.elapsed() >= Duration::from_millis(500)
        {
            last_bin_check = Instant::now();
            if matches!(bin_id.check(), BinaryStatus::Replaced(_)) {
                let has_active_runs = {
                    let guard = core.lock().unwrap_or_else(|p| p.into_inner());
                    !guard.active_runs.is_empty()
                };
                if !has_active_runs {
                    pending_exec_path = Some(bin_id.exe_path().to_path_buf());
                    break;
                }
            }
        }

        if core
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .shutdown_flag
            .load(Ordering::SeqCst)
        {
            break;
        }

        let step_wait = if monitor_rx_and_handle.is_some() || bin_identity.is_some() {
            Duration::from_millis(25)
        } else {
            Duration::from_millis(50)
        };

        if let Some(end) = deadline {
            let now = Instant::now();
            if now >= end {
                break;
            }
            let wait = (end - now).min(step_wait);
            if quit_rx.recv_timeout(wait).is_ok() {
                break;
            }
        } else if quit_rx.recv_timeout(step_wait).is_ok() {
            break;
        }
    }

    while let Ok(action) = action_rx.try_recv() {
        handle_tray_action(&core, &conn, &config, action);
    }

    if let Some((_, handle)) = monitor_rx_and_handle.take() {
        handle.stop();
    }

    {
        let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
        guard.cancel_any_active_run();
        guard.tray = None;
        guard.conn = None;
    }

    if let Ok(owned) = Arc::try_unwrap(tray) {
        owned.shutdown();
    }
    let _ = zbus::block_on(conn.release_name(config.bus_name.as_str()));
    drop(lock);
    let _ = zbus::block_on(conn.close());

    if let Some(exe_path) = pending_exec_path {
        use std::os::unix::process::CommandExt;
        let mut cmd = Command::new(&exe_path);
        if let Some(args) = &config.binary_watch_args {
            cmd.args(args);
        } else {
            cmd.args(env::args_os().skip(1));
        }
        let err = cmd.exec();
        return Err(format!(
            "failed to exec replaced binary {}: {err}",
            exe_path.display()
        ));
    }

    Ok(DaemonOutcome::RanAndStopped)
}

fn handle_system_monitor_event(
    core: &Arc<Mutex<DaemonCore>>,
    conn: &Connection,
    tray: &Arc<TrayHandle>,
    config: &DaemonRunConfig,
    ev: SystemMonitorEvent,
    last_network_reconnect_probe: &mut Option<time::OffsetDateTime>,
) {
    match ev {
        SystemMonitorEvent::EnvironmentChanged(new_env) => {
            let (state_dto, tray_state) = {
                let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
                let old_env = guard.system_env.clone();
                let network_changed = old_env.network_connected != new_env.network_connected
                    || old_env.network_metered != new_env.network_metered
                    || old_env.active_networks != new_env.active_networks;
                let network_just_came_online =
                    !old_env.network_connected && new_env.network_connected;
                guard.system_env = new_env;

                if network_changed {
                    let now = guard.clock.now_utc();
                    let cooldown_elapsed = match *last_network_reconnect_probe {
                        None => true,
                        Some(prev) => {
                            (now - prev) > time::Duration::seconds(AVAILABILITY_CACHE_TTL_SECS)
                        }
                    };
                    let force_probe = network_just_came_online && cooldown_elapsed;
                    if force_probe {
                        *last_network_reconnect_probe = Some(now);
                    }
                    let _ = guard.refresh_availability_if_needed(force_probe);

                    for backup in guard.config.backups.clone() {
                        let dest_ok = guard
                            .dest_availability
                            .get(&backup.destination_id)
                            .map(|d| d.reachable)
                            .unwrap_or(true);
                        if dest_ok
                            && let Ok(mut rs) = guard.history.get_backup_runtime_state(&backup.id)
                            && (rs.backoff_until.is_some() || rs.backoff_attempt > 0)
                        {
                            rs.backoff_until = None;
                            rs.backoff_attempt = 0;
                            let _ = guard.history.upsert_backup_runtime_state(&backup.id, &rs);
                        }
                    }
                }
                guard.build_state_and_tray()
            };
            tray.set_full_state(tray_state);
            emit_status_changed(conn, &state_dto);
            if config.enable_scheduler {
                tick_scheduler_async(core);
            }
        }
        SystemMonitorEvent::MountChanged => {
            let (state_dto, tray_state) = {
                let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
                let home = current_home_dir();
                for backup in guard.config.backups.clone() {
                    let status = check_source_status_with(
                        &backup,
                        home.as_deref(),
                        guard.mount_inspector.as_ref(),
                    );
                    guard.source_availability.insert(backup.id.clone(), status);
                }
                let now_unix = guard.clock.now_unix_secs();
                for dest in guard.config.destinations.clone() {
                    if matches!(dest.kind, DestinationKind::Local { .. }) {
                        let res = preflight_check_destination(&dest, DEFAULT_CONNECT_TIMEOUT);
                        let dto = match res {
                            Ok(()) => DestinationAvailabilityDto {
                                destination_id: dest.id.as_str().to_string(),
                                reachable: true,
                                detail: None,
                                checked_at_unix: now_unix,
                            },
                            Err(err) => DestinationAvailabilityDto {
                                destination_id: dest.id.as_str().to_string(),
                                reachable: false,
                                detail: Some(err.to_string()),
                                checked_at_unix: now_unix,
                            },
                        };
                        guard.dest_availability.insert(dest.id.clone(), dto);
                    }
                }
                guard.build_state_and_tray()
            };
            tray.set_full_state(tray_state);
            emit_status_changed(conn, &state_dto);
            if config.enable_scheduler {
                tick_scheduler_async(core);
            }
        }
        SystemMonitorEvent::PrepareForSleep(true) => {
            // AC-M3-09-2: interrupt running jobs cleanly and mark them for post-wake resume.
            let (state_dto, tray_state) = {
                let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
                let running_ids: Vec<BackupId> = guard
                    .active_runs
                    .values()
                    .filter_map(|e| e.backup_id.clone())
                    .collect();
                for entry in guard.active_runs.values_mut() {
                    entry.sleep_interrupted = true;
                }
                for bid in running_ids {
                    guard.sleep_interrupted_backups.insert(bid);
                }
                guard.cancel_any_active_run();
                guard.build_state_and_tray()
            };
            tray.set_full_state(tray_state);
            emit_status_changed(conn, &state_dto);
        }
        SystemMonitorEvent::PrepareForSleep(false) => {
            // AC-M3-09-3 / AC-M3-01-7: re-read environment, refresh availability, and catch up.
            let new_env = if config.enable_system_monitors {
                read_system_environment(&config.monitor_bus)
            } else {
                SystemEnvironment::desktop_connected()
            };
            let (state_dto, tray_state) = {
                let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
                guard.system_env = new_env;
                let _ = guard.refresh_availability_if_needed(true);
                guard.build_state_and_tray()
            };
            tray.set_full_state(tray_state);
            emit_status_changed(conn, &state_dto);
            if config.enable_scheduler {
                tick_scheduler_async(core);
            }
        }
    }
}

fn check_gui_sender_alive(conn: &Connection, sender: &str) -> Result<bool, zbus::Error> {
    let bus_name = zbus::names::BusName::try_from(sender)
        .map_err(|e| zbus::Error::Failure(format!("invalid bus name `{sender}`: {e}")))?;
    zbus::block_on(async move {
        let proxy = zbus::fdo::DBusProxy::new(conn).await?;
        proxy.name_has_owner(bus_name).await.map_err(Into::into)
    })
}

fn is_gui_live(core: &Arc<Mutex<DaemonCore>>, conn: &Connection) -> bool {
    let sender_opt = {
        let guard = core.lock().unwrap_or_else(|p| p.into_inner());
        guard.registered_gui_sender.clone()
    };
    let Some(sender) = sender_opt else {
        let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
        guard.window_active = false;
        return false;
    };
    match check_gui_sender_alive(conn, &sender) {
        Ok(true) => true,
        Ok(false) | Err(_) => {
            let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
            if guard.registered_gui_sender.as_deref() == Some(sender.as_str()) {
                guard.registered_gui_sender = None;
                guard.window_active = false;
            }
            false
        }
    }
}

fn spawn_gui_process(kestic_binary: &std::path::Path, bus_name: &str, confirm_quit: bool) {
    let mut cmd = Command::new(kestic_binary);
    if bus_name != KESTIC_BUS_NAME {
        cmd.arg("--bus-name").arg(bus_name);
    }
    if confirm_quit {
        cmd.arg("--confirm-quit");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(mut child) = cmd.spawn() {
        thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

fn handle_tray_action(
    core: &Arc<Mutex<DaemonCore>>,
    conn: &Connection,
    config: &DaemonRunConfig,
    action: TrayAction,
) {
    match action {
        TrayAction::StartBackup(backup_id) => {
            let _ = start_backup_async(core, &backup_id);
        }
        TrayAction::StartAllBackups => {
            let res = start_all_backups_async(core);
            if !res.skipped.is_empty() {
                let notif_settings = {
                    let guard = core.lock().unwrap_or_else(|p| p.into_inner());
                    guard.config.notifications.clone()
                };
                let summary: Vec<String> = res
                    .skipped
                    .iter()
                    .map(|s| format!("• {}: {}", s.backup_name, s.reason_text))
                    .collect();
                let title = i18n("Some backups were skipped").to_string();
                let body = summary.join("\n");
                let _ = send_configured_notification(
                    &notif_settings,
                    KesticEvent::BackupWarning,
                    &title,
                    &body,
                );
            }
        }
        TrayAction::CancelRun(run_id) => {
            let guard = core.lock().unwrap_or_else(|p| p.into_inner());
            let _ = guard.cancel_run(&run_id);
        }
        TrayAction::PauseGlobal(duration) => {
            if let Ok(dur_json) = serde_json::to_string(&duration) {
                let mut pair = None;
                let tray_opt = {
                    let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
                    if guard.pause_global(&dur_json).is_ok() {
                        pair = Some(guard.build_state_and_tray());
                    }
                    guard.tray.clone()
                };
                if let Some((dto, ts)) = pair {
                    if let Some(tray) = tray_opt {
                        tray.set_full_state(ts);
                    }
                    emit_status_changed(conn, &dto);
                }
            }
        }
        TrayAction::ResumeGlobal => {
            let mut pair = None;
            let tray_opt = {
                let mut guard = core.lock().unwrap_or_else(|p| p.into_inner());
                if guard.resume_global().is_ok() {
                    pair = Some(guard.build_state_and_tray());
                }
                guard.tray.clone()
            };
            if let Some((dto, ts)) = pair {
                if let Some(tray) = tray_opt {
                    tray.set_full_state(ts);
                }
                emit_status_changed(conn, &dto);
            }
        }
        TrayAction::OpenWindow => {
            let gui_live = is_gui_live(core, conn);
            let (kestic_binary, suppress) = {
                let guard = core.lock().unwrap_or_else(|p| p.into_inner());
                (
                    guard.kestic_binary.clone(),
                    guard.suppress_ui_spawn || config.suppress_ui_spawn,
                )
            };
            emit_show_window_requested(conn);
            if !gui_live && !suppress {
                spawn_gui_process(&kestic_binary, &config.bus_name, false);
            }
        }
        TrayAction::RequestQuit => {
            // AC-M1-11-3: Never terminate immediately; ask user via warning confirmation first.
            let gui_live = is_gui_live(core, conn);
            let (has_running, running_name, kestic_binary, suppress) = {
                let guard = core.lock().unwrap_or_else(|p| p.into_inner());
                let first_active = guard.active_runs.values().next();
                (
                    first_active.is_some(),
                    first_active
                        .map(|a| a.backup_name.clone())
                        .unwrap_or_default(),
                    guard.kestic_binary.clone(),
                    guard.suppress_ui_spawn || config.suppress_ui_spawn,
                )
            };
            emit_quit_confirmation_requested(conn, has_running, &running_name);
            if !gui_live && !suppress {
                spawn_gui_process(&kestic_binary, &config.bus_name, true);
            }
        }
    }
}

/// Returns `true` if string literals contain localized action label mapping (compile-time check helper).
pub fn localized_action_title(action: kestic_core::restic::RecoveryAction) -> String {
    localized_recovery_action_label(action)
}
