// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `BackupsModel` implementation (`QAbstractListModel` for configured backups).

use super::{SyncPlan, plan_model_sync, qobject};
use crate::overall_text::localized_availability_reason;
use crate::state_cache::{
    backup_exclude_file_path_str, daemon_now, format_backup_status, format_bytes,
    format_conditions_summary, format_duration_ms, format_exclude_summary,
    format_next_scheduled_line, format_pause_line, format_retention_summary,
    format_schedule_summary, format_source_kind_label, i18n, i18np, localized_last_error,
    localized_unmet_conditions, pending_reason_line, shared_state,
};
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::model::{RetentionPreset, Run, SourceKind};
use kestic_daemon::dbus_api::{ActiveRunProgressDto, BackupSummaryDto, RunProgressPhase};
use std::pin::Pin;
use time::OffsetDateTime;

#[derive(Clone, Default, PartialEq)]
struct BackupRow {
    backup_id: String,
    name: String,
    source_path: String,
    source_kind: String,
    destination_id: String,
    destination_name: String,
    excludes_text: String,
    exclude_caches: bool,
    status_text: String,
    status_icon: String,
    status_color_kind: String,
    last_run_summary: String,
    availability_reason: String,
    available: bool,
    has_error: bool,
    is_overdue: bool,
    is_running: bool,
    active_run_id: String,
    active_percent: f64,
    active_progress_text: String,
    /// restic is still finding files: bar without end point, no percentage.
    active_scanning: bool,
    error_summary: String,
    error_action: String,
    unreadable_files_json: String,
    schedule_summary: String,
    retention_summary: String,
    conditions_summary: String,
    last_run_id: String,
    last_run_timestamp_iso: String,
    last_run_time_ms: f64,
    excludes_json: String,
    exclude_folders_count: i32,
    expert_rule_count: i32,
    exclude_summary: String,
    exclude_file_path: String,
    exclude_lint_json: String,
    retention_json: String,
    retention_preset_name: String,
    source_kind_label: String,
    drive_label: String,
    pending_forget: bool,
    nested_warnings_json: String,
    has_attention_warning: bool,
    schedule_json: String,
    conditions_json: String,
    next_scheduled_at_text: String,
    is_paused: bool,
    paused_line: String,
    pending_primary_code: String,
    pending_primary_reason: String,
    pending_reasons_tooltip: String,
    missed_slots_count: i32,
    overdue_threshold: i32,
    has_unmet_run_condition: bool,
    unmet_conditions_text: String,
}

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
        return i18np("%2 · %1 file", "%2 · %1 files", files, &[&done_str]);
    }
    let pct = p.percent_done.round().clamp(0.0, 100.0) as u32;
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

fn format_last_run_info(last_run: Option<&Run>) -> (String, String, String, f64) {
    let Some(run) = last_run else {
        return (
            i18n("No runs yet").to_string(),
            String::new(),
            String::new(),
            0.0,
        );
    };
    let dur = run
        .duration_ms
        .map(format_duration_ms)
        .unwrap_or_else(|| String::from("–"));
    let added = run
        .stats
        .as_ref()
        .map(|s| format_bytes(s.data_added))
        .unwrap_or_else(|| String::from("0 B"));
    let summary = i18n("%1 · %2 new").subs(&[&dur, &added]);
    let ts = run.finished_at.unwrap_or(run.started_at);
    let ts_iso = ts
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let ts_ms = (ts.unix_timestamp() as f64) * 1000.0;
    (summary, run.id.as_str().to_string(), ts_iso, ts_ms)
}

fn build_backup_row(
    b: &BackupSummaryDto,
    active_progress: Option<&ActiveRunProgressDto>,
    now: OffsetDateTime,
) -> BackupRow {
    let (status_text, status_icon, status_color_kind) =
        format_backup_status(b, active_progress, now);
    let active_percent = active_progress
        .map(|p| p.percent_done.clamp(0.0, 100.0))
        .unwrap_or(0.0);
    let active_progress_text = format_active_progress_text(active_progress);
    let (last_run_summary, last_run_id, last_run_timestamp_iso, last_run_time_ms) =
        format_last_run_info(b.last_run.as_ref());

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
    let has_attention_warning = b.pending_forget
        || !b.nested_warnings.is_empty()
        || !b.exclude_lint_warnings.is_empty()
        || b.is_overdue
        || b.has_error;
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
        excludes_text: b.backup.excludes.lines.join("\n"),
        exclude_caches: b.backup.excludes.exclude_caches,
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
            .map(|r| r.id.as_str().to_string())
            .unwrap_or_default(),
        active_percent,
        active_progress_text,
        active_scanning: active_progress.is_some_and(|p| p.phase == RunProgressPhase::Scanning),
        error_summary: if b.error_kind.is_some() {
            localized_last_error(b)
        } else {
            String::new()
        },
        error_action: b
            .error_action
            .map(|a| a.as_str().to_string())
            .unwrap_or_default(),
        unreadable_files_json: serde_json::to_string(&b.unreadable_files)
            .unwrap_or_else(|_| "[]".to_string()),
        schedule_summary: format_schedule_summary(&b.backup.schedule),
        retention_summary: format_retention_summary(&b.backup.retention),
        conditions_summary: format_conditions_summary(&b.backup.run_conditions),
        last_run_id,
        last_run_timestamp_iso,
        last_run_time_ms,
        excludes_json: serde_json::to_string(&b.backup.excludes)
            .unwrap_or_else(|_| "{}".to_string()),
        exclude_folders_count: i32::try_from(b.backup.excludes.folders.len()).unwrap_or(i32::MAX),
        expert_rule_count: i32::try_from(b.backup.excludes.expert_rule_count()).unwrap_or(i32::MAX),
        exclude_summary: format_exclude_summary(&b.backup.excludes),
        exclude_file_path: backup_exclude_file_path_str(&b.backup.id),
        exclude_lint_json: serde_json::to_string(&b.exclude_lint_warnings)
            .unwrap_or_else(|_| "[]".to_string()),
        retention_json: serde_json::to_string(&b.backup.retention)
            .unwrap_or_else(|_| "{}".to_string()),
        retention_preset_name,
        source_kind_label: format_source_kind_label(&b.backup.source),
        drive_label: b.backup.source.drive_label.clone().unwrap_or_default(),
        pending_forget: b.pending_forget,
        nested_warnings_json: serde_json::to_string(&b.nested_warnings)
            .unwrap_or_else(|_| "[]".to_string()),
        has_attention_warning,
        schedule_json: serde_json::to_string(&b.backup.schedule)
            .unwrap_or_else(|_| "{}".to_string()),
        conditions_json: serde_json::to_string(&b.backup.run_conditions)
            .unwrap_or_else(|_| "{}".to_string()),
        next_scheduled_at_text: format_next_scheduled_line(
            b.next_scheduled_at_unix,
            &b.backup.schedule,
            now,
        ),
        is_paused: b.is_paused,
        paused_line,
        pending_primary_code: b
            .pending_primary_code
            .map(|code| code.as_str().to_string())
            .unwrap_or_default(),
        pending_primary_reason: b
            .pending_primary_code
            .map(|code| pending_reason_line(code, b.pending_retry_at_unix, now))
            .unwrap_or_default(),
        pending_reasons_tooltip: b
            .pending_reason_codes
            .iter()
            .map(|code| pending_reason_line(*code, b.pending_retry_at_unix, now))
            .collect::<Vec<_>>()
            .join("\n"),
        missed_slots_count: i32::try_from(b.missed_slots_count).unwrap_or(i32::MAX),
        overdue_threshold: b
            .backup
            .overdue_threshold
            .map(|v| i32::try_from(v).unwrap_or(99))
            .unwrap_or(0),
        has_unmet_run_condition: b.has_unmet_run_condition,
        unmet_conditions_text: localized_unmet_conditions(b).unwrap_or_default(),
    }
}

impl BackupsModelRust {
    fn compute_rows(&self) -> Vec<BackupRow> {
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
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

impl qobject::BackupsModel {
    pub fn sync(mut self: Pin<&mut Self>) {
        let new_rows = self.as_ref().compute_rows();
        match plan_model_sync(&self.rows, new_rows, |a, b| a.backup_id == b.backup_id) {
            SyncPlan::NoChange => {}
            SyncPlan::InPlace {
                first,
                last,
                new_rows,
            } => {
                self.as_mut().rust_mut().rows = new_rows;
                let parent = QModelIndex::default();
                let top = self.as_ref().index_backups(first, 0, &parent);
                let bottom = self.as_ref().index_backups(last, 0, &parent);
                self.as_mut().data_changed_backups(&top, &bottom);
            }
            SyncPlan::Reset(new_rows) => {
                self.as_mut().begin_reset_backups();
                self.as_mut().rust_mut().rows = new_rows;
                self.as_mut().end_reset_backups();
            }
        }
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (qobject::BackupRoles::BackupId, "backupId"),
            (qobject::BackupRoles::Name, "name"),
            (qobject::BackupRoles::SourcePath, "sourcePath"),
            (qobject::BackupRoles::SourceKind, "sourceKind"),
            (qobject::BackupRoles::DestinationId, "destinationId"),
            (qobject::BackupRoles::DestinationName, "destinationName"),
            (qobject::BackupRoles::ExcludesText, "excludesText"),
            (qobject::BackupRoles::ExcludeCaches, "excludeCaches"),
            (qobject::BackupRoles::StatusText, "statusText"),
            (qobject::BackupRoles::StatusIcon, "statusIcon"),
            (qobject::BackupRoles::StatusColorKind, "statusColorKind"),
            (qobject::BackupRoles::LastRunSummary, "lastRunSummary"),
            (
                qobject::BackupRoles::AvailabilityReason,
                "availabilityReason",
            ),
            (qobject::BackupRoles::Available, "available"),
            (qobject::BackupRoles::HasError, "hasError"),
            (qobject::BackupRoles::IsOverdue, "isOverdue"),
            (qobject::BackupRoles::IsRunning, "isRunning"),
            (qobject::BackupRoles::ActiveRunId, "activeRunId"),
            (qobject::BackupRoles::ActivePercent, "activePercent"),
            (
                qobject::BackupRoles::ActiveProgressText,
                "activeProgressText",
            ),
            (qobject::BackupRoles::ErrorSummary, "errorSummary"),
            (qobject::BackupRoles::ErrorAction, "errorAction"),
            (
                qobject::BackupRoles::UnreadableFilesJson,
                "unreadableFilesJson",
            ),
            (qobject::BackupRoles::ScheduleSummary, "scheduleSummary"),
            (qobject::BackupRoles::RetentionSummary, "retentionSummary"),
            (qobject::BackupRoles::ConditionsSummary, "conditionsSummary"),
            (qobject::BackupRoles::LastRunId, "lastRunId"),
            (
                qobject::BackupRoles::LastRunTimestampIso,
                "lastRunTimestampIso",
            ),
            (qobject::BackupRoles::LastRunTimeMs, "lastRunTimeMs"),
            (qobject::BackupRoles::ExcludesJson, "excludesJson"),
            (
                qobject::BackupRoles::ExcludeFoldersCount,
                "excludeFoldersCount",
            ),
            (qobject::BackupRoles::ExpertRuleCount, "expertRuleCount"),
            (qobject::BackupRoles::ExcludeSummary, "excludeSummary"),
            (qobject::BackupRoles::ExcludeFilePath, "excludeFilePath"),
            (qobject::BackupRoles::ExcludeLintJson, "excludeLintJson"),
            (qobject::BackupRoles::RetentionJson, "retentionJson"),
            (qobject::BackupRoles::RetentionPresetName, "retentionPreset"),
            (qobject::BackupRoles::SourceKindLabel, "sourceKindLabel"),
            (qobject::BackupRoles::DriveLabel, "driveLabel"),
            (qobject::BackupRoles::PendingForget, "pendingForget"),
            (
                qobject::BackupRoles::NestedWarningsJson,
                "nestedWarningsJson",
            ),
            (
                qobject::BackupRoles::HasAttentionWarning,
                "hasAttentionWarning",
            ),
            (qobject::BackupRoles::ScheduleJson, "scheduleJson"),
            (qobject::BackupRoles::ConditionsJson, "conditionsJson"),
            (
                qobject::BackupRoles::NextScheduledAtText,
                "nextScheduledAtText",
            ),
            (qobject::BackupRoles::IsPaused, "isPaused"),
            (qobject::BackupRoles::PausedLine, "pausedLine"),
            (
                qobject::BackupRoles::PendingPrimaryCode,
                "pendingPrimaryCode",
            ),
            (
                qobject::BackupRoles::PendingPrimaryReason,
                "pendingPrimaryReason",
            ),
            (
                qobject::BackupRoles::PendingReasonsTooltip,
                "pendingReasonsTooltip",
            ),
            (qobject::BackupRoles::MissedSlotsCount, "missedSlotsCount"),
            (qobject::BackupRoles::OverdueThreshold, "overdueThreshold"),
            (
                qobject::BackupRoles::HasUnmetRunCondition,
                "hasUnmetRunCondition",
            ),
            (
                qobject::BackupRoles::UnmetConditionsText,
                "unmetConditionsText",
            ),
            (qobject::BackupRoles::ActiveScanning, "activeScanning"),
        ] {
            roles.insert(role.repr, QByteArray::from(name));
        }
        roles
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row())
            .ok()
            .and_then(|i| self.rows.get(i))
        else {
            return QVariant::default();
        };
        match (qobject::BackupRoles { repr: role }) {
            qobject::BackupRoles::BackupId => QVariant::from(&QString::from(&row.backup_id)),
            qobject::BackupRoles::Name => QVariant::from(&QString::from(&row.name)),
            qobject::BackupRoles::SourcePath => QVariant::from(&QString::from(&row.source_path)),
            qobject::BackupRoles::SourceKind => QVariant::from(&QString::from(&row.source_kind)),
            qobject::BackupRoles::DestinationId => {
                QVariant::from(&QString::from(&row.destination_id))
            }
            qobject::BackupRoles::DestinationName => {
                QVariant::from(&QString::from(&row.destination_name))
            }
            qobject::BackupRoles::ExcludesText => {
                QVariant::from(&QString::from(&row.excludes_text))
            }
            qobject::BackupRoles::ExcludeCaches => QVariant::from(&row.exclude_caches),
            qobject::BackupRoles::StatusText => QVariant::from(&QString::from(&row.status_text)),
            qobject::BackupRoles::StatusIcon => QVariant::from(&QString::from(&row.status_icon)),
            qobject::BackupRoles::StatusColorKind => {
                QVariant::from(&QString::from(&row.status_color_kind))
            }
            qobject::BackupRoles::LastRunSummary => {
                QVariant::from(&QString::from(&row.last_run_summary))
            }
            qobject::BackupRoles::AvailabilityReason => {
                QVariant::from(&QString::from(&row.availability_reason))
            }
            qobject::BackupRoles::Available => QVariant::from(&row.available),
            qobject::BackupRoles::HasError => QVariant::from(&row.has_error),
            qobject::BackupRoles::IsOverdue => QVariant::from(&row.is_overdue),
            qobject::BackupRoles::IsRunning => QVariant::from(&row.is_running),
            qobject::BackupRoles::ActiveRunId => QVariant::from(&QString::from(&row.active_run_id)),
            qobject::BackupRoles::ActivePercent => QVariant::from(&row.active_percent),
            qobject::BackupRoles::ActiveProgressText => {
                QVariant::from(&QString::from(&row.active_progress_text))
            }
            qobject::BackupRoles::ErrorSummary => {
                QVariant::from(&QString::from(&row.error_summary))
            }
            qobject::BackupRoles::ErrorAction => QVariant::from(&QString::from(&row.error_action)),
            qobject::BackupRoles::UnreadableFilesJson => {
                QVariant::from(&QString::from(&row.unreadable_files_json))
            }
            qobject::BackupRoles::ScheduleSummary => {
                QVariant::from(&QString::from(&row.schedule_summary))
            }
            qobject::BackupRoles::RetentionSummary => {
                QVariant::from(&QString::from(&row.retention_summary))
            }
            qobject::BackupRoles::ConditionsSummary => {
                QVariant::from(&QString::from(&row.conditions_summary))
            }
            qobject::BackupRoles::LastRunId => QVariant::from(&QString::from(&row.last_run_id)),
            qobject::BackupRoles::LastRunTimestampIso => {
                QVariant::from(&QString::from(&row.last_run_timestamp_iso))
            }
            qobject::BackupRoles::LastRunTimeMs => QVariant::from(&row.last_run_time_ms),
            qobject::BackupRoles::ExcludesJson => {
                QVariant::from(&QString::from(&row.excludes_json))
            }
            qobject::BackupRoles::ExcludeFoldersCount => QVariant::from(&row.exclude_folders_count),
            qobject::BackupRoles::ExpertRuleCount => QVariant::from(&row.expert_rule_count),
            qobject::BackupRoles::ExcludeSummary => {
                QVariant::from(&QString::from(&row.exclude_summary))
            }
            qobject::BackupRoles::ExcludeFilePath => {
                QVariant::from(&QString::from(&row.exclude_file_path))
            }
            qobject::BackupRoles::ExcludeLintJson => {
                QVariant::from(&QString::from(&row.exclude_lint_json))
            }
            qobject::BackupRoles::RetentionJson => {
                QVariant::from(&QString::from(&row.retention_json))
            }
            qobject::BackupRoles::RetentionPresetName => {
                QVariant::from(&QString::from(&row.retention_preset_name))
            }
            qobject::BackupRoles::SourceKindLabel => {
                QVariant::from(&QString::from(&row.source_kind_label))
            }
            qobject::BackupRoles::DriveLabel => QVariant::from(&QString::from(&row.drive_label)),
            qobject::BackupRoles::PendingForget => QVariant::from(&row.pending_forget),
            qobject::BackupRoles::NestedWarningsJson => {
                QVariant::from(&QString::from(&row.nested_warnings_json))
            }
            qobject::BackupRoles::HasAttentionWarning => QVariant::from(&row.has_attention_warning),
            qobject::BackupRoles::ScheduleJson => {
                QVariant::from(&QString::from(&row.schedule_json))
            }
            qobject::BackupRoles::ConditionsJson => {
                QVariant::from(&QString::from(&row.conditions_json))
            }
            qobject::BackupRoles::NextScheduledAtText => {
                QVariant::from(&QString::from(&row.next_scheduled_at_text))
            }
            qobject::BackupRoles::IsPaused => QVariant::from(&row.is_paused),
            qobject::BackupRoles::PausedLine => QVariant::from(&QString::from(&row.paused_line)),
            qobject::BackupRoles::PendingPrimaryCode => {
                QVariant::from(&QString::from(&row.pending_primary_code))
            }
            qobject::BackupRoles::PendingPrimaryReason => {
                QVariant::from(&QString::from(&row.pending_primary_reason))
            }
            qobject::BackupRoles::PendingReasonsTooltip => {
                QVariant::from(&QString::from(&row.pending_reasons_tooltip))
            }
            qobject::BackupRoles::MissedSlotsCount => QVariant::from(&row.missed_slots_count),
            qobject::BackupRoles::OverdueThreshold => QVariant::from(&row.overdue_threshold),
            qobject::BackupRoles::HasUnmetRunCondition => {
                QVariant::from(&row.has_unmet_run_condition)
            }
            qobject::BackupRoles::UnmetConditionsText => {
                QVariant::from(&QString::from(&row.unmet_conditions_text))
            }
            qobject::BackupRoles::ActiveScanning => QVariant::from(&row.active_scanning),
            _ => QVariant::default(),
        }
    }
}
