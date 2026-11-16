// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

mod common;

use kestic_core::clock::FakeClock;
use kestic_core::destination::{
    DestinationError, DestinationSetupMode, remove_destination_secrets, setup_destination,
    verify_and_update_stored_password,
};
use kestic_core::model::{Destination, DestinationKind};
use kestic_core::provider::check_binary_version;
use kestic_core::restic::{ResticCommandSpec, run_restic};
use kestic_core::secrets::{FileSecretStore, SecretStore, SecretValue};
use std::fs;
use std::time::Instant;
use tempfile::TempDir;
use time::macros::datetime;

#[test]
fn ac_m1_06_1_06_3_06_4_06_7_local_destination_init_connect_and_missing() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let repo_dir = dir.path().join("local-repo");
        fs::create_dir_all(&repo_dir).expect("mkdir");
        let secrets_hex = dir.path().join("secrets.hex");
        let store = FileSecretStore::new(&secrets_hex);
        let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
        let resolved = check_binary_version(&restic_bin).expect("resolved restic");
        let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));

        let dest = Destination::new(
            "Local Disk",
            DestinationKind::Local {
                path: repo_dir.clone(),
            },
        )
        .expect("dest");
        let pw = SecretValue::from_str_secret("correct-repo-password");

        // AC-M1-06-1: init empty folder -> repo exists and `restic snapshots` exits 0.
        setup_destination(
            &dest,
            DestinationSetupMode::InitNew,
            &pw,
            None,
            &store,
            &resolved,
            &helper,
            &clock,
        )
        .unwrap_or_else(|e| panic!("{label}: local init failed: {e}"));

        let snap_spec = ResticCommandSpec::for_destination(
            &resolved.binary,
            &helper,
            &dest,
            vec!["snapshots".to_string()],
        );
        let snap_res = run_restic(&snap_spec, &dest, Some(&store), &clock, None, &[], |_| {})
            .expect("snapshots");
        assert_eq!(
            snap_res.exit_code, 0,
            "{label}: snapshots after init failed: {:?}\nlogs: {:?}",
            snap_res.presentation, snap_res.log_lines
        );

        // AC-M1-06-3: connect existing repo with wrong password -> WrongPassword, not stored.
        let dest_wrong = Destination::new(
            "Wrong Pw Connect",
            DestinationKind::Local {
                path: repo_dir.clone(),
            },
        )
        .expect("dest");
        let wrong_pw = SecretValue::from_str_secret("definitely-wrong");
        let err = setup_destination(
            &dest_wrong,
            DestinationSetupMode::ConnectExisting,
            &wrong_pw,
            None,
            &store,
            &resolved,
            &helper,
            &clock,
        )
        .expect_err("wrong password must fail");
        assert!(
            matches!(err, DestinationError::WrongPassword),
            "{label}: expected WrongPassword, got {err:?}"
        );
        assert_eq!(
            store
                .get_secret(dest_wrong.id.as_str(), "repo")
                .expect("get"),
            None,
            "failed connect must not store secret"
        );

        // AC-M1-06-4: connect existing repo with right password -> succeeds without init.
        let dest_ok = Destination::new(
            "Right Pw Connect",
            DestinationKind::Local {
                path: repo_dir.clone(),
            },
        )
        .expect("dest");
        setup_destination(
            &dest_ok,
            DestinationSetupMode::ConnectExisting,
            &pw,
            None,
            &store,
            &resolved,
            &helper,
            &clock,
        )
        .unwrap_or_else(|e| panic!("{label}: connect failed: {e}"));
        assert_eq!(
            store.get_secret(dest_ok.id.as_str(), "repo").expect("get"),
            Some(pw.clone())
        );

        // AC-M1-06-7: missing local directory -> preflight fails with Unreachable without running restic.
        let missing_dest = Destination::new(
            "Missing",
            DestinationKind::Local {
                path: dir.path().join("removed-mount"),
            },
        )
        .expect("dest");
        let missing_err = setup_destination(
            &missing_dest,
            DestinationSetupMode::ConnectExisting,
            &pw,
            None,
            &store,
            &resolved,
            &helper,
            &clock,
        )
        .expect_err("missing folder must fail preflight");
        assert!(matches!(missing_err, DestinationError::Unreachable(_)));

        // AC-M1-19-3 helper: removing destination secrets deletes them cleanly.
        remove_destination_secrets(&dest_ok, &store).expect("remove secrets");
        assert_eq!(
            store.get_secret(dest_ok.id.as_str(), "repo").expect("get"),
            None
        );
    }
}

#[test]
fn ac_m1_06_2_06_5_06_6_rest_destination_auth_and_unreachable_preflight() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let server_dir = dir.path().join("rest-data");
        let server = common::start_rest_server_with_auth(&server_dir);

        let secrets_hex = dir.path().join("secrets.hex");
        let store = FileSecretStore::new(&secrets_hex);
        let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
        let resolved = check_binary_version(&restic_bin).expect("resolved restic");
        let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));

        // AC-M1-06-2: REST init with separate REST credentials (`RESTIC_REST_USERNAME`/`PASSWORD`).
        let dest = Destination::new(
            "REST Auth",
            DestinationKind::Rest {
                url: server.url.clone(),
                username: Some("alice".to_string()),
            },
        )
        .expect("dest");
        let repo_pw = SecretValue::from_str_secret("repo-secret-123");
        let rest_pw = SecretValue::from_str_secret("secret");

        setup_destination(
            &dest,
            DestinationSetupMode::InitNew,
            &repo_pw,
            Some(&rest_pw),
            &store,
            &resolved,
            &helper,
            &clock,
        )
        .unwrap_or_else(|e| panic!("{label}: REST init failed: {e}"));

        assert!(!dest.kind.repository_uri().contains("secret"));
        assert!(!dest.kind.repository_uri().contains("repo-secret-123"));

        // AC-M1-06-6: Wrong REST credentials -> AuthFailed, secrets not saved.
        let bad_auth_dest = Destination::new(
            "REST Bad Auth",
            DestinationKind::Rest {
                url: server.url.clone(),
                username: Some("alice".to_string()),
            },
        )
        .expect("dest");
        let wrong_rest_pw = SecretValue::from_str_secret("wrong-http-pass");
        let auth_err = setup_destination(
            &bad_auth_dest,
            DestinationSetupMode::ConnectExisting,
            &repo_pw,
            Some(&wrong_rest_pw),
            &store,
            &resolved,
            &helper,
            &clock,
        )
        .expect_err("wrong REST auth must fail");
        assert!(
            matches!(auth_err, DestinationError::AuthFailed(_)),
            "{label}: expected AuthFailed, got {auth_err:?}"
        );
        assert_eq!(
            store
                .get_secret(bad_auth_dest.id.as_str(), "repo")
                .expect("get"),
            None
        );

        // AC-M1-06-5: Stopped/unreachable REST server -> preflight fails cleanly <= 15 s.
        drop(server);
        let closed_port = common::unused_loopback_port();
        let down_dest = Destination::new(
            "REST Down",
            DestinationKind::Rest {
                url: format!("http://127.0.0.1:{closed_port}/alice"),
                username: Some("alice".to_string()),
            },
        )
        .expect("dest");
        let start = Instant::now();
        let down_err = setup_destination(
            &down_dest,
            DestinationSetupMode::ConnectExisting,
            &repo_pw,
            Some(&rest_pw),
            &store,
            &resolved,
            &helper,
            &clock,
        )
        .expect_err("stopped server must fail");
        assert!(start.elapsed().as_secs() <= 15);
        assert!(matches!(down_err, DestinationError::Unreachable(_)));
    }
}

#[test]
fn ac_m1_20_1_20_2_20_3_verify_and_update_password_without_key_mutation() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let repo_dir = dir.path().join("repo");
        fs::create_dir_all(&repo_dir).expect("mkdir");
        let secrets_hex = dir.path().join("secrets.hex");
        let store = FileSecretStore::new(&secrets_hex);
        let helper = common::write_secret_get_helper(dir.path(), &secrets_hex);
        let resolved = check_binary_version(&restic_bin).expect("resolved restic");
        let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));

        let dest = Destination::new(
            "Local",
            DestinationKind::Local {
                path: repo_dir.clone(),
            },
        )
        .expect("dest");
        let real_pw = SecretValue::from_str_secret("real-repo-pw");
        let wrong_stored_pw = SecretValue::from_str_secret("stale-wrong-pw");

        // Init repo with real password, then simulate stale password stored in Secret Service.
        setup_destination(
            &dest,
            DestinationSetupMode::InitNew,
            &real_pw,
            None,
            &store,
            &resolved,
            &helper,
            &clock,
        )
        .expect("init");
        store
            .set_secret(dest.id.as_str(), "repo", &wrong_stored_pw)
            .expect("overwrite stale");

        // Capture `restic key list --json` before password update (AC-M1-20-3).
        let keys_before = std::process::Command::new(&resolved.binary)
            .args(["-r", &repo_dir.to_string_lossy(), "key", "list", "--json"])
            .env("RESTIC_PASSWORD", "real-repo-pw")
            .output()
            .expect("key list before");
        assert!(keys_before.status.success());

        // AC-M1-20-2: Wrong candidate keeps stored password unchanged.
        let wrong_candidate = SecretValue::from_str_secret("still-wrong");
        let err = verify_and_update_stored_password(
            &dest,
            &wrong_candidate,
            &store,
            &resolved,
            &helper,
            &clock,
        )
        .expect_err("wrong candidate must fail");
        assert!(matches!(err, DestinationError::WrongPassword), "{label}");
        assert_eq!(
            store.get_secret(dest.id.as_str(), "repo").expect("get"),
            Some(wrong_stored_pw)
        );

        // AC-M1-20-1: Correct candidate updates Secret Service without touching repo keys.
        verify_and_update_stored_password(&dest, &real_pw, &store, &resolved, &helper, &clock)
            .unwrap_or_else(|e| panic!("{label}: update stored pw failed: {e}"));
        assert_eq!(
            store.get_secret(dest.id.as_str(), "repo").expect("get"),
            Some(real_pw)
        );

        let keys_after = std::process::Command::new(&resolved.binary)
            .args(["-r", &repo_dir.to_string_lossy(), "key", "list", "--json"])
            .env("RESTIC_PASSWORD", "real-repo-pw")
            .output()
            .expect("key list after");
        assert_eq!(
            keys_before.stdout, keys_after.stdout,
            "{label}: keys mutated!"
        );
    }
}
