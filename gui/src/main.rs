// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Kestic GUI and CLI entrypoint.

fn main() {
    let _ = kestic_daemon::daemon_version();
}

#[cfg(test)]
mod tests {
    #[test]
    fn gui_links_daemon_and_core() {
        assert_eq!(kestic_daemon::daemon_version(), kestic_core::version());
    }
}
