// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The synchronous client of `net.daester.Kestic1`.

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

    async fn proxy(&self) -> zbus::Result<Proxy<'_>> {
        Proxy::new(
            &self.conn,
            self.bus_name.as_str(),
            self.object_path.as_str(),
            KESTIC_INTERFACE_V1,
        )
        .await
    }

    /// Calls `method` on the daemon interface and blocks for the reply.
    fn call<B, R>(&self, method: &'static str, body: &B) -> zbus::Result<R>
    where
        B: Serialize + DynamicType,
        R: for<'d> DynamicDeserialize<'d>,
    {
        block_on(async { self.proxy().await?.call(method, body).await })
    }

    /// Calls a method that replies with a JSON string and parses the reply into `T`.
    fn call_json<B, T>(&self, method: &'static str, body: &B) -> Result<T, KesticDbusError>
    where
        B: Serialize + DynamicType,
        T: DeserializeOwned,
    {
        let raw: String = self.call(method, body).map_err(KesticDbusError::from)?;
        serde_json::from_str(&raw).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    pub fn daemon_version(&self) -> zbus::Result<String> {
        self.call("DaemonVersion", &())
    }

    pub fn get_state(&self) -> Result<DaemonStateDto, KesticDbusError> {
        self.call_json("GetState", &())
    }

    pub fn get_run_detail(&self, run_id: &str) -> Result<RunDetailDto, KesticDbusError> {
        self.call_json("GetRunDetail", &(run_id,))
    }

    pub fn list_backup_snapshots(
        &self,
        backup_id: &str,
    ) -> Result<BackupSnapshotsDto, KesticDbusError> {
        self.call_json("ListBackupSnapshots", &(backup_id,))
    }

    pub fn list_destination_snapshots(
        &self,
        destination_id: &str,
    ) -> Result<DestinationSnapshotsDto, KesticDbusError> {
        self.call_json("ListDestinationSnapshots", &(destination_id,))
    }

    pub fn start_snapshot_query(
        &self,
        request: &SnapshotQueryRequest,
    ) -> Result<String, KesticDbusError> {
        let json = to_json_arg(request)?;
        self.call("StartSnapshotQuery", &(json.as_str(),))
            .map_err(KesticDbusError::from)
    }

    pub fn get_snapshot_query_status(
        &self,
        query_id: &str,
        page: &SnapshotQueryPage,
    ) -> Result<SnapshotQueryStatusDto, KesticDbusError> {
        let json = to_json_arg(page)?;
        self.call_json("GetSnapshotQueryStatus", &(query_id, json.as_str()))
    }

    pub fn cancel_snapshot_query(&self, query_id: &str) -> zbus::Result<()> {
        self.call("CancelSnapshotQuery", &(query_id,))
    }

    /// Starts a restore run; returns its run ID.
    pub fn start_restore(&self, request: &RestoreRequest) -> Result<String, KesticDbusError> {
        let json = to_json_arg(request)?;
        self.call("StartRestore", &(json.as_str(),))
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
        self.call_json(
            "TestDestinationConnection",
            &(json.as_str(), mode, repo_password, secrets.as_str()),
        )
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
        self.call(
            "AddDestination",
            &(json.as_str(), mode, repo_password, secrets.as_str()),
        )
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
        let raw: String = self.call(
            "UpdateDestination",
            &(
                json.as_str(),
                mode,
                repo_password,
                secrets.as_str(),
                verify_connection,
            ),
        )?;
        serde_json::from_str(&raw)
            .map_err(|e| zbus::Error::Failure(format!("UpdateDestination reply: {e}")))
    }

    pub fn update_destination_password(
        &self,
        dest_id: &str,
        new_password: &str,
    ) -> zbus::Result<()> {
        self.call("UpdateDestinationPassword", &(dest_id, new_password))
    }

    pub fn trust_host_key(&self, host_key_id: &str) -> zbus::Result<()> {
        self.call("TrustHostKey", &(host_key_id,))
    }

    pub fn remove_destination(&self, dest_id: &str) -> zbus::Result<()> {
        self.call("RemoveDestination", &(dest_id,))
    }

    pub fn add_backup(&self, backup: &Backup) -> zbus::Result<String> {
        let json = to_failure_arg("backup", backup)?;
        self.call("AddBackup", &(json.as_str(),))
    }

    pub fn update_backup(&self, backup: &Backup) -> zbus::Result<()> {
        let json = to_failure_arg("backup", backup)?;
        self.call("UpdateBackup", &(json.as_str(),))
    }

    pub fn rename_backup(&self, backup_id: &str, new_name: &str) -> zbus::Result<()> {
        self.call("RenameBackup", &(backup_id, new_name))
    }

    pub fn reassign_backup(&self, backup_id: &str, new_dest_id: &str) -> zbus::Result<()> {
        self.call("ReassignBackup", &(backup_id, new_dest_id))
    }

    pub fn remove_backup(&self, backup_id: &str) -> zbus::Result<()> {
        self.call("RemoveBackup", &(backup_id,))
    }

    pub fn set_restic_provider(
        &self,
        provider: &kestic_core::model::ResticProvider,
    ) -> Result<ResticStatusDto, KesticDbusError> {
        let json = to_json_arg(provider)?;
        self.call_json("SetResticProvider", &(json.as_str(),))
    }

    pub fn remove_app_managed_restic(&self) -> zbus::Result<()> {
        self.call("RemoveAppManagedRestic", &())
    }

    pub fn set_expert_mode(&self, enabled: bool) -> zbus::Result<()> {
        self.call("SetExpertMode", &(enabled,))
    }

    pub fn start_backup(&self, backup_id: &str) -> zbus::Result<String> {
        self.call("StartBackup", &(backup_id,))
    }

    pub fn start_all_backups(&self) -> Result<StartAllResultDto, KesticDbusError> {
        self.call_json("StartAllBackups", &())
    }

    pub fn cancel_run(&self, run_id: &str) -> zbus::Result<()> {
        self.call("CancelRun", &(run_id,))
    }

    pub fn check_availability(
        &self,
        force: bool,
    ) -> Result<AvailabilitySnapshotDto, KesticDbusError> {
        self.call_json("CheckAvailability", &(force,))
    }

    pub fn window_active(&self, active: bool, page: &str) -> zbus::Result<()> {
        self.call("WindowActive", &(active, page))
    }

    pub fn register_gui(&self) -> zbus::Result<()> {
        self.call("RegisterGui", &())
    }

    pub fn confirm_quit(&self) -> zbus::Result<()> {
        self.call("ConfirmQuit", &())
    }

    pub fn unlock_secret_store(&self) -> zbus::Result<bool> {
        self.call("UnlockSecretStore", &())
    }

    pub fn detect_source(&self, path: &str) -> Result<Source, KesticDbusError> {
        self.call_json("DetectSource", &(path,))
    }

    pub fn check_nested_sources(
        &self,
        candidate_id: &str,
        candidate_path: &str,
        destination_id: &str,
    ) -> Result<Vec<NestedSourceConflict>, KesticDbusError> {
        self.call_json(
            "CheckNestedSources",
            &(candidate_id, candidate_path, destination_id),
        )
    }

    pub fn lint_excludes(
        &self,
        excludes: &ExcludeRules,
    ) -> Result<Vec<ExcludeLintWarning>, KesticDbusError> {
        let json = to_json_arg(excludes)?;
        self.call_json("LintExcludes", &(json.as_str(),))
    }

    pub fn test_exclude_path(
        &self,
        backup_id: &str,
        excludes: &ExcludeRules,
        test_path: &str,
    ) -> Result<ExcludePathTestResult, KesticDbusError> {
        let json = to_json_arg(excludes)?;
        self.call_json("TestExcludePath", &(backup_id, json.as_str(), test_path))
    }

    pub fn start_retention_preview(
        &self,
        backup_id: &str,
        retention: &RetentionPolicy,
    ) -> zbus::Result<String> {
        let json = to_failure_arg("retention", retention)?;
        self.call("StartRetentionPreview", &(backup_id, json.as_str()))
    }

    pub fn cancel_retention_preview(&self, preview_id: &str) -> zbus::Result<()> {
        self.call("CancelRetentionPreview", &(preview_id,))
    }

    pub fn get_retention_preview_status(
        &self,
        preview_id: &str,
    ) -> Result<RetentionPreviewStatusDto, KesticDbusError> {
        self.call_json("GetRetentionPreviewStatus", &(preview_id,))
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
        self.call("StartBackupDryRun", &(backup_id, json.as_str()))
    }

    pub fn cancel_backup_dry_run(&self, preview_id: &str) -> zbus::Result<()> {
        self.call("CancelBackupDryRun", &(preview_id,))
    }

    pub fn get_backup_dry_run_status(
        &self,
        preview_id: &str,
    ) -> Result<BackupDryRunStatusDto, KesticDbusError> {
        self.call_json("GetBackupDryRunStatus", &(preview_id,))
    }

    pub fn start_restic_install(&self) -> zbus::Result<String> {
        self.call("StartResticInstall", &())
    }

    pub fn get_restic_install_status(
        &self,
        install_id: &str,
    ) -> Result<ResticInstallStatusDto, KesticDbusError> {
        self.call_json("GetResticInstallStatus", &(install_id,))
    }

    pub fn cancel_restic_install(&self, install_id: &str) -> zbus::Result<()> {
        self.call("CancelResticInstall", &(install_id,))
    }

    pub fn format_exclude_rule_for_item(
        &self,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, KesticDbusError> {
        self.call_json("FormatExcludeRuleForItem", &(item_path,))
    }

    pub fn add_exclude_rule_for_item(
        &self,
        backup_id: &str,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, KesticDbusError> {
        self.call_json("AddExcludeRuleForItem", &(backup_id, item_path))
    }

    pub fn run_destination_maintenance(&self, destination_id: &str) -> zbus::Result<String> {
        self.call("RunDestinationMaintenance", &(destination_id,))
    }

    pub fn pause_global(&self, duration: &PauseDuration) -> zbus::Result<()> {
        let json = to_failure_arg("duration", duration)?;
        self.call("PauseGlobal", &(json.as_str(),))
    }

    pub fn resume_global(&self) -> zbus::Result<()> {
        self.call("ResumeGlobal", &())
    }

    pub fn pause_backup(&self, backup_id: &str, duration: &PauseDuration) -> zbus::Result<()> {
        let json = to_failure_arg("duration", duration)?;
        self.call("PauseBackup", &(backup_id, json.as_str()))
    }

    pub fn resume_backup(&self, backup_id: &str) -> zbus::Result<()> {
        self.call("ResumeBackup", &(backup_id,))
    }

    pub fn start_backup_override(&self, backup_id: &str) -> zbus::Result<String> {
        self.call("StartBackupOverride", &(backup_id,))
    }

    pub fn unlock_destination(&self, destination_id: &str) -> zbus::Result<u32> {
        self.call("UnlockDestination", &(destination_id,))
    }

    pub fn preview_destination_maintenance(
        &self,
        destination_id: &str,
    ) -> Result<PrunePreviewSummary, KesticDbusError> {
        self.call_json("PreviewDestinationMaintenance", &(destination_id,))
    }

    pub fn update_destination_maintenance(
        &self,
        destination_id: &str,
        maintenance: &MaintenanceSchedule,
    ) -> zbus::Result<()> {
        let json = to_failure_arg("maintenance", maintenance)?;
        self.call(
            "UpdateDestinationMaintenance",
            &(destination_id, json.as_str()),
        )
    }

    pub fn update_settings(
        &self,
        default_overdue_threshold: u32,
        notifications: &NotificationSettings,
    ) -> zbus::Result<()> {
        let json = to_failure_arg("notifications", notifications)?;
        self.call(
            "UpdateSettings",
            &(default_overdue_threshold, json.as_str()),
        )
    }

    pub fn list_known_networks(&self) -> Result<Vec<String>, KesticDbusError> {
        self.call_json("ListKnownNetworks", &())
    }

    pub fn list_destination_types(&self) -> Result<Vec<DestinationTypeDto>, KesticDbusError> {
        self.call_json("ListDestinationTypes", &())
    }

    pub fn list_removable_drives(&self) -> Result<Vec<RemovableDriveDto>, KesticDbusError> {
        self.call_json("ListRemovableDrives", &())
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
