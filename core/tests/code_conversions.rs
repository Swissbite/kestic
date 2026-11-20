// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! `kestic-core` converts its domain types into the shared codes of `kestic-i18n`.

use kestic_core::mount::SourceCheckResult;
use kestic_core::restic::ResticFailureKind;
use kestic_core::scheduler::UnmetPrecondition;
use kestic_i18n::codes::{ErrorKind, PendingReasonCode, SourceStatus};

#[test]
fn restic_failure_kinds_map_to_error_kinds() {
    assert_eq!(
        ResticFailureKind::DnsResolutionFailed.error_kind(),
        ErrorKind::DestinationUnreachable
    );
    assert_eq!(
        ResticFailureKind::RepositoryLocked.error_kind(),
        ErrorKind::Locked
    );
    assert_eq!(
        ResticFailureKind::Unknown { exit_code: 99 }.error_kind(),
        ErrorKind::Unknown
    );
}

#[test]
fn restic_failure_summary_keeps_dns_and_exit_code_detail() {
    assert_eq!(
        ResticFailureKind::DnsResolutionFailed.default_summary(),
        "Destination name not found (DNS)"
    );
    assert_eq!(
        ResticFailureKind::Unknown { exit_code: 99 }.default_summary(),
        "Unknown error (exit 99)"
    );
    assert_eq!(ResticFailureKind::None.default_summary(), "Completed");
}

#[test]
fn unmet_preconditions_and_source_checks_map_to_codes() {
    assert_eq!(
        PendingReasonCode::from(&UnmetPrecondition::MeteredNetwork),
        PendingReasonCode::Metered
    );
    assert_eq!(
        PendingReasonCode::from(&UnmetPrecondition::Queued),
        PendingReasonCode::Queue
    );
    assert_eq!(
        SourceCheckResult::DriveNotMounted.status_code(),
        Some(SourceStatus::DriveNotMounted)
    );
    assert_eq!(SourceCheckResult::Available.status_code(), None);
}
