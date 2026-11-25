// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Fixtures shared by the Kestic integration tests: workspace lookups, test tools, a private
//! session bus, a headless Secret Service, generated test passwords, prepared restic
//! repositories, a reader for the German catalog and the offscreen GUI smoke harness. Only a
//! dev-dependency; never part of a shipped binary.

pub mod bus;
pub mod gui;
pub mod keyring;
pub mod password;
pub mod po;
mod random;
pub mod repo_template;
pub mod tools;
pub mod workspace;
