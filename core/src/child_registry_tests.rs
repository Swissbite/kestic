// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use kestic_test_support::process::catches_sigterm;
use kestic_test_support::wait::wait_until_within;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

fn shell(script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd
}

fn alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            let rest = &stat[stat.rfind(')')? + 1..];
            rest.split_whitespace().next().map(|s| s != "Z")
        })
        .unwrap_or(false)
}

fn read_pid(path: &Path) -> u32 {
    wait_until_within(Duration::from_secs(5), "pid file", || {
        fs::read_to_string(path).is_ok_and(|s| s.trim().parse::<u32>().is_ok())
    });
    fs::read_to_string(path)
        .expect("pid file")
        .trim()
        .parse()
        .expect("pid")
}

/// AC-M6-36-2 (c): after `close` no child starts; the refusal names the stop.
#[test]
fn ac_m6_36_2_c_no_start_after_close() {
    let registry = ChildRegistry::new();
    registry.close();
    let err = registry
        .spawn(&mut shell("exit 0"))
        .expect_err("spawn after close");
    assert_eq!(err.to_string(), STOPPING_MESSAGE);
    assert_eq!(registry.live_count(), 0);
}

/// AC-M6-36-2 (c): once the flag of the signal handler is set, no child starts, before `close`.
#[test]
fn ac_m6_36_2_c_no_start_once_the_stop_flag_is_set() {
    let registry = ChildRegistry::new();
    let flag = Arc::new(AtomicBool::new(false));
    registry.stop_on(Arc::clone(&flag));
    let (mut child, reg) = registry
        .spawn(&mut shell("exit 0"))
        .expect("spawn before the flag");
    child.wait().expect("wait");
    drop(reg);
    flag.store(true, Ordering::SeqCst);
    let err = registry
        .spawn(&mut shell("exit 0"))
        .expect_err("spawn after the flag");
    assert_eq!(err.to_string(), STOPPING_MESSAGE);
    assert!(!registry.is_closed());
}

/// AC-M6-36-2 (a): `close` sends `SIGTERM` to every live child, once, also when the run's own
/// cancel already sent one.
#[test]
fn ac_m6_36_2_a_close_sends_sigterm_once_per_child() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("signals");
    let script = format!(
        "trap 'echo T >> {log}' TERM; while :; do sleep 0.05; done",
        log = log.display()
    );
    let registry = ChildRegistry::new();
    let (mut first, reg_first) = registry.spawn(&mut shell(&script)).expect("spawn first");
    let (mut second, reg_second) = registry.spawn(&mut shell(&script)).expect("spawn second");
    assert_eq!(registry.live_count(), 2);
    wait_until_within(Duration::from_secs(5), "both traps installed", || {
        catches_sigterm(first.id()) && catches_sigterm(second.id())
    });

    assert!(registry.terminate(first.id()), "first SIGTERM is sent");
    assert!(!registry.terminate(first.id()), "a second one is not");
    registry.close();
    wait_until_within(Duration::from_secs(5), "two SIGTERM lines", || {
        fs::read_to_string(&log).is_ok_and(|s| s.lines().count() >= 2)
    });
    // quiet window: no third SIGTERM follows
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        fs::read_to_string(&log).expect("log").lines().count(),
        2,
        "each child got exactly one SIGTERM"
    );
    assert!(registry.is_closed());

    for (child, reg) in [(&mut first, reg_first), (&mut second, reg_second)] {
        child.kill().expect("kill");
        child.wait().expect("wait");
        drop(reg);
    }
    assert_eq!(registry.live_count(), 0);
}

/// AC-M6-36-3 (b), (c): `kill_remaining` ends a child that ignores `SIGTERM` together with a
/// grandchild that holds its stdout open.
#[test]
fn ac_m6_36_3_kill_remaining_ends_the_whole_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let grandchild_pid = dir.path().join("grandchild.pid");
    let script = format!(
        "trap '' TERM; sh -c 'trap \"\" TERM; echo $$ > {pidfile}; while :; do sleep 0.05; done' & \
         while :; do sleep 0.05; done",
        pidfile = grandchild_pid.display()
    );
    let registry = ChildRegistry::new();
    let (mut child, reg) = registry.spawn(&mut shell(&script)).expect("spawn");
    let grandchild = read_pid(&grandchild_pid);
    registry.close();
    // quiet window: the SIGTERM of `close` arrives and is ignored
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(child.id()), "the child ignores SIGTERM");
    assert!(alive(grandchild), "the grandchild ignores SIGTERM");

    assert!(
        registry.kill_remaining() >= 2,
        "child and grandchild get SIGKILL"
    );
    child.wait().expect("reap child");
    drop(reg);
    wait_until_within(Duration::from_secs(5), "grandchild gone", || {
        !alive(grandchild)
    });
}

/// AC-M6-36-3 (c): a process that was below a child at `close` is killed after the child ended
/// and left it to another parent.
#[test]
fn ac_m6_36_3_kill_remaining_reaches_a_left_behind_descendant() {
    let dir = tempfile::tempdir().expect("tempdir");
    let grandchild_pid = dir.path().join("grandchild.pid");
    let script = format!(
        "sh -c 'trap \"\" TERM; echo $$ > {pidfile}; while :; do sleep 0.05; done' & \
         trap 'exit 0' TERM; while :; do sleep 0.05; done",
        pidfile = grandchild_pid.display()
    );
    let registry = ChildRegistry::new();
    let (mut child, reg) = registry.spawn(&mut shell(&script)).expect("spawn");
    let grandchild = read_pid(&grandchild_pid);
    registry.close();
    wait_until_within(Duration::from_secs(5), "the child ends on SIGTERM", || {
        !alive(child.id())
    });
    assert!(
        alive(grandchild),
        "precondition: the left-behind process runs"
    );

    assert!(
        registry.kill_remaining() >= 1,
        "the left-behind process gets SIGKILL"
    );
    wait_until_within(Duration::from_secs(5), "left-behind process gone", || {
        !alive(grandchild)
    });
    child.wait().expect("reap child");
    drop(reg);
}

/// AC-M6-36-3 (b): a reaped child is no target any more, so a reused number is never hit.
#[test]
fn ac_m6_36_3_kill_remaining_skips_reaped_children() {
    let registry = ChildRegistry::new();
    let (mut child, reg) = registry.spawn(&mut shell("exit 0")).expect("spawn");
    child.wait().expect("wait");
    drop(reg);
    assert_eq!(registry.kill_remaining(), 0);
}

/// The parent pid comes from the fields after the program name, which may hold `)` and spaces.
#[test]
fn parent_of_stat_reads_past_the_program_name() {
    assert_eq!(parent_of_stat("42 (a) b) S 7 42 42 0"), Some(7));
    assert_eq!(parent_of_stat("1 (init) S 0 1 1"), Some(0));
    assert_eq!(parent_of_stat("garbage"), None);
}

/// The tree below a process holds every generation, not only the direct children.
#[test]
fn descendants_cover_every_generation() {
    let table = HashMap::from([(2, 1), (3, 2), (4, 3), (5, 1), (9, 8)]);
    let mut found = descendants(1, &table);
    found.sort_unstable();
    assert_eq!(found, vec![2, 3, 4, 5]);
}
