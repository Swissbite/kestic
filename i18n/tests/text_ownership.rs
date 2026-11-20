// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Every text has one owner (`docs/spec/dbus-api.md` §Texte und Übersetzungen): texts that
//! daemon and GUI both need live in `kestic-i18n`, texts that only one side needs stay there.
//! Reads the source files only.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn files_with_ext(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            files_with_ext(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// String literal starting at byte `start` (just after the opening quote), unescaped.
fn literal_at(src: &str, start: usize) -> Option<String> {
    let mut out = String::new();
    let mut chars = src[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => out.push(chars.next()?),
            other => out.push(other),
        }
    }
    None
}

/// First string argument of every call to one of `functions` in `src`.
fn first_literals(src: &str, functions: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for function in functions {
        let pattern = format!("{function}(");
        let mut from = 0;
        while let Some(pos) = src[from..].find(&pattern) {
            let at = from + pos;
            from = at + pattern.len();
            let preceded_by_ident = src[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
            if preceded_by_ident {
                continue;
            }
            let rest = &src[from..];
            let trimmed = rest.trim_start();
            if let Some(stripped) = trimmed.strip_prefix('"') {
                let start = src.len() - stripped.len();
                if let Some(lit) = literal_at(src, start) {
                    found.push(lit);
                }
            }
        }
    }
    found
}

/// msgid → files that use it as an `i18n(...)` literal.
fn literals_in(dirs: &[(&str, &str)]) -> BTreeMap<String, BTreeSet<String>> {
    let root = workspace_root();
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (dir, ext) in dirs {
        let mut files = Vec::new();
        files_with_ext(&root.join(dir), ext, &mut files);
        for file in files {
            let src = fs::read_to_string(&file).expect("read source");
            for lit in first_literals(&src, &["i18n", "i18np", "ki18n"]) {
                map.entry(lit).or_default().insert(
                    file.strip_prefix(&root)
                        .unwrap_or(&file)
                        .display()
                        .to_string(),
                );
            }
        }
    }
    map
}

fn shared_msgids() -> BTreeSet<String> {
    let src = fs::read_to_string(workspace_root().join("i18n/src/messages.rs"))
        .expect("read i18n/src/messages.rs");
    first_literals(&src, &["i18n_noop"]).into_iter().collect()
}

/// A text of `kestic-i18n` is never repeated as an `i18n("…")` literal in daemon or GUI.
#[test]
fn shared_texts_are_not_repeated_as_literals() {
    let shared = shared_msgids();
    assert!(
        shared.len() > 50,
        "expected the shared msgids, got {}",
        shared.len()
    );
    let literals = literals_in(&[
        ("daemon/src", "rs"),
        ("gui/src", "rs"),
        ("gui/src/qml", "qml"),
    ]);
    assert!(
        literals.len() > 100,
        "expected the UI literals, got {}",
        literals.len()
    );
    let repeated: Vec<_> = literals
        .iter()
        .filter(|(msgid, _)| shared.contains(*msgid))
        .collect();
    assert!(
        repeated.is_empty(),
        "use the kestic-i18n function instead of these literals: {repeated:#?}"
    );
}

/// A text that daemon and GUI both need lives in `kestic-i18n`, not as a literal on both sides.
#[test]
fn texts_used_by_daemon_and_gui_live_in_the_shared_crate() {
    let daemon = literals_in(&[("daemon/src", "rs")]);
    let gui = literals_in(&[("gui/src", "rs"), ("gui/src/qml", "qml")]);
    assert!(
        !daemon.is_empty() && !gui.is_empty(),
        "expected literals on both sides"
    );
    let both: Vec<_> = daemon
        .keys()
        .filter(|msgid| gui.contains_key(*msgid))
        .collect();
    assert!(
        both.is_empty(),
        "move these texts to kestic-i18n (daemon and GUI both use them): {both:#?}"
    );
}
