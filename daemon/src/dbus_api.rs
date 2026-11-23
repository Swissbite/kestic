// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Versioned D-Bus API v1 (`net.daester.Kestic1`) shared DTOs, error types, and client proxy
//! (`docs/spec/dbus-api.md`).

use kestic_core::backend_credentials::{
    BackendCredentials, SECRET_KIND_REST, SECRET_KIND_S3_ACCESS_KEY, SECRET_KIND_S3_SECRET_KEY,
};
use kestic_core::backup::{BackupDryRunResult, ForgetPreviewResult};
use kestic_core::config::AppConfig;
use kestic_core::maintenance::PrunePreviewSummary;
use kestic_core::model::{
    Backup, Destination, DestinationId, ExcludeLintWarning, ExcludePathTestResult, ExcludeRules,
    MaintenanceSchedule, NestedSourceConflict, NotificationSettings, RetentionPolicy, Run, Source,
};
use kestic_core::provider::MIN_RESTIC_VERSION;
use kestic_core::restic::RecoveryAction;
use kestic_core::scheduler::PauseDuration;
use kestic_core::secrets::SecretValue;
use kestic_i18n::codes::{
    BlockedReason, BlockingKind, ConnectionNoteCode, ConnectionTestCode, ErrorKind,
    PendingReasonCode, ResticStatusCode, ServiceErrorCode, SkipReason, SourceStatus,
};
pub use kestic_i18n::codes::{OverallStatusKind, RunProgressPhase};
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
    HostKeyUnknown(String),
    HostKeyChanged(String),
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
            Self::HostKeyUnknown(_) => ServiceErrorCode::HostKeyUnknown,
            Self::HostKeyChanged(_) => ServiceErrorCode::HostKeyChanged,
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
    pub previous_destinations: Vec<PreviousDestinationSnapshotsDto>,
}

/// Entry of `ListDestinationTypes`: a destination type and whether it is ready.
pub type DestinationTypeDto = kestic_core::destination_types::DestinationTypeStatus;

/// Result of `TestDestinationConnection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionTestResultDto {
    pub ok: bool,
    pub error_kind: Option<ConnectionTestCode>,
    /// Raw error detail, or the parameter of the code's text (e.g. the destination name for
    /// `duplicate_location`).
    pub detail: String,
    /// Warnings of a successful test that the user confirms before adding the destination.
    #[serde(default)]
    pub notes: Vec<ConnectionNoteCode>,
    /// The program that is not installed, with `program_missing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// The host key to confirm, with `unknown_host_key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_key: Option<Box<HostKeyDto>>,
}

/// Host key of an SFTP server that no `known_hosts` file knows yet (`unknown_host_key`);
/// `TrustHostKey(host_key_id)` adds exactly this key to Kestic's `known_hosts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKeyDto {
    pub host_key_id: String,
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint_sha256: String,
}

/// `backend_secrets_json` of `TestDestinationConnection`, `AddDestination` and
/// `UpdateDestination`: the backend credentials entered in the form. An empty field keeps the
/// saved value when editing. The values go only into the password store and into the
/// environment of restic.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendSecrets {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rest_password: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub s3_access_key_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub s3_secret_access_key: String,
    /// Values of the secret variables of a generic destination, by variable name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub env: std::collections::BTreeMap<String, String>,
}

impl std::fmt::Debug for BackendSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BackendSecrets([REDACTED])")
    }
}

impl BackendSecrets {
    /// Only the REST server password.
    pub fn rest(password: &str) -> Self {
        Self {
            rest_password: password.to_string(),
            ..Self::default()
        }
    }

    /// Only the secret variables of a generic destination.
    pub fn env(values: &[(&str, &str)]) -> Self {
        Self {
            env: values
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
            ..Self::default()
        }
    }

    /// Only the S3 keys.
    pub fn s3(access_key_id: &str, secret_access_key: &str) -> Self {
        Self {
            s3_access_key_id: access_key_id.to_string(),
            s3_secret_access_key: secret_access_key.to_string(),
            ..Self::default()
        }
    }

    /// Parses `backend_secrets_json`; an empty string means no credentials.
    pub fn parse(json: &str) -> Result<Self, String> {
        if json.trim().is_empty() {
            return Ok(Self::default());
        }
        // serde's message can quote the input, which holds secrets: only say where it failed.
        serde_json::from_str(json).map_err(|e| {
            format!(
                "invalid backend secrets JSON ({:?} error at line {} column {})",
                e.classify(),
                e.line(),
                e.column()
            )
        })
    }

    /// The entered values by password store kind; empty fields are left out.
    pub fn credentials(&self) -> BackendCredentials {
        let mut creds = BackendCredentials::none();
        for (kind, value) in [
            (SECRET_KIND_REST, &self.rest_password),
            (SECRET_KIND_S3_ACCESS_KEY, &self.s3_access_key_id),
            (SECRET_KIND_S3_SECRET_KEY, &self.s3_secret_access_key),
        ] {
            creds.set(kind, Some(SecretValue::from_str_secret(value.as_str())));
        }
        for (name, value) in &self.env {
            creds.set(
                &kestic_core::generic::secret_kind(name),
                Some(SecretValue::from_str_secret(value.as_str())),
            );
        }
        creds
    }
}

/// Entry of `ListRemovableDrives`: a mounted removable drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovableDriveDto {
    /// Filesystem label; empty when the filesystem has none.
    pub label: String,
    pub mount_point: String,
    /// Filesystem UUID; empty when unknown.
    pub fs_uuid: String,
    pub size_bytes: u64,
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
            is_maintenance: self.backup_id.is_none(),
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

    pub fn test_destination_connection(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        backend: &BackendSecrets,
    ) -> Result<ConnectionTestResultDto, KesticDbusError> {
        let json = to_json_arg(dest)?;
        let secrets = to_json_arg(backend)?;
        self.call_json(
            "TestDestinationConnection",
            &(json.as_str(), mode, repo_password, secrets.as_str()),
        )
    }

    pub fn add_destination(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        backend: &BackendSecrets,
    ) -> zbus::Result<String> {
        let json = to_failure_arg("dest", dest)?;
        let secrets = to_failure_arg("backend secrets", backend)?;
        self.call(
            "AddDestination",
            &(json.as_str(), mode, repo_password, secrets.as_str()),
        )
    }

    pub fn update_destination(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        backend: &BackendSecrets,
        verify_connection: bool,
    ) -> zbus::Result<()> {
        let json = to_failure_arg("dest", dest)?;
        let secrets = to_failure_arg("backend secrets", backend)?;
        self.call(
            "UpdateDestination",
            &(
                json.as_str(),
                mode,
                repo_password,
                secrets.as_str(),
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

    pub fn trust_host_key(&self, host_key_id: &str) -> zbus::Result<()> {
        self.call("TrustHostKey", &(host_key_id,))
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

    pub fn list_destination_types(&self) -> Result<Vec<DestinationTypeDto>, KesticDbusError> {
        self.call_json("ListDestinationTypes", &())
    }

    pub fn list_removable_drives(&self) -> Result<Vec<RemovableDriveDto>, KesticDbusError> {
        self.call_json("ListRemovableDrives", &())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A malformed `backend_secrets_json` (an old GUI sent the REST password as a bare string)
    /// is refused without quoting the value in the error.
    #[test]
    fn invalid_backend_secrets_error_does_not_quote_the_input() {
        for (input, value) in [
            ("\"hunter2-secret\"", "hunter2-secret"),
            ("{\"rest_password\": 4711}", "4711"),
        ] {
            let err = BackendSecrets::parse(input).expect_err("malformed input is refused");
            assert!(
                err.starts_with("invalid backend secrets JSON"),
                "the error says what failed"
            );
            assert!(!err.contains(value), "the error repeats the input");
        }
    }
}
