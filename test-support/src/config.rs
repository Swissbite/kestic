// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading the configuration a test daemon or test run wrote.

use kestic_core::config::{AppConfig, ConfigError, load_optional_config};
use std::path::Path;

/// Loads an existing `config.toml` through the production loader (`load_optional_config`); a
/// missing file is an error.
pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
    load_optional_config(path)?
        .ok_or_else(|| ConfigError::Io(std::io::Error::from(std::io::ErrorKind::NotFound)))
}
