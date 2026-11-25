// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! A daemon translator backed by the German catalog `po/de/kestic.po`, for GUI smoke scenes in
//! German: the in-process test daemon has no KI18n, so without it its texts (e.g. the suggested
//! folder «~/Wiederhergestellt/…») stay English while the window is German.

use kestic_daemon::i18n::{DaemonTranslator, PassthroughTranslator, set_daemon_translator};
use kestic_test_support::po::{catalog, translations};
use kestic_test_support::workspace::workspace_root;
use std::collections::HashMap;
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

/// Makes the daemon of this test process speak `lang`: German from `po/de/kestic.po`, any
/// other language English (the msgids).
pub fn set_daemon_language(lang: &str) {
    if lang == "de" {
        set_daemon_translator(Arc::new(PoTranslator {
            messages: translations(&catalog(&workspace_root())),
        }));
    } else {
        set_daemon_translator(Arc::new(PassthroughTranslator));
    }
}
