// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Core domain logic, restic integration, configuration, and scheduling for Kestic.

pub mod backend_credentials;
pub mod backup;
pub mod backup_progress;
pub mod build_info;
pub mod clock;
pub mod config;
pub mod destination;
pub mod destination_types;
pub mod excludes;
pub mod generic;
pub mod history;
pub mod local_path;
pub mod maintenance;
pub mod migrations;
pub mod model;
pub mod mount;
pub mod provider;
pub mod restic;
pub mod retention;
pub mod s3;
pub mod scheduler;
pub mod secrets;
pub mod snapshot;
pub mod sync;
