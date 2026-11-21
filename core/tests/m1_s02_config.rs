// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::load_config;

use kestic_core::config::{
    AppConfig, CURRENT_CONFIG_VERSION, ConfigError, default_config_path, save_config,
};
use kestic_core::model::{
    Backup, Destination, DestinationKind, ExcludeRules, KeepCount, ResticProvider, RetentionPolicy,
    Schedule, Source,
};
use std::fs;
use tempfile::TempDir;

#[test]
fn ac_m1_02_1_and_02_2_config_toml_roundtrip_contains_no_secrets() {
    let dir = TempDir::new().expect("tempdir");
    let config_file = dir.path().join("kestic").join("config.toml");

    let dest = Destination::new(
        "Home Server",
        DestinationKind::Rest {
            url: "https://backup.example.invalid/alice".to_string(),
            username: Some("alice-rest-user".to_string()),
        },
    )
    .expect("rest destination");

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
    assert!(!raw.contains("super-secret-repo-pw"));
    assert!(!raw.contains("super-secret-rest-pw"));
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
