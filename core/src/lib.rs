// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Core domain logic, restic integration, configuration, and scheduling for Kestic.

pub mod backup;
pub mod backup_progress;
pub mod build_info;
pub mod clock;
pub mod config;
pub mod destination;
pub mod excludes;
pub mod history;
pub mod maintenance;
pub mod migrations;
pub mod model;
pub mod mount;
pub mod provider;
pub mod query_list;
pub mod restic;
pub mod retention;
pub mod scheduler;
pub mod secrets;
pub mod snapshot;
pub mod snapshot_list;
pub mod snapshot_ls;
pub mod sync;

/// Returns the crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_version_is_non_empty() {
        assert!(!version().is_empty());
    }
}
