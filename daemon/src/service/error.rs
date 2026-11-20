// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Service error types, error mappers, and localized failure/action helpers.

use crate::dbus_api::KesticDbusError;
use crate::i18n::{i18n, localized_failure_summary, localized_recovery_action_label};
use kestic_core::backup::BackupRunError;
use kestic_core::config::ConfigError;
use kestic_core::destination::{DestinationError, DestinationSetupMode};
use kestic_core::mount::SourceCheckResult;
use kestic_core::provider::ResticProviderError;
use kestic_core::restic::{RecoveryAction, ResticFailureKind, classify_exit};
use kestic_core::secrets::SecretError;

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
        DestinationError::WrongPassword => {
            KesticServiceErr::WrongPassword(i18n("Stored password does not match").into())
        }
        DestinationError::AuthFailed(m) => KesticServiceErr::AuthFailed(m),
        DestinationError::Unreachable(m) => KesticServiceErr::DestinationUnreachable(m),
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

/// Locale-independent source check code for `BackupAvailabilityDto::source_status`.
pub(super) fn source_status_code(status: &SourceCheckResult) -> Option<&'static str> {
    match status {
        SourceCheckResult::Available => None,
        SourceCheckResult::PathMissing(_) => Some("path_missing"),
        SourceCheckResult::DriveNotMounted => Some("drive_not_mounted"),
        SourceCheckResult::DriveMismatch => Some("drive_mismatch"),
        SourceCheckResult::DriveEmpty => Some("drive_empty"),
    }
}

/// Locale-independent error-kind code of a service error (`SkippedBackupDto::error_kind`).
pub(super) fn service_error_kind_code(err: &KesticServiceErr) -> &'static str {
    match err {
        KesticServiceErr::SecretStoreLocked(_) => "secret_store_locked",
        KesticServiceErr::ResticUnavailable(_) => "restic_unavailable",
        KesticServiceErr::WrongPassword(_) => "wrong_password",
        KesticServiceErr::AuthFailed(_) => "auth_failed",
        KesticServiceErr::DestinationUnreachable(_) => "destination_unreachable",
        _ => "unknown",
    }
}

pub(super) fn localized_source_check_reason(status: &SourceCheckResult) -> String {
    match status {
        SourceCheckResult::DriveNotMounted => i18n("Source unavailable: drive not mounted").into(),
        SourceCheckResult::DriveMismatch => i18n("Source unavailable: different drive").into(),
        SourceCheckResult::DriveEmpty => i18n("Source unavailable: source empty").into(),
        SourceCheckResult::PathMissing(_) | SourceCheckResult::Available => {
            i18n("Source unavailable").into()
        }
    }
}

pub(super) fn localized_backup_run_error(err: &BackupRunError) -> String {
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

pub(super) fn action_code(action: RecoveryAction) -> &'static str {
    match action {
        RecoveryAction::UpdateStoredPassword => "update_stored_password",
        RecoveryAction::AddToExcludes => "add_to_excludes",
        RecoveryAction::CheckResticSource => "check_restic_source",
        RecoveryAction::UnlockRepository => "unlock_repository",
    }
}

/// Returns localized action label for recovery actions (compile-time check helper).
pub fn localized_action_title(action: RecoveryAction) -> String {
    localized_recovery_action_label(action)
}
