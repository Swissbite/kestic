// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Language-independent status and error codes shared by daemon and GUI over D-Bus
//! (`docs/spec/dbus-api.md` §Sprachunabhängige Codes).
//!
//! Every code serialises as a `snake_case` string. The texts for these codes live in
//! [`crate::messages`]. `kestic-core` converts its domain types into these codes.

use serde::{Deserialize, Serialize};

/// Action offered to the user for recoverable errors (`FR-37`, `error_action`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    UpdateStoredPassword,
    AddToExcludes,
    CheckResticSource,
    UnlockRepository,
}

impl RecoveryAction {
    pub const ALL: [Self; 4] = [
        Self::UpdateStoredPassword,
        Self::AddToExcludes,
        Self::CheckResticSource,
        Self::UnlockRepository,
    ];

    /// Code string as exposed to QML (`errorAction`), equal to the serde name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UpdateStoredPassword => "update_stored_password",
            Self::AddToExcludes => "add_to_excludes",
            Self::CheckResticSource => "check_restic_source",
            Self::UnlockRepository => "unlock_repository",
        }
    }

    /// Parses an `error_action` code; `None` for unknown codes.
    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == code)
    }
}

/// Cause of a failed run, a run with warning, or a skipped backup (`error_kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    WrongPassword,
    RepoMissing,
    Locked,
    /// Destination unreachable, including DNS failures.
    DestinationUnreachable,
    SourceUnavailable,
    AuthFailed,
    IncompleteSnapshot,
    /// Backed up, but the retention (`forget`) did not run.
    RetentionPending,
    Cancelled,
    /// Only for skipped backups (`SkippedBackupDto`): the password store is locked.
    SecretStoreLocked,
    /// Only for skipped backups (`SkippedBackupDto`): restic is missing or too old.
    ResticUnavailable,
    /// Anything not recognised (never a transient code).
    Unknown,
}

impl ErrorKind {
    pub const ALL: [Self; 12] = [
        Self::WrongPassword,
        Self::RepoMissing,
        Self::Locked,
        Self::DestinationUnreachable,
        Self::SourceUnavailable,
        Self::AuthFailed,
        Self::IncompleteSnapshot,
        Self::RetentionPending,
        Self::Cancelled,
        Self::SecretStoreLocked,
        Self::ResticUnavailable,
        Self::Unknown,
    ];

    /// Code string, equal to the serde name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WrongPassword => "wrong_password",
            Self::RepoMissing => "repo_missing",
            Self::Locked => "locked",
            Self::DestinationUnreachable => "destination_unreachable",
            Self::SourceUnavailable => "source_unavailable",
            Self::AuthFailed => "auth_failed",
            Self::IncompleteSnapshot => "incomplete_snapshot",
            Self::RetentionPending => "retention_pending",
            Self::Cancelled => "cancelled",
            Self::SecretStoreLocked => "secret_store_locked",
            Self::ResticUnavailable => "restic_unavailable",
            Self::Unknown => "unknown",
        }
    }
}

/// Why a due backup or due maintenance waits (`pending_reason_codes`,
/// `maintenance_pending_reason`) and which run condition is not met (`unmet_run_conditions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingReasonCode {
    SourceUnavailable,
    DestinationUnreachable,
    OnBattery,
    NoNetwork,
    Metered,
    WrongNetwork,
    /// Another run on the same destination is active.
    Queue,
    /// Retry cooldown after a transient error; see `pending_retry_at_unix`.
    Backoff,
}

impl PendingReasonCode {
    pub const ALL: [Self; 8] = [
        Self::SourceUnavailable,
        Self::DestinationUnreachable,
        Self::OnBattery,
        Self::NoNetwork,
        Self::Metered,
        Self::WrongNetwork,
        Self::Queue,
        Self::Backoff,
    ];

    /// Code string as exposed to QML (`pendingPrimaryCode`), equal to the serde name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceUnavailable => "source_unavailable",
            Self::DestinationUnreachable => "destination_unreachable",
            Self::OnBattery => "on_battery",
            Self::NoNetwork => "no_network",
            Self::Metered => "metered",
            Self::WrongNetwork => "wrong_network",
            Self::Queue => "queue",
            Self::Backoff => "backoff",
        }
    }
}

/// Result of the source check when the source is unavailable (`source_status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    PathMissing,
    DriveNotMounted,
    DriveMismatch,
    DriveEmpty,
}

impl SourceStatus {
    pub const ALL: [Self; 4] = [
        Self::PathMissing,
        Self::DriveNotMounted,
        Self::DriveMismatch,
        Self::DriveEmpty,
    ];

    /// Code string, equal to the serde name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PathMissing => "path_missing",
            Self::DriveNotMounted => "drive_not_mounted",
            Self::DriveMismatch => "drive_mismatch",
            Self::DriveEmpty => "drive_empty",
        }
    }
}

/// Why a backup cannot start right now (`blocked_reason`, signal `ManualRunBlocked`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    SourceUnavailable,
    DestinationUnreachable,
    ConditionNotMet,
}

impl BlockedReason {
    pub const ALL: [Self; 3] = [
        Self::SourceUnavailable,
        Self::DestinationUnreachable,
        Self::ConditionNotMet,
    ];

    /// Code string as sent in the `ManualRunBlocked` signal and in service errors.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceUnavailable => "source_unavailable",
            Self::DestinationUnreachable => "destination_unreachable",
            Self::ConditionNotMet => "condition_not_met",
        }
    }

    /// Parses a code string; `None` for unknown codes.
    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == code)
    }
}

/// Why «Back Up All Now» skipped a backup (`SkippedBackupDto.reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    PermanentError,
    SourceUnavailable,
    DestinationUnreachable,
    ConditionNotMet,
    /// Start failed; the cause is in `SkippedBackupDto.error_kind`.
    Error,
}

impl SkipReason {
    pub const ALL: [Self; 5] = [
        Self::PermanentError,
        Self::SourceUnavailable,
        Self::DestinationUnreachable,
        Self::ConditionNotMet,
        Self::Error,
    ];

    /// Code string as stored in the run history of the skipped backup.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PermanentError => "permanent_error",
            Self::SourceUnavailable => "source_unavailable",
            Self::DestinationUnreachable => "destination_unreachable",
            Self::ConditionNotMet => "condition_not_met",
            Self::Error => "error",
        }
    }
}

impl From<BlockedReason> for SkipReason {
    fn from(reason: BlockedReason) -> Self {
        match reason {
            BlockedReason::SourceUnavailable => Self::SourceUnavailable,
            BlockedReason::DestinationUnreachable => Self::DestinationUnreachable,
            BlockedReason::ConditionNotMet => Self::ConditionNotMet,
        }
    }
}

/// Why the configured restic is unusable (`ResticStatusDto.error_kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResticStatusCode {
    NotFound,
    TooOld,
    InvalidCustomPath,
    UnreadableVersion,
}

impl ResticStatusCode {
    pub const ALL: [Self; 4] = [
        Self::NotFound,
        Self::TooOld,
        Self::InvalidCustomPath,
        Self::UnreadableVersion,
    ];

    /// Code string, equal to the serde name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::TooOld => "too_old",
            Self::InvalidCustomPath => "invalid_custom_path",
            Self::UnreadableVersion => "unreadable_version",
        }
    }
}

/// Blocking state of the whole app (`BlockingStateDto.kind`, `zustaende.md` §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockingKind {
    SecretStoreLocked,
    ResticUnavailable,
}

impl BlockingKind {
    pub const ALL: [Self; 2] = [Self::SecretStoreLocked, Self::ResticUnavailable];

    /// Code string as exposed to QML (`blockingKind`), equal to the serde name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SecretStoreLocked => "secret_store_locked",
            Self::ResticUnavailable => "restic_unavailable",
        }
    }
}

/// Result code of `TestDestinationConnection` (`ConnectionTestResultDto.error_kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionTestCode {
    InvalidInput,
    /// `detail` = name of the destination that already uses the location.
    DuplicateLocation,
    DestinationUnreachable,
    RepositoryExists,
    NotWritable,
    MissingPassword,
    SecretStoreLocked,
    Internal,
    ResticUnavailable,
    WrongPassword,
    AuthFailed,
    RepositoryMissing,
    CommandFailed,
    /// Set by the GUI when the daemon cannot be reached.
    DaemonUnreachable,
}

impl ConnectionTestCode {
    pub const ALL: [Self; 14] = [
        Self::InvalidInput,
        Self::DuplicateLocation,
        Self::DestinationUnreachable,
        Self::RepositoryExists,
        Self::NotWritable,
        Self::MissingPassword,
        Self::SecretStoreLocked,
        Self::Internal,
        Self::ResticUnavailable,
        Self::WrongPassword,
        Self::AuthFailed,
        Self::RepositoryMissing,
        Self::CommandFailed,
        Self::DaemonUnreachable,
    ];

    /// Code string as exposed to QML (`connectionTestErrorKind`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::DuplicateLocation => "duplicate_location",
            Self::DestinationUnreachable => "destination_unreachable",
            Self::RepositoryExists => "repository_exists",
            Self::NotWritable => "not_writable",
            Self::MissingPassword => "missing_password",
            Self::SecretStoreLocked => "secret_store_locked",
            Self::Internal => "internal",
            Self::ResticUnavailable => "restic_unavailable",
            Self::WrongPassword => "wrong_password",
            Self::AuthFailed => "auth_failed",
            Self::RepositoryMissing => "repository_missing",
            Self::CommandFailed => "command_failed",
            Self::DaemonUnreachable => "daemon_unreachable",
        }
    }
}

/// Code of a D-Bus error: the error name `net.daester.Kestic.Error.<Name>`
/// (`dbus-api.md` §Fehler-Namen). The error message is raw detail only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ServiceErrorCode {
    InvalidInput,
    NotFound,
    InUse,
    ManualRunBlocked,
    DestinationUnreachable,
    WrongPassword,
    AuthFailed,
    SecretStoreLocked,
    ResticUnavailable,
    AlreadyRunning,
    /// Unlock refused: the lock belongs to an active or remote process.
    LockInUse,
    /// The repository location is already used by another destination.
    DuplicateLocation,
    Internal,
    /// No D-Bus error name: the daemon could not be reached (transport error, set by the GUI).
    DaemonUnreachable,
}

impl ServiceErrorCode {
    pub const ALL: [Self; 14] = [
        Self::InvalidInput,
        Self::NotFound,
        Self::InUse,
        Self::ManualRunBlocked,
        Self::DestinationUnreachable,
        Self::WrongPassword,
        Self::AuthFailed,
        Self::SecretStoreLocked,
        Self::ResticUnavailable,
        Self::AlreadyRunning,
        Self::LockInUse,
        Self::DuplicateLocation,
        Self::Internal,
        Self::DaemonUnreachable,
    ];
}

/// Aggregated overall status precedence (`tray-und-benachrichtigungen.md` §10.1,
/// `zustaende.md` §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

impl OverallStatusKind {
    pub const ALL: [Self; 12] = [
        Self::Blocked,
        Self::Error,
        Self::Overdue,
        Self::Warning,
        Self::Running,
        Self::Paused,
        Self::Pending,
        Self::Interrupted,
        Self::Cancelled,
        Self::NeverRun,
        Self::ManualOnly,
        Self::Ok,
    ];

    /// Code string, equal to the serde name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Error => "error",
            Self::Overdue => "overdue",
            Self::Warning => "warning",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Pending => "pending",
            Self::Interrupted => "interrupted",
            Self::Cancelled => "cancelled",
            Self::NeverRun => "never_run",
            Self::ManualOnly => "manual_only",
            Self::Ok => "ok",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json<T: Serialize>(value: T) -> String {
        serde_json::to_string(&value).expect("serialize code")
    }

    /// The typed codes keep the exact JSON strings of the former string fields.
    #[test]
    fn codes_serialize_as_the_former_snake_case_strings() {
        assert_eq!(json(ErrorKind::RepoMissing), "\"repo_missing\"");
        assert_eq!(
            json(ErrorKind::IncompleteSnapshot),
            "\"incomplete_snapshot\""
        );
        assert_eq!(json(PendingReasonCode::Queue), "\"queue\"");
        assert_eq!(json(PendingReasonCode::WrongNetwork), "\"wrong_network\"");
        assert_eq!(json(SourceStatus::DriveNotMounted), "\"drive_not_mounted\"");
        assert_eq!(json(SkipReason::PermanentError), "\"permanent_error\"");
        assert_eq!(
            json(ResticStatusCode::InvalidCustomPath),
            "\"invalid_custom_path\""
        );
        assert_eq!(
            json(BlockingKind::SecretStoreLocked),
            "\"secret_store_locked\""
        );
        assert_eq!(json(OverallStatusKind::NeverRun), "\"never_run\"");
    }

    /// `as_str` and the serde name agree for every variant, which pins all JSON names.
    #[test]
    fn as_str_matches_serde_name() {
        for r in BlockedReason::ALL {
            assert_eq!(json(r), format!("\"{}\"", r.as_str()));
            assert_eq!(BlockedReason::parse(r.as_str()), Some(r));
        }
        for r in SkipReason::ALL {
            assert_eq!(json(r), format!("\"{}\"", r.as_str()));
        }
        for k in ResticStatusCode::ALL {
            assert_eq!(json(k), format!("\"{}\"", k.as_str()));
        }
        for k in OverallStatusKind::ALL {
            assert_eq!(json(k), format!("\"{}\"", k.as_str()));
        }
        for k in ErrorKind::ALL {
            assert_eq!(json(k), format!("\"{}\"", k.as_str()));
        }
        for k in SourceStatus::ALL {
            assert_eq!(json(k), format!("\"{}\"", k.as_str()));
        }
        for k in BlockingKind::ALL {
            assert_eq!(json(k), format!("\"{}\"", k.as_str()));
        }
        for c in PendingReasonCode::ALL {
            assert_eq!(json(c), format!("\"{}\"", c.as_str()));
        }
        for a in RecoveryAction::ALL {
            assert_eq!(json(a), format!("\"{}\"", a.as_str()));
            assert_eq!(RecoveryAction::parse(a.as_str()), Some(a));
        }
        for c in ConnectionTestCode::ALL {
            assert_eq!(json(c), format!("\"{}\"", c.as_str()));
        }
        assert_eq!(BlockedReason::parse("nope"), None);
    }
}
