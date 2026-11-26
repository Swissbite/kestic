// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M5 milestone review, inherited backend credentials (restic-integration.md §Credentials):
//! variables of the daemon's environment that a backend reads as credentials (`GOOGLE_` for
//! Google Cloud Storage, `ST_` for Swift, `RESTIC_REST_` for REST, `AWS_` for rclone, `MINIO_`
//! for S3) do not reach restic, neither in a run nor in the setup probe; a variable entered in
//! the destination does. Each test re-runs itself with the inherited variables set, and a restic
//! stand-in records its environment. AC-M5-03-9 adds Azure's managed identity, the gcloud
//! configuration and the TLS certificates; AC-M5-03-8 refuses `azure:` and `gs:` destinations
//! without a credential variable entered in them.

use kestic_core::backend_credentials::{
    BackendCredentials, SECRET_KIND_REST, SECRET_KIND_S3_ACCESS_KEY, SECRET_KIND_S3_SECRET_KEY,
};
use kestic_core::clock::FakeClock;
use kestic_core::destination::{DestinationError, DestinationSetupMode, setup_destination};
use kestic_core::generic::{EnvVar, secret_kind};
use kestic_core::model::{Destination, DestinationKind};
use kestic_core::provider::ResolvedRestic;
use kestic_core::restic::{ResticCommandSpec, ResticRunError, ResticRuntime, run_restic};
use kestic_core::secrets::{InMemorySecretStore, SecretStore, SecretValue};
use kestic_test_support::password::{s3_key, strong_password};
use kestic_test_support::tools::write_executable;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::thread;
use tempfile::TempDir;
use time::macros::datetime;

/// Set in the re-run of a test, which then sees the inherited variables.
const INNER: &str = "KESTIC_M5_INHERITED_CREDENTIALS_INNER";

/// Runs the test `name` again in a child process whose environment has `names` set to random
/// values; returns `true` when this call is that child and the test body should run.
fn in_child_with_inherited(name: &str, names: &[&str]) -> bool {
    if std::env::var_os(INNER).is_some() {
        return true;
    }
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("test binary"));
    cmd.args([name, "--exact"]).env(INNER, "1");
    for var in names {
        cmd.env(var, s3_key(24));
    }
    let out = cmd.output().expect("run the test with inherited variables");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "inner test failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

/// The inherited values of `names` in this (re-run) process.
fn inherited(names: &[&'static str]) -> Vec<(&'static str, String)> {
    names
        .iter()
        .map(|name| (*name, std::env::var(name).expect("inherited variable")))
        .collect()
}

/// restic stand-in: writes its environment to `env.log` and prints an empty repository config.
fn fake_restic(dir: &Path) -> ResolvedRestic {
    let script = dir.join("restic");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\n/usr/bin/env > '{env}'\necho '{{}}'\n",
            env = dir.join("env.log").display()
        ),
    );
    ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    }
}

/// A plain HTTP answer on a loopback port, so the reachability check of the setup probe passes.
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

/// The value restic saw for `name`, if any.
fn env_value(env: &str, name: &str) -> Option<String> {
    env.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}=")).map(ToOwned::to_owned))
}

/// The environment restic got in a run of `dest` (credentials from `store`) and in its setup
/// probe (credentials `creds` as typed in the form), in this order.
fn observed_environments(
    dest: &Destination,
    store: &InMemorySecretStore,
    creds: &BackendCredentials,
) -> [(&'static str, String); 2] {
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let helper = dir.path().join("kestic");
    let env_log = dir.path().join("env.log");

    let spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        &helper,
        dest,
        vec!["snapshots".to_string()],
    );
    run_restic(&spec, dest, Some(store), &clock, None, &[], |_| {}).expect("run");
    let run = fs::read_to_string(&env_log).expect("run environment");

    let probe_store = InMemorySecretStore::new();
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &probe_store,
        clock: &clock,
        cache_dir: None,
    };
    setup_destination(
        dest,
        DestinationSetupMode::ConnectExisting,
        &SecretValue::from_str_secret(strong_password()),
        creds,
        &runtime,
    )
    .expect("setup probe");
    let probe = fs::read_to_string(&env_log).expect("probe environment");
    [("run", run), ("probe", probe)]
}

fn var(name: &str, secret: bool, value: Option<&str>) -> EnvVar {
    EnvVar {
        name: name.to_string(),
        secret,
        value: value.map(ToOwned::to_owned),
    }
}

/// A generic destination at `url` with `plain` variables and one secret variable, whose value is
/// in the returned store and credentials.
fn generic_with_secret(
    url: &str,
    plain: &[(&str, &str)],
    secret: (&str, &str),
) -> (Destination, InMemorySecretStore, BackendCredentials) {
    let mut env: Vec<EnvVar> = plain
        .iter()
        .map(|(name, value)| var(name, false, Some(value)))
        .collect();
    env.push(var(secret.0, true, None));
    let dest = Destination::new(
        "Cloud",
        DestinationKind::Generic {
            url: url.to_string(),
            env,
        },
    )
    .expect("generic destination");
    let value = SecretValue::from_str_secret(secret.1);
    let store = InMemorySecretStore::new();
    store
        .set_secret(dest.id.as_str(), &secret_kind(secret.0), &value)
        .expect("store secret");
    let mut creds = BackendCredentials::none();
    creds.set(&secret_kind(secret.0), Some(value));
    (dest, store, creds)
}

/// Asserts (a) that no inherited value reached restic and an inherited variable the destination
/// does not set is gone, and (b) that every `entered` variable has the destination's value.
fn assert_only_entered(
    what: &str,
    env: &str,
    foreign: &[(&'static str, String)],
    entered: &[(&str, &str)],
) {
    for (name, value) in foreign {
        assert!(
            !env.contains(value.as_str()),
            "(a) {what}: inherited {name} reached restic"
        );
        if !entered.iter().any(|(n, _)| n == name) {
            assert!(
                env_value(env, name).is_none(),
                "(a) {what}: inherited {name} is removed"
            );
        }
    }
    for (name, value) in entered {
        assert!(
            env_value(env, name).as_deref() == Some(*value),
            "(b) {what}: {name} entered in the destination"
        );
    }
}

const FOREIGN_GOOGLE: [&str; 4] = [
    "GOOGLE_PROJECT_ID",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_ACCESS_TOKEN",
    "GOOGLE_KESTIC_UNLISTED",
];

/// M5 milestone review, Google Cloud Storage: (a) inherited `GOOGLE_*` variables do not reach
/// restic for a `gs:` destination, (b) the variables entered in the destination do.
#[test]
fn m5_review_gs_inherited_google_variables_do_not_reach_restic() {
    if !in_child_with_inherited(
        "m5_review_gs_inherited_google_variables_do_not_reach_restic",
        &FOREIGN_GOOGLE,
    ) {
        return;
    }
    let foreign = inherited(&FOREIGN_GOOGLE);
    let token = strong_password();
    let entered = [
        ("GOOGLE_PROJECT_ID", "kestic-project"),
        ("GOOGLE_ACCESS_TOKEN", token.as_str()),
    ];
    let (dest, store, creds) =
        generic_with_secret("gs:kestic-bucket:/laptop", &entered[..1], entered[1]);
    for (what, env) in observed_environments(&dest, &store, &creds) {
        assert_only_entered(what, &env, &foreign, &entered);
    }
}

const FOREIGN_SWIFT: [&str; 4] = ["ST_AUTH", "ST_USER", "ST_KEY", "ST_KESTIC_UNLISTED"];

/// M5 milestone review, Swift (v1 login): (a) inherited `ST_*` variables do not reach restic for
/// a `swift:` destination, (b) the variables entered in the destination do.
#[test]
fn m5_review_swift_inherited_st_variables_do_not_reach_restic() {
    if !in_child_with_inherited(
        "m5_review_swift_inherited_st_variables_do_not_reach_restic",
        &FOREIGN_SWIFT,
    ) {
        return;
    }
    let foreign = inherited(&FOREIGN_SWIFT);
    let key = strong_password();
    let entered = [
        ("ST_AUTH", "https://swift.example.com/auth/v1.0"),
        ("ST_USER", "kestic:backup"),
        ("ST_KEY", key.as_str()),
    ];
    let (dest, store, creds) =
        generic_with_secret("swift:kestic-container:/laptop", &entered[..2], entered[2]);
    for (what, env) in observed_environments(&dest, &store, &creds) {
        assert_only_entered(what, &env, &foreign, &entered);
    }
}

const FOREIGN_REST: [&str; 3] = [
    "RESTIC_REST_USERNAME",
    "RESTIC_REST_PASSWORD",
    "RESTIC_REST_KESTIC_UNLISTED",
];

fn rest_destination(url: &str, username: Option<&str>) -> Destination {
    Destination::new(
        "NAS",
        DestinationKind::Rest {
            url: url.to_string(),
            username: username.map(ToOwned::to_owned),
        },
    )
    .expect("REST destination")
}

/// M5 milestone review, REST: (a) inherited `RESTIC_REST_*` variables do not reach restic, also
/// for a destination without a user name, (b) the user name and password of the destination do.
#[test]
fn m5_review_rest_inherited_rest_variables_do_not_reach_restic() {
    if !in_child_with_inherited(
        "m5_review_rest_inherited_rest_variables_do_not_reach_restic",
        &FOREIGN_REST,
    ) {
        return;
    }
    let foreign = inherited(&FOREIGN_REST);
    let endpoint = http_endpoint();

    // Without a user name restic gets no REST login at all.
    let anonymous = rest_destination(&endpoint, None);
    for (what, env) in observed_environments(
        &anonymous,
        &InMemorySecretStore::new(),
        &BackendCredentials::none(),
    ) {
        assert_only_entered(&format!("{what} without user name"), &env, &foreign, &[]);
    }

    let password = strong_password();
    let dest = rest_destination(&endpoint, Some("kestic"));
    let value = SecretValue::from_str_secret(&password);
    let store = InMemorySecretStore::new();
    store
        .set_secret(dest.id.as_str(), SECRET_KIND_REST, &value)
        .expect("store password");
    let entered = [
        ("RESTIC_REST_USERNAME", "kestic"),
        ("RESTIC_REST_PASSWORD", password.as_str()),
    ];
    for (what, env) in observed_environments(&dest, &store, &BackendCredentials::rest(Some(value)))
    {
        assert_only_entered(what, &env, &foreign, &entered);
    }
}

const FOREIGN_AWS_FOR_RCLONE: [&str; 4] = [
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_PROFILE",
    "AWS_SESSION_TOKEN",
];

/// M5 milestone review, rclone (an S3 remote with `env_auth` reads the AWS variables): (a)
/// inherited `AWS_*` variables do not reach restic for an `rclone:` destination, (b) the ones
/// entered in the destination do.
#[test]
fn m5_review_rclone_inherited_aws_variables_do_not_reach_restic() {
    if !in_child_with_inherited(
        "m5_review_rclone_inherited_aws_variables_do_not_reach_restic",
        &FOREIGN_AWS_FOR_RCLONE,
    ) {
        return;
    }
    let foreign = inherited(&FOREIGN_AWS_FOR_RCLONE);
    let secret = strong_password();
    let entered = [
        ("AWS_ACCESS_KEY_ID", "KESTICRCLONEKEY"),
        ("AWS_SECRET_ACCESS_KEY", secret.as_str()),
    ];
    let (dest, store, creds) =
        generic_with_secret("rclone:s3remote:laptop", &entered[..1], entered[1]);
    for (what, env) in observed_environments(&dest, &store, &creds) {
        assert_only_entered(what, &env, &foreign, &entered);
    }
}

const FOREIGN_MINIO: [&str; 5] = [
    "MINIO_ACCESS_KEY",
    "MINIO_SECRET_KEY",
    "MINIO_ROOT_USER",
    "MINIO_ROOT_PASSWORD",
    "MINIO_SHARED_CREDENTIALS_FILE",
];

/// M5 milestone review, S3 (restic also reads MinIO's variables): (a) inherited `MINIO_*`
/// variables do not reach restic for an S3 destination, (b) Kestic's own S3 keys do.
#[test]
fn m5_review_s3_inherited_minio_variables_do_not_reach_restic() {
    if !in_child_with_inherited(
        "m5_review_s3_inherited_minio_variables_do_not_reach_restic",
        &FOREIGN_MINIO,
    ) {
        return;
    }
    let foreign = inherited(&FOREIGN_MINIO);
    let access = s3_key(20);
    let secret = strong_password();
    let dest = Destination::new(
        "Cloud",
        DestinationKind::S3 {
            endpoint: http_endpoint(),
            bucket: "kestic-backup".to_string(),
            path: Some("laptop".to_string()),
            region: None,
        },
    )
    .expect("S3 destination");
    let store = InMemorySecretStore::new();
    let mut creds = BackendCredentials::none();
    for (kind, value) in [
        (SECRET_KIND_S3_ACCESS_KEY, access.as_str()),
        (SECRET_KIND_S3_SECRET_KEY, secret.as_str()),
    ] {
        let value = SecretValue::from_str_secret(value);
        store
            .set_secret(dest.id.as_str(), kind, &value)
            .expect("store key");
        creds.set(kind, Some(value));
    }
    let entered = [
        ("AWS_ACCESS_KEY_ID", access.as_str()),
        ("AWS_SECRET_ACCESS_KEY", secret.as_str()),
    ];
    for (what, env) in observed_environments(&dest, &store, &creds) {
        assert_only_entered(what, &env, &foreign, &entered);
    }
}

const FOREIGN_AZURE_IDENTITY: [&str; 6] = [
    "IDENTITY_ENDPOINT",
    "IDENTITY_HEADER",
    "MSI_ENDPOINT",
    "MSI_SECRET",
    "IMDS_ENDPOINT",
    "AZURE_ACCOUNT_KEY",
];

/// AC-M5-03-9 (NFR-02), Azure: (a) inherited managed-identity variables (`IDENTITY_*`, `MSI_*`,
/// `IMDS_*`) and `AZURE_*` do not reach restic for an `azure:` destination, (b) the variables
/// entered in the destination do, also one with such a prefix.
#[test]
fn ac_m5_03_9_azure_inherited_identity_variables_do_not_reach_restic() {
    if !in_child_with_inherited(
        "ac_m5_03_9_azure_inherited_identity_variables_do_not_reach_restic",
        &FOREIGN_AZURE_IDENTITY,
    ) {
        return;
    }
    let foreign = inherited(&FOREIGN_AZURE_IDENTITY);
    let key = strong_password();
    let entered = [
        ("AZURE_ACCOUNT_NAME", "kestic"),
        ("IMDS_ENDPOINT", "http://127.0.0.1:9"),
        ("AZURE_ACCOUNT_KEY", key.as_str()),
    ];
    let (dest, store, creds) =
        generic_with_secret("azure:kestic-container:/laptop", &entered[..2], entered[2]);
    for (what, env) in observed_environments(&dest, &store, &creds) {
        assert_only_entered(what, &env, &foreign, &entered);
    }
}

const FOREIGN_GCLOUD: [&str; 5] = [
    "CLOUDSDK_CONFIG",
    "CLOUDSDK_AUTH_ACCESS_TOKEN_FILE",
    "GCE_METADATA_HOST",
    "GCE_METADATA_IP",
    "GOOGLE_APPLICATION_CREDENTIALS",
];

/// AC-M5-03-9 (NFR-02), Google Cloud Storage: (a) inherited gcloud and metadata server
/// variables (`CLOUDSDK_*`, `GCE_*`) and `GOOGLE_*` do not reach restic for a `gs:` destination,
/// (b) the variables entered in the destination do, also one with such a prefix.
#[test]
fn ac_m5_03_9_gs_inherited_gcloud_variables_do_not_reach_restic() {
    if !in_child_with_inherited(
        "ac_m5_03_9_gs_inherited_gcloud_variables_do_not_reach_restic",
        &FOREIGN_GCLOUD,
    ) {
        return;
    }
    let foreign = inherited(&FOREIGN_GCLOUD);
    let token = strong_password();
    let entered = [
        ("CLOUDSDK_CONFIG", "/nonexistent/kestic-gcloud"),
        ("GOOGLE_ACCESS_TOKEN", token.as_str()),
    ];
    let (dest, store, creds) =
        generic_with_secret("gs:kestic-bucket:/laptop", &entered[..1], entered[1]);
    for (what, env) in observed_environments(&dest, &store, &creds) {
        assert_only_entered(what, &env, &foreign, &entered);
    }
}

const FOREIGN_TLS: [&str; 2] = ["RESTIC_TLS_CLIENT_CERT", "RESTIC_CACERT"];

/// AC-M5-03-9 (NFR-02), TLS: (a) an inherited `RESTIC_TLS_CLIENT_CERT` and `RESTIC_CACERT` do
/// not reach restic for any destination (REST and the generic form here), (b) the ones entered
/// in a generic destination do.
#[test]
fn ac_m5_03_9_inherited_tls_certificates_do_not_reach_restic() {
    if !in_child_with_inherited(
        "ac_m5_03_9_inherited_tls_certificates_do_not_reach_restic",
        &FOREIGN_TLS,
    ) {
        return;
    }
    let foreign = inherited(&FOREIGN_TLS);
    let rest = rest_destination(&http_endpoint(), None);
    for (what, env) in observed_environments(
        &rest,
        &InMemorySecretStore::new(),
        &BackendCredentials::none(),
    ) {
        assert_only_entered(&format!("REST {what}"), &env, &foreign, &[]);
    }

    let key = strong_password();
    let entered = [
        ("RESTIC_CACERT", "/etc/kestic-test/ca.pem"),
        ("RESTIC_TLS_CLIENT_CERT", "/etc/kestic-test/client.pem"),
        ("B2_ACCOUNT_KEY", key.as_str()),
    ];
    let (dest, store, creds) =
        generic_with_secret("b2:kestic-bucket:laptop", &entered[..2], entered[2]);
    for (what, env) in observed_environments(&dest, &store, &creds) {
        assert_only_entered(&format!("generic {what}"), &env, &foreign, &entered);
    }
}

/// A generic destination at `url` with the plain `variables`, no secret one.
fn generic_plain(url: &str, variables: &[(&str, &str)]) -> Destination {
    Destination::new(
        "Cloud",
        DestinationKind::Generic {
            url: url.to_string(),
            env: variables
                .iter()
                .map(|(name, value)| var(name, false, Some(value)))
                .collect(),
        },
    )
    .expect("generic destination")
}

/// Runs restic for `dest` (credentials from `store`) and its setup probe (credentials `creds`):
/// both must be refused for the credential variables `needs`, and restic must not start.
fn assert_refused(
    what: &str,
    dest: &Destination,
    store: &InMemorySecretStore,
    creds: &BackendCredentials,
    needs: &str,
) {
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let helper = dir.path().join("kestic");
    let spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        &helper,
        dest,
        vec!["snapshots".to_string()],
    );
    let run = run_restic(&spec, dest, Some(store), &clock, None, &[], |_| {});
    assert!(
        matches!(
            &run,
            Err(ResticRunError::CredentialsNotEntered { variables, .. }) if variables == needs
        ),
        "(a) {what}: the run is refused naming {needs}: {run:?}"
    );
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: store,
        clock: &clock,
        cache_dir: None,
    };
    let probe = setup_destination(
        dest,
        DestinationSetupMode::ConnectExisting,
        &SecretValue::from_str_secret(strong_password()),
        creds,
        &runtime,
    );
    assert!(
        matches!(
            &probe,
            Err(DestinationError::Runner(ResticRunError::CredentialsNotEntered { variables, .. }))
                if variables == needs
        ),
        "(a) {what}: the probe is refused naming {needs}: {probe:?}"
    );
    assert!(
        !dir.path().join("env.log").exists(),
        "(a) {what}: restic was started"
    );
}

const AZURE_NEEDS: &str = "AZURE_ACCOUNT_KEY, AZURE_ACCOUNT_SAS, AZURE_FORCE_CLI_CREDENTIAL=true";
const GS_NEEDS: &str = "GOOGLE_ACCESS_TOKEN, GOOGLE_APPLICATION_CREDENTIALS";

/// AC-M5-03-8 (FR-01, NFR-02), core part: (a) a run and the setup probe of an `azure:`
/// destination without `AZURE_ACCOUNT_KEY`, `AZURE_ACCOUNT_SAS` or
/// `AZURE_FORCE_CLI_CREDENTIAL=true`, and of a `gs:` destination without `GOOGLE_ACCESS_TOKEN`
/// or `GOOGLE_APPLICATION_CREDENTIALS`, are refused naming those variables without starting
/// restic, also when the variable is declared but empty, has no saved secret value or
/// `AZURE_FORCE_CLI_CREDENTIAL` is not true; (d) such a destination is still a valid
/// configuration.
#[test]
fn ac_m5_03_8_cloud_destination_without_credential_variable_does_not_start_restic() {
    let none = InMemorySecretStore::new();
    let empty = BackendCredentials::none();
    for (what, dest) in [
        (
            "azure without variables",
            generic_plain("azure:kestic-container:/laptop", &[]),
        ),
        (
            "azure with only the account name",
            generic_plain(
                "azure:kestic-container:/laptop",
                &[("AZURE_ACCOUNT_NAME", "kestic")],
            ),
        ),
        (
            "azure with an empty key",
            generic_plain(
                "azure:kestic-container:/laptop",
                &[("AZURE_ACCOUNT_KEY", " ")],
            ),
        ),
        (
            "azure with the CLI login switched off",
            generic_plain(
                "azure:kestic-container:/laptop",
                &[("AZURE_FORCE_CLI_CREDENTIAL", "false")],
            ),
        ),
    ] {
        assert_refused(what, &dest, &none, &empty, AZURE_NEEDS);
    }
    assert_refused(
        "gs without variables",
        &generic_plain("gs:kestic-bucket:/laptop", &[("GOOGLE_PROJECT_ID", "p")]),
        &none,
        &empty,
        GS_NEEDS,
    );
    let declared = Destination::new(
        "Cloud",
        DestinationKind::Generic {
            url: "gs:kestic-bucket:/laptop".to_string(),
            env: vec![var("GOOGLE_ACCESS_TOKEN", true, None)],
        },
    )
    .expect("generic destination");
    assert_refused(
        "gs with a secret token without value",
        &declared,
        &none,
        &empty,
        GS_NEEDS,
    );

    // (d)
    let mut config = kestic_core::config::AppConfig::default();
    config
        .destinations
        .push(generic_plain("azure:old:/laptop", &[]));
    config
        .destinations
        .push(generic_plain("gs:old:/laptop", &[]));
    let raw = toml::to_string(&config).expect("serialize config");
    let loaded = kestic_core::config::parse_config_str(&raw);
    assert_eq!(
        loaded.map(|c| c.destinations.len()).ok(),
        Some(2),
        "(d) the configuration still loads"
    );
}

/// AC-M5-03-8 (FR-01, NFR-02), core part: (e) a credential variable entered in the destination
/// reaches restic in a run and in the setup probe: `AZURE_FORCE_CLI_CREDENTIAL=true`, a secret
/// `AZURE_ACCOUNT_SAS`, `GOOGLE_APPLICATION_CREDENTIALS`, a secret `GOOGLE_ACCESS_TOKEN`.
#[test]
fn ac_m5_03_8_credential_variable_entered_in_the_destination_reaches_restic() {
    let cli = [("AZURE_FORCE_CLI_CREDENTIAL", "true")];
    let file = [("GOOGLE_APPLICATION_CREDENTIALS", "/home/u/kestic-gcs.json")];
    for (url, entered) in [
        ("azure:kestic-container:/laptop", &cli[..]),
        ("gs:kestic-bucket:/laptop", &file[..]),
    ] {
        let dest = generic_plain(url, entered);
        for (what, env) in observed_environments(
            &dest,
            &InMemorySecretStore::new(),
            &BackendCredentials::none(),
        ) {
            assert_only_entered(&format!("{url} {what}"), &env, &[], entered);
        }
    }
    let sas = strong_password();
    let token = strong_password();
    for (url, secret) in [
        (
            "azure:kestic-container:/laptop",
            ("AZURE_ACCOUNT_SAS", sas.as_str()),
        ),
        (
            "gs:kestic-bucket:/laptop",
            ("GOOGLE_ACCESS_TOKEN", token.as_str()),
        ),
    ] {
        let (dest, store, creds) = generic_with_secret(url, &[], secret);
        for (what, env) in observed_environments(&dest, &store, &creds) {
            assert_only_entered(&format!("{url} {what}"), &env, &[], &[secret]);
        }
    }
}
