// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Kestic GUI and CLI entrypoint.

mod backend;
mod models;
mod overall_text;
mod smoke;
mod state_cache;
mod ui_texts;

use cxx_kde_frameworks::ki18n::{self, KLocalizedString};
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QByteArray, QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};
use cxx_qt_lib_extras::QApplication;
use kestic_core::sync::LockUnpoisoned;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

struct SmokeOptions {
    scenario: String,
    screenshot: String,
    layout: String,
    lang: String,
}

fn has_kestic_config(explicit_path: Option<&PathBuf>) -> bool {
    if let Some(p) = explicit_path {
        return p.is_file();
    }
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

fn parse_simulated_status(raw: &str) -> Option<kestic_daemon::service::SimulatedOverallStatus> {
    use kestic_daemon::service::SimulatedOverallStatus as S;
    match raw {
        "overdue" => Some(S::Overdue),
        "paused" => Some(S::Paused),
        "waiting" => Some(S::Waiting),
        "paused_waiting" => Some(S::PausedAndWaiting),
        "maintenance" => Some(S::MaintenanceRunning),
        "restore" => Some(S::RestoreRunning),
        _ => None,
    }
}

fn default_icon_theme_path() -> String {
    let repo_icons = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../packaging/icons")
        .canonicalize()
        .ok();
    if let Some(dir) = repo_icons
        && dir
            .join("hicolor/scalable/status/kestic-symbolic.svg")
            .is_file()
    {
        return dir.to_string_lossy().into_owned();
    }
    let cwd_icons = PathBuf::from("packaging/icons").canonicalize().ok();
    if let Some(dir) = cwd_icons
        && dir
            .join("hicolor/scalable/status/kestic-symbolic.svg")
            .is_file()
    {
        return dir.to_string_lossy().into_owned();
    }
    String::new()
}

fn parse_gui_launch_config(args: &[String]) -> state_cache::GuiLaunchConfig {
    let mut cfg = state_cache::GuiLaunchConfig::default();
    if let Some(bus_name) = flag_value(args, "--bus-name") {
        cfg.bus_name = bus_name;
    }
    if args.iter().any(|a| a == "--confirm-quit") {
        cfg.show_quit_dialog_on_start = true;
    }
    if args.iter().any(|a| a == "--no-autostart") {
        cfg.no_autostart_daemon = true;
    }
    cfg
}

fn parse_smoke_args() -> Option<SmokeOptions> {
    let mut args = env::args().skip(1);
    let mut smoke = false;
    let mut scenario = String::new();
    let mut screenshot = String::new();
    let mut layout = String::new();
    let mut lang = String::from("en");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke-m1" => {
                smoke = true;
                scenario = args.next().unwrap_or_default();
            }
            "--screenshot" => {
                screenshot = args.next().unwrap_or_default();
            }
            "--smoke-layout" => {
                layout = args.next().unwrap_or_default();
            }
            "--smoke-lang" => {
                if let Some(val) = args.next() {
                    lang = val;
                }
            }
            _ => {}
        }
    }
    smoke.then_some(SmokeOptions {
        scenario,
        screenshot,
        layout,
        lang,
    })
}

fn configure_dev_locale_search_path() {
    let current = env::var_os("XDG_DATA_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut paths: Vec<PathBuf> = env::split_paths(&current).collect();
    let mut modified = false;

    if let Ok(repo_packaging) = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../packaging")
        .canonicalize()
        && repo_packaging.join("icons/hicolor").is_dir()
        && !paths.iter().any(|p| p == &repo_packaging)
    {
        paths.insert(0, repo_packaging);
        modified = true;
    }

    let dev_locale_dir = PathBuf::from(env!("KESTIC_DEV_LOCALE_DIR"));
    if dev_locale_dir.is_dir()
        && let Some(data_root) = dev_locale_dir.parent()
        && !paths.iter().any(|p| p == data_root)
    {
        paths.push(data_root.to_path_buf());
        modified = true;
    }

    if modified && let Ok(joined) = env::join_paths(paths) {
        // SAFETY: Called in single-threaded `main` before constructing QApplication or spawning threads.
        unsafe {
            env::set_var("XDG_DATA_DIRS", joined);
        }
    }
}

fn install_daemon_ki18n() {
    use std::sync::{Arc, Mutex};

    let qt_lock = Arc::new(Mutex::new(()));
    let qt_lock_subs = Arc::clone(&qt_lock);
    let translator = Arc::new(kestic_daemon::i18n::CallbackSubsTranslator {
        translate_fn: move |msgid: &str| {
            let _g = qt_lock.lock_unpoisoned();
            ki18n::i18n(msgid).to_string()
        },
        translate_subs_fn: move |msgid: &str, args: &[&str]| {
            let _g = qt_lock_subs.lock_unpoisoned();
            crate::smoke::ffi::i18n_subs(msgid, args)
        },
    });
    kestic_daemon::i18n::set_daemon_translator(translator);
}

fn run_daemon_subcommand(args: &[String]) -> ExitCode {
    let autostart = args.iter().any(|a| a == "--autostart");
    let config_path = flag_value(args, "--config-path").map(PathBuf::from);
    if autostart && !has_kestic_config(config_path.as_ref()) {
        return ExitCode::SUCCESS;
    }

    configure_dev_locale_search_path();
    let _qt_core_app = cxx_qt_lib::QCoreApplication::new();
    KLocalizedString::set_application_domain(&QByteArray::from("kestic"));
    install_daemon_ki18n();

    let duration = if args.iter().any(|a| a == "--once") {
        Some(Duration::from_millis(50))
    } else {
        flag_value(args, "--duration-ms")
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_millis)
    };

    let icon_theme_path =
        flag_value(args, "--icon-theme-path").unwrap_or_else(default_icon_theme_path);
    let bus_name = flag_value(args, "--bus-name")
        .unwrap_or_else(|| kestic_daemon::dbus_api::KESTIC_BUS_NAME.to_string());
    let simulated_status = flag_value(args, "--simulate-status")
        .as_deref()
        .and_then(parse_simulated_status);

    let config = kestic_daemon::service::DaemonRunConfig {
        bus_name,
        assume_sni_available: !args.iter().any(|a| a == "--require-sni"),
        tray_state: kestic_daemon::tray::KesticTrayState {
            icon_theme_path,
            ..Default::default()
        },
        ready_file: flag_value(args, "--ready-file").map(PathBuf::from),
        duration,
        config_path,
        state_dir: flag_value(args, "--state-dir").map(PathBuf::from),
        suppress_ui_spawn: args.iter().any(|a| a == "--suppress-ui-spawn"),
        enable_job_view: !args.iter().any(|a| a == "--no-job-view"),
        enable_portal_inhibit: !args.iter().any(|a| a == "--no-portal-inhibit"),
        enable_system_monitors: !args.iter().any(|a| a == "--no-monitors"),
        enable_scheduler: !args.iter().any(|a| a == "--no-scheduler"),
        enable_binary_watch: !args.iter().any(|a| a == "--no-binary-watch"),
        simulated_status,
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

fn run_secret_subcommand(args: &[String]) -> ExitCode {
    use kestic_core::secrets::{FileSecretStore, SecretStore};
    use std::io::Write;

    let Some("get") = args.first().map(String::as_str) else {
        return ExitCode::FAILURE;
    };
    let Some(dest_id) = args.get(1).map(String::as_str).filter(|s| !s.is_empty()) else {
        return ExitCode::FAILURE;
    };
    let kind = args
        .get(2)
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("repo");

    let secret_opt =
        if let Some(file_path) = env::var_os("KESTIC_SECRET_FILE").filter(|v| !v.is_empty()) {
            FileSecretStore::new(PathBuf::from(file_path)).get_secret(dest_id, kind)
        } else {
            kestic_daemon::secrets::SecretServiceStore::default().get_secret(dest_id, kind)
        };

    match secret_opt {
        Ok(Some(secret)) => {
            if let Ok(text) = secret.as_str()
                && std::io::stdout().write_all(text.as_bytes()).is_ok()
                && std::io::stdout().flush().is_ok()
            {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Ok(None) | Err(_) => ExitCode::FAILURE,
    }
}

fn main() -> ExitCode {
    let raw_args: Vec<String> = env::args().skip(1).collect();
    if let Some(subcmd) = raw_args.first().map(String::as_str) {
        match subcmd {
            "daemon" => return run_daemon_subcommand(&raw_args[1..]),
            "secret" => return run_secret_subcommand(&raw_args[1..]),
            _ => {}
        }
    }

    cxx_qt::init_crate!(cxx_qt);
    cxx_qt::init_qml_module!("net.daester.kestic");
    let smoke_opts = parse_smoke_args();
    let launch_cfg = parse_gui_launch_config(&raw_args);
    state_cache::set_launch_config(launch_cfg);
    configure_dev_locale_search_path();

    let mut app = QApplication::new();
    QGuiApplication::set_desktop_file_name(&QString::from("net.daester.Kestic"));
    KLocalizedString::set_application_domain(&QByteArray::from("kestic"));
    if env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        QQuickStyle::set_style(&QString::from("org.kde.desktop"));
    }

    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine_pin) = engine.as_mut() {
        ki18n::setup_localized_context(engine_pin.as_mut().upcast_pin());
        smoke::ffi::install_keyboard_handlers(engine_pin.as_mut());
        if let Some(opts) = smoke_opts.as_ref() {
            smoke::ffi::install_m1_smoke_check(
                engine_pin.as_mut(),
                &opts.scenario,
                &opts.screenshot,
                &opts.layout,
                &opts.lang,
            );
        }
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
