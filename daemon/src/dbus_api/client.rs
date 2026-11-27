// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The synchronous client of `net.daester.Kestic1`.

use super::proxy::Kestic1Proxy;
use super::*;

/// Synchronous D-Bus client for `net.daester.Kestic1` used by the GUI and D-Bus integration tests.
#[derive(Clone)]
pub struct KesticClient {
    conn: Connection,
    bus_name: String,
    object_path: String,
}

impl KesticClient {
    /// Connects targeting custom bus name and object path.
    pub fn connect(bus_name: &str, object_path: &str) -> zbus::Result<Self> {
        let conn = block_on(Connection::session())?;
        Ok(Self {
            conn,
            bus_name: bus_name.to_string(),
            object_path: object_path.to_string(),
        })
    }

    /// Wraps an already open `zbus::Connection` (used by persistent GUI signal listener).
    pub fn from_connection(conn: Connection, bus_name: &str, object_path: &str) -> Self {
        Self {
            conn,
            bus_name: bus_name.to_string(),
            object_path: object_path.to_string(),
        }
    }

    /// Returns underlying `zbus::Connection` for receiving signals.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Checks if the daemon well-known name currently has an owner on the session bus.
    pub fn is_bus_owned(&self) -> zbus::Result<bool> {
        let conn = self.conn.clone();
        let bus = self.bus_name.clone();
        block_on(async move {
            let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
            dbus.name_has_owner(bus.as_str().try_into()?).await
        })
        .map_err(Into::into)
    }

    /// The generated proxy of the daemon interface on this client's connection. It caches no
    /// properties, so building it sends nothing on the bus.
    pub async fn proxy(&self) -> zbus::Result<Kestic1Proxy<'_>> {
        Kestic1Proxy::builder(&self.conn)
            .destination(self.bus_name.as_str())
            .path(self.object_path.as_str())
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
    }

    /// Runs `call` on the daemon proxy and blocks for the reply.
    fn with_proxy<R>(
        &self,
        call: impl AsyncFnOnce(&Kestic1Proxy<'_>) -> zbus::Result<R>,
    ) -> zbus::Result<R> {
        block_on(async { call(&self.proxy().await?).await })
    }

    /// Runs `call` for a method that replies with a JSON string and parses the reply into `T`.
    fn call_json<T: DeserializeOwned>(
        &self,
        call: impl AsyncFnOnce(&Kestic1Proxy<'_>) -> zbus::Result<String>,
    ) -> Result<T, KesticDbusError> {
        let raw = self.with_proxy(call).map_err(KesticDbusError::from)?;
        serde_json::from_str(&raw).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    pub fn daemon_version(&self) -> zbus::Result<String> {
        self.with_proxy(async |p| p.daemon_version().await)
    }

    pub fn get_state(&self) -> Result<DaemonStateDto, KesticDbusError> {
        self.call_json(async |p| p.get_state().await)
    }

    pub fn get_run_detail(&self, run_id: &str) -> Result<RunDetailDto, KesticDbusError> {
        self.call_json(async |p| p.get_run_detail(run_id).await)
    }

    pub fn list_backup_snapshots(
        &self,
        backup_id: &str,
    ) -> Result<BackupSnapshotsDto, KesticDbusError> {
        self.call_json(async |p| p.list_backup_snapshots(backup_id).await)
    }

    pub fn list_destination_snapshots(
        &self,
        destination_id: &str,
    ) -> Result<DestinationSnapshotsDto, KesticDbusError> {
        self.call_json(async |p| p.list_destination_snapshots(destination_id).await)
    }

    pub fn start_snapshot_query(
        &self,
        request: &SnapshotQueryRequest,
    ) -> Result<String, KesticDbusError> {
        let json = to_json_arg(request)?;
        self.with_proxy(async |p| p.start_snapshot_query(json.as_str()).await)
            .map_err(KesticDbusError::from)
    }

    pub fn get_snapshot_query_status(
        &self,
        query_id: &str,
        page: &SnapshotQueryPage,
    ) -> Result<SnapshotQueryStatusDto, KesticDbusError> {
        let json = to_json_arg(page)?;
        self.call_json(async |p| p.get_snapshot_query_status(query_id, json.as_str()).await)
    }

    pub fn cancel_snapshot_query(&self, query_id: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.cancel_snapshot_query(query_id).await)
    }

    /// Starts a restore run; returns its run ID.
    pub fn start_restore(&self, request: &RestoreRequest) -> Result<String, KesticDbusError> {
        let json = to_json_arg(request)?;
        self.with_proxy(async |p| p.start_restore(json.as_str()).await)
            .map_err(KesticDbusError::from)
    }

    pub fn test_destination_connection(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        backend: &BackendSecrets,
    ) -> Result<ConnectionTestResultDto, KesticDbusError> {
        let json = to_json_arg(dest)?;
        let secrets = to_json_arg(backend)?;
        self.call_json(async |p| {
            p.test_destination_connection(json.as_str(), mode, repo_password, secrets.as_str())
                .await
        })
    }

    pub fn add_destination(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        backend: &BackendSecrets,
    ) -> zbus::Result<String> {
        let json = to_failure_arg("dest", dest)?;
        let secrets = to_failure_arg("backend secrets", backend)?;
        self.with_proxy(async |p| {
            p.add_destination(json.as_str(), mode, repo_password, secrets.as_str())
                .await
        })
    }

    pub fn update_destination(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        backend: &BackendSecrets,
        verify_connection: bool,
    ) -> zbus::Result<DestinationUpdateResultDto> {
        let json = to_failure_arg("dest", dest)?;
        let secrets = to_failure_arg("backend secrets", backend)?;
        let raw: String = self.with_proxy(async |p| {
            p.update_destination(
                json.as_str(),
                mode,
                repo_password,
                secrets.as_str(),
                verify_connection,
            )
            .await
        })?;
        serde_json::from_str(&raw)
            .map_err(|e| zbus::Error::Failure(format!("UpdateDestination reply: {e}")))
    }

    pub fn update_destination_password(
        &self,
        dest_id: &str,
        new_password: &str,
    ) -> zbus::Result<()> {
        self.with_proxy(async |p| p.update_destination_password(dest_id, new_password).await)
    }

    pub fn trust_host_key(&self, host_key_id: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.trust_host_key(host_key_id).await)
    }

    pub fn remove_destination(&self, dest_id: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.remove_destination(dest_id).await)
    }

    pub fn add_backup(&self, backup: &Backup) -> zbus::Result<String> {
        let json = to_failure_arg("backup", backup)?;
        self.with_proxy(async |p| p.add_backup(json.as_str()).await)
    }

    pub fn update_backup(&self, backup: &Backup) -> zbus::Result<()> {
        let json = to_failure_arg("backup", backup)?;
        self.with_proxy(async |p| p.update_backup(json.as_str()).await)
    }

    pub fn rename_backup(&self, backup_id: &str, new_name: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.rename_backup(backup_id, new_name).await)
    }

    pub fn reassign_backup(&self, backup_id: &str, new_dest_id: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.reassign_backup(backup_id, new_dest_id).await)
    }

    pub fn remove_backup(&self, backup_id: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.remove_backup(backup_id).await)
    }

    pub fn set_restic_provider(
        &self,
        provider: &kestic_core::model::ResticProvider,
    ) -> Result<ResticStatusDto, KesticDbusError> {
        let json = to_json_arg(provider)?;
        self.call_json(async |p| p.set_restic_provider(json.as_str()).await)
    }

    pub fn remove_app_managed_restic(&self) -> zbus::Result<()> {
        self.with_proxy(async |p| p.remove_app_managed_restic().await)
    }

    pub fn set_expert_mode(&self, enabled: bool) -> zbus::Result<()> {
        self.with_proxy(async |p| p.set_expert_mode(enabled).await)
    }

    pub fn start_backup(&self, backup_id: &str) -> zbus::Result<String> {
        self.with_proxy(async |p| p.start_backup(backup_id).await)
    }

    pub fn start_all_backups(&self) -> Result<StartAllResultDto, KesticDbusError> {
        self.call_json(async |p| p.start_all_backups().await)
    }

    pub fn cancel_run(&self, run_id: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.cancel_run(run_id).await)
    }

    pub fn check_availability(
        &self,
        force: bool,
    ) -> Result<AvailabilitySnapshotDto, KesticDbusError> {
        self.call_json(async |p| p.check_availability(force).await)
    }

    pub fn window_active(&self, active: bool, page: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.window_active(active, page).await)
    }

    pub fn register_gui(&self) -> zbus::Result<()> {
        self.with_proxy(async |p| p.register_gui().await)
    }

    pub fn confirm_quit(&self) -> zbus::Result<()> {
        self.with_proxy(async |p| p.confirm_quit().await)
    }

    pub fn unlock_secret_store(&self) -> zbus::Result<bool> {
        self.with_proxy(async |p| p.unlock_secret_store().await)
    }

    pub fn detect_source(&self, path: &str) -> Result<Source, KesticDbusError> {
        self.call_json(async |p| p.detect_source(path).await)
    }

    pub fn check_nested_sources(
        &self,
        candidate_id: &str,
        candidate_path: &str,
        destination_id: &str,
    ) -> Result<Vec<NestedSourceConflict>, KesticDbusError> {
        self.call_json(async |p| {
            p.check_nested_sources(candidate_id, candidate_path, destination_id)
                .await
        })
    }

    pub fn lint_excludes(
        &self,
        excludes: &ExcludeRules,
    ) -> Result<Vec<ExcludeLintWarning>, KesticDbusError> {
        let json = to_json_arg(excludes)?;
        self.call_json(async |p| p.lint_excludes(json.as_str()).await)
    }

    pub fn test_exclude_path(
        &self,
        backup_id: &str,
        excludes: &ExcludeRules,
        test_path: &str,
    ) -> Result<ExcludePathTestResult, KesticDbusError> {
        let json = to_json_arg(excludes)?;
        self.call_json(async |p| {
            p.test_exclude_path(backup_id, json.as_str(), test_path)
                .await
        })
    }

    pub fn start_retention_preview(
        &self,
        backup_id: &str,
        retention: &RetentionPolicy,
    ) -> zbus::Result<String> {
        let json = to_failure_arg("retention", retention)?;
        self.with_proxy(async |p| p.start_retention_preview(backup_id, json.as_str()).await)
    }

    pub fn cancel_retention_preview(&self, preview_id: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.cancel_retention_preview(preview_id).await)
    }

    pub fn get_retention_preview_status(
        &self,
        preview_id: &str,
    ) -> Result<RetentionPreviewStatusDto, KesticDbusError> {
        self.call_json(async |p| p.get_retention_preview_status(preview_id).await)
    }

    pub fn start_backup_dry_run(
        &self,
        backup_id: &str,
        excludes: Option<&ExcludeRules>,
    ) -> zbus::Result<String> {
        let json = match excludes {
            Some(ex) => to_failure_arg("excludes", ex)?,
            None => String::new(),
        };
        self.with_proxy(async |p| p.start_backup_dry_run(backup_id, json.as_str()).await)
    }

    pub fn cancel_backup_dry_run(&self, preview_id: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.cancel_backup_dry_run(preview_id).await)
    }

    pub fn get_backup_dry_run_status(
        &self,
        preview_id: &str,
    ) -> Result<BackupDryRunStatusDto, KesticDbusError> {
        self.call_json(async |p| p.get_backup_dry_run_status(preview_id).await)
    }

    pub fn start_restic_install(&self) -> zbus::Result<String> {
        self.with_proxy(async |p| p.start_restic_install().await)
    }

    pub fn get_restic_install_status(
        &self,
        install_id: &str,
    ) -> Result<ResticInstallStatusDto, KesticDbusError> {
        self.call_json(async |p| p.get_restic_install_status(install_id).await)
    }

    pub fn cancel_restic_install(&self, install_id: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.cancel_restic_install(install_id).await)
    }

    pub fn format_exclude_rule_for_item(
        &self,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, KesticDbusError> {
        self.call_json(async |p| p.format_exclude_rule_for_item(item_path).await)
    }

    pub fn add_exclude_rule_for_item(
        &self,
        backup_id: &str,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, KesticDbusError> {
        self.call_json(async |p| p.add_exclude_rule_for_item(backup_id, item_path).await)
    }

    pub fn run_destination_maintenance(&self, destination_id: &str) -> zbus::Result<String> {
        self.with_proxy(async |p| p.run_destination_maintenance(destination_id).await)
    }

    pub fn pause_global(&self, duration: &PauseDuration) -> zbus::Result<()> {
        let json = to_failure_arg("duration", duration)?;
        self.with_proxy(async |p| p.pause_global(json.as_str()).await)
    }

    pub fn resume_global(&self) -> zbus::Result<()> {
        self.with_proxy(async |p| p.resume_global().await)
    }

    pub fn pause_backup(&self, backup_id: &str, duration: &PauseDuration) -> zbus::Result<()> {
        let json = to_failure_arg("duration", duration)?;
        self.with_proxy(async |p| p.pause_backup(backup_id, json.as_str()).await)
    }

    pub fn resume_backup(&self, backup_id: &str) -> zbus::Result<()> {
        self.with_proxy(async |p| p.resume_backup(backup_id).await)
    }

    pub fn start_backup_override(&self, backup_id: &str) -> zbus::Result<String> {
        self.with_proxy(async |p| p.start_backup_override(backup_id).await)
    }

    pub fn unlock_destination(&self, destination_id: &str) -> zbus::Result<u32> {
        self.with_proxy(async |p| p.unlock_destination(destination_id).await)
    }

    pub fn preview_destination_maintenance(
        &self,
        destination_id: &str,
    ) -> Result<PrunePreviewSummary, KesticDbusError> {
        self.call_json(async |p| p.preview_destination_maintenance(destination_id).await)
    }

    pub fn update_destination_maintenance(
        &self,
        destination_id: &str,
        maintenance: &MaintenanceSchedule,
    ) -> zbus::Result<()> {
        let json = to_failure_arg("maintenance", maintenance)?;
        self.with_proxy(async |p| {
            p.update_destination_maintenance(destination_id, json.as_str())
                .await
        })
    }

    pub fn update_settings(
        &self,
        default_overdue_threshold: u32,
        notifications: &NotificationSettings,
    ) -> zbus::Result<()> {
        let json = to_failure_arg("notifications", notifications)?;
        self.with_proxy(async |p| {
            p.update_settings(default_overdue_threshold, json.as_str())
                .await
        })
    }

    pub fn list_known_networks(&self) -> Result<Vec<String>, KesticDbusError> {
        self.call_json(async |p| p.list_known_networks().await)
    }

    pub fn list_destination_types(&self) -> Result<Vec<DestinationTypeDto>, KesticDbusError> {
        self.call_json(async |p| p.list_destination_types().await)
    }

    pub fn list_removable_drives(&self) -> Result<Vec<RemovableDriveDto>, KesticDbusError> {
        self.call_json(async |p| p.list_removable_drives().await)
    }
}

/// Serializes a method argument for a call whose error type is `KesticDbusError`.
pub(crate) fn to_json_arg<T: Serialize>(value: &T) -> Result<String, KesticDbusError> {
    serde_json::to_string(value).map_err(|e| KesticDbusError::Internal(e.to_string()))
}

/// Serializes a method argument for a call whose error type is `zbus::Error`.
fn to_failure_arg<T: Serialize>(what: &str, value: &T) -> zbus::Result<String> {
    serde_json::to_string(value).map_err(|e| zbus::Error::Failure(format!("serialize {what}: {e}")))
}
