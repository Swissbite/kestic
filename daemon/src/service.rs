// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Full Kestic M1 daemon service (`net.daester.Kestic1` D-Bus server, single-writer config/history,
//! async backup worker with cancellation & Plasma JobView/Portal Inhibit, 60s availability cache,
//! SNI tray synchronization, and desktop notifications).

use crate::dbus_api::{
    ActiveRunProgressDto, AvailabilitySnapshotDto, BackupAvailabilityDto, BackupSnapshotsDto,
    BackupSummaryDto, BlockingStateDto, ConnectionTestResultDto, DaemonStateDto,
    DestinationAvailabilityDto, DestinationSummaryDto, KESTIC_BUS_NAME, KESTIC_OBJECT_PATH,
    KesticDbusError, OverallStatusDto, OverallStatusKind, PreviousDestinationSnapshotsDto,
    REASON_DESTINATION_UNREACHABLE, REASON_SOURCE_UNAVAILABLE, ResticStatusDto, RunDetailDto,
    SecretStoreStatusDto, SkippedBackupDto, SnapshotSummaryDto, StartAllResultDto,
};
use crate::i18n::{i18n, localized_failure_summary, localized_recovery_action_label};
use crate::inhibit::PortalInhibitGuard;
use crate::job_view::{CAPABILITY_KILLABLE, DEFAULT_JOB_VIEW_DEST, JobProgress, JobViewHandle};
use crate::notifications::{KESTIC_DESKTOP_ENTRY, KesticEvent, send_event_notification};
use crate::secrets::SecretServiceStore;
use crate::single_instance::{InstanceAcquire, SingleInstanceLock};
use crate::tray::{
    KesticTrayState, TrayAction, TrayBackupItem, TrayHandle, TrayStatus, compute_tray_presentation,
};
use kestic_core::backup::{
    BackupRunError, check_source_available, default_state_dir, execute_backup,
    list_snapshots_for_backup,
};
use kestic_core::clock::{Clock, SystemClock};
use kestic_core::config::{
    AppConfig, ConfigError, default_config_path, load_optional_config, save_config,
};
use kestic_core::destination::{
    DEFAULT_CONNECT_TIMEOUT, DestinationError, DestinationSetupMode, preflight_check_destination,
    remove_destination_secrets, setup_destination, verify_and_update_stored_password,
};
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, DestinationKind, Run, RunId, RunStatus,
};
use kestic_core::provider::{ResticProviderError, resolve_and_check};
use kestic_core::restic::{
    CancelHandle, ResticCommandSpec, ResticFailureKind, ResticRuntime, ResticStreamEvent,
    SECRET_KIND_REPO, SECRET_KIND_REST, classify_exit, redact_secrets,
};
use kestic_core::secrets::{FileSecretStore, SecretError, SecretStore, SecretValue};
use std::collections::HashMap;
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
    pub suppress_ui_spawn: bool,
    pub enable_job_view: bool,
    pub job_view_dest: String,
    pub enable_portal_inhibit: bool,
    pub simulated_status: Option<SimulatedOverallStatus>,
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
            suppress_ui_spawn: false,
            enable_job_view: true,
            job_view_dest: DEFAULT_JOB_VIEW_DEST.to_string(),
            enable_portal_inhibit: true,
            simulated_status: None,
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
}

/// Core stateful daemon engine shared between D-Bus interface methods, tray actions, and unit tests.
pub struct DaemonCore {
    config_path: PathBuf,
    state_dir: PathBuf,
    kestic_binary: PathBuf,
    clock: Arc<dyn Clock>,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    config: AppConfig,
    has_config_file: bool,
    config_load_error: Option<String>,
    history: RunHistory,
    last_availability_check: Option<OffsetDateTime>,
    dest_availability: HashMap<DestinationId, DestinationAvailabilityDto>,
    source_availability: HashMap<BackupId, bool>,
    availability_probe_count: u64,
    active_runs: HashMap<RunId, ActiveRunEntry>,
    window_active: bool,
    window_page: String,
    registered_gui_sender: Option<String>,
    simulated_status: Option<SimulatedOverallStatus>,
    icon_theme_path: String,
    enable_job_view: bool,
    job_view_dest: String,
    enable_portal_inhibit: bool,
    suppress_ui_spawn: bool,
    conn: Option<Connection>,
    tray: Option<Arc<TrayHandle>>,
    shutdown_flag: Arc<AtomicBool>,
    quit_tx: Option<mpsc::Sender<()>>,
}

impl DaemonCore {
    /// Initializes daemon state, opens SQLite `RunHistory`, recovers stuck `Running` runs as
    /// `Interrupted` (`AC-M1-10-2`), and loads `config.toml` if present (`AC-M1-12-2`).
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

        let history = RunHistory::open(&state_dir).map_err(|e| e.to_string())?;
        // AC-M1-10-2: any runs left in `running` state from a hard crash/kill become `interrupted`.
        let _ = history.mark_interrupted_on_startup(run_config.clock.as_ref());

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

        let mut core = Self {
            config_path,
            state_dir,
            kestic_binary,
            clock: Arc::clone(&run_config.clock),
            secret_store,
            config,
            has_config_file,
            config_load_error,
            history,
            last_availability_check: None,
            dest_availability: HashMap::new(),
            source_availability: HashMap::new(),
            availability_probe_count: 0,
            active_runs: HashMap::new(),
            window_active: false,
            window_page: String::new(),
            registered_gui_sender: None,
            simulated_status: run_config.simulated_status,
            icon_theme_path: run_config.tray_state.icon_theme_path.clone(),
            enable_job_view: run_config.enable_job_view,
            job_view_dest: run_config.job_view_dest.clone(),
            enable_portal_inhibit: run_config.enable_portal_inhibit,
            suppress_ui_spawn: run_config.suppress_ui_spawn,
            conn: None,
            tray: None,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            quit_tx: None,
        };
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
                let ok = check_source_available(backup).is_ok();
                self.source_availability.insert(backup.id.clone(), ok);
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
        let source_available = self
            .source_availability
            .get(&backup.id)
            .copied()
            .unwrap_or_else(|| check_source_available(backup).is_ok());
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
                reason_text: Some(i18n("Source unavailable").into()),
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

    /// Computes full state snapshot (`DaemonStateDto`) and synchronized `KesticTrayState`.
    pub fn build_state_and_tray(&self) -> (DaemonStateDto, KesticTrayState) {
        let (restic_status, secret_store_status, blocking) = self.probe_restic_and_secrets();
        let all_runs = self.history.list_runs().unwrap_or_default();

        let mut backup_summaries = Vec::with_capacity(self.config.backups.len());
        let mut dest_password_rejected: HashMap<DestinationId, bool> = HashMap::new();
        let mut dest_stale_lock: HashMap<DestinationId, bool> = HashMap::new();

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
                .find(|r| r.backup_id() == Some(&backup.id))
                .cloned();

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
                        if matches!(pres.kind, ResticFailureKind::RepositoryLocked) {
                            dest_stale_lock.insert(backup.destination_id.clone(), true);
                        }
                    }
                    RunStatus::Warning => {
                        error_summary = Some(localized_failure_summary(
                            &ResticFailureKind::IncompleteSnapshot {
                                unreadable_items: Vec::new(),
                            },
                        ));
                        error_action = Some("add_to_excludes".to_string());
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
                    }
                    _ => {}
                }
            }

            let availability = self.availability_for_backup(backup);

            backup_summaries.push(BackupSummaryDto {
                backup: backup.clone(),
                destination_name: dest_name,
                last_run,
                active_run,
                availability,
                has_error,
                is_overdue: false,
                error_action,
                error_summary,
                unreadable_files,
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
            let has_stale_lock = dest_stale_lock.get(&dest.id).copied().unwrap_or(false);
            let has_problem = !avail.reachable || password_rejected || has_stale_lock;

            dest_summaries.push(DestinationSummaryDto {
                destination: dest.clone(),
                backup_count,
                availability: avail,
                password_rejected,
                has_stale_lock,
                has_problem,
            });
        }

        let overall =
            self.compute_overall_status(&backup_summaries, &dest_summaries, blocking.as_ref());

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
        let tray_state = KesticTrayState {
            status: tray_status,
            icon_name: "kestic-symbolic".to_string(),
            icon_theme_path: self.icon_theme_path.clone(),
            overlay_icon_name: overall.overlay_icon.clone(),
            tooltip_title: "Kestic".to_string(),
            tooltip_description: overall.headline.clone(),
            status_line: overall.headline.clone(),
            running_run: overall.active_run_progress.clone(),
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
                b.last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Pending)
            })
            .count();

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
                .all(|b| matches!(b.backup.schedule, kestic_core::model::Schedule::ManualOnly));

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
            compute_tray_presentation(worst_kind, running_count > 0, false);

        let headline = if let Some(b) = blocking {
            b.message.clone()
        } else if failed_count > 0 {
            if failed_count == 1 {
                i18n("1 backup failed").into()
            } else {
                i18n("%1 backups failed").subs(&[&failed_count.to_string()])
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
            if failed_count > 0 {
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
            paused: false,
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
        Ok(())
    }

    pub fn add_backup(&mut self, backup_json: &str) -> Result<String, KesticServiceErr> {
        let mut backup: Backup = serde_json::from_str(backup_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid backup JSON: {e}")))?;
        if self.config.backup(&backup.id).is_some() {
            backup.id = BackupId::generate_unique(self.config.backups.iter().map(|b| &b.id))
                .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        }
        backup
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

        let new_id = backup.id.as_str().to_string();
        let mut next = self.config.clone();
        next.backups.push(backup);
        self.persist_config(next)?;
        Ok(new_id)
    }

    pub fn update_backup(&mut self, backup_json: &str) -> Result<(), KesticServiceErr> {
        let updated: Backup = serde_json::from_str(backup_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid backup JSON: {e}")))?;
        updated
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

        let idx = self
            .config
            .backups
            .iter()
            .position(|b| b.id == updated.id)
            .ok_or_else(|| KesticServiceErr::NotFound(updated.id.as_str().to_string()))?;

        let mut next = self.config.clone();
        next.backups[idx] = updated;
        self.persist_config(next)?;
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
        kestic_core::restic::RecoveryAction::UpdateStoredPassword => "update_password",
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

/// Starts a backup asynchronously if source & destination preflight pass; otherwise emits
/// `ManualRunBlocked` signal + optional notification (`AC-M1-09-6`, `AC-M1-09-7`, `AC-M1-23-1/2`).
pub fn start_backup_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
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
        run,
        cancel,
        conn_opt,
        tray_opt,
        enable_job_view,
        job_view_dest,
        enable_portal_inhibit,
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

        // Check source and destination availability right before launch (FR-30, P32, AC-M1-09-6).
        let source_ok = check_source_available(&backup).is_ok();
        let dest_res = if source_ok {
            preflight_check_destination(&dest, DEFAULT_CONNECT_TIMEOUT)
        } else {
            Ok(())
        };
        let now = guard.clock.now_utc();
        guard
            .source_availability
            .insert(backup.id.clone(), source_ok);
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
                let body = if reason == REASON_SOURCE_UNAVAILABLE {
                    i18n("Source unavailable").to_string()
                } else {
                    i18n("Destination unreachable").to_string()
                };
                let _ = send_event_notification(KesticEvent::ManualRunBlocked, &title, &body);
            }
            return Err(KesticServiceErr::ManualRunBlocked(reason.to_string()));
        }

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
            run,
            cancel,
            conn_opt,
            tray_opt,
            guard.enable_job_view,
            guard.job_view_dest.clone(),
            guard.enable_portal_inhibit,
        );
        drop(guard);

        if let Some(conn) = &params.9 {
            emit_run_updated(conn, &params.7);
            emit_status_changed(conn, &state_dto);
        }
        params
    };

    let run_id_str = run.id.as_str().to_string();
    let shared_worker = Arc::clone(shared);

    thread::spawn(move || {
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

        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &kestic_binary,
            secret_store: secret_store.as_ref(),
            clock: clock.as_ref(),
        };
        let exec_res = execute_backup(
            &backup,
            &dest,
            &state_dir,
            &runtime,
            Some(&cancel),
            &[],
            |ev| {
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
            },
        );
        drop(job_view_cancel_guard);

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

        let (final_status, exit_code, raw_detail, jv_code, jv_msg) = match exec_res {
            Ok(res) => {
                let status = res.presentation.status;
                let log_path = {
                    let guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
                    guard
                        .history
                        .write_redacted_log(&run_id, &res.log_lines, &secret_strs)
                        .ok()
                };
                run.commands = vec![res.command_record];
                run.stats = res.stats;
                run.log_path = log_path;
                let summary = match status {
                    RunStatus::Cancelled => Some(i18n("Cancelled by user").into()),
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
                };
                run.summary_message = summary.clone();
                let jv_err = match status {
                    RunStatus::Ok => (0, String::new()),
                    _ => (1, summary.clone().unwrap_or_default()),
                };
                (status, Some(res.exit_code), summary, jv_err.0, jv_err.1)
            }
            Err(err) => {
                let msg = redact_secrets(&err.to_string(), &secret_strs);
                let (status, exit_code) = match err {
                    BackupRunError::SourceUnavailable(_) | BackupRunError::Destination(_) => {
                        (RunStatus::Error, None)
                    }
                    _ => (RunStatus::Error, Some(1)),
                };
                run.summary_message = Some(msg.clone());
                (status, exit_code, Some(msg.clone()), 1, msg)
            }
        };

        run.finish(clock.as_ref(), final_status, exit_code);
        if let Some(jv) = job_view {
            let _ = jv.terminate(jv_code, &jv_msg);
        }

        let state_dto = {
            let mut guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
            guard.active_runs.remove(&run_id);
            let _ = guard.history.upsert_run(&run, &secret_strs);
            let _ = guard
                .history
                .prune_logs_for_backup(&backup.id, clock.as_ref());
            let (state_dto, tray_state) = guard.build_state_and_tray();
            if let Some(tray) = &tray_opt {
                tray.set_full_state(tray_state);
            }
            state_dto
        };

        // AC-M1-11-4: Notification ONLY on Error or Warning, never on Ok or Cancelled.
        match final_status {
            RunStatus::Error => {
                let summary = i18n("%1: Backup failed").subs(&[&backup.name]);
                let body = raw_detail.unwrap_or_else(|| i18n("Backup command failed").into());
                let _ = send_event_notification(KesticEvent::BackupFailed, &summary, &body);
            }
            RunStatus::Warning => {
                let summary = i18n("%1: Backup finished with warning").subs(&[&backup.name]);
                let body = raw_detail.unwrap_or_else(|| {
                    i18n("Snapshot incomplete: some files could not be read").into()
                });
                let _ = send_event_notification(KesticEvent::BackupWarning, &summary, &body);
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

pub fn start_all_backups_async(shared: &Arc<Mutex<DaemonCore>>) -> StartAllResultDto {
    let backup_ids: Vec<(String, String)> = {
        let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .config
            .backups
            .iter()
            .map(|b| (b.id.as_str().to_string(), b.name.clone()))
            .collect()
    };

    let mut started_run_ids = Vec::new();
    let mut skipped = Vec::new();
    for (id, name) in backup_ids {
        match start_backup_async(shared, &id) {
            Ok(run_id) => started_run_ids.push(run_id),
            Err(err) => {
                let (reason, reason_text) = match &err {
                    KesticServiceErr::ManualRunBlocked(r) if r == REASON_SOURCE_UNAVAILABLE => {
                        (r.clone(), i18n("Source unavailable").into())
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
                skipped.push(SkippedBackupDto {
                    backup_id: id,
                    backup_name: name,
                    reason,
                    reason_text,
                });
            }
        }
    }
    StartAllResultDto {
        started_run_ids,
        skipped,
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

    let deadline = config.duration.map(|d| Instant::now() + d);
    loop {
        while let Ok(action) = action_rx.try_recv() {
            handle_tray_action(&core, &conn, &config, action);
        }

        if core
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .shutdown_flag
            .load(Ordering::SeqCst)
        {
            break;
        }

        if let Some(end) = deadline {
            let now = Instant::now();
            if now >= end {
                break;
            }
            let wait = (end - now).min(Duration::from_millis(50));
            if quit_rx.recv_timeout(wait).is_ok() {
                break;
            }
        } else if quit_rx.recv_timeout(Duration::from_millis(50)).is_ok() {
            break;
        }
    }

    while let Ok(action) = action_rx.try_recv() {
        handle_tray_action(&core, &conn, &config, action);
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
    Ok(DaemonOutcome::RanAndStopped)
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
            let _ = start_all_backups_async(core);
        }
        TrayAction::CancelRun(run_id) => {
            let guard = core.lock().unwrap_or_else(|p| p.into_inner());
            let _ = guard.cancel_run(&run_id);
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
