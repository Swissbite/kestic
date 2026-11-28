// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Core domain logic, restic integration, configuration, and scheduling for Kestic.

pub mod backend_credentials;
pub mod backup;
pub mod backup_progress;
mod bounded_command;
pub mod build_info;
mod child_guard;
pub mod child_registry;
pub mod clock;
pub mod config;
pub mod destination;
pub mod destination_types;
pub mod excludes;
pub mod generic;
pub mod history;
pub mod known_hosts;
pub mod local_path;
pub mod maintenance;
pub mod migrations;
pub mod model;
pub mod mount;
pub mod provider;
pub mod query_list;
pub mod repo_identity;
pub mod restic;
pub mod restic_gate;
pub mod restic_install;
pub mod restic_release;
pub mod restore;
pub mod restore_preview;
pub mod restore_target;
pub mod retention;
pub mod revision;
pub mod s3;
pub mod scheduler;
pub mod secrets;
pub mod sftp;
pub mod snapshot;
pub mod snapshot_diff;
pub mod snapshot_find;
pub mod snapshot_list;
pub mod snapshot_ls;
pub mod sync;
pub mod transfer_speed;
