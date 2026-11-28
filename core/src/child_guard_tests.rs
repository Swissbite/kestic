// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::path::Path;
use std::process::Command;

fn sleeper() -> ChildGuard {
    ChildGuard::spawn(Command::new("sleep").arg("30")).expect("spawn sleep")
}

/// `/proc/<pid>` exists while the process runs or is an unreaped zombie.
fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// An early return drops the guard: the child is killed and reaped, so neither a running
/// process nor a zombie is left.
#[test]
fn dropped_guard_kills_and_reaps_the_child() {
    let guard = sleeper();
    let pid = guard.lock().id();
    assert!(process_exists(pid), "precondition: the child runs");
    drop(guard);
    assert!(!process_exists(pid), "the child is gone after the drop");
}

/// The same holds when another thread still holds the shared child.
#[test]
fn dropped_guard_kills_a_shared_child() {
    let guard = sleeper();
    let shared = guard.shared();
    let pid = shared.lock().expect("lock").id();
    drop(guard);
    assert!(!process_exists(pid), "the child is gone after the drop");
    drop(shared);
}

/// A child that was waited for is left alone; its status is the real one.
#[test]
fn waited_child_keeps_its_exit_status() {
    let mut guard = ChildGuard::spawn(Command::new("sh").args(["-c", "exit 3"])).expect("sh");
    let status = guard.wait().expect("wait");
    assert_eq!(status.code(), Some(3));
    drop(guard);

    let mut guard = ChildGuard::spawn(&mut Command::new("true")).expect("true");
    let status = guard.shared().lock().expect("lock").wait().expect("wait");
    guard.mark_waited();
    assert!(status.success());
}
