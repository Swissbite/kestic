// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Versioned D-Bus API v1 (`net.daester.Kestic1`) shared DTOs, error types, and client proxy
//! (`F14`, `FR-30`, `M1-S09`, `docs/spec/dbus-api.md`).

use kestic_core::backup::{BackupDryRunResult, ForgetPreviewResult};
use kestic_core::config::AppConfig;
use kestic_core::model::{
    Backup, Destination, ExcludeLintWarning, ExcludePathTestResult, ExcludeRules,
    NestedSourceConflict, RetentionPolicy, Run, Source,
};
use serde::{Deserialize, Serialize};
use zbus::{Connection, DBusError, Proxy, block_on};

/// Well-known session bus name used as the daemon single-instance lock.
pub const KESTIC_BUS_NAME: &str = "net.daester.Kestic";

/// Primary object path exported by the Kestic daemon.
pub const KESTIC_OBJECT_PATH: &str = "/net/daester/Kestic";

/// Versioned v1 D-Bus interface name (`F14`).
pub const KESTIC_INTERFACE_V1: &str = "net.daester.Kestic1";

/// Machine-readable reason codes for `ManualRunBlocked` (`FR-30`, `AC-M1-09-6`).
pub const REASON_SOURCE_UNAVAILABLE: &str = "source_unavailable";
pub const REASON_DESTINATION_UNREACHABLE: &str = "destination_unreachable";

/// D-Bus error hierarchy under `net.daester.Kestic.Error.*`.
#[derive(DBusError, Debug, Clone)]
#[zbus(prefix = "net.daester.Kestic.Error")]
pub enum KesticDbusError {
    #[zbus(error)]
    ZBus(zbus::Error),
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

/// Aggregated overall status precedence (`tray-und-benachrichtigungen.md` §10.1, `zustaende.md` §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverallStatusKind {
    Blocked,
    Error,
    Overdue,
    Warning,
    Running,
    Paused,
    Pending,
    Interrupted,
    Cancelled,
    NeverRun,
    ManualOnly,
    Ok,
}

/// Restic binary probe state reported to GUI & Tray (`M1-S04`, `U19`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResticStatusDto {
    pub valid: bool,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    /// One of `"not_found"`, `"too_old"`, `"invalid_custom_path"`, `"unreadable_version"`.
    pub error_kind: Option<String>,
    pub found_version: Option<String>,
    pub message: String,
}

/// Secret store probe state reported to GUI (`U19`, `§6.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretStoreStatusDto {
    pub available: bool,
    pub locked: bool,
    pub message: Option<String>,
}

/// Blocking state rendered in the window header (`zustaende.md` §6.1, `AC-M1-13-5`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockingStateDto {
    /// One of `"secret_store_locked"`, `"restic_unavailable"`.
    pub kind: String,
    pub message: String,
}

/// Cached destination reachability check (`P32`, `M1-S23`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationAvailabilityDto {
    pub destination_id: String,
    pub reachable: bool,
    pub detail: Option<String>,
    pub checked_at_unix: i64,
}

/// Combined availability of a backup (`M1-S23`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupAvailabilityDto {
    pub backup_id: String,
    pub source_available: bool,
    pub destination_available: bool,
    pub available: bool,
    /// `"source_unavailable"` | `"destination_unreachable"` when `!available`.
    pub blocked_reason: Option<String>,
    pub reason_text: Option<String>,
}

/// Progress snapshot for an active run (`AC-M1-11-5`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveRunProgressDto {
    pub run_id: String,
    pub backup_id: Option<String>,
    pub backup_name: String,
    pub percent_done: f64,
    pub bytes_done: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
    pub seconds_remaining: u64,
}

/// Overall system + tray status summary (`§6.2`, `§10.1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverallStatusDto {
    pub kind: OverallStatusKind,
    /// `"Passive"` | `"Active"` | `"NeedsAttention"`
    pub tray_status: String,
    pub overlay_icon: String,
    pub headline: String,
    pub subtitle: String,
    pub failed_count: usize,
    pub warning_count: usize,
    pub overdue_count: usize,
    pub running_count: usize,
    pub waiting_count: usize,
    pub paused: bool,
    pub active_run_progress: Option<ActiveRunProgressDto>,
    pub backups_have_attention: bool,
    pub destinations_have_attention: bool,
}

/// Enriched backup item returned in `DaemonStateDto`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupSummaryDto {
    pub backup: Backup,
    pub destination_name: String,
    pub last_run: Option<Run>,
    pub active_run: Option<Run>,
    pub availability: BackupAvailabilityDto,
    pub has_error: bool,
    pub is_overdue: bool,
    pub error_action: Option<String>,
    pub error_summary: Option<String>,
    pub unreadable_files: Vec<String>,
    #[serde(default)]
    pub pending_forget: bool,
    #[serde(default)]
    pub nested_warnings: Vec<NestedSourceConflict>,
    #[serde(default)]
    pub exclude_lint_warnings: Vec<ExcludeLintWarning>,
}

/// Enriched destination item returned in `DaemonStateDto`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DestinationSummaryDto {
    pub destination: Destination,
    pub backup_count: usize,
    pub availability: DestinationAvailabilityDto,
    pub password_rejected: bool,
    pub has_stale_lock: bool,
    pub has_problem: bool,
}

/// Full daemon state snapshot returned by `GetState`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonStateDto {
    pub daemon_version: String,
    pub has_config_file: bool,
    pub config_load_error: Option<String>,
    pub config: AppConfig,
    pub restic_status: ResticStatusDto,
    pub secret_store_status: SecretStoreStatusDto,
    pub blocking: Option<BlockingStateDto>,
    pub overall: OverallStatusDto,
    pub backups: Vec<BackupSummaryDto>,
    pub destinations: Vec<DestinationSummaryDto>,
    pub recent_runs: Vec<Run>,
}

/// Detailed run view returned by `GetRunDetail` (`§7.8`, `M1-S08`, `M1-S14`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDetailDto {
    pub run: Run,
    pub backup_name: Option<String>,
    pub destination_name: String,
    pub log_lines: Vec<String>,
    pub transparent_commands: Vec<String>,
    pub error_summary: Option<String>,
    pub error_action: Option<String>,
    pub error_raw_detail: Option<String>,
    pub unreadable_files: Vec<String>,
}

/// Compact snapshot DTO (`AC-M1-22-4`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSummaryDto {
    pub id: String,
    pub short_id: String,
    pub time: String,
    pub hostname: String,
    pub paths: Vec<String>,
    pub tags: Vec<String>,
    pub total_bytes_processed: Option<u64>,
    pub total_files_processed: Option<u64>,
}

/// Earlier snapshots on a previously assigned, still configured destination (`AC-M1-22-4`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviousDestinationSnapshotsDto {
    pub destination_id: String,
    pub destination_name: String,
    pub snapshots: Vec<SnapshotSummaryDto>,
}

/// Result of `ListBackupSnapshots`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSnapshotsDto {
    pub backup_id: String,
    pub current_destination_id: String,
    pub current_destination_name: String,
    pub current_snapshots: Vec<SnapshotSummaryDto>,
    pub current_error: Option<String>,
    pub previous_destinations: Vec<PreviousDestinationSnapshotsDto>,
}

/// Result of `TestDestinationConnection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionTestResultDto {
    pub ok: bool,
    pub error_kind: Option<String>,
    pub message: String,
}

/// Skipped backup reason returned by `StartAllBackups` (`§7.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedBackupDto {
    pub backup_id: String,
    pub backup_name: String,
    pub reason: String,
    pub reason_text: String,
}

/// Summary returned by `StartAllBackups`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartAllResultDto {
    pub started_run_ids: Vec<String>,
    pub skipped: Vec<SkippedBackupDto>,
}

/// Result of `CheckAvailability`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailabilitySnapshotDto {
    pub checked_at_unix: Option<i64>,
    pub backups: Vec<BackupAvailabilityDto>,
    pub destinations: Vec<DestinationAvailabilityDto>,
}

/// Synchronous D-Bus client for `net.daester.Kestic1` used by the GUI and D-Bus integration tests.
#[derive(Clone)]
pub struct KesticClient {
    conn: Connection,
    bus_name: String,
    object_path: String,
}

impl KesticClient {
    /// Connects to the current D-Bus session bus targeting default `net.daester.Kestic`.
    pub fn connect_default() -> zbus::Result<Self> {
        Self::connect(KESTIC_BUS_NAME, KESTIC_OBJECT_PATH)
    }

    /// Connects targeting custom bus name and object path.
    pub fn connect(bus_name: &str, object_path: &str) -> zbus::Result<Self> {
        let conn = block_on(Connection::session())?;
        Ok(Self {
            conn,
            bus_name: bus_name.to_string(),
            object_path: object_path.to_string(),
        })
    }

    /// Wraps an already open `zbus::Connection` (used by persistent GUI signal listener).
    pub fn from_connection(conn: Connection, bus_name: &str, object_path: &str) -> Self {
        Self {
            conn,
            bus_name: bus_name.to_string(),
            object_path: object_path.to_string(),
        }
    }

    /// Returns underlying `zbus::Connection` for receiving signals.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Checks if the daemon well-known name currently has an owner on the session bus.
    pub fn is_bus_owned(&self) -> zbus::Result<bool> {
        let conn = self.conn.clone();
        let bus = self.bus_name.clone();
        block_on(async move {
            let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
            dbus.name_has_owner(bus.as_str().try_into()?).await
        })
        .map_err(Into::into)
    }

    async fn proxy(&self) -> zbus::Result<Proxy<'_>> {
        Proxy::new(
            &self.conn,
            self.bus_name.as_str(),
            self.object_path.as_str(),
            KESTIC_INTERFACE_V1,
        )
        .await
    }

    pub fn daemon_version(&self) -> zbus::Result<String> {
        block_on(async { self.proxy().await?.call("DaemonVersion", &()).await })
    }

    pub fn get_state(&self) -> Result<DaemonStateDto, String> {
        let raw: String = block_on(async { self.proxy().await?.call("GetState", &()).await })
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn get_run_detail(&self, run_id: &str) -> Result<RunDetailDto, String> {
        let raw: String =
            block_on(async { self.proxy().await?.call("GetRunDetail", &(run_id,)).await })
                .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn list_backup_snapshots(&self, backup_id: &str) -> Result<BackupSnapshotsDto, String> {
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call("ListBackupSnapshots", &(backup_id,))
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn test_destination_connection(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> Result<ConnectionTestResultDto, String> {
        let json = serde_json::to_string(dest).map_err(|e| e.to_string())?;
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call(
                    "TestDestinationConnection",
                    &(json.as_str(), mode, repo_password, rest_password),
                )
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn add_destination(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> zbus::Result<String> {
        let json = serde_json::to_string(dest)
            .map_err(|e| zbus::Error::Failure(format!("serialize dest: {e}")))?;
        block_on(async {
            self.proxy()
                .await?
                .call(
                    "AddDestination",
                    &(json.as_str(), mode, repo_password, rest_password),
                )
                .await
        })
    }

    pub fn update_destination(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
        verify_connection: bool,
    ) -> zbus::Result<()> {
        let json = serde_json::to_string(dest)
            .map_err(|e| zbus::Error::Failure(format!("serialize dest: {e}")))?;
        block_on(async {
            self.proxy()
                .await?
                .call(
                    "UpdateDestination",
                    &(
                        json.as_str(),
                        mode,
                        repo_password,
                        rest_password,
                        verify_connection,
                    ),
                )
                .await
        })
    }

    pub fn update_destination_password(
        &self,
        dest_id: &str,
        new_password: &str,
    ) -> zbus::Result<()> {
        block_on(async {
            self.proxy()
                .await?
                .call("UpdateDestinationPassword", &(dest_id, new_password))
                .await
        })
    }

    pub fn remove_destination(&self, dest_id: &str) -> zbus::Result<()> {
        block_on(async {
            self.proxy()
                .await?
                .call("RemoveDestination", &(dest_id,))
                .await
        })
    }

    pub fn add_backup(&self, backup: &Backup) -> zbus::Result<String> {
        let json = serde_json::to_string(backup)
            .map_err(|e| zbus::Error::Failure(format!("serialize backup: {e}")))?;
        block_on(async {
            self.proxy()
                .await?
                .call("AddBackup", &(json.as_str(),))
                .await
        })
    }

    pub fn update_backup(&self, backup: &Backup) -> zbus::Result<()> {
        let json = serde_json::to_string(backup)
            .map_err(|e| zbus::Error::Failure(format!("serialize backup: {e}")))?;
        block_on(async {
            self.proxy()
                .await?
                .call("UpdateBackup", &(json.as_str(),))
                .await
        })
    }

    pub fn rename_backup(&self, backup_id: &str, new_name: &str) -> zbus::Result<()> {
        block_on(async {
            self.proxy()
                .await?
                .call("RenameBackup", &(backup_id, new_name))
                .await
        })
    }

    pub fn reassign_backup(&self, backup_id: &str, new_dest_id: &str) -> zbus::Result<()> {
        block_on(async {
            self.proxy()
                .await?
                .call("ReassignBackup", &(backup_id, new_dest_id))
                .await
        })
    }

    pub fn remove_backup(&self, backup_id: &str) -> zbus::Result<()> {
        block_on(async {
            self.proxy()
                .await?
                .call("RemoveBackup", &(backup_id,))
                .await
        })
    }

    pub fn set_restic_provider(
        &self,
        provider: &kestic_core::model::ResticProvider,
    ) -> Result<ResticStatusDto, String> {
        let json = serde_json::to_string(provider).map_err(|e| e.to_string())?;
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call("SetResticProvider", &(json.as_str(),))
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn set_expert_mode(&self, enabled: bool) -> zbus::Result<()> {
        block_on(async { self.proxy().await?.call("SetExpertMode", &(enabled,)).await })
    }

    pub fn start_backup(&self, backup_id: &str) -> zbus::Result<String> {
        block_on(async { self.proxy().await?.call("StartBackup", &(backup_id,)).await })
    }

    pub fn start_all_backups(&self) -> Result<StartAllResultDto, String> {
        let raw: String =
            block_on(async { self.proxy().await?.call("StartAllBackups", &()).await })
                .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn cancel_run(&self, run_id: &str) -> zbus::Result<()> {
        block_on(async { self.proxy().await?.call("CancelRun", &(run_id,)).await })
    }

    pub fn check_availability(&self, force: bool) -> Result<AvailabilitySnapshotDto, String> {
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call("CheckAvailability", &(force,))
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn window_active(&self, active: bool, page: &str) -> zbus::Result<()> {
        block_on(async {
            self.proxy()
                .await?
                .call("WindowActive", &(active, page))
                .await
        })
    }

    pub fn register_gui(&self) -> zbus::Result<()> {
        block_on(async { self.proxy().await?.call("RegisterGui", &()).await })
    }

    pub fn confirm_quit(&self) -> zbus::Result<()> {
        block_on(async { self.proxy().await?.call("ConfirmQuit", &()).await })
    }

    pub fn unlock_secret_store(&self) -> zbus::Result<bool> {
        block_on(async { self.proxy().await?.call("UnlockSecretStore", &()).await })
    }

    pub fn detect_source(&self, path: &str) -> Result<Source, String> {
        let raw: String =
            block_on(async { self.proxy().await?.call("DetectSource", &(path,)).await })
                .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn check_nested_sources(
        &self,
        candidate_id: &str,
        candidate_path: &str,
        destination_id: &str,
    ) -> Result<Vec<NestedSourceConflict>, String> {
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call(
                    "CheckNestedSources",
                    &(candidate_id, candidate_path, destination_id),
                )
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn lint_excludes(
        &self,
        excludes: &ExcludeRules,
    ) -> Result<Vec<ExcludeLintWarning>, String> {
        let json = serde_json::to_string(excludes).map_err(|e| e.to_string())?;
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call("LintExcludes", &(json.as_str(),))
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn test_exclude_path(
        &self,
        backup_id: &str,
        excludes: &ExcludeRules,
        test_path: &str,
    ) -> Result<ExcludePathTestResult, String> {
        let json = serde_json::to_string(excludes).map_err(|e| e.to_string())?;
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call("TestExcludePath", &(backup_id, json.as_str(), test_path))
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn preview_retention(
        &self,
        backup_id: &str,
        retention: &RetentionPolicy,
    ) -> Result<ForgetPreviewResult, String> {
        let json = serde_json::to_string(retention).map_err(|e| e.to_string())?;
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call("PreviewRetention", &(backup_id, json.as_str()))
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn start_retention_preview(
        &self,
        backup_id: &str,
        retention: &RetentionPolicy,
    ) -> zbus::Result<String> {
        let json = serde_json::to_string(retention)
            .map_err(|e| zbus::Error::Failure(format!("serialize retention: {e}")))?;
        block_on(async {
            self.proxy()
                .await?
                .call("StartRetentionPreview", &(backup_id, json.as_str()))
                .await
        })
    }

    pub fn cancel_retention_preview(&self, preview_id: &str) -> zbus::Result<()> {
        block_on(async {
            self.proxy()
                .await?
                .call("CancelRetentionPreview", &(preview_id,))
                .await
        })
    }

    pub fn get_retention_preview_status(
        &self,
        preview_id: &str,
    ) -> Result<RetentionPreviewStatusDto, String> {
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call("GetRetentionPreviewStatus", &(preview_id,))
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn start_backup_dry_run(
        &self,
        backup_id: &str,
        excludes: Option<&ExcludeRules>,
    ) -> zbus::Result<String> {
        let json = match excludes {
            Some(ex) => serde_json::to_string(ex)
                .map_err(|e| zbus::Error::Failure(format!("serialize excludes: {e}")))?,
            None => String::new(),
        };
        block_on(async {
            self.proxy()
                .await?
                .call("StartBackupDryRun", &(backup_id, json.as_str()))
                .await
        })
    }

    pub fn cancel_backup_dry_run(&self, preview_id: &str) -> zbus::Result<()> {
        block_on(async {
            self.proxy()
                .await?
                .call("CancelBackupDryRun", &(preview_id,))
                .await
        })
    }

    pub fn get_backup_dry_run_status(
        &self,
        preview_id: &str,
    ) -> Result<BackupDryRunStatusDto, String> {
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call("GetBackupDryRunStatus", &(preview_id,))
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn format_exclude_rule_for_item(
        &self,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, String> {
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call("FormatExcludeRuleForItem", &(item_path,))
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn add_exclude_rule_for_item(
        &self,
        backup_id: &str,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, String> {
        let raw: String = block_on(async {
            self.proxy()
                .await?
                .call("AddExcludeRuleForItem", &(backup_id, item_path))
                .await
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn run_destination_maintenance(&self, destination_id: &str) -> zbus::Result<String> {
        block_on(async {
            self.proxy()
                .await?
                .call("RunDestinationMaintenance", &(destination_id,))
                .await
        })
    }
}

/// Asynchronous dry-run preview status returned by `GetBackupDryRunStatus` (`M2-S04`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupDryRunStatusDto {
    pub preview_id: String,
    /// `"running"` | `"done"` | `"cancelled"` | `"error"`
    pub state: String,
    pub result: Option<BackupDryRunResult>,
    pub error: Option<String>,
}

/// Asynchronous retention preview status returned by `GetRetentionPreviewStatus` (`M2-S06`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPreviewStatusDto {
    pub preview_id: String,
    /// `"running"` | `"done"` | `"cancelled"` | `"error"`
    pub state: String,
    pub result: Option<ForgetPreviewResult>,
    pub error: Option<String>,
}

/// Preview of the exclude rule generated for an unreadable path (`M2-S08`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludeRulePreviewDto {
    pub is_dir: bool,
    pub rule: String,
    pub is_expert_rule: bool,
}
