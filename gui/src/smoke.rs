// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX bridge to Qt/QML offscreen smoke-check helpers and clipboard access.

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        include!("src/smoke_helper.h");
        #[namespace = "kestic::smoke"]
        fn copy_to_clipboard(text: &str);

        #[namespace = "kestic::smoke"]
        fn i18n_subs(msgid: &str, args: &[&str]) -> String;

        #[namespace = "kestic::smoke"]
        fn i18np_subs(singular: &str, plural: &str, n: i64, args: &[&str]) -> String;

        #[namespace = "kestic::smoke"]
        fn local_offset_seconds(unix_secs: i64) -> i32;

        #[namespace = "kestic::smoke"]
        fn install_keyboard_handlers(engine: Pin<&mut QQmlApplicationEngine>);

        #[namespace = "kestic::smoke"]
        fn install_m1_smoke_check(
            engine: Pin<&mut QQmlApplicationEngine>,
            scenario: &str,
            screenshot_path: &str,
            layout_mode: &str,
            expected_lang: &str,
        );
    }
}
