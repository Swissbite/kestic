// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::load_config;

use kestic_core::config::{
    AppConfig, CURRENT_CONFIG_VERSION, ConfigError, default_config_path, save_config,
};
use kestic_core::destination::{DestinationSetupMode, setup_destination};
use kestic_core::model::{
    Backup, Destination, DestinationKind, ExcludeRules, KeepCount, ResticProvider, RetentionPolicy,
    Schedule, Source,
};
use kestic_core::secrets::{SecretStore, SecretValue};
use kestic_test_support::password::assert_no_secret;
use kestic_test_support::repo_template::copy_template_repo;
use std::fs;
use tempfile::TempDir;
use time::macros::datetime;

#[test]
fn ac_m1_02_1_and_02_2_config_toml_roundtrip_contains_no_secrets() {
    let dir = TempDir::new().expect("tempdir");
    let config_file = dir.path().join("kestic").join("config.toml");

    // The destination is set up through the production path with a real `rest-server`: both
    // generated secrets (repository password, REST password) pass through `setup_destination`
    // and land in the secret store, which is where they could leak into the saved config.
    let server =
        kestic_test_support::tools::RestServerGuard::start_with_auth(&dir.path().join("rest-data"));
    let (_label, restic_bin) = common::restic_test_binaries()
        .into_iter()
        .next()
        .expect("restic binary");
    let rig = common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-12-01 10:00:00 UTC));
    let repo = copy_template_repo(&rig.resolved.binary, &server.repo_dir());
    let (username, _) = server.auth().expect("rest auth");
    let dest = Destination::new(
        "Home Server",
        DestinationKind::Rest {
            url: server.url.clone(),
            username: Some(username.to_string()),
        },
    )
    .expect("rest destination");
    setup_destination(
        &dest,
        DestinationSetupMode::ConnectExisting,
        &SecretValue::from_str_secret(&repo.password),
        &kestic_core::backend_credentials::BackendCredentials::rest(Some(
            SecretValue::from_str_secret(server.password()).clone(),
        )),
        &rig.runtime(),
    )
    .expect("set up REST destination");
    assert_eq!(
        rig.store
            .get_secret(dest.id.as_str(), "repo")
            .expect("get repo secret"),
        Some(SecretValue::from_str_secret(&repo.password)),
        "precondition: the repository password went through the secret store"
    );
    assert_eq!(
        rig.store
            .get_secret(dest.id.as_str(), "rest")
            .expect("get rest secret"),
        Some(SecretValue::from_str_secret(server.password())),
        "precondition: the REST password went through the secret store"
    );

    let mut backup =
        Backup::new("Home", Source::folder("/home/alice"), dest.id.clone()).expect("backup");
    backup.excludes = ExcludeRules {
        lines: vec![".cache".to_string(), "!important.cache".to_string()],
        exclude_caches: true,
        ..ExcludeRules::default()
    };
    backup.schedule = Schedule::Daily {
        hour: 21,
        minute: 30,
    };
    backup.retention = RetentionPolicy {
        keep_daily: Some(KeepCount::Count(7)),
        keep_weekly: Some(KeepCount::Count(4)),
        ..RetentionPolicy::default()
    };

    let cfg = AppConfig {
        config_version: CURRENT_CONFIG_VERSION,
        restic_provider: ResticProvider::Custom {
            path: "/opt/bin/restic".into(),
        },
        expert_mode: true,
        destinations: vec![dest.clone()],
        backups: vec![backup],
        ..Default::default()
    };

    save_config(&config_file, &cfg).expect("save config");
    let loaded = load_config(&config_file).expect("load config");
    assert_eq!(loaded, cfg);

    // AC-M1-02-2: Verify TOML contains only secret reference keys, never plaintext secret values.
    let raw = fs::read_to_string(&config_file).expect("read raw config");
    assert!(raw.contains(&format!("config_version = {CURRENT_CONFIG_VERSION}")));
    assert!(raw.contains(&dest.secrets.repo_password_ref));
    assert!(
        raw.contains(
            dest.secrets
                .rest_password_ref
                .as_deref()
                .expect("rest ref present")
        )
    );
    assert_no_secret(&raw, &repo.password, "config.toml (repository password)");
    assert_no_secret(&raw, server.password(), "config.toml (REST password)");
}

#[test]
fn ac_m1_02_3_higher_config_version_is_rejected_without_overwriting() {
    let dir = TempDir::new().expect("tempdir");
    let config_file = dir.path().join("config.toml");
    let future_toml = format!(
        "config_version = {}\nfuture_new_field = 'must survive unchanged'\n",
        CURRENT_CONFIG_VERSION + 5
    );
    fs::write(&config_file, &future_toml).expect("write future config");

    let err = load_config(&config_file).expect_err("must reject newer config_version");
    match err {
        ConfigError::UnsupportedVersion {
            found,
            max_supported,
        } => {
            assert_eq!(found, CURRENT_CONFIG_VERSION + 5);
            assert_eq!(max_supported, CURRENT_CONFIG_VERSION);
        }
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }

    let after = fs::read_to_string(&config_file).expect("read config after error");
    assert_eq!(after, future_toml);
}

#[test]
fn config_validation_rejects_unknown_destination_reference() {
    let dir = TempDir::new().expect("tempdir");
    let config_file = dir.path().join("config.toml");

    let missing_dest = kestic_core::model::DestinationId::generate().expect("id");
    let backup =
        Backup::new("Orphan", Source::folder("/home/alice"), missing_dest).expect("backup");
    let cfg = AppConfig {
        backups: vec![backup],
        ..AppConfig::default()
    };

    let err = save_config(&config_file, &cfg).expect_err("unknown destination must fail");
    assert!(matches!(err, ConfigError::Validation(_)));
    assert!(!config_file.exists());
}

#[test]
fn default_config_path_uses_xdg_config_home() {
    // Only asserts path structure cleanly using current process env or fallback check.
    let path = default_config_path().expect("default config path");
    assert!(path.ends_with("kestic/config.toml"));
}
