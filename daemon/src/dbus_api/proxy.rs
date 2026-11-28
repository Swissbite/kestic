// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The client side of `net.daester.Kestic1`, generated from the method and signal signatures.
//! Arguments and replies that carry DTOs are JSON text, as on the server.

#[zbus::proxy(
    interface = "net.daester.Kestic1",
    default_service = "net.daester.Kestic",
    default_path = "/net/daester/Kestic"
)]
pub trait Kestic1 {
    fn daemon_version(&self) -> zbus::Result<String>;
    fn get_state(&self) -> zbus::Result<String>;
    fn get_run_detail(&self, run_id: &str) -> zbus::Result<String>;
    fn list_backup_snapshots(&self, backup_id: &str) -> zbus::Result<String>;
    fn list_destination_snapshots(&self, destination_id: &str) -> zbus::Result<String>;
    fn start_snapshot_query(&self, request_json: &str) -> zbus::Result<String>;
    fn start_restore(&self, request_json: &str) -> zbus::Result<String>;
    fn get_snapshot_query_status(&self, query_id: &str, page_json: &str) -> zbus::Result<String>;
    fn cancel_snapshot_query(&self, query_id: &str) -> zbus::Result<()>;
    fn test_destination_connection(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        backend_secrets_json: &str,
    ) -> zbus::Result<String>;
    fn add_destination(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        backend_secrets_json: &str,
    ) -> zbus::Result<String>;
    fn update_destination(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        backend_secrets_json: &str,
        verify_connection: bool,
        revision: &str,
    ) -> zbus::Result<String>;
    fn update_destination_password(&self, dest_id: &str, new_password: &str) -> zbus::Result<()>;
    fn trust_host_key(&self, host_key_id: &str) -> zbus::Result<()>;
    fn remove_destination(&self, dest_id: &str) -> zbus::Result<()>;
    fn add_backup(&self, backup_json: &str) -> zbus::Result<String>;
    fn update_backup(&self, backup_json: &str, revision: &str) -> zbus::Result<String>;
    fn rename_backup(&self, backup_id: &str, new_name: &str) -> zbus::Result<()>;
    fn reassign_backup(&self, backup_id: &str, new_dest_id: &str) -> zbus::Result<()>;
    fn remove_backup(&self, backup_id: &str) -> zbus::Result<()>;
    fn set_restic_provider(&self, provider_json: &str) -> zbus::Result<String>;
    fn remove_app_managed_restic(&self) -> zbus::Result<()>;
    fn set_expert_mode(&self, enabled: bool) -> zbus::Result<()>;
    fn start_backup(&self, backup_id: &str) -> zbus::Result<String>;
    fn start_all_backups(&self) -> zbus::Result<String>;
    fn cancel_run(&self, run_id: &str) -> zbus::Result<()>;
    fn check_availability(&self, force: bool) -> zbus::Result<String>;
    fn window_active(&self, active: bool, page: &str) -> zbus::Result<()>;
    fn register_gui(&self) -> zbus::Result<()>;
    fn confirm_quit(&self) -> zbus::Result<()>;
    fn unlock_secret_store(&self) -> zbus::Result<bool>;
    fn detect_source(&self, path: &str) -> zbus::Result<String>;
    fn check_nested_sources(
        &self,
        candidate_id: &str,
        candidate_path: &str,
        destination_id: &str,
    ) -> zbus::Result<String>;
    fn lint_excludes(&self, excludes_json: &str) -> zbus::Result<String>;
    fn test_exclude_path(
        &self,
        backup_id: &str,
        excludes_json: &str,
        test_path: &str,
    ) -> zbus::Result<String>;
    fn start_retention_preview(
        &self,
        backup_id: &str,
        retention_json: &str,
    ) -> zbus::Result<String>;
    fn cancel_retention_preview(&self, preview_id: &str) -> zbus::Result<()>;
    fn get_retention_preview_status(&self, preview_id: &str) -> zbus::Result<String>;
    fn start_backup_dry_run(&self, backup_id: &str, excludes_json: &str) -> zbus::Result<String>;
    fn cancel_backup_dry_run(&self, preview_id: &str) -> zbus::Result<()>;
    fn get_backup_dry_run_status(&self, preview_id: &str) -> zbus::Result<String>;
    fn start_restic_install(&self) -> zbus::Result<String>;
    fn get_restic_install_status(&self, install_id: &str) -> zbus::Result<String>;
    fn cancel_restic_install(&self, install_id: &str) -> zbus::Result<()>;
    fn format_exclude_rule_for_item(&self, item_path: &str) -> zbus::Result<String>;
    fn add_exclude_rule_for_item(&self, backup_id: &str, item_path: &str) -> zbus::Result<String>;
    fn run_destination_maintenance(&self, destination_id: &str) -> zbus::Result<String>;
    fn pause_global(&self, duration_json: &str) -> zbus::Result<()>;
    fn resume_global(&self) -> zbus::Result<()>;
    fn pause_backup(&self, backup_id: &str, duration_json: &str) -> zbus::Result<()>;
    fn resume_backup(&self, backup_id: &str) -> zbus::Result<()>;
    fn start_backup_override(&self, backup_id: &str) -> zbus::Result<String>;
    fn unlock_destination(&self, destination_id: &str) -> zbus::Result<u32>;
    fn preview_destination_maintenance(&self, destination_id: &str) -> zbus::Result<String>;
    fn update_destination_maintenance(
        &self,
        destination_id: &str,
        maintenance_json: &str,
    ) -> zbus::Result<()>;
    fn update_settings(
        &self,
        default_overdue_threshold: u32,
        notifications_json: &str,
    ) -> zbus::Result<()>;
    fn list_known_networks(&self) -> zbus::Result<String>;
    fn list_destination_types(&self) -> zbus::Result<String>;
    fn list_removable_drives(&self) -> zbus::Result<String>;

    #[zbus(signal)]
    fn status_changed(&self, overall_status_json: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    fn run_updated(&self, run_json: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    fn run_progress(&self, progress_json: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    fn manual_run_blocked(&self, backup_id: &str, reason: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    fn show_window_requested(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn show_about_requested(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn show_restore_requested(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn quit_confirmation_requested(
        &self,
        has_running_run: bool,
        running_backup_name: &str,
    ) -> zbus::Result<()>;
}
