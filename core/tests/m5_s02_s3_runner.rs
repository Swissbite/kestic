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
use kestic_test_support::password::{assert_no_secret, s3_key, strong_password};
use kestic_test_support::tools::write_executable;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use tempfile::TempDir;
use time::macros::datetime;

/// S3 keys generated for one test: the access key in S3 syntax, the secret key as strong test
/// password, so `assert_no_secret` finds it in any form.
struct Keys {
    access: String,
    secret: String,
}

fn keys() -> Keys {
    Keys {
        access: s3_key(20),
        secret: strong_password(),
    }
}

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

fn assert_keys_only_in_env(dir: &Path, keys: &Keys, what: &str) {
    let args = read(dir.join("args.log"));
    let env = read(dir.join("env.log"));
    assert!(!args.contains(&keys.access), "{what}: access key in argv");
    assert_no_secret(&args, &keys.secret, &format!("{what}: argv"));
    assert!(
        args.lines()
            .any(|a| a.starts_with("s3:http://127.0.0.1:") && a.ends_with("/kestic-backup/laptop")),
        "{what}: repository address in argv: {args}"
    );
    assert!(
        env.lines()
            .any(|l| l == format!("AWS_ACCESS_KEY_ID={}", keys.access)),
        "{what}: access key in the environment"
    );
    assert!(
        env.lines()
            .any(|l| l == format!("AWS_SECRET_ACCESS_KEY={}", keys.secret)),
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
    let keys = keys();
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
            &SecretValue::from_str_secret(keys.access.as_str()),
        )
        .expect("store access key");
    store
        .set_secret(
            id,
            SECRET_KIND_S3_SECRET_KEY,
            &SecretValue::from_str_secret(keys.secret.as_str()),
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
    assert_keys_only_in_env(dir.path(), &keys, "run");
    let record_args = res.command_record.args.join(" ");
    assert!(
        !record_args.contains(&keys.access),
        "access key in the command record"
    );
    assert_no_secret(&record_args, &keys.secret, "command record");
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
        "shown command reads the secret key from the password store"
    );
    assert!(
        !shown.contains(&keys.access),
        "access key in the shown command"
    );
    assert_no_secret(&shown, &keys.secret, "shown command");
}

/// AC-M5-02-2 (d), runner: without a stored secret key a run does not start restic.
#[test]
fn ac_m5_02_2_runner_refuses_to_start_without_stored_s3_keys() {
    let dir = TempDir::new().expect("tempdir");
    let keys = keys();
    let resolved = fake_restic(dir.path());
    let store = InMemorySecretStore::new();
    let dest = s3_destination("http://127.0.0.1:9");
    store
        .set_secret(
            dest.id.as_str(),
            SECRET_KIND_S3_ACCESS_KEY,
            &SecretValue::from_str_secret(keys.access.as_str()),
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
    let keys = keys();
    let resolved = fake_restic(dir.path());
    let store = InMemorySecretStore::new();
    let dest = s3_destination(&http_endpoint());
    let mut creds = BackendCredentials::none();
    creds.set(
        SECRET_KIND_S3_ACCESS_KEY,
        Some(SecretValue::from_str_secret(keys.access.as_str())),
    );
    creds.set(
        SECRET_KIND_S3_SECRET_KEY,
        Some(SecretValue::from_str_secret(keys.secret.as_str())),
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

    assert_keys_only_in_env(dir.path(), &keys, "setup");
    let stored = |kind: &str| {
        store
            .get_secret(dest.id.as_str(), kind)
            .expect("read store")
            .map(|v| v.as_str().expect("utf-8").to_string())
    };
    assert!(
        stored(SECRET_KIND_S3_ACCESS_KEY).as_deref() == Some(keys.access.as_str()),
        "access key saved"
    );
    assert!(
        stored(SECRET_KIND_S3_SECRET_KEY).as_deref() == Some(keys.secret.as_str()),
        "secret key saved"
    );
}

/// AWS variables set in the re-executed test process, as if the daemon had inherited them;
/// their generated values must not reach restic for an S3 destination.
const FOREIGN_AWS: [&str; 11] = [
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_SECURITY_TOKEN",
    "AWS_PROFILE",
    "AWS_SHARED_CREDENTIALS_FILE",
    "AWS_CONFIG_FILE",
    "AWS_ROLE_ARN",
    "AWS_WEB_IDENTITY_TOKEN_FILE",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
];

/// The inherited values of [`FOREIGN_AWS`] in this (re-executed) process.
fn foreign_values() -> Vec<(&'static str, String)> {
    FOREIGN_AWS
        .iter()
        .map(|name| (*name, std::env::var(name).expect("inherited AWS variable")))
        .collect()
}

/// The value restic saw for `name`, if any.
fn env_value(env: &str, name: &str) -> Option<String> {
    env.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}=")).map(ToOwned::to_owned))
}

/// M5-S02, S3 environment (restic-integration.md §Credentials): AWS variables inherited from
/// the daemon's environment do not reach restic, neither in a run nor in the setup probe; the
/// keys and the region are Kestic's own, and without a configured region restic gets none.
#[test]
fn m5_02_inherited_aws_variables_do_not_reach_restic() {
    const INNER: &str = "KESTIC_M5_02_AWS_ENV_INNER";
    if std::env::var_os(INNER).is_none() {
        let mut cmd = std::process::Command::new(std::env::current_exe().expect("test binary"));
        cmd.args([
            "m5_02_inherited_aws_variables_do_not_reach_restic",
            "--exact",
        ])
        .env(INNER, "1");
        for name in FOREIGN_AWS {
            cmd.env(name, s3_key(24));
        }
        let out = cmd
            .output()
            .expect("run the test with inherited AWS variables");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "inner test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }

    let foreign = foreign_values();
    let dir = TempDir::new().expect("tempdir");
    let keys = keys();
    let resolved = fake_restic(dir.path());
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let helper = dir.path().join("kestic");

    // A run of a destination with region.
    let store = InMemorySecretStore::new();
    let dest = s3_destination("http://127.0.0.1:9");
    let id = dest.id.as_str();
    for (kind, value) in [
        (SECRET_KIND_S3_ACCESS_KEY, keys.access.as_str()),
        (SECRET_KIND_S3_SECRET_KEY, keys.secret.as_str()),
    ] {
        store
            .set_secret(id, kind, &SecretValue::from_str_secret(value))
            .expect("store key");
    }
    let spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        &helper,
        &dest,
        vec!["snapshots".to_string()],
    );
    run_restic(&spec, &dest, Some(&store), &clock, None, &[], |_| {}).expect("run");
    let run_env = read(dir.path().join("env.log"));
    for (name, value) in &foreign {
        assert!(
            !run_env.contains(value),
            "run: inherited {name} reached restic"
        );
    }
    assert!(
        env_value(&run_env, "AWS_ACCESS_KEY_ID").as_deref() == Some(keys.access.as_str()),
        "run: Kestic's access key"
    );
    assert_eq!(
        env_value(&run_env, "AWS_DEFAULT_REGION").as_deref(),
        Some("eu-central-1"),
        "run: the configured region"
    );

    // The setup probe of a destination without region.
    let mut no_region = s3_destination(&http_endpoint());
    if let DestinationKind::S3 { region, .. } = &mut no_region.kind {
        *region = None;
    }
    let mut creds = BackendCredentials::none();
    creds.set(
        SECRET_KIND_S3_ACCESS_KEY,
        Some(SecretValue::from_str_secret(keys.access.as_str())),
    );
    creds.set(
        SECRET_KIND_S3_SECRET_KEY,
        Some(SecretValue::from_str_secret(keys.secret.as_str())),
    );
    let probe_store = InMemorySecretStore::new();
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &probe_store,
        clock: &clock,
        cache_dir: None,
    };
    setup_destination(
        &no_region,
        DestinationSetupMode::ConnectExisting,
        &SecretValue::from_str_secret(strong_password()),
        &creds,
        &runtime,
    )
    .expect("setup");
    let probe_env = read(dir.path().join("env.log"));
    for (name, value) in &foreign {
        assert!(
            !probe_env.contains(value),
            "probe: inherited {name} reached restic"
        );
    }
    assert_eq!(
        env_value(&probe_env, "AWS_DEFAULT_REGION"),
        None,
        "probe: no region without a configured one"
    );
    assert!(
        env_value(&probe_env, "AWS_SECRET_ACCESS_KEY").as_deref() == Some(keys.secret.as_str()),
        "probe: Kestic's secret key"
    );
}

/// M5-S02, `BackendSecrets` (dbus-api.md: an empty field means no value): an empty REST
/// password counts as absent, so restic gets no `RESTIC_REST_PASSWORD` and nothing is saved.
#[test]
fn m5_02_empty_backend_password_is_absent_and_never_sent() {
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let store = InMemorySecretStore::new();
    let dest = Destination::new(
        "NAS",
        DestinationKind::Rest {
            url: http_endpoint(),
            username: Some(s3_key(8)),
        },
    )
    .expect("REST destination");
    let creds = BackendCredentials::rest(Some(SecretValue::from_bytes(Vec::new())));
    assert!(
        creds
            .get(kestic_core::backend_credentials::SECRET_KIND_REST)
            .is_none(),
        "an empty password is no password"
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
    .expect("setup");

    let env = read(dir.path().join("env.log"));
    assert_eq!(
        env_value(&env, "RESTIC_REST_PASSWORD"),
        None,
        "no REST password sent to restic"
    );
    assert!(
        store
            .get_secret(
                dest.id.as_str(),
                kestic_core::backend_credentials::SECRET_KIND_REST
            )
            .expect("read store")
            .is_none(),
        "no REST password saved"
    );
}

/// M5-S02, error reading: a bucket, path or host in the URL or address of a connection error
/// (`backup-403`, `authentication-logs`, `forbidden/x`, `[fd00::403]`) does not turn it into a
/// rejected login; a real login error still counts as one.
#[test]
fn m5_02_names_in_urls_do_not_read_as_login_errors() {
    use kestic_core::restic::{ResticFailureKind, classify_exit};
    let refused = |url: &str, addr: &str| {
        format!("Stat: Get \"{url}/?location=\": dial tcp {addr}: connect: connection refused")
    };
    for message in [
        refused("http://127.0.0.1:9000/backup-403", "127.0.0.1:9000"),
        refused(
            "http://127.0.0.1:9000/authentication-logs",
            "127.0.0.1:9000",
        ),
        refused("http://127.0.0.1:9000/backup/forbidden/x", "127.0.0.1:9000"),
        refused("http://[fd00::403]:9000/backup", "[fd00::403]:9000"),
        refused("http://127.0.0.1:40312/backup", "127.0.0.1:40312"),
    ] {
        assert_eq!(
            classify_exit(1, Some(&message), Vec::new(), "").kind,
            ResticFailureKind::DestinationUnreachable,
            "{message}"
        );
    }
    assert_eq!(
        classify_exit(
            1,
            Some("Stat: The request signature we calculated does not match the signature you provided."),
            Vec::new(),
            ""
        )
        .kind,
        ResticFailureKind::BackendAuthFailed
    );
    assert_eq!(
        classify_exit(
            1,
            Some("unexpected HTTP response (403): 403 Forbidden"),
            Vec::new(),
            ""
        )
        .kind,
        ResticFailureKind::BackendAuthFailed
    );
}

/// M5-S02, redaction with several backend secrets: a secret that contains a shorter one (an
/// access key inside the secret key) is redacted as a whole, whatever the order of the list.
#[test]
fn m5_02_redaction_handles_a_secret_inside_another() {
    let short = s3_key(12);
    let long = format!("{short}{}", strong_password());
    let text = format!("restic said: {long} and {short}");

    let redacted = kestic_core::restic::redact_secrets(&text, &[&short, &long]);

    assert_no_secret(&redacted, &long, "redacted text");
    assert!(!redacted.contains(&short), "short secret left in the text");
}
