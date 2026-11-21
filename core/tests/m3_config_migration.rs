// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Migration of M1 configurations (`config_version = 1`) to the current schema.

mod common;

use common::load_config;

use kestic_core::config::{CURRENT_CONFIG_VERSION, ConfigError, save_config};
use kestic_core::model::{KeepCount, ModelError, RetentionPolicy};
use std::fs;
use tempfile::TempDir;

/// Shape of a `config.toml` written by the M1 build: the backup has an empty
/// `[backups.retention]` table, because retention was not configurable yet.
fn m1_config(version: u32, retention_body: &str) -> String {
    format!(
        r#"config_version = {version}
expert_mode = true

[restic_provider]
kind = "system"

[[destinations]]
id = "dcde624u"
name = "Test repository"

[destinations.kind]
kind = "local"
path = "/srv/kestic-test/repo"

[destinations.secrets]
repo_password_ref = "kestic:dest:dcde624u:repo"

[destinations.maintenance]

[[backups]]
id = "nucwnmmd"
name = "Projects"
destination_id = "dcde624u"

[backups.source]
path = "/srv/kestic-test/projects"
kind = "folder"

[backups.excludes]
lines = []
exclude_caches = true

[backups.schedule]
kind = "manual_only"

[backups.run_conditions]
skip_on_battery = false
skip_on_metered_network = false

[backups.retention]
{retention_body}"#
    )
}

fn write_config(dir: &TempDir, raw: &str) -> std::path::PathBuf {
    let path = dir.path().join("kestic").join("config.toml");
    fs::create_dir_all(path.parent().expect("parent")).expect("config dir");
    fs::write(&path, raw).expect("write config");
    path
}

#[test]
fn m1_config_with_empty_retention_migrates_to_keep_every_snapshot() {
    let dir = TempDir::new().expect("tempdir");
    let raw = m1_config(1, "");
    let path = write_config(&dir, &raw);

    let loaded = load_config(&path).expect("M1 config must load after migration");
    let retention = &loaded.backups[0].retention;

    // (a) The empty retention becomes «keep the last snapshots: unlimited».
    assert_eq!(retention.keep_last, Some(KeepCount::Unlimited));
    // (b) No other rule is added, so `forget` deletes nothing.
    assert_eq!(
        retention.to_forget_args(),
        vec!["--keep-last".to_string(), "unlimited".to_string()]
    );
    // (c) The in-memory config carries the current schema version.
    assert_eq!(loaded.config_version, CURRENT_CONFIG_VERSION);
    // (d) Loading alone does not rewrite the file.
    assert_eq!(fs::read_to_string(&path).expect("reread"), raw);

    // (e) The next save writes the migrated version, which loads unchanged.
    save_config(&path, &loaded).expect("save migrated config");
    let saved = fs::read_to_string(&path).expect("read saved");
    assert!(
        saved.contains(&format!("config_version = {CURRENT_CONFIG_VERSION}")),
        "saved config must carry version {CURRENT_CONFIG_VERSION}:\n{saved}"
    );
    assert_eq!(load_config(&path).expect("reload"), loaded);
}

#[test]
fn m1_config_with_retention_rules_keeps_them_unchanged() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_config(&dir, &m1_config(1, "keep_daily = 7\nkeep_weekly = 4\n"));

    let loaded = load_config(&path).expect("load M1 config with rules");

    assert_eq!(
        loaded.backups[0].retention,
        RetentionPolicy {
            keep_daily: Some(KeepCount::Count(7)),
            keep_weekly: Some(KeepCount::Count(4)),
            ..RetentionPolicy::empty()
        }
    );
}

#[test]
fn current_config_with_empty_retention_is_still_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_config(&dir, &m1_config(CURRENT_CONFIG_VERSION, ""));

    let err = load_config(&path).expect_err("empty retention is invalid from version 2 on");

    assert!(
        matches!(
            err,
            ConfigError::Model(ModelError::InvalidBackup(ref msg))
                if msg.contains("at least one active keep rule")
        ),
        "unexpected error: {err}"
    );
}

/// Version 2 config: one backup on a local folder and one on a REST server, both with all
/// network run conditions set.
const V2_CONFIG: &str = r#"config_version = 2

[[destinations]]
id = "locabcde"
name = "USB"

[destinations.kind]
kind = "local"
path = "/srv/kestic-test/repo"

[destinations.secrets]
repo_password_ref = "kestic:dest:locabcde:repo"

[[destinations]]
id = "rstabcde"
name = "NAS"

[destinations.kind]
kind = "rest"
url = "https://nas.example.invalid/kestic"

[destinations.secrets]
repo_password_ref = "kestic:dest:rstabcde:repo"

[[backups]]
id = "bklocabc"
name = "Local"
destination_id = "locabcde"

[backups.source]
path = "/srv/kestic-test/a"
kind = "folder"

[backups.run_conditions]
require_ac_power = true
require_network = true
skip_on_metered_network = true
allowed_networks = ["Heim"]

[backups.retention]
keep_daily = 7

[[backups]]
id = "bkrstabc"
name = "Remote"
destination_id = "rstabcde"

[backups.source]
path = "/srv/kestic-test/b"
kind = "folder"

[backups.run_conditions]
require_ac_power = true
require_network = true
skip_on_metered_network = true
allowed_networks = ["Heim"]

[backups.retention]
keep_daily = 7
"#;

/// Network run conditions apply only to network destinations; a version 2 config is migrated
/// accordingly.
#[test]
fn v2_config_drops_network_conditions_of_local_destinations_only() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_config(&dir, V2_CONFIG);

    let loaded = load_config(&path).expect("load v2 config");
    let local = &loaded.backups[0].run_conditions;
    let remote = &loaded.backups[1].run_conditions;

    // (a) local destination: no network conditions left, power condition kept.
    assert!(local.require_ac_power);
    assert!(!local.require_network);
    assert!(!local.skip_on_metered_network);
    assert!(local.allowed_networks.is_empty());
    // (b) network destination: unchanged.
    assert!(remote.require_network);
    assert!(remote.skip_on_metered_network);
    assert_eq!(remote.allowed_networks, vec!["Heim".to_string()]);
    // (c) migrated to the current version.
    assert_eq!(loaded.config_version, CURRENT_CONFIG_VERSION);
}
