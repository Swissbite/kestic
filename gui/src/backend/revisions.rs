// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Revisions of the backups and destinations the editor pages show. A page keeps the revision it
//! opened with and sends it when it saves; the daemon refuses with `Conflict` when the backup or
//! destination changed meanwhile. A save of this window moves its own revision on: the next save
//! of the same or another page, sent before the window has the new state, takes the revision the
//! save returned instead of conflicting with itself.

use crate::state_cache::shared_state;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::KesticDbusError;
use kestic_i18n::codes::ServiceErrorCode;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The answer of a save that the daemon refused with `Conflict`; the page reloads and says so.
pub(crate) const CONFLICT_ANSWER: &str = "conflict";

/// How many own saves in a row are followed; more than a window makes before it has the state.
const MAX_CHAIN: usize = 64;

fn own_saves() -> &'static Mutex<HashMap<String, String>> {
    static OWN: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    OWN.get_or_init(Mutex::default)
}

/// The revision to send for a page that opened with `revision`: the newest one the window's own
/// saves led to from it.
pub(crate) fn latest_own(revision: &str) -> String {
    let saves = own_saves().lock_unpoisoned();
    let mut current = revision.to_string();
    for _ in 0..MAX_CHAIN {
        match saves.get(&current) {
            Some(next) if next != &current => current.clone_from(next),
            _ => break,
        }
    }
    current
}

/// Records that a save of this window turned `from` into `to`.
pub(crate) fn record_own(from: &str, to: &str) {
    if !from.is_empty() && !to.is_empty() && from != to {
        own_saves()
            .lock_unpoisoned()
            .insert(from.to_string(), to.to_string());
    }
}

fn is_conflict_error<E: Clone + Into<KesticDbusError>>(err: &E) -> bool {
    err.clone().into().code() == ServiceErrorCode::Conflict
}

/// Whether `result` of a save is the daemon's `Conflict`.
pub(crate) fn is_conflict_reply<T, E>(result: &Result<Result<T, E>, String>) -> bool
where
    E: Clone + Into<KesticDbusError>,
{
    matches!(result, Ok(Err(err)) if is_conflict_error(err))
}

/// [`is_conflict_reply`] for a save whose input is checked before the call (`Ok(Err(text))` is
/// input it refused).
pub(crate) fn is_conflict<T, E>(result: &Result<Result<Result<T, E>, String>, String>) -> bool
where
    E: Clone + Into<KesticDbusError>,
{
    matches!(result, Ok(Ok(Err(err))) if is_conflict_error(err))
}

/// The revision of the backup `backup_id` in the window's state (empty when it is unknown).
pub(crate) fn cached_backup_revision(backup_id: &str) -> String {
    let guard = shared_state().lock_unpoisoned();
    guard
        .state
        .as_ref()
        .and_then(|s| s.backups.iter().find(|b| b.backup.id.as_str() == backup_id))
        .map_or_else(String::new, |b| b.revision.clone())
}

/// The revision of the destination `dest_id` in the window's state (empty when it is unknown).
pub(crate) fn cached_destination_revision(dest_id: &str) -> String {
    let guard = shared_state().lock_unpoisoned();
    guard
        .state
        .as_ref()
        .and_then(|s| {
            s.destinations
                .iter()
                .find(|d| d.destination.id.as_str() == dest_id)
        })
        .map_or_else(String::new, |d| d.revision.clone())
}

#[cfg(test)]
#[path = "revisions_tests.rs"]
mod tests;
