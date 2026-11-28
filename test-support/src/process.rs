// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Child processes of a test that end with the test, also when it panics.

use std::fs;
use std::io::Read;
use std::ops::{Deref, DerefMut};
use std::process::{Child, Command, ExitStatus};

/// Owns a spawned child and kills and reaps it when dropped, so a panicking test leaves no
/// process behind. Derefs to the [`Child`] for `id`, `wait`, `stdin` and the like.
#[derive(Debug)]
pub struct KillOnDrop(Child);

impl KillOnDrop {
    /// Takes ownership of an already spawned `child`.
    pub fn new(child: Child) -> Self {
        Self(child)
    }

    /// Spawns `cmd`; fails the test naming `what` when that does not work.
    pub fn spawn(cmd: &mut Command, what: &str) -> Self {
        Self(
            cmd.spawn()
                .unwrap_or_else(|err| panic!("spawn {what}: {err}")),
        )
    }

    /// Reads the piped stderr to its end (the child closes it when it exits), then reaps the
    /// child; returns its exit status and stderr.
    pub fn wait_with_stderr(&mut self) -> (ExitStatus, String) {
        let mut stderr = String::new();
        self.0
            .stderr
            .take()
            .expect("stderr of the child is piped")
            .read_to_string(&mut stderr)
            .expect("read stderr of the child");
        (self.0.wait().expect("wait for the child"), stderr)
    }
}

impl Deref for KillOnDrop {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.0
    }
}

impl DerefMut for KillOnDrop {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // A child that has already ended (or was waited for) fails to be killed harmlessly;
        // `wait` reaps it either way.
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

/// Whether process `pid` has a handler for `SIGTERM` (`SigCgt` in `/proc/<pid>/status`); false
/// once the process is gone.
pub fn catches_sigterm(pid: u32) -> bool {
    const SIGTERM_BIT: u64 = 1 << (15 - 1);
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| {
            let mask = status.lines().find_map(|l| l.strip_prefix("SigCgt:"))?;
            u64::from_str_radix(mask.trim(), 16).ok()
        })
        .is_some_and(|mask| mask & SIGTERM_BIT != 0)
}
