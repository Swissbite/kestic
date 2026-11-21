// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

mod common;

use common::{execute_backup, load_config};

use kestic_core::config::{AppConfig, save_config};
use kestic_core::history::RunHistory;
use kestic_core::model::{Backup, Run, RunStatus, Source};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tempfile::TempDir;
use time::macros::datetime;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSnapshot {
    Missing,
    File {
        len: u64,
        modified: Option<SystemTime>,
    },
    Dir {
        modified: Option<SystemTime>,
        children: Vec<(String, PathSnapshot)>,
    },
    Other,
}

fn snapshot_path(path: &Path) -> PathSnapshot {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return PathSnapshot::Missing;
    };
    let modified = meta.modified().ok();
    if meta.is_file() {
        PathSnapshot::File {
            len: meta.len(),
            modified,
        }
    } else if meta.is_dir() {
        let mut children = Vec::new();
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                children.push((name, snapshot_path(&entry.path())));
            }
        }
        children.sort_by(|a, b| a.0.cmp(&b.0));
        PathSnapshot::Dir { modified, children }
    } else {
        PathSnapshot::Other
    }
}

fn current_real_uid() -> u32 {
    let status = fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    let uid_line = status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .expect("Uid line in /proc/self/status");
    let uids: Vec<u32> = uid_line
        .strip_prefix("Uid:")
        .expect("strip Uid:")
        .split_whitespace()
        .map(|s| s.parse::<u32>().expect("parse uid"))
        .collect();
    assert!(!uids.is_empty(), "expected uid values in /proc/self/status");
    for uid in &uids {
        assert_ne!(*uid, 0, "tests must never run as root (`AC-M1-Q-1`)");
    }
    uids[0]
}

fn passwd_home_for_uid(uid: u32) -> PathBuf {
    let passwd = fs::read_to_string("/etc/passwd").expect("read /etc/passwd");
    for line in passwd.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = trimmed.split(':').collect();
        if fields.len() >= 6 && fields[2].parse::<u32>().ok() == Some(uid) {
            return PathBuf::from(fields[5]);
        }
    }
    panic!("could not locate /etc/passwd home directory for uid {uid}");
}

#[test]
fn ac_m1_q1_non_root_and_real_home_directories_stay_untouched() {
    let uid = current_real_uid();
    let real_home = passwd_home_for_uid(uid);

    let guarded_paths = [
        real_home.join(".config/kestic"),
        real_home.join(".local/state/kestic"),
        real_home.join(".local/share/kestic"),
        real_home.join(".config/autostart"),
        real_home.join(".ssh/known_hosts"),
    ];
    let before: Vec<PathSnapshot> = guarded_paths.iter().map(|p| snapshot_path(p)).collect();

    // Perform isolated config, secret store, repo init, backup, and SQLite history operations in TempDir.
    let dir = TempDir::new().expect("tempdir");
    let _cache_dir = common::restic_cache_dir(dir.path());
    let cfg_path = dir.path().join("config.toml");
    let repo_dir = dir.path().join("repo");
    let state_dir = dir.path().join("state");
    let src_dir = dir.path().join("src");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("hello.txt"), b"isolated test").expect("write src");

    let (_label, restic_bin) = common::restic_test_binaries()
        .into_iter()
        .next()
        .expect("restic binary");
    let rig = common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-12-01 16:00:00 UTC));
    let runtime = rig.runtime();
    let clock = &rig.clock;

    let dest = rig.init_local("Isolated", &repo_dir, "isolated-pw");

    let backup = Backup::new("Isolated", Source::folder(&src_dir), dest.id.clone()).expect("bk");
    let cfg = AppConfig {
        destinations: vec![dest.clone()],
        backups: vec![backup.clone()],
        ..AppConfig::default()
    };
    save_config(&cfg_path, &cfg).expect("save config");
    assert_eq!(load_config(&cfg_path).expect("load config"), cfg);

    let res = execute_backup(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
        .expect("execute isolated backup");
    assert_eq!(res.exit_code, 0);

    let mut history = RunHistory::open(&state_dir).expect("open isolated history");
    let mut run = Run::new_backup(clock, &backup).expect("run");
    run.finish(clock, RunStatus::Ok, Some(0));
    history.upsert_run(&run, &[]).expect("upsert run");

    let after: Vec<PathSnapshot> = guarded_paths.iter().map(|p| snapshot_path(p)).collect();
    for ((path, prev), curr) in guarded_paths.iter().zip(before.iter()).zip(after.iter()) {
        assert_eq!(
            prev,
            curr,
            "real home path `{}` was modified during isolated test!",
            path.display()
        );
    }
}

/// Builds an SSH command string for restic `sftp.command` / `sftp.args` that always enforces
/// `-o UserKnownHostsFile=<path>` and optionally adds
/// `-F <ssh_config>` and `-o IdentitiesOnly=yes` for isolated test environments.
fn build_isolated_ssh_sftp_command(
    known_hosts_path: &Path,
    ssh_config_path: Option<&Path>,
    strict_identities: bool,
) -> String {
    let mut parts = vec![
        "ssh".to_string(),
        "-o".to_string(),
        format!(
            "UserKnownHostsFile={}",
            quote_arg(&known_hosts_path.to_string_lossy())
        ),
    ];
    if let Some(cfg) = ssh_config_path {
        parts.push("-F".to_string());
        parts.push(quote_arg(&cfg.to_string_lossy()));
    }
    if strict_identities {
        parts.push("-o".to_string());
        parts.push("IdentitiesOnly=yes".to_string());
    }
    parts.join(" ")
}

fn quote_arg(raw: &str) -> String {
    shlex::try_quote(raw)
        .map(|cow| cow.into_owned())
        .unwrap_or_else(|_| format!("'{raw}'"))
}

#[test]
fn ac_m1_q1_build_isolated_ssh_sftp_command_enforces_known_hosts_and_test_flags() {
    let dir = TempDir::new().expect("tempdir");
    let known_hosts = dir.path().join("known_hosts");
    let ssh_config = dir.path().join("ssh_config");

    let prod_cmd = build_isolated_ssh_sftp_command(&known_hosts, None, false);
    assert!(
        prod_cmd.contains(&format!("-o UserKnownHostsFile={}", known_hosts.display())),
        "missing UserKnownHostsFile in `{prod_cmd}`"
    );
    assert!(
        !prod_cmd.contains("-F "),
        "production command should not include test -F: `{prod_cmd}`"
    );
    assert!(
        !prod_cmd.contains("IdentitiesOnly=yes"),
        "production command should not force IdentitiesOnly=yes: `{prod_cmd}`"
    );

    let test_cmd = build_isolated_ssh_sftp_command(&known_hosts, Some(&ssh_config), true);
    assert!(
        test_cmd.contains(&format!("-o UserKnownHostsFile={}", known_hosts.display())),
        "missing UserKnownHostsFile in `{test_cmd}`"
    );
    assert!(
        test_cmd.contains(&format!("-F {}", ssh_config.display())),
        "missing -F in `{test_cmd}`"
    );
    assert!(
        test_cmd.contains("-o IdentitiesOnly=yes"),
        "missing -o IdentitiesOnly=yes in `{test_cmd}`"
    );

    // Paths containing spaces must remain properly shell-quoted.
    let spaced_kh = dir.path().join("my known hosts");
    let spaced_cfg = dir.path().join("my ssh config");
    let spaced_cmd = build_isolated_ssh_sftp_command(&spaced_kh, Some(&spaced_cfg), true);
    let parsed = shlex::split(&spaced_cmd).expect("valid shell quoting");
    assert!(parsed.contains(&format!("UserKnownHostsFile={}", spaced_kh.display())));
    assert!(parsed.contains(&spaced_cfg.to_string_lossy().into_owned()));
    assert!(parsed.contains(&"IdentitiesOnly=yes".to_string()));
}
