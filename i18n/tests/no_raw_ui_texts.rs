// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Every text the user sees goes through the catalog (`po/de/kestic.po`): QML shows texts only
//! via `i18n(…)`, `Texts.label(…)` / `Texts.labelArgs(…)` or `appBackend.recoveryActionLabel(…)`,
//! and daemon and GUI pass no raw sentence to the tray, notifications, the window or the system.
//! Together with `simple_language.rs` (which checks every catalog entry) this covers all visible
//! texts.

mod common;

use common::workspace::{files_with_ext, workspace_root};
use std::fs;
use std::path::PathBuf;

fn files(dir: &str, ext: &str) -> Vec<PathBuf> {
    files_with_ext(&workspace_root().join(dir), ext)
}

/// A string literal and whether it is an argument of a translating call.
struct Literal {
    text: String,
    translated: bool,
    /// Compared with `===`/`!==`/`==` (a code, not a text).
    compared: bool,
}

const TRANSLATING_CALLS: &[&str] = &[
    "i18n",
    "i18nc",
    "i18np",
    "label",
    "labelArgs",
    "recoveryActionLabel",
];

/// String literals of a single-line expression with their call context.
fn literals(expr: &str) -> Vec<Literal> {
    let chars: Vec<char> = expr.chars().collect();
    let mut out = Vec::new();
    let mut calls: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '(' => {
                let name: String = chars[..i]
                    .iter()
                    .rev()
                    .take_while(|c| c.is_alphanumeric() || **c == '_')
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                calls.push(name);
            }
            ')' => {
                calls.pop();
            }
            '"' => {
                let before: String = chars[..i].iter().collect();
                let compared = before.trim_end().ends_with("==");
                let mut text = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    if i < chars.len() {
                        text.push(chars[i]);
                    }
                    i += 1;
                }
                let translated = calls
                    .iter()
                    .any(|c| TRANSLATING_CALLS.contains(&c.as_str()));
                out.push(Literal {
                    text,
                    translated,
                    compared,
                });
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Text with a word of at least two letters (paths, URLs and patterns aside).
fn has_words(text: &str) -> bool {
    text.split(|c: char| !c.is_alphabetic())
        .any(|w| w.chars().count() >= 2)
}

/// Sample values in placeholders (paths, URLs, patterns), not sentences.
fn is_sample_value(text: &str) -> bool {
    !text.contains(' ') && (text.contains('/') || text.starts_with('.') || text.starts_with('*'))
}

/// QML properties that show text.
const UI_PROPERTIES: &[&str] = &[
    "text",
    "title",
    "placeholderText",
    "tooltip",
    "subtitle",
    "description",
    "explanation",
    "label",
    "Accessible.name",
    "Accessible.description",
    "Kirigami.FormData.label",
    "Controls.ToolTip.text",
    "ToolTip.text",
    "displayText",
];

#[test]
fn qml_shows_texts_only_through_the_catalog() {
    let mut raw = Vec::new();
    let qml = files("gui/src/qml", "qml");
    assert!(qml.len() > 20, "expected the QML pages");
    for file in qml {
        let src = fs::read_to_string(&file).expect("read qml");
        for (n, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            let Some(prop) = UI_PROPERTIES
                .iter()
                .find(|p| trimmed.starts_with(&format!("{p}:")))
            else {
                continue;
            };
            let expr = &trimmed[prop.len() + 1..];
            for lit in literals(expr) {
                let allowed = lit.translated
                    || lit.compared
                    || !has_words(&lit.text)
                    || (*prop == "placeholderText" && is_sample_value(&lit.text))
                    || (*prop == "title" && lit.text == "Kestic");
                if !allowed {
                    raw.push(format!("{}:{}: {}", file.display(), n + 1, lit.text));
                }
            }
        }
    }
    assert!(raw.is_empty(), "QML texts without i18n: {raw:#?}");
}

/// Calls and fields whose text reaches the user (tray, notifications, window, system).
const RUST_SINKS: &[&str] = &[
    "send_event_notification(",
    "send_configured_notification(",
    "acquire_default(",
    "PortalInhibitGuard::acquire(",
    "JobViewHandle::request(",
    "set_last_action_error(",
    "passive_notification(",
    "label: ",
    "tooltip_description: ",
    "status_line: ",
    "info_message: ",
];

/// Arguments of a call sink up to its closing parenthesis, or the value of a field sink up to
/// the next comma or line end.
fn sink_region(rest: &str, is_call: bool) -> &str {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => return &rest[..i],
            ')' | ']' | '}' => depth -= 1,
            ',' | '\n' if depth == 0 && !is_call => return &rest[..i],
            _ => {}
        }
    }
    rest
}

#[test]
fn rust_passes_no_raw_sentence_to_the_user() {
    let mut raw = Vec::new();
    let mut sources = files("daemon/src", "rs");
    sources.extend(files("gui/src", "rs"));
    for file in sources {
        if file.to_string_lossy().ends_with("_tests.rs") {
            continue;
        }
        let src = fs::read_to_string(&file).expect("read rs");
        // Unit tests at the end of a file are not product code.
        let src = src.split("#[cfg(test)]").next().unwrap_or("");
        for sink in RUST_SINKS {
            for (at, _) in src.match_indices(sink) {
                let rest = &src[at + sink.len()..];
                let args = sink_region(rest, sink.ends_with('(')).replace('\n', " ");
                for lit in literals(&args) {
                    let allowed =
                        lit.translated || !has_words(&lit.text) || !lit.text.contains(' ');
                    if !allowed {
                        raw.push(format!("{}: {sink}{}", file.display(), lit.text));
                    }
                }
            }
        }
    }
    assert!(raw.is_empty(), "raw sentences shown to the user: {raw:#?}");
}

#[test]
fn literal_scanner_sees_translation_context() {
    let lits = literals(r#"x ? i18n("Save") : "Raw text""#);
    assert_eq!(lits.len(), 2);
    assert!(lits[0].translated);
    assert!(!lits[1].translated);
    let cmp = literals(r#"card.errorAction === "update_stored_password""#);
    assert!(cmp[0].compared);
}
