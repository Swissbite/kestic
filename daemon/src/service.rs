// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Full Kestic daemon service (`net.daester.Kestic1` D-Bus server, single-writer config/history,
//! async backup/maintenance workers with cancellation & Plasma JobView/Portal Inhibit,
//! 60s availability cache, SNI tray synchronization, and desktop notifications).

mod availability;
mod backup_runner;
mod backups;
mod destination_update;
mod destinations;
mod error;
mod host_keys;
mod maintenance;
mod previews;
mod restic_install;
mod run_errors;
mod runtime;
mod scheduler;
mod state;
mod state_cache;
mod state_summary;
mod supervision;

pub use availability::unlock_destination_async;
pub(crate) use availability::{
    drain_queued_backups_async, refresh_availability_unlocked,
    trigger_background_availability_probe,
};
pub use backup_runner::{start_backup_async, start_backup_override_async};
pub(crate) use backups::get_run_detail_unlocked;
pub(crate) use destination_update::update_destination_unlocked;
pub(crate) use destinations::{
    add_destination_unlocked, list_backup_snapshots_unlocked, remove_destination_unlocked,
    test_destination_connection_unlocked, update_destination_password_unlocked,
};
pub use error::KesticServiceErr;
pub(crate) use host_keys::trust_host_key_unlocked;
pub use maintenance::{run_destination_maintenance_async, start_all_backups_async};
pub(crate) use previews::preview_destination_maintenance_unlocked;
pub use previews::{start_backup_dry_run_async, start_retention_preview_async};
pub(crate) use restic_install::remove_install_leftovers;
pub use restic_install::start_restic_install_async;
pub use runtime::run_daemon;
pub use scheduler::tick_scheduler_async;
pub use state::{
    AVAILABILITY_CACHE_TTL_SECS, DaemonCore, DaemonOutcome, DaemonRunConfig, SimulatedOverallStatus,
};
pub(crate) use state::{refresh_probe_unlocked, set_restic_provider_unlocked};
