// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Kestic GUI and CLI entrypoint.

mod list_model;
mod smoke;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};
use cxx_qt_lib_extras::QApplication;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

fn has_kestic_config() -> bool {
    let config_root = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    config_root.is_some_and(|dir| dir.join("kestic").join("config.toml").is_file())
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find_map(|pair| (pair[0] == flag).then(|| pair[1].clone()))
}

fn run_spike_watch_exe(args: &[String]) -> ExitCode {
    let ready_file = flag_value(args, "--ready-file");
    let detected_file = flag_value(args, "--detected-file");
    let timeout_ms: u64 = flag_value(args, "--timeout-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000);

    let identity = match kestic_daemon::binary_watch::BinaryIdentity::capture_current() {
        Ok(id) => id,
        Err(err) => {
            eprintln!("failed to capture binary identity: {err}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(path) = ready_file
        && let Err(err) = fs::write(&path, "ready\n")
    {
        eprintln!("failed to write ready file {path}: {err}");
        return ExitCode::FAILURE;
    }

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        if let kestic_daemon::binary_watch::BinaryStatus::Replaced(reason) = identity.check() {
            if let Some(path) = detected_file
                && let Err(err) = fs::write(&path, format!("{}\n", reason.as_str()))
            {
                eprintln!("failed to write detected file {path}: {err}");
                return ExitCode::FAILURE;
            }
            return ExitCode::SUCCESS;
        }
        thread::sleep(Duration::from_millis(20));
    }

    eprintln!("timed out waiting for executable replacement");
    ExitCode::FAILURE
}

fn run_spike_inhibit(args: &[String]) -> ExitCode {
    let seconds: u64 = flag_value(args, "--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let dest =
        flag_value(args, "--dest").unwrap_or_else(|| "org.freedesktop.portal.Desktop".to_string());

    let guard = match kestic_daemon::inhibit::PortalInhibitGuard::acquire(
        &dest,
        "Kestic backup inhibit spike check (F9)",
        kestic_daemon::inhibit::INHIBIT_IDLE_AND_SUSPEND,
    ) {
        Ok(g) => g,
        Err(err) => {
            eprintln!("portal inhibit failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    println!("Portal inhibit active: handle={}", guard.handle_path());
    thread::sleep(Duration::from_secs(seconds));
    if let Err(err) = guard.release() {
        eprintln!("portal inhibit close failed: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

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
    let raw_args: Vec<String> = env::args().skip(1).collect();
    if let Some(subcmd) = raw_args.first().map(String::as_str) {
        match subcmd {
            "daemon" => {
                let autostart = raw_args.iter().any(|a| a == "--autostart");
                if autostart && !has_kestic_config() {
                    return ExitCode::SUCCESS;
                }
            }
            "spike-watch-exe" => return run_spike_watch_exe(&raw_args[1..]),
            "spike-inhibit" => return run_spike_inhibit(&raw_args[1..]),
            _ => {}
        }
    }

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
