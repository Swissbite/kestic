// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Kestic GUI and CLI entrypoint.

mod backend;
mod list_model;
mod models;
mod smoke;
mod state_cache;

use cxx_kde_frameworks::ki18n::{self, KLocalizedString};
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QByteArray, QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};
use cxx_qt_lib_extras::QApplication;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

struct SmokeOptions {
    is_m1: bool,
    scenario: String,
    screenshot: String,
    layout: String,
    lang: String,
    trigger_shortcut: bool,
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

fn parse_tray_status(raw: &str) -> kestic_daemon::tray::TrayStatus {
    match raw {
        "passive" => kestic_daemon::tray::TrayStatus::Passive,
        "needs-attention" => kestic_daemon::tray::TrayStatus::NeedsAttention,
        _ => kestic_daemon::tray::TrayStatus::Active,
    }
}

fn run_spike_tray(args: &[String]) -> ExitCode {
    let seconds: u64 = flag_value(args, "--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let cycle = args.iter().any(|a| a == "--cycle");
    let assume_sni_available = !args.iter().any(|a| a == "--require-sni");
    let icon_theme_path =
        flag_value(args, "--icon-theme-path").unwrap_or_else(default_icon_theme_path);
    let initial_status = flag_value(args, "--status")
        .map(|s| parse_tray_status(&s))
        .unwrap_or(kestic_daemon::tray::TrayStatus::Active);

    let (quit_tx, quit_rx) = std::sync::mpsc::channel();
    let state = kestic_daemon::tray::KesticTrayState {
        status: initial_status,
        icon_theme_path: icon_theme_path.clone(),
        tooltip_description: format!("Spike tray ({initial_status:?})"),
        ..Default::default()
    };

    let tray =
        match kestic_daemon::tray::TrayHandle::spawn(state, assume_sni_available, Some(quit_tx)) {
            Ok(t) => t,
            Err(err) => {
                eprintln!("tray spawn failed: {err}");
                return ExitCode::FAILURE;
            }
        };

    println!(
        "Tray registered: icon=kestic-symbolic icon_theme_path={:?} status={initial_status:?}",
        icon_theme_path
    );

    if cycle {
        let steps = [
            (
                kestic_daemon::tray::TrayStatus::Active,
                "",
                "Active (visible in Plasma panel)",
            ),
            (
                kestic_daemon::tray::TrayStatus::Passive,
                "",
                "Passive (hidden in Plasma expanded tray)",
            ),
            (
                kestic_daemon::tray::TrayStatus::NeedsAttention,
                "dialog-error",
                "NeedsAttention (highlighted in Plasma panel)",
            ),
            (
                kestic_daemon::tray::TrayStatus::Active,
                "",
                "Active again (returned to Plasma panel)",
            ),
        ];
        let step_ms = ((seconds * 1000) / u64::try_from(steps.len()).unwrap_or(4)).max(500);
        for (status, overlay, desc) in steps {
            println!("SNI Status -> {status:?}: {desc}");
            tray.update_state(status, overlay, desc);
            if quit_rx.recv_timeout(Duration::from_millis(step_ms)).is_ok() {
                break;
            }
        }
    } else {
        let _ = quit_rx.recv_timeout(Duration::from_secs(seconds));
    }

    tray.shutdown();
    ExitCode::SUCCESS
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
    let mut is_m1 = false;
    let mut scenario = String::new();
    let mut screenshot = String::new();
    let mut layout = String::new();
    let mut lang = String::from("en");
    let mut trigger_shortcut = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke-test" => smoke = true,
            "--smoke-m1" => {
                smoke = true;
                is_m1 = true;
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
            "--smoke-shortcut" => trigger_shortcut = true,
            _ => {}
        }
    }
    smoke.then_some(SmokeOptions {
        is_m1,
        scenario,
        screenshot,
        layout,
        lang,
        trigger_shortcut,
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
            let _g = qt_lock.lock().unwrap_or_else(|p| p.into_inner());
            ki18n::i18n(msgid).to_string()
        },
        translate_subs_fn: move |msgid: &str, args: &[&str]| {
            let _g = qt_lock_subs.lock().unwrap_or_else(|p| p.into_inner());
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
        send_test_notification: args.iter().any(|a| a == "--notify-test"),
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
    let _ = kestic_daemon::daemon_version();
    let raw_args: Vec<String> = env::args().skip(1).collect();
    if let Some(subcmd) = raw_args.first().map(String::as_str) {
        match subcmd {
            "daemon" => return run_daemon_subcommand(&raw_args[1..]),
            "secret" => return run_secret_subcommand(&raw_args[1..]),
            "spike-watch-exe" => return run_spike_watch_exe(&raw_args[1..]),
            "spike-inhibit" => return run_spike_inhibit(&raw_args[1..]),
            "spike-jobview" => return run_spike_jobview(&raw_args[1..]),
            "spike-notify" => return run_spike_notify(&raw_args[1..]),
            "spike-tray" => return run_spike_tray(&raw_args[1..]),
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
        let qml_url = match smoke_opts.as_ref() {
            Some(opts) if !opts.is_m1 => {
                smoke::ffi::install_smoke_check(
                    engine_pin.as_mut(),
                    &opts.screenshot,
                    3,
                    &opts.layout,
                    &opts.lang,
                    opts.trigger_shortcut,
                );
                "qrc:/qt/qml/net/daester/kestic/src/qml/SpikeMain.qml"
            }
            Some(opts) => {
                smoke::ffi::install_m1_smoke_check(
                    engine_pin.as_mut(),
                    &opts.scenario,
                    &opts.screenshot,
                    &opts.layout,
                    &opts.lang,
                );
                "qrc:/qt/qml/net/daester/kestic/src/qml/Main.qml"
            }
            None => "qrc:/qt/qml/net/daester/kestic/src/qml/Main.qml",
        };
        engine_pin.load(&QUrl::from(qml_url));
    }

    let code = if let Some(app_pin) = app.as_mut() {
        app_pin.exec()
    } else {
        1
    };

    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
