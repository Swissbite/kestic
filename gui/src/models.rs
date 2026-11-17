// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `QAbstractListModel` bridges exposing backups, destinations, runs, snapshots,
//! and redacted run logs to Kirigami/QML views.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++Qt" {
        include!(<QtCore/QAbstractListModel>);
        #[qobject]
        type QAbstractListModel;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;

        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;

        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[qenum(BackupsModel)]
    enum BackupRoles {
        BackupId,
        Name,
        SourcePath,
        SourceKind,
        DestinationId,
        DestinationName,
        ExcludesText,
        ExcludeCaches,
        StatusText,
        StatusIcon,
        StatusColorKind,
        LastRunSummary,
        AvailabilityReason,
        Available,
        HasError,
        IsOverdue,
        IsRunning,
        ActiveRunId,
        ActivePercent,
        ActiveProgressText,
        ErrorSummary,
        ErrorAction,
        UnreadableFilesJson,
        ScheduleSummary,
        RetentionSummary,
        ConditionsSummary,
        LastRunId,
        LastRunTimestampIso,
        LastRunTimeMs,
        ExcludesJson,
        ExcludeFoldersCount,
        ExpertRuleCount,
        ExcludeSummary,
        ExcludeFilePath,
        ExcludeLintJson,
        RetentionJson,
        RetentionPresetName,
        SourceKindLabel,
        DriveLabel,
        PendingForget,
        NestedWarningsJson,
        HasAttentionWarning,
    }

    #[qenum(DestinationsModel)]
    enum DestinationRoles {
        DestinationId,
        Name,
        KindName,
        KindLabel,
        Location,
        Username,
        BackupCount,
        StatusText,
        StatusIcon,
        Reachable,
        PasswordRejected,
        HasStaleLock,
        HasProblem,
        DetailText,
        PendingForgetCount,
    }

    #[qenum(RunsModel)]
    enum RunRoles {
        RunId,
        Kind,
        BackupId,
        Title,
        DestinationId,
        DestinationName,
        Status,
        StatusText,
        StatusIcon,
        StatusColorKind,
        StartedAtIso,
        StartedAtMs,
        DayGroup,
        TimeShort,
        DurationText,
        ExitCode,
        HasExitCode,
        SummaryText,
        DataAddedText,
    }

    #[qenum(SnapshotsModel)]
    enum SnapshotRoles {
        SnapshotId,
        ShortId,
        TimeIso,
        TimeDisplay,
        Hostname,
        PathsText,
        TagsText,
        StatsText,
        SectionName,
        IsPrevious,
    }

    #[qenum(RunLogsModel)]
    enum RunLogRoles {
        LineText,
        Level,
    }

    #[qenum(SpikeListModel)]
    enum Roles {
        Title,
        Subtitle,
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        type SpikeListModel = super::SpikeListModelRust;

        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        type BackupsModel = super::BackupsModelRust;

        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        type DestinationsModel = super::DestinationsModelRust;

        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        type RunsModel = super::RunsModelRust;

        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        type SnapshotsModel = super::SnapshotsModelRust;

        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        type RunLogsModel = super::RunLogsModelRust;
    }

    unsafe extern "RustQt" {
        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_backups(self: Pin<&mut BackupsModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_backups(self: Pin<&mut BackupsModel>);
        #[cxx_name = "index"]
        #[inherit]
        fn index_backups(
            self: &BackupsModel,
            row: i32,
            column: i32,
            parent: &QModelIndex,
        ) -> QModelIndex;
        #[cxx_name = "dataChanged"]
        #[inherit]
        fn data_changed_backups(
            self: Pin<&mut BackupsModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
        );

        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_destinations(self: Pin<&mut DestinationsModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_destinations(self: Pin<&mut DestinationsModel>);
        #[cxx_name = "index"]
        #[inherit]
        fn index_destinations(
            self: &DestinationsModel,
            row: i32,
            column: i32,
            parent: &QModelIndex,
        ) -> QModelIndex;
        #[cxx_name = "dataChanged"]
        #[inherit]
        fn data_changed_destinations(
            self: Pin<&mut DestinationsModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
        );

        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_runs(self: Pin<&mut RunsModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_runs(self: Pin<&mut RunsModel>);
        #[cxx_name = "index"]
        #[inherit]
        fn index_runs(self: &RunsModel, row: i32, column: i32, parent: &QModelIndex)
        -> QModelIndex;
        #[cxx_name = "dataChanged"]
        #[inherit]
        fn data_changed_runs(
            self: Pin<&mut RunsModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
        );

        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_snapshots(self: Pin<&mut SnapshotsModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_snapshots(self: Pin<&mut SnapshotsModel>);
        #[cxx_name = "index"]
        #[inherit]
        fn index_snapshots(
            self: &SnapshotsModel,
            row: i32,
            column: i32,
            parent: &QModelIndex,
        ) -> QModelIndex;
        #[cxx_name = "dataChanged"]
        #[inherit]
        fn data_changed_snapshots(
            self: Pin<&mut SnapshotsModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
        );

        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_logs(self: Pin<&mut RunLogsModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_logs(self: Pin<&mut RunLogsModel>);
        #[cxx_name = "index"]
        #[inherit]
        fn index_logs(
            self: &RunLogsModel,
            row: i32,
            column: i32,
            parent: &QModelIndex,
        ) -> QModelIndex;
        #[cxx_name = "dataChanged"]
        #[inherit]
        fn data_changed_logs(
            self: Pin<&mut RunLogsModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
        );
    }

    extern "RustQt" {
        // SpikeListModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &SpikeListModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &SpikeListModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &SpikeListModel, _parent: &QModelIndex) -> i32;

        // BackupsModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &BackupsModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &BackupsModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &BackupsModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        fn sync(self: Pin<&mut BackupsModel>);

        // DestinationsModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &DestinationsModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &DestinationsModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &DestinationsModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        fn sync(self: Pin<&mut DestinationsModel>);

        // RunsModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &RunsModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &RunsModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &RunsModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        fn sync(self: Pin<&mut RunsModel>);
        #[qinvokable]
        #[cxx_name = "setFilter"]
        fn set_filter(
            self: Pin<&mut RunsModel>,
            backup_id: &QString,
            issues_only: bool,
            query: &QString,
        );

        // SnapshotsModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &SnapshotsModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &SnapshotsModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &SnapshotsModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        fn sync(self: Pin<&mut SnapshotsModel>);

        // RunLogsModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &RunLogsModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &RunLogsModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &RunLogsModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        fn sync(self: Pin<&mut RunLogsModel>);
        #[qinvokable]
        #[cxx_name = "setFilter"]
        fn set_filter(self: Pin<&mut RunLogsModel>, level_filter: &QString, query: &QString);
    }
}

pub use crate::list_model::SpikeListModelRust;
use crate::state_cache::{
    backup_exclude_file_path_str, current_now, format_backup_status, format_bytes,
    format_datetime_display, format_day_group, format_destination_status, format_duration_ms,
    format_exclude_summary, format_retention_summary, format_run_kind_name, format_run_status,
    format_source_kind_label, format_time_short, i18n, shared_state,
};
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::model::{DestinationKind, RetentionPreset, RunKind, RunStatus, SourceKind};
use std::pin::Pin;
use time::OffsetDateTime;

enum SyncPlan<T> {
    NoChange,
    InPlace {
        first: i32,
        last: i32,
        new_rows: Vec<T>,
    },
    Reset(Vec<T>),
}

fn plan_model_sync<T: PartialEq>(
    old: &[T],
    new_rows: Vec<T>,
    same_identity: impl Fn(&T, &T) -> bool,
) -> SyncPlan<T> {
    if old == new_rows.as_slice() {
        return SyncPlan::NoChange;
    }
    if old.len() == new_rows.len()
        && !old.is_empty()
        && old
            .iter()
            .zip(new_rows.iter())
            .all(|(a, b)| same_identity(a, b))
    {
        let mut first = None;
        let mut last = 0usize;
        for (i, (a, b)) in old.iter().zip(new_rows.iter()).enumerate() {
            if a != b {
                if first.is_none() {
                    first = Some(i);
                }
                last = i;
            }
        }
        if let Some(f) = first {
            return SyncPlan::InPlace {
                first: i32::try_from(f).unwrap_or(0),
                last: i32::try_from(last).unwrap_or(0),
                new_rows,
            };
        }
    }
    SyncPlan::Reset(new_rows)
}

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
}

#[derive(Default)]
pub struct BackupsModelRust {
    rows: Vec<BackupRow>,
}

impl BackupsModelRust {
    fn compute_rows(&self) -> Vec<BackupRow> {
        let now = current_now();
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = &guard.state else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(state.backups.len());
        for b in &state.backups {
            let active_progress = b
                .active_run
                .as_ref()
                .and_then(|r| guard.active_progress.get(r.id.as_str()));
            let (status_text, status_icon, status_color_kind) =
                format_backup_status(b, active_progress, now);
            let active_percent = active_progress
                .map(|p| p.percent_done.clamp(0.0, 100.0))
                .unwrap_or(0.0);
            let active_progress_text = if let Some(p) = active_progress {
                let pct = p.percent_done.round().clamp(0.0, 100.0) as u32;
                let done_str = format_bytes(p.bytes_done);
                let total_str = format_bytes(p.total_bytes);
                if p.total_bytes > 0 {
                    i18n("%1% · %2 of %3").subs(&[&pct.to_string(), &done_str, &total_str])
                } else {
                    i18n("%1%").subs(&[&pct.to_string()])
                }
            } else {
                String::new()
            };

            let last_run_summary = if let Some(run) = &b.last_run {
                let dur = run
                    .duration_ms
                    .map(format_duration_ms)
                    .unwrap_or_else(|| String::from("–"));
                let added = run
                    .stats
                    .as_ref()
                    .map(|s| format_bytes(s.data_added))
                    .unwrap_or_else(|| String::from("0 B"));
                i18n("%1 · %2 new").subs(&[&dur, &added])
            } else {
                i18n("No runs yet").to_string()
            };

            let (last_run_id, last_run_timestamp_iso, last_run_time_ms) =
                if let Some(run) = &b.last_run {
                    let ts = run.finished_at.unwrap_or(run.started_at);
                    (
                        run.id.as_str().to_string(),
                        ts.format(&time::format_description::well_known::Rfc3339)
                            .unwrap_or_default(),
                        (ts.unix_timestamp() as f64) * 1000.0,
                    )
                } else {
                    (String::new(), String::new(), 0.0)
                };

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
                || !b.exclude_lint_warnings.is_empty();

            out.push(BackupRow {
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
                availability_reason: b.availability.reason_text.clone().unwrap_or_default(),
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
                error_summary: b.error_summary.clone().unwrap_or_default(),
                error_action: b.error_action.clone().unwrap_or_default(),
                unreadable_files_json: serde_json::to_string(&b.unreadable_files)
                    .unwrap_or_else(|_| "[]".to_string()),
                schedule_summary: i18n("Manual only (schedules follow in a future update)")
                    .to_string(),
                retention_summary: format_retention_summary(&b.backup.retention),
                conditions_summary: i18n("Always allowed on manual start").to_string(),
                last_run_id,
                last_run_timestamp_iso,
                last_run_time_ms,
                excludes_json: serde_json::to_string(&b.backup.excludes)
                    .unwrap_or_else(|_| "{}".to_string()),
                exclude_folders_count: i32::try_from(b.backup.excludes.folders.len())
                    .unwrap_or(i32::MAX),
                expert_rule_count: i32::try_from(b.backup.excludes.expert_rule_count())
                    .unwrap_or(i32::MAX),
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
            });
        }
        out
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
        roles.insert(
            qobject::BackupRoles::BackupId.repr,
            QByteArray::from("backupId"),
        );
        roles.insert(qobject::BackupRoles::Name.repr, QByteArray::from("name"));
        roles.insert(
            qobject::BackupRoles::SourcePath.repr,
            QByteArray::from("sourcePath"),
        );
        roles.insert(
            qobject::BackupRoles::SourceKind.repr,
            QByteArray::from("sourceKind"),
        );
        roles.insert(
            qobject::BackupRoles::DestinationId.repr,
            QByteArray::from("destinationId"),
        );
        roles.insert(
            qobject::BackupRoles::DestinationName.repr,
            QByteArray::from("destinationName"),
        );
        roles.insert(
            qobject::BackupRoles::ExcludesText.repr,
            QByteArray::from("excludesText"),
        );
        roles.insert(
            qobject::BackupRoles::ExcludeCaches.repr,
            QByteArray::from("excludeCaches"),
        );
        roles.insert(
            qobject::BackupRoles::StatusText.repr,
            QByteArray::from("statusText"),
        );
        roles.insert(
            qobject::BackupRoles::StatusIcon.repr,
            QByteArray::from("statusIcon"),
        );
        roles.insert(
            qobject::BackupRoles::StatusColorKind.repr,
            QByteArray::from("statusColorKind"),
        );
        roles.insert(
            qobject::BackupRoles::LastRunSummary.repr,
            QByteArray::from("lastRunSummary"),
        );
        roles.insert(
            qobject::BackupRoles::AvailabilityReason.repr,
            QByteArray::from("availabilityReason"),
        );
        roles.insert(
            qobject::BackupRoles::Available.repr,
            QByteArray::from("available"),
        );
        roles.insert(
            qobject::BackupRoles::HasError.repr,
            QByteArray::from("hasError"),
        );
        roles.insert(
            qobject::BackupRoles::IsOverdue.repr,
            QByteArray::from("isOverdue"),
        );
        roles.insert(
            qobject::BackupRoles::IsRunning.repr,
            QByteArray::from("isRunning"),
        );
        roles.insert(
            qobject::BackupRoles::ActiveRunId.repr,
            QByteArray::from("activeRunId"),
        );
        roles.insert(
            qobject::BackupRoles::ActivePercent.repr,
            QByteArray::from("activePercent"),
        );
        roles.insert(
            qobject::BackupRoles::ActiveProgressText.repr,
            QByteArray::from("activeProgressText"),
        );
        roles.insert(
            qobject::BackupRoles::ErrorSummary.repr,
            QByteArray::from("errorSummary"),
        );
        roles.insert(
            qobject::BackupRoles::ErrorAction.repr,
            QByteArray::from("errorAction"),
        );
        roles.insert(
            qobject::BackupRoles::UnreadableFilesJson.repr,
            QByteArray::from("unreadableFilesJson"),
        );
        roles.insert(
            qobject::BackupRoles::ScheduleSummary.repr,
            QByteArray::from("scheduleSummary"),
        );
        roles.insert(
            qobject::BackupRoles::RetentionSummary.repr,
            QByteArray::from("retentionSummary"),
        );
        roles.insert(
            qobject::BackupRoles::ConditionsSummary.repr,
            QByteArray::from("conditionsSummary"),
        );
        roles.insert(
            qobject::BackupRoles::LastRunId.repr,
            QByteArray::from("lastRunId"),
        );
        roles.insert(
            qobject::BackupRoles::LastRunTimestampIso.repr,
            QByteArray::from("lastRunTimestampIso"),
        );
        roles.insert(
            qobject::BackupRoles::LastRunTimeMs.repr,
            QByteArray::from("lastRunTimeMs"),
        );
        roles.insert(
            qobject::BackupRoles::ExcludesJson.repr,
            QByteArray::from("excludesJson"),
        );
        roles.insert(
            qobject::BackupRoles::ExcludeFoldersCount.repr,
            QByteArray::from("excludeFoldersCount"),
        );
        roles.insert(
            qobject::BackupRoles::ExpertRuleCount.repr,
            QByteArray::from("expertRuleCount"),
        );
        roles.insert(
            qobject::BackupRoles::ExcludeSummary.repr,
            QByteArray::from("excludeSummary"),
        );
        roles.insert(
            qobject::BackupRoles::ExcludeFilePath.repr,
            QByteArray::from("excludeFilePath"),
        );
        roles.insert(
            qobject::BackupRoles::ExcludeLintJson.repr,
            QByteArray::from("excludeLintJson"),
        );
        roles.insert(
            qobject::BackupRoles::RetentionJson.repr,
            QByteArray::from("retentionJson"),
        );
        roles.insert(
            qobject::BackupRoles::RetentionPresetName.repr,
            QByteArray::from("retentionPreset"),
        );
        roles.insert(
            qobject::BackupRoles::SourceKindLabel.repr,
            QByteArray::from("sourceKindLabel"),
        );
        roles.insert(
            qobject::BackupRoles::DriveLabel.repr,
            QByteArray::from("driveLabel"),
        );
        roles.insert(
            qobject::BackupRoles::PendingForget.repr,
            QByteArray::from("pendingForget"),
        );
        roles.insert(
            qobject::BackupRoles::NestedWarningsJson.repr,
            QByteArray::from("nestedWarningsJson"),
        );
        roles.insert(
            qobject::BackupRoles::HasAttentionWarning.repr,
            QByteArray::from("hasAttentionWarning"),
        );
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
            _ => QVariant::default(),
        }
    }
}

#[derive(Clone, Default, PartialEq)]
struct DestinationRow {
    destination_id: String,
    name: String,
    kind_name: String,
    kind_label: String,
    location: String,
    username: String,
    backup_count: i32,
    status_text: String,
    status_icon: String,
    reachable: bool,
    password_rejected: bool,
    has_stale_lock: bool,
    has_problem: bool,
    detail_text: String,
    pending_forget_count: i32,
}

#[derive(Default)]
pub struct DestinationsModelRust {
    rows: Vec<DestinationRow>,
}

impl DestinationsModelRust {
    fn compute_rows(&self) -> Vec<DestinationRow> {
        let now = current_now();
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = &guard.state else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(state.destinations.len());
        for d in &state.destinations {
            let (kind_name, kind_label, location, username) = match &d.destination.kind {
                DestinationKind::Local { path } => (
                    "local".to_string(),
                    i18n("Local Folder").to_string(),
                    path.to_string_lossy().into_owned(),
                    String::new(),
                ),
                DestinationKind::Rest { url, username } => (
                    "rest".to_string(),
                    i18n("REST Server").to_string(),
                    url.clone(),
                    username.clone().unwrap_or_default(),
                ),
            };
            let (status_text, status_icon) = format_destination_status(d, now);
            let pending_forget_count = state
                .backups
                .iter()
                .filter(|b| b.backup.destination_id == d.destination.id && b.pending_forget)
                .count();
            out.push(DestinationRow {
                destination_id: d.destination.id.as_str().to_string(),
                name: d.destination.name.clone(),
                kind_name,
                kind_label,
                location,
                username,
                backup_count: i32::try_from(d.backup_count).unwrap_or(i32::MAX),
                status_text,
                status_icon,
                reachable: d.availability.reachable,
                password_rejected: d.password_rejected,
                has_stale_lock: d.has_stale_lock,
                has_problem: d.has_problem,
                detail_text: d.availability.detail.clone().unwrap_or_default(),
                pending_forget_count: i32::try_from(pending_forget_count).unwrap_or(i32::MAX),
            });
        }
        out
    }
}

impl qobject::DestinationsModel {
    pub fn sync(mut self: Pin<&mut Self>) {
        let new_rows = self.as_ref().compute_rows();
        match plan_model_sync(&self.rows, new_rows, |a, b| {
            a.destination_id == b.destination_id
        }) {
            SyncPlan::NoChange => {}
            SyncPlan::InPlace {
                first,
                last,
                new_rows,
            } => {
                self.as_mut().rust_mut().rows = new_rows;
                let parent = QModelIndex::default();
                let top = self.as_ref().index_destinations(first, 0, &parent);
                let bottom = self.as_ref().index_destinations(last, 0, &parent);
                self.as_mut().data_changed_destinations(&top, &bottom);
            }
            SyncPlan::Reset(new_rows) => {
                self.as_mut().begin_reset_destinations();
                self.as_mut().rust_mut().rows = new_rows;
                self.as_mut().end_reset_destinations();
            }
        }
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(
            qobject::DestinationRoles::DestinationId.repr,
            QByteArray::from("destinationId"),
        );
        roles.insert(
            qobject::DestinationRoles::Name.repr,
            QByteArray::from("name"),
        );
        roles.insert(
            qobject::DestinationRoles::KindName.repr,
            QByteArray::from("kindName"),
        );
        roles.insert(
            qobject::DestinationRoles::KindLabel.repr,
            QByteArray::from("kindLabel"),
        );
        roles.insert(
            qobject::DestinationRoles::Location.repr,
            QByteArray::from("location"),
        );
        roles.insert(
            qobject::DestinationRoles::Username.repr,
            QByteArray::from("username"),
        );
        roles.insert(
            qobject::DestinationRoles::BackupCount.repr,
            QByteArray::from("backupCount"),
        );
        roles.insert(
            qobject::DestinationRoles::StatusText.repr,
            QByteArray::from("statusText"),
        );
        roles.insert(
            qobject::DestinationRoles::StatusIcon.repr,
            QByteArray::from("statusIcon"),
        );
        roles.insert(
            qobject::DestinationRoles::Reachable.repr,
            QByteArray::from("reachable"),
        );
        roles.insert(
            qobject::DestinationRoles::PasswordRejected.repr,
            QByteArray::from("passwordRejected"),
        );
        roles.insert(
            qobject::DestinationRoles::HasStaleLock.repr,
            QByteArray::from("hasStaleLock"),
        );
        roles.insert(
            qobject::DestinationRoles::HasProblem.repr,
            QByteArray::from("hasProblem"),
        );
        roles.insert(
            qobject::DestinationRoles::DetailText.repr,
            QByteArray::from("detailText"),
        );
        roles.insert(
            qobject::DestinationRoles::PendingForgetCount.repr,
            QByteArray::from("pendingForgetCount"),
        );
        roles
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row())
            .ok()
            .and_then(|i| self.rows.get(i))
        else {
            return QVariant::default();
        };
        match (qobject::DestinationRoles { repr: role }) {
            qobject::DestinationRoles::DestinationId => {
                QVariant::from(&QString::from(&row.destination_id))
            }
            qobject::DestinationRoles::Name => QVariant::from(&QString::from(&row.name)),
            qobject::DestinationRoles::KindName => QVariant::from(&QString::from(&row.kind_name)),
            qobject::DestinationRoles::KindLabel => QVariant::from(&QString::from(&row.kind_label)),
            qobject::DestinationRoles::Location => QVariant::from(&QString::from(&row.location)),
            qobject::DestinationRoles::Username => QVariant::from(&QString::from(&row.username)),
            qobject::DestinationRoles::BackupCount => QVariant::from(&row.backup_count),
            qobject::DestinationRoles::StatusText => {
                QVariant::from(&QString::from(&row.status_text))
            }
            qobject::DestinationRoles::StatusIcon => {
                QVariant::from(&QString::from(&row.status_icon))
            }
            qobject::DestinationRoles::Reachable => QVariant::from(&row.reachable),
            qobject::DestinationRoles::PasswordRejected => QVariant::from(&row.password_rejected),
            qobject::DestinationRoles::HasStaleLock => QVariant::from(&row.has_stale_lock),
            qobject::DestinationRoles::HasProblem => QVariant::from(&row.has_problem),
            qobject::DestinationRoles::DetailText => {
                QVariant::from(&QString::from(&row.detail_text))
            }
            qobject::DestinationRoles::PendingForgetCount => {
                QVariant::from(&row.pending_forget_count)
            }
            _ => QVariant::default(),
        }
    }
}

#[derive(Clone, Default, PartialEq)]
struct RunRow {
    run_id: String,
    kind: String,
    backup_id: String,
    title: String,
    destination_id: String,
    destination_name: String,
    status: String,
    status_text: String,
    status_icon: String,
    status_color_kind: String,
    started_at_iso: String,
    started_at_ms: f64,
    day_group: String,
    time_short: String,
    duration_text: String,
    exit_code: i32,
    has_exit_code: bool,
    summary_text: String,
    data_added_text: String,
}

#[derive(Default)]
pub struct RunsModelRust {
    rows: Vec<RunRow>,
    filter_backup_id: String,
    issues_only: bool,
    query: String,
}

impl RunsModelRust {
    fn compute_rows(&self) -> Vec<RunRow> {
        let now = current_now();
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = &guard.state else {
            return Vec::new();
        };

        let query_lower = self.query.trim().to_lowercase();
        let mut out = Vec::new();
        for run in &state.recent_runs {
            let backup_id_opt = run.backup_id();
            let backup_id_str = backup_id_opt
                .map(|id| id.as_str().to_string())
                .unwrap_or_default();

            if !self.filter_backup_id.is_empty() && backup_id_str != self.filter_backup_id {
                continue;
            }
            if self.issues_only
                && !matches!(
                    run.status,
                    RunStatus::Error | RunStatus::Warning | RunStatus::Interrupted
                )
            {
                continue;
            }

            let backup_name = backup_id_opt.and_then(|bid| {
                state
                    .backups
                    .iter()
                    .find(|b| &b.backup.id == bid)
                    .map(|b| b.backup.name.as_str())
            });
            let dest_name = state
                .destinations
                .iter()
                .find(|d| d.destination.id == run.destination_id)
                .map(|d| d.destination.name.as_str())
                .unwrap_or_else(|| run.destination_id.as_str());

            let title = format_run_kind_name(run, backup_name, dest_name);
            let summary_text = run.summary_message.clone().unwrap_or_default();

            if !query_lower.is_empty()
                && !title.to_lowercase().contains(&query_lower)
                && !dest_name.to_lowercase().contains(&query_lower)
                && !summary_text.to_lowercase().contains(&query_lower)
            {
                continue;
            }

            let kind = match run.kind {
                RunKind::Backup { .. } => "backup",
                RunKind::Maintenance => "maintenance",
                RunKind::Restore { .. } => "restore",
                RunKind::Meta => "meta",
            }
            .to_string();
            let status = match run.status {
                RunStatus::Pending => "pending",
                RunStatus::Running => "running",
                RunStatus::Ok => "ok",
                RunStatus::Warning => "warning",
                RunStatus::Error => "error",
                RunStatus::Cancelled => "cancelled",
                RunStatus::Interrupted => "interrupted",
            }
            .to_string();

            let (status_text, status_icon, status_color_kind) = format_run_status(run.status);
            let duration_text = run
                .duration_ms
                .map(format_duration_ms)
                .unwrap_or_else(|| status_text.clone());
            let data_added_text = run
                .stats
                .as_ref()
                .map(|s| format_bytes(s.data_added))
                .unwrap_or_else(|| String::from("0 B"));

            out.push(RunRow {
                run_id: run.id.as_str().to_string(),
                kind,
                backup_id: backup_id_str,
                title,
                destination_id: run.destination_id.as_str().to_string(),
                destination_name: dest_name.to_string(),
                status,
                status_text,
                status_icon: status_icon.to_string(),
                status_color_kind: status_color_kind.to_string(),
                started_at_iso: run
                    .started_at
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default(),
                started_at_ms: (run.started_at.unix_timestamp() as f64) * 1000.0,
                day_group: format_day_group(run.started_at, now),
                time_short: format_time_short(run.started_at),
                duration_text,
                exit_code: run.exit_code.unwrap_or(0),
                has_exit_code: run.exit_code.is_some(),
                summary_text,
                data_added_text,
            });
        }
        out
    }
}

impl qobject::RunsModel {
    pub fn sync(mut self: Pin<&mut Self>) {
        let new_rows = self.as_ref().compute_rows();
        match plan_model_sync(&self.rows, new_rows, |a, b| {
            a.run_id == b.run_id && a.day_group == b.day_group
        }) {
            SyncPlan::NoChange => {}
            SyncPlan::InPlace {
                first,
                last,
                new_rows,
            } => {
                self.as_mut().rust_mut().rows = new_rows;
                let parent = QModelIndex::default();
                let top = self.as_ref().index_runs(first, 0, &parent);
                let bottom = self.as_ref().index_runs(last, 0, &parent);
                self.as_mut().data_changed_runs(&top, &bottom);
            }
            SyncPlan::Reset(new_rows) => {
                self.as_mut().begin_reset_runs();
                self.as_mut().rust_mut().rows = new_rows;
                self.as_mut().end_reset_runs();
            }
        }
    }

    pub fn set_filter(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        issues_only: bool,
        query: &QString,
    ) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.filter_backup_id = backup_id.to_string();
            rust.issues_only = issues_only;
            rust.query = query.to_string();
        }
        self.sync();
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(qobject::RunRoles::RunId.repr, QByteArray::from("runId"));
        roles.insert(qobject::RunRoles::Kind.repr, QByteArray::from("kind"));
        roles.insert(
            qobject::RunRoles::BackupId.repr,
            QByteArray::from("backupId"),
        );
        roles.insert(qobject::RunRoles::Title.repr, QByteArray::from("title"));
        roles.insert(
            qobject::RunRoles::DestinationId.repr,
            QByteArray::from("destinationId"),
        );
        roles.insert(
            qobject::RunRoles::DestinationName.repr,
            QByteArray::from("destinationName"),
        );
        roles.insert(qobject::RunRoles::Status.repr, QByteArray::from("status"));
        roles.insert(
            qobject::RunRoles::StatusText.repr,
            QByteArray::from("statusText"),
        );
        roles.insert(
            qobject::RunRoles::StatusIcon.repr,
            QByteArray::from("statusIcon"),
        );
        roles.insert(
            qobject::RunRoles::StatusColorKind.repr,
            QByteArray::from("statusColorKind"),
        );
        roles.insert(
            qobject::RunRoles::StartedAtIso.repr,
            QByteArray::from("startedAtIso"),
        );
        roles.insert(
            qobject::RunRoles::StartedAtMs.repr,
            QByteArray::from("startedAtMs"),
        );
        roles.insert(
            qobject::RunRoles::DayGroup.repr,
            QByteArray::from("dayGroup"),
        );
        roles.insert(
            qobject::RunRoles::TimeShort.repr,
            QByteArray::from("timeShort"),
        );
        roles.insert(
            qobject::RunRoles::DurationText.repr,
            QByteArray::from("durationText"),
        );
        roles.insert(
            qobject::RunRoles::ExitCode.repr,
            QByteArray::from("exitCode"),
        );
        roles.insert(
            qobject::RunRoles::HasExitCode.repr,
            QByteArray::from("hasExitCode"),
        );
        roles.insert(
            qobject::RunRoles::SummaryText.repr,
            QByteArray::from("summaryText"),
        );
        roles.insert(
            qobject::RunRoles::DataAddedText.repr,
            QByteArray::from("dataAddedText"),
        );
        roles
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row())
            .ok()
            .and_then(|i| self.rows.get(i))
        else {
            return QVariant::default();
        };
        match (qobject::RunRoles { repr: role }) {
            qobject::RunRoles::RunId => QVariant::from(&QString::from(&row.run_id)),
            qobject::RunRoles::Kind => QVariant::from(&QString::from(&row.kind)),
            qobject::RunRoles::BackupId => QVariant::from(&QString::from(&row.backup_id)),
            qobject::RunRoles::Title => QVariant::from(&QString::from(&row.title)),
            qobject::RunRoles::DestinationId => QVariant::from(&QString::from(&row.destination_id)),
            qobject::RunRoles::DestinationName => {
                QVariant::from(&QString::from(&row.destination_name))
            }
            qobject::RunRoles::Status => QVariant::from(&QString::from(&row.status)),
            qobject::RunRoles::StatusText => QVariant::from(&QString::from(&row.status_text)),
            qobject::RunRoles::StatusIcon => QVariant::from(&QString::from(&row.status_icon)),
            qobject::RunRoles::StatusColorKind => {
                QVariant::from(&QString::from(&row.status_color_kind))
            }
            qobject::RunRoles::StartedAtIso => QVariant::from(&QString::from(&row.started_at_iso)),
            qobject::RunRoles::StartedAtMs => QVariant::from(&row.started_at_ms),
            qobject::RunRoles::DayGroup => QVariant::from(&QString::from(&row.day_group)),
            qobject::RunRoles::TimeShort => QVariant::from(&QString::from(&row.time_short)),
            qobject::RunRoles::DurationText => QVariant::from(&QString::from(&row.duration_text)),
            qobject::RunRoles::ExitCode => QVariant::from(&row.exit_code),
            qobject::RunRoles::HasExitCode => QVariant::from(&row.has_exit_code),
            qobject::RunRoles::SummaryText => QVariant::from(&QString::from(&row.summary_text)),
            qobject::RunRoles::DataAddedText => {
                QVariant::from(&QString::from(&row.data_added_text))
            }
            _ => QVariant::default(),
        }
    }
}

#[derive(Clone, Default, PartialEq)]
struct SnapshotRow {
    snapshot_id: String,
    short_id: String,
    time_iso: String,
    time_display: String,
    hostname: String,
    paths_text: String,
    tags_text: String,
    stats_text: String,
    section_name: String,
    is_previous: bool,
}

#[derive(Default)]
pub struct SnapshotsModelRust {
    rows: Vec<SnapshotRow>,
}

impl SnapshotsModelRust {
    fn push_snapshot(
        out: &mut Vec<SnapshotRow>,
        s: &kestic_daemon::dbus_api::SnapshotSummaryDto,
        section_name: String,
        is_previous: bool,
    ) {
        let time_display =
            OffsetDateTime::parse(&s.time, &time::format_description::well_known::Rfc3339)
                .map(format_datetime_display)
                .unwrap_or_else(|_| s.time.clone());
        let stats_text = match (s.total_files_processed, s.total_bytes_processed) {
            (Some(files), Some(bytes)) => {
                i18n("%1 files · %2").subs(&[&files.to_string(), &format_bytes(bytes)])
            }
            (None, Some(bytes)) => format_bytes(bytes),
            _ => s.hostname.clone(),
        };
        out.push(SnapshotRow {
            snapshot_id: s.id.clone(),
            short_id: s.short_id.clone(),
            time_iso: s.time.clone(),
            time_display,
            hostname: s.hostname.clone(),
            paths_text: s.paths.join(", "),
            tags_text: s.tags.join(", "),
            stats_text,
            section_name,
            is_previous,
        });
    }

    fn compute_rows(&self) -> Vec<SnapshotRow> {
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(snaps) = &guard.snapshots else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let current_section =
            i18n("Current destination (%1)").subs(&[&snaps.current_destination_name]);
        for s in snaps.current_snapshots.iter().rev() {
            Self::push_snapshot(&mut out, s, current_section.clone(), false);
        }
        for prev in &snaps.previous_destinations {
            let section = i18n("Earlier snapshots on '%1'").subs(&[&prev.destination_name]);
            for s in prev.snapshots.iter().rev() {
                Self::push_snapshot(&mut out, s, section.clone(), true);
            }
        }
        out
    }
}

impl qobject::SnapshotsModel {
    pub fn sync(mut self: Pin<&mut Self>) {
        let new_rows = self.as_ref().compute_rows();
        match plan_model_sync(&self.rows, new_rows, |a, b| {
            a.snapshot_id == b.snapshot_id && a.section_name == b.section_name
        }) {
            SyncPlan::NoChange => {}
            SyncPlan::InPlace {
                first,
                last,
                new_rows,
            } => {
                self.as_mut().rust_mut().rows = new_rows;
                let parent = QModelIndex::default();
                let top = self.as_ref().index_snapshots(first, 0, &parent);
                let bottom = self.as_ref().index_snapshots(last, 0, &parent);
                self.as_mut().data_changed_snapshots(&top, &bottom);
            }
            SyncPlan::Reset(new_rows) => {
                self.as_mut().begin_reset_snapshots();
                self.as_mut().rust_mut().rows = new_rows;
                self.as_mut().end_reset_snapshots();
            }
        }
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(
            qobject::SnapshotRoles::SnapshotId.repr,
            QByteArray::from("snapshotId"),
        );
        roles.insert(
            qobject::SnapshotRoles::ShortId.repr,
            QByteArray::from("shortId"),
        );
        roles.insert(
            qobject::SnapshotRoles::TimeIso.repr,
            QByteArray::from("timeIso"),
        );
        roles.insert(
            qobject::SnapshotRoles::TimeDisplay.repr,
            QByteArray::from("timeDisplay"),
        );
        roles.insert(
            qobject::SnapshotRoles::Hostname.repr,
            QByteArray::from("hostname"),
        );
        roles.insert(
            qobject::SnapshotRoles::PathsText.repr,
            QByteArray::from("pathsText"),
        );
        roles.insert(
            qobject::SnapshotRoles::TagsText.repr,
            QByteArray::from("tagsText"),
        );
        roles.insert(
            qobject::SnapshotRoles::StatsText.repr,
            QByteArray::from("statsText"),
        );
        roles.insert(
            qobject::SnapshotRoles::SectionName.repr,
            QByteArray::from("sectionName"),
        );
        roles.insert(
            qobject::SnapshotRoles::IsPrevious.repr,
            QByteArray::from("isPrevious"),
        );
        roles
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row())
            .ok()
            .and_then(|i| self.rows.get(i))
        else {
            return QVariant::default();
        };
        match (qobject::SnapshotRoles { repr: role }) {
            qobject::SnapshotRoles::SnapshotId => QVariant::from(&QString::from(&row.snapshot_id)),
            qobject::SnapshotRoles::ShortId => QVariant::from(&QString::from(&row.short_id)),
            qobject::SnapshotRoles::TimeIso => QVariant::from(&QString::from(&row.time_iso)),
            qobject::SnapshotRoles::TimeDisplay => {
                QVariant::from(&QString::from(&row.time_display))
            }
            qobject::SnapshotRoles::Hostname => QVariant::from(&QString::from(&row.hostname)),
            qobject::SnapshotRoles::PathsText => QVariant::from(&QString::from(&row.paths_text)),
            qobject::SnapshotRoles::TagsText => QVariant::from(&QString::from(&row.tags_text)),
            qobject::SnapshotRoles::StatsText => QVariant::from(&QString::from(&row.stats_text)),
            qobject::SnapshotRoles::SectionName => {
                QVariant::from(&QString::from(&row.section_name))
            }
            qobject::SnapshotRoles::IsPrevious => QVariant::from(&row.is_previous),
            _ => QVariant::default(),
        }
    }
}

#[derive(Clone, Default, PartialEq)]
struct LogRow {
    line_text: String,
    level: String,
}

#[derive(Default)]
pub struct RunLogsModelRust {
    rows: Vec<LogRow>,
    level_filter: String,
    query: String,
}

impl RunLogsModelRust {
    fn compute_rows(&self) -> Vec<LogRow> {
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(detail) = &guard.run_detail else {
            return Vec::new();
        };
        let query_lower = self.query.trim().to_lowercase();
        let mut out = Vec::new();
        for line in &detail.log_lines {
            let level = if line.contains("\"message_type\":\"error\"") || line.contains("error") {
                "error"
            } else if line.contains("warning") {
                "warning"
            } else {
                "info"
            };
            if self.level_filter == "warnings" && level != "warning" && level != "error" {
                continue;
            }
            if self.level_filter == "errors" && level != "error" {
                continue;
            }
            if !query_lower.is_empty() && !line.to_lowercase().contains(&query_lower) {
                continue;
            }
            out.push(LogRow {
                line_text: line.clone(),
                level: level.to_string(),
            });
        }
        out
    }
}

impl qobject::RunLogsModel {
    pub fn sync(mut self: Pin<&mut Self>) {
        let new_rows = self.as_ref().compute_rows();
        match plan_model_sync(&self.rows, new_rows, |a, b| a == b) {
            SyncPlan::NoChange => {}
            SyncPlan::InPlace {
                first,
                last,
                new_rows,
            } => {
                self.as_mut().rust_mut().rows = new_rows;
                let parent = QModelIndex::default();
                let top = self.as_ref().index_logs(first, 0, &parent);
                let bottom = self.as_ref().index_logs(last, 0, &parent);
                self.as_mut().data_changed_logs(&top, &bottom);
            }
            SyncPlan::Reset(new_rows) => {
                self.as_mut().begin_reset_logs();
                self.as_mut().rust_mut().rows = new_rows;
                self.as_mut().end_reset_logs();
            }
        }
    }

    pub fn set_filter(mut self: Pin<&mut Self>, level_filter: &QString, query: &QString) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.level_filter = level_filter.to_string();
            rust.query = query.to_string();
        }
        self.sync();
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(
            qobject::RunLogRoles::LineText.repr,
            QByteArray::from("lineText"),
        );
        roles.insert(qobject::RunLogRoles::Level.repr, QByteArray::from("level"));
        roles
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row())
            .ok()
            .and_then(|i| self.rows.get(i))
        else {
            return QVariant::default();
        };
        match (qobject::RunLogRoles { repr: role }) {
            qobject::RunLogRoles::LineText => QVariant::from(&QString::from(&row.line_text)),
            qobject::RunLogRoles::Level => QVariant::from(&QString::from(&row.level)),
            _ => QVariant::default(),
        }
    }
}
