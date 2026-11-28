// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The revision of a saved backup or destination: a check value over its stored form. Two
//! clients that read the same revision saw the same backup; a save that names an older revision
//! would overwrite a change it never saw. The value is not stored, it is the same for the same
//! content (also after a restart), and secrets are not part of it.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The revision of `value`: SHA-256 over its JSON form, as lowercase hex.
pub fn revision_of<T: Serialize>(value: &T) -> String {
    // A backup or destination always serialises; an error would only come from a map with
    // non-string keys, which neither has.
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "revision_tests.rs"]
mod tests;
