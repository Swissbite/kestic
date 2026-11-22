// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M5-S02, runner side of `AC-M5-02-2` (d): the S3 keys reach restic only as environment
//! variables, never as arguments, both for runs and for the setup probe; a restic stand-in
//! records its arguments and environment.

use kestic_core::backend_credentials::{
    BackendCredentials, SECRET_KIND_S3_ACCESS_KEY, SECRET_KIND_S3_SECRET_KEY,
};
use kestic_core::clock::FakeClock;
use kestic_core::destination::{DestinationSetupMode, setup_destination};
use kestic_core::model::{Destination, DestinationKind};
use kestic_core::provider::ResolvedRestic;
use kestic_core::restic::{
    ResticCommandSpec, ResticRunError, ResticRuntime, SECRET_KIND_REPO, run_restic,
};
use kestic_core::secrets::{InMemorySecretStore, SecretStore, SecretValue};
use kestic_test_support::password::strong_password;
use kestic_test_support::tools::write_executable;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use tempfile::TempDir;
use time::macros::datetime;

const ACCESS_KEY: &str = "KESTICRUNNERACCESS1";
const SECRET_KEY: &str = "runner-secret-key-0123456789";

/// restic stand-in: writes its arguments (one per line) to `args.log`, its environment to
/// `env.log` and prints an empty repository config.
fn fake_restic(dir: &Path) -> ResolvedRestic {
    let script = dir.join("restic");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{args}'\nenv > '{env}'\necho '{{}}'\n",
            args = dir.join("args.log").display(),
            env = dir.join("env.log").display()
        ),
    );
    ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    }
}

/// A plain HTTP answer on a loopback port, so the pre-check of the setup probe passes.
fn http_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            use std::io::{Read, Write};
            let mut stream = stream;
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn s3_destination(endpoint: &str) -> Destination {
    Destination::new(
        "Cloud",
        DestinationKind::S3 {
            endpoint: endpoint.to_string(),
            bucket: "kestic-backup".to_string(),
            path: Some("laptop".to_string()),
            region: Some("eu-central-1".to_string()),
        },
    )
    .expect("S3 destination")
}

fn read(path: PathBuf) -> String {
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn assert_keys_only_in_env(dir: &Path, what: &str) {
    let args = read(dir.join("args.log"));
    let env = read(dir.join("env.log"));
    assert!(!args.contains(ACCESS_KEY), "{what}: access key in argv");
    assert!(!args.contains(SECRET_KEY), "{what}: secret key in argv");
    assert!(
        args.lines()
            .any(|a| a.starts_with("s3:http://127.0.0.1:") && a.ends_with("/kestic-backup/laptop")),
        "{what}: repository address in argv: {args}"
    );
    assert!(
        env.lines()
            .any(|l| l == format!("AWS_ACCESS_KEY_ID={ACCESS_KEY}")),
        "{what}: access key in the environment"
    );
    assert!(
        env.lines()
            .any(|l| l == format!("AWS_SECRET_ACCESS_KEY={SECRET_KEY}")),
        "{what}: secret key in the environment"
    );
    assert!(
        env.lines().any(|l| l == "AWS_DEFAULT_REGION=eu-central-1"),
        "{what}: region in the environment"
    );
}

/// AC-M5-02-2 (d), runner: a run gets the stored S3 keys and the region as environment
/// variables, never as arguments; the command record and the shown command name the variables
/// without values.
#[test]
fn ac_m5_02_2_runner_passes_s3_keys_only_as_environment_variables() {
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let store = InMemorySecretStore::new();
    let dest = s3_destination("http://127.0.0.1:9");
    let id = dest.id.as_str();
    store
        .set_secret(
            id,
            SECRET_KIND_REPO,
            &SecretValue::from_str_secret(strong_password()),
        )
        .expect("store repo password");
    store
        .set_secret(
            id,
            SECRET_KIND_S3_ACCESS_KEY,
            &SecretValue::from_str_secret(ACCESS_KEY),
        )
        .expect("store access key");
    store
        .set_secret(
            id,
            SECRET_KIND_S3_SECRET_KEY,
            &SecretValue::from_str_secret(SECRET_KEY),
        )
        .expect("store secret key");
    let mut spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        dir.path().join("kestic"),
        &dest,
        vec!["snapshots".to_string()],
    );
    spec.low_priority = false;
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));

    let res = run_restic(&spec, &dest, Some(&store), &clock, None, &[], |_| {})
        .expect("run the stand-in");

    assert_eq!(res.exit_code, 0);
    assert_keys_only_in_env(dir.path(), "run");
    let record_args = res.command_record.args.join(" ");
    assert!(
        !record_args.contains(ACCESS_KEY),
        "access key in the command record"
    );
    assert!(
        !record_args.contains(SECRET_KEY),
        "secret key in the command record"
    );
    for name in [
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_DEFAULT_REGION",
    ] {
        assert!(
            res.command_record.env_keys.iter().any(|k| k == name),
            "command record lists {name}: {:?}",
            res.command_record.env_keys
        );
    }
    let shown = spec.to_transparent_shell_command();
    assert!(
        shown.contains(&format!(
            "AWS_SECRET_ACCESS_KEY=\"$(kestic secret get {id} s3_secret_key)\""
        )),
        "shown command reads the secret key from the password store: {shown}"
    );
    assert!(
        !shown.contains(ACCESS_KEY),
        "access key in the shown command: {shown}"
    );
    assert!(
        !shown.contains(SECRET_KEY),
        "secret key in the shown command: {shown}"
    );
}

/// AC-M5-02-2 (d), runner: without a stored secret key a run does not start restic.
#[test]
fn ac_m5_02_2_runner_refuses_to_start_without_stored_s3_keys() {
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let store = InMemorySecretStore::new();
    let dest = s3_destination("http://127.0.0.1:9");
    store
        .set_secret(
            dest.id.as_str(),
            SECRET_KIND_S3_ACCESS_KEY,
            &SecretValue::from_str_secret(ACCESS_KEY),
        )
        .expect("store access key");
    let spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        dir.path().join("kestic"),
        &dest,
        vec!["snapshots".to_string()],
    );
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));

    let err = run_restic(&spec, &dest, Some(&store), &clock, None, &[], |_| {})
        .expect_err("run without secret key");

    assert!(
        matches!(
            err,
            ResticRunError::MissingBackendSecret {
                kind: SECRET_KIND_S3_SECRET_KEY,
                ..
            }
        ),
        "{err}"
    );
    assert!(!dir.path().join("args.log").exists(), "restic started");
}

/// AC-M5-02-2 (c)/(d), setup: connecting an S3 destination hands the entered keys to restic as
/// environment variables only and saves them in the password store after success.
#[test]
fn ac_m5_02_2_setup_passes_s3_keys_only_as_environment_and_saves_them() {
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let store = InMemorySecretStore::new();
    let dest = s3_destination(&http_endpoint());
    let mut creds = BackendCredentials::none();
    creds.set(
        SECRET_KIND_S3_ACCESS_KEY,
        Some(SecretValue::from_str_secret(ACCESS_KEY)),
    );
    creds.set(
        SECRET_KIND_S3_SECRET_KEY,
        Some(SecretValue::from_str_secret(SECRET_KEY)),
    );
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let helper = dir.path().join("kestic");
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
        cache_dir: None,
    };

    setup_destination(
        &dest,
        DestinationSetupMode::ConnectExisting,
        &SecretValue::from_str_secret(strong_password()),
        &creds,
        &runtime,
    )
    .expect("setup with the stand-in");

    assert_keys_only_in_env(dir.path(), "setup");
    let stored = |kind: &str| {
        store
            .get_secret(dest.id.as_str(), kind)
            .expect("read store")
            .map(|v| v.as_str().expect("utf-8").to_string())
    };
    assert_eq!(
        stored(SECRET_KIND_S3_ACCESS_KEY).as_deref(),
        Some(ACCESS_KEY)
    );
    assert_eq!(
        stored(SECRET_KIND_S3_SECRET_KEY).as_deref(),
        Some(SECRET_KEY)
    );
}
