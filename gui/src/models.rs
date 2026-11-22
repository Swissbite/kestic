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

        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        type RestoreBackupsModel = super::RestoreBackupsModelRust;

        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        type RestoreSnapshotsModel = super::RestoreSnapshotsModelRust;

        /// One folder of a snapshot, fetched page by page, with the selection to restore.
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(QString, state)]
        #[qproperty(QString, error_text)]
        #[qproperty(QString, dir)]
        #[qproperty(bool, dir_exists)]
        #[qproperty(bool, too_large)]
        #[qproperty(i32, total)]
        #[qproperty(i32, matched)]
        #[qproperty(QString, count_text)]
        #[qproperty(QString, too_large_text)]
        #[qproperty(i32, selection_count)]
        #[qproperty(QString, selection_text)]
        type SnapshotBrowseModel = super::SnapshotBrowseModelRust;

        /// The hits of a search in the snapshots of one backup, fetched page by page.
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(QString, state)]
        #[qproperty(QString, error_text)]
        #[qproperty(QString, search_text)]
        #[qproperty(bool, too_large)]
        #[qproperty(i32, total)]
        #[qproperty(i32, matched)]
        #[qproperty(QString, count_text)]
        #[qproperty(QString, too_large_text)]
        #[qproperty(QString, no_hits_text)]
        type SnapshotSearchModel = super::SnapshotSearchModelRust;

        /// The changes from one snapshot to another, fetched page by page.
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(QString, state)]
        #[qproperty(QString, error_text)]
        #[qproperty(bool, identical)]
        #[qproperty(bool, too_large)]
        #[qproperty(i32, total)]
        #[qproperty(i32, matched)]
        #[qproperty(QString, count_text)]
        #[qproperty(QString, too_large_text)]
        #[qproperty(QStringList, kind_codes)]
        #[qproperty(QStringList, kind_texts)]
        type SnapshotDiffModel = super::SnapshotDiffModelRust;

        /// The dialog «Restore n items»: target, mode, preview list and the run it starts.
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(QString, state)]
        #[qproperty(QString, error_text)]
        #[qproperty(QString, title)]
        #[qproperty(QString, new_folder)]
        #[qproperty(QString, original_dir)]
        #[qproperty(bool, original_available)]
        #[qproperty(bool, target_original)]
        #[qproperty(QString, target_dir)]
        #[qproperty(i32, overwrite)]
        #[qproperty(QString, check_text)]
        #[qproperty(bool, preview_required)]
        #[qproperty(QString, summary_text)]
        #[qproperty(QString, consequence_text)]
        #[qproperty(bool, overwrites)]
        #[qproperty(QString, notes_text)]
        #[qproperty(bool, can_restore)]
        #[qproperty(i32, matched)]
        #[qproperty(bool, too_large)]
        #[qproperty(QString, too_large_text)]
        #[qproperty(QString, count_text)]
        #[qproperty(QStringList, kind_codes)]
        #[qproperty(QStringList, kind_texts)]
        #[qproperty(QString, run_state)]
        #[qproperty(QString, run_text)]
        #[qproperty(QString, run_target)]
        type RestorePreviewModel = super::RestorePreviewModelRust;

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

        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_restore_backups(self: Pin<&mut RestoreBackupsModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_restore_backups(self: Pin<&mut RestoreBackupsModel>);
        #[cxx_name = "index"]
        #[inherit]
        fn index_restore_backups(
            self: &RestoreBackupsModel,
            row: i32,
            column: i32,
            parent: &QModelIndex,
        ) -> QModelIndex;
        #[cxx_name = "dataChanged"]
        #[inherit]
        fn data_changed_restore_backups(
            self: Pin<&mut RestoreBackupsModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
        );

        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_restore_snapshots(self: Pin<&mut RestoreSnapshotsModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_restore_snapshots(self: Pin<&mut RestoreSnapshotsModel>);
        #[cxx_name = "index"]
        #[inherit]
        fn index_restore_snapshots(
            self: &RestoreSnapshotsModel,
            row: i32,
            column: i32,
            parent: &QModelIndex,
        ) -> QModelIndex;
        #[cxx_name = "dataChanged"]
        #[inherit]
        fn data_changed_restore_snapshots(
            self: Pin<&mut RestoreSnapshotsModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
        );

        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_browse(self: Pin<&mut SnapshotBrowseModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_browse(self: Pin<&mut SnapshotBrowseModel>);
        #[cxx_name = "beginInsertRows"]
        #[inherit]
        fn begin_insert_rows_browse(
            self: Pin<&mut SnapshotBrowseModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[cxx_name = "endInsertRows"]
        #[inherit]
        fn end_insert_rows_browse(self: Pin<&mut SnapshotBrowseModel>);
        #[cxx_name = "index"]
        #[inherit]
        fn index_browse(
            self: &SnapshotBrowseModel,
            row: i32,
            column: i32,
            parent: &QModelIndex,
        ) -> QModelIndex;
        #[cxx_name = "dataChanged"]
        #[inherit]
        fn data_changed_browse(
            self: Pin<&mut SnapshotBrowseModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
        );

        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_search(self: Pin<&mut SnapshotSearchModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_search(self: Pin<&mut SnapshotSearchModel>);
        #[cxx_name = "beginInsertRows"]
        #[inherit]
        fn begin_insert_rows_search(
            self: Pin<&mut SnapshotSearchModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[cxx_name = "endInsertRows"]
        #[inherit]
        fn end_insert_rows_search(self: Pin<&mut SnapshotSearchModel>);

        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_diff(self: Pin<&mut SnapshotDiffModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_diff(self: Pin<&mut SnapshotDiffModel>);
        #[cxx_name = "beginInsertRows"]
        #[inherit]
        fn begin_insert_rows_diff(
            self: Pin<&mut SnapshotDiffModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[cxx_name = "endInsertRows"]
        #[inherit]
        fn end_insert_rows_diff(self: Pin<&mut SnapshotDiffModel>);

        #[cxx_name = "beginResetModel"]
        #[inherit]
        fn begin_reset_preview(self: Pin<&mut RestorePreviewModel>);
        #[cxx_name = "endResetModel"]
        #[inherit]
        fn end_reset_preview(self: Pin<&mut RestorePreviewModel>);
        #[cxx_name = "beginInsertRows"]
        #[inherit]
        fn begin_insert_rows_preview(
            self: Pin<&mut RestorePreviewModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[cxx_name = "endInsertRows"]
        #[inherit]
        fn end_insert_rows_preview(self: Pin<&mut RestorePreviewModel>);
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
        #[qinvokable]
        #[cxx_name = "rowMap"]
        fn row_map(self: &BackupsModel, backup_id: &QString) -> QVariant;
        #[qinvokable]
        #[cxx_name = "rowOf"]
        fn row_of(self: &BackupsModel, backup_id: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(self: &BackupsModel, row: i32) -> QString;

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
        #[qinvokable]
        #[cxx_name = "rowMap"]
        fn row_map(self: &DestinationsModel, destination_id: &QString) -> QVariant;
        #[qinvokable]
        #[cxx_name = "rowOf"]
        fn row_of(self: &DestinationsModel, destination_id: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(self: &DestinationsModel, row: i32) -> QString;

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
        #[cxx_name = "rowOf"]
        fn row_of(self: &RunsModel, run_id: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(self: &RunsModel, row: i32) -> QString;
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

        // RestoreBackupsModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &RestoreBackupsModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &RestoreBackupsModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &RestoreBackupsModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        fn sync(self: Pin<&mut RestoreBackupsModel>);
        #[qinvokable]
        #[cxx_name = "rowOf"]
        fn row_of(self: &RestoreBackupsModel, backup_id: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(self: &RestoreBackupsModel, row: i32) -> QString;

        // RestoreSnapshotsModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &RestoreSnapshotsModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &RestoreSnapshotsModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &RestoreSnapshotsModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        fn sync(self: Pin<&mut RestoreSnapshotsModel>);
        #[qinvokable]
        #[cxx_name = "rowOf"]
        fn row_of(self: &RestoreSnapshotsModel, snapshot_id: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(self: &RestoreSnapshotsModel, row: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "rowMap"]
        fn row_map(self: &RestoreSnapshotsModel, snapshot_id: &QString) -> QVariant;

        // SnapshotBrowseModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &SnapshotBrowseModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &SnapshotBrowseModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &SnapshotBrowseModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "canFetchMore"]
        fn can_fetch_more(self: &SnapshotBrowseModel, parent: &QModelIndex) -> bool;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "fetchMore"]
        fn fetch_more(self: Pin<&mut SnapshotBrowseModel>, parent: &QModelIndex);
        #[qinvokable]
        fn open(
            self: Pin<&mut SnapshotBrowseModel>,
            destination_id: &QString,
            snapshot_id: &QString,
            dir: &QString,
        );
        #[qinvokable]
        fn poll(self: Pin<&mut SnapshotBrowseModel>);
        #[qinvokable]
        fn cancel(self: Pin<&mut SnapshotBrowseModel>);
        #[qinvokable]
        #[cxx_name = "setFilter"]
        fn set_filter(self: Pin<&mut SnapshotBrowseModel>, text: &QString);
        #[qinvokable]
        #[cxx_name = "toggleSelected"]
        fn toggle_selected(self: Pin<&mut SnapshotBrowseModel>, row: i32);
        #[qinvokable]
        #[cxx_name = "clearSelection"]
        fn clear_selection(self: Pin<&mut SnapshotBrowseModel>);
        #[qinvokable]
        #[cxx_name = "selectedPaths"]
        fn selected_paths(self: &SnapshotBrowseModel) -> QStringList;
        #[qinvokable]
        #[cxx_name = "segmentNames"]
        fn segment_names(self: &SnapshotBrowseModel, dir: &QString) -> QStringList;
        #[qinvokable]
        #[cxx_name = "segmentPaths"]
        fn segment_paths(self: &SnapshotBrowseModel, dir: &QString) -> QStringList;
        #[qinvokable]
        #[cxx_name = "rowOfPath"]
        fn row_of_path(self: Pin<&mut SnapshotBrowseModel>, path: &QString) -> i32;

        // SnapshotSearchModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &SnapshotSearchModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &SnapshotSearchModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &SnapshotSearchModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "canFetchMore"]
        fn can_fetch_more(self: &SnapshotSearchModel, parent: &QModelIndex) -> bool;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "fetchMore"]
        fn fetch_more(self: Pin<&mut SnapshotSearchModel>, parent: &QModelIndex);
        #[qinvokable]
        fn search(
            self: Pin<&mut SnapshotSearchModel>,
            destination_id: &QString,
            backup_id: &QString,
            text: &QString,
        );
        #[qinvokable]
        fn poll(self: Pin<&mut SnapshotSearchModel>);
        #[qinvokable]
        #[cxx_name = "setFilter"]
        fn set_filter(self: Pin<&mut SnapshotSearchModel>, text: &QString);
        #[qinvokable]
        fn cancel(self: Pin<&mut SnapshotSearchModel>);

        // SnapshotDiffModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &SnapshotDiffModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &SnapshotDiffModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &SnapshotDiffModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "canFetchMore"]
        fn can_fetch_more(self: &SnapshotDiffModel, parent: &QModelIndex) -> bool;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "fetchMore"]
        fn fetch_more(self: Pin<&mut SnapshotDiffModel>, parent: &QModelIndex);
        #[qinvokable]
        fn compare(
            self: Pin<&mut SnapshotDiffModel>,
            destination_id: &QString,
            from_snapshot_id: &QString,
            to_snapshot_id: &QString,
        );
        #[qinvokable]
        fn poll(self: Pin<&mut SnapshotDiffModel>);
        #[qinvokable]
        #[cxx_name = "setFilter"]
        fn set_filter(self: Pin<&mut SnapshotDiffModel>, text: &QString);
        #[qinvokable]
        #[cxx_name = "setKinds"]
        fn set_kinds(self: Pin<&mut SnapshotDiffModel>, codes: &QStringList);
        #[qinvokable]
        fn cancel(self: Pin<&mut SnapshotDiffModel>);

        // RestorePreviewModel
        #[qinvokable]
        #[cxx_override]
        fn data(self: &RestorePreviewModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &RestorePreviewModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &RestorePreviewModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "canFetchMore"]
        fn can_fetch_more(self: &RestorePreviewModel, parent: &QModelIndex) -> bool;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "fetchMore"]
        fn fetch_more(self: Pin<&mut RestorePreviewModel>, parent: &QModelIndex);
        #[qinvokable]
        fn open(
            self: Pin<&mut RestorePreviewModel>,
            destination_id: &QString,
            snapshot_id: &QString,
            selection: &QStringList,
        );
        #[qinvokable]
        #[cxx_name = "chooseFolder"]
        fn choose_folder(self: Pin<&mut RestorePreviewModel>, dir: &QString);
        #[qinvokable]
        #[cxx_name = "chooseOriginal"]
        fn choose_original(self: Pin<&mut RestorePreviewModel>);
        #[qinvokable]
        #[cxx_name = "chooseOverwrite"]
        fn choose_overwrite(self: Pin<&mut RestorePreviewModel>, index: i32);
        #[qinvokable]
        #[cxx_name = "setKinds"]
        fn set_kinds(self: Pin<&mut RestorePreviewModel>, codes: &QStringList);
        #[qinvokable]
        #[cxx_name = "setFilter"]
        fn set_filter(self: Pin<&mut RestorePreviewModel>, text: &QString);
        #[qinvokable]
        fn poll(self: Pin<&mut RestorePreviewModel>);
        #[qinvokable]
        fn cancel(self: Pin<&mut RestorePreviewModel>);
        #[qinvokable]
        #[cxx_name = "startRestore"]
        fn start_restore(self: Pin<&mut RestorePreviewModel>);
        #[qinvokable]
        #[cxx_name = "pollRun"]
        fn poll_run(self: Pin<&mut RestorePreviewModel>);
        #[qinvokable]
        #[cxx_name = "cancelRun"]
        fn cancel_run(self: Pin<&mut RestorePreviewModel>);
        #[qinvokable]
        #[cxx_name = "dismissRun"]
        fn dismiss_run(self: Pin<&mut RestorePreviewModel>);
    }
}

#[macro_use]
mod rows;

mod backups_model;
mod destinations_model;
mod restore_backups_model;
mod restore_preview_model;
mod restore_snapshots_model;
mod run_logs_model;
mod runs_model;
mod snapshot_browse_model;
mod snapshot_diff_model;
mod snapshot_query;
mod snapshot_search_model;
mod snapshots_model;

pub use backups_model::BackupsModelRust;
pub use destinations_model::DestinationsModelRust;
pub use restore_backups_model::RestoreBackupsModelRust;
pub use restore_preview_model::RestorePreviewModelRust;
pub use restore_snapshots_model::RestoreSnapshotsModelRust;
pub use run_logs_model::RunLogsModelRust;
pub use runs_model::RunsModelRust;
pub use snapshot_browse_model::SnapshotBrowseModelRust;
pub use snapshot_diff_model::SnapshotDiffModelRust;
pub use snapshot_search_model::SnapshotSearchModelRust;
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
