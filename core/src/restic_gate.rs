// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Keeps the app-managed restic from being replaced while restic runs. Every restic child process
//! holds a read lease for its lifetime; an installation replaces the binary only under the
//! replace lease, which it gets once no read lease is held. A restic process that wants to start
//! while the binary is being replaced waits until the replace lease is gone. New read leases are
//! granted while an installation waits, so a long backup never blocks other restic calls.
//!
//! A read lease is bound to the binary its process runs once that binary is known. Removing the
//! app-managed binary needs that no lease is bound to it and no lease is still unbound (its
//! process may run any binary); then it holds the replace lease while it deletes.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// How often a waiting installation looks whether it was cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Default)]
struct GateState {
    readers: usize,
    /// Read leases not bound to a binary yet.
    unbound: usize,
    /// Read leases per binary.
    bound: Vec<(PathBuf, usize)>,
    replacing: bool,
}

impl GateState {
    fn add(&mut self, binary: Option<&Path>) {
        self.readers += 1;
        match binary {
            None => self.unbound += 1,
            Some(binary) => match self.bound.iter_mut().find(|(path, _)| path == binary) {
                Some((_, count)) => *count += 1,
                None => self.bound.push((binary.to_path_buf(), 1)),
            },
        }
    }

    fn remove(&mut self, binary: Option<&Path>) {
        self.readers = self.readers.saturating_sub(1);
        match binary {
            None => self.unbound = self.unbound.saturating_sub(1),
            Some(binary) => {
                if let Some(pos) = self.bound.iter().position(|(path, _)| path == binary) {
                    self.bound[pos].1 -= 1;
                    if self.bound[pos].1 == 0 {
                        self.bound.swap_remove(pos);
                    }
                }
            }
        }
    }

    fn may_run(&self, binary: &Path) -> bool {
        self.unbound > 0 || self.bound.iter().any(|(path, _)| path == binary)
    }
}

/// The name a binary is counted under: its canonical path, so a link to it counts as it.
fn binary_key(binary: &Path) -> PathBuf {
    std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf())
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
                unbound: 0,
                bound: Vec::new(),
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

    /// A read lease for one restic process whose binary is not known yet; waits while the
    /// binary is being replaced.
    pub fn read(&self) -> ReadLease<'_> {
        self.lease(None)
    }

    fn lease(&self, binary: Option<PathBuf>) -> ReadLease<'_> {
        let mut state = self.lock();
        while state.replacing {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.add(binary.as_deref());
        ReadLease { gate: self, binary }
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

    /// A read lease for a restic process that runs `binary`; waits while a binary is being
    /// replaced.
    pub fn read_for(&self, binary: &Path) -> ReadLease<'_> {
        self.lease(Some(binary_key(binary)))
    }

    /// The replace lease for removing `binary`, at once, or `None` while a restic process may
    /// run it (a lease bound to it or not bound yet) or a binary is being replaced.
    pub fn remove_if_unused(&self, binary: &Path) -> Option<ReplaceLease<'_>> {
        let key = binary_key(binary);
        let mut state = self.lock();
        if state.replacing || state.may_run(&key) {
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
    /// The binary the process runs, `None` until it is known.
    binary: Option<PathBuf>,
}

impl ReadLease<'_> {
    /// Binds this lease to the binary its restic process runs.
    pub fn bind(&mut self, binary: &Path) {
        let key = binary_key(binary);
        let mut state = self.gate.lock();
        state.remove(self.binary.as_deref());
        state.add(Some(&key));
        self.binary = Some(key);
    }
}

impl Drop for ReadLease<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.lock();
        state.remove(self.binary.as_deref());
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
