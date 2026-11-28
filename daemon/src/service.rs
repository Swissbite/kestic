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
mod restic_status;
mod restore_preview;
mod restore_runner;
mod run_errors;
mod run_progress;
mod runtime;
mod scheduler;
mod shutdown;
mod snapshot_queries;
mod snapshots;
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
pub(crate) use backups::{
    add_exclude_rule_for_item_unlocked, detect_source_unlocked, format_exclude_rule_for_item,
    get_run_detail_unlocked,
};
pub(crate) use destination_update::update_destination_unlocked;
pub(crate) use destinations::{
    add_destination_unlocked, remove_destination_unlocked, test_destination_connection_unlocked,
    update_destination_password_unlocked,
};
pub use error::KesticServiceErr;
pub(crate) use host_keys::trust_host_key_unlocked;
pub use maintenance::{run_destination_maintenance_async, start_all_backups_async};
pub(crate) use previews::preview_destination_maintenance_unlocked;
pub use previews::{start_backup_dry_run_async, start_retention_preview_async};
pub(crate) use restic_install::remove_install_leftovers;
pub use restic_install::{remove_app_managed_restic, start_restic_install_async};
pub use restore_runner::start_restore_async;
pub use runtime::run_daemon;
pub use scheduler::tick_scheduler_async;
pub use shutdown::install_stop_signal;
pub use snapshot_queries::start_snapshot_query_async;
pub(crate) use snapshots::{list_backup_snapshots_unlocked, list_destination_snapshots_unlocked};
pub use state::{
    AVAILABILITY_CACHE_TTL_SECS, DaemonCore, DaemonOutcome, DaemonRunConfig, SimulatedOverallStatus,
};
pub(crate) use state::{refresh_probe_unlocked, set_restic_provider_unlocked};
