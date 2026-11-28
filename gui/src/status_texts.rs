// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Status lines and messages of backups, destinations and runs.

use crate::i18n::{format_percent, i18n, i18np, tr, tr_text};
use crate::overall_text::localized_availability_reason;
use crate::summary_texts::{format_pause_line, format_relative_time, format_slot_timestamp};
use crate::ui_texts::GuiText;
use kestic_core::model::{Run, RunKind, RunStatus, Schedule};
use kestic_daemon::dbus_api::{
    ActiveRunProgressDto, BackupAvailabilityDto, BackupSummaryDto, DestinationSummaryDto,
    OverallStatusKind, StartAllResultDto,
};
use kestic_i18n::codes::ErrorKind;
use kestic_i18n::codes::{BlockedReason, PendingReasonCode, RunProgressPhase};
use kestic_i18n::messages::{
    completed_message, interrupted_by_sleep_message, not_backed_up_message,
    unmet_conditions_message,
};
use time::OffsetDateTime;

/// Waiting reason in the GUI locale from the daemon's reason code; the backoff reason uses
/// `retry_at_unix` (`pending_retry_at_unix`).
pub fn pending_reason_line(
    code: PendingReasonCode,
    retry_at_unix: Option<i64>,
    now: OffsetDateTime,
) -> String {
    tr(&code.waiting_message(&format_slot_timestamp(retry_at_unix, now)))
}

/// Cause of the last run's error/warning of `summary` in the GUI locale (`error_kind`).
pub fn localized_last_error(summary: &BackupSummaryDto) -> String {
    let is_warning = summary
        .last_run
        .as_ref()
        .is_some_and(|r| r.status == RunStatus::Warning);
    match summary.error_kind {
        Some(kind) => tr(&kind.message(summary.error_exit_code, is_warning)),
        None if is_warning => String::new(),
        None => i18n("Backup failed").to_string(),
    }
}

/// Unmet run conditions of `summary` as «Condition not met: …» in the GUI locale.
pub fn localized_unmet_conditions(summary: &BackupSummaryDto) -> Option<String> {
    if summary.unmet_run_conditions.is_empty() {
        return None;
    }
    let allowed = &summary.backup.run_conditions.allowed_networks;
    let parts: Vec<String> = summary
        .unmet_run_conditions
        .iter()
        .map(|code| tr(&code.condition_message(summary.battery_percent, allowed)))
        .collect();
    Some(tr(&unmet_conditions_message(&parts.join(", "))))
}

/// Status line of a backup whose source or destination is unavailable.
///
/// A manual-only backup is never due, so it never waits (glossary «wartet»): it shows the
/// availability only, e.g. «Manual only · Destination unreachable».
fn unavailable_status(
    schedule: &Schedule,
    availability: &BackupAvailabilityDto,
) -> (String, String, String) {
    if matches!(schedule, Schedule::ManualOnly) {
        let text = GuiText::TwoParts.with_args(&[
            &GuiText::ManualOnly.text(),
            &localized_availability_reason(availability),
        ]);
        return (text, "state-information".to_string(), "text".to_string());
    }
    let reason_line = tr(&BlockedReason::waiting_message(
        availability.blocked_reason,
        availability.source_status,
    ));
    (reason_line, "chronometer".to_string(), "text".to_string())
}

pub fn format_backup_status(
    summary: &BackupSummaryDto,
    active_progress: Option<&ActiveRunProgressDto>,
    now: OffsetDateTime,
) -> (String, String, String) {
    if summary.active_run.is_some() {
        let text = match active_progress {
            Some(p) if p.phase == RunProgressPhase::ScanFinished => {
                let pct = kestic_i18n::messages::rounded_percent(p.percent_done);
                i18n("Backing up · %1").subs(&[&format_percent(pct)])
            }
            _ => i18n("Backing up · looking through files").to_string(),
        };
        return (text, "state-sync".to_string(), "highlight".to_string());
    }

    if summary.has_error
        && let Some(run) = &summary.last_run
        && run.status == RunStatus::Error
    {
        let cause = localized_last_error(summary);
        let text = i18n("Failed: %1").subs(&[&cause]);
        return (text, "state-error".to_string(), "negative".to_string());
    }

    if summary.is_overdue {
        let text = if summary.missed_slots_count > 0 {
            i18n("Overdue (%1 missed)").subs(&[&summary.missed_slots_count.to_string()])
        } else {
            i18n("Overdue").to_string()
        };
        return (text, "alarm-symbolic".to_string(), "negative".to_string());
    }

    if summary.is_paused {
        return (
            format_pause_line(summary.paused_since_unix, summary.paused_until_unix, now),
            "state-pause".to_string(),
            "text".to_string(),
        );
    }

    if let Some(code) = summary.pending_primary_code {
        return (
            pending_reason_line(code, summary.pending_retry_at_unix, now),
            "chronometer".to_string(),
            "text".to_string(),
        );
    }

    if !summary.availability.available {
        return unavailable_status(&summary.backup.schedule, &summary.availability);
    }

    if let Some(run) = &summary.last_run {
        match run.status {
            RunStatus::Error => {
                let cause = localized_last_error(summary);
                let text = i18n("Failed: %1").subs(&[&cause]);
                return (text, "state-error".to_string(), "negative".to_string());
            }
            RunStatus::Warning => {
                let count = summary.unreadable_files.len();
                let text = if count > 0 {
                    i18np(
                        "Backed up, %1 file unreadable",
                        "Backed up, %1 files unreadable",
                        count,
                        &[],
                    )
                } else if summary.pending_forget {
                    tr(&ErrorKind::RetentionPending.message(None, false))
                } else {
                    tr(&ErrorKind::Unknown.message(None, true))
                };
                return (text, "state-warning".to_string(), "neutral".to_string());
            }
            RunStatus::Ok => {
                let rel = format_relative_time(run.finished_at.unwrap_or(run.started_at), now);
                let text = i18n("Backed up · %1").subs(&[&rel]);
                return (text, "state-ok".to_string(), "positive".to_string());
            }
            RunStatus::Cancelled => {
                return (
                    i18n("Cancelled").to_string(),
                    "process-stop".to_string(),
                    "text".to_string(),
                );
            }
            RunStatus::Interrupted => {
                return (
                    tr(&interrupted_by_sleep_message()),
                    "state-offline".to_string(),
                    "text".to_string(),
                );
            }
            RunStatus::Pending => {
                return (
                    i18n("Waiting").to_string(),
                    "chronometer".to_string(),
                    "text".to_string(),
                );
            }
            RunStatus::Running => {
                return (
                    i18n("Backing up · %1").subs(&[&format_percent(0)]),
                    "state-sync".to_string(),
                    "highlight".to_string(),
                );
            }
        }
    }

    (
        tr(&not_backed_up_message()),
        "state-information".to_string(),
        "text".to_string(),
    )
}

pub fn format_destination_status(
    summary: &DestinationSummaryDto,
    now: OffsetDateTime,
) -> (String, String) {
    if summary.password_rejected {
        return (
            tr(&ErrorKind::WrongPassword.message(None, false)),
            "state-error".to_string(),
        );
    }
    if summary.has_stale_lock {
        return (
            i18n("Lock from an interrupted run").to_string(),
            "object-locked".to_string(),
        );
    }
    if !summary.availability.reachable {
        return (
            tr(&ErrorKind::DestinationUnreachable.message(None, false)),
            "network-disconnect".to_string(),
        );
    }
    let checked =
        OffsetDateTime::from_unix_timestamp(summary.availability.checked_at_unix).unwrap_or(now);
    let rel = format_relative_time(checked, now);
    let text = i18n("Reachable · last checked %1").subs(&[&rel]);
    (text, "network-connect".to_string())
}

pub fn format_overall_visual(kind: OverallStatusKind) -> (&'static str, &'static str) {
    match kind {
        OverallStatusKind::Blocked | OverallStatusKind::Error => ("state-error", "negative"),
        OverallStatusKind::Overdue => ("alarm-symbolic", "negative"),
        OverallStatusKind::Warning => ("state-warning", "neutral"),
        OverallStatusKind::Running => ("state-sync", "highlight"),
        OverallStatusKind::Paused => ("state-pause", "text"),
        OverallStatusKind::Pending => ("chronometer", "text"),
        OverallStatusKind::Interrupted => ("state-offline", "text"),
        OverallStatusKind::Cancelled => ("process-stop", "text"),
        OverallStatusKind::NeverRun | OverallStatusKind::ManualOnly => {
            ("state-information", "text")
        }
        OverallStatusKind::Ok => ("state-ok", "positive"),
    }
}

pub fn format_run_status(
    kind: &RunKind,
    status: RunStatus,
) -> (String, &'static str, &'static str) {
    match status {
        // A maintenance run (`forget`/`prune`/`check`) backs nothing up.
        RunStatus::Ok if matches!(kind, RunKind::Maintenance) => {
            (tr(&completed_message()), "state-ok", "positive")
        }
        RunStatus::Ok => (i18n("Backed up").to_string(), "state-ok", "positive"),
        RunStatus::Warning => (i18n("Warning").to_string(), "state-warning", "neutral"),
        RunStatus::Error => (i18n("Failed").to_string(), "state-error", "negative"),
        RunStatus::Running => (i18n("Running").to_string(), "state-sync", "highlight"),
        RunStatus::Pending => (i18n("Waiting").to_string(), "chronometer", "text"),
        RunStatus::Cancelled => (i18n("Cancelled").to_string(), "process-stop", "text"),
        RunStatus::Interrupted => (GuiText::Interrupted.text(), "state-offline", "text"),
    }
}

/// Title of a run in the history; a backup that no longer exists is named as deleted, never
/// by its internal ID.
pub fn format_run_kind_name(run: &Run, backup_name: Option<&str>, dest_name: &str) -> String {
    match &run.kind {
        RunKind::Backup { .. } => {
            backup_name.map_or_else(|| GuiText::DeletedBackup.text(), ToOwned::to_owned)
        }
        RunKind::Maintenance => i18n("Maintenance %1").subs(&[dest_name]),
        RunKind::Restore { .. } => i18n("Restore %1").subs(&[backup_name.unwrap_or(dest_name)]),
        RunKind::Meta => i18n("Metadata %1").subs(&[dest_name]),
    }
}

pub fn localized_manual_block_message(backup_name: &str, reason: BlockedReason) -> String {
    tr(&reason.manual_block_message(backup_name))
}

pub fn localized_condition_override_message(backup_name: &str, unmet_text: Option<&str>) -> String {
    if let Some(detail) = unmet_text.filter(|s| !s.trim().is_empty()) {
        i18n("Run conditions for '%1' are currently not met (%2). Start backup anyway?")
            .subs(&[backup_name, detail])
    } else {
        i18n("Run conditions for '%1' are currently not met. Start backup anyway?")
            .subs(&[backup_name])
    }
}

/// Localized reason for a skipped backup of «Back Up All Now».
fn localized_skip_reason(skipped: &kestic_daemon::dbus_api::SkippedBackupDto) -> String {
    tr_text(
        &skipped
            .reason
            .text(skipped.error_kind, skipped.error_detail.as_deref()),
    )
}

pub fn localized_start_all_summary(res: &StartAllResultDto) -> Option<String> {
    let queued = res.queued_backup_ids.len();
    let skipped = res.skipped.len();
    if queued == 0 && skipped == 0 {
        return None;
    }
    let details: Vec<String> = res
        .skipped
        .iter()
        .map(|s| format!("{} ({})", s.backup_name, localized_skip_reason(s)))
        .collect();
    let details = details.join(", ");
    if skipped == 0 {
        return Some(i18np(
            "%1 backup waits for the current run on its destination",
            "%1 backups wait for the current run on their destination",
            queued,
            &[],
        ));
    }
    let skipped_text = i18np(
        "%1 backup skipped: %2",
        "%1 backups skipped: %2",
        skipped,
        &[&details],
    );
    if queued == 0 {
        return Some(skipped_text);
    }
    Some(i18np(
        "%1 backup queued, %2",
        "%1 backups queued, %2",
        queued,
        &[&skipped_text],
    ))
}

#[cfg(test)]
#[path = "status_texts_tests.rs"]
mod tests;
