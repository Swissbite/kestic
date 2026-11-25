// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared helpers for the M5 tests of the restic download: a release mock the daemon of this
//! process trusts.

use kestic_core::restic_release::{TrustAnchor, set_test_anchor};
use kestic_test_support::release_mock::{ReleaseFiles, ReleaseMock, Route, TestSigningKey};

/// Makes the daemon of this process trust `key` and download from `base_url`.
pub fn trust(base_url: &str, key: &TestSigningKey) {
    let key_path = key.public_key_file().to_string_lossy().into_owned();
    let fpr = key.fingerprint().to_string();
    let anchor = TrustAnchor::from_lookup(|name| match name {
        "KESTIC_TEST_RELEASE_URL" => Some(base_url.to_string()),
        "KESTIC_TEST_SIGNING_KEY" => Some(key_path.clone()),
        "KESTIC_TEST_SIGNING_FPR" => Some(fpr.clone()),
        _ => None,
    })
    .expect("test anchor");
    set_test_anchor(Some(anchor));
}

/// A release mock for `files`, trusted by the daemon of this process.
pub fn serve(
    files: &ReleaseFiles,
    key: &TestSigningKey,
    routes: Vec<(String, Route)>,
) -> ReleaseMock {
    let mock = ReleaseMock::start_with(files, routes);
    trust(&mock.base_url(), key);
    mock
}
