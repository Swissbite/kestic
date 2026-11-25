// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! A small reader for the gettext catalog `po/de/kestic.po`: its entries with their English and
//! German forms, for tests that check or use the translations.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// One catalog entry: msgid, for a plural entry msgid_plural, then the German forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoEntry {
    pub texts: Vec<String>,
    pub plural: bool,
}

impl PoEntry {
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

/// The text of a quoted PO string (`"…"`), with `\"`, `\\`, `\n` and `\t` resolved.
fn unquote(quoted: &str) -> String {
    let inner = quoted
        .trim()
        .strip_prefix('"')
        .and_then(|l| l.strip_suffix('"'))
        .unwrap_or_else(|| panic!("malformed PO string: {quoted}"));
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// The entries of the PO text `text` in file order, without the header (the entry with an
/// empty msgid).
pub fn parse_po(text: &str) -> Vec<PoEntry> {
    let mut out = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let mut plural = false;
    let mut flush = |texts: &mut Vec<String>, plural: &mut bool| {
        if texts.first().is_some_and(|m| !m.is_empty()) {
            out.push(PoEntry {
                texts: std::mem::take(texts),
                plural: *plural,
            });
        }
        texts.clear();
        *plural = false;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush(&mut texts, &mut plural);
        } else if line.starts_with('#') {
            continue;
        } else if line.starts_with('"') {
            if let Some(last) = texts.last_mut() {
                last.push_str(&unquote(line));
            }
        } else if let Some((key, rest)) = line.split_once(' ') {
            plural |= key == "msgid_plural";
            texts.push(unquote(rest));
        }
    }
    flush(&mut texts, &mut plural);
    out
}

/// All entries of `po/de/kestic.po`; `workspace` is the workspace root.
pub fn catalog(workspace: &Path) -> Vec<PoEntry> {
    let text = fs::read_to_string(workspace.join("po/de/kestic.po")).expect("read po/de/kestic.po");
    parse_po(&text)
}

/// msgid → German text of `entries`; of a plural entry, the msgid maps to the first German form
/// and the plural to the second.
pub fn translations(entries: &[PoEntry]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entry in entries {
        for (english, german) in entry.english().iter().zip(entry.german()) {
            map.insert(english.clone(), german.clone());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_singular_plural_and_continued_entries_without_the_header() {
        let text = concat!(
            "msgid \"\"\n",
            "msgstr \"\"\n",
            "\"Language: de\\n\"\n",
            "\n",
            "#: a.rs:1\n",
            "msgid \"Say \\\"hi\\\"\"\n",
            "msgstr \"Sag \"\n",
            "\"\\\"hallo\\\"\"\n",
            "\n",
            "msgid \"%1 file\"\n",
            "msgid_plural \"%1 files\"\n",
            "msgstr[0] \"%1 Datei\"\n",
            "msgstr[1] \"%1 Dateien\"\n",
        );
        let entries = parse_po(text);
        assert_eq!(entries.len(), 2, "the header is left out");
        assert_eq!(entries[0].msgid(), "Say \"hi\"");
        assert_eq!(entries[0].german(), ["Sag \"hallo\""]);
        assert!(entries[1].plural);
        assert_eq!(entries[1].english(), ["%1 file", "%1 files"]);
        assert_eq!(entries[1].german(), ["%1 Datei", "%1 Dateien"]);
        let map = translations(&entries);
        assert_eq!(map["%1 files"], "%1 Dateien");
    }
}
