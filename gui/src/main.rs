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

struct SmokeOptions {
    screenshot: String,
    layout: String,
    trigger_shortcut: bool,
}

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

fn run_spike_jobview(args: &[String]) -> ExitCode {
    let seconds: u64 = flag_value(args, "--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let dest = flag_value(args, "--dest")
        .unwrap_or_else(|| kestic_daemon::job_view::DEFAULT_JOB_VIEW_DEST.to_string());

    let handle = match kestic_daemon::job_view::JobViewHandle::request(
        &dest,
        kestic_daemon::notifications::KESTIC_DESKTOP_ENTRY,
        "Kestic spike backup",
        kestic_daemon::job_view::CAPABILITY_KILLABLE,
    ) {
        Ok(h) => h,
        Err(err) => {
            eprintln!("job view request failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    println!("JobView active: path={}", handle.view_path());
    let steps = seconds.max(1);
    for i in 1..=steps {
        let pct = u32::try_from((i * 100) / steps).unwrap_or(100).min(99);
        let progress = kestic_daemon::job_view::JobProgress {
            percent: pct,
            processed_bytes: i * 50_000_000,
            total_bytes: steps * 50_000_000,
            speed_bytes_per_sec: 50_000_000,
            info_message: format!("Backing up Personal folder ({pct} %)"),
            elapsed_ms: i * 1_000,
        };
        if let Err(err) = handle.update(&progress) {
            eprintln!("job view update failed: {err}");
            return ExitCode::FAILURE;
        }
        match handle.wait_cancel_requested(Duration::from_secs(1)) {
            Ok(true) => {
                println!("JobView cancelRequested received!");
                let _ = handle.terminate(1, "Cancelled by user");
                return ExitCode::SUCCESS;
            }
            Ok(false) => {}
            Err(err) => {
                eprintln!("job view signal watch failed: {err}");
                return ExitCode::FAILURE;
            }
        }
    }

    if let Err(err) = handle.terminate(0, "") {
        eprintln!("job view terminate failed: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run_spike_notify(args: &[String]) -> ExitCode {
    let event = match flag_value(args, "--event").as_deref() {
        Some("BackupWarning") => kestic_daemon::notifications::KesticEvent::BackupWarning,
        Some("BackupOverdue") => kestic_daemon::notifications::KesticEvent::BackupOverdue,
        _ => kestic_daemon::notifications::KesticEvent::BackupFailed,
    };

    match kestic_daemon::notifications::send_event_notification(
        event,
        "Kestic spike notification",
        "Testing kestic.notifyrc event filtering with desktop-entry hint.",
    ) {
        Ok(id) => {
            println!("Sent notification id={id} event={}", event.as_str());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("notification failed: {err}");
            ExitCode::FAILURE
        }
    }
}

fn parse_smoke_args() -> Option<SmokeOptions> {
    let mut args = env::args().skip(1);
    let mut smoke = false;
    let mut screenshot = String::new();
    let mut layout = String::new();
    let mut trigger_shortcut = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke-test" => smoke = true,
            "--screenshot" => {
                screenshot = args.next().unwrap_or_default();
            }
            "--smoke-layout" => {
                layout = args.next().unwrap_or_default();
            }
            "--smoke-shortcut" => trigger_shortcut = true,
            _ => {}
        }
    }
    smoke.then_some(SmokeOptions {
        screenshot,
        layout,
        trigger_shortcut,
    })
}

fn run_daemon_subcommand(args: &[String]) -> ExitCode {
    let autostart = args.iter().any(|a| a == "--autostart");
    if autostart && !has_kestic_config() {
        return ExitCode::SUCCESS;
    }

    let duration = if args.iter().any(|a| a == "--once") {
        Some(Duration::from_millis(50))
    } else {
        flag_value(args, "--duration-ms")
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_millis)
    };

    let config = kestic_daemon::service::DaemonRunConfig {
        assume_sni_available: !args.iter().any(|a| a == "--require-sni"),
        send_test_notification: args.iter().any(|a| a == "--notify-test"),
        ready_file: flag_value(args, "--ready-file").map(PathBuf::from),
        duration,
        ..Default::default()
    };

    match kestic_daemon::service::run_daemon(config) {
        Ok(kestic_daemon::service::DaemonOutcome::AlreadyRunning) => {
            println!("Kestic daemon is already running; exiting.");
            ExitCode::SUCCESS
        }
        Ok(kestic_daemon::service::DaemonOutcome::RanAndStopped) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("daemon error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let _ = kestic_daemon::daemon_version();
    let raw_args: Vec<String> = env::args().skip(1).collect();
    if let Some(subcmd) = raw_args.first().map(String::as_str) {
        match subcmd {
            "daemon" => return run_daemon_subcommand(&raw_args[1..]),
            "spike-watch-exe" => return run_spike_watch_exe(&raw_args[1..]),
            "spike-inhibit" => return run_spike_inhibit(&raw_args[1..]),
            "spike-jobview" => return run_spike_jobview(&raw_args[1..]),
            "spike-notify" => return run_spike_notify(&raw_args[1..]),
            _ => {}
        }
    }

    cxx_qt::init_crate!(cxx_qt);
    cxx_qt::init_qml_module!("net.daester.kestic");
    let smoke_opts = parse_smoke_args();

    let mut app = QApplication::new();
    QGuiApplication::set_desktop_file_name(&QString::from("net.daester.Kestic"));
    if env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        QQuickStyle::set_style(&QString::from("org.kde.desktop"));
    }

    let mut engine = QQmlApplicationEngine::new();
    if let (Some(engine_pin), Some(opts)) = (engine.as_mut(), smoke_opts.as_ref()) {
        smoke::ffi::install_smoke_check(
            engine_pin,
            &opts.screenshot,
            3,
            &opts.layout,
            opts.trigger_shortcut,
        );
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
