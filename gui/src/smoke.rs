// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! CXX bridge to the Qt/QML offscreen smoke-check helper.

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        include!("src/smoke_helper.h");
        #[namespace = "kestic::smoke"]
        fn install_smoke_check(
            engine: Pin<&mut QQmlApplicationEngine>,
            screenshot_path: &str,
            expected_list_count: i32,
            layout_mode: &str,
            trigger_shortcut: bool,
        );
    }
}
