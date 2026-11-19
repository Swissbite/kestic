// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Background service (daemon), tray, notifications, and D-Bus server for Kestic.

pub mod binary_watch;
pub mod dbus_api;
pub mod dbus_server;
pub mod i18n;
pub mod inhibit;
pub mod job_view;
pub mod monitors;
pub mod notifications;
pub mod secrets;
pub mod service;
pub mod single_instance;
pub mod tray;

/// Returns the daemon crate version via core.
pub fn daemon_version() -> &'static str {
    kestic_core::version()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_reports_core_version() {
        assert_eq!(daemon_version(), kestic_core::version());
    }
}
