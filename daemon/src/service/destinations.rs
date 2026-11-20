// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination CRUD, connection testing, password updates, repository unlock, and snapshot listing.

use super::error::{
    KesticServiceErr, map_config_err, map_destination_err, map_provider_err, map_secret_err,
    parse_setup_mode,
};
use super::state::DaemonCore;
use crate::dbus_api::{
    BackupSnapshotsDto, ConnectionTestResultDto, DaemonStateDto, PreviousDestinationSnapshotsDto,
    SnapshotSummaryDto,
};
use crate::tray::KesticTrayState;
use kestic_core::backup::list_snapshots_for_backup;
use kestic_core::clock::Clock;
use kestic_core::config::ConfigError;
use kestic_core::destination::{
    DEFAULT_CONNECT_TIMEOUT, DestinationError, DestinationSetupMode, preflight_check_destination,
    remove_destination_secrets, setup_destination, verify_and_update_stored_password,
};
use kestic_core::model::{
    BackupId, Destination, DestinationId, DestinationKind, MaintenanceSchedule, ResticProvider,
    RunId,
};
use kestic_core::provider::resolve_and_check;
use kestic_core::restic::{ResticRuntime, SECRET_KIND_REPO, SECRET_KIND_REST};
use kestic_core::secrets::{InMemorySecretStore, SecretError, SecretStore, SecretValue};
use kestic_core::snapshot::ResticSnapshot;
use kestic_i18n::codes::ConnectionTestCode;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use zbus::Connection;

impl DaemonCore {
    pub fn add_destination(
        &mut self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> Result<String, KesticServiceErr> {
        let mut dest: Destination = serde_json::from_str(dest_json).map_err(|e| {
            KesticServiceErr::InvalidInput(format!("invalid destination JSON: {e}"))
        })?;
        if self.config.destination(&dest.id).is_some() {
            dest.id =
                DestinationId::generate_unique(self.config.destinations.iter().map(|d| &d.id))
                    .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
            dest.secrets = kestic_core::model::DestinationSecretRefs::for_destination(
                &dest.id,
                dest.kind.uses_rest_credentials(),
            );
        }
        dest.validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        if let Some(existing) = self.config.find_duplicate_destination(&dest.kind, None) {
            return Err(KesticServiceErr::DuplicateLocation(existing.name.clone()));
        }

        let setup_mode = parse_setup_mode(mode)?;
        if repo_password.is_empty() {
            return Err(KesticServiceErr::InvalidInput(
                "repository password must not be empty".to_string(),
            ));
        }

        let resolved = resolve_and_check(&self.config.restic_provider).map_err(map_provider_err)?;
        let repo_pw = SecretValue::from_str_secret(repo_password);
        let rest_pw =
            (!rest_password.is_empty()).then(|| SecretValue::from_str_secret(rest_password));

        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &self.kestic_binary,
            secret_store: self.secret_store.as_ref(),
            clock: self.clock.as_ref(),
            cache_dir: None,
        };
        setup_destination(&dest, setup_mode, &repo_pw, rest_pw.as_ref(), &runtime)
            .map_err(map_destination_err)?;

        let new_id = dest.id.as_str().to_string();
        let mut next = self.config.clone();
        next.destinations.push(dest);
        self.persist_config(next)?;
        self.ensure_baselines();
        Ok(new_id)
    }

    pub fn test_destination_connection(
        &self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
    ) -> ConnectionTestResultDto {
        let dest = match parse_and_validate_dest(dest_json) {
            Ok(d) => d,
            Err(dto) => return dto,
        };
        let is_existing = self.config.destination(&dest.id).is_some();
        let exclude_id = is_existing.then_some(&dest.id);
        let dup_name = self
            .config
            .find_duplicate_destination(&dest.kind, exclude_id)
            .map(|d| d.name.as_str());

        test_destination_connection_impl(DestinationTestInputs {
            dest: &dest,
            mode,
            repo_password,
            rest_password,
            is_existing,
            dup_name,
            provider: &self.config.restic_provider,
            kestic_binary: &self.kestic_binary,
            secret_store: self.secret_store.as_ref(),
            clock: self.clock.as_ref(),
        })
    }

    pub fn update_destination(
        &mut self,
        dest_json: &str,
        mode: &str,
        repo_password: &str,
        rest_password: &str,
        verify_connection: bool,
    ) -> Result<(), KesticServiceErr> {
        let updated: Destination = serde_json::from_str(dest_json).map_err(|e| {
            KesticServiceErr::InvalidInput(format!("invalid destination JSON: {e}"))
        })?;
        updated
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

        let existing_idx = self
            .config
            .destinations
            .iter()
            .position(|d| d.id == updated.id)
            .ok_or_else(|| KesticServiceErr::NotFound(updated.id.as_str().to_string()))?;

        let setup_mode = parse_setup_mode(mode)?;

        if let Some(existing) = self
            .config
            .find_duplicate_destination(&updated.kind, Some(&updated.id))
        {
            return Err(KesticServiceErr::DuplicateLocation(existing.name.clone()));
        }

        if verify_connection || matches!(setup_mode, DestinationSetupMode::InitNew) {
            let resolved =
                resolve_and_check(&self.config.restic_provider).map_err(map_provider_err)?;
            let effective_repo_pw = if !repo_password.is_empty() {
                SecretValue::from_str_secret(repo_password)
            } else {
                self.secret_store
                    .get_secret(updated.id.as_str(), SECRET_KIND_REPO)
                    .map_err(map_secret_err)?
                    .ok_or_else(|| {
                        KesticServiceErr::InvalidInput(
                            "missing stored repository password".to_string(),
                        )
                    })?
            };

            let effective_rest_pw = if !rest_password.is_empty() {
                Some(SecretValue::from_str_secret(rest_password))
            } else if updated.kind.uses_rest_credentials() {
                self.secret_store
                    .get_secret(updated.id.as_str(), SECRET_KIND_REST)
                    .map_err(map_secret_err)?
            } else {
                None
            };

            // AC-M1-19-6: preflight + setup (`init` or `connect`) must pass before mutating secrets or config.
            let runtime = ResticRuntime {
                resolved: &resolved,
                kestic_binary: &self.kestic_binary,
                secret_store: self.secret_store.as_ref(),
                clock: self.clock.as_ref(),
                cache_dir: None,
            };
            setup_destination(
                &updated,
                setup_mode,
                &effective_repo_pw,
                effective_rest_pw.as_ref(),
                &runtime,
            )
            .map_err(map_destination_err)?;
        }

        let mut next = self.config.clone();
        next.destinations[existing_idx] = updated;
        self.persist_config(next)?;
        Ok(())
    }

    pub fn update_destination_password(
        &mut self,
        dest_id: &str,
        new_password: &str,
    ) -> Result<(), KesticServiceErr> {
        let id = DestinationId::parse(dest_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let dest = self
            .config
            .destination(&id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(dest_id.to_string()))?;
        if new_password.is_empty() {
            return Err(KesticServiceErr::InvalidInput(
                "password must not be empty".to_string(),
            ));
        }

        let resolved = resolve_and_check(&self.config.restic_provider).map_err(map_provider_err)?;
        let candidate = SecretValue::from_str_secret(new_password);
        verify_and_update_stored_password(
            &dest,
            &candidate,
            self.secret_store.as_ref(),
            &resolved,
            &self.kestic_binary,
            self.clock.as_ref(),
        )
        .map_err(map_destination_err)?;
        Ok(())
    }

    pub fn remove_destination(&mut self, dest_id: &str) -> Result<(), KesticServiceErr> {
        let id = DestinationId::parse(dest_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let mut next = self.config.clone();
        let removed = next.remove_destination(&id).map_err(|err| match err {
            ConfigError::Validation(msg) if msg.contains("still referenced by") => {
                KesticServiceErr::InUse(msg)
            }
            ConfigError::Validation(msg) if msg.contains("not found") => {
                KesticServiceErr::NotFound(msg)
            }
            other => map_config_err(other),
        })?;

        self.persist_config(next)?;
        remove_destination_secrets(&removed, self.secret_store.as_ref()).map_err(map_secret_err)?;
        self.dest_availability.remove(&id);
        self.dest_stale_locks.remove(&id);
        let _ = self.clear_destination_state_cached(&id);
        Ok(())
    }

    pub fn update_destination_maintenance(
        &mut self,
        destination_id: &str,
        maintenance_json: &str,
    ) -> Result<(), KesticServiceErr> {
        let d_id = DestinationId::parse(destination_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let maintenance: MaintenanceSchedule =
            serde_json::from_str(maintenance_json).map_err(|e| {
                KesticServiceErr::InvalidInput(format!("invalid maintenance JSON: {e}"))
            })?;
        maintenance
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let idx = self
            .config
            .destinations
            .iter()
            .position(|d| d.id == d_id)
            .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;
        let mut next = self.config.clone();
        next.destinations[idx].maintenance = maintenance;
        self.persist_config(next)?;
        self.ensure_baselines();
        let mut ds = self.load_destination_maintenance_state(&d_id);
        ds.maintenance_backoff_attempt = 0;
        ds.maintenance_backoff_until = None;
        ds.last_maintenance_failure_at = None;
        let _ = self.store_destination_maintenance_state(&d_id, &ds);
        Ok(())
    }
}

/// Failed connection test; `detail` is raw error text or the parameter of the code's text.
fn test_failure(code: ConnectionTestCode, detail: String) -> ConnectionTestResultDto {
    ConnectionTestResultDto {
        ok: false,
        error_kind: Some(code),
        detail,
    }
}

fn test_ok() -> ConnectionTestResultDto {
    ConnectionTestResultDto {
        ok: true,
        error_kind: None,
        detail: String::new(),
    }
}

fn parse_and_validate_dest(dest_json: &str) -> Result<Destination, ConnectionTestResultDto> {
    let dest: Destination = serde_json::from_str(dest_json)
        .map_err(|e| test_failure(ConnectionTestCode::InvalidInput, e.to_string()))?;
    dest.validate()
        .map_err(|e| test_failure(ConnectionTestCode::InvalidInput, e.to_string()))?;
    Ok(dest)
}

struct DestinationTestInputs<'a> {
    dest: &'a Destination,
    mode: &'a str,
    repo_password: &'a str,
    rest_password: &'a str,
    is_existing: bool,
    dup_name: Option<&'a str>,
    provider: &'a ResticProvider,
    kestic_binary: &'a Path,
    secret_store: &'a dyn SecretStore,
    clock: &'a dyn Clock,
}

fn test_destination_connection_impl(inputs: DestinationTestInputs<'_>) -> ConnectionTestResultDto {
    if let Some(existing_name) = inputs.dup_name {
        return test_failure(
            ConnectionTestCode::DuplicateLocation,
            existing_name.to_string(),
        );
    }

    if let Err(err) = preflight_check_destination(inputs.dest, DEFAULT_CONNECT_TIMEOUT) {
        return test_failure(ConnectionTestCode::DestinationUnreachable, err.to_string());
    }

    let setup_mode = match parse_setup_mode(inputs.mode) {
        Ok(m) => m,
        Err(e) => {
            return test_failure(ConnectionTestCode::InvalidInput, e.to_string());
        }
    };

    if matches!(setup_mode, DestinationSetupMode::InitNew) {
        return check_init_new_destination(
            inputs.dest,
            inputs.is_existing,
            inputs.repo_password,
            inputs.secret_store,
        );
    }

    check_connect_existing_destination(&inputs)
}

fn check_init_new_destination(
    dest: &Destination,
    is_existing: bool,
    repo_password: &str,
    secret_store: &dyn SecretStore,
) -> ConnectionTestResultDto {
    if let DestinationKind::Local { path } = &dest.kind {
        if path.join("config").exists() {
            return test_failure(ConnectionTestCode::RepositoryExists, String::new());
        }
        let nonce = RunId::generate()
            .map(|r| r.as_str().to_string())
            .unwrap_or_else(|_| "0".to_string());
        let probe_path = path.join(format!(".kestic-write-probe-{nonce}"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe_path)
        {
            Ok(f) => {
                drop(f);
                let _ = fs::remove_file(&probe_path);
            }
            Err(err) => {
                return test_failure(ConnectionTestCode::NotWritable, err.to_string());
            }
        }
    }
    if is_existing
        && repo_password.is_empty()
        && let Err(dto) = resolve_stored_repo_password(dest, true, repo_password, secret_store)
    {
        return dto;
    }
    test_ok()
}

fn check_connect_existing_destination(
    inputs: &DestinationTestInputs<'_>,
) -> ConnectionTestResultDto {
    let effective_repo_pw = match resolve_stored_repo_password(
        inputs.dest,
        inputs.is_existing,
        inputs.repo_password,
        inputs.secret_store,
    ) {
        Ok(pw) => pw,
        Err(dto) => return dto,
    };

    if let Some(repo_pw) = effective_repo_pw {
        let effective_rest_pw = match resolve_stored_rest_password(
            inputs.dest,
            inputs.is_existing,
            inputs.rest_password,
            inputs.secret_store,
        ) {
            Ok(pw) => pw,
            Err(dto) => return dto,
        };

        let resolved = match resolve_and_check(inputs.provider) {
            Ok(r) => r,
            Err(e) => {
                return test_failure(ConnectionTestCode::ResticUnavailable, e.to_string());
            }
        };
        let temp_store = InMemorySecretStore::new();
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: inputs.kestic_binary,
            secret_store: &temp_store,
            clock: inputs.clock,
            cache_dir: None,
        };
        if let Err(err) = setup_destination(
            inputs.dest,
            DestinationSetupMode::ConnectExisting,
            &repo_pw,
            effective_rest_pw.as_ref(),
            &runtime,
        ) {
            let kind = match err {
                DestinationError::WrongPassword => ConnectionTestCode::WrongPassword,
                DestinationError::AuthFailed(_) => ConnectionTestCode::AuthFailed,
                DestinationError::RepositoryMissing(_) => ConnectionTestCode::RepositoryMissing,
                DestinationError::Unreachable(_) => ConnectionTestCode::DestinationUnreachable,
                _ => ConnectionTestCode::CommandFailed,
            };
            return test_failure(kind, err.to_string());
        }
    }

    test_ok()
}

fn resolve_stored_repo_password(
    dest: &Destination,
    is_existing: bool,
    repo_password: &str,
    secret_store: &dyn SecretStore,
) -> Result<Option<SecretValue>, ConnectionTestResultDto> {
    if !repo_password.is_empty() {
        return Ok(Some(SecretValue::from_str_secret(repo_password)));
    }
    if !is_existing {
        return Ok(None);
    }
    match secret_store.get_secret(dest.id.as_str(), SECRET_KIND_REPO) {
        Ok(Some(pw)) => Ok(Some(pw)),
        Ok(None) => Err(test_failure(
            ConnectionTestCode::MissingPassword,
            String::new(),
        )),
        Err(SecretError::LockedOrPromptRequired(msg)) => {
            Err(test_failure(ConnectionTestCode::SecretStoreLocked, msg))
        }
        Err(err) => Err(test_failure(ConnectionTestCode::Internal, err.to_string())),
    }
}

fn resolve_stored_rest_password(
    dest: &Destination,
    is_existing: bool,
    rest_password: &str,
    secret_store: &dyn SecretStore,
) -> Result<Option<SecretValue>, ConnectionTestResultDto> {
    if !rest_password.is_empty() {
        return Ok(Some(SecretValue::from_str_secret(rest_password)));
    }
    if !is_existing || !dest.kind.uses_rest_credentials() {
        return Ok(None);
    }
    match secret_store.get_secret(dest.id.as_str(), SECRET_KIND_REST) {
        Ok(pw) => Ok(pw),
        Err(SecretError::LockedOrPromptRequired(msg)) => {
            Err(test_failure(ConnectionTestCode::SecretStoreLocked, msg))
        }
        Err(err) => Err(test_failure(ConnectionTestCode::Internal, err.to_string())),
    }
}

fn collect_backup_snapshots(
    b_id: &BackupId,
    current_dest: &Destination,
    other_dests: &[&Destination],
    resolved: &kestic_core::provider::ResolvedRestic,
    kestic_binary: &Path,
    secret_store: &dyn SecretStore,
    clock: &dyn Clock,
) -> BackupSnapshotsDto {
    let mut current_error = None;
    let current_snapshots = match list_snapshots_for_backup(
        b_id,
        current_dest,
        resolved,
        kestic_binary,
        secret_store,
        clock,
        &[],
    ) {
        Ok(snaps) => snaps.into_iter().map(snapshot_to_dto).collect(),
        Err(err) => {
            current_error = Some(err.to_string());
            Vec::new()
        }
    };

    // AC-M1-22-4: also scan any other configured destinations for earlier `kestic:<backup-id>` snapshots.
    let mut previous_destinations = Vec::new();
    for other_dest in other_dests {
        if let Ok(snaps) = list_snapshots_for_backup(
            b_id,
            other_dest,
            resolved,
            kestic_binary,
            secret_store,
            clock,
            &[],
        ) && !snaps.is_empty()
        {
            previous_destinations.push(PreviousDestinationSnapshotsDto {
                destination_id: other_dest.id.as_str().to_string(),
                destination_name: other_dest.name.clone(),
                snapshots: snaps.into_iter().map(snapshot_to_dto).collect(),
            });
        }
    }

    BackupSnapshotsDto {
        backup_id: b_id.as_str().to_string(),
        current_destination_id: current_dest.id.as_str().to_string(),
        current_destination_name: current_dest.name.clone(),
        current_snapshots,
        current_error,
        previous_destinations,
    }
}

fn snapshot_to_dto(s: ResticSnapshot) -> SnapshotSummaryDto {
    SnapshotSummaryDto {
        short_id: s.short_id.unwrap_or_else(|| s.id.chars().take(8).collect()),
        id: s.id,
        time: s.time,
        hostname: s.hostname.unwrap_or_default(),
        paths: s.paths,
        tags: s.tags,
        total_bytes_processed: s.summary.as_ref().and_then(|x| x.total_bytes_processed),
        total_files_processed: s.summary.as_ref().and_then(|x| x.total_files_processed),
    }
}

pub(crate) fn add_destination_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    dest_json: &str,
    mode: &str,
    repo_password: &str,
    rest_password: &str,
) -> Result<(String, Option<Connection>, DaemonStateDto, KesticTrayState), KesticServiceErr> {
    let mut dest: Destination = serde_json::from_str(dest_json)
        .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid destination JSON: {e}")))?;
    let setup_mode = parse_setup_mode(mode)?;
    if repo_password.is_empty() {
        return Err(KesticServiceErr::InvalidInput(
            "repository password must not be empty".to_string(),
        ));
    }

    let (provider, kestic_binary, secret_store, clock) = {
        let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        if guard.config.destination(&dest.id).is_some() {
            dest.id =
                DestinationId::generate_unique(guard.config.destinations.iter().map(|d| &d.id))
                    .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
            dest.secrets = kestic_core::model::DestinationSecretRefs::for_destination(
                &dest.id,
                dest.kind.uses_rest_credentials(),
            );
        }
        dest.validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        if let Some(existing) = guard.config.find_duplicate_destination(&dest.kind, None) {
            return Err(KesticServiceErr::DuplicateLocation(existing.name.clone()));
        }
        (
            guard.config.restic_provider.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
        )
    };

    let resolved = resolve_and_check(&provider).map_err(map_provider_err)?;
    let repo_pw = SecretValue::from_str_secret(repo_password);
    let rest_pw = (!rest_password.is_empty()).then(|| SecretValue::from_str_secret(rest_password));
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &kestic_binary,
        secret_store: secret_store.as_ref(),
        clock: clock.as_ref(),
        cache_dir: None,
    };
    setup_destination(&dest, setup_mode, &repo_pw, rest_pw.as_ref(), &runtime)
        .map_err(map_destination_err)?;

    let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(existing) = guard.config.find_duplicate_destination(&dest.kind, None) {
        return Err(KesticServiceErr::DuplicateLocation(existing.name.clone()));
    }
    let new_id = dest.id.as_str().to_string();
    let mut next = guard.config.clone();
    next.destinations.push(dest);
    guard.persist_config(next)?;
    guard.ensure_baselines();
    let (state_dto, tray_state) = guard.build_state_and_tray();
    Ok((new_id, guard.conn.clone(), state_dto, tray_state))
}

pub(crate) fn update_destination_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    dest_json: &str,
    mode: &str,
    repo_password: &str,
    rest_password: &str,
    verify_connection: bool,
) -> Result<(Option<Connection>, DaemonStateDto, KesticTrayState), KesticServiceErr> {
    let updated: Destination = serde_json::from_str(dest_json)
        .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid destination JSON: {e}")))?;
    updated
        .validate()
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    let setup_mode = parse_setup_mode(mode)?;

    let (provider, kestic_binary, secret_store, clock) = {
        let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        if guard.config.destination(&updated.id).is_none() {
            return Err(KesticServiceErr::NotFound(updated.id.as_str().to_string()));
        }
        if let Some(existing) = guard
            .config
            .find_duplicate_destination(&updated.kind, Some(&updated.id))
        {
            return Err(KesticServiceErr::DuplicateLocation(existing.name.clone()));
        }
        (
            guard.config.restic_provider.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
        )
    };

    if verify_connection || matches!(setup_mode, DestinationSetupMode::InitNew) {
        let resolved = resolve_and_check(&provider).map_err(map_provider_err)?;
        let effective_repo_pw = if !repo_password.is_empty() {
            SecretValue::from_str_secret(repo_password)
        } else {
            secret_store
                .get_secret(updated.id.as_str(), SECRET_KIND_REPO)
                .map_err(map_secret_err)?
                .ok_or_else(|| {
                    KesticServiceErr::InvalidInput("missing stored repository password".to_string())
                })?
        };
        let effective_rest_pw = if !rest_password.is_empty() {
            Some(SecretValue::from_str_secret(rest_password))
        } else if updated.kind.uses_rest_credentials() {
            secret_store
                .get_secret(updated.id.as_str(), SECRET_KIND_REST)
                .map_err(map_secret_err)?
        } else {
            None
        };
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &kestic_binary,
            secret_store: secret_store.as_ref(),
            clock: clock.as_ref(),
            cache_dir: None,
        };
        setup_destination(
            &updated,
            setup_mode,
            &effective_repo_pw,
            effective_rest_pw.as_ref(),
            &runtime,
        )
        .map_err(map_destination_err)?;
    }

    let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
    let existing_idx = guard
        .config
        .destinations
        .iter()
        .position(|d| d.id == updated.id)
        .ok_or_else(|| KesticServiceErr::NotFound(updated.id.as_str().to_string()))?;
    if let Some(existing) = guard
        .config
        .find_duplicate_destination(&updated.kind, Some(&updated.id))
    {
        return Err(KesticServiceErr::DuplicateLocation(existing.name.clone()));
    }
    let mut next = guard.config.clone();
    next.destinations[existing_idx] = updated;
    guard.persist_config(next)?;
    let (state_dto, tray_state) = guard.build_state_and_tray();
    Ok((guard.conn.clone(), state_dto, tray_state))
}

pub(crate) fn update_destination_password_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    dest_id: &str,
    new_password: &str,
) -> Result<(Option<Connection>, DaemonStateDto, KesticTrayState), KesticServiceErr> {
    let id =
        DestinationId::parse(dest_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    if new_password.is_empty() {
        return Err(KesticServiceErr::InvalidInput(
            "password must not be empty".to_string(),
        ));
    }
    let (dest, provider, kestic_binary, secret_store, clock) = {
        let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let dest = guard
            .config
            .destination(&id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(dest_id.to_string()))?;
        (
            dest,
            guard.config.restic_provider.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
        )
    };
    let resolved = resolve_and_check(&provider).map_err(map_provider_err)?;
    let candidate = SecretValue::from_str_secret(new_password);
    verify_and_update_stored_password(
        &dest,
        &candidate,
        secret_store.as_ref(),
        &resolved,
        &kestic_binary,
        clock.as_ref(),
    )
    .map_err(map_destination_err)?;

    let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
    let (state_dto, tray_state) = guard.build_state_and_tray();
    Ok((guard.conn.clone(), state_dto, tray_state))
}

pub(crate) fn test_destination_connection_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    dest_json: &str,
    mode: &str,
    repo_password: &str,
    rest_password: &str,
) -> ConnectionTestResultDto {
    let dest = match parse_and_validate_dest(dest_json) {
        Ok(d) => d,
        Err(dto) => return dto,
    };

    let (is_existing, dup_name, provider, kestic_binary, secret_store, clock) = {
        let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let is_existing = guard.config.destination(&dest.id).is_some();
        let exclude_id = is_existing.then_some(&dest.id);
        let dup_name = guard
            .config
            .find_duplicate_destination(&dest.kind, exclude_id)
            .map(|d| d.name.clone());
        (
            is_existing,
            dup_name,
            guard.config.restic_provider.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
        )
    };

    test_destination_connection_impl(DestinationTestInputs {
        dest: &dest,
        mode,
        repo_password,
        rest_password,
        is_existing,
        dup_name: dup_name.as_deref(),
        provider: &provider,
        kestic_binary: &kestic_binary,
        secret_store: secret_store.as_ref(),
        clock: clock.as_ref(),
    })
}

pub(crate) fn list_backup_snapshots_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<BackupSnapshotsDto, KesticServiceErr> {
    let b_id =
        BackupId::parse(backup_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    let (current_dest, other_dests, provider, kestic_binary, secret_store, clock) = {
        let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let backup = guard
            .config
            .backup(&b_id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let current_dest = guard
            .config
            .destination(&backup.destination_id)
            .cloned()
            .ok_or_else(|| {
                KesticServiceErr::NotFound(backup.destination_id.as_str().to_string())
            })?;
        let other_dests: Vec<Destination> = guard
            .config
            .destinations
            .iter()
            .filter(|d| d.id != current_dest.id)
            .cloned()
            .collect();
        (
            current_dest,
            other_dests,
            guard.config.restic_provider.clone(),
            guard.kestic_binary.clone(),
            Arc::clone(&guard.secret_store),
            Arc::clone(&guard.clock),
        )
    };

    let resolved = resolve_and_check(&provider).map_err(map_provider_err)?;
    let other_refs: Vec<&Destination> = other_dests.iter().collect();
    Ok(collect_backup_snapshots(
        &b_id,
        &current_dest,
        &other_refs,
        &resolved,
        &kestic_binary,
        secret_store.as_ref(),
        clock.as_ref(),
    ))
}
