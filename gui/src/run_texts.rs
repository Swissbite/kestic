// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Texts of the run detail page and the history rows that differ by run kind.

use crate::state_cache::{
    format_bytes, format_datetime_display, format_run_kind_name, format_run_status, i18n, i18np, tr,
};
use crate::ui_texts::GuiText;
use kestic_core::model::{BackupId, MaintenanceOutcome, MaintenanceStats, Run, RunKind, RunStatus};
use kestic_i18n::messages::completed_message;

/// Page title: «Lauf vom …» for backups, «Wartung vom …» for maintenance runs.
pub fn run_detail_title(run: &Run) -> String {
    let started = format_datetime_display(run.started_at);
    match run.kind {
        RunKind::Maintenance => i18n("Maintenance on %1").subs(&[&started]),
        _ => i18n("Run on %1").subs(&[&started]),
    }
}

/// Status text, icon and colour kind; a successful maintenance run names the steps it
/// completed («Aufgeräumt und geprüft»).
pub fn run_status(run: &Run) -> (String, &'static str, &'static str) {
    let (text, icon, color) = format_run_status(&run.kind, run.status);
    let (RunKind::Maintenance, RunStatus::Ok) = (&run.kind, run.status) else {
        return (text, icon, color);
    };
    let outcome = run.maintenance.as_ref().map(MaintenanceStats::outcome);
    let text = match outcome {
        Some(MaintenanceOutcome::CleanedUpAndVerified) => {
            i18n("Cleaned up and verified").to_string()
        }
        Some(MaintenanceOutcome::CleanedUp) => i18n("Cleaned up").to_string(),
        Some(MaintenanceOutcome::Verified) => i18n("Verified").to_string(),
        Some(MaintenanceOutcome::RetentionOnly) | None => tr(&completed_message()),
    };
    (text, icon, color)
}

/// Line below the status: backup and added data for backups, the destination for maintenance.
pub fn run_detail_subtitle(run: &Run, backup_name: Option<&str>, dest_name: &str) -> String {
    match run.kind {
        RunKind::Maintenance => dest_name.to_string(),
        _ => {
            let added = format_bytes(run.stats.as_ref().map_or(0, |s| s.data_added));
            i18n("%1 · %2 added")
                .subs(&[&format_run_kind_name(run, backup_name, dest_name), &added])
        }
    }
}

/// Content of the «Result» tab; `backup_name` names a backup by its ID (`None` once it is
/// deleted). A maintenance run that is still active has no result yet.
pub fn run_result_summary(run: &Run, backup_name: &dyn Fn(&BackupId) -> Option<String>) -> String {
    if let RunKind::Maintenance = run.kind {
        if matches!(run.status, RunStatus::Running | RunStatus::Pending) {
            return String::new();
        }
        return run.maintenance.as_ref().map_or_else(
            || i18n("No result recorded for this maintenance run.").to_string(),
            |stats| maintenance_result(stats, run.status, backup_name),
        );
    }
    match &run.stats {
        Some(s) => i18n("%1 new · %2 changed · %3 unchanged · %4 processed").subs(&[
            &s.files_new.to_string(),
            &s.files_changed.to_string(),
            &s.files_unmodified.to_string(),
            &format_bytes(s.total_bytes_processed),
        ]),
        None => i18n("No file statistics available for this run.").to_string(),
    }
}

/// Removed snapshots per backup, then the result of the check, one line each. «No snapshots
/// removed» only for a successful run: after a failed `forget` it would not be true.
fn maintenance_result(
    stats: &MaintenanceStats,
    status: RunStatus,
    backup_name: &dyn Fn(&BackupId) -> Option<String>,
) -> String {
    let mut lines: Vec<String> = stats
        .forgets
        .iter()
        .map(|f| {
            let name = backup_name(&f.backup_id).unwrap_or_else(|| GuiText::DeletedBackup.text());
            f.removed_snapshots.map_or_else(
                || i18n("%1: snapshots removed").subs(&[&name]),
                |n| {
                    i18np(
                        "%2: %1 snapshot removed",
                        "%2: %1 snapshots removed",
                        n,
                        &[&name],
                    )
                },
            )
        })
        .collect();
    if lines.is_empty() && status == RunStatus::Ok {
        lines.push(i18n("No snapshots removed.").to_string());
    }
    if let Some(check) = &stats.check {
        lines.push(if check.num_errors == 0 {
            i18n("Verify: no errors found").to_string()
        } else {
            i18np(
                "Verify: %1 error found",
                "Verify: %1 errors found",
                check.num_errors,
                &[],
            )
        });
    }
    lines.join("\n")
}

#[cfg(test)]
#[path = "run_texts_tests.rs"]
mod tests;
