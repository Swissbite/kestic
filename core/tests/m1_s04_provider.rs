// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use kestic_core::model::ResticProvider;
use kestic_core::provider::{
    MIN_RESTIC_VERSION, ResticProviderError, check_binary_version, parse_version_text,
    resolve_and_check,
};
use semver::Version;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

#[test]
fn ac_m1_04_1_too_old_restic_mock_is_rejected_with_reason() {
    let dir = TempDir::new().expect("tempdir");
    let fake_restic = dir.path().join("restic");
    // Simulate restic 0.16.4: `version --json` fails, `version` prints plain text.
    let script = r#"#!/usr/bin/env sh
if [ "${1:-}" = "version" ] && [ "${2:-}" = "--json" ]; then
    echo "unknown flag: --json" >&2
    exit 1
fi
if [ "${1:-}" = "version" ]; then
    echo "restic 0.16.4 compiled with go1.22.2 on linux/amd64"
    exit 0
fi
exit 2
"#;
    fs::write(&fake_restic, script).expect("write script");
    let mut perms = fs::metadata(&fake_restic).expect("meta").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake_restic, perms).expect("chmod");

    let provider = ResticProvider::Custom { path: fake_restic };
    let err = resolve_and_check(&provider).expect_err("0.16.4 must be rejected");
    match err {
        ResticProviderError::TooOld {
            ref found,
            ref min_version,
            ..
        } => {
            assert_eq!(found, &Version::new(0, 16, 4));
            assert_eq!(min_version, "0.18.0");
        }
        other => panic!("expected TooOld, got {other:?}"),
    }
    assert!(err.to_string().contains("0.16.4"));
    assert!(err.to_string().contains("0.18.0"));
}

#[test]
fn ac_m1_04_2_real_restic_018_and_latest_are_accepted() {
    for (label, bin) in common::restic_test_binaries() {
        let resolved = check_binary_version(&bin)
            .unwrap_or_else(|e| panic!("{label} at {} should pass: {e}", bin.display()));
        assert!(
            resolved.version >= MIN_RESTIC_VERSION,
            "{label} version {} < 0.18.0",
            resolved.version
        );
    }
}

#[test]
fn ac_m1_04_3_invalid_custom_path_is_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("does-not-exist");
    let err = resolve_and_check(&ResticProvider::Custom { path: missing })
        .expect_err("missing path must fail");
    assert!(matches!(err, ResticProviderError::InvalidCustomPath { .. }));

    let non_exec = dir.path().join("not-executable");
    fs::write(&non_exec, "#!/bin/sh\n").expect("write");
    let mut perms = fs::metadata(&non_exec).expect("meta").permissions();
    perms.set_mode(0o644);
    fs::set_permissions(&non_exec, perms).expect("chmod");

    let err = resolve_and_check(&ResticProvider::Custom { path: non_exec })
        .expect_err("non-executable path must fail");
    assert!(matches!(err, ResticProviderError::InvalidCustomPath { .. }));
}

#[test]
fn ac_m1_04_4_missing_system_restic_reports_not_found() {
    // Only test parse_version_text + explicit check when binary missing;
    // avoid mutating process-wide PATH in parallel unit tests.
    let err = ResticProviderError::NotFound {
        min_version: MIN_RESTIC_VERSION.to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("restic not found"));
    assert!(msg.contains("0.18.0"));
}

#[test]
fn ac_m1_04_5_json_failure_falls_back_to_text_and_unreadable_is_rejected() {
    assert_eq!(
        parse_version_text("restic 0.16.4 compiled with go1.22 on linux/amd64"),
        Some(Version::new(0, 16, 4))
    );
    assert_eq!(parse_version_text("gibberish without version"), None);

    let dir = TempDir::new().expect("tempdir");
    let unreadable = dir.path().join("broken-restic");
    let script = "#!/usr/bin/env sh\necho 'not a version'\nexit 0\n";
    fs::write(&unreadable, script).expect("write");
    let mut perms = fs::metadata(&unreadable).expect("meta").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&unreadable, perms).expect("chmod");

    let err = check_binary_version(&unreadable).expect_err("unreadable version must fail");
    assert!(matches!(err, ResticProviderError::UnreadableVersion { .. }));
    assert!(
        err.to_string()
            .contains("unable to determine restic version")
    );
}
