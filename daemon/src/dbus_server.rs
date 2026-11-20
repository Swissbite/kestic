// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! D-Bus server (`net.daester.Kestic1`) and asynchronous signal emitters for the Kestic daemon.

use crate::dbus_api::{ActiveRunProgressDto, DaemonStateDto, KESTIC_OBJECT_PATH, KesticDbusError};
use crate::monitors::list_known_networks;
use crate::service::{
    DaemonCore, add_destination_unlocked, drain_queued_backups_async, get_run_detail_unlocked,
    list_backup_snapshots_unlocked, preview_destination_maintenance_unlocked,
    preview_retention_unlocked, refresh_availability_unlocked, refresh_probe_unlocked,
    run_destination_maintenance_async, set_restic_provider_unlocked, start_all_backups_async,
    start_backup_async, start_backup_dry_run_async, start_backup_override_async,
    start_retention_preview_async, test_destination_connection_unlocked,
    trigger_background_availability_probe, unlock_destination_async,
    update_destination_password_unlocked, update_destination_unlocked,
};
use crate::single_instance::{InstanceAcquire, SingleInstanceLock};
use crate::tray::KesticTrayState;
use kestic_core::model::{BackupId, Run};
use kestic_core::secrets::SecretError;
use kestic_i18n::codes::BlockedReason;
use std::sync::{Arc, Mutex};
use std::thread;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

pub(crate) struct KesticDbusV1Service {
    core: Arc<Mutex<DaemonCore>>,
}

impl KesticDbusV1Service {
    pub(crate) fn new(core: Arc<Mutex<DaemonCore>>) -> Self {
        Self { core }
    }

    async fn sync_tray_and_emit(
        &self,
        conn: &Connection,
        state_dto: &DaemonStateDto,
        tray_state: KesticTrayState,
    ) {
        let tray_opt = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.tray.clone()
        };
        if let Some(tray) = tray_opt {
            tray.set_full_state(tray_state);
        }
        emit_status_changed_async(conn, state_dto).await;
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
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let (state_dto, _) = guard.build_state_and_tray();
        serde_json::to_string(&state_dto).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "GetRunDetail")]
    async fn get_run_detail(&self, run_id: &str) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = run_id.to_string();
        let detail = run_blocking_task(move || get_run_detail_unlocked(&core, &id))
            .await
            .map_err(KesticDbusError::from)?;
        serde_json::to_string(&detail).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "ListBackupSnapshots")]
    async fn list_backup_snapshots(&self, backup_id: &str) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = backup_id.to_string();
        let snaps = run_blocking_task(move || list_backup_snapshots_unlocked(&core, &id))
            .await
            .map_err(KesticDbusError::from)?;
        serde_json::to_string(&snaps).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "TestDestinationConnection")]
    async fn test_destination_connection(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let dj = dest_json.to_string();
        let m = mode.to_string();
        let rp = repo_password.to_string();
        let rsp = rest_password.to_string();
        let res = run_blocking_task(move || {
            test_destination_connection_unlocked(&core, &dj, &m, &rp, &rsp)
        })
        .await;
        serde_json::to_string(&res).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "AddDestination")]
    async fn add_destination(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let dj = dest_json.to_string();
        let m = mode.to_string();
        let rp = repo_password.to_string();
        let rsp = rest_password.to_string();
        let (new_id, conn_opt, state_dto, tray_state) =
            run_blocking_task(move || add_destination_unlocked(&core, &dj, &m, &rp, &rsp))
                .await
                .map_err(KesticDbusError::from)?;
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(new_id)
    }

    #[zbus(name = "UpdateDestination")]
    async fn update_destination(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
        verify_connection: bool,
    ) -> Result<(), KesticDbusError> {
        let core = Arc::clone(&self.core);
        let dj = dest_json.to_string();
        let m = mode.to_string();
        let rp = repo_password.to_string();
        let rsp = rest_password.to_string();
        let (conn_opt, state_dto, tray_state) = run_blocking_task(move || {
            update_destination_unlocked(&core, &dj, &m, &rp, &rsp, verify_connection)
        })
        .await
        .map_err(KesticDbusError::from)?;
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "UpdateDestinationPassword")]
    async fn update_destination_password(
        &self,
        dest_id: &str,
        new_password: &str,
    ) -> Result<(), KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = dest_id.to_string();
        let pw = new_password.to_string();
        let (conn_opt, state_dto, tray_state) =
            run_blocking_task(move || update_destination_password_unlocked(&core, &id, &pw))
                .await
                .map_err(KesticDbusError::from)?;
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "RemoveDestination")]
    async fn remove_destination(&self, dest_id: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.remove_destination(dest_id)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "AddBackup")]
    async fn add_backup(&self, backup_json: &str) -> Result<String, KesticDbusError> {
        let (new_id, conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let new_id = guard.add_backup(backup_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (new_id, guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(new_id)
    }

    #[zbus(name = "UpdateBackup")]
    async fn update_backup(&self, backup_json: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.update_backup(backup_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "RenameBackup")]
    async fn rename_backup(&self, backup_id: &str, new_name: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.rename_backup(backup_id, new_name)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "ReassignBackup")]
    async fn reassign_backup(
        &self,
        backup_id: &str,
        new_dest_id: &str,
    ) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.reassign_backup(backup_id, new_dest_id)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "RemoveBackup")]
    async fn remove_backup(&self, backup_id: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.remove_backup(backup_id)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "SetResticProvider")]
    async fn set_restic_provider(&self, provider_json: &str) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let pj = provider_json.to_string();
        let (res, conn_opt, state_dto, tray_state) =
            run_blocking_task(move || set_restic_provider_unlocked(&core, &pj))
                .await
                .map_err(KesticDbusError::from)?;
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        serde_json::to_string(&res).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "SetExpertMode")]
    async fn set_expert_mode(&self, enabled: bool) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.set_expert_mode(enabled)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "StartBackup")]
    async fn start_backup(&self, backup_id: &str) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = backup_id.to_string();
        run_blocking_task(move || start_backup_async(&core, &id))
            .await
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "StartAllBackups")]
    async fn start_all_backups(&self) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let res = run_blocking_task(move || start_all_backups_async(&core)).await;
        serde_json::to_string(&res).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "CancelRun")]
    fn cancel_run(&self, run_id: &str) -> Result<(), KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        Ok(guard.cancel_run(run_id)?)
    }

    #[zbus(name = "CheckAvailability")]
    async fn check_availability(&self, force: bool) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let avail = run_blocking_task(move || refresh_availability_unlocked(&core, force)).await;
        let (conn_opt, state_dto, tray_state) = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        serde_json::to_string(&avail).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "WindowActive")]
    async fn window_active(&self, active: bool, page: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.set_window_active_state(active, page);
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
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
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
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
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.request_shutdown();
        Ok(())
    }

    #[zbus(name = "UnlockSecretStore")]
    async fn unlock_secret_store(&self) -> Result<bool, KesticDbusError> {
        let secret_store = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            Arc::clone(&guard.secret_store)
        };
        let core_probe = Arc::clone(&self.core);
        let unlock_res = run_blocking_task(move || {
            let res = secret_store.unlock_interactive();
            refresh_probe_unlocked(&core_probe);
            res
        })
        .await;

        let (conn_opt, state_dto, tray_state) = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }

        match unlock_res {
            Ok(()) => Ok(true),
            Err(SecretError::LockedOrPromptRequired(_)) => Ok(false),
            Err(err) => Err(KesticDbusError::SecretStoreLocked(err.to_string())),
        }
    }

    #[zbus(name = "DetectSource")]
    fn detect_source(&self, path: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let src = guard.detect_source(path)?;
        serde_json::to_string(&src).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "CheckNestedSources")]
    fn check_nested_sources(
        &self,
        candidate_id: &str,
        candidate_path: &str,
        destination_id: &str,
    ) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let conflicts = guard.check_nested_sources(candidate_id, candidate_path, destination_id)?;
        serde_json::to_string(&conflicts).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "LintExcludes")]
    fn lint_excludes(&self, excludes_json: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let warnings = guard.lint_excludes(excludes_json)?;
        serde_json::to_string(&warnings).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "TestExcludePath")]
    fn test_exclude_path(
        &self,
        backup_id: &str,
        excludes_json: &str,
        test_path: &str,
    ) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let res = guard.test_exclude_path(backup_id, excludes_json, test_path)?;
        serde_json::to_string(&res).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "PreviewRetention")]
    async fn preview_retention(
        &self,
        backup_id: &str,
        retention_json: &str,
    ) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = backup_id.to_string();
        let ret = retention_json.to_string();
        let preview = run_blocking_task(move || preview_retention_unlocked(&core, &id, &ret))
            .await
            .map_err(KesticDbusError::from)?;
        serde_json::to_string(&preview).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "StartRetentionPreview")]
    async fn start_retention_preview(
        &self,
        backup_id: &str,
        retention_json: &str,
    ) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = backup_id.to_string();
        let ret = retention_json.to_string();
        run_blocking_task(move || start_retention_preview_async(&core, &id, &ret))
            .await
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "CancelRetentionPreview")]
    fn cancel_retention_preview(&self, preview_id: &str) -> Result<(), KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        Ok(guard.cancel_retention_preview(preview_id)?)
    }

    #[zbus(name = "GetRetentionPreviewStatus")]
    fn get_retention_preview_status(&self, preview_id: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let st = guard.get_retention_preview_status(preview_id)?;
        serde_json::to_string(&st).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "StartBackupDryRun")]
    async fn start_backup_dry_run(
        &self,
        backup_id: &str,
        excludes_json: &str,
    ) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = backup_id.to_string();
        let ex = excludes_json.to_string();
        run_blocking_task(move || start_backup_dry_run_async(&core, &id, &ex))
            .await
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "CancelBackupDryRun")]
    fn cancel_backup_dry_run(&self, preview_id: &str) -> Result<(), KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        Ok(guard.cancel_backup_dry_run(preview_id)?)
    }

    #[zbus(name = "GetBackupDryRunStatus")]
    fn get_backup_dry_run_status(&self, preview_id: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let st = guard.get_backup_dry_run_status(preview_id)?;
        serde_json::to_string(&st).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "FormatExcludeRuleForItem")]
    fn format_exclude_rule_for_item(&self, item_path: &str) -> Result<String, KesticDbusError> {
        let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let preview = guard.format_exclude_rule_for_item(item_path)?;
        serde_json::to_string(&preview).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "AddExcludeRuleForItem")]
    async fn add_exclude_rule_for_item(
        &self,
        backup_id: &str,
        item_path: &str,
    ) -> Result<String, KesticDbusError> {
        let (preview, conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let preview = guard.add_exclude_rule_for_item(backup_id, item_path)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (preview, guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        serde_json::to_string(&preview).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "RunDestinationMaintenance")]
    async fn run_destination_maintenance(
        &self,
        destination_id: &str,
    ) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = destination_id.to_string();
        run_blocking_task(move || run_destination_maintenance_async(&core, &id))
            .await
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "PauseGlobal")]
    async fn pause_global(&self, duration_json: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.pause_global(duration_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "ResumeGlobal")]
    async fn resume_global(&self) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.resume_global()?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        let core = Arc::clone(&self.core);
        run_blocking_task(move || drain_queued_backups_async(&core)).await;
        Ok(())
    }

    #[zbus(name = "PauseBackup")]
    async fn pause_backup(
        &self,
        backup_id: &str,
        duration_json: &str,
    ) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.pause_backup(backup_id, duration_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "ResumeBackup")]
    async fn resume_backup(&self, backup_id: &str) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.resume_backup(backup_id)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        let core = Arc::clone(&self.core);
        run_blocking_task(move || drain_queued_backups_async(&core)).await;
        Ok(())
    }

    #[zbus(name = "StartBackupOverride")]
    async fn start_backup_override(&self, backup_id: &str) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = backup_id.to_string();
        run_blocking_task(move || start_backup_override_async(&core, &id))
            .await
            .map_err(KesticDbusError::from)
    }

    #[zbus(name = "UnlockDestination")]
    async fn unlock_destination(&self, destination_id: &str) -> Result<u32, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = destination_id.to_string();
        let count = run_blocking_task(move || unlock_destination_async(&core, &id))
            .await
            .map_err(KesticDbusError::from)?;

        let (conn_opt, state_dto, tray_state) = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(count)
    }

    #[zbus(name = "PreviewDestinationMaintenance")]
    async fn preview_destination_maintenance(
        &self,
        destination_id: &str,
    ) -> Result<String, KesticDbusError> {
        let core = Arc::clone(&self.core);
        let id = destination_id.to_string();
        let preview =
            run_blocking_task(move || preview_destination_maintenance_unlocked(&core, &id))
                .await
                .map_err(KesticDbusError::from)?;
        serde_json::to_string(&preview).map_err(|e| KesticDbusError::Internal(e.to_string()))
    }

    #[zbus(name = "UpdateDestinationMaintenance")]
    async fn update_destination_maintenance(
        &self,
        destination_id: &str,
        maintenance_json: &str,
    ) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.update_destination_maintenance(destination_id, maintenance_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "UpdateSettings")]
    async fn update_settings(
        &self,
        default_overdue_threshold: u32,
        notifications_json: &str,
    ) -> Result<(), KesticDbusError> {
        let (conn_opt, state_dto, tray_state) = {
            let mut guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.update_settings(default_overdue_threshold, notifications_json)?;
            let (state_dto, tray_state) = guard.build_state_and_tray();
            (guard.conn.clone(), state_dto, tray_state)
        };
        if let Some(conn) = &conn_opt {
            self.sync_tray_and_emit(conn, &state_dto, tray_state).await;
        }
        Ok(())
    }

    #[zbus(name = "ListKnownNetworks")]
    async fn list_known_networks(&self) -> Result<String, KesticDbusError> {
        let monitor_bus = {
            let guard = self.core.lock().unwrap_or_else(|p| p.into_inner());
            guard.monitor_bus.clone()
        };
        let networks = run_blocking_task(move || list_known_networks(&monitor_bus)).await;
        serde_json::to_string(&networks).map_err(|e| KesticDbusError::Internal(e.to_string()))
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

    #[zbus(signal, name = "QuitConfirmationRequested")]
    async fn quit_confirmation_requested(
        ctxt: &SignalEmitter<'_>,
        has_running_run: bool,
        running_backup_name: &str,
    ) -> zbus::Result<()>;
}

pub(crate) fn start_dbus_server(
    bus_name: &str,
    core: Arc<Mutex<DaemonCore>>,
) -> Result<InstanceAcquire, String> {
    let dbus_service = KesticDbusV1Service::new(core);
    SingleInstanceLock::acquire_with_interface(bus_name, dbus_service)
        .map_err(|err| format!("failed to acquire D-Bus name `{bus_name}`: {err}"))
}

async fn run_blocking_task<T, F>(f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let state: Arc<Mutex<(Option<T>, Option<std::task::Waker>)>> =
        Arc::new(Mutex::new((None, None)));
    let state_bg = Arc::clone(&state);
    thread::spawn(move || {
        let value = f();
        let waker = {
            let mut guard = state_bg.lock().unwrap_or_else(|p| p.into_inner());
            guard.0 = Some(value);
            guard.1.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    });
    std::future::poll_fn(move |cx| {
        let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(val) = guard.0.take() {
            std::task::Poll::Ready(val)
        } else {
            guard.1 = Some(cx.waker().clone());
            std::task::Poll::Pending
        }
    })
    .await
}

async fn emit_status_changed_async(conn: &Connection, state_dto: &DaemonStateDto) {
    if let Ok(json) = serde_json::to_string(&state_dto.overall)
        && let Ok(emitter) = SignalEmitter::new(conn, KESTIC_OBJECT_PATH)
    {
        let _ = KesticDbusV1Service::status_changed(&emitter, &json).await;
    }
}

pub(crate) fn emit_status_changed(conn: &Connection, state_dto: &DaemonStateDto) {
    if let Ok(json) = serde_json::to_string(&state_dto.overall) {
        let conn = conn.clone();
        thread::spawn(move || {
            let _ = zbus::block_on(async move {
                let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
                KesticDbusV1Service::status_changed(&emitter, &json).await
            });
        });
    }
}

pub(crate) fn emit_run_updated(conn: &Connection, run: &Run) {
    if let Ok(json) = serde_json::to_string(run) {
        let conn = conn.clone();
        thread::spawn(move || {
            let _ = zbus::block_on(async move {
                let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
                KesticDbusV1Service::run_updated(&emitter, &json).await
            });
        });
    }
}

pub(crate) fn emit_run_progress(conn: &Connection, progress: &ActiveRunProgressDto) {
    if let Ok(json) = serde_json::to_string(progress) {
        let conn = conn.clone();
        thread::spawn(move || {
            let _ = zbus::block_on(async move {
                let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
                KesticDbusV1Service::run_progress(&emitter, &json).await
            });
        });
    }
}

pub(crate) fn emit_manual_run_blocked(
    conn: &Connection,
    backup_id: &BackupId,
    reason: BlockedReason,
) {
    let conn = conn.clone();
    let bid = backup_id.as_str().to_string();
    let reason = reason.as_str();
    thread::spawn(move || {
        let _ = zbus::block_on(async move {
            let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
            KesticDbusV1Service::manual_run_blocked(&emitter, &bid, reason).await
        });
    });
}

pub(crate) fn emit_show_window_requested(conn: &Connection) {
    let conn = conn.clone();
    thread::spawn(move || {
        let _ = zbus::block_on(async move {
            let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
            KesticDbusV1Service::show_window_requested(&emitter).await
        });
    });
}

pub(crate) fn emit_quit_confirmation_requested(
    conn: &Connection,
    has_running_run: bool,
    running_backup_name: &str,
) {
    let conn = conn.clone();
    let name = running_backup_name.to_string();
    thread::spawn(move || {
        let _ = zbus::block_on(async move {
            let emitter = SignalEmitter::new(&conn, KESTIC_OBJECT_PATH)?;
            KesticDbusV1Service::quit_confirmation_requested(&emitter, has_running_run, &name).await
        });
    });
}
