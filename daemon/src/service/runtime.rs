// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Daemon runtime loop (`run_daemon`), system monitor event handling, and tray action dispatch.

use super::availability::{drain_queued_backups_async, refresh_availability_unlocked};
use super::backup_runner::start_backup_async;
use super::maintenance::start_all_backups_async;
use super::scheduler::tick_scheduler_async;
use super::state::{AVAILABILITY_CACHE_TTL_SECS, DaemonCore, DaemonOutcome, DaemonRunConfig};
use crate::binary_watch::{BinaryIdentity, BinaryStatus};
use crate::dbus_api::{DestinationAvailabilityDto, KESTIC_BUS_NAME};
use crate::dbus_server::{
    emit_quit_confirmation_requested, emit_show_window_requested, emit_status_changed,
    start_dbus_server,
};
use crate::i18n::{i18n, tr_text};
use crate::monitors::{SystemMonitorEvent, SystemMonitorHandle, read_system_environment};
use crate::notifications::{KesticEvent, send_configured_notification};
use crate::single_instance::{InstanceAcquire, SingleInstanceLock};
use crate::tray::{KesticTrayState, TrayAction, TrayHandle};
use kestic_core::destination::{DEFAULT_CONNECT_TIMEOUT, preflight_check_destination};
use kestic_core::history::BackoffKind;
use kestic_core::model::{BackupId, DestinationKind};
use kestic_core::mount::{check_source_status_with, current_home_dir};
use kestic_core::scheduler::SystemEnvironment;
use kestic_core::sync::LockUnpoisoned;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use zbus::Connection;

/// Runs the Kestic daemon service loop (`SingleInstanceLock` + `net.daester.Kestic1` + `TrayHandle`).
pub fn run_daemon(config: DaemonRunConfig) -> Result<DaemonOutcome, String> {
    let core = Arc::new(Mutex::new(DaemonCore::new(&config)?));
    let (quit_tx, quit_rx) = mpsc::channel();
    let (action_tx, action_rx) = mpsc::channel();

    {
        let mut guard = core.lock_unpoisoned();
        guard.quit_tx = Some(quit_tx.clone());
    }

    let lock = match start_dbus_server(&config.bus_name, Arc::clone(&core))? {
        InstanceAcquire::Acquired(lock) => lock,
        InstanceAcquire::AlreadyRunning => return Ok(DaemonOutcome::AlreadyRunning),
    };

    eprintln!(
        "{}",
        crate::startup_line(&crate::log_label(), std::process::id())
    );

    let conn = lock.connection().clone();
    let (_, initial_tray_state) = {
        let mut guard = core.lock_unpoisoned();
        guard.conn = Some(conn.clone());
        guard.build_state_and_tray()
    };

    let core_for_about = Arc::clone(&core);
    let conn_for_about = conn.clone();
    let about_to_show_hook: Arc<dyn Fn() -> Option<KesticTrayState> + Send + Sync> =
        Arc::new(move || {
            let _ = refresh_availability_unlocked(&core_for_about, false);
            let (state_dto, tray_state) = {
                let guard = core_for_about.lock_unpoisoned();
                guard.build_state_and_tray()
            };
            emit_status_changed(&conn_for_about, &state_dto);
            Some(tray_state)
        });

    let tray = Arc::new(
        TrayHandle::spawn_with_actions(
            initial_tray_state,
            config.assume_sni_available,
            Some(action_tx),
            Some(quit_tx),
            Some(about_to_show_hook),
        )
        .map_err(|err| format!("failed to register StatusNotifierItem tray: {err}"))?,
    );

    {
        let mut guard = core.lock_unpoisoned();
        guard.tray = Some(Arc::clone(&tray));
    }

    if let Some(ready_path) = &config.ready_file {
        fs::write(ready_path, "ready\n")
            .map_err(|err| format!("failed to write ready file {}: {err}", ready_path.display()))?;
    }

    let mut monitor_rx_and_handle: Option<(
        mpsc::Receiver<SystemMonitorEvent>,
        SystemMonitorHandle,
    )> = if config.enable_system_monitors {
        let (mon_tx, mon_rx) = mpsc::channel();
        let handle = SystemMonitorHandle::start(config.monitor_bus.clone(), mon_tx, None);
        Some((mon_rx, handle))
    } else {
        None
    };

    let bin_identity = if config.enable_binary_watch {
        BinaryIdentity::capture_current().ok()
    } else {
        None
    };

    if config.enable_scheduler {
        tick_scheduler_async(&core);
    }

    let pending_exec_path = run_daemon_event_loop(DaemonLoopContext {
        core: &core,
        conn: &conn,
        tray: &tray,
        config: &config,
        action_rx: &action_rx,
        quit_rx: &quit_rx,
        monitor_rx: monitor_rx_and_handle.as_ref().map(|(rx, _)| rx),
        bin_identity: bin_identity.as_ref(),
    });

    while let Ok(action) = action_rx.try_recv() {
        handle_tray_action(&core, &conn, &config, action);
    }

    if let Some((_, handle)) = monitor_rx_and_handle.take() {
        handle.stop();
    }

    shutdown_daemon_and_maybe_reexec(&core, lock, conn, tray, &config, pending_exec_path)
}

struct DaemonLoopContext<'a> {
    core: &'a Arc<Mutex<DaemonCore>>,
    conn: &'a Connection,
    tray: &'a Arc<TrayHandle>,
    config: &'a DaemonRunConfig,
    action_rx: &'a mpsc::Receiver<TrayAction>,
    quit_rx: &'a mpsc::Receiver<()>,
    monitor_rx: Option<&'a mpsc::Receiver<SystemMonitorEvent>>,
    bin_identity: Option<&'a BinaryIdentity>,
}

struct DaemonLoopState {
    deadline: Option<Instant>,
    last_sched_tick: Instant,
    last_bin_check: Instant,
    last_network_reconnect_probe: Option<time::OffsetDateTime>,
}

fn run_daemon_event_loop(ctx: DaemonLoopContext<'_>) -> Option<PathBuf> {
    let DaemonLoopContext {
        core,
        conn,
        tray,
        config,
        action_rx,
        quit_rx,
        monitor_rx,
        bin_identity,
    } = ctx;
    let mut state = DaemonLoopState {
        deadline: config.duration.map(|d| Instant::now() + d),
        last_sched_tick: Instant::now(),
        last_bin_check: Instant::now(),
        last_network_reconnect_probe: None,
    };

    loop {
        while let Ok(action) = action_rx.try_recv() {
            handle_tray_action(core, conn, config, action);
        }

        if let Some(mon_rx) = monitor_rx {
            while let Ok(ev) = mon_rx.try_recv() {
                handle_system_monitor_event(
                    core,
                    conn,
                    tray,
                    config,
                    ev,
                    &mut state.last_network_reconnect_probe,
                );
            }
        }

        if config.enable_scheduler && state.last_sched_tick.elapsed() >= Duration::from_secs(1) {
            state.last_sched_tick = Instant::now();
            tick_scheduler_async(core);
        }

        if let Some(bin_id) = bin_identity
            && state.last_bin_check.elapsed() >= Duration::from_millis(500)
        {
            state.last_bin_check = Instant::now();
            if let BinaryStatus::Replaced(reason) = bin_id.check() {
                let has_active_runs = {
                    let guard = core.lock_unpoisoned();
                    !guard.active_runs.is_empty()
                };
                if !has_active_runs {
                    eprintln!(
                        "{}: event loop ends: binary replaced ({}), re-executing",
                        crate::log_label(),
                        reason.as_str()
                    );
                    return Some(bin_id.exe_path().to_path_buf());
                }
            }
        }

        if core.lock_unpoisoned().shutdown_flag.load(Ordering::SeqCst) {
            eprintln!("kestic-daemon: event loop ends: shutdown requested");
            return None;
        }

        let step_wait = if monitor_rx.is_some() || bin_identity.is_some() {
            Duration::from_millis(25)
        } else {
            Duration::from_millis(50)
        };

        if let Some(end) = state.deadline {
            let now = Instant::now();
            if now >= end {
                eprintln!("kestic-daemon: event loop ends: run duration elapsed");
                return None;
            }
            let wait = (end - now).min(step_wait);
            if quit_rx.recv_timeout(wait).is_ok() {
                eprintln!("kestic-daemon: event loop ends: quit requested");
                return None;
            }
        } else if quit_rx.recv_timeout(step_wait).is_ok() {
            eprintln!("kestic-daemon: event loop ends: quit requested");
            return None;
        }
    }
}

fn shutdown_daemon_and_maybe_reexec(
    core: &Arc<Mutex<DaemonCore>>,
    lock: SingleInstanceLock,
    conn: Connection,
    tray: Arc<TrayHandle>,
    config: &DaemonRunConfig,
    pending_exec_path: Option<PathBuf>,
) -> Result<DaemonOutcome, String> {
    {
        let mut guard = core.lock_unpoisoned();
        // Runs cancelled by shutdown are interrupted, not user-cancelled (no slot handling).
        guard.shutdown_flag.store(true, Ordering::SeqCst);
        guard.cancel_any_active_run();
        guard.tray = None;
        guard.conn = None;
    }

    if let Ok(owned) = Arc::try_unwrap(tray) {
        owned.shutdown();
    }
    let _ = zbus::block_on(conn.release_name(config.bus_name.as_str()));
    drop(lock);
    let _ = zbus::block_on(conn.close());

    if let Some(exe_path) = pending_exec_path {
        use std::os::unix::process::CommandExt;
        let mut cmd = Command::new(&exe_path);
        if let Some(args) = &config.binary_watch_args {
            cmd.args(args);
        } else {
            cmd.args(env::args_os().skip(1));
        }
        let err = cmd.exec();
        return Err(format!(
            "failed to exec replaced binary {}: {err}",
            exe_path.display()
        ));
    }

    eprintln!("kestic-daemon: stopped");
    Ok(DaemonOutcome::RanAndStopped)
}

fn handle_system_monitor_event(
    core: &Arc<Mutex<DaemonCore>>,
    conn: &Connection,
    tray: &Arc<TrayHandle>,
    config: &DaemonRunConfig,
    ev: SystemMonitorEvent,
    last_network_reconnect_probe: &mut Option<time::OffsetDateTime>,
) {
    match ev {
        SystemMonitorEvent::EnvironmentChanged(new_env) => {
            handle_environment_changed(
                core,
                conn,
                tray,
                config,
                new_env,
                last_network_reconnect_probe,
            );
        }
        SystemMonitorEvent::MountChanged => {
            handle_mount_changed(core, conn, tray, config);
        }
        SystemMonitorEvent::PrepareForSleep(going_to_sleep) => {
            handle_prepare_for_sleep(core, conn, tray, config, going_to_sleep);
        }
    }
}

fn handle_environment_changed(
    core: &Arc<Mutex<DaemonCore>>,
    conn: &Connection,
    tray: &Arc<TrayHandle>,
    config: &DaemonRunConfig,
    new_env: SystemEnvironment,
    last_network_reconnect_probe: &mut Option<time::OffsetDateTime>,
) {
    let ((state_dto, tray_state), network_changed, force_probe) = {
        let mut guard = core.lock_unpoisoned();
        let old_env = guard.system_env.clone();
        let network_changed = old_env.network_connected != new_env.network_connected
            || old_env.network_metered != new_env.network_metered
            || old_env.active_networks != new_env.active_networks;
        let network_just_came_online = !old_env.network_connected && new_env.network_connected;
        guard.system_env = new_env;

        let force_probe = if network_changed {
            let now = guard.clock.now_utc();
            let cooldown_elapsed = match *last_network_reconnect_probe {
                None => true,
                Some(prev) => (now - prev) > time::Duration::seconds(AVAILABILITY_CACHE_TTL_SECS),
            };
            let force = network_just_came_online && cooldown_elapsed;
            if force {
                *last_network_reconnect_probe = Some(now);
            }
            force
        } else {
            false
        };
        (guard.build_state_and_tray(), network_changed, force_probe)
    };
    tray.set_full_state(tray_state);
    emit_status_changed(conn, &state_dto);

    if network_changed {
        let core_bg = Arc::clone(core);
        let conn_bg = conn.clone();
        let tray_bg = Arc::clone(tray);
        let enable_scheduler = config.enable_scheduler;
        thread::spawn(move || {
            let _ = refresh_availability_unlocked(&core_bg, force_probe);
            let (state_dto, tray_state) = {
                let mut guard = core_bg.lock_unpoisoned();
                for backup in guard.config.backups.clone() {
                    let dest_ok = guard
                        .dest_availability
                        .get(&backup.destination_id)
                        .is_none_or(|d| d.reachable);
                    if !dest_ok {
                        continue;
                    }
                    let mut rs = guard.load_backup_runtime_state(&backup.id);
                    if rs.backoff_kind == Some(BackoffKind::Network)
                        && (rs.backoff_until.is_some() || rs.backoff_attempt > 0)
                    {
                        kestic_core::scheduler::clear_backup_backoff(&mut rs);
                        let _ = guard.store_backup_runtime_state(&backup.id, &rs);
                    }
                }
                guard.build_state_and_tray()
            };
            tray_bg.set_full_state(tray_state);
            emit_status_changed(&conn_bg, &state_dto);
            if enable_scheduler {
                tick_scheduler_async(&core_bg);
            }
        });
    } else if config.enable_scheduler {
        tick_scheduler_async(core);
    }
}

fn handle_mount_changed(
    core: &Arc<Mutex<DaemonCore>>,
    conn: &Connection,
    tray: &Arc<TrayHandle>,
    config: &DaemonRunConfig,
) {
    let core_bg = Arc::clone(core);
    let conn_bg = conn.clone();
    let tray_bg = Arc::clone(tray);
    let enable_scheduler = config.enable_scheduler;
    thread::spawn(move || {
        let (backups, destinations, mount_inspector, clock) = {
            let guard = core_bg.lock_unpoisoned();
            (
                guard.config.backups.clone(),
                guard.config.destinations.clone(),
                Arc::clone(&guard.mount_inspector),
                Arc::clone(&guard.clock),
            )
        };
        let home = current_home_dir();
        let mut source_updates = Vec::with_capacity(backups.len());
        for backup in &backups {
            let status =
                check_source_status_with(backup, home.as_deref(), mount_inspector.as_ref());
            source_updates.push((backup.id.clone(), status));
        }
        let now_unix = clock.now_unix_secs();
        let mut dest_updates = Vec::new();
        for dest in &destinations {
            if matches!(dest.kind, DestinationKind::Local { .. }) {
                let res = preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT);
                let dto = DestinationAvailabilityDto::from_probe(&dest.id, &res, now_unix);
                dest_updates.push((dest.id.clone(), dto));
            }
        }
        let (state_dto, tray_state) = {
            let mut guard = core_bg.lock_unpoisoned();
            for (id, status) in source_updates {
                guard.source_availability.insert(id, status);
            }
            for (id, dto) in dest_updates {
                guard.dest_availability.insert(id, dto);
            }
            guard.build_state_and_tray()
        };
        tray_bg.set_full_state(tray_state);
        emit_status_changed(&conn_bg, &state_dto);
        if enable_scheduler {
            tick_scheduler_async(&core_bg);
        }
    });
}

fn handle_prepare_for_sleep(
    core: &Arc<Mutex<DaemonCore>>,
    conn: &Connection,
    tray: &Arc<TrayHandle>,
    config: &DaemonRunConfig,
    going_to_sleep: bool,
) {
    if going_to_sleep {
        // interrupt running jobs cleanly, lock scheduler until wake, and mark for resume.
        let (state_dto, tray_state) = {
            let mut guard = core.lock_unpoisoned();
            guard.sleep_in_progress = true;
            let running_ids: Vec<BackupId> = guard
                .active_runs
                .values()
                .filter_map(|e| e.backup_id.clone())
                .collect();
            let queued_ids: Vec<BackupId> = guard.queued_backups.iter().cloned().collect();
            for entry in guard.active_runs.values_mut() {
                entry.sleep_interrupted = true;
            }
            for bid in running_ids.into_iter().chain(queued_ids) {
                guard.sleep_interrupted_backups.insert(bid);
            }
            guard.cancel_any_active_run();
            guard.build_state_and_tray()
        };
        tray.set_full_state(tray_state);
        emit_status_changed(conn, &state_dto);
    } else {
        // re-read environment, refresh availability, unlock, and catch up.
        let core_bg = Arc::clone(core);
        let conn_bg = conn.clone();
        let tray_bg = Arc::clone(tray);
        let enable_monitors = config.enable_system_monitors;
        let monitor_bus = config.monitor_bus.clone();
        let enable_scheduler = config.enable_scheduler;
        thread::spawn(move || {
            let new_env = if enable_monitors {
                read_system_environment(&monitor_bus)
            } else {
                SystemEnvironment::desktop_connected()
            };
            let _ = refresh_availability_unlocked(&core_bg, true);
            let (state_dto, tray_state) = {
                let mut guard = core_bg.lock_unpoisoned();
                guard.system_env = new_env;
                guard.sleep_in_progress = false;
                guard.build_state_and_tray()
            };
            tray_bg.set_full_state(tray_state);
            emit_status_changed(&conn_bg, &state_dto);
            drain_queued_backups_async(&core_bg);
            if enable_scheduler {
                tick_scheduler_async(&core_bg);
            }
        });
    }
}

fn check_gui_sender_alive(conn: &Connection, sender: &str) -> Result<bool, zbus::Error> {
    let bus_name = zbus::names::BusName::try_from(sender)
        .map_err(|e| zbus::Error::Failure(format!("invalid bus name `{sender}`: {e}")))?;
    zbus::block_on(async move {
        let proxy = zbus::fdo::DBusProxy::new(conn).await?;
        proxy.name_has_owner(bus_name).await.map_err(Into::into)
    })
}

fn is_gui_live(core: &Arc<Mutex<DaemonCore>>, conn: &Connection) -> bool {
    let sender_opt = {
        let guard = core.lock_unpoisoned();
        guard.registered_gui_sender.clone()
    };
    let Some(sender) = sender_opt else {
        let mut guard = core.lock_unpoisoned();
        guard.window_active = false;
        return false;
    };
    match check_gui_sender_alive(conn, &sender) {
        Ok(true) => true,
        Ok(false) | Err(_) => {
            let mut guard = core.lock_unpoisoned();
            if guard.registered_gui_sender.as_deref() == Some(sender.as_str()) {
                guard.registered_gui_sender = None;
                guard.window_active = false;
            }
            false
        }
    }
}

fn spawn_gui_process(kestic_binary: &std::path::Path, bus_name: &str, confirm_quit: bool) {
    let mut cmd = Command::new(kestic_binary);
    if bus_name != KESTIC_BUS_NAME {
        cmd.arg("--bus-name").arg(bus_name);
    }
    if confirm_quit {
        cmd.arg("--confirm-quit");
    }
    // stderr stays the daemon's, so GUI warnings land in the same journal as the daemon's own.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Ok(mut child) = cmd.spawn() {
        thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

fn handle_tray_action(
    core: &Arc<Mutex<DaemonCore>>,
    conn: &Connection,
    config: &DaemonRunConfig,
    action: TrayAction,
) {
    match action {
        TrayAction::StartBackup(backup_id) => {
            let core_bg = Arc::clone(core);
            thread::spawn(move || {
                let _ = start_backup_async(&core_bg, &backup_id);
            });
        }
        TrayAction::StartAllBackups => {
            let core_bg = Arc::clone(core);
            thread::spawn(move || {
                let res = start_all_backups_async(&core_bg);
                if !res.skipped.is_empty() {
                    let notif_settings = {
                        let guard = core_bg.lock_unpoisoned();
                        guard.config.notifications.clone()
                    };
                    let summary: Vec<String> = res
                        .skipped
                        .iter()
                        .map(|s| {
                            let reason =
                                tr_text(&s.reason.text(s.error_kind, s.error_detail.as_deref()));
                            format!("• {}: {reason}", s.backup_name)
                        })
                        .collect();
                    let title = i18n("Some backups were skipped").to_string();
                    let body = summary.join("\n");
                    let _ = send_configured_notification(
                        &notif_settings,
                        KesticEvent::ManualRunBlocked,
                        &title,
                        &body,
                    );
                }
            });
        }
        TrayAction::CancelRun(run_id) => {
            let guard = core.lock_unpoisoned();
            let _ = guard.cancel_run(&run_id);
        }
        TrayAction::PauseGlobal(duration) => {
            if let Ok(dur_json) = serde_json::to_string(&duration) {
                let mut pair = None;
                let tray_opt = {
                    let mut guard = core.lock_unpoisoned();
                    if guard.pause_global(&dur_json).is_ok() {
                        pair = Some(guard.build_state_and_tray());
                    }
                    guard.tray.clone()
                };
                if let Some((dto, ts)) = pair {
                    if let Some(tray) = tray_opt {
                        tray.set_full_state(ts);
                    }
                    emit_status_changed(conn, &dto);
                }
            }
        }
        TrayAction::ResumeGlobal => {
            let mut pair = None;
            let tray_opt = {
                let mut guard = core.lock_unpoisoned();
                if guard.resume_global().is_ok() {
                    pair = Some(guard.build_state_and_tray());
                }
                guard.tray.clone()
            };
            if let Some((dto, ts)) = pair {
                if let Some(tray) = tray_opt {
                    tray.set_full_state(ts);
                }
                emit_status_changed(conn, &dto);
            }
            let core_bg = Arc::clone(core);
            thread::spawn(move || {
                drain_queued_backups_async(&core_bg);
            });
        }
        TrayAction::OpenWindow => {
            let gui_live = is_gui_live(core, conn);
            let (kestic_binary, suppress) = {
                let guard = core.lock_unpoisoned();
                (
                    guard.kestic_binary.clone(),
                    guard.suppress_ui_spawn || config.suppress_ui_spawn,
                )
            };
            emit_show_window_requested(conn);
            if !gui_live && !suppress {
                spawn_gui_process(&kestic_binary, &config.bus_name, false);
            }
        }
        TrayAction::RequestQuit => {
            // Never terminate immediately; ask user via warning confirmation first.
            let gui_live = is_gui_live(core, conn);
            let (has_running, running_name, kestic_binary, suppress) = {
                let guard = core.lock_unpoisoned();
                let first_active = guard.active_runs.values().next();
                (
                    first_active.is_some(),
                    first_active.map_or_default(|a| a.backup_name.clone()),
                    guard.kestic_binary.clone(),
                    guard.suppress_ui_spawn || config.suppress_ui_spawn,
                )
            };
            emit_quit_confirmation_requested(conn, has_running, &running_name);
            if !gui_live && !suppress {
                spawn_gui_process(&kestic_binary, &config.bus_name, true);
            }
        }
    }
}
