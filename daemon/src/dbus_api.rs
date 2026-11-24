// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Versioned D-Bus API v1 (`net.daester.Kestic1`) shared DTOs, error types, and client proxy
//! (`docs/spec/dbus-api.md`).

use kestic_core::backup::{BackupDryRunResult, ForgetPreviewResult};
use kestic_core::config::AppConfig;
use kestic_core::maintenance::PrunePreviewSummary;
use kestic_core::model::{
    Backup, Destination, DestinationId, ExcludeLintWarning, ExcludePathTestResult, ExcludeRules,
    MaintenanceSchedule, NestedSourceConflict, NotificationSettings, RetentionPolicy, Run, Source,
};
use kestic_core::provider::MIN_RESTIC_VERSION;
pub use kestic_core::query_list::{PageFilter, PageRequest as SnapshotQueryPage};
use kestic_core::restic::RecoveryAction;
pub use kestic_core::restore::RestoreTarget;
pub use kestic_core::restore_preview::RestorePreviewEntry as RestorePreviewEntryDto;
pub use kestic_core::restore_target::OriginalLocation as RestoreOriginalDto;
use kestic_core::scheduler::PauseDuration;
use kestic_core::snapshot::ResticSnapshot;
pub use kestic_core::snapshot_diff::DiffChange as DiffChangeDto;
pub use kestic_core::snapshot_find::{FindHit as FindHitDto, FindScope};
pub use kestic_core::snapshot_ls::LsEntry as LsEntryDto;
use kestic_i18n::codes::{
    BlockedReason, BlockingKind, ConnectionTestCode, ErrorKind, PendingReasonCode,
    ResticStatusCode, ServiceErrorCode, SkipReason, SourceStatus,
};
pub use kestic_i18n::codes::{
    OverallStatusKind, RestoreNoteCode, RestoreOutcome, RestoreOverwrite, RestoreTargetCode,
    RunKindCode, RunProgressPhase, SnapshotChange, SnapshotQueryKind,
};
use kestic_i18n::messages::{
    ActiveRunInfo, Message, OverallCounts, ResticStatusParams, UiText, restic_status_text,
};

use crate::tray::TrayStatus;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use zbus::zvariant::{DynamicDeserialize, DynamicType};
use zbus::{Connection, DBusError, Proxy, block_on};

/// Well-known session bus name used as the daemon single-instance lock.
pub const KESTIC_BUS_NAME: &str = "net.daester.Kestic";

/// Primary object path exported by the Kestic daemon.
pub const KESTIC_OBJECT_PATH: &str = "/net/daester/Kestic";

/// Versioned v1 D-Bus interface name.
pub const KESTIC_INTERFACE_V1: &str = "net.daester.Kestic1";

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
    LockInUse(String),
    DuplicateLocation(String),
    PreviewRequired(String),
    RestoreRunning(String),
    Internal(String),
}

impl KesticDbusError {
    /// Code of the error (its D-Bus name); the message is raw detail only (`dbus-api.md`).
    pub fn code(&self) -> ServiceErrorCode {
        match self {
            Self::ZBus(_) => ServiceErrorCode::DaemonUnreachable,
            Self::InvalidInput(_) => ServiceErrorCode::InvalidInput,
            Self::NotFound(_) => ServiceErrorCode::NotFound,
            Self::InUse(_) => ServiceErrorCode::InUse,
            Self::ManualRunBlocked(_) => ServiceErrorCode::ManualRunBlocked,
            Self::DestinationUnreachable(_) => ServiceErrorCode::DestinationUnreachable,
            Self::WrongPassword(_) => ServiceErrorCode::WrongPassword,
            Self::AuthFailed(_) => ServiceErrorCode::AuthFailed,
            Self::SecretStoreLocked(_) => ServiceErrorCode::SecretStoreLocked,
            Self::ResticUnavailable(_) => ServiceErrorCode::ResticUnavailable,
            Self::AlreadyRunning(_) => ServiceErrorCode::AlreadyRunning,
            Self::LockInUse(_) => ServiceErrorCode::LockInUse,
            Self::DuplicateLocation(_) => ServiceErrorCode::DuplicateLocation,
            Self::PreviewRequired(_) => ServiceErrorCode::PreviewRequired,
            Self::RestoreRunning(_) => ServiceErrorCode::RestoreRunning,
            Self::Internal(_) => ServiceErrorCode::Internal,
        }
    }

    /// Blocked reason of a `ManualRunBlocked` error (its message is the reason code).
    pub fn blocked_reason(&self) -> Option<BlockedReason> {
        match self {
            Self::ManualRunBlocked(code) => BlockedReason::parse(code),
            _ => None,
        }
    }
}

/// Restic binary probe state reported to GUI & Tray.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResticStatusDto {
    pub valid: bool,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    /// Why restic is unusable (`None` while `valid`).
    pub error_kind: Option<ResticStatusCode>,
    pub found_version: Option<String>,
    /// Raw reason for `invalid_custom_path` / detail for `unreadable_version`.
    #[serde(default)]
    pub detail: Option<String>,
}

/// Secret store probe state reported to GUI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretStoreStatusDto {
    pub available: bool,
    pub locked: bool,
    /// Raw error of the password store (not translated).
    pub detail: Option<String>,
}

/// Blocking state rendered in the window header (`docs/spec/ui/zustaende.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockingStateDto {
    /// Parameters for `restic_unavailable` come from `ResticStatusDto`.
    pub kind: BlockingKind,
}

/// Cached destination reachability check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationAvailabilityDto {
    pub destination_id: String,
    pub reachable: bool,
    pub detail: Option<String>,
    pub checked_at_unix: i64,
}

impl DestinationAvailabilityDto {
    /// Reachability of `destination_id` from the outcome of a preflight check.
    pub fn from_probe<E: std::fmt::Display>(
        destination_id: &DestinationId,
        probe: &Result<(), E>,
        checked_at_unix: i64,
    ) -> Self {
        Self {
            destination_id: destination_id.as_str().to_string(),
            reachable: probe.is_ok(),
            detail: probe.as_ref().err().map(ToString::to_string),
            checked_at_unix,
        }
    }
}

/// Combined availability of a backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupAvailabilityDto {
    pub backup_id: String,
    pub source_available: bool,
    pub destination_available: bool,
    pub available: bool,
    /// `source_unavailable` | `destination_unreachable` when `!available`.
    pub blocked_reason: Option<BlockedReason>,
    /// Source check code when the source is unavailable.
    #[serde(default)]
    pub source_status: Option<SourceStatus>,
}

/// Progress snapshot for an active run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveRunProgressDto {
    pub run_id: String,
    /// Backup, maintenance or restore.
    #[serde(default)]
    pub kind: RunKindCode,
    pub backup_id: Option<String>,
    pub backup_name: String,
    /// `scanning` until restic knows the total, then `scan_finished`.
    #[serde(default)]
    pub phase: RunProgressPhase,
    /// 0–100; 0 while scanning, never falls afterwards.
    pub percent_done: f64,
    pub bytes_done: u64,
    /// 0 while scanning.
    pub total_bytes: u64,
    #[serde(default)]
    pub files_done: u64,
    pub speed_bps: u64,
    /// 0 = unknown.
    pub seconds_remaining: u64,
}

/// Overall system + tray status summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverallStatusDto {
    pub kind: OverallStatusKind,
    pub tray_status: TrayStatus,
    pub overlay_icon: String,
    pub failed_count: usize,
    pub warning_count: usize,
    pub overdue_count: usize,
    pub running_count: usize,
    pub waiting_count: usize,
    pub paused: bool,
    #[serde(default)]
    pub now_unix: i64,
    #[serde(default)]
    pub paused_since_unix: Option<i64>,
    #[serde(default)]
    pub paused_until_unix: Option<i64>,
    #[serde(default)]
    pub next_scheduled_backup_name: Option<String>,
    #[serde(default)]
    pub next_scheduled_at_unix: Option<i64>,
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
    pub error_action: Option<RecoveryAction>,
    /// Cause of the last run's error or warning.
    #[serde(default)]
    pub error_kind: Option<ErrorKind>,
    /// Raw restic/daemon text of the last run's error (detail for `error_kind`).
    #[serde(default)]
    pub error_detail: Option<String>,
    /// restic exit code of the last run's error, if restic ran.
    #[serde(default)]
    pub error_exit_code: Option<i32>,
    pub unreadable_files: Vec<String>,
    #[serde(default)]
    pub pending_forget: bool,
    #[serde(default)]
    pub nested_warnings: Vec<NestedSourceConflict>,
    #[serde(default)]
    pub exclude_lint_warnings: Vec<ExcludeLintWarning>,
    #[serde(default)]
    pub is_paused: bool,
    #[serde(default)]
    pub paused_since_unix: Option<i64>,
    #[serde(default)]
    pub paused_until_unix: Option<i64>,
    #[serde(default)]
    pub next_scheduled_at_unix: Option<i64>,
    /// Why the due backup waits (first of `pending_reason_codes`).
    #[serde(default)]
    pub pending_primary_code: Option<PendingReasonCode>,
    #[serde(default)]
    pub pending_reason_codes: Vec<PendingReasonCode>,
    /// Next retry time while a `backoff` reason is listed.
    #[serde(default)]
    pub pending_retry_at_unix: Option<i64>,
    #[serde(default)]
    pub missed_slots_count: usize,
    #[serde(default)]
    pub has_unmet_run_condition: bool,
    /// Run conditions not met right now (`on_battery` | `no_network` | `metered` |
    /// `wrong_network`); the allowed networks come from the backup's run conditions.
    #[serde(default)]
    pub unmet_run_conditions: Vec<PendingReasonCode>,
    /// Battery level while `on_battery` is listed.
    #[serde(default)]
    pub battery_percent: Option<u8>,
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
    #[serde(default)]
    pub next_prune_at_unix: Option<i64>,
    #[serde(default)]
    pub next_check_at_unix: Option<i64>,
    #[serde(default)]
    pub last_prune_at_unix: Option<i64>,
    #[serde(default)]
    pub last_check_at_unix: Option<i64>,
    #[serde(default)]
    pub last_maintenance_run: Option<Run>,
    #[serde(default)]
    pub maintenance_running: bool,
    #[serde(default)]
    pub pending_forget_count: usize,
    #[serde(default)]
    pub stale_lock_count: usize,
    /// Why due maintenance waits («pending»): `destination_unreachable` |
    /// `on_battery` | `metered` | `queue` | `backoff` (same codes as backup pending reasons).
    #[serde(default)]
    pub maintenance_pending_reason: Option<PendingReasonCode>,
    /// Next maintenance retry while `maintenance_pending_reason` is `backoff`.
    #[serde(default)]
    pub maintenance_retry_at_unix: Option<i64>,
}

/// Full daemon state snapshot returned by `GetState`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonStateDto {
    pub daemon_version: String,
    /// Build commit of the daemon binary (see [`crate::build_commit`]). Empty from a daemon that
    /// does not report it yet.
    #[serde(default)]
    pub daemon_build_commit: String,
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

/// Detailed run view returned by `GetRunDetail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDetailDto {
    pub run: Run,
    pub backup_name: Option<String>,
    pub destination_name: String,
    pub log_lines: Vec<String>,
    pub transparent_commands: Vec<String>,
    pub error_action: Option<RecoveryAction>,
    pub error_raw_detail: Option<String>,
    pub unreadable_files: Vec<String>,
    /// Cause of the error (see [`BackupSummaryDto::error_kind`]); detail: `error_raw_detail`.
    #[serde(default)]
    pub error_kind: Option<ErrorKind>,
    #[serde(default)]
    pub error_exit_code: Option<i32>,
}

/// Compact snapshot DTO.
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
    /// Data newly written by the snapshot; `null` without it in the summary.
    #[serde(default)]
    pub data_added: Option<u64>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub uid: Option<u32>,
}

impl From<ResticSnapshot> for SnapshotSummaryDto {
    fn from(s: ResticSnapshot) -> Self {
        let summary = s.summary.as_ref();
        Self {
            short_id: s.short_id.unwrap_or_else(|| s.id.chars().take(8).collect()),
            total_bytes_processed: summary.and_then(|x| x.total_bytes_processed),
            total_files_processed: summary.and_then(|x| x.total_files_processed),
            data_added: summary.and_then(|x| x.data_added),
            id: s.id,
            time: s.time,
            hostname: s.hostname.unwrap_or_default(),
            paths: s.paths,
            tags: s.tags,
            username: s.username,
            uid: s.uid,
        }
    }
}

/// Number of snapshots of one configured backup in a destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSnapshotCountDto {
    pub backup_id: String,
    pub backup_name: String,
    pub count: u64,
    /// Time of the newest snapshot; `null` without snapshots.
    pub latest_time: Option<String>,
}

/// Snapshots of a backup ID that is neither configured nor described by a metadata snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrphanedBackupDto {
    pub backup_id: String,
    pub count: u64,
    pub latest_time: Option<String>,
    pub latest_hostname: Option<String>,
    pub snapshots: Vec<SnapshotSummaryDto>,
}

/// Snapshots of a backup ID that is not configured here but described by a metadata snapshot of
/// the destination: the backup can still be taken over, so they are not orphaned. Same fields
/// as an orphaned backup.
pub type DescribedBackupDto = OrphanedBackupDto;

/// Result of `ListDestinationSnapshots`: all snapshots of a destination, sorted, newest first,
/// without metadata snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationSnapshotsDto {
    pub destination_id: String,
    /// Why the snapshots could not be listed; detail: `error_detail`.
    pub error_kind: Option<ErrorKind>,
    pub error_detail: Option<String>,
    pub backups: Vec<BackupSnapshotCountDto>,
    pub unassigned: Vec<SnapshotSummaryDto>,
    pub orphaned: Vec<OrphanedBackupDto>,
    #[serde(default)]
    pub described: Vec<DescribedBackupDto>,
}

/// Earlier snapshots on a previously assigned, still configured destination.
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
    pub current_destination_name: String,
    pub current_snapshots: Vec<SnapshotSummaryDto>,
    pub current_error: Option<String>,
    /// Error code of `current_error`.
    #[serde(default)]
    pub current_error_kind: Option<ErrorKind>,
    pub previous_destinations: Vec<PreviousDestinationSnapshotsDto>,
}

/// Request of `StartSnapshotQuery`: the kind, the destination and the fields of that kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotQueryRequest {
    pub kind: SnapshotQueryKind,
    pub destination_id: String,
    /// `ls`: the full snapshot ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    /// `ls`: the absolute folder to list (one level, never recursive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    /// `find`: the text to search for in the paths, taken literally, any case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// `find`: the snapshots to search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<FindScope>,
    /// `diff`: the full ID of the snapshot compared from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_snapshot_id: Option<String>,
    /// `diff`: the full ID of the snapshot compared to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_snapshot_id: Option<String>,
    /// `restore_preview`: absolute snapshot paths to restore; empty = everything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Vec<String>>,
    /// `restore_preview`: the target; without it the daemon takes the suggested new folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<RestoreTarget>,
    /// `restore_preview`: what happens to existing files (default `never`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overwrite: Option<RestoreOverwrite>,
}

impl SnapshotQueryRequest {
    /// Lists `dir` of snapshot `snapshot_id` (one level).
    pub fn ls(destination_id: &str, snapshot_id: &str, dir: &str) -> Self {
        Self {
            kind: SnapshotQueryKind::Ls,
            destination_id: destination_id.to_string(),
            snapshot_id: Some(snapshot_id.to_string()),
            dir: Some(dir.to_string()),
            pattern: None,
            scope: None,
            from_snapshot_id: None,
            to_snapshot_id: None,
            selection: None,
            target: None,
            overwrite: None,
        }
    }

    /// Searches the snapshots of `scope` for paths that contain `text`.
    pub fn find(destination_id: &str, scope: FindScope, text: &str) -> Self {
        Self {
            kind: SnapshotQueryKind::Find,
            destination_id: destination_id.to_string(),
            snapshot_id: None,
            dir: None,
            pattern: Some(text.to_string()),
            scope: Some(scope),
            from_snapshot_id: None,
            to_snapshot_id: None,
            selection: None,
            target: None,
            overwrite: None,
        }
    }

    /// Compares snapshot `from` with snapshot `to`.
    pub fn diff(destination_id: &str, from: &str, to: &str) -> Self {
        Self {
            kind: SnapshotQueryKind::Diff,
            destination_id: destination_id.to_string(),
            snapshot_id: None,
            dir: None,
            pattern: None,
            scope: None,
            from_snapshot_id: Some(from.to_string()),
            to_snapshot_id: Some(to.to_string()),
            selection: None,
            target: None,
            overwrite: None,
        }
    }

    /// Previews restoring `selection` of `snapshot_id` into `target` (`None` = the suggested
    /// new folder).
    pub fn restore_preview(
        destination_id: &str,
        snapshot_id: &str,
        selection: &[String],
        target: Option<RestoreTarget>,
        overwrite: RestoreOverwrite,
    ) -> Self {
        Self {
            kind: SnapshotQueryKind::RestorePreview,
            destination_id: destination_id.to_string(),
            snapshot_id: Some(snapshot_id.to_string()),
            dir: None,
            pattern: None,
            scope: None,
            from_snapshot_id: None,
            to_snapshot_id: None,
            selection: Some(selection.to_vec()),
            target,
            overwrite: Some(overwrite),
        }
    }
}

/// A page of a query list (`ListPageDto`): the page, its offset, all entries of the query,
/// those matching the filter, and whether the list was too large to hold.
pub type ListPageDto<T> = kestic_core::query_list::ListPage<T>;

/// Result of an `ls` query: one folder level of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsResultDto {
    pub dir: String,
    /// From the first node; restic lists nothing for a missing folder.
    pub dir_exists: bool,
    pub entries: ListPageDto<LsEntryDto>,
}

/// Result of a `find` query: the hits, newest snapshot first and by path; `total` 0 means «no
/// hits».
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindResultDto {
    pub hits: ListPageDto<FindHitDto>,
    /// Number of snapshots with at least one hit.
    pub snapshots_matched: u64,
}

/// Result of a `diff` query: the changed paths by path, and the counts of the whole comparison
/// (independent of the filter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffResultDto {
    pub changes: ListPageDto<DiffChangeDto>,
    pub added_files: u64,
    pub removed_files: u64,
    pub changed_files: u64,
    pub bytes_added: u64,
    pub bytes_removed: u64,
    /// restic reported no change («no differences»).
    pub identical: bool,
}

/// What the daemon suggests as target of a restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreSuggestionDto {
    /// The suggested new folder (`~/Restored/<backup> <time>`).
    pub new_folder: String,
    pub original: RestoreOriginalDto,
}

/// The daemon's check of a restore target; `detail` is the resolved folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreTargetCheckDto {
    pub code: RestoreTargetCode,
    pub detail: String,
}

/// The target a preview used, with its check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePreviewTargetDto {
    #[serde(flatten)]
    pub target: RestoreTarget,
    pub check: RestoreTargetCheckDto,
}

/// Result of a `restore_preview` query. At the default target (a new, empty folder) no dry run
/// runs: the counts and `entries` stay empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePreviewResultDto {
    pub suggestion: RestoreSuggestionDto,
    pub target: RestorePreviewTargetDto,
    pub new_files: u64,
    pub overwritten_files: u64,
    pub kept_files: u64,
    pub identical_files: u64,
    pub bytes_to_write: u64,
    pub entries: ListPageDto<RestorePreviewEntryDto>,
    pub notes: Vec<RestoreNoteCode>,
}

/// Request of `StartRestore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreRequest {
    pub destination_id: String,
    pub snapshot_id: String,
    /// Absolute snapshot paths; empty = everything.
    #[serde(default)]
    pub selection: Vec<String>,
    pub target: RestoreTarget,
    #[serde(default)]
    pub overwrite: RestoreOverwrite,
    /// `query_id` of a finished `restore_preview` with the same parameters.
    #[serde(default)]
    pub preview_id: Option<String>,
}

/// Result of a finished snapshot query, tagged with its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SnapshotQueryResultDto {
    Ls(LsResultDto),
    Find(FindResultDto),
    Diff(DiffResultDto),
    RestorePreview(Box<RestorePreviewResultDto>),
}

/// Status of a snapshot query (`GetSnapshotQueryStatus`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotQueryStatusDto {
    /// `"running"` | `"done"` | `"cancelled"` | `"error"`
    pub state: String,
    /// Cause while `state == "error"`.
    pub error_kind: Option<ErrorKind>,
    /// Raw error text (shown for `unknown`).
    pub error_detail: Option<String>,
    /// Only once the query is done.
    pub result: Option<SnapshotQueryResultDto>,
}

/// Result of `TestDestinationConnection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionTestResultDto {
    pub ok: bool,
    pub error_kind: Option<ConnectionTestCode>,
    /// Raw error detail, or the parameter of the code's text (e.g. the destination name for
    /// `duplicate_location`).
    pub detail: String,
}

/// Skipped backup reason returned by `StartAllBackups`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedBackupDto {
    pub backup_id: String,
    pub backup_name: String,
    pub reason: SkipReason,
    /// Cause for `permanent_error` (last run's `error_kind`) and `error` (`secret_store_locked`
    /// | `restic_unavailable` | `auth_failed` | `wrong_password` | `destination_unreachable` |
    /// `unknown`).
    #[serde(default)]
    pub error_kind: Option<ErrorKind>,
    /// Raw start error for `reason == error` (shown when `error_kind` is `unknown`).
    #[serde(default)]
    pub error_detail: Option<String>,
}

/// Summary returned by `StartAllBackups`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartAllResultDto {
    pub started_run_ids: Vec<String>,
    pub skipped: Vec<SkippedBackupDto>,
    #[serde(default)]
    pub queued_backup_ids: Vec<String>,
}

/// Result of `CheckAvailability`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailabilitySnapshotDto {
    pub checked_at_unix: Option<i64>,
    pub backups: Vec<BackupAvailabilityDto>,
    pub destinations: Vec<DestinationAvailabilityDto>,
}

impl ActiveRunProgressDto {
    /// The active run as input for the shared texts in `kestic_i18n::messages` (a run without
    /// backup id is destination maintenance).
    pub fn info(&self) -> ActiveRunInfo<'_> {
        ActiveRunInfo {
            name: &self.backup_name,
            kind: self.kind,
            phase: self.phase,
            percent_done: self.percent_done,
        }
    }
}

impl OverallStatusDto {
    /// Counts behind the headline (`kestic_i18n::messages::overall_headline`).
    pub fn counts(&self, backups: usize) -> OverallCounts {
        OverallCounts {
            kind: self.kind,
            failed: self.failed_count,
            overdue: self.overdue_count,
            warning: self.warning_count,
            waiting: self.waiting_count,
            backups,
            paused: self.paused,
        }
    }

    /// The active run, if any, for headline and subtitle.
    pub fn active_run_info(&self) -> Option<ActiveRunInfo<'_>> {
        self.active_run_progress
            .as_ref()
            .map(ActiveRunProgressDto::info)
    }
}

impl ResticStatusDto {
    /// Parameters of the restic status texts; `min_version` is the version Kestic needs.
    fn text_params<'a>(&'a self, min_version: &'a str) -> ResticStatusParams<'a> {
        ResticStatusParams {
            binary_path: self.binary_path.as_deref(),
            found_version: self.found_version.as_deref(),
            detail: self.detail.as_deref(),
            min_version,
        }
    }

    /// Why restic is unusable, `None` while it is usable.
    pub fn error_message(&self) -> Option<Message> {
        let min = MIN_RESTIC_VERSION.to_string();
        self.error_kind
            .map(|code| code.message(&self.text_params(&min)))
    }

    /// Status line: «restic ‹version›» or the reason restic is unusable.
    pub fn status_text(&self) -> UiText {
        let min = MIN_RESTIC_VERSION.to_string();
        restic_status_text(
            self.error_kind,
            self.version.as_deref(),
            &self.text_params(&min),
        )
    }
}

/// Blocking-state text from `BlockingStateDto.kind` and the `ResticStatusDto` parameters.
pub fn blocking_message(
    blocking: Option<&BlockingStateDto>,
    restic: &ResticStatusDto,
) -> Option<Message> {
    blocking.map(|b| b.kind.message(restic.error_message()))
}

/// Synchronous D-Bus client for `net.daester.Kestic1` used by the GUI and D-Bus integration tests.
#[derive(Clone)]
pub struct KesticClient {
    conn: Connection,
    bus_name: String,
    object_path: String,
}

impl KesticClient {
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

    /// Calls `method` on the daemon interface and blocks for the reply.
    fn call<B, R>(&self, method: &'static str, body: &B) -> zbus::Result<R>
    where
        B: Serialize + DynamicType,
        R: for<'d> DynamicDeserialize<'d>,
    {
        block_on(async { self.proxy().await?.call(method, body).await })
    }

    /// Calls a method that replies with a JSON string and parses the reply into `T`.
    fn call_json<B, T>(&self, method: &'static str, body: &B) -> Result<T, KesticDbusError>
    where
        B: Serialize + DynamicType,
        T: DeserializeOwned,
    {
        let raw: String = self.call(method, body).map_err(KesticDbusError::from)?;
        serde_json::from_str(&raw).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    pub fn daemon_version(&self) -> zbus::Result<String> {
        self.call("DaemonVersion", &())
    }

    pub fn get_state(&self) -> Result<DaemonStateDto, KesticDbusError> {
        self.call_json("GetState", &())
    }

    pub fn get_run_detail(&self, run_id: &str) -> Result<RunDetailDto, KesticDbusError> {
        self.call_json("GetRunDetail", &(run_id,))
    }

    pub fn list_backup_snapshots(
        &self,
        backup_id: &str,
    ) -> Result<BackupSnapshotsDto, KesticDbusError> {
        self.call_json("ListBackupSnapshots", &(backup_id,))
    }

    pub fn list_destination_snapshots(
        &self,
        destination_id: &str,
    ) -> Result<DestinationSnapshotsDto, KesticDbusError> {
        self.call_json("ListDestinationSnapshots", &(destination_id,))
    }

    pub fn start_snapshot_query(
        &self,
        request: &SnapshotQueryRequest,
    ) -> Result<String, KesticDbusError> {
        let json = to_json_arg(request)?;
        self.call("StartSnapshotQuery", &(json.as_str(),))
            .map_err(KesticDbusError::from)
    }

    pub fn get_snapshot_query_status(
        &self,
        query_id: &str,
        page: &SnapshotQueryPage,
    ) -> Result<SnapshotQueryStatusDto, KesticDbusError> {
        let json = to_json_arg(page)?;
        self.call_json("GetSnapshotQueryStatus", &(query_id, json.as_str()))
    }

    pub fn cancel_snapshot_query(&self, query_id: &str) -> zbus::Result<()> {
        self.call("CancelSnapshotQuery", &(query_id,))
    }

    /// Starts a restore run; returns its run ID.
    pub fn start_restore(&self, request: &RestoreRequest) -> Result<String, KesticDbusError> {
        let json = to_json_arg(request)?;
        self.call("StartRestore", &(json.as_str(),))
            .map_err(KesticDbusError::from)
    }

    pub fn test_destination_connection(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> Result<ConnectionTestResultDto, KesticDbusError> {
        let json = to_json_arg(dest)?;
        self.call_json(
            "TestDestinationConnection",
            &(json.as_str(), mode, repo_password, rest_password),
        )
    }

    pub fn add_destination(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> zbus::Result<String> {
        let json = to_failure_arg("dest", dest)?;
        self.call(
            "AddDestination",
            &(json.as_str(), mode, repo_password, rest_password),
        )
    }

    pub fn update_destination(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
        verify_connection: bool,
    ) -> zbus::Result<()> {
        let json = to_failure_arg("dest", dest)?;
        self.call(
            "UpdateDestination",
            &(
                json.as_str(),
                mode,
                repo_password,
                rest_password,
                verify_connection,
            ),
        )
    }

    pub fn update_destination_password(
        &self,
        dest_id: &str,
        new_password: &str,
    ) -> zbus::Result<()> {
        self.call("UpdateDestinationPassword", &(dest_id, new_password))
    }

    pub fn remove_destination(&self, dest_id: &str) -> zbus::Result<()> {
        self.call("RemoveDestination", &(dest_id,))
    }

    pub fn add_backup(&self, backup: &Backup) -> zbus::Result<String> {
        let json = to_failure_arg("backup", backup)?;
        self.call("AddBackup", &(json.as_str(),))
    }

    pub fn update_backup(&self, backup: &Backup) -> zbus::Result<()> {
        let json = to_failure_arg("backup", backup)?;
        self.call("UpdateBackup", &(json.as_str(),))
    }

    pub fn rename_backup(&self, backup_id: &str, new_name: &str) -> zbus::Result<()> {
        self.call("RenameBackup", &(backup_id, new_name))
    }

    pub fn reassign_backup(&self, backup_id: &str, new_dest_id: &str) -> zbus::Result<()> {
        self.call("ReassignBackup", &(backup_id, new_dest_id))
    }

    pub fn remove_backup(&self, backup_id: &str) -> zbus::Result<()> {
        self.call("RemoveBackup", &(backup_id,))
    }

    pub fn set_restic_provider(
        &self,
        provider: &kestic_core::model::ResticProvider,
    ) -> Result<ResticStatusDto, KesticDbusError> {
        let json = to_json_arg(provider)?;
        self.call_json("SetResticProvider", &(json.as_str(),))
    }

    pub fn set_expert_mode(&self, enabled: bool) -> zbus::Result<()> {
        self.call("SetExpertMode", &(enabled,))
    }

    pub fn start_backup(&self, backup_id: &str) -> zbus::Result<String> {
        self.call("StartBackup", &(backup_id,))
    }

    pub fn start_all_backups(&self) -> Result<StartAllResultDto, KesticDbusError> {
        self.call_json("StartAllBackups", &())
    }

    pub fn cancel_run(&self, run_id: &str) -> zbus::Result<()> {
        self.call("CancelRun", &(run_id,))
    }

    pub fn check_availability(
        &self,
        force: bool,
    ) -> Result<AvailabilitySnapshotDto, KesticDbusError> {
        self.call_json("CheckAvailability", &(force,))
    }

    pub fn window_active(&self, active: bool, page: &str) -> zbus::Result<()> {
        self.call("WindowActive", &(active, page))
    }

    pub fn register_gui(&self) -> zbus::Result<()> {
        self.call("RegisterGui", &())
    }

    pub fn confirm_quit(&self) -> zbus::Result<()> {
        self.call("ConfirmQuit", &())
    }

    pub fn unlock_secret_store(&self) -> zbus::Result<bool> {
        self.call("UnlockSecretStore", &())
    }

    pub fn detect_source(&self, path: &str) -> Result<Source, KesticDbusError> {
        self.call_json("DetectSource", &(path,))
    }

    pub fn check_nested_sources(
        &self,
        candidate_id: &str,
        candidate_path: &str,
        destination_id: &str,
    ) -> Result<Vec<NestedSourceConflict>, KesticDbusError> {
        self.call_json(
            "CheckNestedSources",
            &(candidate_id, candidate_path, destination_id),
        )
    }

    pub fn lint_excludes(
        &self,
        excludes: &ExcludeRules,
    ) -> Result<Vec<ExcludeLintWarning>, KesticDbusError> {
        let json = to_json_arg(excludes)?;
        self.call_json("LintExcludes", &(json.as_str(),))
    }

    pub fn test_exclude_path(
        &self,
        backup_id: &str,
        excludes: &ExcludeRules,
        test_path: &str,
    ) -> Result<ExcludePathTestResult, KesticDbusError> {
        let json = to_json_arg(excludes)?;
        self.call_json("TestExcludePath", &(backup_id, json.as_str(), test_path))
    }

    pub fn start_retention_preview(
        &self,
        backup_id: &str,
        retention: &RetentionPolicy,
    ) -> zbus::Result<String> {
        let json = to_failure_arg("retention", retention)?;
        self.call("StartRetentionPreview", &(backup_id, json.as_str()))
    }

    pub fn cancel_retention_preview(&self, preview_id: &str) -> zbus::Result<()> {
        self.call("CancelRetentionPreview", &(preview_id,))
    }

    pub fn get_retention_preview_status(
        &self,
        preview_id: &str,
    ) -> Result<RetentionPreviewStatusDto, KesticDbusError> {
        self.call_json("GetRetentionPreviewStatus", &(preview_id,))
    }

    pub fn start_backup_dry_run(
        &self,
        backup_id: &str,
        excludes: Option<&ExcludeRules>,
    ) -> zbus::Result<String> {
        let json = match excludes {
            Some(ex) => to_failure_arg("excludes", ex)?,
            None => String::new(),
        };
        self.call("StartBackupDryRun", &(backup_id, json.as_str()))
    }

    pub fn cancel_backup_dry_run(&self, preview_id: &str) -> zbus::Result<()> {
        self.call("CancelBackupDryRun", &(preview_id,))
    }

    pub fn get_backup_dry_run_status(
        &self,
        preview_id: &str,
    ) -> Result<BackupDryRunStatusDto, KesticDbusError> {
        self.call_json("GetBackupDryRunStatus", &(preview_id,))
    }

    pub fn format_exclude_rule_for_item(
        &self,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, KesticDbusError> {
        self.call_json("FormatExcludeRuleForItem", &(item_path,))
    }

    pub fn add_exclude_rule_for_item(
        &self,
        backup_id: &str,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, KesticDbusError> {
        self.call_json("AddExcludeRuleForItem", &(backup_id, item_path))
    }

    pub fn run_destination_maintenance(&self, destination_id: &str) -> zbus::Result<String> {
        self.call("RunDestinationMaintenance", &(destination_id,))
    }

    pub fn pause_global(&self, duration: &PauseDuration) -> zbus::Result<()> {
        let json = to_failure_arg("duration", duration)?;
        self.call("PauseGlobal", &(json.as_str(),))
    }

    pub fn resume_global(&self) -> zbus::Result<()> {
        self.call("ResumeGlobal", &())
    }

    pub fn pause_backup(&self, backup_id: &str, duration: &PauseDuration) -> zbus::Result<()> {
        let json = to_failure_arg("duration", duration)?;
        self.call("PauseBackup", &(backup_id, json.as_str()))
    }

    pub fn resume_backup(&self, backup_id: &str) -> zbus::Result<()> {
        self.call("ResumeBackup", &(backup_id,))
    }

    pub fn start_backup_override(&self, backup_id: &str) -> zbus::Result<String> {
        self.call("StartBackupOverride", &(backup_id,))
    }

    pub fn unlock_destination(&self, destination_id: &str) -> zbus::Result<u32> {
        self.call("UnlockDestination", &(destination_id,))
    }

    pub fn preview_destination_maintenance(
        &self,
        destination_id: &str,
    ) -> Result<PrunePreviewSummary, KesticDbusError> {
        self.call_json("PreviewDestinationMaintenance", &(destination_id,))
    }

    pub fn update_destination_maintenance(
        &self,
        destination_id: &str,
        maintenance: &MaintenanceSchedule,
    ) -> zbus::Result<()> {
        let json = to_failure_arg("maintenance", maintenance)?;
        self.call(
            "UpdateDestinationMaintenance",
            &(destination_id, json.as_str()),
        )
    }

    pub fn update_settings(
        &self,
        default_overdue_threshold: u32,
        notifications: &NotificationSettings,
    ) -> zbus::Result<()> {
        let json = to_failure_arg("notifications", notifications)?;
        self.call(
            "UpdateSettings",
            &(default_overdue_threshold, json.as_str()),
        )
    }

    pub fn list_known_networks(&self) -> Result<Vec<String>, KesticDbusError> {
        self.call_json("ListKnownNetworks", &())
    }
}

/// Serializes a method argument for a call whose error type is `KesticDbusError`.
pub(crate) fn to_json_arg<T: Serialize>(value: &T) -> Result<String, KesticDbusError> {
    serde_json::to_string(value).map_err(|e| KesticDbusError::Internal(e.to_string()))
}

/// Serializes a method argument for a call whose error type is `zbus::Error`.
fn to_failure_arg<T: Serialize>(what: &str, value: &T) -> zbus::Result<String> {
    serde_json::to_string(value).map_err(|e| zbus::Error::Failure(format!("serialize {what}: {e}")))
}

/// Status of an asynchronous preview (`GetBackupDryRunStatus`, `GetRetentionPreviewStatus`);
/// `T` is the result of the finished preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewStatusDto<T> {
    pub preview_id: String,
    /// `"running"` | `"done"` | `"cancelled"` | `"error"`
    pub state: String,
    pub result: Option<T>,
    /// Cause while `state == "error"`.
    pub error_kind: Option<ErrorKind>,
    /// Raw error text (shown for `unknown`).
    pub error_detail: Option<String>,
}

/// Dry-run preview status returned by `GetBackupDryRunStatus`.
pub type BackupDryRunStatusDto = PreviewStatusDto<BackupDryRunResult>;

/// Retention preview status returned by `GetRetentionPreviewStatus`.
pub type RetentionPreviewStatusDto = PreviewStatusDto<ForgetPreviewResult>;

/// Preview of the exclude rule generated for an unreadable path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludeRulePreviewDto {
    pub is_dir: bool,
    pub rule: String,
    pub is_expert_rule: bool,
}
