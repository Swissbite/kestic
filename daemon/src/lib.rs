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

/// Version of the binary, from the Git tags of its source tree or `KESTIC_VERSION` (see
/// `build.rs`), e.g. `0.4.0` or `0.4.0~m4.3`.
pub fn daemon_version() -> &'static str {
    env!("KESTIC_VERSION")
}

/// Git commit the binary was built from: short hash, `-dirty` when tracked source files had
/// uncommitted changes, or `unknown` when built outside a Git checkout (see `build.rs`).
pub fn build_commit() -> &'static str {
    env!("KESTIC_BUILD_COMMIT")
}

/// Version and build commit of the running binary.
pub fn build_id() -> kestic_core::build_info::BuildId<'static> {
    kestic_core::build_info::BuildId {
        version: daemon_version(),
        commit: build_commit(),
    }
}

/// Prefix of the daemon's lifecycle lines in the journal, e.g. `kestic-daemon 0.1.0 (a561018)`.
/// The commit tells two builds of the same version apart, before and after a package update.
pub(crate) fn daemon_log_label(version: &str, commit: &str) -> String {
    format!("kestic-daemon {version} ({commit})")
}

/// [`daemon_log_label`] of the running binary, in front of the lines that mark a start or a
/// restart into a replaced binary.
pub fn log_label() -> String {
    daemon_log_label(daemon_version(), build_commit())
}

/// The line the daemon writes once it owns its bus name.
pub(crate) fn startup_line(label: &str, pid: u32) -> String {
    format!("{label}: started (pid {pid})")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The startup line names (a) version, (b) build commit and (c) PID, so the journal shows
    /// which build runs after a package update restarted the daemon with the same PID.
    #[test]
    fn startup_line_names_version_and_build_commit() {
        let line = startup_line(&daemon_log_label("0.1.0", "a561018-dirty"), 4242);
        assert!(
            line.starts_with("kestic-daemon 0.1.0 "),
            "(a) version: {line}"
        );
        assert!(line.contains(" (a561018-dirty): "), "(b) commit: {line}");
        assert!(line.ends_with(": started (pid 4242)"), "(c) pid: {line}");
    }
}
