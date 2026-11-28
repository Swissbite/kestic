// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use kestic_core::backend_credentials::BackendCredentials;
use kestic_core::destination::{
    DestinationError, DestinationSetupMode, preflight_check_destination,
    remove_destination_secrets, setup_destination, verify_and_update_stored_password_with_cache,
};
use kestic_core::model::{Destination, DestinationKind};
use kestic_core::restic::{ResticCommandSpec, run_restic};
use kestic_core::secrets::{SecretStore, SecretValue};
use kestic_test_support::password::{assert_no_secret, strong_password};
use std::fs;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use time::macros::datetime;

#[test]
fn ac_m1_06_1_06_3_06_4_06_7_local_destination_init_connect_and_missing() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let repo_dir = dir.path().join("local-repo");
        fs::create_dir_all(&repo_dir).expect("mkdir");
        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-12-01 10:00:00 UTC));
        let runtime = rig.runtime();

        let dest = Destination::new(
            "Local Disk",
            DestinationKind::Local {
                path: repo_dir.clone(),
            },
        )
        .expect("dest");
        let pw = SecretValue::from_str_secret(strong_password());

        // AC-M1-06-1: init empty folder -> repo exists and `restic snapshots` exits 0.
        setup_destination(
            &dest,
            DestinationSetupMode::InitNew,
            &pw,
            &BackendCredentials::none(),
            &runtime,
        )
        .unwrap_or_else(|e| panic!("{label}: local init failed: {e}"));

        let snap_spec = ResticCommandSpec::for_destination(
            &rig.resolved.binary,
            &rig.helper,
            &dest,
            vec!["snapshots".to_string()],
        )
        .with_cache_dir(Some(&cache_dir));
        let snap_res = run_restic(
            &snap_spec,
            &dest,
            Some(&rig.store),
            &rig.clock,
            None,
            &[],
            |_| {},
        )
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
        let wrong_pw = SecretValue::from_str_secret(strong_password());
        let err = setup_destination(
            &dest_wrong,
            DestinationSetupMode::ConnectExisting,
            &wrong_pw,
            &BackendCredentials::none(),
            &runtime,
        )
        .expect_err("wrong password must fail");
        assert!(
            matches!(err, DestinationError::WrongPassword),
            "{label}: expected WrongPassword, got {err:?}"
        );
        assert_eq!(
            rig.store
                .get_secret(dest_wrong.id.as_str(), "repo")
                .expect("get"),
            None,
            "failed connect must not store secret"
        );

        // AC-M1-06-4: connect existing repo with right password -> succeeds without init.
        let dest_ok = Destination::new(
            "Right Pw Connect",
            DestinationKind::Local { path: repo_dir },
        )
        .expect("dest");
        setup_destination(
            &dest_ok,
            DestinationSetupMode::ConnectExisting,
            &pw,
            &BackendCredentials::none(),
            &runtime,
        )
        .unwrap_or_else(|e| panic!("{label}: connect failed: {e}"));
        assert_eq!(
            rig.store
                .get_secret(dest_ok.id.as_str(), "repo")
                .expect("get"),
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
            &BackendCredentials::none(),
            &runtime,
        )
        .expect_err("missing folder must fail preflight");
        assert!(matches!(missing_err, DestinationError::Unreachable(_)));

        // AC-M1-19-3 helper: removing destination secrets deletes them cleanly.
        remove_destination_secrets(&dest_ok, &rig.store).expect("remove secrets");
        assert_eq!(
            rig.store
                .get_secret(dest_ok.id.as_str(), "repo")
                .expect("get"),
            None
        );
    }
}

#[test]
fn ac_m1_06_2_06_5_06_6_rest_destination_auth_and_unreachable_preflight() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let server_dir = dir.path().join("rest-data");
        let server = kestic_test_support::tools::RestServerGuard::start_with_auth(&server_dir);

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-12-01 10:00:00 UTC));
        let runtime = rig.runtime();

        // AC-M1-06-2: REST init with separate REST credentials (`RESTIC_REST_USERNAME`/`PASSWORD`).
        let dest = Destination::new(
            "REST Auth",
            DestinationKind::Rest {
                url: server.url.clone(),
                username: Some("alice".to_string()),
            },
        )
        .expect("dest");
        let repo_pw_text = strong_password();
        let repo_pw = SecretValue::from_str_secret(&repo_pw_text);
        let rest_pw = SecretValue::from_str_secret(server.password());

        setup_destination(
            &dest,
            DestinationSetupMode::InitNew,
            &repo_pw,
            &BackendCredentials::rest(Some(rest_pw.clone())),
            &runtime,
        )
        .unwrap_or_else(|e| panic!("{label}: REST init failed: {e}"));

        let repository_uri = dest.kind.repository_uri();
        assert_no_secret(
            &repository_uri,
            &repo_pw_text,
            "repository URI (repo password)",
        );
        assert_no_secret(
            &repository_uri,
            server.password(),
            "repository URI (REST password)",
        );

        // AC-M1-06-6: Wrong REST credentials -> AuthFailed, secrets not saved.
        let bad_auth_dest = Destination::new(
            "REST Bad Auth",
            DestinationKind::Rest {
                url: server.url.clone(),
                username: Some("alice".to_string()),
            },
        )
        .expect("dest");
        let wrong_rest_pw = SecretValue::from_str_secret(strong_password());
        let auth_err = setup_destination(
            &bad_auth_dest,
            DestinationSetupMode::ConnectExisting,
            &repo_pw,
            &BackendCredentials::rest(Some(wrong_rest_pw)),
            &runtime,
        )
        .expect_err("wrong REST auth must fail");
        assert!(
            matches!(auth_err, DestinationError::AuthFailed(_)),
            "{label}: expected AuthFailed, got {auth_err:?}"
        );
        assert_eq!(
            rig.store
                .get_secret(bad_auth_dest.id.as_str(), "repo")
                .expect("get"),
            None
        );

        // AC-M1-06-5: Stopped/unreachable REST server -> preflight fails cleanly <= 15 s.
        drop(server);
        let closed_port = kestic_test_support::tools::free_loopback_port();
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
            &BackendCredentials::rest(Some(rest_pw)),
            &runtime,
        )
        .expect_err("stopped server must fail");
        assert!(start.elapsed().as_secs() <= 15);
        assert!(matches!(down_err, DestinationError::Unreachable(_)));

        // IDN / Umlaut hostnames must be translated to Punycode instead of failing URI parsing.
        let idn_dest = Destination::new(
            "REST IDN",
            DestinationKind::Rest {
                url: "http://ümlaut-test.invalid:1/pfäd".to_string(),
                username: None,
            },
        )
        .expect("idn dest");
        let idn_err = preflight_check_destination(&idn_dest, Duration::from_secs(2))
            .expect_err("nonexistent .invalid host should fail network/DNS, not URI syntax");
        let idn_msg = idn_err.to_string();
        assert!(
            !idn_msg.contains("invalid uri character"),
            "IDN URL must be Punycode-encoded before ureq call; got: {idn_msg}"
        );
    }
}

#[test]
fn ac_m1_20_1_20_2_20_3_verify_and_update_password_without_key_mutation() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let repo_dir = dir.path().join("repo");
        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-12-01 10:00:00 UTC));

        // A prepared repository (setup only), then simulate a stale password stored in Secret
        // Service.
        let (dest, real_pw_text) = rig.local_repo("Local", &repo_dir);
        let real_pw = SecretValue::from_str_secret(&real_pw_text);
        let wrong_stored_pw = SecretValue::from_str_secret(strong_password());
        rig.store
            .set_secret(dest.id.as_str(), "repo", &wrong_stored_pw)
            .expect("overwrite stale");

        // Capture `restic key list --json` before password update (AC-M1-20-3).
        let keys_before = std::process::Command::new(&rig.resolved.binary)
            .args(["-r", &repo_dir.to_string_lossy(), "key", "list", "--json"])
            .env("RESTIC_PASSWORD", &real_pw_text)
            .env("RESTIC_CACHE_DIR", &cache_dir)
            .output()
            .expect("key list before");
        assert!(keys_before.status.success());

        // AC-M1-20-2: Wrong candidate keeps stored password unchanged.
        let wrong_candidate = SecretValue::from_str_secret(strong_password());
        let err = verify_and_update_stored_password_with_cache(
            &dest,
            &wrong_candidate,
            &rig.store,
            &rig.resolved,
            &rig.helper,
            &rig.clock,
            Some(&cache_dir),
        )
        .expect_err("wrong candidate must fail");
        assert!(matches!(err, DestinationError::WrongPassword), "{label}");
        assert_eq!(
            rig.store.get_secret(dest.id.as_str(), "repo").expect("get"),
            Some(wrong_stored_pw)
        );

        // AC-M1-20-1: Correct candidate updates Secret Service without touching repo keys.
        verify_and_update_stored_password_with_cache(
            &dest,
            &real_pw,
            &rig.store,
            &rig.resolved,
            &rig.helper,
            &rig.clock,
            Some(&cache_dir),
        )
        .unwrap_or_else(|e| panic!("{label}: update stored pw failed: {e}"));
        assert_eq!(
            rig.store.get_secret(dest.id.as_str(), "repo").expect("get"),
            Some(real_pw)
        );

        let keys_after = std::process::Command::new(&rig.resolved.binary)
            .args(["-r", &repo_dir.to_string_lossy(), "key", "list", "--json"])
            .env("RESTIC_PASSWORD", &real_pw_text)
            .env("RESTIC_CACHE_DIR", &cache_dir)
            .output()
            .expect("key list after");
        assert_eq!(
            keys_before.stdout, keys_after.stdout,
            "{label}: keys mutated!"
        );
    }
}

#[test]
fn rest_preflight_handles_non_ascii_redirect_location_and_https_tls_without_panic() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // 1. A 3xx redirect carrying an unescaped UTF-8 Location header still proves TCP+HTTP
    //    reachability and must not fail with `http: invalid uri character`.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock 302 listener");
    let port = listener.local_addr().expect("local_addr").port();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: https://synology.d\xc3\xa4ster.ch:6789/tuxedo-dave/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            let _ = stream.flush();
        }
    });

    let redirect_dest = Destination::new(
        "REST Redirect",
        DestinationKind::Rest {
            url: format!("http://127.0.0.1:{port}/tuxedo-dave\u{200B}"),
            username: None,
        },
    )
    .expect("redirect dest");
    preflight_check_destination(&redirect_dest, Duration::from_secs(2))
        .expect("302 with UTF-8 Location must still satisfy HTTP reachability preflight");
    let _ = handle.join();

    // 2. HTTPS preflight must configure NativeTls + PlatformVerifier without panicking on `https://`.
    let closed_port = kestic_test_support::tools::free_loopback_port();
    let https_dest = Destination::new(
        "REST HTTPS",
        DestinationKind::Rest {
            url: format!("https://127.0.0.1:{closed_port}/tuxedo-dave"),
            username: None,
        },
    )
    .expect("https dest");
    let err = preflight_check_destination(&https_dest, Duration::from_secs(2))
        .expect_err("closed HTTPS port must return Unreachable instead of panicking");
    assert!(matches!(err, DestinationError::Unreachable(_)));
}
