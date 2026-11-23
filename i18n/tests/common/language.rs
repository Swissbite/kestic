// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared checks for simple language (`docs/spec/dbus-api.md` §Texte und Übersetzungen), used
//! by `i18n/tests/simple_language.rs` (whole catalog) and `daemon/tests/m3_gui_smoke.rs`
//! (texts visible on every page).

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
