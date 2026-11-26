// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Service error types, error mappers, and localized failure/action helpers.

use crate::dbus_api::KesticDbusError;
use crate::i18n::tr;
use kestic_core::backup::BackupRunError;
use kestic_core::config::ConfigError;
use kestic_core::destination::{DestinationError, DestinationSetupMode};
use kestic_core::model::BackupId;
use kestic_core::mount::SourceCheckResult;
use kestic_core::provider::ResticProviderError;
use kestic_core::restic::{ResticFailureKind, ResticRunError};
use kestic_core::secrets::SecretError;
use kestic_i18n::codes::{BlockedReason, ErrorKind, SourceStatus};
use kestic_i18n::messages::credentials_not_entered_message;

/// Daemon-language summary of a classified restic failure (notifications, run history).
pub(crate) fn localized_failure_summary(kind: &ResticFailureKind) -> String {
    tr(&kind.summary_message())
}

/// Maps the failure of a dry-run preview (retention or prune) to the service error.
pub(crate) fn map_backup_run_err(err: BackupRunError) -> KesticServiceErr {
    match err {
        BackupRunError::Destination(d) => map_destination_err(d),
        BackupRunError::InvalidModel(m) => KesticServiceErr::InvalidInput(m.to_string()),
        other => KesticServiceErr::Internal(other.to_string()),
    }
}

/// Parses a backup id received over D-Bus.
pub(crate) fn parse_backup_id(raw: &str) -> Result<BackupId, KesticServiceErr> {
    BackupId::parse(raw).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))
}

/// Internal service error mapped to `net.daester.Kestic.Error.*` D-Bus errors.
#[derive(Debug)]
pub enum KesticServiceErr {
    InvalidInput(String),
    NotFound(String),
    InUse(String),
    /// Manual start blocked by an unavailable source/destination or unmet run conditions; the
    /// D-Bus error payload is the reason code.
    ManualRunBlocked(BlockedReason),
    /// Scheduled start refused while the system prepares for sleep. Only scheduled runs fail
    /// this way and the scheduler drops their result, so it never reaches D-Bus.
    SleepInProgress,
    DestinationUnreachable(String),
    WrongPassword(String),
    AuthFailed(String),
    SecretStoreLocked(String),
    ResticUnavailable(String),
    AlreadyRunning(String),
    /// Unlock refused: the lock belongs to an active or remote process.
    LockInUse(String),
    /// Location already used; the message is the name of that destination (raw detail).
    DuplicateLocation(String),
    /// SFTP: the server's host key is not known yet.
    HostKeyUnknown(String),
    /// SFTP: the server shows another host key; the message is the file with the old entry.
    HostKeyChanged(String),
    /// `StartRestore` without a matching, finished, fresh preview.
    PreviewRequired(String),
    /// A destination changed or removed while a restore from it starts, waits or runs.
    RestoreRunning(String),
    Internal(String),
}

impl std::fmt::Display for KesticServiceErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(m)
            | Self::NotFound(m)
            | Self::InUse(m)
            | Self::DestinationUnreachable(m)
            | Self::WrongPassword(m)
            | Self::AuthFailed(m)
            | Self::SecretStoreLocked(m)
            | Self::ResticUnavailable(m)
            | Self::AlreadyRunning(m)
            | Self::LockInUse(m)
            | Self::DuplicateLocation(m)
            | Self::HostKeyUnknown(m)
            | Self::HostKeyChanged(m)
            | Self::PreviewRequired(m)
            | Self::RestoreRunning(m)
            | Self::Internal(m) => f.write_str(m),
            Self::ManualRunBlocked(reason) => f.write_str(reason.as_str()),
            Self::SleepInProgress => f.write_str("sleep_in_progress"),
        }
    }
}

impl From<KesticServiceErr> for KesticDbusError {
    fn from(err: KesticServiceErr) -> Self {
        // The GUI shows only the text of the error name; the raw detail of unexpected errors
        // goes to the session journal.
        if let KesticServiceErr::Internal(detail) | KesticServiceErr::InvalidInput(detail) = &err {
            eprintln!("kestic-daemon: D-Bus call failed: {detail}");
        }
        match err {
            KesticServiceErr::InvalidInput(m) => KesticDbusError::InvalidInput(m),
            KesticServiceErr::NotFound(m) => KesticDbusError::NotFound(m),
            KesticServiceErr::InUse(m) => KesticDbusError::InUse(m),
            KesticServiceErr::ManualRunBlocked(reason) => {
                KesticDbusError::ManualRunBlocked(reason.as_str().to_string())
            }
            err @ KesticServiceErr::SleepInProgress => KesticDbusError::Internal(err.to_string()),
            KesticServiceErr::DestinationUnreachable(m) => {
                KesticDbusError::DestinationUnreachable(m)
            }
            KesticServiceErr::WrongPassword(m) => KesticDbusError::WrongPassword(m),
            KesticServiceErr::AuthFailed(m) => KesticDbusError::AuthFailed(m),
            KesticServiceErr::SecretStoreLocked(m) => KesticDbusError::SecretStoreLocked(m),
            KesticServiceErr::ResticUnavailable(m) => KesticDbusError::ResticUnavailable(m),
            KesticServiceErr::AlreadyRunning(m) => KesticDbusError::AlreadyRunning(m),
            KesticServiceErr::LockInUse(m) => KesticDbusError::LockInUse(m),
            KesticServiceErr::DuplicateLocation(m) => KesticDbusError::DuplicateLocation(m),
            KesticServiceErr::HostKeyUnknown(m) => KesticDbusError::HostKeyUnknown(m),
            KesticServiceErr::HostKeyChanged(m) => KesticDbusError::HostKeyChanged(m),
            KesticServiceErr::PreviewRequired(m) => KesticDbusError::PreviewRequired(m),
            KesticServiceErr::RestoreRunning(m) => KesticDbusError::RestoreRunning(m),
            KesticServiceErr::Internal(m) => KesticDbusError::Internal(m),
        }
    }
}

/// An `InvalidInput` error with `message`.
pub(super) fn invalid(message: impl std::fmt::Display) -> KesticServiceErr {
    KesticServiceErr::InvalidInput(message.to_string())
}

pub(super) fn map_config_err(err: ConfigError) -> KesticServiceErr {
    match err {
        ConfigError::Validation(_)
        | ConfigError::UnsupportedVersion { .. }
        | ConfigError::MissingVersion => KesticServiceErr::InvalidInput(err.to_string()),
        ConfigError::Model(m) => KesticServiceErr::InvalidInput(m.to_string()),
        ConfigError::Parse(e) => KesticServiceErr::InvalidInput(e.to_string()),
        other => KesticServiceErr::Internal(other.to_string()),
    }
}

pub(super) fn map_provider_err(err: ResticProviderError) -> KesticServiceErr {
    KesticServiceErr::ResticUnavailable(err.to_string())
}

pub(super) fn map_secret_err(err: SecretError) -> KesticServiceErr {
    match err {
        SecretError::LockedOrPromptRequired(m) => KesticServiceErr::SecretStoreLocked(m),
        other => KesticServiceErr::Internal(other.to_string()),
    }
}

pub(super) fn map_destination_err(err: DestinationError) -> KesticServiceErr {
    match err {
        err @ DestinationError::WrongPassword => KesticServiceErr::WrongPassword(err.to_string()),
        DestinationError::AuthFailed(m) | DestinationError::SshKeyRejected(m) => {
            KesticServiceErr::AuthFailed(m)
        }
        DestinationError::Unreachable(m) | DestinationError::NotResponding(m) => {
            KesticServiceErr::DestinationUnreachable(m)
        }
        err @ DestinationError::HostKeyCheck(kestic_core::known_hosts::HostKeyError::NoAnswer {
            ..
        }) => KesticServiceErr::DestinationUnreachable(err.to_string()),
        err @ DestinationError::HostKeyUnknown(_) => {
            KesticServiceErr::HostKeyUnknown(err.to_string())
        }
        DestinationError::HostKeyChanged(file) => {
            KesticServiceErr::HostKeyChanged(file.to_string_lossy().into_owned())
        }
        DestinationError::Secret(e) => map_secret_err(e),
        other => KesticServiceErr::InvalidInput(other.to_string()),
    }
}

pub(super) fn parse_setup_mode(raw: &str) -> Result<DestinationSetupMode, KesticServiceErr> {
    match raw {
        "init" => Ok(DestinationSetupMode::InitNew),
        "connect" => Ok(DestinationSetupMode::ConnectExisting),
        other => Err(KesticServiceErr::InvalidInput(format!(
            "invalid setup mode '{other}' (expected 'init' or 'connect')"
        ))),
    }
}

/// Error kind of a service error (`SkippedBackupDto::error_kind`).
pub(super) fn service_error_kind(err: &KesticServiceErr) -> ErrorKind {
    match err {
        KesticServiceErr::SecretStoreLocked(_) => ErrorKind::SecretStoreLocked,
        KesticServiceErr::ResticUnavailable(_) => ErrorKind::ResticUnavailable,
        KesticServiceErr::WrongPassword(_) => ErrorKind::WrongPassword,
        KesticServiceErr::AuthFailed(_) => ErrorKind::AuthFailed,
        KesticServiceErr::DestinationUnreachable(_) => ErrorKind::DestinationUnreachable,
        _ => ErrorKind::Unknown,
    }
}

/// Blocked reason carried by a [`KesticServiceErr::ManualRunBlocked`].
pub(super) fn manual_run_blocked_reason(err: &KesticServiceErr) -> Option<BlockedReason> {
    match err {
        KesticServiceErr::ManualRunBlocked(reason) => Some(*reason),
        _ => None,
    }
}

/// Daemon-language reason of an unavailable source (run history, notifications).
pub(super) fn localized_source_check_reason(status: &SourceCheckResult) -> String {
    tr(&SourceStatus::message(status.status_code()))
}

/// Restic failure behind a destination error, if the error carries one.
fn destination_failure(err: &DestinationError) -> Option<ResticFailureKind> {
    match err {
        DestinationError::Unreachable(_) => Some(ResticFailureKind::DestinationUnreachable),
        DestinationError::AuthFailed(_) => Some(ResticFailureKind::BackendAuthFailed),
        DestinationError::WrongPassword => Some(ResticFailureKind::WrongPassword),
        DestinationError::SshKeyRejected(_) => Some(ResticFailureKind::BackendAuthFailed),
        DestinationError::HostKeyUnknown(_) => Some(ResticFailureKind::HostKeyUnknown),
        DestinationError::HostKeyChanged(_) => Some(ResticFailureKind::HostKeyChanged),
        DestinationError::RepositoryMissing(_) => Some(ResticFailureKind::RepositoryMissing),
        DestinationError::CommandFailed { kind, .. } => Some(kind.clone()),
        _ => None,
    }
}

/// Error kind of a failed run or preview (`BackupDryRunStatusDto`, `RetentionPreviewStatusDto`);
/// the raw detail is `err.to_string()`.
pub(super) fn backup_run_error_kind(err: &BackupRunError) -> ErrorKind {
    match err {
        BackupRunError::SourceUnavailable(_) => ErrorKind::SourceUnavailable,
        BackupRunError::Cancelled => ErrorKind::Cancelled,
        BackupRunError::PathNotInSnapshot(_) => ErrorKind::PathNotInSnapshot,
        BackupRunError::InvalidTarget { .. } => ErrorKind::InvalidTarget,
        BackupRunError::Destination(DestinationError::Secret(_)) => ErrorKind::SecretStoreLocked,
        BackupRunError::Destination(dest_err) => {
            destination_failure(dest_err).map_or(ErrorKind::Unknown, |kind| kind.error_kind())
        }
        _ => ErrorKind::Unknown,
    }
}

/// The variables named by a run refused before restic started because the destination needs a
/// credential variable entered in it.
fn credentials_not_entered(err: &BackupRunError) -> Option<&str> {
    match err {
        BackupRunError::Runner(ResticRunError::CredentialsNotEntered { variables, .. })
        | BackupRunError::Destination(DestinationError::Runner(
            ResticRunError::CredentialsNotEntered { variables, .. },
        )) => Some(variables),
        _ => None,
    }
}

/// Detail of a failed restic step: the daemon-language text where Kestic refused to start
/// restic for a credential variable not entered in the destination, otherwise the raw error.
pub(super) fn run_error_detail(err: &BackupRunError) -> String {
    credentials_not_entered(err).map_or_else(
        || err.to_string(),
        |variables| tr(&credentials_not_entered_message(variables)),
    )
}

pub(super) fn localized_backup_run_error(err: &BackupRunError) -> String {
    match err {
        BackupRunError::SourceUnavailable(status) => localized_source_check_reason(status),
        // The raw text names the server or the file with the old entry, and the run's error
        // kind is read back from it: it starts the message, where no file name or server text
        // can stand.
        BackupRunError::Destination(
            dest_err @ (DestinationError::HostKeyUnknown(_) | DestinationError::HostKeyChanged(_)),
        ) => destination_failure(dest_err).map_or_else(
            || dest_err.to_string(),
            |kind| format!("{dest_err} ({})", localized_failure_summary(&kind)),
        ),
        err if credentials_not_entered(err).is_some() => run_error_detail(err),
        BackupRunError::Destination(dest_err) => destination_failure(dest_err).map_or_else(
            || dest_err.to_string(),
            |kind| localized_failure_summary(&kind),
        ),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AddDestination` / `UpdateDestination` in mode `connect`: restic stopped after the time
    /// limit means the destination is not reachable, not invalid input.
    #[test]
    fn not_responding_restic_is_an_unreachable_destination() {
        let err = map_destination_err(DestinationError::NotResponding(
            "restic did not finish within 15 s".to_string(),
        ));
        assert!(
            matches!(err, KesticServiceErr::DestinationUnreachable(_)),
            "restic that does not answer must map to an unreachable destination"
        );
    }
}
