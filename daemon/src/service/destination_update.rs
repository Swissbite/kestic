// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Editing the connection of a saved destination: the connection test of the edit and
//! `UpdateDestination` read the repository ID at the edited connection and accept it only when it
//! is the destination's own repository (the stored ID, or for a destination without one the ID at
//! its saved address). Saved credentials go only to the server they were entered for.

use super::destinations::{destination_error_result, secret_error_result, test_failure, test_ok};
use super::error::{
    KesticServiceErr, map_destination_err, map_provider_err, map_secret_err, parse_setup_mode,
};
use super::host_keys::SftpSetupGuard;
use super::state::{DaemonCore, ResticDeps};
use super::state_summary::StatePublication;
use crate::dbus_api::{BackendSecrets, ConnectionTestResultDto, DestinationUpdateResultDto};
use kestic_core::backend_credentials::BackendCredentials;
use kestic_core::destination::{
    DestinationError, DestinationSetupMode, check_existing_repository, prepare_repository,
};
use kestic_core::model::{Destination, DestinationId, DestinationKind};
use kestic_core::provider::{ResticProviderError, resolve_and_check};
use kestic_core::repo_identity::{
    RepositoryId, RepositoryMatch, connection_changed, decide, same_server,
};
use kestic_core::restic::{ResticRuntime, SECRET_KIND_REPO};
use kestic_core::restic_gate::ResticBinaryGate;
use kestic_core::revision::revision_of;
use kestic_core::secrets::{InMemorySecretStore, SecretError, SecretValue};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::{ConnectionTestCode, DestinationUpdateOutcome};
use std::sync::{Arc, Mutex};

/// Why an edited connection could not be checked.
pub(super) enum EditError {
    /// A credential the backend needs is neither entered nor saved for this server: the variable
    /// restic reads it from.
    CredentialsRequired(String),
    MissingRepoPassword,
    NotWritable(String),
    Secret(SecretError),
    Restic(ResticProviderError),
    Destination(DestinationError),
}

impl EditError {
    pub(super) fn into_test_result(
        self,
        clock: &dyn kestic_core::clock::Clock,
    ) -> ConnectionTestResultDto {
        match self {
            Self::CredentialsRequired(name) => {
                test_failure(ConnectionTestCode::CredentialsRequired, name)
            }
            Self::MissingRepoPassword => {
                test_failure(ConnectionTestCode::MissingPassword, String::new())
            }
            Self::NotWritable(detail) => test_failure(ConnectionTestCode::NotWritable, detail),
            Self::Secret(err) => secret_error_result(err),
            Self::Restic(err) => {
                test_failure(ConnectionTestCode::ResticUnavailable, err.to_string())
            }
            Self::Destination(err) => destination_error_result(err, clock),
        }
    }

    fn into_service_err(self) -> KesticServiceErr {
        match self {
            Self::CredentialsRequired(name) => KesticServiceErr::InvalidInput(format!(
                "enter the backend credential `{name}` again for this server"
            )),
            Self::MissingRepoPassword => {
                KesticServiceErr::InvalidInput("missing stored repository password".to_string())
            }
            Self::NotWritable(detail) => {
                KesticServiceErr::InvalidInput(format!("the folder is not writable: {detail}"))
            }
            Self::Secret(err) => map_secret_err(err),
            Self::Restic(err) => map_provider_err(&err),
            Self::Destination(err) => map_destination_err(err),
        }
    }
}

impl From<SecretError> for EditError {
    fn from(err: SecretError) -> Self {
        Self::Secret(err)
    }
}

impl From<DestinationError> for EditError {
    fn from(err: DestinationError) -> Self {
        Self::Destination(err)
    }
}

/// An edit of a saved destination to check.
pub(super) struct EditCheck<'a> {
    pub(super) saved: &'a Destination,
    pub(super) edited: &'a Destination,
    /// Credentials entered in the form.
    pub(super) entered: BackendCredentials,
    pub(super) stored_id: Option<RepositoryId>,
    pub(super) deps: &'a ResticDeps,
}

/// A checked edit.
pub(super) enum EditVerdict {
    /// The destination's own repository: the credentials to save and the repository ID to store.
    Same {
        credentials: BackendCredentials,
        repository_id: RepositoryId,
    },
    /// Another repository, or none Kestic could compare with.
    Other,
}

/// The credentials for the edited connection: the entered ones, completed with the saved ones
/// only on the same server. Every credential the backend needs must be there; a REST password is
/// needed only when one was saved for the previous server.
fn edit_credentials(check: &EditCheck<'_>) -> Result<BackendCredentials, EditError> {
    let store = check.deps.secret_store.as_ref();
    let on_same_server = same_server(&check.saved.kind, &check.edited.kind);
    let credentials = if on_same_server {
        check.entered.clone().or_stored(check.edited, store)?
    } else {
        check.entered.clone()
    };
    let Some(missing) = credentials.first_missing(&check.edited.kind) else {
        return Ok(credentials);
    };
    let required = match check.edited.kind {
        DestinationKind::Rest { .. } => {
            !on_same_server && !BackendCredentials::load_stored(check.saved, store)?.is_empty()
        }
        _ => true,
    };
    if required {
        let name = check
            .edited
            .kind
            .backend_secret_slots()
            .into_iter()
            .find(|slot| slot.kind == missing)
            .map_or(missing, |slot| slot.env_name);
        return Err(EditError::CredentialsRequired(name));
    }
    Ok(credentials)
}

/// A local folder Kestic is to write the next backups to must be writable.
fn check_writable(kind: &DestinationKind) -> Result<(), EditError> {
    let DestinationKind::Local { path } = kind else {
        return Ok(());
    };
    kestic_core::local_path::probe_writable(path)
        .map_err(|err| EditError::NotWritable(err.to_string()))
}

/// Checks the edited connection: the login, the saved repository password and the repository
/// ID. Without a stored ID the saved address is read at the same time, so the check takes no
/// longer than one connection test.
pub(super) fn check_edited_connection(check: &EditCheck<'_>) -> Result<EditVerdict, EditError> {
    let credentials = edit_credentials(check)?;
    let changed = connection_changed(&check.saved.kind, &check.edited.kind);
    if changed {
        check_writable(&check.edited.kind)?;
    }
    let store = check.deps.secret_store.as_ref();
    let password = store
        .get_secret(check.saved.id.as_str(), SECRET_KIND_REPO)?
        .ok_or(EditError::MissingRepoPassword)?;
    let resolved = resolve_and_check(&check.deps.provider).map_err(EditError::Restic)?;
    let temp_store = InMemorySecretStore::new();
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &check.deps.kestic_binary,
        secret_store: &temp_store,
        clock: check.deps.clock.as_ref(),
        cache_dir: None,
    };
    let read_edited = || check_existing_repository(check.edited, &password, &credentials, &runtime);
    if !changed {
        // Another repository at the saved address (the folder was swapped) is not the
        // destination's own; only creating a new repository in place replaces the known ID.
        let repository_id = read_edited()?;
        return Ok(match &check.stored_id {
            Some(stored) if *stored != repository_id => EditVerdict::Other,
            _ => EditVerdict::Same {
                credentials,
                repository_id,
            },
        });
    }
    let (found, known) = match &check.stored_id {
        Some(stored) => (read_edited(), Some(stored.clone())),
        None => {
            let saved_credentials = BackendCredentials::load_stored(check.saved, store)?;
            std::thread::scope(|scope| {
                let at_saved = scope.spawn(|| {
                    let saved_store = InMemorySecretStore::new();
                    let saved_runtime = ResticRuntime {
                        resolved: &resolved,
                        kestic_binary: &check.deps.kestic_binary,
                        secret_store: &saved_store,
                        clock: check.deps.clock.as_ref(),
                        cache_dir: None,
                    };
                    check_existing_repository(
                        check.saved,
                        &password,
                        &saved_credentials,
                        &saved_runtime,
                    )
                    .ok()
                });
                let found = read_edited();
                (found, at_saved.join().ok().flatten())
            })
        }
    };
    let found = found?;
    Ok(match decide(known.as_ref(), &found) {
        RepositoryMatch::Same => EditVerdict::Same {
            credentials,
            repository_id: found,
        },
        RepositoryMatch::Other => EditVerdict::Other,
    })
}

/// The connection test of an edit of the saved destination `saved` in mode `connect`; the
/// repository ID is stored when the connection is unchanged.
pub(super) fn test_edited_connection(
    shared: &Arc<Mutex<DaemonCore>>,
    saved: &Destination,
    edited: &Destination,
    backend: &BackendSecrets,
    deps: &ResticDeps,
) -> ConnectionTestResultDto {
    let stored_id = shared
        .lock_unpoisoned()
        .history
        .repository_id(&saved.id)
        .ok()
        .flatten();
    let check = EditCheck {
        saved,
        edited,
        entered: backend.credentials(),
        stored_id: stored_id.clone(),
        deps,
    };
    match check_edited_connection(&check) {
        Ok(EditVerdict::Same { repository_id, .. }) => {
            // Same means: equal to the stored ID, or none was stored yet.
            if stored_id.is_none() {
                remember_repository_id(shared, saved, &repository_id);
            }
            test_ok()
        }
        Ok(EditVerdict::Other) => test_failure(ConnectionTestCode::OtherRepository, String::new()),
        Err(err) => err.into_test_result(deps.clock.as_ref()),
    }
}

/// Stores the repository ID of `dest` when it is still configured with that connection and no
/// ID is known yet; a known ID is replaced only by creating a new repository in place.
pub(super) fn remember_repository_id(
    shared: &Arc<Mutex<DaemonCore>>,
    dest: &Destination,
    id: &RepositoryId,
) {
    let guard = shared.lock_unpoisoned();
    let configured = guard
        .config
        .destination(&dest.id)
        .is_some_and(|current| current.kind == dest.kind);
    if configured && matches!(guard.history.repository_id(&dest.id), Ok(None)) {
        store_repository_id(&guard, dest, id);
    }
}

/// Writes the repository ID of `dest`. A failure is logged and does not undo the save it follows:
/// the configuration is already written, and without a stored ID the next edit reads the ID at
/// the saved address.
fn store_repository_id(guard: &DaemonCore, dest: &Destination, id: &RepositoryId) {
    if let Err(err) = guard.history.set_repository_id(&dest.id, id) {
        eprintln!(
            "kestic-daemon: failed to store the repository ID of destination {}: {err}",
            dest.id
        );
    }
}

type UpdateReply = (DestinationUpdateResultDto, StatePublication);

fn reply(guard: &DaemonCore, outcome: DestinationUpdateOutcome, revision: String) -> UpdateReply {
    (
        DestinationUpdateResultDto { outcome, revision },
        guard.state_publication(),
    )
}

/// What `UpdateDestination` saves after its check.
struct Accepted {
    credentials: BackendCredentials,
    repository_id: Option<RepositoryId>,
    /// `init` created a new repository at the unchanged address.
    new_repository: bool,
}

/// `UpdateDestination`: checks `revision` against the saved destination before any restic call,
/// checks every change of the connection or of the credentials (whatever `verify_connection`
/// says; it only decides about checking an otherwise unchanged connection), then checks the
/// revision again and saves the secrets and `config.toml` together under the daemon lock.
pub(crate) fn update_destination_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    dest_json: &str,
    mode: &str,
    repo_password: &str,
    backend_secrets_json: &str,
    verify_connection: bool,
    revision: &str,
) -> Result<UpdateReply, KesticServiceErr> {
    // Held for the whole call, so the app-managed restic is not replaced between its restic
    // processes.
    let _run_lease = ResticBinaryGate::global().read();
    let updated: Destination = serde_json::from_str(dest_json)
        .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid destination JSON: {e}")))?;
    let entered = BackendSecrets::parse(backend_secrets_json)
        .map_err(KesticServiceErr::InvalidInput)?
        .credentials();
    updated
        .validate_input()
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
    let setup_mode = parse_setup_mode(mode)?;

    let (saved, stored_id, deps, changed, _setup) = {
        let mut guard = shared.lock_unpoisoned();
        let saved = guard
            .config
            .destination(&updated.id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(updated.id.as_str().to_string()))?;
        refuse_stale_revision(&saved, revision)?;
        guard.refuse_while_restoring(&updated.id)?;
        guard.refuse_duplicate(&updated)?;
        let changed = connection_changed(&saved.kind, &updated.kind);
        // A new name alone needs no restic, so it can be saved while `ssh` or `rclone` is
        // missing.
        if changed
            || !entered.is_empty()
            || verify_connection
            || setup_mode == DestinationSetupMode::InitNew
        {
            super::destinations::refuse_missing_program(&updated.kind, &guard)?;
        }
        let stored_id = guard.history.repository_id(&saved.id).ok().flatten();
        // A new server's host key checked below stays in Kestic's `known_hosts` until the
        // change is saved, even when another destination on that server is removed meanwhile.
        let setup = changed.then(|| SftpSetupGuard::begin(shared, &mut guard, &updated));
        (saved, stored_id, guard.restic_deps(), changed, setup)
    };
    // Also a new name alone is refused, so a destination that would log in with credentials
    // found on the machine is not saved again unchanged. The password store is read only when
    // the entered values do not settle it.
    if updated
        .kind
        .missing_credential_variables(&entered)
        .is_some()
    {
        let with_saved = entered
            .clone()
            .or_stored(&updated, deps.secret_store.as_ref())
            .map_err(map_secret_err)?;
        super::destinations::refuse_missing_credentials(&updated, &with_saved)?;
    }

    let accepted = if setup_mode == DestinationSetupMode::InitNew {
        if changed {
            // A new repository has a new ID: at another address it is another destination.
            return other_repository(shared, &updated.id, revision);
        }
        init_in_place(&updated, repo_password, &entered, &deps)?
    } else if changed || !entered.is_empty() || verify_connection {
        let check = EditCheck {
            saved: &saved,
            edited: &updated,
            entered,
            stored_id,
            deps: &deps,
        };
        match check_edited_connection(&check).map_err(EditError::into_service_err)? {
            EditVerdict::Same {
                credentials,
                repository_id,
            } => Accepted {
                credentials,
                repository_id: Some(repository_id),
                new_repository: false,
            },
            EditVerdict::Other => return other_repository(shared, &updated.id, revision),
        }
    } else {
        Accepted {
            credentials: BackendCredentials::none(),
            repository_id: None,
            new_repository: false,
        }
    };

    let new_repository_id = accepted
        .new_repository
        .then(|| accepted.repository_id.clone());
    // Bound first: the lock taken for the save ends with this statement, before the error arm
    // takes it again.
    let save_result = save_update(
        &mut shared.lock_unpoisoned(),
        &saved,
        updated,
        accepted,
        revision,
    );
    let saved_revision = match save_result {
        Ok(saved_revision) => saved_revision,
        Err(err) => {
            if let Some(id) = new_repository_id {
                keep_new_repository_id(&shared.lock_unpoisoned(), &saved, id.as_ref());
            }
            return Err(err);
        }
    };
    // An SFTP destination moved to another server leaves the old server's lines in Kestic's
    // `known_hosts` only while another destination uses it.
    if changed {
        super::host_keys::forget_unused_host(shared, &saved);
    }
    let guard = shared.lock_unpoisoned();
    Ok(reply(
        &guard,
        DestinationUpdateOutcome::Saved,
        saved_revision,
    ))
}

/// `OtherRepository`: nothing is saved. Also here a destination changed or removed meanwhile is
/// `Conflict` or `NotFound`, so the answer never names a state the caller did not see.
fn other_repository(
    shared: &Arc<Mutex<DaemonCore>>,
    dest_id: &DestinationId,
    revision: &str,
) -> Result<UpdateReply, KesticServiceErr> {
    let guard = shared.lock_unpoisoned();
    let current = guard
        .config
        .destination(dest_id)
        .ok_or_else(|| KesticServiceErr::NotFound(dest_id.as_str().to_string()))?;
    refuse_stale_revision(current, revision)?;
    let current_revision = revision_of(current);
    Ok(reply(
        &guard,
        DestinationUpdateOutcome::OtherRepository,
        current_revision,
    ))
}

/// After a refused save, the repository `init` already created at the unchanged address keeps
/// its ID in the history (its password is stored already), so the next save recognises it.
fn keep_new_repository_id(guard: &DaemonCore, saved: &Destination, id: Option<&RepositoryId>) {
    let Some(current) = guard.config.destination(&saved.id) else {
        return;
    };
    if current.kind != saved.kind {
        return;
    }
    match id {
        Some(id) => store_repository_id(guard, current, id),
        None => {
            if let Err(err) = guard.history.forget_repository_id(&current.id) {
                eprintln!(
                    "kestic-daemon: failed to forget the repository ID of destination {}: {err}",
                    current.id
                );
            }
        }
    }
}

/// `Conflict` unless `revision` is the one of `saved` (also when it is empty or unknown).
fn refuse_stale_revision(saved: &Destination, revision: &str) -> Result<(), KesticServiceErr> {
    if revision_of(saved) == revision {
        Ok(())
    } else {
        Err(KesticServiceErr::Conflict(format!(
            "destination {} was changed meanwhile",
            saved.id
        )))
    }
}

impl DaemonCore {
    /// Refuses `dest` when another destination uses its location.
    fn refuse_duplicate(&self, dest: &Destination) -> Result<(), KesticServiceErr> {
        match self
            .config
            .find_duplicate_destination(&dest.kind, Some(&dest.id))
        {
            Some(existing) => Err(KesticServiceErr::DuplicateLocation(existing.name.clone())),
            None => Ok(()),
        }
    }
}

/// `init` at the unchanged address (the repository there was deleted), with the entered or the
/// saved repository password.
fn init_in_place(
    dest: &Destination,
    repo_password: &str,
    entered: &BackendCredentials,
    deps: &ResticDeps,
) -> Result<Accepted, KesticServiceErr> {
    let store = deps.secret_store.as_ref();
    let password = if repo_password.is_empty() {
        store
            .get_secret(dest.id.as_str(), SECRET_KIND_REPO)
            .map_err(map_secret_err)?
            .ok_or_else(|| {
                KesticServiceErr::InvalidInput("missing stored repository password".to_string())
            })?
    } else {
        SecretValue::from_str_secret(repo_password)
    };
    let credentials = entered
        .clone()
        .or_stored(dest, store)
        .map_err(map_secret_err)?;
    if let Some(kind) = credentials.first_missing(&dest.kind)
        && !matches!(dest.kind, DestinationKind::Rest { .. })
    {
        return Err(KesticServiceErr::InvalidInput(format!(
            "missing backend credential `{kind}`"
        )));
    }
    let resolved = resolve_and_check(&deps.provider).map_err(|e| map_provider_err(&e))?;
    let prepared = prepare_repository(
        dest,
        DestinationSetupMode::InitNew,
        &password,
        &credentials,
        &deps.runtime(&resolved),
    )
    .map_err(map_destination_err)?;
    // The new repository opens only with this password.
    if !repo_password.is_empty() {
        store
            .set_secret(dest.id.as_str(), SECRET_KIND_REPO, &password)
            .map_err(map_secret_err)?;
    }
    Ok(Accepted {
        credentials,
        repository_id: prepared.repository_id,
        new_repository: true,
    })
}

/// Saves an accepted edit under the daemon lock: checks again that the destination exists
/// unchanged since `revision` was read and that its location is free, writes the secrets, then
/// `config.toml`; when the config cannot be written the secrets get their previous values back.
/// The password of a repository `init` created in this call is already stored, so that
/// repository stays readable also when this check fails.
fn save_update(
    guard: &mut DaemonCore,
    saved: &Destination,
    updated: Destination,
    accepted: Accepted,
    revision: &str,
) -> Result<String, KesticServiceErr> {
    let idx = guard
        .config
        .destinations
        .iter()
        .position(|d| d.id == updated.id)
        .ok_or_else(|| KesticServiceErr::NotFound(updated.id.as_str().to_string()))?;
    // The check was made against the destination saved then; any change in between voids it.
    refuse_stale_revision(&guard.config.destinations[idx], revision)?;
    // This call changes the name and the connection; the maintenance schedule has a call of its
    // own and keeps its saved value. The recovery kit belongs to the repository: it is kept for
    // the same repository at a new address and dropped for one created anew.
    let mut updated = updated;
    let current = &guard.config.destinations[idx];
    updated.maintenance = current.maintenance.clone();
    updated.recovery_kit_key_id = if accepted.new_repository {
        None
    } else {
        current.recovery_kit_key_id.clone()
    };
    guard.refuse_while_restoring(&updated.id)?;
    guard.refuse_duplicate(&updated)?;
    let store = Arc::clone(&guard.secret_store);
    let previous = accepted
        .credentials
        .persist(&updated, store.as_ref())
        .map_err(map_secret_err)?;
    let mut next = guard.config.clone();
    next.destinations[idx] = updated.clone();
    if let Err(err) = guard.persist_config(next) {
        previous.restore(store.as_ref());
        return Err(err);
    }
    // A secret variable that was removed or renamed leaves nothing behind in the password store.
    // A failure leaves an unused entry behind; the saved destination works without it, so the
    // save stands.
    if let Err(err) = BackendCredentials::delete_unused(saved, &updated, store.as_ref()) {
        eprintln!(
            "kestic-daemon: failed to remove unused credentials of destination {}: {err}",
            updated.id
        );
    }
    match accepted.repository_id {
        // The ID of the checked repository: the stored one, or the first one known.
        Some(id) => store_repository_id(guard, &updated, &id),
        // A repository created in place whose ID restic did not print: the old ID is void.
        None if accepted.new_repository => {
            if let Err(err) = guard.history.forget_repository_id(&updated.id) {
                eprintln!(
                    "kestic-daemon: failed to forget the repository ID of destination {}: {err}",
                    updated.id
                );
            }
        }
        None => {}
    }
    Ok(guard
        .config
        .destination(&updated.id)
        .map_or_else(String::new, revision_of))
}
