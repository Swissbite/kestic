// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Daemon localization abstraction (`NFR-06`, `AC-M1-16-2`).
//!
//! Uses `i18n("...")` call sites so GNU `xgettext --keyword=i18n:1` extracts every
//! daemon tray, notification, and error string into `po/de/kestic.po`, while
//! allowing `gui` (`cxx-kde-frameworks`) or test doubles to supply translations.

use kestic_core::restic::{RecoveryAction, ResticFailureKind};
use std::sync::{Arc, OnceLock, RwLock};

use std::fmt;

/// Single-pass `%1`..`%9` placeholder substitution for fallback translators.
fn substitute_placeholders(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 1] != b'0'
        {
            let idx = (bytes[i + 1] - b'1') as usize;
            if let Some(arg) = args.get(idx) {
                out.push_str(arg);
                i += 2;
                continue;
            }
        }
        let ch = template[i..].chars().next().expect("valid utf8 boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Translation backend invoked by [`i18n`].
pub trait DaemonTranslator: Send + Sync {
    fn translate(&self, msgid: &str) -> String;

    fn translate_subs(&self, msgid: &str, args: &[&str]) -> String {
        substitute_placeholders(&self.translate(msgid), args)
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

/// Localized single-sentence summary for a classified restic failure (`FR-37`, `M1-S15`).
pub fn localized_failure_summary(kind: &ResticFailureKind) -> String {
    match kind {
        ResticFailureKind::None => i18n("Completed").into(),
        ResticFailureKind::Cancelled => i18n("Cancelled by user").into(),
        ResticFailureKind::IncompleteSnapshot { .. } => {
            i18n("Snapshot incomplete: some files could not be read").into()
        }
        ResticFailureKind::DestinationUnreachable => i18n("Destination unreachable").into(),
        ResticFailureKind::DnsResolutionFailed => {
            i18n("Destination hostname could not be resolved (DNS)").into()
        }
        ResticFailureKind::BackendAuthFailed => i18n("Authentication at destination failed").into(),
        ResticFailureKind::GenericFailure => i18n("Backup command failed").into(),
        ResticFailureKind::RepositoryMissing => i18n("Repository is missing").into(),
        ResticFailureKind::RepositoryLocked => i18n("Repository is locked").into(),
        ResticFailureKind::WrongPassword => i18n("Stored password does not match").into(),
        ResticFailureKind::Unknown { exit_code } => {
            i18n("Unknown error (exit %1)").subs(&[&exit_code.to_string()])
        }
    }
}

/// Localized label for a [`RecoveryAction`] (`FR-37`).
pub fn localized_recovery_action_label(action: RecoveryAction) -> String {
    match action {
        RecoveryAction::UpdateStoredPassword => i18n("Update Saved Password…").into(),
        RecoveryAction::AddToExcludes => i18n("Add to Exclude Rules…").into(),
        RecoveryAction::CheckResticSource => i18n("Configure restic Source…").into(),
        RecoveryAction::UnlockRepository => i18n("Release Lock…").into(),
    }
}
