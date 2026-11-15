// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Background service (daemon), tray, notifications, and D-Bus server for Kestic.

pub mod binary_watch;
pub mod inhibit;
pub mod job_view;
pub mod notifications;

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
