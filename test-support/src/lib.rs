// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Fixtures shared by the Kestic integration tests: workspace lookups, test tools, a private
//! session bus, a headless Secret Service, generated test passwords, prepared restic
//! repositories, a mock of restic's release downloads, a reader for the German catalog and the
//! offscreen GUI smoke harness. Only a
//! dev-dependency; never part of a shipped binary.

pub mod bus;
pub mod delayed_daemon;
pub mod gui;
pub mod keyring;
pub mod password;
pub mod po;
mod random;
pub mod release_mock;
pub mod repo_template;
pub mod s3_server;
pub mod sftp_server;
pub mod tools;
pub mod workspace;
