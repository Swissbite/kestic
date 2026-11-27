// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S33: the GUI calls the daemon only through the client generated with `#[proxy]`, never
//! by a method name as text (`AC-M6-33-2` (b)).

use kestic_test_support::workspace::workspace_root;
use std::fs;
use std::path::{Path, PathBuf};

/// Calls that name the D-Bus method as text, in source without whitespace.
const TEXTUAL_CALLS: [&str; 6] = [
    ".call(\"",
    ".call::<",
    ".call_noreply(",
    ".call_with_flags(",
    "call_method(",
    "Proxy::new(",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Source text without line comments and without any whitespace, so a call split over lines
/// is found as well.
fn compact(source: &str) -> String {
    source
        .lines()
        .map(|l| l.split("//").next().unwrap_or_default())
        .collect::<String>()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Whether `source` contains `pattern` as a whole name: `Proxy::new(` does not match the
/// generated `DBusProxy::new(`.
fn contains_call(source: &str, pattern: &str) -> bool {
    source.match_indices(pattern).any(|(at, _)| {
        !pattern.starts_with(|c: char| c.is_alphanumeric())
            || !source[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// AC-M6-33-2 (b): the GUI and the client it uses call the daemon through a client generated
/// with `#[proxy]` for `net.daester.Kestic1`, not through method names as text.
#[test]
fn ac_m6_33_2b_daemon_calls_go_through_generated_proxy() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("gui/src"), &mut files);
    rust_files(&root.join("daemon/src/dbus_api"), &mut files);
    let mut offenders = Vec::new();
    for file in &files {
        let source = compact(&fs::read_to_string(file).expect("read source"));
        for pattern in TEXTUAL_CALLS {
            if contains_call(&source, pattern) {
                offenders.push(format!(
                    "{}: {pattern}",
                    file.strip_prefix(&root).unwrap_or(file).display()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "(b) daemon calls by method name as text: {offenders:#?}"
    );

    let generated = files.iter().any(|file| {
        let source = compact(&fs::read_to_string(file).expect("read source"));
        source.contains("#[zbus::proxy(interface=\"net.daester.Kestic1\"")
    });
    let client = compact(
        &fs::read_to_string(root.join("daemon/src/dbus_api/client.rs")).expect("read client"),
    );
    assert!(
        generated && client.contains("Kestic1Proxy"),
        "(b) the client is built on a `#[proxy]` trait for net.daester.Kestic1"
    );
}
