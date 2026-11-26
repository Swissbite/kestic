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
            "(a) message names {name:?}"
        );
    }

    // (c) A configuration read without the check (hand-edited file): one run per refused name
    // as a plain variable with a value of its own, and one with `RESTIC_PASSWORD_COMMAND` as a
    // secret variable with a stored value.
    for (i, name) in refused.iter().enumerate() {
        let value = format!("/tmp/refused-value-{i}");
        let env = run_hand_edited(vec![var(name, false, Some(&value))], None);
        assert!(!env.contains(&value), "(c) {name:?} passed: {env}");
    }
    let env = run_hand_edited(
        vec![var("RESTIC_PASSWORD_COMMAND", true, None)],
        Some(("RESTIC_PASSWORD_COMMAND", "echo stored-command")),
    );
    assert!(
        !env.contains("stored-command"),
        "(c) the secret RESTIC_PASSWORD_COMMAND passed: {env}"
    );
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

/// Runs restic for a generic destination whose variables are `vars` plus a valid
/// `B2_ACCOUNT_ID`, set without the check like a hand-edited configuration, with `secret` (name
/// and value of a secret variable) in the password store; returns the environment restic saw.
fn run_hand_edited(mut vars: Vec<EnvVar>, secret: Option<(&str, &str)>) -> String {
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let store = InMemorySecretStore::new();
    let mut dest = generic("b2:bucket:laptop", Vec::new()).expect("destination");
    vars.push(var("B2_ACCOUNT_ID", false, Some("0012ab")));
    dest.kind = DestinationKind::Generic {
        url: "b2:bucket:laptop".to_string(),
        env: vars,
    };
    store
        .set_secret(
            dest.id.as_str(),
            SECRET_KIND_REPO,
            &SecretValue::from_str_secret(strong_password()),
        )
        .expect("store repository password");
    if let Some((name, value)) = secret {
        store
            .set_secret(
                dest.id.as_str(),
                &secret_kind(name),
                &SecretValue::from_str_secret(value),
            )
            .expect("store the secret variable");
    }
    let spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        dir.path().join("kestic"),
        &dest,
        vec!["snapshots".to_string()],
    );
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    run_restic(&spec, &dest, Some(&store), &clock, None, &[], |_| {}).expect("run");
    read(dir.path().join("env.log"))
}

/// M5-S03, variables of the generic form: a variable named `RESTIC_CACHE_DIR` does not move
/// Kestic's own cache directory.
#[test]
fn m5_03_kestic_cache_dir_wins_over_a_variable() {
    let dir = TempDir::new().expect("tempdir");
    let resolved = fake_restic(dir.path());
    let store = InMemorySecretStore::new();
    let dest = generic(
        "b2:bucket:laptop",
        vec![var("RESTIC_CACHE_DIR", false, Some("/tmp/elsewhere"))],
    )
    .expect("destination");
    store
        .set_secret(
            dest.id.as_str(),
            SECRET_KIND_REPO,
            &SecretValue::from_str_secret(strong_password()),
        )
        .expect("store repository password");
    let cache = dir.path().join("cache");
    let spec = ResticCommandSpec::for_destination(
        &resolved.binary,
        dir.path().join("kestic"),
        &dest,
        vec!["snapshots".to_string()],
    )
    .with_cache_dir(Some(&cache));
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    run_restic(&spec, &dest, Some(&store), &clock, None, &[], |_| {}).expect("run");
    assert_eq!(
        env_value(&read(dir.path().join("env.log")), "RESTIC_CACHE_DIR").as_deref(),
        Some(cache.to_string_lossy().as_ref()),
        "Kestic's cache directory"
    );
}

/// M5-S03, pre-check (P32) of the other restic calls: the snapshot list and the prune preview of
/// a generic destination first run `restic cat config` and stop when it cannot reach the
/// backend, instead of starting the actual command, which restic would retry for minutes.
#[test]
fn m5_03_snapshot_list_and_prune_preview_check_a_generic_destination_first() {
    use kestic_core::backup::{BackupRunError, list_snapshots_for_backup};
    use kestic_core::destination::DestinationError;
    use kestic_core::maintenance::execute_prune_dry_run;
    use kestic_core::model::BackupId;

    let dir = TempDir::new().expect("tempdir");
    let script = dir.path().join("restic");
    let others = dir.path().join("other-commands.log");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\ncase \" $* \" in\n  *\" cat \"*) echo '{{\"message_type\":\"exit_error\",\"code\":1,\"message\":\"Fatal: unable to open repository at b2:bucket:laptop: dial tcp 192.0.2.1:443: connect: connection refused\"}}' >&2; exit 1;;\nesac\necho \"$*\" >> '{}'\nexit 1\n",
            others.display()
        ),
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let store = InMemorySecretStore::new();
    let dest = generic("b2:bucket:laptop", Vec::new()).expect("destination");
    store
        .set_secret(
            dest.id.as_str(),
            SECRET_KIND_REPO,
            &SecretValue::from_str_secret(strong_password()),
        )
        .expect("store repository password");
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let helper = dir.path().join("kestic");
    let unreachable = |res: &BackupRunError| {
        matches!(
            res,
            BackupRunError::Destination(DestinationError::Unreachable(_))
        )
    };

    let listed = list_snapshots_for_backup(
        &BackupId::generate().expect("backup id"),
        &dest,
        &resolved,
        &helper,
        &store,
        &clock,
        &[],
    );
    assert!(
        listed.as_ref().is_err_and(unreachable),
        "the snapshot list stops at the pre-check"
    );

    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
        cache_dir: None,
    };
    let preview = execute_prune_dry_run(&dest, &runtime, None, &[]);
    assert!(
        preview.as_ref().is_err_and(unreachable),
        "the prune preview stops at the pre-check"
    );
    assert!(
        !others.exists(),
        "restic ran a command after the failed pre-check"
    );
}

/// M5-S03, pre-check (P32) of an rclone destination whose remote cannot be reached: rclone fails
/// fast and restic only says that rclone ended, so the reason comes from rclone's own line. The
/// pre-check fails as unreachable, and the run is not started.
#[test]
fn m5_03_rclone_connection_error_fails_the_pre_check() {
    use kestic_core::destination::{DestinationError, preflight_check_with_restic};

    let dir = TempDir::new().expect("tempdir");
    let script = dir.path().join("restic");
    write_executable(
        &script,
        "#!/bin/sh\n\
         echo 'rclone: 2026/12/01 10:00:00 CRITICAL: Failed to create file system for \"X:path\": NewFs: couldn'\\''t connect SSH: dial tcp 127.0.0.1:1: connect: connection refused' >&2\n\
         echo '{\"message_type\":\"exit_error\",\"code\":1,\"message\":\"Fatal: unable to open repository at rclone:X:path: error talking HTTP to rclone: exit status 1\"}' >&2\n\
         exit 1\n",
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let store = InMemorySecretStore::new();
    let dest = generic("rclone:X:path", Vec::new()).expect("destination");
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let helper = dir.path().join("kestic");
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
        cache_dir: None,
    };

    assert!(
        matches!(
            preflight_check_with_restic(&dest, &runtime),
            Err(DestinationError::Unreachable(_))
        ),
        "the remote rclone cannot reach makes the destination unreachable"
    );
}

/// M5-S03: a repository URL with a trailing slash names the same location, so a second
/// destination on it counts as a duplicate.
#[test]
fn m5_03_trailing_slash_is_the_same_location() {
    let kind = |url: &str| DestinationKind::Generic {
        url: url.to_string(),
        env: Vec::new(),
    };
    assert!(kind("b2:bucket:laptop").same_location(&kind("b2:bucket:laptop/")));
    assert!(kind("rclone:nas:/srv").same_location(&kind("rclone:nas:/srv/")));
    assert!(kind("rclone:nas:/").same_location(&kind("rclone:nas://")));
    assert!(!kind("b2:bucket:laptop").same_location(&kind("b2:bucket:desktop")));
    assert!(
        !kind("rclone:nas:").same_location(&kind("rclone:nas:/")),
        "the remote's own folder is not its root"
    );
}
