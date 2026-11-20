// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared checks for simple language (`docs/spec/dbus-api.md` §Texte und Übersetzungen), used
//! by `i18n/tests/simple_language.rs` (whole catalog) and `daemon/tests/m3_gui_smoke.rs`
//! (texts visible on every page).

use std::fs;
use std::path::Path;

/// Longest allowed sentence, in words.
pub const MAX_WORDS_PER_SENTENCE: usize = 20;

/// Technical words that a user should not need to know (lower case, whole words).
const JARGON: &[&str] = &[
    "daemon",
    "d-bus",
    "dbus",
    "pack",
    "packs",
    "slot",
    "slots",
    "cooldown",
    "execute",
    "executed",
    "executable",
    "unreferenced",
    "redundantly",
    "plaintext",
    "plain text",
    "invoke",
    "argv",
    "stderr",
    "stdout",
    "json",
    "dry-run",
    "dry run",
    "forget",
    "prune",
    "regex",
    "globs",
    "process",
    "trockenlauf",
    "authentifizierung",
    "klartext",
    "prozess",
    "referenziert",
];

/// Exceptions: (word, text that may use it, reason).
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "regex",
        "Rules in restic syntax (one per line: globs, **, !exceptions, $VAR; no regex):",
        "expert mode hint about restic syntax",
    ),
    (
        "globs",
        "Rules in restic syntax (one per line: globs, **, !exceptions, $VAR; no regex):",
        "expert mode hint about restic syntax",
    ),
    (
        "regex",
        "Regeln in restic-Syntax (eine pro Zeile: Globs, **, !Ausnahmen, $VAR; keine Regex):",
        "expert mode hint about restic syntax",
    ),
    (
        "globs",
        "Regeln in restic-Syntax (eine pro Zeile: Globs, **, !Ausnahmen, $VAR; keine Regex):",
        "expert mode hint about restic syntax",
    ),
];

/// One catalog entry: msgid, optional msgid_plural, then the German forms.
pub struct Entry {
    pub texts: Vec<String>,
    pub plural: bool,
}

impl Entry {
    pub fn msgid(&self) -> &str {
        &self.texts[0]
    }

    /// English forms (msgid and plural).
    pub fn english(&self) -> &[String] {
        &self.texts[..if self.plural { 2 } else { 1 }]
    }

    /// German forms.
    pub fn german(&self) -> &[String] {
        &self.texts[if self.plural { 2 } else { 1 }..]
    }
}

fn literal(line: &str) -> String {
    let inner = line
        .trim()
        .strip_prefix('"')
        .and_then(|l| l.strip_suffix('"'))
        .unwrap_or_else(|| panic!("malformed PO line: {line}"));
    inner.replace("\\n", "\n").replace("\\\"", "\"")
}

/// All entries of `po/de/kestic.po`; `workspace` is the workspace root.
pub fn catalog(workspace: &Path) -> Vec<Entry> {
    let text = fs::read_to_string(workspace.join("po/de/kestic.po")).expect("read po/de/kestic.po");
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let mut texts: Vec<String> = Vec::new();
        let mut plural = false;
        for line in block.lines() {
            if line.starts_with('#') {
                continue;
            }
            if line.starts_with('"') {
                if let Some(last) = texts.last_mut() {
                    last.push_str(&literal(line));
                }
            } else if let Some((key, rest)) = line.split_once(' ') {
                plural |= key == "msgid_plural";
                texts.push(literal(rest));
            }
        }
        if texts.first().is_some_and(|m| !m.is_empty()) {
            out.push(Entry { texts, plural });
        }
    }
    out
}

/// Sentences of `text` longer than [`MAX_WORDS_PER_SENTENCE`] words.
pub fn long_sentences(text: &str) -> Vec<String> {
    text.split(['.', '!', '?', ';', ':', '\n'])
        .map(str::trim)
        .filter(|s| {
            s.split_whitespace()
                .filter(|w| w.chars().any(char::is_alphanumeric))
                .count()
                > MAX_WORDS_PER_SENTENCE
        })
        .map(str::to_string)
        .collect()
}

/// Technical words in `text` that are not allowed for it.
pub fn jargon(text: &str) -> Vec<&'static str> {
    let lower = format!(" {} ", text.to_lowercase());
    JARGON
        .iter()
        .copied()
        .filter(|word| {
            lower.match_indices(word).any(|(i, _)| {
                let before = lower[..i].chars().next_back().unwrap_or(' ');
                let after = lower[i + word.len()..].chars().next().unwrap_or(' ');
                !before.is_alphanumeric() && !after.is_alphanumeric()
            })
        })
        .filter(|word| !ALLOWED.iter().any(|(w, t, _)| w == word && *t == text))
        .collect()
}
