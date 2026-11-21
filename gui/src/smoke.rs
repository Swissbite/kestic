// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX bridge to the offscreen QML smoke scenes (only built with the `test-harness` feature).

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        include!("src/smoke_helper.h");
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
