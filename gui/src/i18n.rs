// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Translation through KI18n: plain and plural texts, shared messages, percentages and the
//! text of a failed daemon call.

use kestic_i18n::codes::ServiceErrorCode;
use kestic_i18n::messages::{Message, PERCENT_PATTERN, UiText};

pub struct PendingI18n<'a>(pub &'a str);

impl PendingI18n<'_> {
    pub fn subs(self, args: &[&str]) -> String {
        crate::qt_bridge::ffi::i18n_subs(self.0, args)
    }
}

impl std::fmt::Display for PendingI18n<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&cxx_kde_frameworks::ki18n::i18n(self.0).to_string())
    }
}

pub fn i18n(text: &str) -> PendingI18n<'_> {
    PendingI18n(text)
}

/// Plural-aware translation (`i18np`): `%1` is `n`, further `%N` come from `args`. A count
/// beyond `i64` is shown as `i64::MAX`.
pub fn i18np(singular: &str, plural: &str, n: impl TryInto<i64>, args: &[&str]) -> String {
    crate::qt_bridge::ffi::i18np_subs(singular, plural, n.try_into().unwrap_or(i64::MAX), args)
}

/// Translates a `kestic-i18n` [`Message`] (code → msgid mapping in `kestic_i18n::messages`)
/// in the GUI language with KI18n.
pub fn tr(message: &Message) -> String {
    if message.args.is_empty() {
        i18n(message.msgid).to_string()
    } else {
        let args = message.rendered_args(format_percent);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        i18n(message.msgid).subs(&args)
    }
}

/// `percent` in the GUI language: the number from `QLocale`, the place of the percent sign and
/// the space before it from the catalog («25%», German «25 %»); the one formatting path for
/// percentages in the GUI.
pub fn format_percent(percent: u8) -> String {
    i18n(PERCENT_PATTERN).subs(&[&crate::qt_bridge::ffi::format_count(i64::from(percent))])
}

/// Translates a [`UiText`]: messages via [`tr`], raw detail unchanged.
pub fn tr_text(text: &UiText) -> String {
    match text {
        UiText::Message(m) => tr(m),
        UiText::Raw(raw) => raw.clone(),
    }
}

/// Text of a failed D-Bus call in the GUI language: the error name is the code
/// (`kestic_i18n::messages`), the error message is never shown as main text.
pub fn dbus_error_text<E>(err: &E) -> String
where
    E: Clone + Into<kestic_daemon::dbus_api::KesticDbusError>,
{
    let err: kestic_daemon::dbus_api::KesticDbusError = err.clone().into();
    tr(&err.code().message())
}

/// «Background service unavailable.» (text of `ServiceErrorCode::DaemonUnreachable`).
pub fn daemon_unavailable_text() -> String {
    tr(&ServiceErrorCode::DaemonUnreachable.message())
}

/// The requested item no longer exists (text of `ServiceErrorCode::NotFound`).
pub fn not_found_text() -> String {
    tr(&ServiceErrorCode::NotFound.message())
}

#[cfg(test)]
#[path = "i18n_tests.rs"]
mod tests;
