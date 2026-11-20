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

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
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
        ScheduleJson,
        ConditionsJson,
        NextScheduledAtText,
        IsPaused,
        PausedLine,
        PendingPrimaryCode,
        PendingPrimaryReason,
        PendingReasonsTooltip,
        MissedSlotsCount,
        OverdueThreshold,
        HasUnmetRunCondition,
        UnmetConditionsText,
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
        MaintenanceJson,
        PruneSummary,
        CheckSummary,
        NextPruneAtText,
        NextCheckAtText,
        LastPruneAtText,
        LastCheckAtText,
        MaintenanceRunning,
        StaleLockCount,
        MaintenanceStatusText,
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

    extern "RustQt" {
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

        /// Texts used in more than one place, by code (`Texts.label("close")`): labels shared
        /// with the tray (`kestic-i18n`) and GUI texts (`ui_texts`). No daemon connection.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type Texts = super::TextsRust;
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
        /// Text by code in the GUI language; an unknown code yields an empty text.
        #[qinvokable]
        fn label(self: &Texts, code: &QString) -> QString;

        /// Text by code with its `%1`, `%2`, … filled from `args`.
        #[qinvokable]
        #[cxx_name = "labelArgs"]
        fn label_args(self: &Texts, code: &QString, args: &QStringList) -> QString;

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

mod backups_model;
mod destinations_model;
mod run_logs_model;
mod runs_model;
mod snapshots_model;

pub use backups_model::BackupsModelRust;
pub use destinations_model::DestinationsModelRust;
pub use run_logs_model::RunLogsModelRust;
pub use runs_model::RunsModelRust;
pub use snapshots_model::SnapshotsModelRust;

/// Backing struct of the `Texts` QML singleton.
#[derive(Default)]
pub struct TextsRust;

impl qobject::Texts {
    pub fn label(&self, code: &cxx_qt_lib::QString) -> cxx_qt_lib::QString {
        self.label_args(code, &cxx_qt_lib::QStringList::default())
    }

    pub fn label_args(
        &self,
        code: &cxx_qt_lib::QString,
        args: &cxx_qt_lib::QStringList,
    ) -> cxx_qt_lib::QString {
        let code = code.to_string();
        let args: Vec<String> = args.iter().map(ToString::to_string).collect();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let text = if let Some(label) = kestic_i18n::messages::Label::parse(&code) {
            crate::state_cache::tr(&label.message())
        } else if let Some(text) = crate::ui_texts::GuiText::parse(&code) {
            text.with_args(&args)
        } else {
            eprintln!("Kestic: unknown text code '{code}'");
            String::new()
        };
        cxx_qt_lib::QString::from(&text)
    }
}

pub(super) enum SyncPlan<T> {
    NoChange,
    InPlace {
        first: i32,
        last: i32,
        new_rows: Vec<T>,
    },
    Reset(Vec<T>),
}

pub(super) fn plan_model_sync<T: PartialEq>(
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
