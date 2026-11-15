// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Kestic GUI and CLI entrypoint.

mod list_model;
mod smoke;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};
use cxx_qt_lib_extras::QApplication;
use std::env;
use std::process::ExitCode;

fn parse_smoke_args() -> Option<String> {
    let mut args = env::args().skip(1);
    let mut smoke = false;
    let mut screenshot = String::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke-test" => smoke = true,
            "--screenshot" => {
                screenshot = args.next().unwrap_or_default();
            }
            _ => {}
        }
    }
    smoke.then_some(screenshot)
}

fn main() -> ExitCode {
    let _ = kestic_daemon::daemon_version();
    cxx_qt::init_crate!(cxx_qt);
    cxx_qt::init_qml_module!("net.daester.kestic");
    let smoke_screenshot = parse_smoke_args();

    let mut app = QApplication::new();
    QGuiApplication::set_desktop_file_name(&QString::from("net.daester.Kestic"));
    if env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        QQuickStyle::set_style(&QString::from("org.kde.desktop"));
    }

    let mut engine = QQmlApplicationEngine::new();
    if let (Some(engine_pin), Some(screenshot_path)) =
        (engine.as_mut(), smoke_screenshot.as_deref())
    {
        smoke::ffi::install_smoke_check(engine_pin, screenshot_path, 3);
    }

    if let Some(engine_pin) = engine.as_mut() {
        engine_pin.load(&QUrl::from(
            "qrc:/qt/qml/net/daester/kestic/src/qml/Main.qml",
        ));
    }

    let code = if let Some(app_pin) = app.as_mut() {
        app_pin.exec()
    } else {
        1
    };

    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
