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
        #[qproperty(i32, destinations_count)]
        #[qproperty(i32, runs_count)]
        #[qproperty(QString, manual_run_blocked_message)]
        #[qproperty(QString, manual_run_blocked_backup_id)]
        #[qproperty(bool, connection_test_ok)]
        #[qproperty(QString, connection_test_message)]
        #[qproperty(QString, connection_test_error_kind)]
        #[qproperty(bool, snapshots_loading)]
        #[qproperty(QString, snapshots_error)]
        #[qproperty(bool, snapshots_has_previous)]
        #[qproperty(QString, selected_run_id)]
        #[qproperty(QString, selected_run_title)]
        #[qproperty(QString, selected_run_status_line)]
        #[qproperty(QString, selected_run_subtitle)]
        #[qproperty(QString, selected_run_status_icon)]
        #[qproperty(QString, selected_run_status_color)]
        #[qproperty(QString, selected_run_stats_summary)]
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
        #[cxx_name = "testDestination"]
        fn test_destination(
            self: Pin<&mut KesticApp>,
            fields_json: &QString,
            mode: &QString,
            repo_pw: &QString,
            rest_pw: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "addDestination"]
        fn add_destination(
            self: Pin<&mut KesticApp>,
            fields_json: &QString,
            mode: &QString,
            repo_pw: &QString,
            rest_pw: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "updateDestination"]
        fn update_destination(
            self: Pin<&mut KesticApp>,
            id: &QString,
            fields_json: &QString,
            mode: &QString,
            repo_pw: &QString,
            rest_pw: &QString,
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
        fn preview_destination_maintenance(self: Pin<&mut KesticApp>, dest_id: &QString)
        -> QString;

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

use crate::smoke;
use crate::state_cache::{
    build_destination, build_new_backup, build_provider, clone_updated_backup,
    clone_updated_backup_conditions, clone_updated_backup_excludes, clone_updated_backup_retention,
    clone_updated_backup_schedule, clone_updated_backup_source, current_now,
    ensure_daemon_and_refresh, format_bytes, format_conditions_summary, format_datetime_display,
    format_duration_ms, format_maintenance_summary, format_overall_visual, format_pause_line,
    format_retention_summary, format_run_kind_name, format_run_status, format_schedule_summary,
    initialize_and_sync, localized_condition_override_message, localized_manual_block_message,
    localized_start_all_summary, lookup_backup_summary, make_client, preview_next_schedule_slots,
    refresh_state_now, shared_state, test_destination_connection,
};
use cxx_kde_frameworks::ki18n::i18n;
use cxx_qt_lib::QString;
use kestic_core::backup::build_backup_command_spec;
use kestic_core::model::{
    BackupId, ExcludeRules, MaintenanceSchedule, NotificationSettings, ResticProvider,
    RetentionPolicy, RunConditions, Schedule, normalize_folder_rule,
};
use kestic_core::provider::resolve_and_check;
use kestic_core::scheduler::PauseDuration;
use kestic_daemon::dbus_api::{REASON_CONDITION_NOT_MET, REASON_DESTINATION_UNREACHABLE};
use std::env;
use std::path::PathBuf;
use std::pin::Pin;

#[derive(Default)]
pub struct KesticAppRust {
    initialized: bool,
    has_config_file: bool,
    config_load_error: QString,
    expert_mode: bool,
    daemon_version: QString,
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
    destinations_count: i32,
    runs_count: i32,
    manual_run_blocked_message: QString,
    manual_run_blocked_backup_id: QString,
    connection_test_ok: bool,
    connection_test_message: QString,
    connection_test_error_kind: QString,
    snapshots_loading: bool,
    snapshots_error: QString,
    snapshots_has_previous: bool,
    selected_run_id: QString,
    selected_run_title: QString,
    selected_run_status_line: QString,
    selected_run_subtitle: QString,
    selected_run_status_icon: QString,
    selected_run_status_color: QString,
    selected_run_stats_summary: QString,
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
}

fn friendly_dbus_error(err: &zbus::Error) -> String {
    if let zbus::Error::MethodError(_, Some(detail), _) = err
        && let Some(name) = detail.strip_prefix("DUPLICATE_LOCATION:")
    {
        return crate::state_cache::i18n("This location is already used by destination '%1'.")
            .subs(&[name]);
    }
    let raw = err.to_string();
    if raw.contains("WrongPassword") {
        i18n("Wrong password: the repository rejected this password.").to_string()
    } else if raw.contains("AuthFailed") {
        i18n("Authentication at destination failed.").to_string()
    } else if raw.contains("DestinationUnreachable") {
        i18n("Destination unreachable.").to_string()
    } else if raw.contains("InUse") {
        i18n("Remove or reassign all backups on this destination first.").to_string()
    } else if raw.contains("ManualRunBlocked") {
        i18n("Backup could not start because source or destination is unavailable.").to_string()
    } else if raw.contains("SecretStoreLocked") {
        i18n("The system password store (Secret Service / KWallet) is locked.").to_string()
    } else if raw.contains("ResticUnavailable") {
        i18n("restic is unavailable or too old.").to_string()
    } else if raw.contains("AlreadyRunning") {
        i18n("Another run is already active on this destination.").to_string()
    } else {
        raw
    }
}

impl qobject::KesticApp {
    pub fn initialize(mut self: Pin<&mut Self>) {
        initialize_and_sync();
        self.as_mut().set_initialized(true);
        self.as_mut().apply_cached_state();
        let show_quit = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard.launch.show_quit_dialog_on_start
        };
        if show_quit {
            let (has_running, running_name) = {
                let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                let active = guard
                    .state
                    .as_ref()
                    .and_then(|s| s.overall.active_run_progress.as_ref());
                (
                    active.is_some(),
                    active.map(|a| a.backup_name.clone()).unwrap_or_default(),
                )
            };
            self.as_mut().set_quit_has_running_run(has_running);
            self.as_mut()
                .set_quit_running_backup_name(QString::from(&running_name));
            self.as_mut().set_show_quit_dialog_on_start(true);
            self.as_mut()
                .quit_confirmation_requested(has_running, QString::from(&running_name));
        }
    }

    pub fn poll_events(mut self: Pin<&mut Self>) {
        let (state_dirty, progress_dirty, blocked, show_win, quit_req) = {
            let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            let sd = guard.state_dirty;
            let pd = guard.progress_dirty;
            guard.state_dirty = false;
            guard.progress_dirty = false;
            let blocked = guard.pending_blocked.take();
            let sw = guard.pending_show_window;
            guard.pending_show_window = false;
            let qr = guard.pending_quit.take();
            (sd, pd, blocked, sw, qr)
        };

        if state_dirty {
            refresh_state_now();
            self.as_mut().apply_cached_state();
        } else if progress_dirty {
            self.as_mut().models_updated();
        }

        if let Some((backup_id, reason)) = blocked {
            let (backup_name, unmet_text) = {
                let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                guard
                    .state
                    .as_ref()
                    .and_then(|s| lookup_backup_summary(s, &backup_id))
                    .map(|b| (b.backup.name.clone(), b.unmet_conditions_text.clone()))
                    .unwrap_or_else(|| (backup_id.clone(), None))
            };
            if reason == REASON_CONDITION_NOT_MET {
                let msg = localized_condition_override_message(&backup_name, unmet_text.as_deref());
                self.as_mut()
                    .set_condition_override_backup_id(QString::from(&backup_id));
                self.as_mut()
                    .set_condition_override_backup_name(QString::from(&backup_name));
                self.as_mut()
                    .set_condition_override_message(QString::from(&msg));
                self.as_mut().condition_override_requested(
                    QString::from(&backup_id),
                    QString::from(&backup_name),
                    QString::from(&msg),
                );
            } else {
                let msg = localized_manual_block_message(&backup_name, &reason);
                self.as_mut()
                    .set_manual_run_blocked_backup_id(QString::from(&backup_id));
                self.as_mut()
                    .set_manual_run_blocked_message(QString::from(&msg));
            }
        }

        if show_win {
            self.as_mut().show_window_requested();
        }

        if let Some((has_running, name)) = quit_req {
            self.as_mut().set_quit_has_running_run(has_running);
            self.as_mut()
                .set_quit_running_backup_name(QString::from(&name));
            self.as_mut()
                .quit_confirmation_requested(has_running, QString::from(&name));
        }
    }

    fn apply_cached_state(mut self: Pin<&mut Self>) {
        let (reachable, state_opt) = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            (guard.daemon_reachable, guard.state.clone())
        };

        self.as_mut().set_daemon_reachable(reachable);

        if !reachable || state_opt.is_none() {
            self.as_mut()
                .set_blocking_kind(QString::from("daemon_unreachable"));
            self.as_mut().set_blocking_message(i18n(
                "The background service is not running. Scheduled backups will not take place.",
            ));
            self.as_mut().set_has_config_file(false);
            self.as_mut()
                .set_overall_headline(i18n("Background service not running"));
            self.as_mut().set_overall_subtitle(QString::from(""));
            self.as_mut().set_overall_icon(QString::from("state-error"));
            self.as_mut()
                .set_overall_color_kind(QString::from("negative"));
            self.as_mut().set_backups_have_attention(false);
            self.as_mut().set_destinations_have_attention(false);
            self.as_mut().set_backups_count(0);
            self.as_mut().set_destinations_count(0);
            self.as_mut().set_runs_count(0);
            self.as_mut().set_global_paused(false);
            self.as_mut().set_global_paused_line(QString::from(""));
            self.as_mut().set_default_overdue_threshold(2);
            self.as_mut().set_notif_on_error(true);
            self.as_mut().set_notif_on_warning(true);
            self.as_mut().set_notif_on_overdue(true);
            self.as_mut().set_notif_on_maintenance_failed(true);
            self.as_mut().set_notif_on_restic_update(false);
            self.as_mut().models_updated();
            return;
        }

        let state = state_opt.expect("checked Some above");
        self.as_mut().set_has_config_file(state.has_config_file);
        self.as_mut().set_config_load_error(QString::from(
            state.config_load_error.as_deref().unwrap_or(""),
        ));
        self.as_mut().set_expert_mode(state.config.expert_mode);
        self.as_mut()
            .set_daemon_version(QString::from(&state.daemon_version));

        self.as_mut().set_restic_valid(state.restic_status.valid);
        self.as_mut().set_restic_version(QString::from(
            state.restic_status.version.as_deref().unwrap_or(""),
        ));
        self.as_mut().set_restic_binary_path(QString::from(
            state.restic_status.binary_path.as_deref().unwrap_or(""),
        ));
        let provider_kind = match &state.config.restic_provider {
            ResticProvider::System => "system",
            ResticProvider::AppManaged { .. } => "app_managed",
            ResticProvider::Custom { .. } => "custom",
        };
        self.as_mut()
            .set_restic_provider_kind(QString::from(provider_kind));
        self.as_mut()
            .set_restic_message(QString::from(&state.restic_status.message));

        // Blocking order (§6.1): Daemon -> SecretStore -> restic (Daemon checked above)
        if let Some(b) = &state.blocking {
            self.as_mut().set_blocking_kind(QString::from(&b.kind));
            self.as_mut()
                .set_blocking_message(QString::from(&b.message));
        } else {
            self.as_mut().set_blocking_kind(QString::from(""));
            self.as_mut().set_blocking_message(QString::from(""));
        }

        let (o_icon, o_color) = format_overall_visual(state.overall.kind);
        self.as_mut()
            .set_overall_headline(QString::from(&state.overall.headline));
        self.as_mut()
            .set_overall_subtitle(QString::from(&state.overall.subtitle));
        self.as_mut().set_overall_icon(QString::from(o_icon));
        self.as_mut().set_overall_color_kind(QString::from(o_color));
        self.as_mut()
            .set_backups_have_attention(state.overall.backups_have_attention);
        self.as_mut()
            .set_destinations_have_attention(state.overall.destinations_have_attention);
        self.as_mut()
            .set_backups_count(i32::try_from(state.backups.len()).unwrap_or(0));
        self.as_mut()
            .set_destinations_count(i32::try_from(state.destinations.len()).unwrap_or(0));
        self.as_mut()
            .set_runs_count(i32::try_from(state.recent_runs.len()).unwrap_or(0));

        let global_paused_line = if state.overall.paused {
            state
                .overall
                .paused_label
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    format_pause_line(
                        state.overall.paused_since_unix,
                        state.overall.paused_until_unix,
                        current_now(),
                    )
                })
        } else {
            String::new()
        };
        self.as_mut().set_global_paused(state.overall.paused);
        self.as_mut()
            .set_global_paused_line(QString::from(&global_paused_line));
        self.as_mut().set_default_overdue_threshold(
            i32::try_from(state.config.default_overdue_threshold).unwrap_or(2),
        );
        self.as_mut()
            .set_notif_on_error(state.config.notifications.on_error);
        self.as_mut()
            .set_notif_on_warning(state.config.notifications.on_warning);
        self.as_mut()
            .set_notif_on_overdue(state.config.notifications.on_overdue);
        self.as_mut()
            .set_notif_on_maintenance_failed(state.config.notifications.on_maintenance_failed);
        self.as_mut()
            .set_notif_on_restic_update(state.config.notifications.on_restic_update);

        self.as_mut().models_updated();
    }

    pub fn retry_start_daemon(mut self: Pin<&mut Self>) {
        ensure_daemon_and_refresh();
        self.as_mut().apply_cached_state();
    }

    pub fn unlock_secret_store(self: Pin<&mut Self>) {
        std::thread::spawn(|| {
            if let Ok(client) = make_client() {
                let _ = client.unlock_secret_store();
            }
            let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard.state_dirty = true;
        });
    }

    pub fn window_active(mut self: Pin<&mut Self>, active: bool, page: &QString) {
        if let Ok(client) = make_client() {
            let _ = client.window_active(active, &page.to_string());
            if active {
                refresh_state_now();
                self.as_mut().apply_cached_state();
            }
        }
    }

    pub fn check_availability(mut self: Pin<&mut Self>, force: bool) {
        if let Ok(client) = make_client() {
            let _ = client.check_availability(force);
            refresh_state_now();
            self.as_mut().apply_cached_state();
        }
    }

    pub fn start_backup(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let b_id = backup_id.to_string();
        let Ok(client) = make_client() else {
            self.as_mut()
                .set_last_action_error(i18n("Background service unavailable."));
            return false;
        };
        match client.start_backup(&b_id) {
            Ok(_) => {
                self.as_mut()
                    .set_manual_run_blocked_message(QString::from(""));
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let raw = err.to_string();
                let is_blocked = raw.contains("ManualRunBlocked");
                let err_is_condition = raw.contains(REASON_CONDITION_NOT_MET);
                refresh_state_now();
                self.as_mut().apply_cached_state();
                if is_blocked {
                    {
                        let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                        if let Some((pending_id, _)) = &guard.pending_blocked
                            && pending_id == &b_id
                        {
                            guard.pending_blocked = None;
                        }
                    }
                    let (backup_name, reason_code, unmet_text) = {
                        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                        guard
                            .state
                            .as_ref()
                            .and_then(|s| lookup_backup_summary(s, &b_id))
                            .map(|b| {
                                let code = if err_is_condition {
                                    REASON_CONDITION_NOT_MET.to_string()
                                } else {
                                    b.availability.blocked_reason.clone().unwrap_or_else(|| {
                                        if b.has_unmet_run_condition {
                                            REASON_CONDITION_NOT_MET.to_string()
                                        } else {
                                            REASON_DESTINATION_UNREACHABLE.to_string()
                                        }
                                    })
                                };
                                (b.backup.name.clone(), code, b.unmet_conditions_text.clone())
                            })
                            .unwrap_or_else(|| {
                                (
                                    b_id.clone(),
                                    if err_is_condition {
                                        REASON_CONDITION_NOT_MET.to_string()
                                    } else {
                                        REASON_DESTINATION_UNREACHABLE.to_string()
                                    },
                                    None,
                                )
                            })
                    };
                    if reason_code == REASON_CONDITION_NOT_MET {
                        let msg = localized_condition_override_message(
                            &backup_name,
                            unmet_text.as_deref(),
                        );
                        self.as_mut()
                            .set_condition_override_backup_id(QString::from(&b_id));
                        self.as_mut()
                            .set_condition_override_backup_name(QString::from(&backup_name));
                        self.as_mut()
                            .set_condition_override_message(QString::from(&msg));
                        self.as_mut().condition_override_requested(
                            QString::from(&b_id),
                            QString::from(&backup_name),
                            QString::from(&msg),
                        );
                    } else {
                        let msg = localized_manual_block_message(&backup_name, &reason_code);
                        self.as_mut()
                            .set_manual_run_blocked_backup_id(QString::from(&b_id));
                        self.as_mut()
                            .set_manual_run_blocked_message(QString::from(&msg));
                    }
                } else {
                    let msg = friendly_dbus_error(&err);
                    self.as_mut().set_last_action_error(QString::from(&msg));
                    self.as_mut().passive_notification(QString::from(&msg));
                }
                false
            }
        }
    }

    pub fn start_all_backups(mut self: Pin<&mut Self>) {
        let Ok(client) = make_client() else {
            return;
        };
        match client.start_all_backups() {
            Ok(res) => {
                if let Some(msg) = localized_start_all_summary(&res) {
                    self.as_mut()
                        .set_start_all_skipped_message(QString::from(&msg));
                    self.as_mut().passive_notification(QString::from(&msg));
                } else {
                    self.as_mut()
                        .set_start_all_skipped_message(QString::from(""));
                }
                refresh_state_now();
                self.as_mut().apply_cached_state();
            }
            Err(err) => {
                self.as_mut().passive_notification(QString::from(&err));
            }
        }
    }

    pub fn cancel_run(mut self: Pin<&mut Self>, run_id: &QString) {
        if let Ok(client) = make_client() {
            let _ = client.cancel_run(&run_id.to_string());
            refresh_state_now();
            self.as_mut().apply_cached_state();
        }
    }

    pub fn dismiss_manual_run_blocked(mut self: Pin<&mut Self>) {
        self.as_mut()
            .set_manual_run_blocked_message(QString::from(""));
        self.as_mut()
            .set_manual_run_blocked_backup_id(QString::from(""));
    }

    pub fn test_destination(
        mut self: Pin<&mut Self>,
        fields_json: &QString,
        mode: &QString,
        repo_pw: &QString,
        rest_pw: &QString,
    ) -> bool {
        let res = test_destination_connection(
            &fields_json.to_string(),
            &mode.to_string(),
            &repo_pw.to_string(),
            &rest_pw.to_string(),
        );
        self.as_mut().set_connection_test_ok(res.ok);
        self.as_mut()
            .set_connection_test_message(QString::from(&res.message));
        self.as_mut()
            .set_connection_test_error_kind(QString::from(res.error_kind.as_deref().unwrap_or("")));
        res.ok
    }

    pub fn add_destination(
        mut self: Pin<&mut Self>,
        fields_json: &QString,
        mode: &QString,
        repo_pw: &QString,
        rest_pw: &QString,
    ) -> QString {
        let dest = match build_destination(None, &fields_json.to_string()) {
            Ok(d) => d,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return QString::from("");
            }
        };
        let Ok(client) = make_client() else {
            self.as_mut()
                .set_last_action_error(i18n("Background service unavailable."));
            return QString::from("");
        };
        match client.add_destination(
            &dest,
            &mode.to_string(),
            &repo_pw.to_string(),
            &rest_pw.to_string(),
        ) {
            Ok(id) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                QString::from(&id)
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                if err.to_string().contains("SecretStoreLocked") {
                    refresh_state_now();
                    self.as_mut().apply_cached_state();
                }
                QString::from("")
            }
        }
    }

    pub fn update_destination(
        mut self: Pin<&mut Self>,
        id: &QString,
        fields_json: &QString,
        mode: &QString,
        repo_pw: &QString,
        rest_pw: &QString,
        verify_connection: bool,
    ) -> bool {
        let dest = match build_destination(Some(&id.to_string()), &fields_json.to_string()) {
            Ok(d) => d,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_destination(
            &dest,
            &mode.to_string(),
            &repo_pw.to_string(),
            &rest_pw.to_string(),
            verify_connection,
        ) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_destination_password(
        mut self: Pin<&mut Self>,
        dest_id: &QString,
        new_pw: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_destination_password(&dest_id.to_string(), &new_pw.to_string()) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                self.as_mut()
                    .passive_notification(i18n("Saved password updated"));
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn remove_destination(mut self: Pin<&mut Self>, dest_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.remove_destination(&dest_id.to_string()) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn add_backup(
        mut self: Pin<&mut Self>,
        name: &QString,
        source_path: &QString,
        dest_id: &QString,
        excludes_text: &QString,
    ) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        let src_str = source_path.to_string();
        let detected = client.detect_source(src_str.trim()).ok();
        let backup = match build_new_backup(
            &name.to_string(),
            &src_str,
            &dest_id.to_string(),
            &excludes_text.to_string(),
            detected,
        ) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return QString::from("");
            }
        };
        match client.add_backup(&backup) {
            Ok(id) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                QString::from(&id)
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                QString::from("")
            }
        }
    }

    pub fn update_backup(
        mut self: Pin<&mut Self>,
        id: &QString,
        name: &QString,
        source_path: &QString,
        dest_id: &QString,
        excludes_text: &QString,
        exclude_caches: bool,
    ) -> bool {
        let id_str = id.to_string();
        let existing = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, &id_str))
                .map(|b| b.backup.clone())
        };
        let Some(existing) = existing else {
            self.as_mut()
                .set_last_action_error(i18n("Backup not found."));
            return false;
        };
        let Ok(client) = make_client() else {
            return false;
        };
        let src_str = source_path.to_string();
        let detected = if src_str.trim() != existing.source.path.to_string_lossy().as_ref() {
            client.detect_source(src_str.trim()).ok()
        } else {
            None
        };
        let updated = match clone_updated_backup(
            &existing,
            &name.to_string(),
            &src_str,
            &dest_id.to_string(),
            &excludes_text.to_string(),
            exclude_caches,
            detected,
        ) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        match client.update_backup(&updated) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_backup_excludes(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        excludes_json: &QString,
    ) -> bool {
        let id_str = backup_id.to_string();
        let existing = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, &id_str))
                .map(|b| b.backup.clone())
        };
        let Some(existing) = existing else {
            self.as_mut()
                .set_last_action_error(i18n("Backup not found."));
            return false;
        };
        let excludes: ExcludeRules = match serde_json::from_str(&excludes_json.to_string()) {
            Ok(e) => e,
            Err(err) => {
                self.as_mut()
                    .set_last_action_error(QString::from(&err.to_string()));
                return false;
            }
        };
        let updated = match clone_updated_backup_excludes(&existing, excludes) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_backup(&updated) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_backup_retention(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        retention_json: &QString,
    ) -> bool {
        let id_str = backup_id.to_string();
        let existing = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, &id_str))
                .map(|b| b.backup.clone())
        };
        let Some(existing) = existing else {
            self.as_mut()
                .set_last_action_error(i18n("Backup not found."));
            return false;
        };
        let retention: RetentionPolicy = match serde_json::from_str(&retention_json.to_string()) {
            Ok(r) => r,
            Err(err) => {
                self.as_mut()
                    .set_last_action_error(QString::from(&err.to_string()));
                return false;
            }
        };
        let updated = match clone_updated_backup_retention(&existing, retention) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_backup(&updated) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_backup_source(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        source_path: &QString,
    ) -> bool {
        let id_str = backup_id.to_string();
        let existing = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, &id_str))
                .map(|b| b.backup.clone())
        };
        let Some(existing) = existing else {
            self.as_mut()
                .set_last_action_error(i18n("Backup not found."));
            return false;
        };
        let Ok(client) = make_client() else {
            return false;
        };
        let trimmed = source_path.to_string().trim().to_string();
        let source = client
            .detect_source(&trimmed)
            .unwrap_or_else(|_| kestic_core::model::Source::folder(PathBuf::from(&trimmed)));
        let updated = match clone_updated_backup_source(&existing, source) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        match client.update_backup(&updated) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn rename_backup(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        new_name: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.rename_backup(&backup_id.to_string(), &new_name.to_string()) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn reassign_backup(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        new_dest_id: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.reassign_backup(&backup_id.to_string(), &new_dest_id.to_string()) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn remove_backup(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.remove_backup(&backup_id.to_string()) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn add_exclude_path_to_backup(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        path: &QString,
    ) -> bool {
        let id_str = backup_id.to_string();
        let trimmed = path.to_string().trim().to_string();
        if trimmed.is_empty() {
            return false;
        }
        let Ok(client) = make_client() else {
            return false;
        };
        match client.add_exclude_rule_for_item(&id_str, &trimmed) {
            Ok(_) => {
                refresh_state_now();
                self.as_mut().apply_cached_state();
                self.as_mut()
                    .passive_notification(i18n("Exclude rule added"));
                true
            }
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                false
            }
        }
    }

    pub fn format_exclude_rule_for_item(&self, item_path: &QString) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.format_exclude_rule_for_item(item_path.to_string().trim()) {
            Ok(dto) => {
                QString::from(&serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(_) => QString::from(""),
        }
    }

    pub fn lint_excludes(&self, excludes_json: &QString) -> QString {
        let Ok(excludes) = serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()) else {
            return QString::from("[]");
        };
        let Ok(client) = make_client() else {
            return QString::from("[]");
        };
        match client.lint_excludes(&excludes) {
            Ok(warnings) => QString::from(
                &serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".to_string()),
            ),
            Err(_) => QString::from("[]"),
        }
    }

    pub fn test_exclude_path(
        &self,
        backup_id: &QString,
        excludes_json: &QString,
        test_path: &QString,
    ) -> QString {
        let Ok(excludes) = serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()) else {
            return QString::from("");
        };
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.test_exclude_path(&backup_id.to_string(), &excludes, &test_path.to_string()) {
            Ok(res) => {
                QString::from(&serde_json::to_string(&res).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(_) => QString::from(""),
        }
    }

    pub fn normalize_exclude_folder(&self, folder_path: &QString) -> QString {
        let home = env::var_os("HOME").map(PathBuf::from);
        let normalized = normalize_folder_rule(&folder_path.to_string(), home.as_deref());
        QString::from(&normalized)
    }

    pub fn build_exclude_file_preview(&self, excludes_json: &QString) -> QString {
        let Ok(excludes) = serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()) else {
            return QString::from("");
        };
        QString::from(&excludes.to_exclude_file_contents())
    }

    pub fn start_backup_dry_run(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        excludes_json: &QString,
    ) -> QString {
        let excludes_opt = if excludes_json.to_string().trim().is_empty() {
            None
        } else {
            match serde_json::from_str::<ExcludeRules>(&excludes_json.to_string()) {
                Ok(e) => Some(e),
                Err(err) => {
                    self.as_mut().set_dry_run_state(QString::from("error"));
                    self.as_mut()
                        .set_dry_run_error(QString::from(&err.to_string()));
                    return QString::from("");
                }
            }
        };
        let Ok(client) = make_client() else {
            self.as_mut().set_dry_run_state(QString::from("error"));
            self.as_mut()
                .set_dry_run_error(i18n("Background service unavailable."));
            return QString::from("");
        };
        self.as_mut().set_dry_run_result_json(QString::from(""));
        self.as_mut().set_dry_run_error(QString::from(""));
        match client.start_backup_dry_run(&backup_id.to_string(), excludes_opt.as_ref()) {
            Ok(preview_id) => {
                self.as_mut()
                    .set_dry_run_preview_id(QString::from(&preview_id));
                self.as_mut().set_dry_run_state(QString::from("running"));
                QString::from(&preview_id)
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_dry_run_state(QString::from("error"));
                self.as_mut().set_dry_run_error(QString::from(&msg));
                QString::from("")
            }
        }
    }

    pub fn poll_backup_dry_run(mut self: Pin<&mut Self>, preview_id: &QString) -> QString {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return QString::from("");
        }
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.get_backup_dry_run_status(&pid) {
            Ok(status) => {
                self.as_mut()
                    .set_dry_run_state(QString::from(&status.state));
                if let Some(ref res) = status.result {
                    let rjson = serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string());
                    self.as_mut().set_dry_run_result_json(QString::from(&rjson));
                }
                if let Some(ref err) = status.error {
                    self.as_mut().set_dry_run_error(QString::from(err));
                }
                QString::from(&serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(err) => {
                self.as_mut().set_dry_run_state(QString::from("error"));
                self.as_mut().set_dry_run_error(QString::from(&err));
                QString::from("")
            }
        }
    }

    pub fn cancel_backup_dry_run(mut self: Pin<&mut Self>, preview_id: &QString) {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return;
        }
        if let Ok(client) = make_client() {
            let _ = client.cancel_backup_dry_run(&pid);
            let _ = self.as_mut().poll_backup_dry_run(preview_id);
        }
    }

    pub fn retention_preset_json(&self, preset_name: &QString) -> QString {
        let policy = match preset_name.to_string().as_str() {
            "economical" => RetentionPolicy::economical(),
            "long" => RetentionPolicy::long(),
            _ => RetentionPolicy::standard(),
        };
        QString::from(&serde_json::to_string(&policy).unwrap_or_else(|_| "{}".to_string()))
    }

    pub fn format_retention_summary_json(&self, retention_json: &QString) -> QString {
        let Ok(policy) = serde_json::from_str::<RetentionPolicy>(&retention_json.to_string())
        else {
            return QString::from("");
        };
        QString::from(&format_retention_summary(&policy))
    }

    pub fn start_retention_preview(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        retention_json: &QString,
    ) -> QString {
        let retention = match serde_json::from_str::<RetentionPolicy>(&retention_json.to_string()) {
            Ok(r) => r,
            Err(err) => {
                self.as_mut()
                    .set_retention_preview_state(QString::from("error"));
                self.as_mut()
                    .set_retention_preview_error(QString::from(&err.to_string()));
                return QString::from("");
            }
        };
        let Ok(client) = make_client() else {
            self.as_mut()
                .set_retention_preview_state(QString::from("error"));
            self.as_mut()
                .set_retention_preview_error(i18n("Background service unavailable."));
            return QString::from("");
        };
        self.as_mut()
            .set_retention_preview_result_json(QString::from(""));
        self.as_mut().set_retention_preview_error(QString::from(""));
        match client.start_retention_preview(&backup_id.to_string(), &retention) {
            Ok(preview_id) => {
                self.as_mut()
                    .set_retention_preview_id(QString::from(&preview_id));
                self.as_mut()
                    .set_retention_preview_state(QString::from("running"));
                QString::from(&preview_id)
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut()
                    .set_retention_preview_state(QString::from("error"));
                self.as_mut()
                    .set_retention_preview_error(QString::from(&msg));
                QString::from("")
            }
        }
    }

    pub fn poll_retention_preview(mut self: Pin<&mut Self>, preview_id: &QString) -> QString {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return QString::from("");
        }
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.get_retention_preview_status(&pid) {
            Ok(status) => {
                self.as_mut()
                    .set_retention_preview_state(QString::from(&status.state));
                if let Some(ref res) = status.result {
                    let rjson = serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string());
                    self.as_mut()
                        .set_retention_preview_result_json(QString::from(&rjson));
                }
                if let Some(ref err) = status.error {
                    self.as_mut()
                        .set_retention_preview_error(QString::from(err));
                }
                QString::from(&serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(err) => {
                self.as_mut()
                    .set_retention_preview_state(QString::from("error"));
                self.as_mut()
                    .set_retention_preview_error(QString::from(&err));
                QString::from("")
            }
        }
    }

    pub fn cancel_retention_preview(mut self: Pin<&mut Self>, preview_id: &QString) {
        let pid = preview_id.to_string();
        if pid.is_empty() {
            return;
        }
        if let Ok(client) = make_client() {
            let _ = client.cancel_retention_preview(&pid);
            let _ = self.as_mut().poll_retention_preview(preview_id);
        }
    }

    pub fn detect_source(&self, path: &QString) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.detect_source(path.to_string().trim()) {
            Ok(src) => {
                QString::from(&serde_json::to_string(&src).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(_) => QString::from(""),
        }
    }

    pub fn check_nested_sources(
        &self,
        candidate_id: &QString,
        candidate_path: &QString,
        destination_id: &QString,
    ) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("[]");
        };
        match client.check_nested_sources(
            &candidate_id.to_string(),
            candidate_path.to_string().trim(),
            destination_id.to_string().trim(),
        ) {
            Ok(conflicts) => QString::from(
                &serde_json::to_string(&conflicts).unwrap_or_else(|_| "[]".to_string()),
            ),
            Err(_) => QString::from("[]"),
        }
    }

    pub fn run_destination_maintenance(
        mut self: Pin<&mut Self>,
        destination_id: &QString,
    ) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("");
        };
        match client.run_destination_maintenance(&destination_id.to_string()) {
            Ok(run_id) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                QString::from(&run_id)
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                QString::from("")
            }
        }
    }

    pub fn load_run_detail(mut self: Pin<&mut Self>, run_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.get_run_detail(&run_id.to_string()) {
            Ok(detail) => {
                let title = crate::state_cache::i18n("Run from %1")
                    .subs(&[&format_datetime_display(detail.run.started_at)]);
                let (status_text, status_icon, status_color) = format_run_status(detail.run.status);
                let dur = detail
                    .run
                    .duration_ms
                    .map(format_duration_ms)
                    .unwrap_or_else(|| String::from("–"));
                let status_line = format!("{status_text} · {dur}");
                let kind_title = format_run_kind_name(
                    &detail.run,
                    detail.backup_name.as_deref(),
                    &detail.destination_name,
                );
                let added = detail
                    .run
                    .stats
                    .as_ref()
                    .map(|s| format_bytes(s.data_added))
                    .unwrap_or_else(|| String::from("0 B"));
                let subtitle =
                    crate::state_cache::i18n("%1 · %2 added").subs(&[&kind_title, &added]);

                let stats_summary = if let Some(s) = &detail.run.stats {
                    crate::state_cache::i18n("%1 new · %2 changed · %3 unchanged · %4 processed")
                        .subs(&[
                            &s.files_new.to_string(),
                            &s.files_changed.to_string(),
                            &s.files_unmodified.to_string(),
                            &format_bytes(s.total_bytes_processed),
                        ])
                } else {
                    i18n("No file statistics available for this run.").to_string()
                };

                #[derive(serde::Serialize)]
                struct CmdDisplay {
                    command: String,
                    meta: String,
                }
                let cmds: Vec<CmdDisplay> = detail
                    .transparent_commands
                    .iter()
                    .enumerate()
                    .map(|(idx, cmd)| {
                        let meta = detail
                            .run
                            .commands
                            .get(idx)
                            .map(|c| {
                                crate::state_cache::i18n("Exit %1 · %2").subs(&[
                                    &c.exit_code.to_string(),
                                    &format_duration_ms(c.duration_ms),
                                ])
                            })
                            .unwrap_or_default();
                        CmdDisplay {
                            command: cmd.clone(),
                            meta,
                        }
                    })
                    .collect();

                let transparent_joined = detail.transparent_commands.join("\n");
                let commands_json = serde_json::to_string(&cmds).unwrap_or_else(|_| "[]".into());
                let unreadable_json =
                    serde_json::to_string(&detail.unreadable_files).unwrap_or_else(|_| "[]".into());
                let log_path_str = detail
                    .run
                    .log_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let run_backup_id = detail
                    .run
                    .backup_id()
                    .map(|b| b.as_str().to_string())
                    .unwrap_or_default();

                self.as_mut()
                    .set_selected_run_id(QString::from(detail.run.id.as_str()));
                self.as_mut().set_selected_run_title(QString::from(&title));
                self.as_mut()
                    .set_selected_run_status_line(QString::from(&status_line));
                self.as_mut()
                    .set_selected_run_subtitle(QString::from(&subtitle));
                self.as_mut()
                    .set_selected_run_status_icon(QString::from(status_icon));
                self.as_mut()
                    .set_selected_run_status_color(QString::from(status_color));
                self.as_mut()
                    .set_selected_run_stats_summary(QString::from(&stats_summary));
                self.as_mut().set_selected_run_error_summary(QString::from(
                    detail.error_summary.as_deref().unwrap_or(""),
                ));
                self.as_mut().set_selected_run_error_action(QString::from(
                    detail.error_action.as_deref().unwrap_or(""),
                ));
                self.as_mut().set_selected_run_error_raw(QString::from(
                    detail.error_raw_detail.as_deref().unwrap_or(""),
                ));
                self.as_mut()
                    .set_selected_run_unreadable_json(QString::from(&unreadable_json));
                self.as_mut()
                    .set_selected_run_commands_json(QString::from(&commands_json));
                self.as_mut()
                    .set_selected_run_transparent_command(QString::from(&transparent_joined));
                self.as_mut()
                    .set_selected_run_log_path(QString::from(&log_path_str));
                self.as_mut()
                    .set_selected_run_backup_id(QString::from(&run_backup_id));

                {
                    let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                    guard.run_detail = Some(detail);
                }
                self.as_mut().models_updated();
                true
            }
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                false
            }
        }
    }

    pub fn get_backup_transparent_command(&self, backup_id: &QString) -> QString {
        let id_str = backup_id.to_string();
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = &guard.state else {
            return QString::from("");
        };
        let Ok(bid) = BackupId::parse(&id_str) else {
            return QString::from("");
        };
        let Some(backup) = state.config.backup(&bid) else {
            return QString::from("");
        };
        let Some(dest) = state.config.destination(&backup.destination_id) else {
            return QString::from("");
        };
        let state_dir = kestic_core::backup::default_state_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("~/.local/state/kestic"));
        let exe = env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("kestic"));
        if let Ok(resolved) = resolve_and_check(&state.config.restic_provider)
            && let Ok(spec) = build_backup_command_spec(backup, dest, &state_dir, &resolved, &exe)
        {
            return QString::from(&spec.to_transparent_shell_command());
        }
        QString::from(&format!(
            "RESTIC_PASSWORD_COMMAND='kestic secret get {}' restic -r {} backup {}",
            dest.id,
            dest.kind.repository_uri(),
            backup.source.path.display()
        ))
    }

    pub fn load_backup_snapshots(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        self.as_mut().set_snapshots_loading(true);
        self.as_mut().set_snapshots_error(QString::from(""));
        let Ok(client) = make_client() else {
            self.as_mut().set_snapshots_loading(false);
            self.as_mut()
                .set_snapshots_error(i18n("Background service unavailable."));
            return false;
        };
        match client.list_backup_snapshots(&backup_id.to_string()) {
            Ok(dto) => {
                let err_msg = dto.current_error.clone().unwrap_or_default();
                let has_prev = !dto.previous_destinations.is_empty();
                {
                    let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                    guard.snapshots = Some(dto);
                    guard.snapshots_error = (!err_msg.is_empty()).then_some(err_msg.clone());
                }
                self.as_mut().set_snapshots_loading(false);
                self.as_mut().set_snapshots_error(QString::from(&err_msg));
                self.as_mut().set_snapshots_has_previous(has_prev);
                self.as_mut().snapshots_updated();
                true
            }
            Err(err) => {
                {
                    let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                    guard.snapshots = None;
                    guard.snapshots_error = Some(err.clone());
                }
                self.as_mut().set_snapshots_loading(false);
                self.as_mut().set_snapshots_error(QString::from(&err));
                self.as_mut().set_snapshots_has_previous(false);
                self.as_mut().snapshots_updated();
                false
            }
        }
    }

    pub fn set_restic_provider(
        mut self: Pin<&mut Self>,
        kind: &QString,
        custom_path: &QString,
    ) -> bool {
        let provider = match build_provider(&kind.to_string(), &custom_path.to_string()) {
            Ok(p) => p,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        let Ok(client) = make_client() else {
            return false;
        };
        match client.set_restic_provider(&provider) {
            Ok(_) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                false
            }
        }
    }

    pub fn update_expert_mode(mut self: Pin<&mut Self>, enabled: bool) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.set_expert_mode(enabled) {
            Ok(()) => {
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn copy_to_clipboard(&self, text: &QString) {
        smoke::ffi::copy_to_clipboard(&text.to_string());
    }

    pub fn confirm_quit(mut self: Pin<&mut Self>) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.confirm_quit() {
            Ok(()) => true,
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn home_dir(&self) -> QString {
        let home = env::var("HOME").unwrap_or_else(|_| "/home".to_string());
        QString::from(&home)
    }

    pub fn build_backup_config_toml_snippet(&self, backup_id: &QString) -> QString {
        let id_str = backup_id.to_string();
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = &guard.state else {
            return QString::from("");
        };
        let Ok(bid) = BackupId::parse(&id_str) else {
            return QString::from("");
        };
        let Some(backup) = state.config.backup(&bid) else {
            return QString::from("");
        };
        let snippet = kestic_core::config::backup_toml_snippet(backup);
        QString::from(&snippet)
    }

    pub fn recovery_action_label(&self, action_code: &QString) -> QString {
        match action_code.to_string().as_str() {
            "update_stored_password" => i18n("Update Saved Password…"),
            "add_to_excludes" => i18n("Add to Exclude Rules…"),
            "check_restic_source" => i18n("Configure restic Source…"),
            "unlock_repository" => i18n("Release Lock…"),
            _ => i18n("Retry"),
        }
    }

    pub fn pause_global(mut self: Pin<&mut Self>, duration_kind: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let duration = parse_pause_duration(&duration_kind.to_string());
        match client.pause_global(&duration) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn resume_global(mut self: Pin<&mut Self>) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.resume_global() {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn pause_backup(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        duration_kind: &QString,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let duration = parse_pause_duration(&duration_kind.to_string());
        match client.pause_backup(&backup_id.to_string(), &duration) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn resume_backup(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.resume_backup(&backup_id.to_string()) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn start_backup_override(mut self: Pin<&mut Self>, backup_id: &QString) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        match client.start_backup_override(&backup_id.to_string()) {
            Ok(_) => {
                self.as_mut()
                    .set_condition_override_backup_id(QString::from(""));
                self.as_mut()
                    .set_condition_override_backup_name(QString::from(""));
                self.as_mut()
                    .set_condition_override_message(QString::from(""));
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                false
            }
        }
    }

    pub fn dismiss_start_all_skipped(mut self: Pin<&mut Self>) {
        self.as_mut()
            .set_start_all_skipped_message(QString::from(""));
    }

    pub fn unlock_destination(mut self: Pin<&mut Self>, dest_id: &QString) -> i32 {
        let Ok(client) = make_client() else {
            self.as_mut()
                .set_last_action_error(i18n("Background service unavailable."));
            return -1;
        };
        match client.unlock_destination(&dest_id.to_string()) {
            Ok(count) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                let msg = if count == 0 {
                    i18n("No stale locks found.").to_string()
                } else if count == 1 {
                    i18n("1 stale lock removed.").to_string()
                } else {
                    crate::state_cache::i18n("%1 stale locks removed.").subs(&[&count.to_string()])
                };
                self.as_mut().passive_notification(QString::from(&msg));
                i32::try_from(count).unwrap_or(0)
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                self.as_mut().passive_notification(QString::from(&msg));
                -1
            }
        }
    }

    pub fn preview_destination_maintenance(mut self: Pin<&mut Self>, dest_id: &QString) -> QString {
        let Ok(client) = make_client() else {
            self.as_mut()
                .set_last_action_error(i18n("Background service unavailable."));
            return QString::from("");
        };
        match client.preview_destination_maintenance(&dest_id.to_string()) {
            Ok(summary) => {
                self.as_mut().set_last_action_error(QString::from(""));
                QString::from(&serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_string()))
            }
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                QString::from("")
            }
        }
    }

    pub fn update_destination_maintenance(
        mut self: Pin<&mut Self>,
        dest_id: &QString,
        maintenance_json: &QString,
    ) -> bool {
        let maint: MaintenanceSchedule = match serde_json::from_str(&maintenance_json.to_string()) {
            Ok(m) => m,
            Err(err) => {
                self.as_mut()
                    .set_last_action_error(QString::from(&err.to_string()));
                return false;
            }
        };
        if let Err(err) = maint.validate() {
            self.as_mut()
                .set_last_action_error(QString::from(&err.to_string()));
            return false;
        }
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_destination_maintenance(&dest_id.to_string(), &maint) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_backup_schedule(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        schedule_json: &QString,
        overdue_threshold: i32,
    ) -> bool {
        let id_str = backup_id.to_string();
        let existing = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, &id_str))
                .map(|b| b.backup.clone())
        };
        let Some(existing) = existing else {
            self.as_mut()
                .set_last_action_error(i18n("Backup not found."));
            return false;
        };
        let schedule: Schedule = match serde_json::from_str(&schedule_json.to_string()) {
            Ok(s) => s,
            Err(err) => {
                self.as_mut()
                    .set_last_action_error(QString::from(&err.to_string()));
                return false;
            }
        };
        let threshold_opt = (overdue_threshold > 0).then_some(overdue_threshold as u32);
        let updated = match clone_updated_backup_schedule(&existing, schedule, threshold_opt) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_backup(&updated) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_backup_conditions(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        conditions_json: &QString,
    ) -> bool {
        let id_str = backup_id.to_string();
        let existing = {
            let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, &id_str))
                .map(|b| b.backup.clone())
        };
        let Some(existing) = existing else {
            self.as_mut()
                .set_last_action_error(i18n("Backup not found."));
            return false;
        };
        let conditions: RunConditions = match serde_json::from_str(&conditions_json.to_string()) {
            Ok(c) => c,
            Err(err) => {
                self.as_mut()
                    .set_last_action_error(QString::from(&err.to_string()));
                return false;
            }
        };
        let updated = match clone_updated_backup_conditions(&existing, conditions) {
            Ok(b) => b,
            Err(err) => {
                self.as_mut().set_last_action_error(QString::from(&err));
                return false;
            }
        };
        let Ok(client) = make_client() else {
            return false;
        };
        match client.update_backup(&updated) {
            Ok(()) => {
                self.as_mut().set_has_unsaved_changes(false);
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn update_settings(
        mut self: Pin<&mut Self>,
        default_overdue_threshold: i32,
        on_error: bool,
        on_warning: bool,
        on_overdue: bool,
        on_maintenance_failed: bool,
        on_restic_update: bool,
    ) -> bool {
        let Ok(client) = make_client() else {
            return false;
        };
        let threshold = default_overdue_threshold.max(1) as u32;
        let notifications = NotificationSettings {
            on_error,
            on_warning,
            on_overdue,
            on_maintenance_failed,
            on_restic_update,
        };
        match client.update_settings(threshold, &notifications) {
            Ok(()) => {
                self.as_mut().set_last_action_error(QString::from(""));
                refresh_state_now();
                self.as_mut().apply_cached_state();
                true
            }
            Err(err) => {
                let msg = friendly_dbus_error(&err);
                self.as_mut().set_last_action_error(QString::from(&msg));
                false
            }
        }
    }

    pub fn list_known_networks_json(&self) -> QString {
        let Ok(client) = make_client() else {
            return QString::from("[]");
        };
        match client.list_known_networks() {
            Ok(list) => {
                QString::from(&serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()))
            }
            Err(_) => QString::from("[]"),
        }
    }

    pub fn preview_schedule_slots_json(&self, schedule_json: &QString) -> QString {
        let Ok(schedule) = serde_json::from_str::<Schedule>(&schedule_json.to_string()) else {
            return QString::from("[]");
        };
        if schedule.validate().is_err() {
            return QString::from("[]");
        }
        let slots = preview_next_schedule_slots(&schedule, current_now());
        QString::from(&serde_json::to_string(&slots).unwrap_or_else(|_| "[]".into()))
    }

    pub fn format_schedule_summary_json(&self, schedule_json: &QString) -> QString {
        let Ok(schedule) = serde_json::from_str::<Schedule>(&schedule_json.to_string()) else {
            return QString::from("");
        };
        QString::from(&format_schedule_summary(&schedule))
    }

    pub fn format_conditions_summary_json(&self, conditions_json: &QString) -> QString {
        let Ok(conditions) = serde_json::from_str::<RunConditions>(&conditions_json.to_string())
        else {
            return QString::from("");
        };
        QString::from(&format_conditions_summary(&conditions))
    }

    pub fn format_maintenance_summary_json(&self, maintenance_json: &QString) -> QString {
        let Ok(maint) = serde_json::from_str::<MaintenanceSchedule>(&maintenance_json.to_string())
        else {
            return QString::from("");
        };
        QString::from(&format_maintenance_summary(&maint))
    }
}

fn parse_pause_duration(kind: &str) -> PauseDuration {
    match kind {
        "1h" => PauseDuration::Hours { hours: 1 },
        "3h" => PauseDuration::Hours { hours: 3 },
        "tomorrow" => PauseDuration::UntilTomorrow,
        _ => PauseDuration::Indefinite,
    }
}
