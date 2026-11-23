// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Primary CXX-Qt backend QObject (`KesticApp`) bridging QML pages/wizards/dialogs
//! to the Kestic daemon over `net.daester.Kestic1`.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, initialized)]
        #[qproperty(bool, has_config_file)]
        #[qproperty(QString, config_load_error)]
        #[qproperty(bool, expert_mode)]
        #[qproperty(QString, daemon_version)]
        #[qproperty(QString, daemon_build_commit)]
        #[qproperty(bool, daemon_reachable)]
        #[qproperty(bool, restic_valid)]
        #[qproperty(QString, restic_version)]
        #[qproperty(QString, restic_binary_path)]
        #[qproperty(QString, restic_provider_kind)]
        #[qproperty(QString, restic_message)]
        #[qproperty(QString, blocking_kind)]
        #[qproperty(QString, blocking_message)]
        #[qproperty(QString, overall_headline)]
        #[qproperty(QString, overall_subtitle)]
        #[qproperty(QString, overall_icon)]
        #[qproperty(QString, overall_color_kind)]
        #[qproperty(bool, backups_have_attention)]
        #[qproperty(bool, destinations_have_attention)]
        #[qproperty(i32, backups_count)]
        #[qproperty(i32, startable_backups_count)]
        #[qproperty(QString, back_up_all_hint)]
        #[qproperty(i32, destinations_count)]
        #[qproperty(i32, runs_count)]
        #[qproperty(QString, manual_run_blocked_message)]
        #[qproperty(QString, manual_run_blocked_backup_id)]
        #[qproperty(bool, connection_test_ok)]
        #[qproperty(QString, connection_test_message)]
        #[qproperty(QString, connection_test_error_kind)]
        #[qproperty(QString, connection_test_note)]
        #[qproperty(bool, snapshots_loading)]
        #[qproperty(QString, snapshots_error)]
        #[qproperty(bool, snapshots_has_previous)]
        #[qproperty(QString, selected_run_id)]
        #[qproperty(QString, selected_run_title)]
        #[qproperty(QString, selected_run_status_line)]
        #[qproperty(QString, selected_run_subtitle)]
        #[qproperty(QString, selected_run_status_icon)]
        #[qproperty(QString, selected_run_status_color)]
        #[qproperty(QString, selected_run_result_summary)]
        #[qproperty(QString, selected_run_error_summary)]
        #[qproperty(QString, selected_run_error_action)]
        #[qproperty(QString, selected_run_error_raw)]
        #[qproperty(QString, selected_run_unreadable_json)]
        #[qproperty(QString, selected_run_commands_json)]
        #[qproperty(QString, selected_run_transparent_command)]
        #[qproperty(QString, selected_run_log_path)]
        #[qproperty(QString, selected_run_backup_id)]
        #[qproperty(bool, has_unsaved_changes)]
        #[qproperty(QString, last_action_error)]
        #[qproperty(bool, show_quit_dialog_on_start)]
        #[qproperty(bool, show_about_on_start)]
        #[qproperty(bool, quit_has_running_run)]
        #[qproperty(QString, quit_running_backup_name)]
        #[qproperty(QString, dry_run_preview_id)]
        #[qproperty(QString, dry_run_state)]
        #[qproperty(QString, dry_run_result_json)]
        #[qproperty(QString, dry_run_error)]
        #[qproperty(QString, retention_preview_id)]
        #[qproperty(QString, retention_preview_state)]
        #[qproperty(QString, retention_preview_result_json)]
        #[qproperty(QString, retention_preview_error)]
        #[qproperty(bool, global_paused)]
        #[qproperty(QString, global_paused_line)]
        #[qproperty(i32, default_overdue_threshold)]
        #[qproperty(bool, notif_on_error)]
        #[qproperty(bool, notif_on_warning)]
        #[qproperty(bool, notif_on_overdue)]
        #[qproperty(bool, notif_on_maintenance_failed)]
        #[qproperty(bool, notif_on_restic_update)]
        #[qproperty(QString, start_all_skipped_message)]
        #[qproperty(QString, condition_override_backup_id)]
        #[qproperty(QString, condition_override_backup_name)]
        #[qproperty(QString, condition_override_message)]
        #[qproperty(bool, maintenance_preview_loading)]
        type KesticApp = super::KesticAppRust;

        #[qsignal]
        #[cxx_name = "modelsUpdated"]
        fn models_updated(self: Pin<&mut KesticApp>);

        #[qsignal]
        #[cxx_name = "snapshotsUpdated"]
        fn snapshots_updated(self: Pin<&mut KesticApp>);

        #[qsignal]
        #[cxx_name = "passiveNotification"]
        fn passive_notification(self: Pin<&mut KesticApp>, message: QString);

        #[qsignal]
        #[cxx_name = "showWindowRequested"]
        fn show_window_requested(self: Pin<&mut KesticApp>);

        #[qsignal]
        #[cxx_name = "aboutRequested"]
        fn about_requested(self: Pin<&mut KesticApp>);

        #[qsignal]
        #[cxx_name = "quitConfirmationRequested"]
        fn quit_confirmation_requested(
            self: Pin<&mut KesticApp>,
            has_running_run: bool,
            running_backup_name: QString,
        );

        #[qsignal]
        #[cxx_name = "conditionOverrideRequested"]
        fn condition_override_requested(
            self: Pin<&mut KesticApp>,
            backup_id: QString,
            backup_name: QString,
            message: QString,
        );

        #[qsignal]
        #[cxx_name = "maintenancePreviewReady"]
        fn maintenance_preview_ready(
            self: Pin<&mut KesticApp>,
            dest_id: QString,
            summary_json: QString,
            error_message: QString,
        );

        #[qinvokable]
        fn initialize(self: Pin<&mut KesticApp>);

        #[qinvokable]
        #[cxx_name = "pollEvents"]
        fn poll_events(self: Pin<&mut KesticApp>);

        #[qinvokable]
        #[cxx_name = "retryStartDaemon"]
        fn retry_start_daemon(self: Pin<&mut KesticApp>);

        #[qinvokable]
        #[cxx_name = "unlockSecretStore"]
        fn unlock_secret_store(self: Pin<&mut KesticApp>);

        #[qinvokable]
        #[cxx_name = "windowActive"]
        fn window_active(self: Pin<&mut KesticApp>, active: bool, page: &QString);

        #[qinvokable]
        #[cxx_name = "checkAvailability"]
        fn check_availability(self: Pin<&mut KesticApp>, force: bool);

        #[qinvokable]
        #[cxx_name = "startBackup"]
        fn start_backup(self: Pin<&mut KesticApp>, backup_id: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "startAllBackups"]
        fn start_all_backups(self: Pin<&mut KesticApp>);

        #[qinvokable]
        #[cxx_name = "cancelRun"]
        fn cancel_run(self: Pin<&mut KesticApp>, run_id: &QString);

        #[qinvokable]
        #[cxx_name = "dismissManualRunBlocked"]
        fn dismiss_manual_run_blocked(self: Pin<&mut KesticApp>);

        #[qinvokable]
        #[cxx_name = "destinationTypeCardsJson"]
        fn destination_type_cards_json(self: &KesticApp) -> QString;

        #[qinvokable]
        #[cxx_name = "removableDrivesJson"]
        fn removable_drives_json(self: &KesticApp) -> QString;

        #[qinvokable]
        #[cxx_name = "localFolderFromUrl"]
        fn local_folder_from_url(self: &KesticApp, url: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "genericFieldsErrors"]
        fn generic_fields_errors(self: &KesticApp, url: &QString, env_json: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "envVarSecretByDefault"]
        fn env_var_secret_by_default(self: &KesticApp, name: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "testDestination"]
        fn test_destination(
            self: Pin<&mut KesticApp>,
            fields_json: &QString,
            mode: &QString,
            repo_pw: &QString,
            backend_secrets_json: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "addDestination"]
        fn add_destination(
            self: Pin<&mut KesticApp>,
            fields_json: &QString,
            mode: &QString,
            repo_pw: &QString,
            backend_secrets_json: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "updateDestination"]
        fn update_destination(
            self: Pin<&mut KesticApp>,
            id: &QString,
            fields_json: &QString,
            mode: &QString,
            repo_pw: &QString,
            backend_secrets_json: &QString,
            verify_connection: bool,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "updateDestinationPassword"]
        fn update_destination_password(
            self: Pin<&mut KesticApp>,
            dest_id: &QString,
            new_pw: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "removeDestination"]
        fn remove_destination(self: Pin<&mut KesticApp>, dest_id: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "addBackup"]
        fn add_backup(
            self: Pin<&mut KesticApp>,
            name: &QString,
            source_path: &QString,
            dest_id: &QString,
            excludes_text: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "updateBackup"]
        fn update_backup(
            self: Pin<&mut KesticApp>,
            id: &QString,
            name: &QString,
            source_path: &QString,
            dest_id: &QString,
            excludes_text: &QString,
            exclude_caches: bool,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "updateBackupExcludes"]
        fn update_backup_excludes(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            excludes_json: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "updateBackupRetention"]
        fn update_backup_retention(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            retention_json: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "updateBackupSource"]
        fn update_backup_source(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            source_path: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "renameBackup"]
        fn rename_backup(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            new_name: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "reassignBackup"]
        fn reassign_backup(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            new_dest_id: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "removeBackup"]
        fn remove_backup(self: Pin<&mut KesticApp>, backup_id: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "addExcludePathToBackup"]
        fn add_exclude_path_to_backup(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            path: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "formatExcludeRuleForItem"]
        fn format_exclude_rule_for_item(self: &KesticApp, item_path: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "lintExcludes"]
        fn lint_excludes(self: &KesticApp, excludes_json: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "testExcludePath"]
        fn test_exclude_path(
            self: &KesticApp,
            backup_id: &QString,
            excludes_json: &QString,
            test_path: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "normalizeExcludeFolder"]
        fn normalize_exclude_folder(self: &KesticApp, folder_path: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "buildExcludeFilePreview"]
        fn build_exclude_file_preview(self: &KesticApp, excludes_json: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "startBackupDryRun"]
        fn start_backup_dry_run(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            excludes_json: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "pollBackupDryRun"]
        fn poll_backup_dry_run(self: Pin<&mut KesticApp>, preview_id: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "cancelBackupDryRun"]
        fn cancel_backup_dry_run(self: Pin<&mut KesticApp>, preview_id: &QString);

        #[qinvokable]
        #[cxx_name = "retentionPresetJson"]
        fn retention_preset_json(self: &KesticApp, preset_name: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "formatRetentionSummaryJson"]
        fn format_retention_summary_json(self: &KesticApp, retention_json: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "startRetentionPreview"]
        fn start_retention_preview(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            retention_json: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "pollRetentionPreview"]
        fn poll_retention_preview(self: Pin<&mut KesticApp>, preview_id: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "cancelRetentionPreview"]
        fn cancel_retention_preview(self: Pin<&mut KesticApp>, preview_id: &QString);

        #[qinvokable]
        #[cxx_name = "detectSource"]
        fn detect_source(self: &KesticApp, path: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "checkNestedSources"]
        fn check_nested_sources(
            self: &KesticApp,
            candidate_id: &QString,
            candidate_path: &QString,
            destination_id: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "runDestinationMaintenance"]
        fn run_destination_maintenance(
            self: Pin<&mut KesticApp>,
            destination_id: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "loadRunDetail"]
        fn load_run_detail(self: Pin<&mut KesticApp>, run_id: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "getBackupTransparentCommand"]
        fn get_backup_transparent_command(self: &KesticApp, backup_id: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "loadBackupSnapshots"]
        fn load_backup_snapshots(self: Pin<&mut KesticApp>, backup_id: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "setResticProvider"]
        fn set_restic_provider(
            self: Pin<&mut KesticApp>,
            kind: &QString,
            custom_path: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "updateExpertMode"]
        fn update_expert_mode(self: Pin<&mut KesticApp>, enabled: bool) -> bool;

        #[qinvokable]
        #[cxx_name = "copyToClipboard"]
        fn copy_to_clipboard(self: &KesticApp, text: &QString);

        #[qinvokable]
        #[cxx_name = "confirmQuit"]
        fn confirm_quit(self: Pin<&mut KesticApp>) -> bool;

        #[qinvokable]
        #[cxx_name = "homeDir"]
        fn home_dir(self: &KesticApp) -> QString;

        #[qinvokable]
        #[cxx_name = "buildBackupConfigTomlSnippet"]
        fn build_backup_config_toml_snippet(self: &KesticApp, backup_id: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "recoveryActionLabel"]
        fn recovery_action_label(self: &KesticApp, action_code: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "pauseGlobal"]
        fn pause_global(self: Pin<&mut KesticApp>, duration_kind: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "resumeGlobal"]
        fn resume_global(self: Pin<&mut KesticApp>) -> bool;

        #[qinvokable]
        #[cxx_name = "pauseBackup"]
        fn pause_backup(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            duration_kind: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "resumeBackup"]
        fn resume_backup(self: Pin<&mut KesticApp>, backup_id: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "startBackupOverride"]
        fn start_backup_override(self: Pin<&mut KesticApp>, backup_id: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "dismissStartAllSkipped"]
        fn dismiss_start_all_skipped(self: Pin<&mut KesticApp>);

        #[qinvokable]
        #[cxx_name = "unlockDestination"]
        fn unlock_destination(self: Pin<&mut KesticApp>, dest_id: &QString) -> i32;

        #[qinvokable]
        #[cxx_name = "previewDestinationMaintenance"]
        fn preview_destination_maintenance(self: Pin<&mut KesticApp>, dest_id: &QString);

        #[qinvokable]
        #[cxx_name = "updateDestinationMaintenance"]
        fn update_destination_maintenance(
            self: Pin<&mut KesticApp>,
            dest_id: &QString,
            maintenance_json: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "updateBackupSchedule"]
        fn update_backup_schedule(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            schedule_json: &QString,
            overdue_threshold: i32,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "updateBackupConditions"]
        fn update_backup_conditions(
            self: Pin<&mut KesticApp>,
            backup_id: &QString,
            conditions_json: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "updateSettings"]
        fn update_settings(
            self: Pin<&mut KesticApp>,
            default_overdue_threshold: i32,
            on_error: bool,
            on_warning: bool,
            on_overdue: bool,
            on_maintenance_failed: bool,
            on_restic_update: bool,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "listKnownNetworksJson"]
        fn list_known_networks_json(self: &KesticApp) -> QString;

        #[qinvokable]
        #[cxx_name = "todayIsoDate"]
        fn today_iso_date(self: &KesticApp) -> QString;

        #[qinvokable]
        #[cxx_name = "validateScheduleJson"]
        fn validate_schedule_json(self: &KesticApp, schedule_json: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "previewScheduleSlotsJson"]
        fn preview_schedule_slots_json(self: &KesticApp, schedule_json: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "formatScheduleSummaryJson"]
        fn format_schedule_summary_json(self: &KesticApp, schedule_json: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "formatConditionsSummaryJson"]
        fn format_conditions_summary_json(self: &KesticApp, conditions_json: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "formatMaintenanceSummaryJson"]
        fn format_maintenance_summary_json(self: &KesticApp, maintenance_json: &QString)
        -> QString;
    }
}

mod backups;
mod destinations;
mod previews;
mod schedules;
mod state;

use crate::state_cache::{lookup_backup_summary, not_found_text, refresh_state_now, shared_state};
use cxx_qt_lib::QString;
use kestic_core::model::Backup;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::KesticDbusError;
use serde::de::DeserializeOwned;
use std::pin::Pin;

#[derive(Default)]
pub struct KesticAppRust {
    initialized: bool,
    has_config_file: bool,
    config_load_error: QString,
    expert_mode: bool,
    daemon_version: QString,
    daemon_build_commit: QString,
    daemon_reachable: bool,
    restic_valid: bool,
    restic_version: QString,
    restic_binary_path: QString,
    restic_provider_kind: QString,
    restic_message: QString,
    blocking_kind: QString,
    blocking_message: QString,
    overall_headline: QString,
    overall_subtitle: QString,
    overall_icon: QString,
    overall_color_kind: QString,
    backups_have_attention: bool,
    destinations_have_attention: bool,
    backups_count: i32,
    startable_backups_count: i32,
    back_up_all_hint: QString,
    destinations_count: i32,
    runs_count: i32,
    manual_run_blocked_message: QString,
    manual_run_blocked_backup_id: QString,
    connection_test_ok: bool,
    connection_test_message: QString,
    connection_test_error_kind: QString,
    connection_test_note: QString,
    snapshots_loading: bool,
    snapshots_error: QString,
    snapshots_has_previous: bool,
    selected_run_id: QString,
    selected_run_title: QString,
    selected_run_status_line: QString,
    selected_run_subtitle: QString,
    selected_run_status_icon: QString,
    selected_run_status_color: QString,
    selected_run_result_summary: QString,
    selected_run_error_summary: QString,
    selected_run_error_action: QString,
    selected_run_error_raw: QString,
    selected_run_unreadable_json: QString,
    selected_run_commands_json: QString,
    selected_run_transparent_command: QString,
    selected_run_log_path: QString,
    selected_run_backup_id: QString,
    has_unsaved_changes: bool,
    last_action_error: QString,
    show_quit_dialog_on_start: bool,
    show_about_on_start: bool,
    quit_has_running_run: bool,
    quit_running_backup_name: QString,
    dry_run_preview_id: QString,
    dry_run_state: QString,
    dry_run_result_json: QString,
    dry_run_error: QString,
    retention_preview_id: QString,
    retention_preview_state: QString,
    retention_preview_result_json: QString,
    retention_preview_error: QString,
    global_paused: bool,
    global_paused_line: QString,
    default_overdue_threshold: i32,
    notif_on_error: bool,
    notif_on_warning: bool,
    notif_on_overdue: bool,
    notif_on_maintenance_failed: bool,
    notif_on_restic_update: bool,
    start_all_skipped_message: QString,
    condition_override_backup_id: QString,
    condition_override_backup_name: QString,
    condition_override_message: QString,
    maintenance_preview_loading: bool,
}

pub(super) use crate::state_cache::dbus_error_text;

/// What a finished daemon call from QML does besides refreshing the state.
#[derive(Clone, Copy, PartialEq)]
enum Finish {
    /// Only clear or show the error line.
    Plain,
    /// The call saved an editor page: it has no unsaved changes any more.
    Saved,
    /// A failure is also shown as a passive notification.
    NotifyOnError,
}

impl qobject::KesticApp {
    /// Shows `text` in the action error line.
    fn show_action_error(mut self: Pin<&mut Self>, text: &str) {
        self.as_mut().set_last_action_error(QString::from(text));
    }

    /// The value of `result`; otherwise shows the error text and returns `None`.
    fn ok_or_show<T, E: ToString>(mut self: Pin<&mut Self>, result: Result<T, E>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(err) => {
                self.as_mut().show_action_error(&err.to_string());
                None
            }
        }
    }

    /// Like [`Self::ok_or_show`] for a failed daemon call.
    fn ok_or_show_dbus<T, E>(mut self: Pin<&mut Self>, result: Result<T, E>) -> Option<T>
    where
        E: Clone + Into<KesticDbusError>,
    {
        match result {
            Ok(value) => Some(value),
            Err(err) => {
                self.as_mut().show_action_error(&dbus_error_text(&err));
                None
            }
        }
    }

    /// Ends a daemon call from QML: on success clears the error line and refreshes the state,
    /// on failure shows the daemon's error text.
    fn finish_action<T, E>(mut self: Pin<&mut Self>, result: Result<T, E>, how: Finish) -> Option<T>
    where
        E: Clone + Into<KesticDbusError>,
    {
        match result {
            Ok(value) => {
                if how == Finish::Saved {
                    self.as_mut().set_has_unsaved_changes(false);
                }
                self.as_mut().show_action_error("");
                refresh_state_now();
                self.as_mut().apply_cached_state();
                Some(value)
            }
            Err(err) => {
                let msg = dbus_error_text(&err);
                self.as_mut().show_action_error(&msg);
                if how == Finish::NotifyOnError {
                    self.as_mut().passive_notification(QString::from(&msg));
                }
                None
            }
        }
    }

    /// The configured backup `backup_id`; shows "not found" if there is none.
    fn existing_backup_or_err(mut self: Pin<&mut Self>, backup_id: &str) -> Option<Backup> {
        let existing = {
            let guard = shared_state().lock_unpoisoned();
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, backup_id))
                .map(|b| b.backup.clone())
        };
        if existing.is_none() {
            self.as_mut().show_action_error(&not_found_text());
        }
        existing
    }

    /// Parses `json` from QML; shows the parse error if it is malformed.
    fn parse_json_or_err<T: DeserializeOwned>(
        mut self: Pin<&mut Self>,
        json: &QString,
    ) -> Option<T> {
        match serde_json::from_str(&json.to_string()) {
            Ok(value) => Some(value),
            Err(err) => {
                self.as_mut().show_action_error(&err.to_string());
                None
            }
        }
    }
}
