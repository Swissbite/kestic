// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX bridge to the Qt helpers of the product: clipboard, KI18n substitution, local UTC offset
//! and global keyboard handling.

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        include!("src/qt_bridge.h");
        #[namespace = "kestic::bridge"]
        fn copy_to_clipboard(text: &str);

        #[namespace = "kestic::bridge"]
        fn i18n_subs(msgid: &str, args: &[&str]) -> String;

        #[namespace = "kestic::bridge"]
        fn i18np_subs(singular: &str, plural: &str, n: i64, args: &[&str]) -> String;

        #[namespace = "kestic::bridge"]
        fn local_offset_seconds(unix_secs: i64) -> i32;

        #[namespace = "kestic::bridge"]
        fn install_keyboard_handlers(engine: Pin<&mut QQmlApplicationEngine>);
    }
}
