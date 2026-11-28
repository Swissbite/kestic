// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Revisions of the backups and destinations the editor pages show. A page keeps the revision it
//! opened with and sends it when it saves; the daemon refuses with `Conflict` when the backup or
//! destination changed meanwhile. A page that only missed saves of this window (a second save
//! queued while the first was on its way, a page opened before the window had the new state)
//! sends the daemon's current revision instead of conflicting with the window itself.

use crate::state_cache::shared_state;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::KesticDbusError;
use kestic_i18n::codes::ServiceErrorCode;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

/// The answer of a save that the daemon refused with `Conflict`; the page reloads and says so.
pub(crate) const CONFLICT_ANSWER: &str = "conflict";

/// The saves of this window: each revision it saved from, with the revisions it led to.
fn own_saves() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static OWN: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    OWN.get_or_init(Mutex::default)
}

/// The revision to send for a page that opened with `opened` while the daemon holds `current`:
/// `current` when the window's own saves lead from `opened` to it (the page only missed saves of
/// this window), else `opened`, so the daemon answers `Conflict` for a change made elsewhere.
pub(crate) fn sent_revision(opened: &str, current: &str) -> String {
    if opened == current || opened.is_empty() {
        return opened.to_string();
    }
    let saves = own_saves().lock_unpoisoned();
    let mut seen = HashSet::from([opened.to_string()]);
    let mut queue = vec![opened.to_string()];
    while let Some(from) = queue.pop() {
        for to in saves.get(&from).into_iter().flatten() {
            if to == current {
                return current.to_string();
            }
            if seen.insert(to.clone()) {
                queue.push(to.clone());
            }
        }
    }
    opened.to_string()
}

/// Records that a save of this window turned `from` into `to`.
pub(crate) fn record_own(from: &str, to: &str) {
    if from.is_empty() || to.is_empty() || from == to {
        return;
    }
    let mut saves = own_saves().lock_unpoisoned();
    let next = saves.entry(from.to_string()).or_default();
    if !next.iter().any(|t| t == to) {
        next.push(to.to_string());
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
