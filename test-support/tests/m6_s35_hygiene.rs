// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M6-S35: shared test helpers instead of hand-written copies (waits, child guards, config,
//! port and bus name), and polls instead of fixed sleeps in the Rust tests.

use kestic_test_support::process::KillOnDrop;
use kestic_test_support::wait::{poll_within, try_poll_within, wait_until_within};
use kestic_test_support::workspace::{files_with_ext, workspace_root};
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// The crates whose sources hold tests.
const CRATES: [&str; 6] = [
    "build-stamp",
    "core",
    "daemon",
    "gui",
    "i18n",
    "test-support",
];

/// A source file below the workspace root, with its path relative to the root.
struct Source {
    rel: String,
    text: String,
}

/// Every Rust file with test code: the `tests/` directories, the `*_tests.rs` / `tests.rs`
/// modules below `src/` and the whole `test-support/src`.
fn test_sources() -> Vec<Source> {
    let root = workspace_root();
    let mut paths: Vec<PathBuf> = Vec::new();
    for krate in CRATES {
        let tests = root.join(krate).join("tests");
        if tests.is_dir() {
            paths.extend(files_with_ext(&tests, "rs"));
        }
        let src = root.join(krate).join("src");
        paths.extend(files_with_ext(&src, "rs").into_iter().filter(|p| {
            krate == "test-support" || {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                name.ends_with("_tests.rs") || name == "tests.rs"
            }
        }));
    }
    paths
        .into_iter()
        .map(|p| Source {
            rel: rel(&root, &p),
            text: fs::read_to_string(&p).expect("read test source"),
        })
        .collect()
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("below workspace root")
        .to_string_lossy()
        .into_owned()
}

/// `Instant::now()` directly followed (spaces allowed) by `+`: a hand-written deadline.
fn has_hand_rolled_deadline(line: &str) -> bool {
    let needle = concat!("Instant::", "now()");
    line.match_indices(needle)
        .any(|(at, _)| line[at + needle.len()..].trim_start().starts_with('+'))
}

/// The bodies of all `impl Drop for …` blocks of `text`.
fn drop_impls(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices("impl Drop for ") {
        let rest = &text[at..];
        let Some(open) = rest.find('{') else { continue };
        let mut depth = 0usize;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push(&rest[..open + i + 1]);
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

#[test]
fn ac_m6_35_1_wait_until_within_returns_once_the_condition_holds_and_fails_after_the_timeout() {
    // (a) returns as soon as the predicate holds, polling in between
    let mut calls = 0;
    wait_until_within(Duration::from_secs(5), "third poll", || {
        calls += 1;
        calls == 3
    });
    assert_eq!(calls, 3, "polls until the predicate holds, then stops");

    // (a) fails naming what it waited for once the timeout has passed
    let started = Instant::now();
    let failed = panic::catch_unwind(|| {
        wait_until_within(Duration::from_millis(100), "never", || false);
    });
    let message = failed
        .expect_err("a predicate that never holds fails the wait")
        .downcast::<String>()
        .expect("panic message");
    assert_eq!(*message, "timed out waiting for: never");
    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "fails only after the timeout"
    );

    // (a) the value-returning forms
    let mut n = 0;
    let value = poll_within(Duration::from_secs(5), "second poll", || {
        n += 1;
        (n == 2).then_some("ready")
    });
    assert_eq!(value, "ready");
    assert_eq!(
        try_poll_within(Duration::from_millis(50), || None::<()>),
        None
    );
}

#[test]
fn ac_m6_35_1_no_test_has_a_hand_rolled_deadline_loop() {
    let wait_module = "test-support/src/wait.rs";
    // (a) the helper lives in kestic-test-support
    let helper = fs::read_to_string(workspace_root().join(wait_module)).expect("read wait.rs");
    assert!(
        helper.contains("pub fn wait_until_within("),
        "{wait_module} defines `wait_until_within`"
    );

    // (b) no other test source builds its own deadline from the current instant
    let offenders: Vec<String> = test_sources()
        .iter()
        .filter(|s| s.rel != wait_module)
        .flat_map(|s| {
            s.text
                .lines()
                .enumerate()
                .filter(|(_, l)| has_hand_rolled_deadline(l))
                .map(|(i, l)| format!("{}:{}: {}", s.rel, i + 1, l.trim()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "hand-written deadline loops left (use kestic_test_support::wait):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn ac_m6_35_2_kill_on_drop_kills_the_child_when_the_test_panics() {
    let mut pid = None;
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        let child = KillOnDrop::spawn(Command::new("sleep").arg("60"), "sleep 60");
        pid = Some(child.id());
        panic!("simulated test failure");
    }));
    assert!(outcome.is_err(), "the simulated test panicked");
    let pid = pid.expect("child spawned before the panic");

    // (a) the child no longer runs: killed and reaped, so its /proc entry is gone
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "child {pid} still exists after the panic"
    );
}

#[test]
fn ac_m6_35_2_guards_and_listed_tests_use_kill_on_drop() {
    let sources = test_sources();
    // (b) no hand-written guard kills a child in its own `Drop`
    let guards: Vec<String> = sources
        .iter()
        .filter(|s| s.rel != "test-support/src/process.rs")
        .flat_map(|s| {
            drop_impls(&s.text)
                .into_iter()
                .filter(|body| body.contains(".kill()"))
                .map(|body| {
                    format!(
                        "{}: {}",
                        s.rel,
                        body.lines().next().unwrap_or_default().trim()
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        guards.is_empty(),
        "hand-written kill guards left (use KillOnDrop):\n{}",
        guards.join("\n")
    );

    // (b) the tests that leaked children on panic use KillOnDrop
    for rel in [
        "daemon/tests/m1_s12_autostart.rs",
        "daemon/tests/m1_s11_tray_notify.rs",
        "core/tests/m3_s04_s05_s06_restic.rs",
        "core/tests/m1_s10_stale_lock.rs",
    ] {
        let text = fs::read_to_string(workspace_root().join(rel)).expect("read test");
        assert!(text.contains("KillOnDrop"), "{rel} uses KillOnDrop");
    }
}

#[test]
fn ac_m6_35_3_shared_helpers_are_defined_once_in_test_support() {
    let sources = test_sources();
    let defined_in = |pred: &dyn Fn(&str) -> bool| -> Vec<String> {
        sources
            .iter()
            .filter(|s| s.text.lines().any(pred))
            .map(|s| s.rel.clone())
            .collect()
    };

    // (a) `load_config`
    assert_eq!(
        defined_in(&|l| l.trim_start().starts_with("pub fn load_config(")
            || l.trim_start().starts_with("fn load_config(")),
        vec!["test-support/src/config.rs".to_string()],
        "`load_config` is defined once, in test-support"
    );

    // (a) the free port
    assert_eq!(
        defined_in(&|l| {
            let l = l.trim_start();
            (l.starts_with("fn ") || l.starts_with("pub fn "))
                && l.contains("port")
                && l.ends_with("-> u16 {")
        }),
        vec!["test-support/src/tools.rs".to_string()],
        "the free-port helper is defined once, in test-support"
    );

    // (a) the bus name of a test daemon
    assert_eq!(
        defined_in(&|l| {
            let prefix = concat!("\"net.daester.", "Kestic.");
            l.match_indices(prefix).any(|(at, _)| {
                let name = &l[at + prefix.len()..];
                let name = &name[..name.find('"').unwrap_or(name.len())];
                !name.starts_with("Error.") && name.contains('{')
            })
        }),
        vec!["test-support/src/bus.rs".to_string()],
        "test bus names are built once, in test-support"
    );
}

/// Fixed sleeps of at least 200 ms in `text` as `(line number, milliseconds, marked)`: `marked`
/// when a `// quiet window` comment stands on the line or one of the three lines above.
fn long_sleeps(text: &str) -> Vec<(usize, u64, bool)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        for (at, _) in line.match_indices("sleep(") {
            let rest = &line[at + "sleep(".len()..];
            let rest = rest
                .trim_start_matches("std::time::")
                .trim_start_matches("Duration::");
            let (unit, digits) = if let Some(r) = rest.strip_prefix("from_secs(") {
                (1000, r)
            } else if let Some(r) = rest.strip_prefix("from_millis(") {
                (1, r)
            } else {
                continue;
            };
            let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
            let Ok(n) = digits.parse::<u64>() else {
                continue;
            };
            let ms = n * unit;
            if ms >= 200 {
                let marked = lines[i.saturating_sub(3)..=i]
                    .iter()
                    .any(|l| l.contains("// quiet window"));
                out.push((i + 1, ms, marked));
            }
        }
    }
    out
}

#[test]
fn ac_m6_35_4_rust_tests_poll_instead_of_sleeping() {
    let sources = test_sources();
    let regressions = "daemon/tests/m3_daemon_regressions.rs";
    let text = &sources
        .iter()
        .find(|s| s.rel == regressions)
        .expect("regression tests")
        .text;

    // (b) the quiet windows of the regression tests count scheduler ticks instead of sleeping
    assert_eq!(
        long_sleeps(text),
        Vec::new(),
        "{regressions} has no fixed sleeps of 200 ms or more"
    );
    assert!(
        text.contains(".wait_ticks("),
        "{regressions} waits for scheduler ticks"
    );

    // (c) every other fixed sleep of 200 ms or more is a declared quiet window; the rest poll
    let unmarked: Vec<String> = sources
        .iter()
        .flat_map(|s| {
            long_sleeps(&s.text)
                .into_iter()
                .filter(|&(_, _, marked)| !marked)
                .map(|(line, ms, _)| format!("{}:{line}: {ms} ms", s.rel))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        unmarked.is_empty(),
        "fixed sleeps without a `// quiet window` comment (poll instead):\n{}",
        unmarked.join("\n")
    );
}
