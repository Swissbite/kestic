// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Every text in `po/de/kestic.po`, English msgid and German translation, is written in
//! simple language (`docs/spec/dbus-api.md` §Texte und Übersetzungen): short sentences and no
//! technical words, except a few spec terms and expert texts listed with their reason. Also
//! writes the whole catalog as a review list to `target/smoke-artifacts/texts-en-de.md`.

mod common;

use common::language;
use kestic_test_support::po::catalog;
use kestic_test_support::workspace::workspace_root;

use std::fmt::Write as _;
use std::fs;

#[test]
fn every_text_has_short_sentences() {
    let entries = catalog(&workspace_root());
    assert!(
        entries.len() > 500,
        "expected the full catalog, got {}",
        entries.len()
    );
    let long: Vec<String> = entries
        .iter()
        .flat_map(|e| e.texts.iter())
        .flat_map(|t| language::long_sentences(t))
        .collect();
    assert!(
        long.is_empty(),
        "sentences longer than {} words: {long:#?}",
        language::MAX_WORDS_PER_SENTENCE
    );
}

#[test]
fn no_text_uses_technical_words() {
    let found: Vec<String> = catalog(&workspace_root())
        .iter()
        .flat_map(|e| e.texts.iter())
        .flat_map(|t| {
            language::jargon(t)
                .into_iter()
                .map(move |w| format!("{w}: {t}"))
        })
        .collect();
    assert!(
        found.is_empty(),
        "technical words in user texts: {found:#?}"
    );
}

/// The checks see a planted long sentence and a planted technical word.
#[test]
fn checks_find_planted_problems() {
    let long = (0..21).map(|_| "word").collect::<Vec<_>>().join(" ");
    assert_eq!(language::long_sentences(&long).len(), 1);
    assert_eq!(
        language::jargon("restic failed with stderr output"),
        vec!["stderr"]
    );
}

/// Review list of every catalog text (English, German), rebuilt on each run.
#[test]
fn writes_the_review_list_of_all_texts() {
    let entries = catalog(&workspace_root());
    let mut md =
        String::from("# Kestic texts (English / German)\n\n| English | German |\n| --- | --- |\n");
    for e in &entries {
        let cell = |v: &[String]| v.join(" / ").replace('\n', "<br>").replace('|', "\\|");
        writeln!(md, "| {} | {} |", cell(e.english()), cell(e.german())).expect("write row");
    }
    let dir = workspace_root().join("target/smoke-artifacts");
    fs::create_dir_all(&dir).expect("mkdir smoke-artifacts");
    fs::write(dir.join("texts-en-de.md"), md).expect("write review list");
    assert!(entries.iter().all(|e| !e.msgid().is_empty()));
    assert!(entries.iter().all(|e| !e.german().is_empty()));
}
