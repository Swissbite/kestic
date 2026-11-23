// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! A daemon translator backed by the German catalog `po/de/kestic.po`, for GUI smoke scenes in
//! German: the in-process test daemon has no KI18n, so without it its texts (e.g. the suggested
//! folder «~/Wiederhergestellt/…») stay English while the window is German.

use kestic_daemon::i18n::{DaemonTranslator, PassthroughTranslator, set_daemon_translator};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

struct PoTranslator {
    messages: HashMap<String, String>,
}

impl DaemonTranslator for PoTranslator {
    fn translate(&self, msgid: &str) -> String {
        self.messages
            .get(msgid)
            .filter(|t| !t.is_empty())
            .cloned()
            .unwrap_or_else(|| msgid.to_string())
    }
}

/// The text of a quoted PO string line (`"…"`), with `\"`, `\\`, `\n` and `\t` resolved.
fn unquote(line: &str) -> String {
    let inner = line
        .trim()
        .strip_prefix('"')
        .and_then(|l| l.strip_suffix('"'))
        .unwrap_or_default();
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

/// msgid → msgstr of a PO catalog; for a plural entry the singular maps to `msgstr[0]` and the
/// plural to `msgstr[1]`.
fn parse_po(text: &str) -> HashMap<String, String> {
    let mut messages = HashMap::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    let flush = |fields: &mut Vec<(String, String)>, messages: &mut HashMap<String, String>| {
        let get = |key: &str| {
            fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        };
        if let Some(id) = get("msgid") {
            if let Some(plural) = get("msgid_plural") {
                messages.insert(id, get("msgstr[0]").unwrap_or_default());
                messages.insert(plural, get("msgstr[1]").unwrap_or_default());
            } else {
                messages.insert(id, get("msgstr").unwrap_or_default());
            }
        }
        fields.clear();
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush(&mut fields, &mut messages);
        } else if line.starts_with('#') {
            continue;
        } else if line.starts_with('"') {
            if let Some((_, value)) = fields.last_mut() {
                value.push_str(&unquote(line));
            }
        } else if let Some((key, rest)) = line.split_once(' ') {
            fields.push((key.to_string(), unquote(rest)));
        }
    }
    flush(&mut fields, &mut messages);
    messages
}

/// Makes the daemon of this test process speak `lang`: German from `po/de/kestic.po`, any
/// other language English (the msgids).
pub fn set_daemon_language(lang: &str) {
    if lang == "de" {
        let po = Path::new(env!("CARGO_MANIFEST_DIR")).join("../po/de/kestic.po");
        let text = std::fs::read_to_string(&po).expect("read po/de/kestic.po");
        set_daemon_translator(Arc::new(PoTranslator {
            messages: parse_po(&text),
        }));
    } else {
        set_daemon_translator(Arc::new(PassthroughTranslator));
    }
}
