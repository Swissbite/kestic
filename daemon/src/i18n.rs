// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Daemon localization abstraction (`NFR-06`, `AC-M1-16-2`).
//!
//! Uses `i18n("...")` call sites so GNU `xgettext --keyword=i18n:1` extracts every
//! daemon tray, notification, and error string into `po/de/kestic.po`, while
//! allowing `gui` (`cxx-kde-frameworks`) or test doubles to supply translations.

use kestic_core::messages::{Message, UiText, substitute_placeholders};
use std::sync::{Arc, OnceLock, RwLock};

use std::fmt;

/// Translation backend invoked by [`i18n`].
pub trait DaemonTranslator: Send + Sync {
    fn translate(&self, msgid: &str) -> String;

    fn translate_subs(&self, msgid: &str, args: &[&str]) -> String {
        let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        substitute_placeholders(&self.translate(msgid), &args)
    }
}

/// Identity fallback translator returning the English source string verbatim.
#[derive(Debug, Default)]
pub struct PassthroughTranslator;

impl DaemonTranslator for PassthroughTranslator {
    fn translate(&self, msgid: &str) -> String {
        msgid.to_string()
    }
}

/// Callback-backed translator used when only single-string lookup is supplied.
pub struct CallbackTranslator<F>(pub F);

impl<F> DaemonTranslator for CallbackTranslator<F>
where
    F: Fn(&str) -> String + Send + Sync,
{
    fn translate(&self, msgid: &str) -> String {
        (self.0)(msgid)
    }
}

/// Callback-backed translator used by `kestic daemon` to delegate both plain
/// lookups and `.subs(...)` formatting directly to KI18n.
pub struct CallbackSubsTranslator<F0, F1> {
    pub translate_fn: F0,
    pub translate_subs_fn: F1,
}

impl<F0, F1> DaemonTranslator for CallbackSubsTranslator<F0, F1>
where
    F0: Fn(&str) -> String + Send + Sync,
    F1: Fn(&str, &[&str]) -> String + Send + Sync,
{
    fn translate(&self, msgid: &str) -> String {
        (self.translate_fn)(msgid)
    }

    fn translate_subs(&self, msgid: &str, args: &[&str]) -> String {
        (self.translate_subs_fn)(msgid, args)
    }
}

static TRANSLATOR: OnceLock<RwLock<Arc<dyn DaemonTranslator>>> = OnceLock::new();

fn translator_lock() -> &'static RwLock<Arc<dyn DaemonTranslator>> {
    TRANSLATOR.get_or_init(|| RwLock::new(Arc::new(PassthroughTranslator)))
}

/// Installs the active [`DaemonTranslator`] for this daemon process.
pub fn set_daemon_translator(translator: Arc<dyn DaemonTranslator>) {
    let mut guard = translator_lock()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = translator;
}

/// Deferred translation handle returned by [`i18n`].
///
/// Mirrors the GUI `PendingI18n` wrapper so GNU `xgettext --keyword=i18n:1`
/// extracts every msgid while placeholders are passed natively via `.subs(...)`.
#[derive(Clone, Copy, Debug)]
pub struct PendingI18n(pub &'static str);

impl PendingI18n {
    pub fn subs(self, args: &[&str]) -> String {
        let guard = translator_lock()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.translate_subs(self.0, args)
    }
}

impl fmt::Display for PendingI18n {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let guard = translator_lock()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f.write_str(&guard.translate(self.0))
    }
}

impl From<PendingI18n> for String {
    fn from(value: PendingI18n) -> Self {
        value.to_string()
    }
}

/// Translates `msgid` using the currently installed daemon translator.
///
/// Named `i18n` so `xgettext --keyword=i18n:1` extracts all daemon strings alongside GUI code.
pub fn i18n(msgid: &'static str) -> PendingI18n {
    PendingI18n(msgid)
}

/// Translates a `core` [`Message`] (code → msgid mapping in `kestic_core::messages`) in the
/// daemon language (tray, notifications, run history).
pub fn tr(message: &Message) -> String {
    let guard = translator_lock()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if message.args.is_empty() {
        guard.translate(message.msgid)
    } else {
        let args: Vec<&str> = message.args.iter().map(String::as_str).collect();
        guard.translate_subs(message.msgid, &args)
    }
}

/// Translates a [`UiText`]: messages via [`tr`], raw detail unchanged.
pub fn tr_text(text: &UiText) -> String {
    match text {
        UiText::Message(m) => tr(m),
        UiText::Raw(raw) => raw.clone(),
    }
}
