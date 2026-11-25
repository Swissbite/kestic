// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination CRUD, connection testing, password updates and repository unlock.

use super::error::{
    KesticServiceErr, map_config_err, map_destination_err, map_provider_err, map_secret_err,
    parse_setup_mode,
};
use super::state::DaemonCore;
use crate::dbus_api::{BackendSecrets, ConnectionTestResultDto, DaemonStateDto};
use crate::tray::KesticTrayState;
use kestic_core::backend_credentials::BackendCredentials;
use kestic_core::clock::Clock;
use kestic_core::config::ConfigError;
use kestic_core::destination::{
    DEFAULT_CONNECT_TIMEOUT, DestinationError, DestinationSetupMode, RepositoryProbe,
    check_existing_repository, preflight_check_destination, probe_repository,
    remove_destination_secrets, setup_destination, verify_and_update_stored_password,
};
use kestic_core::generic::{GenericInputError, missing_program};
use kestic_core::known_hosts::HostKeyError;
use kestic_core::local_path::{NotALocalPath, connection_notes};
use kestic_core::model::{
    Destination, DestinationId, DestinationKind, MaintenanceSchedule, ModelError, ResticProvider,
    RunId,
};
use kestic_core::provider::resolve_and_check;
use kestic_core::repo_identity::connection_changed;
use kestic_core::restic::{ResticRuntime, SECRET_KIND_REPO};
use kestic_core::restic_gate::ResticBinaryGate;
use kestic_core::secrets::{InMemorySecretStore, SecretError, SecretStore, SecretValue};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::{ConnectionNoteCode, ConnectionTestCode};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use zbus::Connection;

impl DaemonCore {
    /// A destination a restore starts from, waits for or runs from stays as it is until the
    /// restore has ended.
    pub(super) fn refuse_while_restoring(
        &self,
        id: &DestinationId,
    ) -> Result<(), KesticServiceErr> {
        if self.has_restore_on(id) {
            return Err(KesticServiceErr::RestoreRunning(format!(
                "a restore from destination {id} runs or waits"
            )));
        }
        Ok(())
    }

    /// Removes the destination from the configuration and its secrets; returns it.
    pub fn remove_destination(&mut self, dest_id: &str) -> Result<Destination, KesticServiceErr> {
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
        Ok(removed)
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
pub(super) fn test_failure(code: ConnectionTestCode, detail: String) -> ConnectionTestResultDto {
    ConnectionTestResultDto {
        ok: false,
        error_kind: Some(code),
        detail,
        notes: Vec::new(),
        program: None,
        host_key: None,
    }
}

pub(super) fn test_ok() -> ConnectionTestResultDto {
    ConnectionTestResultDto {
        ok: true,
        error_kind: None,
        detail: String::new(),
        notes: Vec::new(),
        program: None,
        host_key: None,
    }
}

fn parse_and_validate_dest(dest_json: &str) -> Result<Destination, ConnectionTestResultDto> {
    let dest: Destination = serde_json::from_str(dest_json)
        .map_err(|e| test_failure(ConnectionTestCode::InvalidInput, e.to_string()))?;
    dest.validate().map_err(|e| match e {
        ModelError::NotALocalPath(NotALocalPath(address)) => {
            test_failure(ConnectionTestCode::NotALocalPath, address)
        }
        ModelError::Generic(GenericInputError::UnsupportedScheme(url)) => {
            test_failure(ConnectionTestCode::UnsupportedScheme, url)
        }
        ModelError::Generic(GenericInputError::ReservedVariable(name)) => {
            test_failure(ConnectionTestCode::ReservedVariable, name)
        }
        ModelError::Generic(GenericInputError::SecretInUrl(param)) => {
            test_failure(ConnectionTestCode::SecretInUrl, param)
        }
        other => test_failure(ConnectionTestCode::InvalidInput, other.to_string()),
    })?;
    Ok(dest)
}

struct DestinationTestInputs<'a> {
    dest: &'a Destination,
    mode: &'a str,
    repo_password: &'a str,
    backend: &'a BackendSecrets,
    is_existing: bool,
    dup_name: Option<&'a str>,
    provider: &'a ResticProvider,
    kestic_binary: &'a Path,
    secret_store: &'a dyn SecretStore,
    clock: &'a dyn Clock,
    /// The `PATH` the daemon starts restic with, for programs a backend needs (`rclone`).
    search_path: Option<&'a OsStr>,
    /// The test of an edit of a saved destination.
    edit: Option<EditedDestination<'a>>,
}

/// A saved destination whose edited connection is tested.
struct EditedDestination<'a> {
    shared: &'a Arc<Mutex<DaemonCore>>,
    saved: &'a Destination,
    deps: &'a super::state::ResticDeps,
}

fn test_destination_connection_impl(inputs: DestinationTestInputs<'_>) -> ConnectionTestResultDto {
    // Held for the whole call, so the app-managed restic is not replaced between its restic
    // processes.
    let _run_lease = ResticBinaryGate::global().read();
    if let Some(existing_name) = inputs.dup_name {
        return test_failure(
            ConnectionTestCode::DuplicateLocation,
            existing_name.to_string(),
        );
    }
    if let Some(program) = missing_program(&inputs.dest.kind, inputs.search_path) {
        return ConnectionTestResultDto {
            program: Some(program.executable().to_string()),
            ..test_failure(
                ConnectionTestCode::ProgramMissing,
                program.executable().to_string(),
            )
        };
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

    if let Some(edit) = &inputs.edit {
        // A new repository has a new ID: at another address it is another destination.
        if setup_mode == DestinationSetupMode::InitNew
            && connection_changed(&edit.saved.kind, &inputs.dest.kind)
        {
            return test_failure(ConnectionTestCode::OtherRepository, String::new());
        }
        if setup_mode == DestinationSetupMode::ConnectExisting {
            return super::destination_update::test_edited_connection(
                edit.shared,
                edit.saved,
                inputs.dest,
                inputs.backend,
                edit.deps,
            );
        }
    }

    if matches!(setup_mode, DestinationSetupMode::InitNew) {
        let local_result = check_init_new_destination(
            inputs.dest,
            inputs.is_existing,
            inputs.repo_password,
            inputs.secret_store,
        );
        if !local_result.ok || !inputs.dest.kind.probes_login_before_init() {
            return local_result;
        }
        return check_login_before_init(&inputs);
    }

    check_connect_existing_destination(&inputs)
}

/// The test of a new repository on a backend with a login (S3): checks the login and that no
/// repository is there yet, without creating anything.
fn check_login_before_init(inputs: &DestinationTestInputs<'_>) -> ConnectionTestResultDto {
    let backend = match resolve_backend_credentials(inputs) {
        Ok(creds) => creds,
        Err(dto) => return dto,
    };
    let resolved = match resolve_and_check(inputs.provider) {
        Ok(r) => r,
        Err(e) => return test_failure(ConnectionTestCode::ResticUnavailable, e.to_string()),
    };
    let temp_store = InMemorySecretStore::new();
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: inputs.kestic_binary,
        secret_store: &temp_store,
        clock: inputs.clock,
        cache_dir: None,
    };
    match probe_repository(inputs.dest, &backend, &runtime) {
        Ok(RepositoryProbe::Missing) => test_ok(),
        Ok(RepositoryProbe::BucketMissing) => ConnectionTestResultDto {
            notes: vec![ConnectionNoteCode::BucketMissing],
            ..test_ok()
        },
        Ok(RepositoryProbe::Exists) => {
            test_failure(ConnectionTestCode::RepositoryExists, String::new())
        }
        Err(err) => destination_error_result(err, inputs.clock),
    }
}

/// The first S3 key or secret variable that is missing; the REST password stays optional because a REST server may
/// accept a user name without one.
fn missing_login(creds: &BackendCredentials, kind: &DestinationKind) -> Option<String> {
    if matches!(
        kind,
        DestinationKind::S3 { .. } | DestinationKind::Generic { .. }
    ) {
        creds.first_missing(kind)
    } else {
        None
    }
}

/// The connection test result of a failed restic probe; an unknown SFTP host key is kept for
/// `TrustHostKey`.
pub(super) fn destination_error_result(
    err: DestinationError,
    clock: &dyn Clock,
) -> ConnectionTestResultDto {
    let kind = match err {
        DestinationError::HostKeyUnknown(key) => {
            return ConnectionTestResultDto {
                host_key: Some(Box::new(super::host_keys::offer_host_key(&key, clock))),
                ..test_failure(ConnectionTestCode::UnknownHostKey, key.name.clone())
            };
        }
        DestinationError::HostKeyChanged(file) => {
            return test_failure(
                ConnectionTestCode::HostKeyChanged,
                file.to_string_lossy().into_owned(),
            );
        }
        DestinationError::HostKeyCheck(HostKeyError::JumpHost(_)) => {
            ConnectionTestCode::JumpHostUnsupported
        }
        DestinationError::HostKeyCheck(HostKeyError::NoAnswer { .. }) => {
            ConnectionTestCode::NotResponding
        }
        DestinationError::SshKeyRejected(_) => ConnectionTestCode::SshKeyRejected,
        DestinationError::WrongPassword => ConnectionTestCode::WrongPassword,
        DestinationError::AuthFailed(_) => ConnectionTestCode::AuthFailed,
        DestinationError::RepositoryMissing(_) => ConnectionTestCode::RepositoryMissing,
        DestinationError::Unreachable(_) => ConnectionTestCode::DestinationUnreachable,
        DestinationError::NotResponding(_) => ConnectionTestCode::NotResponding,
        _ => ConnectionTestCode::CommandFailed,
    };
    test_failure(kind, err.to_string())
}

/// The backend credentials entered in the form, for an existing destination completed with the
/// saved ones; every credential the backend needs must be there.
fn resolve_backend_credentials(
    inputs: &DestinationTestInputs<'_>,
) -> Result<BackendCredentials, ConnectionTestResultDto> {
    let mut creds = inputs.backend.credentials();
    if inputs.is_existing {
        creds = creds
            .or_stored(inputs.dest, inputs.secret_store)
            .map_err(secret_error_result)?;
    }
    if let Some(kind) = missing_login(&creds, &inputs.dest.kind) {
        return Err(test_failure(
            ConnectionTestCode::InvalidInput,
            format!("missing backend credential `{kind}`"),
        ));
    }
    Ok(creds)
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

    // Without a repository password only a backend whose login can be checked on its own (S3)
    // runs restic; the others are checked when the password is entered.
    if effective_repo_pw.is_none() && !inputs.dest.kind.probes_login_before_init() {
        return test_ok();
    }
    let backend = match resolve_backend_credentials(inputs) {
        Ok(creds) => creds,
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
    match effective_repo_pw {
        Some(repo_pw) => {
            if let Err(err) = check_existing_repository(inputs.dest, &repo_pw, &backend, &runtime) {
                return destination_error_result(err, inputs.clock);
            }
        }
        None => match probe_repository(inputs.dest, &backend, &runtime) {
            Ok(RepositoryProbe::Exists) => {}
            Ok(RepositoryProbe::Missing | RepositoryProbe::BucketMissing) => {
                return test_failure(ConnectionTestCode::RepositoryMissing, String::new());
            }
            Err(err) => return destination_error_result(err, inputs.clock),
        },
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

pub(super) fn secret_error_result(err: SecretError) -> ConnectionTestResultDto {
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

/// Refuses a destination whose backend needs a program that is not in the daemon's `PATH`
/// (`rclone`): restic would fail with its own unclear error.
pub(super) fn refuse_missing_program(
    kind: &DestinationKind,
    core: &DaemonCore,
) -> Result<(), KesticServiceErr> {
    match missing_program(kind, core.program_search_path().as_deref()) {
        Some(program) => Err(KesticServiceErr::InvalidInput(format!(
            "the program `{}` is not installed",
            program.executable()
        ))),
        None => Ok(()),
    }
}

pub(crate) fn add_destination_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    dest_json: &str,
    mode: &str,
    repo_password: &str,
    backend_secrets_json: &str,
) -> Result<(String, Option<Connection>, DaemonStateDto, KesticTrayState), KesticServiceErr> {
    // Held for the whole call, so the app-managed restic is not replaced between its restic
    // processes.
    let mut run_lease = ResticBinaryGate::global().read();
    let mut dest: Destination = serde_json::from_str(dest_json)
        .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid destination JSON: {e}")))?;
    let setup_mode = parse_setup_mode(mode)?;
    if repo_password.is_empty() {
        return Err(KesticServiceErr::InvalidInput(
            "repository password must not be empty".to_string(),
        ));
    }
    let backend = BackendSecrets::parse(backend_secrets_json)
        .map_err(KesticServiceErr::InvalidInput)?
        .credentials();
    if let Some(kind) = missing_login(&backend, &dest.kind) {
        return Err(KesticServiceErr::InvalidInput(format!(
            "missing backend credential `{kind}`"
        )));
    }

    let deps = {
        let guard = shared.lock_unpoisoned();
        if guard.config.destination(&dest.id).is_some() {
            dest.id =
                DestinationId::generate_unique(guard.config.destinations.iter().map(|d| &d.id))
                    .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
            dest.secrets =
                kestic_core::model::DestinationSecretRefs::for_destination(&dest.id, &dest.kind);
        }
        dest.validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        refuse_missing_program(&dest.kind, &guard)?;
        if let Some(existing) = guard.config.find_duplicate_destination(&dest.kind, None) {
            return Err(KesticServiceErr::DuplicateLocation(existing.name.clone()));
        }
        guard.restic_deps()
    };

    let resolved = resolve_and_check(&deps.provider).map_err(map_provider_err)?;
    run_lease.bind(&resolved.binary);
    let repo_pw = SecretValue::from_str_secret(repo_password);
    let prepared = setup_destination(
        &dest,
        setup_mode,
        &repo_pw,
        &backend,
        &deps.runtime(&resolved),
    )
    .map_err(map_destination_err)?;

    let mut guard = shared.lock_unpoisoned();
    let saved = match guard.config.find_duplicate_destination(&dest.kind, None) {
        Some(existing) => Err(KesticServiceErr::DuplicateLocation(existing.name.clone())),
        None => {
            let mut next = guard.config.clone();
            next.destinations.push(dest.clone());
            guard.persist_config(next)
        }
    };
    if let Err(err) = saved {
        // The secrets were written for this new destination only.
        let _ = remove_destination_secrets(&dest, deps.secret_store.as_ref());
        return Err(err);
    }
    if let Some(id) = &prepared.repository_id {
        let _ = guard.history.set_repository_id(&dest.id, id);
    }
    let new_id = dest.id.as_str().to_string();
    guard.ensure_baselines();
    let (state_dto, tray_state) = guard.build_state_and_tray();
    Ok((new_id, guard.conn.clone(), state_dto, tray_state))
}

/// `RemoveDestination`: removes the destination, then, without holding the daemon state, the
/// lines of its SFTP server from Kestic's `known_hosts` when no other destination uses it (asking
/// ssh how it resolves the server may take a while).
pub(crate) fn remove_destination_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    dest_id: &str,
) -> Result<(Option<Connection>, DaemonStateDto, KesticTrayState), KesticServiceErr> {
    let (removed, remaining) = {
        let mut guard = shared.lock_unpoisoned();
        let removed = guard.remove_destination(dest_id)?;
        (removed, guard.config.destinations.clone())
    };
    super::host_keys::forget_unused_host(&removed, &remaining);
    let guard = shared.lock_unpoisoned();
    let (state_dto, tray_state) = guard.build_state_and_tray();
    Ok((guard.conn.clone(), state_dto, tray_state))
}

pub(crate) fn update_destination_password_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    dest_id: &str,
    new_password: &str,
) -> Result<(Option<Connection>, DaemonStateDto, KesticTrayState), KesticServiceErr> {
    // Held for the whole call, so the app-managed restic is not replaced between its restic
    // processes.
    let mut run_lease = ResticBinaryGate::global().read();
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
    run_lease.bind(&resolved.binary);
    let candidate = SecretValue::from_str_secret(new_password);
    let repository_id = verify_and_update_stored_password(
        &dest,
        &candidate,
        deps.secret_store.as_ref(),
        &resolved,
        &deps.kestic_binary,
        deps.clock.as_ref(),
    )
    .map_err(map_destination_err)?;
    if let Some(id) = &repository_id {
        super::destination_update::remember_repository_id(shared, &dest, id);
    }

    let guard = shared.lock_unpoisoned();
    let (state_dto, tray_state) = guard.build_state_and_tray();
    Ok((guard.conn.clone(), state_dto, tray_state))
}

pub(crate) fn test_destination_connection_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    dest_json: &str,
    mode: &str,
    repo_password: &str,
    backend_secrets_json: &str,
) -> ConnectionTestResultDto {
    let dest = match parse_and_validate_dest(dest_json) {
        Ok(d) => d,
        Err(dto) => return dto,
    };
    let backend = match BackendSecrets::parse(backend_secrets_json) {
        Ok(b) => b,
        Err(e) => return test_failure(ConnectionTestCode::InvalidInput, e),
    };

    let (saved, dup_name, deps, mount_inspector, search_path) = {
        let guard = shared.lock_unpoisoned();
        let saved = guard.config.destination(&dest.id).cloned();
        let is_existing = saved.is_some();
        let exclude_id = is_existing.then_some(&dest.id);
        let dup_name = guard
            .config
            .find_duplicate_destination(&dest.kind, exclude_id)
            .map(|d| d.name.clone());
        (
            saved,
            dup_name,
            guard.restic_deps(),
            Arc::clone(&guard.mount_inspector),
            guard.program_search_path(),
        )
    };

    let mut result = test_destination_connection_impl(DestinationTestInputs {
        dest: &dest,
        mode,
        repo_password,
        backend: &backend,
        is_existing: saved.is_some(),
        dup_name: dup_name.as_deref(),
        provider: &deps.provider,
        kestic_binary: &deps.kestic_binary,
        secret_store: deps.secret_store.as_ref(),
        clock: deps.clock.as_ref(),
        search_path: search_path.as_deref(),
        edit: saved.as_ref().map(|saved| EditedDestination {
            shared,
            saved,
            deps: &deps,
        }),
    });
    if result.ok {
        result
            .notes
            .extend(connection_notes(&dest, mount_inspector.as_ref()));
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A connection test whose restic did not answer within the limit reports «the server does
    /// not respond», the code the GUI shows with its action.
    #[test]
    fn restic_that_does_not_answer_is_reported_as_not_responding() {
        let result = destination_error_result(
            DestinationError::NotResponding("restic did not finish within 15 s".to_string()),
            &kestic_core::clock::SystemClock,
        );
        assert!(!result.ok, "the test failed");
        assert_eq!(result.error_kind, Some(ConnectionTestCode::NotResponding));
    }
}
