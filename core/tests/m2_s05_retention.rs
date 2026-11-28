// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

#![forbid(unsafe_code)]

mod common;

use kestic_test_support::config::load_config;

use kestic_test_support::process::KillOnDrop;
use kestic_test_support::tools::{assert_has_flag_pair, find_lock_file};
use kestic_test_support::wait::wait_until_within;

use kestic_core::backup::{
    build_forget_ids_command_spec, build_forget_plan_command_spec, execute_forget_dry_run_with,
    execute_retention, list_snapshots_for_backup, parse_forget_json_output,
};
use kestic_core::config::{AppConfig, save_config};
use kestic_core::model::{
    Backup, Destination, DestinationKind, ForgetReason, KeepCount, ModelError, RetentionPeriod,
    RetentionPolicy, RetentionPreset, Source, validate_restic_duration,
};
use kestic_core::provider::ResolvedRestic;
use kestic_core::snapshot::ResticSnapshot;
use semver::Version;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::macros::{datetime, format_description};

const RESTIC_TIME_FMT: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

fn dummy_resolved() -> ResolvedRestic {
    ResolvedRestic {
        binary: PathBuf::from("/usr/bin/restic"),
        version: Version::new(0, 18, 0),
    }
}

fn dummy_dest_and_backup() -> (Destination, Backup) {
    let dest = Destination::new(
        "Unit Dest",
        DestinationKind::Local {
            path: PathBuf::from("/tmp/kestic-unit-repo"),
        },
    )
    .expect("dest");
    let backup = Backup::new(
        "Unit Backup",
        Source::folder("/tmp/kestic-unit-src"),
        dest.id.clone(),
    )
    .expect("backup");
    (dest, backup)
}

fn create_snapshot_at_time(
    restic_bin: &Path,
    repo_dir: &Path,
    password: &str,
    src_dir: &Path,
    tag: &str,
    time_str: &str,
    cache_dir: &Path,
) {
    let out = Command::new(restic_bin)
        .args([
            "-r",
            &repo_dir.to_string_lossy(),
            "backup",
            "--tag",
            tag,
            "--group-by",
            "tags",
            "--time",
            time_str,
            &src_dir.to_string_lossy(),
        ])
        .env("RESTIC_PASSWORD", password)
        .env("RESTIC_CACHE_DIR", cache_dir)
        .env("TZ", "UTC")
        .output()
        .expect("spawn restic backup --time");
    assert!(
        out.status.success(),
        "restic backup --time {time_str} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn list_snapshots_by_tag(
    restic_bin: &Path,
    repo_dir: &Path,
    password: &str,
    tag: &str,
    cache_dir: &Path,
) -> Vec<ResticSnapshot> {
    let out = Command::new(restic_bin)
        .args([
            "-r",
            &repo_dir.to_string_lossy(),
            "snapshots",
            "--no-lock",
            "--tag",
            tag,
            "--json",
        ])
        .env("RESTIC_PASSWORD", password)
        .env("RESTIC_CACHE_DIR", cache_dir)
        .output()
        .expect("run restic snapshots --tag");
    assert!(
        out.status.success(),
        "restic snapshots failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).expect("parse snapshots")
}

/// A restic process holding a lock; its stdin closes before the process is killed (fields drop
/// in declaration order).
struct LockHolderGuard {
    _stdin: ChildStdin,
    child: KillOnDrop,
}

#[test]
fn ac_m2_05_1_all_keep_count_flags_numeric_and_unlimited() {
    let (dest, mut backup) = dummy_dest_and_backup();
    let resolved = dummy_resolved();
    let kestic_bin = Path::new("/usr/bin/kestic");

    // All 6 count rules with numeric values.
    let numeric_policy = RetentionPolicy {
        keep_last: Some(KeepCount::Count(1)),
        keep_hourly: Some(KeepCount::Count(2)),
        keep_daily: Some(KeepCount::Count(7)),
        keep_weekly: Some(KeepCount::Count(4)),
        keep_monthly: Some(KeepCount::Count(12)),
        keep_yearly: Some(KeepCount::Count(3)),
        ..RetentionPolicy::empty()
    };
    numeric_policy.validate().expect("numeric policy valid");
    assert_eq!(
        numeric_policy.to_forget_args(),
        vec![
            "--keep-last",
            "1",
            "--keep-hourly",
            "2",
            "--keep-daily",
            "7",
            "--keep-weekly",
            "4",
            "--keep-monthly",
            "12",
            "--keep-yearly",
            "3",
        ]
    );

    backup.retention = numeric_policy;
    let spec_num = build_forget_plan_command_spec(&backup, &dest, &resolved, kestic_bin)
        .expect("build forget spec");
    for pair in [
        ("--keep-last", "1"),
        ("--keep-hourly", "2"),
        ("--keep-daily", "7"),
        ("--keep-weekly", "4"),
        ("--keep-monthly", "12"),
        ("--keep-yearly", "3"),
    ] {
        let pos = spec_num
            .subcommand_args
            .iter()
            .position(|a| a == pair.0)
            .unwrap_or_else(|| panic!("missing {}", pair.0));
        assert_eq!(spec_num.subcommand_args[pos + 1], pair.1);
    }

    // All 6 count rules with `unlimited`.
    let unlimited_policy = RetentionPolicy {
        keep_last: Some(KeepCount::Unlimited),
        keep_hourly: Some(KeepCount::Unlimited),
        keep_daily: Some(KeepCount::Unlimited),
        keep_weekly: Some(KeepCount::Unlimited),
        keep_monthly: Some(KeepCount::Unlimited),
        keep_yearly: Some(KeepCount::Unlimited),
        ..RetentionPolicy::empty()
    };
    unlimited_policy.validate().expect("unlimited policy valid");
    assert_eq!(
        unlimited_policy.to_forget_args(),
        vec![
            "--keep-last",
            "unlimited",
            "--keep-hourly",
            "unlimited",
            "--keep-daily",
            "unlimited",
            "--keep-weekly",
            "unlimited",
            "--keep-monthly",
            "unlimited",
            "--keep-yearly",
            "unlimited",
        ]
    );

    backup.retention = unlimited_policy;
    let spec_unlim = build_forget_plan_command_spec(&backup, &dest, &resolved, kestic_bin)
        .expect("build forget spec unlimited");
    for flag in [
        "--keep-last",
        "--keep-hourly",
        "--keep-daily",
        "--keep-weekly",
        "--keep-monthly",
        "--keep-yearly",
    ] {
        let pos = spec_unlim
            .subcommand_args
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("missing {flag}"));
        assert_eq!(spec_unlim.subcommand_args[pos + 1], "unlimited");
    }

    // Zero count is rejected.
    let zero_policy = RetentionPolicy {
        keep_daily: Some(KeepCount::Count(0)),
        ..RetentionPolicy::empty()
    };
    assert!(matches!(
        zero_policy.validate(),
        Err(ModelError::InvalidBackup(_))
    ));
}

#[test]
fn ac_m2_05_2_keep_within_durations_passed_verbatim_and_invalid_rejected() {
    let (dest, mut backup) = dummy_dest_and_backup();
    let resolved = dummy_resolved();
    let kestic_bin = Path::new("/usr/bin/kestic");

    for dur in ["30d", "6m", "2y"] {
        assert_eq!(validate_restic_duration(dur).expect("valid dur"), dur);

        let policy = RetentionPolicy {
            keep_within: Some(dur.to_string()),
            keep_within_hourly: Some(dur.to_string()),
            keep_within_daily: Some(dur.to_string()),
            keep_within_weekly: Some(dur.to_string()),
            keep_within_monthly: Some(dur.to_string()),
            keep_within_yearly: Some(dur.to_string()),
            ..RetentionPolicy::empty()
        };
        policy.validate().expect("duration policy valid");
        assert_eq!(
            policy.to_forget_args(),
            vec![
                "--keep-within",
                dur,
                "--keep-within-hourly",
                dur,
                "--keep-within-daily",
                dur,
                "--keep-within-weekly",
                dur,
                "--keep-within-monthly",
                dur,
                "--keep-within-yearly",
                dur,
            ]
        );

        backup.retention = policy;
        let spec = build_forget_plan_command_spec(&backup, &dest, &resolved, kestic_bin)
            .expect("build forget spec");
        for flag in [
            "--keep-within",
            "--keep-within-hourly",
            "--keep-within-daily",
            "--keep-within-weekly",
            "--keep-within-monthly",
            "--keep-within-yearly",
        ] {
            let pos = spec
                .subcommand_args
                .iter()
                .position(|a| a == flag)
                .unwrap_or_else(|| panic!("missing {flag}"));
            assert_eq!(spec.subcommand_args[pos + 1], dur);
        }
    }

    // Invalid duration `30x` rejected across parser and every duration field.
    assert!(matches!(
        validate_restic_duration("30x"),
        Err(ModelError::InvalidBackup(_))
    ));
    for invalid in ["30x", "0d", "d", "30", "-5d"] {
        assert!(
            validate_restic_duration(invalid).is_err(),
            "expected '{invalid}' to be rejected"
        );
    }

    let invalid_policies = [
        RetentionPolicy {
            keep_within: Some("30x".to_string()),
            ..RetentionPolicy::empty()
        },
        RetentionPolicy {
            keep_within_hourly: Some("30x".to_string()),
            ..RetentionPolicy::empty()
        },
        RetentionPolicy {
            keep_within_daily: Some("30x".to_string()),
            ..RetentionPolicy::empty()
        },
        RetentionPolicy {
            keep_within_weekly: Some("30x".to_string()),
            ..RetentionPolicy::empty()
        },
        RetentionPolicy {
            keep_within_monthly: Some("30x".to_string()),
            ..RetentionPolicy::empty()
        },
        RetentionPolicy {
            keep_within_yearly: Some("30x".to_string()),
            ..RetentionPolicy::empty()
        },
    ];
    for p in invalid_policies {
        assert!(
            matches!(p.validate(), Err(ModelError::InvalidBackup(_))),
            "30x must be rejected: {p:?}"
        );
    }
}

#[test]
fn ac_m2_05_3_standard_preset_flags() {
    let policy = RetentionPolicy::standard();
    assert_eq!(policy.preset(), RetentionPreset::Standard);
    assert_eq!(
        policy.to_forget_args(),
        vec![
            "--keep-daily",
            "7",
            "--keep-weekly",
            "4",
            "--keep-monthly",
            "12",
        ]
    );

    let (dest, mut backup) = dummy_dest_and_backup();
    backup.retention = policy;
    let resolved = dummy_resolved();
    let spec = build_forget_plan_command_spec(&backup, &dest, &resolved, Path::new("kestic"))
        .expect("build forget spec");
    assert_eq!(
        spec.subcommand_args,
        vec![
            "forget",
            "--tag",
            &backup.tag(),
            "--group-by",
            "tags",
            "--keep-daily",
            "7",
            "--keep-weekly",
            "4",
            "--keep-monthly",
            "12",
            "--dry-run",
            "--no-lock",
        ]
    );
}

#[test]
fn ac_m2_05_4_empty_retention_policy_rejected_and_config_untouched() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = dir.path().join("config.toml");

    let empty = RetentionPolicy::empty();
    assert!(matches!(
        empty.validate(),
        Err(ModelError::InvalidBackup(_))
    ));

    let (dest, backup) = dummy_dest_and_backup();
    let mut cfg = AppConfig {
        destinations: vec![dest],
        backups: vec![backup],
        ..AppConfig::default()
    };
    save_config(&config_path, &cfg).expect("save valid initial config");
    let before_raw = fs::read_to_string(&config_path).expect("read saved config");

    // Mutate in-memory copy to empty retention and attempt validation & save.
    cfg.backups[0].retention = RetentionPolicy::empty();
    assert!(cfg.backups[0].validate().is_err());
    assert!(cfg.validate().is_err());
    assert!(save_config(&config_path, &cfg).is_err());

    let after_raw = fs::read_to_string(&config_path).expect("read config after rejected save");
    assert_eq!(
        after_raw, before_raw,
        "rejected empty retention must leave config.toml untouched"
    );
    let reloaded = load_config(&config_path).expect("reload valid config");
    assert_eq!(reloaded.backups[0].retention, RetentionPolicy::standard());
}

#[test]
fn ac_m2_05_5_never_passes_unsafe_allow_remove_all() {
    let (dest, mut backup) = dummy_dest_and_backup();
    let resolved = dummy_resolved();
    let kestic_bin = Path::new("/usr/bin/kestic");

    let policies = [
        RetentionPolicy::standard(),
        RetentionPolicy::economical(),
        RetentionPolicy::long(),
        RetentionPolicy {
            keep_last: Some(KeepCount::Count(1)),
            ..RetentionPolicy::empty()
        },
        RetentionPolicy {
            keep_within: Some("30d".to_string()),
            keep_yearly: Some(KeepCount::Unlimited),
            ..RetentionPolicy::empty()
        },
        RetentionPolicy {
            keep_last: Some(KeepCount::Count(5)),
            keep_hourly: Some(KeepCount::Count(12)),
            keep_daily: Some(KeepCount::Count(7)),
            keep_weekly: Some(KeepCount::Count(4)),
            keep_monthly: Some(KeepCount::Count(12)),
            keep_yearly: Some(KeepCount::Unlimited),
            keep_within: Some("1y".to_string()),
            keep_within_hourly: Some("24h".to_string()),
            keep_within_daily: Some("7d".to_string()),
            keep_within_weekly: Some("1m".to_string()),
            keep_within_monthly: Some("6m".to_string()),
            keep_within_yearly: Some("2y".to_string()),
        },
    ];

    for policy in policies {
        policy.validate().expect("valid policy");
        let args = policy.to_forget_args();
        assert!(
            !args.iter().any(|a| a == "--unsafe-allow-remove-all"),
            "to_forget_args emitted forbidden flag: {args:?}"
        );
        backup.retention = policy;
        let specs = [
            build_forget_plan_command_spec(&backup, &dest, &resolved, kestic_bin)
                .expect("build plan spec"),
            build_forget_ids_command_spec(&dest, &["0".repeat(64)], &resolved, kestic_bin)
                .expect("build ids spec"),
        ];
        for spec in specs {
            let argv = spec.restic_argv();
            assert!(
                !argv.iter().any(|a| a == "--unsafe-allow-remove-all"),
                "restic_argv emitted forbidden flag: {argv:?}"
            );
            assert!(
                !spec
                    .to_transparent_shell_command()
                    .contains("--unsafe-allow-remove-all")
            );
        }
    }
}

#[test]
fn ac_m2_05_6_forget_scoped_by_tag_and_leaves_other_backup_and_meta_untouched() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let cache_str = cache_dir.to_string_lossy().into_owned();
        let repo_dir = dir.path().join("repo");
        let src_a = dir.path().join("src-a");
        let src_b = dir.path().join("src-b");
        let meta_src = dir.path().join("meta");
        fs::create_dir_all(&src_a).expect("mkdir src-a");
        fs::create_dir_all(&src_b).expect("mkdir src-b");
        fs::create_dir_all(&meta_src).expect("mkdir meta");

        fs::write(src_a.join("a.txt"), b"backup a data").expect("write a");
        fs::write(src_b.join("b.txt"), b"backup b data").expect("write b");
        fs::write(meta_src.join("meta.toml"), b"config_version = 1\n").expect("write meta");

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-11-17 09:00:00 UTC));
        let runtime = rig.runtime();

        let (dest, pw) = rig.local_repo("Local Scope", &repo_dir);

        let mut backup_a =
            Backup::new("Backup A", Source::folder(&src_a), dest.id.clone()).expect("backup a");
        backup_a.retention = RetentionPolicy {
            keep_last: Some(KeepCount::Count(1)),
            ..RetentionPolicy::empty()
        };
        let backup_b =
            Backup::new("Backup B", Source::folder(&src_b), dest.id.clone()).expect("backup b");

        // Create 3 snapshots for backup A, 3 for backup B, and 2 `kestic-meta:<id_a>` snapshots.
        for day in [10, 11, 12] {
            create_snapshot_at_time(
                &rig.resolved.binary,
                &repo_dir,
                &pw,
                &src_a,
                &backup_a.tag(),
                &format!("2026-11-{day:02} 12:00:00"),
                &cache_dir,
            );
            create_snapshot_at_time(
                &rig.resolved.binary,
                &repo_dir,
                &pw,
                &src_b,
                &backup_b.tag(),
                &format!("2026-11-{day:02} 12:00:00"),
                &cache_dir,
            );
        }
        for day in [10, 11] {
            create_snapshot_at_time(
                &rig.resolved.binary,
                &repo_dir,
                &pw,
                &meta_src,
                &backup_a.meta_tag(),
                &format!("2026-11-{day:02} 12:00:00"),
                &cache_dir,
            );
        }

        let spec = build_forget_plan_command_spec(&backup_a, &dest, &rig.resolved, &rig.helper)
            .expect("build forget spec");
        assert_has_flag_pair(&spec.subcommand_args, "--tag", backup_a.tag().as_str());
        assert_has_flag_pair(&spec.subcommand_args, "--group-by", "tags");

        let res = execute_retention(&backup_a, &dest, &runtime, None, &[])
            .unwrap_or_else(|e| panic!("{label}: retention failed: {e}"));
        assert!(res.succeeded(), "{label}: retention succeeded: {res:?}");

        let snaps_a = list_snapshots_for_backup(
            &backup_a.id,
            &dest,
            &rig.resolved,
            &rig.helper,
            &rig.store,
            &rig.clock,
            &[("RESTIC_CACHE_DIR", cache_str.as_str())],
        )
        .expect("list snaps a");
        let snaps_b = list_snapshots_for_backup(
            &backup_b.id,
            &dest,
            &rig.resolved,
            &rig.helper,
            &rig.store,
            &rig.clock,
            &[("RESTIC_CACHE_DIR", cache_str.as_str())],
        )
        .expect("list snaps b");
        let snaps_meta = list_snapshots_by_tag(
            &rig.resolved.binary,
            &repo_dir,
            &pw,
            &backup_a.meta_tag(),
            &cache_dir,
        );

        assert_eq!(snaps_a.len(), 1, "{label}: backup A should keep 1 snapshot");
        assert_eq!(
            snaps_b.len(),
            3,
            "{label}: backup B snapshots must remain untouched"
        );
        assert_eq!(
            snaps_meta.len(),
            2,
            "{label}: kestic-meta:<id> snapshots must remain untouched"
        );
    }
}

#[test]
fn ac_m2_05_7_keep_daily_vs_keep_within_daily_across_gap() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let repo_dir = dir.path().join("repo");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        fs::write(src_dir.join("data.txt"), b"daily gap test").expect("write data");

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-11-17 09:30:00 UTC));
        let runtime = rig.runtime();

        let (dest, pw) = rig.local_repo("Gap Test", &repo_dir);

        let mut backup =
            Backup::new("Gap Backup", Source::folder(&src_dir), dest.id.clone()).expect("backup");

        // 2026-11-01..04 and 2026-11-12..15 at 12:00 UTC (gap Nov 5–11).
        for day in [1u8, 2, 3, 4, 12, 13, 14, 15] {
            create_snapshot_at_time(
                &rig.resolved.binary,
                &repo_dir,
                &pw,
                &src_dir,
                &backup.tag(),
                &format!("2026-11-{day:02} 12:00:00"),
                &cache_dir,
            );
        }

        // Case A: `--keep-daily 7` keeps 15,14,13,12,4,3,2 (removes 1).
        backup.retention = RetentionPolicy {
            keep_daily: Some(KeepCount::Count(7)),
            ..RetentionPolicy::empty()
        };
        let preview_daily =
            execute_forget_dry_run_with(&backup, &dest, &runtime, None, &[("TZ", "UTC")])
                .unwrap_or_else(|e| panic!("{label}: dry-run keep-daily 7 failed: {e}"));

        assert_eq!(
            preview_daily.keep_count, 7,
            "{label}: keep-daily 7 keep_count"
        );
        assert_eq!(
            preview_daily.remove_count, 1,
            "{label}: keep-daily 7 remove_count"
        );

        let mut kept_dates_daily: Vec<String> = preview_daily
            .snapshots
            .iter()
            .filter(|s| s.action == "keep")
            .map(|s| s.time[..10].to_string())
            .collect();
        kept_dates_daily.sort();
        assert_eq!(
            kept_dates_daily,
            vec![
                "2026-11-02",
                "2026-11-03",
                "2026-11-04",
                "2026-11-12",
                "2026-11-13",
                "2026-11-14",
                "2026-11-15",
            ],
            "{label}: keep-daily 7 kept dates"
        );
        let removed_dates_daily: Vec<String> = preview_daily
            .snapshots
            .iter()
            .filter(|s| s.action == "remove")
            .map(|s| s.time[..10].to_string())
            .collect();
        assert_eq!(
            removed_dates_daily,
            vec!["2026-11-01"],
            "{label}: removed date"
        );

        // Case B: `--keep-within-daily 7d` keeps 15,14,13,12 (removes 1..4).
        backup.retention = RetentionPolicy {
            keep_within_daily: Some("7d".to_string()),
            ..RetentionPolicy::empty()
        };
        let preview_within =
            execute_forget_dry_run_with(&backup, &dest, &runtime, None, &[("TZ", "UTC")])
                .unwrap_or_else(|e| panic!("{label}: dry-run keep-within-daily 7d failed: {e}"));

        assert_eq!(
            preview_within.keep_count, 4,
            "{label}: keep-within-daily 7d keep_count"
        );
        assert_eq!(
            preview_within.remove_count, 4,
            "{label}: keep-within-daily 7d remove_count"
        );

        let mut kept_dates_within: Vec<String> = preview_within
            .snapshots
            .iter()
            .filter(|s| s.action == "keep")
            .map(|s| s.time[..10].to_string())
            .collect();
        kept_dates_within.sort();
        assert_eq!(
            kept_dates_within,
            vec!["2026-11-12", "2026-11-13", "2026-11-14", "2026-11-15"],
            "{label}: keep-within-daily 7d kept dates"
        );

        let mut removed_dates_within: Vec<String> = preview_within
            .snapshots
            .iter()
            .filter(|s| s.action == "remove")
            .map(|s| s.time[..10].to_string())
            .collect();
        removed_dates_within.sort();
        assert_eq!(
            removed_dates_within,
            vec!["2026-11-01", "2026-11-02", "2026-11-03", "2026-11-04"],
            "{label}: keep-within-daily 7d removed dates"
        );
    }
}

#[test]
fn ac_m2_05_8_keep_within_anchors_at_newest_snapshot_not_wall_clock_now() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let cache_str = cache_dir.to_string_lossy().into_owned();
        let repo_dir = dir.path().join("repo");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        fs::write(src_dir.join("data.txt"), b"old snapshot anchor").expect("write data");

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-11-17 10:00:00 UTC));
        let runtime = rig.runtime();

        let (dest, pw) = rig.local_repo("Anchor Test", &repo_dir);

        let mut backup = Backup::new("Anchor Backup", Source::folder(&src_dir), dest.id.clone())
            .expect("backup");
        backup.retention = RetentionPolicy {
            keep_within: Some("30d".to_string()),
            ..RetentionPolicy::empty()
        };

        // Create two snapshots relative to real wall-clock `now`:
        // 100 days old and 60 days old (so newest snapshot itself is 60 days old).
        let now_utc = OffsetDateTime::now_utc();
        let older_ts = (now_utc - time::Duration::days(100))
            .format(RESTIC_TIME_FMT)
            .expect("format older");
        let newest_ts = (now_utc - time::Duration::days(60))
            .format(RESTIC_TIME_FMT)
            .expect("format newest");

        create_snapshot_at_time(
            &rig.resolved.binary,
            &repo_dir,
            &pw,
            &src_dir,
            &backup.tag(),
            &older_ts,
            &cache_dir,
        );
        create_snapshot_at_time(
            &rig.resolved.binary,
            &repo_dir,
            &pw,
            &src_dir,
            &backup.tag(),
            &newest_ts,
            &cache_dir,
        );

        let preview = execute_forget_dry_run_with(&backup, &dest, &runtime, None, &[("TZ", "UTC")])
            .unwrap_or_else(|e| panic!("{label}: dry-run failed: {e}"));
        assert_eq!(
            preview.keep_count, 1,
            "{label}: 60-day-old newest snapshot must be kept by --keep-within 30d"
        );
        assert_eq!(
            preview.remove_count, 1,
            "{label}: 100-day-old snapshot (>30d older than newest) should be removed"
        );

        let kept_snap = preview
            .snapshots
            .iter()
            .find(|s| s.action == "keep")
            .expect("kept snapshot");
        let expected_newest_date = &newest_ts[..10];
        assert!(
            kept_snap.time.starts_with(expected_newest_date),
            "{label}: expected newest snapshot ({expected_newest_date}) kept, got {}",
            kept_snap.time
        );

        // Also execute real `forget` to verify the 60-day-old newest snapshot remains in repo.
        let res = execute_retention(&backup, &dest, &runtime, None, &[("TZ", "UTC")])
            .unwrap_or_else(|e| panic!("{label}: retention failed: {e}"));
        assert!(res.succeeded(), "{label}: {res:?}");
        let remaining = list_snapshots_for_backup(
            &backup.id,
            &dest,
            &rig.resolved,
            &rig.helper,
            &rig.store,
            &rig.clock,
            &[("RESTIC_CACHE_DIR", cache_str.as_str())],
        )
        .expect("list remaining");
        assert_eq!(
            remaining.len(),
            1,
            "{label}: newest snapshot must survive forget"
        );
        assert_eq!(remaining[0].id, kept_snap.snapshot_id);
    }
}

fn spawn_non_exclusive_lock_holder(
    label: &str,
    restic_bin: &Path,
    repo_dir: &Path,
    pw: &str,
    cache_dir: &Path,
) -> LockHolderGuard {
    let mut child = KillOnDrop::spawn(
        Command::new(restic_bin)
            .args([
                "-r",
                &repo_dir.to_string_lossy(),
                "backup",
                "--stdin",
                "--stdin-filename",
                "slow-input.txt",
            ])
            .env("RESTIC_PASSWORD", pw)
            .env("RESTIC_CACHE_DIR", cache_dir)
            .env("RESTIC_HOST", "other-active-backup")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
        "restic backup --stdin",
    );

    let mut stdin = child.stdin.take().expect("piped stdin");
    stdin
        .write_all(b"partial-chunk\n")
        .expect("write initial stdin byte");
    stdin.flush().expect("flush stdin");

    let mut guard = LockHolderGuard {
        _stdin: stdin,
        child,
    };

    let locks_dir = repo_dir.join("locks");
    wait_until_within(
        Duration::from_secs(10),
        &format!("{label}: non-exclusive lock in {}", locks_dir.display()),
        || {
            if let Some(status) = guard.child.try_wait().expect("try_wait lock holder") {
                panic!("{label}: restic backup --stdin exited early with {status}");
            }
            find_lock_file(&locks_dir).is_some()
        },
    );
    guard
}

#[test]
fn ac_m2_06_2_and_06_6_retention_preview_uses_no_lock_with_reasons_while_backup_holds_non_exclusive_lock()
 {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let cache_str = cache_dir.to_string_lossy().into_owned();
        let repo_dir = dir.path().join("repo");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        fs::write(src_dir.join("data.txt"), b"preview no-lock test").expect("write data");

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-11-17 10:30:00 UTC));
        let runtime = rig.runtime();

        let (dest, pw) = rig.local_repo("Preview Lock Test", &repo_dir);

        let mut backup = Backup::new("Preview Backup", Source::folder(&src_dir), dest.id.clone())
            .expect("backup");
        backup.retention = RetentionPolicy {
            keep_daily: Some(KeepCount::Count(1)),
            ..RetentionPolicy::empty()
        };

        for day in [10u8, 11, 12] {
            create_snapshot_at_time(
                &rig.resolved.binary,
                &repo_dir,
                &pw,
                &src_dir,
                &backup.tag(),
                &format!("2026-11-{day:02} 12:00:00"),
                &cache_dir,
            );
        }

        let before_ids: Vec<String> = list_snapshots_for_backup(
            &backup.id,
            &dest,
            &rig.resolved,
            &rig.helper,
            &rig.store,
            &rig.clock,
            &[("RESTIC_CACHE_DIR", cache_str.as_str())],
        )
        .expect("list before")
        .into_iter()
        .map(|s| s.id)
        .collect();
        assert_eq!(before_ids.len(), 3, "{label}");

        // Verify spec passes `--dry-run` and `--no-lock` (AC-M2-06-6).
        let preview_spec =
            build_forget_plan_command_spec(&backup, &dest, &rig.resolved, &rig.helper)
                .expect("build preview spec");
        assert!(
            preview_spec
                .subcommand_args
                .iter()
                .any(|a| a == "--dry-run")
        );
        assert!(
            preview_spec
                .subcommand_args
                .iter()
                .any(|a| a == "--no-lock")
        );

        let guard = spawn_non_exclusive_lock_holder(
            label,
            &rig.resolved.binary,
            &repo_dir,
            &pw,
            &cache_dir,
        );

        // Preview must succeed (`--no-lock`), never fail with Exit 11, and report Keep/Remove + Reasons.
        let preview = execute_forget_dry_run_with(&backup, &dest, &runtime, None, &[("TZ", "UTC")])
            .unwrap_or_else(|e| {
                panic!("{label}: preview failed under active non-exclusive lock: {e}")
            });

        assert_eq!(preview.keep_count, 1, "{label}: keep_count");
        assert_eq!(preview.remove_count, 2, "{label}: remove_count");

        let kept: Vec<_> = preview
            .snapshots
            .iter()
            .filter(|s| s.action == "keep")
            .collect();
        let removed: Vec<_> = preview
            .snapshots
            .iter()
            .filter(|s| s.action == "remove")
            .collect();
        assert_eq!(kept.len(), 1, "{label}");
        assert_eq!(removed.len(), 2, "{label}");
        assert!(
            !kept[0].reasons.is_empty(),
            "{label}: kept snapshot must include restic Reasons: {:?}",
            kept[0]
        );
        assert!(
            kept[0].time.starts_with("2026-11-12"),
            "{label}: newest daily snapshot should be kept, got {}",
            kept[0].time
        );

        drop(guard);

        // Zero snapshots deleted in repo.
        let after_ids: Vec<String> = list_snapshots_for_backup(
            &backup.id,
            &dest,
            &rig.resolved,
            &rig.helper,
            &rig.store,
            &rig.clock,
            &[("RESTIC_CACHE_DIR", cache_str.as_str())],
        )
        .expect("list after")
        .into_iter()
        .map(|s| s.id)
        .collect();
        assert_eq!(
            after_ids, before_ids,
            "{label}: forget --dry-run must delete zero snapshots"
        );
    }
}

/// AC-M2-06-2: every reason the supported restic versions give for a kept snapshot is one
/// Kestic reads (and so shows in the GUI language): `--keep-last`, `--keep-within`, every period
/// rule with and without a duration.
#[test]
fn ac_m2_06_2_every_reason_of_restic_is_read() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let repo_dir = dir.path().join("repo");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        fs::write(src_dir.join("data.txt"), b"reasons test").expect("write data");

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-11-23 10:00:00 UTC));
        let runtime = rig.runtime();
        let (dest, pw) = rig.local_repo("Reasons Test", &repo_dir);

        let mut backup = Backup::new("Reasons Backup", Source::folder(&src_dir), dest.id.clone())
            .expect("backup");
        let within = |d: &str| Some(d.to_string());
        backup.retention = RetentionPolicy {
            keep_last: Some(KeepCount::Count(1)),
            keep_hourly: Some(KeepCount::Count(2)),
            keep_daily: Some(KeepCount::Count(2)),
            keep_weekly: Some(KeepCount::Count(2)),
            keep_monthly: Some(KeepCount::Count(2)),
            keep_yearly: Some(KeepCount::Count(5)),
            keep_within: within("3d"),
            keep_within_hourly: within("48h"),
            keep_within_daily: within("30d"),
            keep_within_weekly: within("6m"),
            keep_within_monthly: within("1y"),
            keep_within_yearly: within("2y"),
        };

        for time in [
            "2026-01-01 10:00:00",
            "2026-02-03 10:00:00",
            "2026-06-03 10:00:00",
            "2026-11-20 10:00:00",
            "2026-11-22 09:00:00",
            "2026-11-22 10:00:00",
        ] {
            create_snapshot_at_time(
                &rig.resolved.binary,
                &repo_dir,
                &pw,
                &src_dir,
                &backup.tag(),
                time,
                &cache_dir,
            );
        }

        let preview = execute_forget_dry_run_with(&backup, &dest, &runtime, None, &[("TZ", "UTC")])
            .unwrap_or_else(|e| panic!("{label}: preview failed: {e}"));
        let reasons: Vec<&String> = preview.snapshots.iter().flat_map(|s| &s.reasons).collect();
        assert!(!reasons.is_empty(), "{label}: restic gave no reasons");
        let parsed: Vec<ForgetReason> = reasons
            .iter()
            .map(|r| {
                ForgetReason::parse(r)
                    .unwrap_or_else(|| panic!("{label}: unknown reason «{r}» from restic"))
            })
            .collect();

        assert!(parsed.contains(&ForgetReason::Last), "{label}: --keep-last");
        assert!(
            parsed.contains(&ForgetReason::Within("3d".to_string())),
            "{label}: --keep-within"
        );
        for (period, duration) in [
            (RetentionPeriod::Hourly, "48h"),
            (RetentionPeriod::Daily, "30d"),
            (RetentionPeriod::Weekly, "6m"),
            (RetentionPeriod::Monthly, "1y"),
            (RetentionPeriod::Yearly, "2y"),
        ] {
            let rule = |within: Option<&str>| {
                parsed.iter().any(|p| {
                    matches!(p, ForgetReason::Period { period: q, within: w, .. }
                        if *q == period && w.as_deref() == within)
                })
            };
            assert!(rule(None), "{label}: --keep-{period:?}");
            assert!(rule(Some(duration)), "{label}: --keep-within-{period:?}");
        }
    }
}

#[test]
fn parse_forget_json_output_sorts_mixed_offsets_and_fractional_seconds_newest_first() {
    let dest = Destination::new(
        "Unit",
        DestinationKind::Local {
            path: PathBuf::from("/tmp/unit-repo"),
        },
    )
    .expect("dest");
    let backup = Backup::new("Sort", Source::folder("/tmp/unit-src"), dest.id).expect("bk");

    // Chronological order (newest -> oldest):
    // 1) 2026-11-17T09:05:00-01:00 == 10:05:00Z
    // 2) 2026-11-17T12:00:00.25+02:00 == 10:00:00.25Z
    // 3) 2026-11-17T10:00:00.1Z == 10:00:00.10Z
    // Note: lexicographically "2026-11-17T12:..." > "2026-11-17T10:..." > "2026-11-17T09:...",
    // so only parsed OffsetDateTime sorting yields the true newest-first order.
    let raw_json = r#"[{"keep":[
        {"id":"snap_oldest_frac000000000000000000000000000000000000000000000000000","short_id":"oldest01","time":"2026-11-17T10:00:00.1Z","paths":["/tmp/unit-src"]},
        {"id":"snap_middle_plus020000000000000000000000000000000000000000000000000","short_id":"middle02","time":"2026-11-17T12:00:00.25+02:00","paths":["/tmp/unit-src"]}
    ],"remove":[
        {"id":"snap_newest_minus01000000000000000000000000000000000000000000000000","short_id":"newest03","time":"2026-11-17T09:05:00-01:00","paths":["/tmp/unit-src"]}
    ],"reasons":[]}]"#;

    let parsed = parse_forget_json_output(&backup.id, &[raw_json.to_string()]).expect("parse");
    let order: Vec<&str> = parsed
        .snapshots
        .iter()
        .map(|s| s.short_id.as_str())
        .collect();
    assert_eq!(order, vec!["newest03", "middle02", "oldest01"]);
}
