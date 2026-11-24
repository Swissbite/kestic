// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Keeps the app-managed restic from being replaced while restic runs. Every restic child process
//! holds a read lease for its lifetime; an installation replaces the binary only under the
//! replace lease, which it gets once no read lease is held. A restic process that wants to start
//! while the binary is being replaced waits until the replace lease is gone. New read leases are
//! granted while an installation waits, so a long backup never blocks other restic calls.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// How often a waiting installation looks whether it was cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Default)]
struct GateState {
    readers: usize,
    replacing: bool,
}

/// Read leases for restic processes, the replace lease for an installation.
#[derive(Debug, Default)]
pub struct ResticBinaryGate {
    state: Mutex<GateState>,
    changed: Condvar,
}

static GLOBAL: ResticBinaryGate = ResticBinaryGate::new();

impl ResticBinaryGate {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(GateState {
                readers: 0,
                replacing: false,
            }),
            changed: Condvar::new(),
        }
    }

    /// The gate of this process, which every restic child process passes.
    pub fn global() -> &'static Self {
        &GLOBAL
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A read lease for one restic process; waits while the binary is being replaced.
    pub fn read(&self) -> ReadLease<'_> {
        let mut state = self.lock();
        while state.replacing {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.readers += 1;
        ReadLease { gate: self }
    }

    /// The replace lease, as soon as no read lease is held; `None` once `cancelled` returns true
    /// while waiting.
    pub fn replace_when_idle(&self, cancelled: &dyn Fn() -> bool) -> Option<ReplaceLease<'_>> {
        let mut state = self.lock();
        // Only one installation runs at a time; a second one waits like a reader would.
        while state.readers > 0 || state.replacing {
            if cancelled() {
                return None;
            }
            state = self
                .changed
                .wait_timeout(state, CANCEL_POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        if cancelled() {
            return None;
        }
        state.replacing = true;
        Some(ReplaceLease { gate: self })
    }

    /// Number of read leases held now.
    pub fn readers(&self) -> usize {
        self.lock().readers
    }
}

/// Held while a restic process runs.
#[derive(Debug)]
pub struct ReadLease<'a> {
    gate: &'a ResticBinaryGate,
}

impl Drop for ReadLease<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.lock();
        state.readers = state.readers.saturating_sub(1);
        self.gate.changed.notify_all();
    }
}

/// Held while the binary is replaced.
#[derive(Debug)]
pub struct ReplaceLease<'a> {
    gate: &'a ResticBinaryGate,
}

impl Drop for ReplaceLease<'_> {
    fn drop(&mut self) {
        self.gate.lock().replacing = false;
        self.gate.changed.notify_all();
    }
}

#[cfg(test)]
#[path = "restic_gate_tests.rs"]
mod tests;
