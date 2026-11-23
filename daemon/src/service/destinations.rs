// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination CRUD, connection testing, password updates and repository unlock.

use super::error::{
    KesticServiceErr, map_config_err, map_destination_err, map_provider_err, map_secret_err,
    parse_setup_mode,
};
use super::state::DaemonCore;
use crate::dbus_api::{ConnectionTestResultDto, DaemonStateDto};
use crate::tray::KesticTrayState;
use kestic_core::clock::Clock;
use kestic_core::config::ConfigError;
use kestic_core::destination::{
    DEFAULT_CONNECT_TIMEOUT, DestinationError, DestinationSetupMode, preflight_check_destination,
    remove_destination_secrets, setup_destination, verify_and_update_stored_password,
};
use kestic_core::model::{
    Destination, DestinationId, DestinationKind, MaintenanceSchedule, ResticProvider, RunId,
};
use kestic_core::provider::resolve_and_check;
use kestic_core::restic::{ResticRuntime, SECRET_KIND_REPO, SECRET_KIND_REST};
use kestic_core::secrets::{InMemorySecretStore, SecretError, SecretStore, SecretValue};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::ConnectionTestCode;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use zbus::Connection;

impl DaemonCore {
    /// A destination a restore starts from, waits for or runs from stays as it is until the
    /// restore has ended.
    fn refuse_while_restoring(&self, id: &DestinationId) -> Result<(), KesticServiceErr> {
        if self.has_restore_on(id) {
            return Err(KesticServiceErr::RestoreRunning(format!(
                "a restore from destination {id} runs or waits"
            )));
        }
        Ok(())
    }

    pub fn remove_destination(&mut self, dest_id: &str) -> Result<(), KesticServiceErr> {
        let id = DestinationId::parse(dest_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        self.refuse_while_restoring(&id)?;
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
        let old = self.config.destinations[idx].maintenance.clone();
        let mut next = self.config.clone();
        next.destinations[idx].maintenance = maintenance.clone();
        self.persist_config(next)?;
        self.ensure_baselines();
        let mut ds = self.load_destination_maintenance_state(&d_id);
        kestic_core::scheduler::clear_maintenance_backoff(&mut ds);
        kestic_core::scheduler::anchor_changed_maintenance_schedules(
            &mut ds,
            &old,
            &maintenance,
            self.clock.now_utc(),
        );
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
        let nonce = RunId::generate().map_or_else(|_| "0".to_string(), |r| r.as_str().to_string());
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

/// The secret `entered` in the form, else the one stored for `dest` under `kind`.
fn stored_secret(
    secret_store: &dyn SecretStore,
    dest: &Destination,
    kind: &str,
    entered: &str,
) -> Result<Option<SecretValue>, SecretError> {
    if !entered.is_empty() {
        return Ok(Some(SecretValue::from_str_secret(entered)));
    }
    secret_store.get_secret(dest.id.as_str(), kind)
}

fn secret_error_result(err: SecretError) -> ConnectionTestResultDto {
    match err {
        SecretError::LockedOrPromptRequired(msg) => {
            test_failure(ConnectionTestCode::SecretStoreLocked, msg)
        }
        err => test_failure(ConnectionTestCode::Internal, err.to_string()),
    }
}

fn resolve_stored_repo_password(
    dest: &Destination,
    is_existing: bool,
    repo_password: &str,
    secret_store: &dyn SecretStore,
) -> Result<Option<SecretValue>, ConnectionTestResultDto> {
    if repo_password.is_empty() && !is_existing {
        return Ok(None);
    }
    match stored_secret(secret_store, dest, SECRET_KIND_REPO, repo_password) {
        Ok(None) => Err(test_failure(
            ConnectionTestCode::MissingPassword,
            String::new(),
        )),
        Ok(pw) => Ok(pw),
        Err(err) => Err(secret_error_result(err)),
    }
}

fn resolve_stored_rest_password(
    dest: &Destination,
    is_existing: bool,
    rest_password: &str,
    secret_store: &dyn SecretStore,
) -> Result<Option<SecretValue>, ConnectionTestResultDto> {
    if rest_password.is_empty() && !(is_existing && dest.kind.uses_rest_credentials()) {
        return Ok(None);
    }
    stored_secret(secret_store, dest, SECRET_KIND_REST, rest_password).map_err(secret_error_result)
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

    let deps = {
        let guard = shared.lock_unpoisoned();
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
        guard.restic_deps()
    };

    let resolved = resolve_and_check(&deps.provider).map_err(map_provider_err)?;
    let repo_pw = SecretValue::from_str_secret(repo_password);
    let rest_pw = (!rest_password.is_empty()).then(|| SecretValue::from_str_secret(rest_password));
    setup_destination(
        &dest,
        setup_mode,
        &repo_pw,
        rest_pw.as_ref(),
        &deps.runtime(&resolved),
    )
    .map_err(map_destination_err)?;

    let mut guard = shared.lock_unpoisoned();
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

    let deps = {
        let guard = shared.lock_unpoisoned();
        if guard.config.destination(&updated.id).is_none() {
            return Err(KesticServiceErr::NotFound(updated.id.as_str().to_string()));
        }
        guard.refuse_while_restoring(&updated.id)?;
        if let Some(existing) = guard
            .config
            .find_duplicate_destination(&updated.kind, Some(&updated.id))
        {
            return Err(KesticServiceErr::DuplicateLocation(existing.name.clone()));
        }
        guard.restic_deps()
    };

    if verify_connection || matches!(setup_mode, DestinationSetupMode::InitNew) {
        let resolved = resolve_and_check(&deps.provider).map_err(map_provider_err)?;
        let effective_repo_pw = stored_secret(
            deps.secret_store.as_ref(),
            &updated,
            SECRET_KIND_REPO,
            repo_password,
        )
        .map_err(map_secret_err)?
        .ok_or_else(|| {
            KesticServiceErr::InvalidInput("missing stored repository password".to_string())
        })?;
        let effective_rest_pw = if rest_password.is_empty() && !updated.kind.uses_rest_credentials()
        {
            None
        } else {
            stored_secret(
                deps.secret_store.as_ref(),
                &updated,
                SECRET_KIND_REST,
                rest_password,
            )
            .map_err(map_secret_err)?
        };
        setup_destination(
            &updated,
            setup_mode,
            &effective_repo_pw,
            effective_rest_pw.as_ref(),
            &deps.runtime(&resolved),
        )
        .map_err(map_destination_err)?;
    }

    let mut guard = shared.lock_unpoisoned();
    let existing_idx = guard
        .config
        .destinations
        .iter()
        .position(|d| d.id == updated.id)
        .ok_or_else(|| KesticServiceErr::NotFound(updated.id.as_str().to_string()))?;
    guard.refuse_while_restoring(&updated.id)?;
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
    let (dest, deps) = {
        let guard = shared.lock_unpoisoned();
        let dest = guard
            .config
            .destination(&id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(dest_id.to_string()))?;
        (dest, guard.restic_deps())
    };
    let resolved = resolve_and_check(&deps.provider).map_err(map_provider_err)?;
    let candidate = SecretValue::from_str_secret(new_password);
    verify_and_update_stored_password(
        &dest,
        &candidate,
        deps.secret_store.as_ref(),
        &resolved,
        &deps.kestic_binary,
        deps.clock.as_ref(),
    )
    .map_err(map_destination_err)?;

    let guard = shared.lock_unpoisoned();
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

    let (is_existing, dup_name, deps) = {
        let guard = shared.lock_unpoisoned();
        let is_existing = guard.config.destination(&dest.id).is_some();
        let exclude_id = is_existing.then_some(&dest.id);
        let dup_name = guard
            .config
            .find_duplicate_destination(&dest.kind, exclude_id)
            .map(|d| d.name.clone());
        (is_existing, dup_name, guard.restic_deps())
    };

    test_destination_connection_impl(DestinationTestInputs {
        dest: &dest,
        mode,
        repo_password,
        rest_password,
        is_existing,
        dup_name: dup_name.as_deref(),
        provider: &deps.provider,
        kestic_binary: &deps.kestic_binary,
        secret_store: deps.secret_store.as_ref(),
        clock: deps.clock.as_ref(),
    })
}
