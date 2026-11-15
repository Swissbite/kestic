// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Desktop notifications over `org.freedesktop.Notifications` via `notify-rust`
//! with `desktop-entry` and `kestic.notifyrc` event hints (F3, U6, U7, AC-M0-11-4).

use notify_rust::{Hint, Notification, Urgency};

/// Application desktop-entry identifier matching `net.daester.Kestic.desktop`.
pub const KESTIC_DESKTOP_ENTRY: &str = "net.daester.Kestic";

/// Event names defined in `packaging/kestic.notifyrc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KesticEvent {
    BackupFailed,
    BackupWarning,
    BackupOverdue,
    MaintenanceFailed,
    ResticUpdateAvailable,
    ManualRunBlocked,
    RestoreFinished,
}

impl KesticEvent {
    /// Event section name in `kestic.notifyrc` (`[Event/<name>]`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BackupFailed => "BackupFailed",
            Self::BackupWarning => "BackupWarning",
            Self::BackupOverdue => "BackupOverdue",
            Self::MaintenanceFailed => "MaintenanceFailed",
            Self::ResticUpdateAvailable => "ResticUpdateAvailable",
            Self::ManualRunBlocked => "ManualRunBlocked",
            Self::RestoreFinished => "RestoreFinished",
        }
    }
}

/// Sends a notification via `org.freedesktop.Notifications` using `notify-rust`
/// with `desktop-entry`, `x-kde-eventId`, and `Urgency::Normal` (U6, U7).
pub fn send_event_notification(
    event: KesticEvent,
    summary: &str,
    body: &str,
) -> notify_rust::error::Result<u32> {
    let handle = Notification::new()
        .appname("Kestic")
        .summary(summary)
        .body(body)
        .icon("kestic")
        .urgency(Urgency::Normal)
        .hint(Hint::DesktopEntry(KESTIC_DESKTOP_ENTRY.to_string()))
        .hint(Hint::Custom(
            "x-kde-eventId".to_string(),
            event.as_str().to_string(),
        ))
        .show()?;
    Ok(handle.id())
}
