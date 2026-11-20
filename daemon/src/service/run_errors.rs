// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Error information of finished runs for state DTOs: permanent-vs-transient classification
//! (`F2`), stable error-kind codes for the GUI (`error_kind`, locale independent), and the
//! unreadable-file list of incomplete snapshots (cached, never read from disk under the lock).
//!
//! The restic exit/text classification itself lives in `kestic_core::restic::classify_exit`
//! (keyword matching) and `ResticFailureKind::{is_transient, is_permanent, error_kind}`; this
//! module only maps a stored [`Run`] onto it.

use kestic_core::model::{PendingReason, Run, RunStatus};
use kestic_core::restic::{
    RecoveryAction, ResticFailureKind, ResticStreamEvent, classify_exit, parse_json_line,
};
use kestic_i18n::codes::ErrorKind;

/// Error details of the last finished run of a backup.
pub(super) struct LastRunErrorInfo {
    pub(super) has_error: bool,
    /// Last run ended with a permanent error that blocks scheduled runs (`F2`).
    pub(super) blocks_schedule: bool,
    pub(super) error_action: Option<RecoveryAction>,
    pub(super) error_kind: Option<ErrorKind>,
    pub(super) error_detail: Option<String>,
    pub(super) error_exit_code: Option<i32>,
    pub(super) wrong_password: bool,
    pub(super) unreadable_files: Vec<String>,
}

/// Whether `run` (the last finished backup run) ended with a **permanent** error that blocks
/// scheduled runs until fixed (`F2`: Exit 12 wrong password, Exit 10 missing repository,
/// rejected backend credentials, see [`ResticFailureKind::is_permanent`]).
///
/// Transient failures (network/destination unreachable, DNS, lock / Exit 11), other errors
/// (e.g. disk full, unknown exit codes: the next regular slot runs again) and pre-start
/// availability failures (source unavailable, destination unreachable — recorded without exit
/// code, `FR-29`, `FR-30`) never block the schedule, also when they ended a manual run.
pub(super) fn run_blocks_schedule(run: &Run) -> bool {
    if run.status != RunStatus::Error {
        return false;
    }
    let Some(exit_code) = run.exit_code else {
        return false;
    };
    let raw_detail = run.summary_message.as_deref().unwrap_or("");
    classify_exit(exit_code, None, Vec::new(), raw_detail)
        .kind
        .is_permanent()
}

/// Error kind of an `Error` run (see [`ResticFailureKind::error_kind`]).
///
/// Runs without exit code failed before restic started; their cause is recorded in
/// `pending_reason` (source unavailable / destination unreachable), anything else is `unknown`.
pub(crate) fn error_run_kind(run: &Run) -> ErrorKind {
    match run.exit_code {
        Some(code) => {
            let raw_detail = run.summary_message.as_deref().unwrap_or("");
            classify_exit(code, None, Vec::new(), raw_detail)
                .kind
                .error_kind()
        }
        None => match run.pending_reason {
            Some(PendingReason::SourceUnavailable) => ErrorKind::SourceUnavailable,
            Some(PendingReason::DestinationUnreachable) => ErrorKind::DestinationUnreachable,
            _ => ErrorKind::Unknown,
        },
    }
}

/// Collects the unreadable items (`message_type: error` with `item`) from restic JSON log lines.
pub(crate) fn unreadable_items_from_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    lines
        .into_iter()
        .filter_map(|line| match parse_json_line(line) {
            Ok(ResticStreamEvent::Error(err)) => err.item,
            _ => None,
        })
        .collect()
}

/// Builds the error info of `last_run`; `unreadable` is the cached unreadable-file list of that
/// run (no file I/O here: this runs under the `DaemonCore` lock).
pub(super) fn extract_last_run_error_info(
    last_run: Option<&Run>,
    pending_forget: bool,
    unreadable: Option<&Vec<String>>,
) -> LastRunErrorInfo {
    let mut info = LastRunErrorInfo {
        has_error: false,
        blocks_schedule: false,
        error_action: None,
        error_kind: None,
        error_detail: None,
        error_exit_code: None,
        wrong_password: false,
        unreadable_files: Vec::new(),
    };
    let Some(run) = last_run else {
        return info;
    };
    match run.status {
        RunStatus::Error => {
            info.has_error = true;
            info.blocks_schedule = run_blocks_schedule(run);
            let exit_code = run.exit_code.unwrap_or(1);
            let raw_detail = run.summary_message.as_deref().unwrap_or("");
            let pres = classify_exit(exit_code, None, Vec::new(), raw_detail);
            info.error_action = pres.action;
            info.error_kind = Some(error_run_kind(run));
            info.error_detail = run.summary_message.clone();
            info.error_exit_code = run.exit_code;
            info.wrong_password = matches!(pres.kind, ResticFailureKind::WrongPassword);
        }
        RunStatus::Warning => {
            info.unreadable_files = unreadable.cloned().unwrap_or_default();
            info.error_detail = run.summary_message.clone();
            info.error_exit_code = run.exit_code;
            if run.exit_code == Some(3) || !info.unreadable_files.is_empty() {
                info.error_action = Some(RecoveryAction::AddToExcludes);
                info.error_kind = Some(ErrorKind::IncompleteSnapshot);
            } else if pending_forget {
                info.error_kind = Some(ErrorKind::RetentionPending);
            } else {
                info.error_kind = Some(ErrorKind::Unknown);
            }
        }
        _ => {}
    }
    info
}
