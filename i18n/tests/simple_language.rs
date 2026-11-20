// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Every text in `po/de/kestic.po`, English msgid and German translation, is written in
//! simple language (`docs/spec/dbus-api.md` §Texte und Übersetzungen): short sentences and no
//! technical words, except a few spec terms and expert texts listed with their reason.

use std::fs;
use std::path::Path;

/// Longest allowed sentence, in words.
const MAX_WORDS_PER_SENTENCE: usize = 20;

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

/// Exceptions: (word, msgid that may use it, reason).
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
];

struct Entry {
    msgid: String,
    texts: Vec<String>,
}

fn literal(line: &str) -> String {
    let inner = line
        .trim()
        .strip_prefix('"')
        .and_then(|l| l.strip_suffix('"'))
        .unwrap_or_else(|| panic!("malformed PO line: {line}"));
    inner.replace("\\n", "\n").replace("\\\"", "\"")
}

/// All entries with their msgid, msgid_plural and msgstr texts.
fn entries() -> Vec<Entry> {
    let po = Path::new(env!("CARGO_MANIFEST_DIR")).join("../po/de/kestic.po");
    let text = fs::read_to_string(po).expect("read po/de/kestic.po");
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let mut texts: Vec<String> = Vec::new();
        for line in block.lines() {
            if line.starts_with('#') {
                continue;
            }
            if line.starts_with('"') {
                if let Some(last) = texts.last_mut() {
                    last.push_str(&literal(line));
                }
            } else if let Some((_, rest)) = line.split_once(' ') {
                texts.push(literal(rest));
            }
        }
        if let Some(msgid) = texts.first().cloned()
            && !msgid.is_empty()
        {
            out.push(Entry { msgid, texts });
        }
    }
    out
}

fn sentences(text: &str) -> Vec<&str> {
    text.split(['.', '!', '?', ';', ':', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

fn words(sentence: &str) -> usize {
    sentence
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
}

#[test]
fn every_text_has_short_sentences() {
    let entries = entries();
    assert!(
        entries.len() > 500,
        "expected the full catalog, got {}",
        entries.len()
    );
    let mut long = Vec::new();
    for entry in &entries {
        for text in &entry.texts {
            for sentence in sentences(text) {
                if words(sentence) > MAX_WORDS_PER_SENTENCE {
                    long.push(sentence.to_string());
                }
            }
        }
    }
    assert!(
        long.is_empty(),
        "sentences longer than {MAX_WORDS_PER_SENTENCE} words: {long:#?}"
    );
}

#[test]
fn no_text_uses_technical_words() {
    let mut found = Vec::new();
    for entry in entries() {
        for text in &entry.texts {
            let lower = format!(" {} ", text.to_lowercase());
            for word in JARGON {
                let hit = lower.match_indices(word).any(|(i, _)| {
                    let before = lower[..i].chars().next_back().unwrap_or(' ');
                    let after = lower[i + word.len()..].chars().next().unwrap_or(' ');
                    !before.is_alphanumeric() && !after.is_alphanumeric()
                });
                let allowed = ALLOWED
                    .iter()
                    .any(|(w, msgid, _)| w == word && *msgid == entry.msgid);
                if hit && !allowed {
                    found.push(format!("{word}: {text}"));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "technical words in user texts: {found:#?}"
    );
}

/// The test sees a technical word when one is there (guards against a broken parser).
#[test]
fn jargon_check_finds_a_planted_word() {
    let lower = " restic failed with stderr output ";
    assert!(JARGON.iter().any(|w| lower.contains(&format!(" {w} "))));
}
