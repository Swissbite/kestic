// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Daemon spike service loop combining single-instance D-Bus lock, `ksni` tray,
//! and `notify-rust` notifications (M0-S07).

use crate::notifications::{KesticEvent, send_event_notification};
use crate::single_instance::{InstanceAcquire, KESTIC_BUS_NAME, SingleInstanceLock};
use crate::tray::{KesticTrayState, TrayHandle, TrayStatus};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Configuration for running the M0 daemon prototype.
#[derive(Debug, Clone)]
pub struct DaemonRunConfig {
    pub bus_name: String,
    pub assume_sni_available: bool,
    pub send_test_notification: bool,
    pub tray_state: KesticTrayState,
    pub ready_file: Option<PathBuf>,
    pub duration: Option<Duration>,
}

impl Default for DaemonRunConfig {
    fn default() -> Self {
        Self {
            bus_name: KESTIC_BUS_NAME.to_string(),
            assume_sni_available: true,
            send_test_notification: false,
            tray_state: KesticTrayState {
                status: TrayStatus::Active,
                ..Default::default()
            },
            ready_file: None,
            duration: None,
        }
    }
}

/// Outcome of starting the daemon prototype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonOutcome {
    /// Another daemon already owned the D-Bus name; this instance exited cleanly (AC-M0-07-1).
    AlreadyRunning,
    /// This instance acquired the D-Bus lock, registered the tray, and completed its run.
    RanAndStopped,
}

/// Runs the daemon prototype according to `config`.
pub fn run_daemon(config: DaemonRunConfig) -> Result<DaemonOutcome, String> {
    let lock = match SingleInstanceLock::acquire(&config.bus_name)
        .map_err(|err| format!("failed to acquire D-Bus name `{}`: {err}", config.bus_name))?
    {
        InstanceAcquire::Acquired(lock) => lock,
        InstanceAcquire::AlreadyRunning => return Ok(DaemonOutcome::AlreadyRunning),
    };

    let (quit_tx, quit_rx) = mpsc::channel();
    let tray = TrayHandle::spawn(
        config.tray_state,
        config.assume_sni_available,
        Some(quit_tx),
    )
    .map_err(|err| format!("failed to register StatusNotifierItem tray: {err}"))?;

    if config.send_test_notification {
        send_event_notification(
            KesticEvent::BackupWarning,
            "Kestic daemon test notification",
            "Tray registered and notification channel active.",
        )
        .map_err(|err| format!("failed to send test notification: {err}"))?;
    }

    if let Some(ready_path) = &config.ready_file {
        fs::write(ready_path, "ready\n")
            .map_err(|err| format!("failed to write ready file {}: {err}", ready_path.display()))?;
    }

    let deadline = config.duration.map(|d| Instant::now() + d);
    loop {
        if let Some(end) = deadline {
            let now = Instant::now();
            if now >= end {
                break;
            }
            let wait = (end - now).min(Duration::from_millis(50));
            if quit_rx.recv_timeout(wait).is_ok() {
                break;
            }
        } else if quit_rx.recv_timeout(Duration::from_millis(100)).is_ok() {
            break;
        }
    }

    tray.shutdown();
    drop(lock);
    Ok(DaemonOutcome::RanAndStopped)
}
