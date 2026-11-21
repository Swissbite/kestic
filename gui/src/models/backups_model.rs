// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `BackupsModel` implementation (`QAbstractListModel` for configured backups).

use super::qobject;
use super::rows::{id_at, row_map, row_of};
use crate::overall_text::localized_availability_reason;
use crate::state_cache::{
    backup_exclude_file_path_str, daemon_now, format_backup_status, format_bytes,
    format_conditions_summary, format_duration_ms, format_exclude_summary,
    format_next_scheduled_line, format_pause_line, format_retention_summary,
    format_schedule_summary, format_source_kind_label, i18n, i18np, json_or, localized_last_error,
    pending_reason_line, shared_state,
};
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::model::{RetentionPreset, Run, SourceKind};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{ActiveRunProgressDto, BackupSummaryDto, RunProgressPhase};
use time::OffsetDateTime;

model_row!(BackupRow {
    backup_id: String => "backupId",
    name: String => "name",
    source_path: String => "sourcePath",
    source_kind: String => "sourceKind",
    destination_id: String => "destinationId",
    destination_name: String => "destinationName",
    status_text: String => "statusText",
    status_icon: String => "statusIcon",
    status_color_kind: String => "statusColorKind",
    last_run_summary: String => "lastRunSummary",
    availability_reason: String => "availabilityReason",
    available: bool => "available",
    has_error: bool => "hasError",
    is_overdue: bool => "isOverdue",
    is_running: bool => "isRunning",
    active_run_id: String => "activeRunId",
    active_percent: f64 => "activePercent",
    active_progress_text: String => "activeProgressText",
    // restic is still finding files: bar without end point, no percentage.
    active_scanning: bool => "activeScanning",
    error_summary: String => "errorSummary",
    error_action: String => "errorAction",
    schedule_summary: String => "scheduleSummary",
    retention_summary: String => "retentionSummary",
    conditions_summary: String => "conditionsSummary",
    excludes_json: String => "excludesJson",
    exclude_summary: String => "excludeSummary",
    exclude_file_path: String => "excludeFilePath",
    exclude_lint_json: String => "excludeLintJson",
    retention_json: String => "retentionJson",
    retention_preset_name: String => "retentionPreset",
    source_kind_label: String => "sourceKindLabel",
    pending_forget: bool => "pendingForget",
    nested_warnings_json: String => "nestedWarningsJson",
    schedule_json: String => "scheduleJson",
    conditions_json: String => "conditionsJson",
    next_scheduled_at_text: String => "nextScheduledAtText",
    is_paused: bool => "isPaused",
    paused_line: String => "pausedLine",
    pending_primary_code: String => "pendingPrimaryCode",
    pending_primary_reason: String => "pendingPrimaryReason",
    pending_reasons_tooltip: String => "pendingReasonsTooltip",
    overdue_threshold: i32 => "overdueThreshold",
});

#[derive(Default)]
pub struct BackupsModelRust {
    rows: Vec<BackupRow>,
}

/// Line below the progress bar: the amount looked through while restic still finds files,
/// afterwards percentage, amount and, when restic knows it, the time left.
fn format_active_progress_text(active_progress: Option<&ActiveRunProgressDto>) -> String {
    let Some(p) = active_progress else {
        return String::new();
    };
    let done_str = format_bytes(p.bytes_done);
    if p.phase == RunProgressPhase::Scanning {
        let files = usize::try_from(p.files_done).unwrap_or(usize::MAX);
        return i18np(
            "%2 · %1 file so far",
            "%2 · %1 files so far",
            files,
            &[&done_str],
        );
    }
    let pct = kestic_i18n::messages::rounded_percent(p.percent_done);
    let total_str = format_bytes(p.total_bytes);
    if p.total_bytes == 0 {
        return i18n("%1%").subs(&[&pct.to_string()]);
    }
    if p.seconds_remaining > 0 {
        let left = format_duration_ms(p.seconds_remaining.saturating_mul(1000));
        return i18n("%1% · %2 of %3 · about %4 left").subs(&[
            &pct.to_string(),
            &done_str,
            &total_str,
            &left,
        ]);
    }
    i18n("%1% · %2 of %3").subs(&[&pct.to_string(), &done_str, &total_str])
}

fn format_last_run_summary(last_run: Option<&Run>) -> String {
    let Some(run) = last_run else {
        return i18n("No runs yet").to_string();
    };
    let dur = run
        .duration_ms
        .map_or_else(|| String::from("–"), format_duration_ms);
    let added = run
        .stats
        .as_ref()
        .map_or_else(|| String::from("0 B"), |s| format_bytes(s.data_added));
    i18n("%1 · %2 new").subs(&[&dur, &added])
}

fn build_backup_row(
    b: &BackupSummaryDto,
    active_progress: Option<&ActiveRunProgressDto>,
    now: OffsetDateTime,
) -> BackupRow {
    let (status_text, status_icon, status_color_kind) =
        format_backup_status(b, active_progress, now);
    let active_percent = active_progress.map_or(0.0, |p| p.percent_done.clamp(0.0, 100.0));
    let active_progress_text = format_active_progress_text(active_progress);
    let last_run_summary = format_last_run_summary(b.last_run.as_ref());

    let source_kind = match b.backup.source.kind {
        SourceKind::Drive => "drive",
        SourceKind::Folder => "folder",
    }
    .to_string();
    let retention_preset_name = match b.backup.retention.preset() {
        RetentionPreset::Standard => "standard",
        RetentionPreset::Economical => "economical",
        RetentionPreset::Long => "long",
        RetentionPreset::Custom => "custom",
    }
    .to_string();
    let paused_line = if b.is_paused {
        format_pause_line(b.paused_since_unix, b.paused_until_unix, now)
    } else {
        String::new()
    };

    BackupRow {
        backup_id: b.backup.id.as_str().to_string(),
        name: b.backup.name.clone(),
        source_path: b.backup.source.path.to_string_lossy().into_owned(),
        source_kind,
        destination_id: b.backup.destination_id.as_str().to_string(),
        destination_name: b.destination_name.clone(),
        status_text,
        status_icon,
        status_color_kind,
        last_run_summary,
        availability_reason: localized_availability_reason(&b.availability),
        available: b.availability.available,
        has_error: b.has_error,
        is_overdue: b.is_overdue,
        is_running: b.active_run.is_some(),
        active_run_id: b
            .active_run
            .as_ref()
            .map_or_default(|r| r.id.as_str().to_string()),
        active_percent,
        active_progress_text,
        active_scanning: active_progress.is_some_and(|p| p.phase == RunProgressPhase::Scanning),
        error_summary: if b.error_kind.is_some() {
            localized_last_error(b)
        } else {
            String::new()
        },
        error_action: b.error_action.map_or_default(|a| a.as_str().to_string()),
        schedule_summary: format_schedule_summary(&b.backup.schedule),
        retention_summary: format_retention_summary(&b.backup.retention),
        conditions_summary: format_conditions_summary(&b.backup.run_conditions),
        excludes_json: json_or(&b.backup.excludes, "{}"),
        exclude_summary: format_exclude_summary(&b.backup.excludes),
        exclude_file_path: backup_exclude_file_path_str(&b.backup.id),
        exclude_lint_json: json_or(&b.exclude_lint_warnings, "[]"),
        retention_json: json_or(&b.backup.retention, "{}"),
        retention_preset_name,
        source_kind_label: format_source_kind_label(&b.backup.source),
        pending_forget: b.pending_forget,
        nested_warnings_json: json_or(&b.nested_warnings, "[]"),
        schedule_json: json_or(&b.backup.schedule, "{}"),
        conditions_json: json_or(&b.backup.run_conditions, "{}"),
        next_scheduled_at_text: format_next_scheduled_line(
            b.next_scheduled_at_unix,
            &b.backup.schedule,
            now,
        ),
        is_paused: b.is_paused,
        paused_line,
        pending_primary_code: b
            .pending_primary_code
            .map_or_default(|code| code.as_str().to_string()),
        pending_primary_reason: b
            .pending_primary_code
            .map_or_default(|code| pending_reason_line(code, b.pending_retry_at_unix, now)),
        pending_reasons_tooltip: b
            .pending_reason_codes
            .iter()
            .map(|code| pending_reason_line(*code, b.pending_retry_at_unix, now))
            .collect::<Vec<_>>()
            .join("\n"),
        overdue_threshold: b
            .backup
            .overdue_threshold
            .map_or(0, |v| i32::try_from(v).unwrap_or(99)),
    }
}

impl BackupsModelRust {
    fn compute_rows(&self) -> Vec<BackupRow> {
        let guard = shared_state().lock_unpoisoned();
        let Some(state) = &guard.state else {
            return Vec::new();
        };
        let now = daemon_now(state);
        state
            .backups
            .iter()
            .map(|b| {
                let active_progress = b
                    .active_run
                    .as_ref()
                    .and_then(|r| guard.active_progress.get(r.id.as_str()));
                build_backup_row(b, active_progress, now)
            })
            .collect()
    }
}

list_model!(
    BackupsModel,
    BackupRow,
    same: |a, b| a.backup_id == b.backup_id,
    begin_reset: begin_reset_backups,
    end_reset: end_reset_backups,
    index: index_backups,
    changed: data_changed_backups,
);

impl qobject::BackupsModel {
    /// Row of the backup `backup_id`, or -1 if unknown.
    pub fn row_of(&self, backup_id: &QString) -> i32 {
        let backup_id = backup_id.to_string();
        row_of(&self.rows, |row| row.backup_id == backup_id)
    }

    /// Id of the backup in `row`, empty if there is no such row.
    pub fn id_at(&self, row: i32) -> QString {
        id_at(&self.rows, row, |row| &row.backup_id)
    }

    /// All roles of the backup `backup_id` as a QML object, or an invalid value if unknown.
    pub fn row_map(&self, backup_id: &QString) -> QVariant {
        let backup_id = backup_id.to_string();
        row_map(self.rows.iter().find(|row| row.backup_id == backup_id))
    }
}
