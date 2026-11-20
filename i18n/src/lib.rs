// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Texts and codes shared by the Kestic daemon and GUI (`docs/spec/dbus-api.md`
//! §Sprachunabhängige Codes): the language-independent codes that travel over D-Bus
//! ([`codes`]) and the single mapping from codes to English msgids ([`messages`]).
//!
//! This crate never translates. The daemon translates with its own i18n (tray,
//! notifications, run history), the GUI with KI18n; both read `po/de/kestic.po`.
//! Texts that only one side needs stay on that side.

pub mod codes;
pub mod messages;
