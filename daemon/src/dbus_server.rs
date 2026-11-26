// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! D-Bus server (`net.daester.Kestic1`) and asynchronous signal emitters for the Kestic daemon.

use crate::dbus_api::{
    ActiveRunProgressDto, DaemonStateDto, KESTIC_OBJECT_PATH, KesticDbusError, to_json_arg,
};
use crate::monitors::list_known_networks;
use crate::removable_drives::list_removable_drives;
use crate::service::{
    DaemonCore, add_destination_unlocked, drain_queued_backups_async, get_run_detail_unlocked,
    list_backup_snapshots_unlocked, list_destination_snapshots_unlocked,
    preview_destination_maintenance_unlocked, refresh_availability_unlocked,
    refresh_probe_unlocked, remove_app_managed_restic, remove_destination_unlocked,
    run_destination_maintenance_async, set_restic_provider_unlocked, start_all_backups_async,
    start_backup_async, start_backup_dry_run_async, start_backup_override_async,
    start_restic_install_async, start_restore_async, start_retention_preview_async,
    start_snapshot_query_async, test_destination_connection_unlocked,
    trigger_background_availability_probe, trust_host_key_unlocked, unlock_destination_async,
    update_destination_password_unlocked, update_destination_unlocked,
};
use crate::single_instance::{InstanceAcquire, SingleInstanceLock};
use crate::tray::KesticTrayState;
use kestic_core::destination_types::destination_types;
use kestic_core::model::{BackupId, Run};
use kestic_core::secrets::SecretError;
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::BlockedReason;
use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::Duration;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

pub(crate) struct KesticDbusV1Service {
    core: Arc<Mutex<DaemonCore>>,
}

impl KesticDbusV1Service {
    pub(crate) fn new(core: Arc<Mutex<DaemonCore>>) -> Self {
        Self { core }
    }

    /// Pushes the new state to the tray and emits `StatusChanged`; without a bus connection
    /// there is nobody to tell.
    async fn sync_tray_and_emit(
        &self,
        conn: Option<&Connection>,
        state_dto: &DaemonStateDto,
        tray_state: KesticTrayState,
    ) {
        let Some(conn) = conn else {
            return;
        };
        let tray_opt = {
            let guard = self.core.lock_unpoisoned();
            guard.tray.clone()
        };
        if let Some(tray) = tray_opt {
            tray.set_full_state(tray_state);
        }
        emit_status_changed_async(conn, state_dto).await;
    }

    /// Runs `f` on a worker thread with the shared core, so a slow restic call never blocks the
    /// bus connection; the core's lock is only taken inside `f`. A panic in `f` becomes
    /// [`KesticDbusError::Internal`].
    async fn blocking<T>(
        &self,
        f: impl FnOnce(&Arc<Mutex<DaemonCore>>) -> T + Send + 'static,
    ) -> Result<T, KesticDbusError>
    where
        T: Send + 'static,
    {
        let core = Arc::clone(&self.core);
        run_blocking_task(move || f(&core)).await
    }

    /// Runs `f` on the locked core, then publishes the resulting state. The lock is released
    /// before the first `await`.
    async fn mutate<T, E>(
        &self,
        f: impl FnOnce(&mut DaemonCore) -> Result<T, E> + Send,
    ) -> Result<T, KesticDbusError>
    where
        KesticDbusError: From<E>,
    {
        let (out, conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock_unpoisoned();
            let out = f(&mut guard)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (out, guard.conn.clone(), state_dto, tray_state)
        };
        self.sync_tray_and_emit(conn_opt.as_ref(), &state_dto, tray_state)
            .await;
        Ok(out)
    }

    /// [`Self::mutate`] for methods that return nothing.
    async fn mutate_unit<T, E>(
        &self,
        f: impl FnOnce(&mut DaemonCore) -> Result<T, E> + Send,
    ) -> Result<(), KesticDbusError>
    where
        KesticDbusError: From<E>,
    {
        self.mutate(f).await.map(drop)
    }

    /// Publishes the current state after a change made elsewhere.
    async fn publish_current(&self) {
        let (conn_opt, state_dto, tray_state) = {
            let guard = self.core.lock_unpoisoned();
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        self.sync_tray_and_emit(conn_opt.as_ref(), &state_dto, tray_state)
            .await;
    }
}

#[interface(name = "net.daester.Kestic1")]
impl KesticDbusV1Service {
    #[zbus(name = "DaemonVersion")]
    fn daemon_version(&self) -> String {
        crate::daemon_version().to_string()
    }

    #[zbus(name = "GetState")]
    fn get_state(&self) -> Result<String, KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        let (state_dto, _) = guard.build_state_and_tray();
        to_json_arg(&state_dto)
    }

    #[zbus(name = "GetRunDetail")]
    async fn get_run_detail(&self, run_id: &str) -> Result<String, KesticDbusError> {
        let id = run_id.to_string();
        let detail = self
            .blocking(move |core| get_run_detail_unlocked(core, &id))
            .await??;
        to_json_arg(&detail)
    }

    #[zbus(name = "ListBackupSnapshots")]
    async fn list_backup_snapshots(&self, backup_id: &str) -> Result<String, KesticDbusError> {
        let id = backup_id.to_string();
        let snaps = self
            .blocking(move |core| list_backup_snapshots_unlocked(core, &id))
            .await??;
        to_json_arg(&snaps)
    }

    #[zbus(name = "ListDestinationSnapshots")]
    async fn list_destination_snapshots(
        &self,
        destination_id: &str,
    ) -> Result<String, KesticDbusError> {
        let id = destination_id.to_string();
        let snaps = self
            .blocking(move |core| list_destination_snapshots_unlocked(core, &id))
            .await??;
        to_json_arg(&snaps)
    }

    #[zbus(name = "StartSnapshotQuery")]
    async fn start_snapshot_query(&self, request_json: &str) -> Result<String, KesticDbusError> {
        let req = request_json.to_string();
        self.blocking(move |core| start_snapshot_query_async(core, &req))
            .await?
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "StartRestore")]
    async fn start_restore(&self, request_json: &str) -> Result<String, KesticDbusError> {
        let req = request_json.to_string();
        self.blocking(move |core| start_restore_async(core, &req))
            .await?
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "GetSnapshotQueryStatus")]
    fn get_snapshot_query_status(
        &self,
        query_id: &str,
        page_json: &str,
    ) -> Result<String, KesticDbusError> {
        // The page is cut without the daemon's lock: filtering a long list takes a while.
        let status = self
            .core
            .lock_unpoisoned()
            .snapshot_query_status(query_id, page_json)?;
        to_json_arg(&status.page()?)
    }

    #[zbus(name = "CancelSnapshotQuery")]
    fn cancel_snapshot_query(&self, query_id: &str) -> Result<(), KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        Ok(guard.cancel_snapshot_query(query_id)?)
    }

    #[zbus(name = "TestDestinationConnection")]
    async fn test_destination_connection(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        backend_secrets_json: &str,
    ) -> Result<String, KesticDbusError> {
        let form = DestinationForm::new(dest_json, mode, repo_password, backend_secrets_json);
        let res = self
            .blocking(move |core| {
                test_destination_connection_unlocked(
                    core,
                    &form.dest_json,
                    &form.mode,
                    &form.repo_password,
                    &form.backend_secrets_json,
                )
            })
            .await?;
        to_json_arg(&res)
    }

    #[zbus(name = "AddDestination")]
    async fn add_destination(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        backend_secrets_json: &str,
    ) -> Result<String, KesticDbusError> {
        let form = DestinationForm::new(dest_json, mode, repo_password, backend_secrets_json);
        let (new_id, conn_opt, state_dto, tray_state) = self
            .blocking(move |core| {
                add_destination_unlocked(
                    core,
                    &form.dest_json,
                    &form.mode,
                    &form.repo_password,
                    &form.backend_secrets_json,
                )
            })
            .await??;
        self.sync_tray_and_emit(conn_opt.as_ref(), &state_dto, tray_state)
            .await;
        Ok(new_id)
    }

    #[zbus(name = "UpdateDestination")]
    async fn update_destination(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        backend_secrets_json: &str,
        verify_connection: bool,
    ) -> Result<String, KesticDbusError> {
        let form = DestinationForm::new(dest_json, mode, repo_password, backend_secrets_json);
        let (result, conn_opt, state_dto, tray_state) = self
            .blocking(move |core| {
                update_destination_unlocked(
                    core,
                    &form.dest_json,
                    &form.mode,
                    &form.repo_password,
                    &form.backend_secrets_json,
                    verify_connection,
                )
            })
            .await??;
        self.sync_tray_and_emit(conn_opt.as_ref(), &state_dto, tray_state)
            .await;
        serde_json::to_string(&result).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "UpdateDestinationPassword")]
    async fn update_destination_password(
        &self,
        dest_id: &str,
        new_password: &str,
    ) -> Result<(), KesticDbusError> {
        let id = dest_id.to_string();
        let pw = new_password.to_string();
        let (conn_opt, state_dto, tray_state) = self
            .blocking(move |core| update_destination_password_unlocked(core, &id, &pw))
            .await??;
        self.sync_tray_and_emit(conn_opt.as_ref(), &state_dto, tray_state)
            .await;
        Ok(())
    }

    #[zbus(name = "TrustHostKey")]
    async fn trust_host_key(&self, host_key_id: &str) -> Result<(), KesticDbusError> {
        let id = host_key_id.to_string();
        self.blocking(move |core| trust_host_key_unlocked(core, &id))
            .await??;
        Ok(())
    }

    #[zbus(name = "RemoveDestination")]
    async fn remove_destination(&self, dest_id: &str) -> Result<(), KesticDbusError> {
        let id = dest_id.to_string();
        let (conn_opt, state_dto, tray_state) = self
            .blocking(move |core| remove_destination_unlocked(core, &id))
            .await??;
        self.sync_tray_and_emit(conn_opt.as_ref(), &state_dto, tray_state)
            .await;
        Ok(())
    }

    #[zbus(name = "AddBackup")]
    async fn add_backup(&self, backup_json: &str) -> Result<String, KesticDbusError> {
        self.mutate(|core| core.add_backup(backup_json)).await
    }

    #[zbus(name = "UpdateBackup")]
    async fn update_backup(&self, backup_json: &str) -> Result<(), KesticDbusError> {
        self.mutate_unit(|core| core.update_backup(backup_json))
            .await
    }

    #[zbus(name = "RenameBackup")]
    async fn rename_backup(&self, backup_id: &str, new_name: &str) -> Result<(), KesticDbusError> {
        self.mutate_unit(|core| core.rename_backup(backup_id, new_name))
            .await
    }

    #[zbus(name = "ReassignBackup")]
    async fn reassign_backup(
        &self,
        backup_id: &str,
        new_dest_id: &str,
    ) -> Result<(), KesticDbusError> {
        self.mutate_unit(|core| core.reassign_backup(backup_id, new_dest_id))
            .await
    }

    #[zbus(name = "RemoveBackup")]
    async fn remove_backup(&self, backup_id: &str) -> Result<(), KesticDbusError> {
        self.mutate_unit(|core| core.remove_backup(backup_id)).await
    }

    #[zbus(name = "SetResticProvider")]
    async fn set_restic_provider(&self, provider_json: &str) -> Result<String, KesticDbusError> {
        let pj = provider_json.to_string();
        let (res, conn_opt, state_dto, tray_state) = self
            .blocking(move |core| set_restic_provider_unlocked(core, &pj))
            .await??;
        self.sync_tray_and_emit(conn_opt.as_ref(), &state_dto, tray_state)
            .await;
        to_json_arg(&res)
    }

    #[zbus(name = "SetExpertMode")]
    async fn set_expert_mode(&self, enabled: bool) -> Result<(), KesticDbusError> {
        self.mutate_unit(|core| core.set_expert_mode(enabled)).await
    }

    #[zbus(name = "StartBackup")]
    async fn start_backup(&self, backup_id: &str) -> Result<String, KesticDbusError> {
        let id = backup_id.to_string();
        self.blocking(move |core| start_backup_async(core, &id))
            .await?
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "StartAllBackups")]
    async fn start_all_backups(&self) -> Result<String, KesticDbusError> {
        let res = self.blocking(start_all_backups_async).await?;
        to_json_arg(&res)
    }

    #[zbus(name = "CancelRun")]
    fn cancel_run(&self, run_id: &str) -> Result<(), KesticDbusError> {
        let mut guard = self.core.lock_unpoisoned();
        Ok(guard.cancel_run(run_id)?)
    }

    #[zbus(name = "CheckAvailability")]
    async fn check_availability(&self, force: bool) -> Result<String, KesticDbusError> {
        let avail = self
            .blocking(move |core| refresh_availability_unlocked(core, force))
            .await?;
        self.publish_current().await;
        to_json_arg(&avail)
    }

    #[zbus(name = "WindowActive")]
    async fn window_active(&self, active: bool, page: &str) -> Result<(), KesticDbusError> {
        self.core
            .lock_unpoisoned()
            .set_window_active_state(active, page);
        self.publish_current().await;
        if active {
            trigger_background_availability_probe(Arc::clone(&self.core));
        }
        Ok(())
    }

    #[zbus(name = "RegisterGui")]
    fn register_gui(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(), KesticDbusError> {
        if let Some(sender) = header.sender() {
            let mut guard = self.core.lock_unpoisoned();
            guard.registered_gui_sender = Some(sender.as_str().to_string());
        }
        Ok(())
    }

    #[zbus(name = "ConfirmQuit")]
    fn confirm_quit(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(), KesticDbusError> {
        let sender = header.sender().map_or("unknown", |s| s.as_str());
        eprintln!("kestic-daemon: quit confirmed by {sender}");
        let guard = self.core.lock_unpoisoned();
        guard.request_shutdown();
        Ok(())
    }

    #[zbus(name = "UnlockSecretStore")]
    async fn unlock_secret_store(&self) -> Result<bool, KesticDbusError> {
        let secret_store = {
            let guard = self.core.lock_unpoisoned();
            Arc::clone(&guard.secret_store)
        };
        let core_probe = Arc::clone(&self.core);
        let unlock_res = run_blocking_task(move || {
            let res = secret_store.unlock_interactive();
            refresh_probe_unlocked(&core_probe);
            res
        })
        .await?;

        self.publish_current().await;

        match unlock_res {
            Ok(()) => Ok(true),
            Err(SecretError::LockedOrPromptRequired(_)) => Ok(false),
            Err(err) => Err(KesticDbusError::SecretStoreLocked(err.to_string())),
        }
    }

    #[zbus(name = "DetectSource")]
    fn detect_source(&self, path: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        let src = guard.detect_source(path)?;
        to_json_arg(&src)
    }

    #[zbus(name = "CheckNestedSources")]
    fn check_nested_sources(
        &self,
        candidate_id: &str,
        candidate_path: &str,
        destination_id: &str,
    ) -> Result<String, KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        let conflicts = guard.check_nested_sources(candidate_id, candidate_path, destination_id)?;
        to_json_arg(&conflicts)
    }

    #[zbus(name = "LintExcludes")]
    fn lint_excludes(&self, excludes_json: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        let warnings = guard.lint_excludes(excludes_json)?;
        to_json_arg(&warnings)
    }

    #[zbus(name = "TestExcludePath")]
    fn test_exclude_path(
        &self,
        backup_id: &str,
        excludes_json: &str,
        test_path: &str,
    ) -> Result<String, KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        let res = guard.test_exclude_path(backup_id, excludes_json, test_path)?;
        to_json_arg(&res)
    }

    #[zbus(name = "StartRetentionPreview")]
    async fn start_retention_preview(
        &self,
        backup_id: &str,
        retention_json: &str,
    ) -> Result<String, KesticDbusError> {
        let id = backup_id.to_string();
        let ret = retention_json.to_string();
        self.blocking(move |core| start_retention_preview_async(core, &id, &ret))
            .await?
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "CancelRetentionPreview")]
    fn cancel_retention_preview(&self, preview_id: &str) -> Result<(), KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        Ok(guard.cancel_retention_preview(preview_id)?)
    }

    #[zbus(name = "GetRetentionPreviewStatus")]
    fn get_retention_preview_status(&self, preview_id: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        let st = guard.get_retention_preview_status(preview_id)?;
        to_json_arg(&st)
    }

    #[zbus(name = "StartBackupDryRun")]
    async fn start_backup_dry_run(
        &self,
        backup_id: &str,
        excludes_json: &str,
    ) -> Result<String, KesticDbusError> {
        let id = backup_id.to_string();
        let ex = excludes_json.to_string();
        self.blocking(move |core| start_backup_dry_run_async(core, &id, &ex))
            .await?
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "CancelBackupDryRun")]
    fn cancel_backup_dry_run(&self, preview_id: &str) -> Result<(), KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        Ok(guard.cancel_backup_dry_run(preview_id)?)
    }

    #[zbus(name = "GetBackupDryRunStatus")]
    fn get_backup_dry_run_status(&self, preview_id: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        let st = guard.get_backup_dry_run_status(preview_id)?;
        to_json_arg(&st)
    }

    #[zbus(name = "StartResticInstall")]
    async fn start_restic_install(&self) -> Result<String, KesticDbusError> {
        self.blocking(start_restic_install_async)
            .await?
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "RemoveAppManagedRestic")]
    async fn remove_app_managed_restic(&self) -> Result<(), KesticDbusError> {
        self.blocking(remove_app_managed_restic)
            .await?
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "GetResticInstallStatus")]
    fn get_restic_install_status(&self, install_id: &str) -> Result<String, KesticDbusError> {
        let mut guard = self.core.lock_unpoisoned();
        let status = guard.get_restic_install_status(install_id)?;
        to_json_arg(&status)
    }

    #[zbus(name = "CancelResticInstall")]
    fn cancel_restic_install(&self, install_id: &str) -> Result<(), KesticDbusError> {
        let mut guard = self.core.lock_unpoisoned();
        Ok(guard.cancel_restic_install(install_id)?)
    }

    #[zbus(name = "FormatExcludeRuleForItem")]
    fn format_exclude_rule_for_item(&self, item_path: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock_unpoisoned();
        let preview = guard.format_exclude_rule_for_item(item_path)?;
        to_json_arg(&preview)
    }

    #[zbus(name = "AddExcludeRuleForItem")]
    async fn add_exclude_rule_for_item(
        &self,
        backup_id: &str,
        item_path: &str,
    ) -> Result<String, KesticDbusError> {
        let preview = self
            .mutate(|core| core.add_exclude_rule_for_item(backup_id, item_path))
            .await?;
        to_json_arg(&preview)
    }

    #[zbus(name = "RunDestinationMaintenance")]
    async fn run_destination_maintenance(
        &self,
        destination_id: &str,
    ) -> Result<String, KesticDbusError> {
        let id = destination_id.to_string();
        self.blocking(move |core| run_destination_maintenance_async(core, &id))
            .await?
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "PauseGlobal")]
    async fn pause_global(&self, duration_json: &str) -> Result<(), KesticDbusError> {
        self.mutate_unit(|core| core.pause_global(duration_json))
            .await
    }

    #[zbus(name = "ResumeGlobal")]
    async fn resume_global(&self) -> Result<(), KesticDbusError> {
        self.mutate(|core| core.resume_global()).await?;
        self.blocking(drain_queued_backups_async).await?;
        Ok(())
    }

    #[zbus(name = "PauseBackup")]
    async fn pause_backup(
        &self,
        backup_id: &str,
        duration_json: &str,
    ) -> Result<(), KesticDbusError> {
        self.mutate_unit(|core| core.pause_backup(backup_id, duration_json))
            .await
    }

    #[zbus(name = "ResumeBackup")]
    async fn resume_backup(&self, backup_id: &str) -> Result<(), KesticDbusError> {
        self.mutate(|core| core.resume_backup(backup_id)).await?;
        self.blocking(drain_queued_backups_async).await?;
        Ok(())
    }

    #[zbus(name = "StartBackupOverride")]
    async fn start_backup_override(&self, backup_id: &str) -> Result<String, KesticDbusError> {
        let id = backup_id.to_string();
        self.blocking(move |core| start_backup_override_async(core, &id))
            .await?
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "UnlockDestination")]
    async fn unlock_destination(&self, destination_id: &str) -> Result<u32, KesticDbusError> {
        let id = destination_id.to_string();
        let count = self
            .blocking(move |core| unlock_destination_async(core, &id))
            .await??;

        self.publish_current().await;
        Ok(count)
    }

    #[zbus(name = "PreviewDestinationMaintenance")]
    async fn preview_destination_maintenance(
        &self,
        destination_id: &str,
    ) -> Result<String, KesticDbusError> {
        let id = destination_id.to_string();
        let preview = self
            .blocking(move |core| preview_destination_maintenance_unlocked(core, &id))
            .await??;
        to_json_arg(&preview)
    }

    #[zbus(name = "UpdateDestinationMaintenance")]
    async fn update_destination_maintenance(
        &self,
        destination_id: &str,
        maintenance_json: &str,
    ) -> Result<(), KesticDbusError> {
        self.mutate_unit(|core| {
            core.update_destination_maintenance(destination_id, maintenance_json)
        })
        .await
    }

    #[zbus(name = "UpdateSettings")]
    async fn update_settings(
        &self,
        default_overdue_threshold: u32,
        notifications_json: &str,
    ) -> Result<(), KesticDbusError> {
        self.mutate_unit(|core| core.update_settings(default_overdue_threshold, notifications_json))
            .await
    }

    #[zbus(name = "ListKnownNetworks")]
    async fn list_known_networks(&self) -> Result<String, KesticDbusError> {
        let monitor_bus = {
            let guard = self.core.lock_unpoisoned();
            guard.monitor_bus.clone()
        };
        let networks = run_blocking_task(move || list_known_networks(&monitor_bus)).await?;
        to_json_arg(&networks)
    }

    /// Destination types with «ready» or «needs installation», checked anew on every call
    /// against the `PATH` the daemon starts restic with.
    #[zbus(name = "ListDestinationTypes")]
    async fn list_destination_types(&self) -> Result<String, KesticDbusError> {
        let search_path = self.core.lock_unpoisoned().program_search_path();
        let types = run_blocking_task(move || destination_types(search_path.as_deref())).await?;
        to_json_arg(&types)
    }

    /// Mounted removable drives from UDisks2, offered by the destination type «External Drive».
    #[zbus(name = "ListRemovableDrives")]
    async fn list_removable_drives(&self) -> Result<String, KesticDbusError> {
        let monitor_bus = {
            let guard = self.core.lock_unpoisoned();
            guard.monitor_bus.clone()
        };
        let drives = run_blocking_task(move || list_removable_drives(&monitor_bus)).await?;
        to_json_arg(&drives)
    }

    #[zbus(signal, name = "StatusChanged")]
    async fn status_changed(
        ctxt: &SignalEmitter<'_>,
        overall_status_json: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal, name = "RunUpdated")]
    async fn run_updated(ctxt: &SignalEmitter<'_>, run_json: &str) -> zbus::Result<()>;

    #[zbus(signal, name = "RunProgress")]
    async fn run_progress(ctxt: &SignalEmitter<'_>, progress_json: &str) -> zbus::Result<()>;

    #[zbus(signal, name = "ManualRunBlocked")]
    async fn manual_run_blocked(
        ctxt: &SignalEmitter<'_>,
        backup_id: &str,
        reason: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal, name = "ShowWindowRequested")]
    async fn show_window_requested(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal, name = "ShowAboutRequested")]
    async fn show_about_requested(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal, name = "ShowRestoreRequested")]
    async fn show_restore_requested(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal, name = "QuitConfirmationRequested")]
    async fn quit_confirmation_requested(
        ctxt: &SignalEmitter<'_>,
        has_running_run: bool,
        running_backup_name: &str,
    ) -> zbus::Result<()>;
}

/// Closes `conn` once every other handle of it is gone, waiting at most `limit`; returns whether
/// it closed in time. A method call holds a handle until its reply has been written, so the reply
/// to the call that stopped the daemon, such as `ConfirmQuit`, still reaches its caller instead of
/// `NoReply`. After `limit` the connection closes as its last handle goes, at the latest when the
/// process ends.
pub fn close_after_pending_replies(conn: Connection, limit: Duration) -> bool {
    let (closed_tx, closed_rx) = mpsc::channel();
    thread::spawn(move || {
        zbus::block_on(conn.graceful_shutdown());
        // The receiver is gone when the wait timed out; nobody is left to tell.
        closed_tx.send(()).ok();
    });
    closed_rx.recv_timeout(limit).is_ok()
}

pub(crate) fn start_dbus_server(
    bus_name: &str,
    core: Arc<Mutex<DaemonCore>>,
) -> Result<InstanceAcquire, String> {
    let dbus_service = KesticDbusV1Service::new(core);
    SingleInstanceLock::acquire_with_interface(bus_name, dbus_service)
        .map_err(|err| format!("failed to acquire D-Bus name `{bus_name}`: {err}"))
}

/// Runs `f` on its own thread and waits for it without blocking the bus connection. A panic in
/// `f` ends the wait with [`KesticDbusError::Internal`], so the caller gets an answer instead of
/// waiting for its D-Bus timeout.
async fn run_blocking_task<T, F>(f: F) -> Result<T, KesticDbusError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    type Slot<T> = (Option<Result<T, KesticDbusError>>, Option<std::task::Waker>);
    let state: Arc<Mutex<Slot<T>>> = Arc::new(Mutex::new((None, None)));
    let state_bg = Arc::clone(&state);
    thread::spawn(move || {
        let value = panic::catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
            KesticDbusError::Internal(format!(
                "background task panicked: {}",
                panic_message(payload.as_ref())
            ))
        });
        let waker = {
            let mut guard = state_bg.lock_unpoisoned();
            guard.0 = Some(value);
            guard.1.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    });
    std::future::poll_fn(move |cx| {
        let mut guard = state.lock_unpoisoned();
        if let Some(val) = guard.0.take() {
            std::task::Poll::Ready(val)
        } else {
            guard.1 = Some(cx.waker().clone());
            std::task::Poll::Pending
        }
    })
    .await
}

/// The text of a panic payload (`panic!` with a literal or a formatted message).
fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

async fn emit_status_changed_async(conn: &Connection, state_dto: &DaemonStateDto) {
    if let Ok(json) = serde_json::to_string(&state_dto.overall)
        && let Ok(emitter) = SignalEmitter::new(conn, KESTIC_OBJECT_PATH)
    {
        let _ = KesticDbusV1Service::status_changed(&emitter, &json).await;
    }
}

type SignalJob = Box<dyn FnOnce() + Send>;

/// Queue of the single signal thread. One thread sends every signal in the order it was
/// queued, so the signals of one run (`RunProgress`, `RunUpdated`) arrive in the order they
/// were emitted and a display never jumps back.
fn signal_queue() -> &'static mpsc::Sender<SignalJob> {
    static QUEUE: OnceLock<mpsc::Sender<SignalJob>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<SignalJob>();
        thread::spawn(move || {
            for job in rx {
                job();
            }
        });
        tx
    })
}

/// Appends a signal to the shared queue of the single signal thread; callers never wait for
/// the bus. A signal nobody receives is not an error worth reporting, so the result is
/// dropped. A bus write that hangs holds back every signal queued after it.
fn queue_signal(
    conn: &Connection,
    emit: impl AsyncFnOnce(&SignalEmitter<'_>) -> zbus::Result<()> + Send + 'static,
) {
    let conn = conn.clone();
    let _ = signal_queue().send(Box::new(move || {
        let _ = zbus::block_on(async move {
            let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
            emit(&emitter).await
        });
    }));
}

/// The destination form fields of a D-Bus call, owned so they can move into a blocking task.
struct DestinationForm {
    dest_json: String,
    mode: String,
    repo_password: String,
    backend_secrets_json: String,
}

impl DestinationForm {
    fn new(dest_json: &str, mode: &str, repo_password: &str, backend_secrets_json: &str) -> Self {
        Self {
            dest_json: dest_json.to_string(),
            mode: mode.to_string(),
            repo_password: repo_password.to_string(),
            backend_secrets_json: backend_secrets_json.to_string(),
        }
    }
}

pub(crate) fn emit_status_changed(conn: &Connection, state_dto: &DaemonStateDto) {
    if let Ok(json) = serde_json::to_string(&state_dto.overall) {
        queue_signal(conn, async move |emitter| {
            KesticDbusV1Service::status_changed(emitter, &json).await
        });
    }
}

pub(crate) fn emit_run_updated(conn: &Connection, run: &Run) {
    if let Ok(json) = serde_json::to_string(run) {
        queue_signal(conn, async move |emitter| {
            KesticDbusV1Service::run_updated(emitter, &json).await
        });
    }
}

pub(crate) fn emit_run_progress(conn: &Connection, progress: &ActiveRunProgressDto) {
    if let Ok(json) = serde_json::to_string(progress) {
        queue_signal(conn, async move |emitter| {
            KesticDbusV1Service::run_progress(emitter, &json).await
        });
    }
}

pub(crate) fn emit_manual_run_blocked(
    conn: &Connection,
    backup_id: &BackupId,
    reason: BlockedReason,
) {
    let bid = backup_id.as_str().to_string();
    let reason = reason.as_str();
    queue_signal(conn, async move |emitter| {
        KesticDbusV1Service::manual_run_blocked(emitter, &bid, reason).await
    });
}

pub(crate) fn emit_show_window_requested(conn: &Connection) {
    queue_signal(conn, async move |emitter| {
        KesticDbusV1Service::show_window_requested(emitter).await
    });
}

pub(crate) fn emit_show_about_requested(conn: &Connection) {
    queue_signal(conn, async move |emitter| {
        KesticDbusV1Service::show_about_requested(emitter).await
    });
}

pub(crate) fn emit_show_restore_requested(conn: &Connection) {
    queue_signal(conn, async move |emitter| {
        KesticDbusV1Service::show_restore_requested(emitter).await
    });
}

pub(crate) fn emit_quit_confirmation_requested(
    conn: &Connection,
    has_running_run: bool,
    running_backup_name: &str,
) {
    let name = running_backup_name.to_string();
    queue_signal(conn, async move |emitter| {
        KesticDbusV1Service::quit_confirmation_requested(emitter, has_running_run, &name).await
    });
}

#[cfg(test)]
#[path = "dbus_server_tests.rs"]
mod tests;
