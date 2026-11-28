// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! A spawned child process that is killed and reaped when the code that started it returns
//! early, so no restic outlives the read lease of the run gate it was started under.

use crate::child_registry::{ChildRegistry, Registration};
use crate::sync::LockUnpoisoned;
use std::process::{Child, Command, ExitStatus};
use std::sync::{Arc, Mutex, MutexGuard};

/// Owns a child process until it was waited for; dropping it before kills and reaps the child.
/// The child is in [`ChildRegistry::global`] until it is reaped.
pub(crate) struct ChildGuard {
    child: Arc<Mutex<Child>>,
    waited: bool,
    registration: Option<Registration<'static>>,
}

impl ChildGuard {
    /// Starts `cmd` through the registry of this process (refused once the daemon stops).
    pub(crate) fn spawn(cmd: &mut Command) -> std::io::Result<Self> {
        let (child, registration) = ChildRegistry::global().spawn(cmd)?;
        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            waited: false,
            registration: Some(registration),
        })
    }

    /// The child, shared with threads that signal or poll it.
    pub(crate) fn shared(&self) -> Arc<Mutex<Child>> {
        Arc::clone(&self.child)
    }

    /// The child, locked.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Child> {
        self.child.lock_unpoisoned()
    }

    /// Waits for the child to end; afterwards the guard leaves it alone.
    pub(crate) fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let status = self.child.lock_unpoisoned().wait()?;
        self.mark_waited();
        Ok(status)
    }

    /// Records that the child was waited for elsewhere (through [`Self::shared`]).
    pub(crate) fn mark_waited(&mut self) {
        self.waited = true;
        self.registration = None;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.waited {
            return;
        }
        let mut child = self.child.lock_unpoisoned();
        // Killing a child that has already ended fails harmlessly; the wait reaps it either way.
        if let Err(err) = child.kill() {
            eprintln!("kestic: could not stop restic after an error: {err}");
        }
        if let Err(err) = child.wait() {
            eprintln!("kestic: could not wait for restic after an error: {err}");
        }
        drop(child);
        self.registration = None;
    }
}

#[cfg(test)]
#[path = "child_guard_tests.rs"]
mod tests;
