// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M5-S09 in `core`: the repository ID and its comparison, which saved credentials an edited
//! connection may use, the repository ID per destination in `history.db`, restoring secrets
//! after a failed write, and the stricter rules of the generic form.

use kestic_core::backend_credentials::{
    BackendCredentials, SECRET_KIND_S3_ACCESS_KEY, SECRET_KIND_S3_SECRET_KEY,
};
use kestic_core::clock::FakeClock;
use kestic_core::destination::{DestinationSetupMode, setup_destination};
use kestic_core::generic::{EnvVar, GenericInputError, secret_by_default, validate_url};
use kestic_core::history::RunHistory;
use kestic_core::model::{Destination, DestinationId, DestinationKind};
use kestic_core::provider::ResolvedRestic;
use kestic_core::repo_identity::{
    RepositoryId, RepositoryMatch, connection_changed, decide, same_server,
};
use kestic_core::restic::{ResticRuntime, SECRET_KIND_REPO};
use kestic_core::secrets::{InMemorySecretStore, SecretError, SecretStore, SecretValue};
use kestic_test_support::tools::write_executable;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tempfile::TempDir;
use time::macros::datetime;

const ID_A: &str = "f9af3b260d9f251571b1074e58c999b6da2de7fa3ab782d66ce1cce5340361d3";
const ID_B: &str = "c09bc3df518974dfa721fed3fe84604d99c5e19fb3923b169dcf3a4e5149560f";

fn id(text: &str) -> RepositoryId {
    RepositoryId::parse(text).expect("repository ID")
}

fn s3(endpoint: &str, bucket: &str) -> DestinationKind {
    DestinationKind::S3 {
        endpoint: endpoint.to_string(),
        bucket: bucket.to_string(),
        path: None,
        region: None,
    }
}

fn rest(url: &str) -> DestinationKind {
    DestinationKind::Rest {
        url: url.to_string(),
        username: Some("alice".to_string()),
    }
}

fn generic(url: &str) -> DestinationKind {
    DestinationKind::Generic {
        url: url.to_string(),
        env: vec![EnvVar {
            name: "B2_ACCOUNT_KEY".to_string(),
            secret: true,
            value: None,
        }],
    }
}

/// The output of `restic cat config` (indented JSON) and of `restic init --json` both carry the
/// repository ID; anything else is no ID.
#[test]
fn m5_09_repository_id_from_restic_output() {
    let cat = format!(
        "{{\n  \"version\": 2,\n  \"id\": \"{ID_A}\",\n  \"chunker_polynomial\": \"3dfbbbb116c61d\"\n}}\n"
    );
    assert_eq!(RepositoryId::from_cat_config(&cat), Ok(id(ID_A)));
    let init = format!(
        "{{\"message_type\":\"initialized\",\"id\":\"{ID_B}\",\"repository\":\"/srv/repo\"}}\n"
    );
    assert_eq!(RepositoryId::from_cat_config(&init), Ok(id(ID_B)));
    let noisy = format!("repository 1a2b3c4d opened (version 2)\n{cat}");
    assert_eq!(
        RepositoryId::from_cat_config(&noisy),
        Ok(id(ID_A)),
        "a line before the JSON object"
    );
    assert_eq!(
        id(&ID_A.to_ascii_uppercase()),
        id(ID_A),
        "stored in lowercase"
    );
    assert!(RepositoryId::from_cat_config("{}").is_err(), "no id field");
    assert!(RepositoryId::parse("f9af3b26").is_err(), "short ID");
    assert!(
        RepositoryId::parse(&format!("{}zz", &ID_A[..62])).is_err(),
        "not hex"
    );
}

/// AC-M5-09-3 (c) and AC-M5-05-7 (a), the comparison: only the known ID itself counts as the same
/// repository; without a known ID every repository is another one.
#[test]
fn ac_m5_09_3_decide_same_only_with_the_known_id() {
    assert_eq!(decide(Some(&id(ID_A)), &id(ID_A)), RepositoryMatch::Same);
    assert_eq!(decide(Some(&id(ID_A)), &id(ID_B)), RepositoryMatch::Other);
    assert_eq!(decide(None, &id(ID_A)), RepositoryMatch::Other);
}

/// Whether an edit has to be checked: every change of the address or of what restic gets from the
/// configuration, field by field, never a rename alone.
#[test]
fn m5_09_connection_changed_by_every_field() {
    let local = |p: &str| DestinationKind::Local {
        path: PathBuf::from(p),
    };
    assert!(!connection_changed(&local("/srv/a"), &local("/srv/a")));
    assert!(connection_changed(&local("/srv/a"), &local("/srv/b")));
    let rest_with = |url: &str, user: Option<&str>| DestinationKind::Rest {
        url: url.to_string(),
        username: user.map(ToOwned::to_owned),
    };
    let sftp_with = |host: &str, port: u16, user: &str, path: &str| DestinationKind::Sftp {
        host: host.to_string(),
        port,
        user: user.to_string(),
        path: path.to_string(),
    };
    let s3_with = |path: Option<&str>, region: Option<&str>| DestinationKind::S3 {
        endpoint: "https://s3.example.com".to_string(),
        bucket: "one".to_string(),
        path: path.map(ToOwned::to_owned),
        region: region.map(ToOwned::to_owned),
    };
    let generic_with =
        |url: &str, name: &str, secret: bool, value: Option<&str>| DestinationKind::Generic {
            url: url.to_string(),
            env: vec![EnvVar {
                name: name.to_string(),
                secret,
                value: value.map(ToOwned::to_owned),
            }],
        };
    let base_rest = rest_with("https://nas:8000/r", Some("alice"));
    let base_sftp = sftp_with("nas", 22, "me", "/srv/r");
    let base_s3 = s3_with(Some("laptop"), Some("eu"));
    let base_generic = generic_with("swift:c:/r", "OS_AUTH_URL", false, Some("https://a/v3"));
    let cases = [
        (
            &base_rest,
            rest_with("https://nas:8001/r", Some("alice")),
            "REST URL",
        ),
        (
            &base_rest,
            rest_with("https://nas:8000/r", Some("bob")),
            "REST user",
        ),
        (
            &base_sftp,
            sftp_with("nas2", 22, "me", "/srv/r"),
            "SFTP host",
        ),
        (
            &base_sftp,
            sftp_with("nas", 2222, "me", "/srv/r"),
            "SFTP port",
        ),
        (
            &base_sftp,
            sftp_with("nas", 22, "you", "/srv/r"),
            "SFTP user",
        ),
        (
            &base_sftp,
            sftp_with("nas", 22, "me", "/srv/q"),
            "SFTP path",
        ),
        (&base_s3, s3("https://s3.example.com", "two"), "S3 bucket"),
        (&base_s3, s3_with(Some("desktop"), Some("eu")), "S3 path"),
        (&base_s3, s3_with(Some("laptop"), Some("us")), "S3 region"),
        (
            &base_generic,
            generic_with("swift:d:/r", "OS_AUTH_URL", false, Some("https://a/v3")),
            "generic URL",
        ),
        (
            &base_generic,
            generic_with("swift:c:/r", "OS_REGION", false, Some("https://a/v3")),
            "generic name",
        ),
        (
            &base_generic,
            generic_with("swift:c:/r", "OS_AUTH_URL", false, Some("https://b/v3")),
            "generic value",
        ),
        (
            &base_generic,
            generic_with("swift:c:/r", "OS_AUTH_URL", true, None),
            "generic secret flag",
        ),
    ];
    for (old, new, field) in cases {
        assert!(connection_changed(old, &new), "{field}");
    }
}

/// Saved credentials go only to the server they were entered for.
/// S3: the same endpoint (another bucket is fine); REST: the same host, the port may change;
/// generic form: the same URL up to the path.
#[test]
fn m5_09_same_server_rules_per_destination_kind() {
    // S3
    assert!(same_server(
        &s3("https://s3.example.com:9000", "one"),
        &s3("https://S3.example.com:9000/", "two")
    ));
    assert!(!same_server(
        &s3("https://s3.example.com:9000", "one"),
        &s3("https://evil.example.net:9000", "one")
    ));
    assert!(!same_server(
        &s3("https://s3.example.com:9000", "one"),
        &s3("https://s3.example.com:9001", "one")
    ));
    assert!(
        !same_server(
            &s3("https://s3.example.com", "one"),
            &s3("http://s3.example.com", "one")
        ),
        "S3 downgrade to http"
    );
    assert!(
        same_server(
            &s3("https://s3.example.com", "one"),
            &s3("https://s3.example.com:443", "one")
        ),
        "S3 default port written out"
    );
    // REST
    assert!(same_server(
        &rest("https://nas.local:8000/laptop"),
        &rest("https://NAS.local:8001/laptop")
    ));
    assert!(!same_server(
        &rest("https://nas.local:8000/laptop"),
        &rest("https://other.local:8000/laptop")
    ));
    assert!(
        !same_server(
            &rest("https://nas.local:8000/laptop"),
            &rest("http://nas.local:8000/laptop")
        ),
        "REST downgrade to http"
    );
    assert!(
        !same_server(
            &rest("https://nas.local:8000/laptop"),
            &rest("https://nas.local@evil.example:8000/laptop")
        ),
        "REST user info before another host"
    );
    assert!(
        !same_server(
            &rest("https://nas.local:8000/laptop"),
            &rest("https://nas.local.evil.example:8000/laptop")
        ),
        "REST look-alike host"
    );
    // Generic form
    assert!(same_server(
        &generic("b2:bucket:laptop"),
        &generic("b2:bucket:desktop")
    ));
    assert!(!same_server(
        &generic("b2:bucket:laptop"),
        &generic("b2:other:laptop")
    ));
    assert!(!same_server(&generic("b2:bucket"), &generic("b2:other")));
    assert!(!same_server(
        &generic("rclone::sftp,host=a:path"),
        &generic("rclone::sftp,host=b:path")
    ));
    assert!(!same_server(
        &generic("rclone::s3,endpoint=a:bucket"),
        &generic("rclone::s3,endpoint=evil:bucket")
    ));
    let swift = |auth_url: &str| DestinationKind::Generic {
        url: "swift:container:/laptop".to_string(),
        env: vec![
            EnvVar {
                name: "OS_AUTH_URL".to_string(),
                secret: false,
                value: Some(auth_url.to_string()),
            },
            EnvVar {
                name: "OS_PASSWORD".to_string(),
                secret: true,
                value: None,
            },
        ],
    };
    assert!(same_server(
        &swift("https://keystone/v3"),
        &swift("https://keystone/v3")
    ));
    assert!(
        !same_server(&swift("https://keystone/v3"), &swift("https://evil/v3")),
        "a variable that is not secret can name the server"
    );
    // Another kind is never the same server.
    assert!(!same_server(
        &s3("https://nas.local:8000", "one"),
        &rest("https://nas.local:8000/one")
    ));
}

/// The repository ID is kept per destination in `history.db`, survives reopening and
/// goes with the destination's state row.
#[test]
fn m5_09_repository_id_is_stored_per_destination() {
    let dir = TempDir::new().expect("tempdir");
    let dest = DestinationId::generate().expect("id");
    let other = DestinationId::generate().expect("id");
    {
        let history = RunHistory::open(dir.path()).expect("open");
        assert_eq!(history.repository_id(&dest).expect("read"), None);
        history.set_repository_id(&dest, &id(ID_A)).expect("store");
        history.set_repository_id(&other, &id(ID_B)).expect("store");
    }
    let history = RunHistory::open(dir.path()).expect("reopen");
    assert_eq!(history.repository_id(&dest).expect("read"), Some(id(ID_A)));
    history
        .set_repository_id(&dest, &id(ID_B))
        .expect("replace");
    assert_eq!(history.repository_id(&dest).expect("read"), Some(id(ID_B)));
    history
        .upsert_destination_maintenance_state(&dest, &Default::default())
        .expect("maintenance state");
    assert_eq!(
        history.repository_id(&dest).expect("read"),
        Some(id(ID_B)),
        "the maintenance state leaves the ID alone"
    );
    history.forget_repository_id(&other).expect("forget");
    assert_eq!(
        history.repository_id(&other).expect("read"),
        None,
        "a forgotten ID is gone"
    );
    history
        .set_repository_id(&other, &id(ID_B))
        .expect("store again");
    history.clear_destination_state(&dest).expect("clear");
    assert_eq!(history.repository_id(&dest).expect("read"), None);
    assert_eq!(history.repository_id(&other).expect("read"), Some(id(ID_B)));
}

/// A password store whose writes of one kind fail; everything else works like the in-memory one.
struct FailingStore {
    inner: InMemorySecretStore,
    fail_kind: Mutex<Option<String>>,
}

impl FailingStore {
    fn new() -> Self {
        Self {
            inner: InMemorySecretStore::new(),
            fail_kind: Mutex::new(None),
        }
    }

    fn fail_on(&self, kind: &str) {
        *self.fail_kind.lock().expect("lock") = Some(kind.to_string());
    }

    fn value(&self, target: &str, kind: &str) -> Option<String> {
        self.inner
            .get_secret(target, kind)
            .expect("read")
            .map(|v| v.as_str().expect("utf-8").to_string())
    }
}

impl SecretStore for FailingStore {
    fn set_secret(&self, target: &str, kind: &str, value: &SecretValue) -> Result<(), SecretError> {
        if self.fail_kind.lock().expect("lock").as_deref() == Some(kind) {
            return Err(SecretError::Io("write refused".to_string()));
        }
        self.inner.set_secret(target, kind, value)
    }

    fn get_secret(&self, target: &str, kind: &str) -> Result<Option<SecretValue>, SecretError> {
        self.inner.get_secret(target, kind)
    }

    fn delete_secret(&self, target: &str, kind: &str) -> Result<(), SecretError> {
        self.inner.delete_secret(target, kind)
    }
}

fn s3_destination() -> Destination {
    Destination::new("Bucket", s3("https://s3.example.com", "one")).expect("destination")
}

fn keys(access: &str, secret: &str) -> BackendCredentials {
    let mut creds = BackendCredentials::none();
    creds.set(
        SECRET_KIND_S3_ACCESS_KEY,
        Some(SecretValue::from_str_secret(access)),
    );
    creds.set(
        SECRET_KIND_S3_SECRET_KEY,
        Some(SecretValue::from_str_secret(secret)),
    );
    creds
}

/// restic stand-in that prints a repository config for every call.
fn fake_restic(dir: &Path) -> ResolvedRestic {
    let script = dir.join("restic");
    write_executable(
        &script,
        &format!("#!/bin/sh\nprintf '{{\"version\":2,\"id\":\"{ID_A}\"}}\\n'\n"),
    );
    ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    }
}

/// When a password store write fails after others succeeded, the entries
/// written before get their previous values back instead of being deleted, in
/// `BackendCredentials::persist` and in `setup_destination`.
#[test]
fn m5_09_failed_secret_write_restores_previous_values() {
    let dest = s3_destination();
    let target = dest.id.as_str();

    // persist
    let store = FailingStore::new();
    keys("old-access", "old-secret")
        .persist(&dest, &store)
        .expect("first save");
    store.fail_on(SECRET_KIND_S3_SECRET_KEY);
    assert!(
        keys("new-access", "new-secret")
            .persist(&dest, &store)
            .is_err(),
        "the failed write is reported"
    );
    assert_eq!(
        store.value(target, SECRET_KIND_S3_ACCESS_KEY).as_deref(),
        Some("old-access"),
        "persist: the access key written before is restored"
    );
    assert_eq!(
        store.value(target, SECRET_KIND_S3_SECRET_KEY).as_deref(),
        Some("old-secret"),
        "persist: the key whose write failed keeps its value"
    );

    // setup_destination
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let helper = dir.path().join("kestic");
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let store = FailingStore::new();
    store
        .set_secret(
            target,
            SECRET_KIND_REPO,
            &SecretValue::from_str_secret("old-repo-password"),
        )
        .expect("old password");
    keys("old-access", "old-secret")
        .persist(&dest, &store)
        .expect("old keys");
    store.fail_on(SECRET_KIND_S3_SECRET_KEY);
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
        cache_dir: None,
    };
    assert!(
        setup_destination(
            &dest,
            DestinationSetupMode::ConnectExisting,
            &SecretValue::from_str_secret("new-repo-password"),
            &keys("new-access", "new-secret"),
            &runtime,
        )
        .is_err(),
        "the failed write is reported"
    );
    assert_eq!(
        store.value(target, SECRET_KIND_REPO).as_deref(),
        Some("old-repo-password"),
        "setup_destination: the repository password is restored"
    );
    assert_eq!(
        store.value(target, SECRET_KIND_S3_ACCESS_KEY).as_deref(),
        Some("old-access"),
        "setup_destination: the access key is restored"
    );
}

/// The generic form: (a) more reserved names, (b) inherited variables of the
/// backends removed, (c) a wider «Geheim» default, (d) no secrets inside an `rclone:` URL.
#[test]
fn m5_09_q6_reserved_names_cleared_prefixes_secret_default_and_rclone_url_secrets() {
    // (a)
    for name in [
        "DBUS_SESSION_BUS_ADDRESS",
        "XDG_RUNTIME_DIR",
        "XDG_CONFIG_HOME",
        "GCONV_PATH",
        "TMPDIR",
    ] {
        assert_eq!(
            kestic_core::generic::validate_variable_name(name),
            Err(GenericInputError::ReservedVariable(name.to_string())),
            "(a) {name}"
        );
    }
    // (b)
    let cleared = generic("b2:bucket:laptop").backend_env_cleared_prefixes();
    for prefix in ["B2_", "AZURE_", "OS_", "RCLONE_"] {
        assert!(cleared.contains(&prefix), "(b) {prefix} in {cleared:?}");
    }
    // (c)
    for name in [
        "RCLONE_CONFIG_PASS",
        "AZURE_STORAGE_SAS_TOKEN",
        "AZURE_SAS",
        "GOOGLE_APPLICATION_CREDENTIALS",
    ] {
        assert!(secret_by_default(name), "(c) {name}");
    }
    assert!(
        !secret_by_default("B2_ACCOUNT_ID"),
        "(c) B2_ACCOUNT_ID stays visible"
    );
    // (d)
    assert_eq!(
        validate_url("rclone::sftp,host=nas,pass=hunter2:backup"),
        Err(GenericInputError::SecretInUrl("pass".to_string())),
        "(d) pass="
    );
    assert_eq!(
        validate_url("rclone:remote,token=abc:backup"),
        Err(GenericInputError::SecretInUrl("token".to_string())),
        "(d) token="
    );
    assert_eq!(
        validate_url("rclone::sftp,host=nas,user=me:backup"),
        Ok(()),
        "(d) parameters that are no secret stay allowed"
    );
    assert_eq!(
        validate_url("rclone::sftp,host=nas,key_file=/home/me/.ssh/id:backup"),
        Ok(()),
        "(d) a file path is no secret"
    );
    assert_eq!(
        validate_url("rclone::sftp,host=nas,PASS=\"a,b\":backup"),
        Err(GenericInputError::SecretInUrl("PASS".to_string())),
        "(d) upper case and a quoted value"
    );
}
