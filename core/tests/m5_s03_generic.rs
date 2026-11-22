// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M5-S03, generic form (URL and environment variables) in `core`: schemes, variable names, the
//! «Geheim» default, secret values only in the password store and in restic's environment, and
//! the program a backend needs. A restic stand-in records its arguments and environment.

use kestic_core::backend_credentials::BackendCredentials;
use kestic_core::clock::FakeClock;
use kestic_core::config::{AppConfig, save_config};
use kestic_core::destination::{DestinationSetupMode, setup_destination};
use kestic_core::destination_types::RequiredProgram;
use kestic_core::generic::{
    EnvVar, GenericInputError, missing_program, secret_by_default, secret_kind,
    validate_variable_name,
};
use kestic_core::model::{Destination, DestinationKind, ModelError};
use kestic_core::provider::ResolvedRestic;
use kestic_core::restic::{ResticCommandSpec, ResticRuntime, SECRET_KIND_REPO, run_restic};
use kestic_core::secrets::{InMemorySecretStore, SecretStore, SecretValue};
use kestic_test_support::password::{assert_no_secret, strong_password};
use kestic_test_support::tools::write_executable;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use time::macros::datetime;

/// restic stand-in: writes its arguments (one per line) to `args.log`, its environment to
/// `env.log` and prints an empty repository config.
fn fake_restic(dir: &Path) -> ResolvedRestic {
    let script = dir.join("restic");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{args}'\n/usr/bin/env > '{env}'\necho '{{}}'\n",
            args = dir.join("args.log").display(),
            env = dir.join("env.log").display()
        ),
    );
    ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    }
}

fn read(path: PathBuf) -> String {
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn var(name: &str, secret: bool, value: Option<&str>) -> EnvVar {
    EnvVar {
        name: name.to_string(),
        secret,
        value: value.map(ToOwned::to_owned),
    }
}

fn generic(url: &str, env: Vec<EnvVar>) -> Result<Destination, ModelError> {
    Destination::new(
        "Cloud",
        DestinationKind::Generic {
            url: url.to_string(),
            env,
        },
    )
}

/// The value restic saw for `name`, if any.
fn env_value(env: &str, name: &str) -> Option<String> {
    env.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}=")).map(ToOwned::to_owned))
}

/// AC-M5-03-1 (FR-01): (a) repository URLs with `b2:`, `azure:`, `gs:`, `swift:` and `rclone:`
/// are accepted, (b) an unknown scheme is a validation error. (c) the GUI keeps «Anlegen» locked
/// on this error: `gui/src/destination_input.rs` and the GUI smoke `m5_03_gui_generic_form`.
#[test]
fn ac_m5_03_1_known_schemes_are_accepted_and_unknown_ones_refused() {
    // (a)
    for url in [
        "b2:bucket-name:laptop",
        "azure:container:/laptop",
        "gs:bucket:/laptop",
        "swift:container:/laptop",
        "rclone:remote:laptop",
    ] {
        assert!(generic(url, Vec::new()).is_ok(), "(a) {url} refused");
    }
    // (b)
    for url in [
        "s3:https://s3.example.com/bucket",
        "ftp:host:/x",
        "b2bucket",
        "/srv/repo",
    ] {
        assert!(
            matches!(
                generic(url, Vec::new()),
                Err(ModelError::Generic(GenericInputError::UnsupportedScheme(_)))
            ),
            "(b) {url} accepted"
        );
    }
}

/// AC-M5-03-2 (FR-01, FR-08), runner side: a variable marked secret (a) is saved only in the
/// password store, (b) is not in the config, and (c) reaches restic as an environment variable,
/// never as an argument. The variable that is not secret goes to restic from the config.
#[test]
fn ac_m5_03_2_secret_variable_only_in_store_and_restic_environment() {
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let store = InMemorySecretStore::new();
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let helper = dir.path().join("kestic");
    let secret = strong_password();
    let dest = generic(
        "b2:bucket-name:laptop",
        vec![
            var("B2_ACCOUNT_ID", false, Some("0012ab")),
            var("B2_ACCOUNT_KEY", true, None),
        ],
    )
    .expect("generic destination");
    let mut creds = BackendCredentials::none();
    creds.set(
        &secret_kind("B2_ACCOUNT_KEY"),
        Some(SecretValue::from_str_secret(&secret)),
    );
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
        cache_dir: None,
    };
    setup_destination(
        &dest,
        DestinationSetupMode::InitNew,
        &SecretValue::from_str_secret(strong_password()),
        &creds,
        &runtime,
    )
    .expect("setup with the stand-in");

    // (a)
    assert!(
        store
            .get_secret(dest.id.as_str(), &secret_kind("B2_ACCOUNT_KEY"))
            .expect("read store")
            .is_some_and(|v| v.as_str().expect("utf-8") == secret),
        "(a) secret value saved in the password store"
    );
    // (b)
    let config_path = dir.path().join("config.toml");
    let config = AppConfig {
        destinations: vec![dest.clone()],
        ..AppConfig::default()
    };
    save_config(&config_path, &config).expect("save config");
    let saved = read(config_path);
    assert_no_secret(&saved, &secret, "(b) config.toml");
    assert!(
        saved.contains("B2_ACCOUNT_KEY"),
        "(b) the config names the secret variable"
    );
    assert!(
        saved.contains("0012ab"),
        "(b) the config keeps the plain value"
    );
    // (c)
    let spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        &helper,
        &dest,
        vec!["snapshots".to_string()],
    );
    run_restic(&spec, &dest, Some(&store), &clock, None, &[], |_| {}).expect("run");
    let env = read(dir.path().join("env.log"));
    assert_eq!(
        env_value(&env, "B2_ACCOUNT_KEY").as_deref(),
        Some(secret.as_str()),
        "(c) restic gets the secret value as environment variable"
    );
    assert_eq!(
        env_value(&env, "B2_ACCOUNT_ID").as_deref(),
        Some("0012ab"),
        "(c) and the plain variable"
    );
    assert_no_secret(
        &read(dir.path().join("args.log")),
        &secret,
        "(c) restic arguments",
    );
    assert_no_secret(
        &spec.to_transparent_shell_command(),
        &secret,
        "(c) shown command",
    );
}

/// A `PATH` with the directory `dir`, an empty entry and a missing directory.
fn fake_path(dir: &Path) -> OsString {
    std::env::join_paths([dir.to_path_buf(), "".into(), dir.join("missing")]).expect("join PATH")
}

/// AC-M5-03-3 (FR-01), `core` part: without `rclone` in the `PATH` the daemon starts restic with,
/// an rclone destination (a) names the missing program, the same `RequiredProgram` as the
/// destination types of M5-S04; with `rclone` there nothing is missing, and other schemes need no
/// program. (b) the daemon then does not start restic: `daemon/tests/m5_s03_generic.rs`.
#[test]
fn ac_m5_03_3_rclone_destination_names_the_missing_program() {
    let dir = TempDir::new().expect("tempdir");
    let rclone = generic("rclone:remote:laptop", Vec::new()).expect("rclone destination");
    let b2 = generic("b2:bucket:laptop", Vec::new()).expect("b2 destination");
    let path = fake_path(dir.path());

    // (a)
    assert_eq!(
        missing_program(&rclone.kind, Some(&path)),
        Some(RequiredProgram::Rclone),
        "(a) rclone missing"
    );
    assert_eq!(
        missing_program(&b2.kind, Some(&path)),
        None,
        "(a) b2 needs no program"
    );
    write_executable(&dir.path().join("rclone"), "#!/bin/sh\nexit 0\n");
    assert_eq!(
        missing_program(&rclone.kind, Some(&path)),
        None,
        "(a) rclone installed"
    );
}

/// AC-M5-03-4 (FR-01, P22): for `B2_ACCOUNT_KEY`, `AWS_SECRET_ACCESS_KEY`, `MY_TOKEN`,
/// `RCLONE_PASSWORD` and `RESTIC_HOST`, (a) «Geheim» starts on for the first four, (b) not for
/// `RESTIC_HOST`, and (c) it can be switched per variable: the choice, not the name, decides
/// whether the value goes to the password store.
#[test]
fn ac_m5_03_4_secret_is_preselected_by_name_and_switchable() {
    // (a)
    for name in [
        "B2_ACCOUNT_KEY",
        "AWS_SECRET_ACCESS_KEY",
        "MY_TOKEN",
        "RCLONE_PASSWORD",
    ] {
        assert!(secret_by_default(name), "(a) {name} not preselected");
    }
    // (b)
    assert!(
        !secret_by_default("RESTIC_HOST"),
        "(b) RESTIC_HOST preselected"
    );
    // (c)
    let dest = generic(
        "b2:bucket:laptop",
        vec![
            var("B2_ACCOUNT_KEY", false, Some("visible")),
            var("RESTIC_HOST", true, None),
        ],
    )
    .expect("switched variables are valid");
    let secret_names: Vec<String> = dest
        .kind
        .backend_secret_slots()
        .into_iter()
        .map(|s| s.env_name)
        .collect();
    assert_eq!(
        secret_names,
        vec!["RESTIC_HOST".to_string()],
        "(c) secret slots"
    );
    assert_eq!(
        dest.kind.backend_plain_env(),
        vec![("B2_ACCOUNT_KEY".to_string(), "visible".to_string())],
        "(c) plain variables"
    );
}

/// AC-M5-03-7 (FR-01, NFR-02), `core` part: `RESTIC_PASSWORD_COMMAND`, `RESTIC_REPOSITORY_FILE`,
/// `PATH`, `HOME`, `LD_PRELOAD` and names outside `[A-Z_][A-Z0-9_]*` are (a) refused by `core`
/// with a message naming the variable, and (c) never handed to restic, even from a configuration
/// that skipped the check. (a) over D-Bus: `daemon/tests/m5_s03_generic.rs`; (b) «Anlegen»
/// locked: `gui/src/destination_input.rs` and the GUI smoke `m5_03_gui_generic_form`.
#[test]
fn ac_m5_03_7_reserved_and_invalid_variable_names_are_refused() {
    let refused = [
        "RESTIC_PASSWORD_COMMAND",
        "RESTIC_REPOSITORY_FILE",
        "PATH",
        "HOME",
        "LD_PRELOAD",
        "my_token",
        "1TOKEN",
        "B2-KEY",
        "",
    ];
    for name in refused {
        // (a)
        assert_eq!(
            validate_variable_name(name),
            Err(GenericInputError::ReservedVariable(name.to_string())),
            "(a) {name:?} accepted"
        );
        let err = generic("b2:bucket:laptop", vec![var(name, false, Some("x"))])
            .expect_err("destination with the variable");
        assert!(
            err.to_string().contains(&format!("'{name}'")),
            "(a) message names {name:?}: {err}"
        );
    }

    // (c) A configuration read without the check (hand-edited file).
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let store = InMemorySecretStore::new();
    let mut dest = generic("b2:bucket:laptop", Vec::new()).expect("destination");
    dest.kind = DestinationKind::Generic {
        url: "b2:bucket:laptop".to_string(),
        env: vec![
            var("PATH", false, Some("/tmp/evil")),
            var("LD_PRELOAD", false, Some("/tmp/evil.so")),
            var("RESTIC_PASSWORD_COMMAND", true, None),
            var("B2_ACCOUNT_ID", false, Some("0012ab")),
        ],
    };
    store
        .set_secret(
            dest.id.as_str(),
            SECRET_KIND_REPO,
            &SecretValue::from_str_secret(strong_password()),
        )
        .expect("store repository password");
    let spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        dir.path().join("kestic"),
        &dest,
        vec!["snapshots".to_string()],
    );
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    run_restic(&spec, &dest, Some(&store), &clock, None, &[], |_| {}).expect("run");
    let env = read(dir.path().join("env.log"));
    assert_ne!(
        env_value(&env, "PATH").as_deref(),
        Some("/tmp/evil"),
        "(c) PATH passed"
    );
    assert_eq!(env_value(&env, "LD_PRELOAD"), None, "(c) LD_PRELOAD passed");
    assert!(
        env_value(&env, "RESTIC_PASSWORD_COMMAND").is_some_and(|v| v.contains("secret get")),
        "(c) Kestic's own password command stays"
    );
    assert_eq!(
        env_value(&env, "B2_ACCOUNT_ID").as_deref(),
        Some("0012ab"),
        "(c) valid variables still reach restic"
    );
}
