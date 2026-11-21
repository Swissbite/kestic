// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Every text has one owner (`docs/spec/dbus-api.md` §Texte und Übersetzungen): texts that
//! daemon and GUI both need live in `kestic-i18n`, texts that only one side needs stay there,
//! and no msgid is written in more than one production file. QML asks for a text used in
//! several places by code (`Texts.label("close")`). Reads the source files only.

mod common;

use kestic_test_support::workspace::{files_with_ext, workspace_root};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

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

/// Translating calls and the position of their msgid among the string arguments
/// (`i18nc` and `i18ncp` start with a context).
const TRANSLATING_CALLS: &[(&str, usize)] = &[
    ("i18n", 0),
    ("i18np", 0),
    ("i18nc", 1),
    ("i18ncp", 1),
    ("ki18n", 0),
    ("i18n_noop", 0),
];

/// The `index`-th string argument of the call whose arguments start at `src[from..]`, if the
/// arguments up to it are string literals.
fn string_argument(src: &str, from: usize, index: usize) -> Option<String> {
    let mut at = from;
    for n in 0..=index {
        let rest = &src[at..];
        let trimmed = rest.trim_start();
        let trimmed = if n > 0 {
            trimmed.strip_prefix(',')?.trim_start()
        } else {
            trimmed
        };
        let stripped = trimmed.strip_prefix('"')?;
        let start = src.len() - stripped.len();
        let lit = literal_at(src, start)?;
        if n == index {
            return Some(lit);
        }
        at = start + raw_literal_len(&src[start..]) + 1;
    }
    None
}

/// Length of the raw literal text up to (not including) its closing quote.
fn raw_literal_len(s: &str) -> usize {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return i,
            _ => {}
        }
    }
    s.len()
}

/// Msgids of every translating call in `src`.
fn msgid_literals(src: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (function, index) in TRANSLATING_CALLS {
        let pattern = format!("{function}(");
        let mut from = 0;
        while let Some(pos) = src[from..].find(&pattern) {
            let at = from + pos;
            from = at + pattern.len();
            let preceded_by_ident = src[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
            if !preceded_by_ident {
                found.extend(string_argument(src, from, *index));
            }
        }
    }
    found
}

/// Production source of a file: unit tests at the end of a Rust file are left out.
fn production_source(file: &Path) -> String {
    let src = fs::read_to_string(file).expect("read source");
    if file.extension().is_some_and(|e| e == "rs") {
        src.split("#[cfg(test)]").next().unwrap_or("").to_string()
    } else {
        src
    }
}

fn relative(file: &Path) -> String {
    let root = workspace_root();
    file.strip_prefix(&root)
        .unwrap_or(file)
        .display()
        .to_string()
}

/// Production files below `dirs` (unit-test files `*_tests.rs` left out).
fn production_files(dirs: &[(&str, &str)]) -> Vec<PathBuf> {
    let root = workspace_root();
    let mut files = Vec::new();
    for (dir, ext) in dirs {
        files.extend(files_with_ext(&root.join(dir), ext));
    }
    files.retain(|f| !f.to_string_lossy().ends_with("_tests.rs"));
    files.sort();
    files.dedup();
    files
}

/// msgid → production files that write it as a literal of a translating call.
fn literals_in(dirs: &[(&str, &str)]) -> BTreeMap<String, BTreeSet<String>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in production_files(dirs) {
        for lit in msgid_literals(&production_source(&file)) {
            map.entry(lit).or_default().insert(relative(&file));
        }
    }
    map
}

const ALL_SOURCES: &[(&str, &str)] = &[
    ("daemon/src", "rs"),
    ("gui/src", "rs"),
    ("gui/src/qml", "qml"),
    ("i18n/src", "rs"),
];

fn shared_msgids() -> BTreeSet<String> {
    let src = fs::read_to_string(workspace_root().join("i18n/src/messages.rs"))
        .expect("read i18n/src/messages.rs");
    msgid_literals(&src).into_iter().collect()
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
    assert!(!daemon.is_empty(), "expected daemon literals");
    assert!(!gui.is_empty(), "expected GUI literals");
    let both: Vec<_> = daemon
        .keys()
        .filter(|msgid| gui.contains_key(*msgid))
        .collect();
    assert!(
        both.is_empty(),
        "move these texts to kestic-i18n (daemon and GUI both use them): {both:#?}"
    );
}

/// No msgid is written in more than one production file (daemon, GUI incl. QML, `kestic-i18n`).
#[test]
fn no_text_is_written_in_more_than_one_file() {
    let literals = literals_in(ALL_SOURCES);
    assert!(
        literals.len() > 500,
        "expected all texts, got {}",
        literals.len()
    );
    let repeated: Vec<_> = literals
        .iter()
        .filter(|(_, files)| files.len() > 1)
        .collect();
    assert!(
        repeated.is_empty(),
        "write each text once (shared GUI texts: gui/src/ui_texts.rs): {repeated:#?}"
    );
}

/// Codes of `gui/src/ui_texts.rs` (`Variant = "code" => i18n_noop("…")`) and their variants.
fn gui_text_codes() -> BTreeMap<String, String> {
    let src = fs::read_to_string(workspace_root().join("gui/src/ui_texts.rs"))
        .expect("read gui/src/ui_texts.rs");
    src.lines()
        .filter_map(|line| {
            let (variant, rest) = line.trim().split_once(" = \"")?;
            let (code, _) = rest.split_once("\" => i18n_noop(")?;
            Some((code.to_string(), variant.to_string()))
        })
        .collect()
}

/// Codes passed to `Texts.label(…)` / `Texts.labelArgs(…)` in a QML source.
fn qml_text_codes(src: &str) -> Vec<String> {
    let mut codes = Vec::new();
    for pattern in ["Texts.label(\"", "Texts.labelArgs(\""] {
        let mut from = 0;
        while let Some(pos) = src[from..].find(pattern) {
            let start = from + pos + pattern.len();
            from = start;
            codes.extend(literal_at(src, start));
        }
    }
    codes
}

/// Every code that QML asks for exists (a GUI text or a label shared with the tray), and every
/// GUI text is used.
#[test]
fn every_text_code_exists_and_is_used() {
    let gui_texts = gui_text_codes();
    assert!(
        gui_texts.len() > 50,
        "expected the GUI texts, got {}",
        gui_texts.len()
    );
    let labels: BTreeSet<String> = kestic_i18n::messages::Label::ALL
        .iter()
        .map(|l| l.as_str().to_string())
        .collect();
    let mut used = BTreeSet::new();
    let mut unknown = Vec::new();
    for file in production_files(&[("gui/src/qml", "qml")]) {
        for code in qml_text_codes(&production_source(&file)) {
            if !gui_texts.contains_key(&code) && !labels.contains(&code) {
                unknown.push(format!("{}: {code}", relative(&file)));
            }
            used.insert(code);
        }
    }
    assert!(unknown.is_empty(), "unknown text codes: {unknown:#?}");
    let rust: String = production_files(&[("gui/src", "rs")])
        .iter()
        .map(|f| production_source(f))
        .collect();
    let unused: Vec<_> = gui_texts
        .iter()
        .filter(|(code, variant)| {
            !used.contains(*code) && !rust.contains(&format!("GuiText::{variant}"))
        })
        .collect();
    assert!(unused.is_empty(), "unused GUI texts: {unused:#?}");
}

/// The scanner finds the msgid of every translating call, also after a context.
#[test]
fn scanner_finds_msgids_of_all_translating_calls() {
    let src = r#"i18n("A") i18nc("ctx", "B") i18np("C", "Cs", n) ki18n("D") i18n_noop("E \"q\"") xi18n("F")"#;
    let found: BTreeSet<String> = msgid_literals(src).into_iter().collect();
    let expected: BTreeSet<String> = ["A", "B", "C", "D", "E \"q\""]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(found, expected);
    assert_eq!(
        qml_text_codes(r#"Texts.label("close") Texts.labelArgs("two_parts", [a, b])"#),
        vec!["close", "two_parts"]
    );
}
