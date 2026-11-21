// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Small helpers for `std::sync` primitives.

use std::sync::{Mutex, MutexGuard};

/// Locks a [`Mutex`] and carries on with the data after a panic in another holder: the state
/// kept behind these mutexes stays usable, so a poisoned lock is not an error.
pub trait LockUnpoisoned<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T>;
}

impl<T> LockUnpoisoned<T> for Mutex<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
