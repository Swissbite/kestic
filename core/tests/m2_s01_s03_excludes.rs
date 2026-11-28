// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

mod common;

use common::execute_backup;
use kestic_test_support::config::load_config;

use kestic_test_support::tools::assert_has_flag_pair;

use kestic_core::backup::{
    BackupExecutionContext, build_backup_command_spec_with_home, exclude_file_path,
    execute_backup_dry_run_with, execute_backup_with_inspector, list_snapshots_for_backup,
    write_exclude_file_with_home,
};
use kestic_core::config::{AppConfig, CURRENT_CONFIG_VERSION, save_config};
use kestic_core::model::{
    Backup, Destination, DestinationKind, ExcludeLintWarning, ExcludeRules, RunStatus, Source,
    TRASH_EXCLUDE_PATTERNS, evaluate_exclude_path, lint_exclude_rules, normalize_folder_rule,
    validate_exclude_larger_than,
};
use kestic_core::mount::SystemMountInspector;
use kestic_core::provider::ResolvedRestic;
use kestic_core::restic::ResticRuntime;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use time::macros::datetime;

struct ResticTestEnv {
    _dir: TempDir,
    rig: common::ResticRig,
    repo_dir: PathBuf,
    state_dir: PathBuf,
    dest: Destination,
    repo_pw: String,
}

impl ResticTestEnv {
    fn new(restic_bin: &Path) -> Self {
        let dir = TempDir::new().expect("tempdir");
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        fs::create_dir_all(&state_dir).expect("mkdir state");

        let rig =
            common::ResticRig::new(dir.path(), restic_bin, datetime!(2026-12-01 12:00:00 UTC));
        let (dest, repo_pw) = rig.local_repo("Local", &repo_dir);
        Self {
            _dir: dir,
            rig,
            repo_dir,
            state_dir,
            dest,
            repo_pw,
        }
    }

    fn root(&self) -> &Path {
        self._dir.path()
    }

    fn runtime(&self) -> ResticRuntime<'_> {
        self.rig.runtime()
    }

    fn snapshot_relative_paths(
        &self,
        backup: &Backup,
        src_root: &Path,
        label: &str,
    ) -> Vec<String> {
        let cache_str = self.rig.cache_dir.to_string_lossy();
        let snaps = list_snapshots_for_backup(
            &backup.id,
            &self.dest,
            &self.rig.resolved,
            &self.rig.helper,
            &self.rig.store,
            &self.rig.clock,
            &[("RESTIC_CACHE_DIR", cache_str.as_ref())],
        )
        .expect("list snapshots");
        assert_eq!(snaps.len(), 1, "{label}: expected exactly 1 snapshot");
        let snap_id = snaps[0].short_id.as_deref().unwrap_or(&snaps[0].id);

        let ls_out = Command::new(&self.rig.resolved.binary)
            .args([
                "-r",
                &self.repo_dir.to_string_lossy(),
                "ls",
                "--no-lock",
                snap_id,
            ])
            .env("RESTIC_PASSWORD", &self.repo_pw)
            .env("RESTIC_CACHE_DIR", &self.rig.cache_dir)
            .output()
            .expect("restic ls");
        assert!(
            ls_out.status.success(),
            "{label}: restic ls failed: {}",
            String::from_utf8_lossy(&ls_out.stderr)
        );

        let prefix = src_root.to_string_lossy();
        let prefix = prefix.trim_end_matches('/');
        String::from_utf8_lossy(&ls_out.stdout)
            .lines()
            .filter_map(|line| {
                let rel = line.strip_prefix(prefix)?;
                let rel = rel.strip_prefix('/').unwrap_or(rel);
                if rel.is_empty() {
                    None
                } else {
                    Some(rel.to_string())
                }
            })
            .collect()
    }
}

fn write_file(base: &Path, rel: &str, bytes: &[u8]) {
    let target = base.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).expect("mkdir parent");
    }
    fs::write(target, bytes).expect("write file");
}

#[test]
fn ac_m2_01_1_exclude_file_path_and_cli_flag() {
    let (_label, restic_bin) = common::restic_test_binaries()
        .into_iter()
        .next()
        .expect("restic binary");
    let env = ResticTestEnv::new(&restic_bin);
    let src = env.root().join("src");
    write_file(&src, "keep.txt", b"keep");
    write_file(&src, "skip.tmp", b"skip");

    let mut backup =
        Backup::new("Excl Path", Source::folder(&src), env.dest.id.clone()).expect("create backup");
    backup.excludes = ExcludeRules::from_text("*.tmp");

    let expected_path = exclude_file_path(&env.state_dir, &backup.id);
    assert_eq!(
        expected_path,
        env.state_dir
            .join("excludes")
            .join(format!("{}.txt", backup.id))
    );

    let spec = build_backup_command_spec_with_home(
        &backup,
        &env.dest,
        &env.state_dir,
        &env.rig.resolved,
        &env.rig.helper,
        Some(Path::new("/home/tester")),
    )
    .expect("build spec");

    assert!(
        expected_path.is_file(),
        "exclude file must exist at {}",
        expected_path.display()
    );
    let expected_str = expected_path.to_string_lossy().into_owned();
    assert_has_flag_pair(&spec.subcommand_args, "--exclude-file", &expected_str);
    assert_has_flag_pair(&spec.restic_argv(), "--exclude-file", &expected_str);

    let res = execute_backup(
        &backup,
        &env.dest,
        &env.state_dir,
        &env.runtime(),
        None,
        &[],
        |_| {},
    )
    .expect("execute backup");
    assert_eq!(res.exit_code, 0);
    assert_eq!(res.presentation.status, RunStatus::Ok);
    assert!(expected_path.is_file());
    assert_has_flag_pair(&res.command_record.args, "--exclude-file", &expected_str);
}

#[test]
fn ac_m2_01_2_unanchored_git_pattern_excludes_nested_git_dirs() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let env = ResticTestEnv::new(&restic_bin);
        let src = env.root().join("src");
        write_file(&src, "a/.git/x", b"git-obj-a");
        write_file(&src, "proj/b/.git/y", b"git-obj-b");
        write_file(&src, "a/f.txt", b"tracked");

        let mut backup =
            Backup::new("Git Excl", Source::folder(&src), env.dest.id.clone()).expect("backup");
        backup.excludes = ExcludeRules::from_text(".git");

        let res = execute_backup(
            &backup,
            &env.dest,
            &env.state_dir,
            &env.runtime(),
            None,
            &[],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: execute_backup failed: {e}"));
        assert_eq!(res.exit_code, 0, "{label}");

        let paths = env.snapshot_relative_paths(&backup, &src, label);
        assert!(
            paths.iter().any(|p| p == "a/f.txt"),
            "{label}: a/f.txt missing from snapshot: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains(".git")),
            "{label}: .git path leaked into snapshot: {paths:?}"
        );
    }
}

#[test]
fn ac_m2_01_3_double_star_build_excludes_nested_build_dirs_keeps_build_txt() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let env = ResticTestEnv::new(&restic_bin);
        let src = env.root().join("src");
        write_file(&src, "build/out.o", b"obj1");
        write_file(&src, "sub/build/deep/artifact.bin", b"obj2");
        write_file(&src, "build.txt", b"keep-root");
        write_file(&src, "sub/build.txt", b"keep-sub");

        let mut backup =
            Backup::new("Build Excl", Source::folder(&src), env.dest.id.clone()).expect("backup");
        backup.excludes = ExcludeRules::from_text("**/build/**");

        let res = execute_backup(
            &backup,
            &env.dest,
            &env.state_dir,
            &env.runtime(),
            None,
            &[],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: execute_backup failed: {e}"));
        assert_eq!(res.exit_code, 0, "{label}");

        let paths = env.snapshot_relative_paths(&backup, &src, label);
        assert!(
            paths.iter().any(|p| p == "build.txt"),
            "{label}: build.txt missing: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p == "sub/build.txt"),
            "{label}: sub/build.txt missing: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p == "build/out.o"),
            "{label}: build/out.o should be excluded: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p == "sub/build/deep/artifact.bin"),
            "{label}: sub/build/deep/artifact.bin should be excluded: {paths:?}"
        );
    }
}

#[test]
fn ac_m2_01_4_env_var_expansion_in_exclude_file() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let env = ResticTestEnv::new(&restic_bin);
        let src = env.root().join("workspace");
        write_file(&src, "src/excl/secret.dat", b"excluded");
        write_file(&src, "src/keep.txt", b"kept");

        let mut backup =
            Backup::new("Env Excl", Source::folder(&src), env.dest.id.clone()).expect("backup");
        backup.excludes = ExcludeRules::from_text("$TESTROOT/src/excl");

        let testroot = src.to_string_lossy().into_owned();
        let res = execute_backup(
            &backup,
            &env.dest,
            &env.state_dir,
            &env.runtime(),
            None,
            &[("TESTROOT", testroot.as_str())],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: execute_backup failed: {e}"));
        assert_eq!(res.exit_code, 0, "{label}");

        // Exclude file itself keeps `$TESTROOT` literal for restic to expand at runtime.
        let excl_content =
            fs::read_to_string(exclude_file_path(&env.state_dir, &backup.id)).expect("read excl");
        assert_eq!(excl_content, "$TESTROOT/src/excl\n");

        let paths = env.snapshot_relative_paths(&backup, &src, label);
        assert!(
            paths.iter().any(|p| p == "src/keep.txt"),
            "{label}: src/keep.txt missing: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with("src/excl")),
            "{label}: src/excl should be excluded: {paths:?}"
        );
    }
}

#[test]
fn ac_m2_01_5_wildcard_with_negation_keeps_dir_inner() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let env = ResticTestEnv::new(&restic_bin);
        let src = env.root().join("src");
        write_file(&src, "dir/inner/kept.txt", b"inner-keep");
        write_file(&src, "dir/other.txt", b"other-skip");
        write_file(&src, "top.txt", b"top-keep");

        let mut backup =
            Backup::new("Negation", Source::folder(&src), env.dest.id.clone()).expect("backup");
        backup.excludes = ExcludeRules::from_text("dir/*\n!dir/inner");

        let res = execute_backup(
            &backup,
            &env.dest,
            &env.state_dir,
            &env.runtime(),
            None,
            &[],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: execute_backup failed: {e}"));
        assert_eq!(res.exit_code, 0, "{label}");

        let paths = env.snapshot_relative_paths(&backup, &src, label);
        assert!(
            paths.iter().any(|p| p == "dir/inner/kept.txt"),
            "{label}: dir/inner/kept.txt missing: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p == "top.txt"),
            "{label}: top.txt missing: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p == "dir/other.txt"),
            "{label}: dir/other.txt should be excluded: {paths:?}"
        );
    }
}

#[test]
fn ac_m2_01_6_negative_regex_not_treated_as_regex_or_converted_to_glob() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let env = ResticTestEnv::new(&restic_bin);
        let src = env.root().join("src");
        write_file(&src, "notes.txt", b"keep-1");
        write_file(&src, "sub/readme.txt", b"keep-2");

        let mut backup =
            Backup::new("No Regex", Source::folder(&src), env.dest.id.clone()).expect("backup");
        backup.excludes = ExcludeRules::from_text(r".*\.txt$");

        let res = execute_backup(
            &backup,
            &env.dest,
            &env.state_dir,
            &env.runtime(),
            None,
            &[],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: execute_backup failed: {e}"));
        assert_eq!(res.exit_code, 0, "{label}");

        // Kestic must not silently rewrite regex into glob `*.txt`.
        let excl_content =
            fs::read_to_string(exclude_file_path(&env.state_dir, &backup.id)).expect("read excl");
        assert_eq!(excl_content, ".*\\.txt$\n");

        let paths = env.snapshot_relative_paths(&backup, &src, label);
        assert!(
            paths.iter().any(|p| p == "notes.txt"),
            "{label}: notes.txt missing: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p == "sub/readme.txt"),
            "{label}: sub/readme.txt missing: {paths:?}"
        );
    }
}

#[test]
fn ac_m2_01_7_and_01_9_simple_folder_rule_home_relative_vs_outside_home() {
    let home_alt = Path::new("/home/alt");

    // AC-M2-01-7: folder inside $HOME -> `$HOME/Downloads`
    let mut rules_inside = ExcludeRules {
        folders: vec!["/home/alt/Downloads".to_string()],
        ..ExcludeRules::default()
    };
    assert_eq!(
        rules_inside.to_exclude_file_contents_with_home(Some(home_alt)),
        "$HOME/Downloads\n"
    );
    rules_inside.normalize_folders(Some(home_alt));
    assert_eq!(rules_inside.folders, vec!["$HOME/Downloads"]);
    assert_eq!(
        normalize_folder_rule("/home/alt/Downloads/", Some(home_alt)),
        "$HOME/Downloads"
    );

    // AC-M2-01-9: folder outside $HOME -> absolute anchored path `/media/daten/tmp`
    let mut rules_outside = ExcludeRules {
        folders: vec!["/media/daten/tmp".to_string()],
        ..ExcludeRules::default()
    };
    assert_eq!(
        rules_outside.to_exclude_file_contents_with_home(Some(home_alt)),
        "/media/daten/tmp\n"
    );
    rules_outside.normalize_folders(Some(home_alt));
    assert_eq!(rules_outside.folders, vec!["/media/daten/tmp"]);

    // Also verify persisted exclude file via write_exclude_file_with_home.
    let dir = TempDir::new().expect("tempdir");
    let dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: dir.path().join("repo"),
        },
    )
    .expect("dest");
    let mut backup = Backup::new("Folders", Source::folder("/home/alt"), dest.id).expect("backup");
    backup.excludes = ExcludeRules {
        folders: vec![
            "/home/alt/Downloads".to_string(),
            "/media/daten/tmp".to_string(),
        ],
        ..ExcludeRules::default()
    };
    let excl_path =
        write_exclude_file_with_home(dir.path(), &backup, Some(home_alt)).expect("write excl");
    assert_eq!(
        fs::read_to_string(excl_path).expect("read excl"),
        "$HOME/Downloads\n/media/daten/tmp\n"
    );
}

#[test]
fn ac_m2_01_10_portable_home_exclude_generated_under_alt_used_under_neu() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let env = ResticTestEnv::new(&restic_bin);
        let home_alt = env.root().join("home_alt");
        let home_neu = env.root().join("home_neu");
        fs::create_dir_all(&home_alt).expect("mkdir home_alt");
        write_file(&home_neu, "Downloads/big.iso", b"iso-bytes");
        write_file(&home_neu, "Documents/report.txt", b"important");

        // Create and normalize under HOME=/home/alt -> emits `$HOME/Downloads`.
        let mut excludes = ExcludeRules {
            folders: vec![home_alt.join("Downloads").to_string_lossy().into_owned()],
            ..ExcludeRules::default()
        };
        excludes.normalize_folders(Some(&home_alt));
        assert_eq!(excludes.folders, vec!["$HOME/Downloads"]);

        let mut backup = Backup::new(
            "Portable Home",
            Source::folder(&home_neu),
            env.dest.id.clone(),
        )
        .expect("backup");
        backup.excludes = excludes;

        // First generate exclude file under HOME=/home/alt and check exact contents.
        let excl_path =
            write_exclude_file_with_home(&env.state_dir, &backup, Some(&home_alt)).expect("excl");
        assert_eq!(
            fs::read_to_string(&excl_path).expect("read excl"),
            "$HOME/Downloads\n"
        );

        // Now execute backup under HOME=/home/neu with source `/.../home_neu`.
        let neu_str = home_neu.to_string_lossy().into_owned();
        let res = execute_backup_with_inspector(
            &backup,
            &env.dest,
            &env.runtime(),
            &BackupExecutionContext {
                state_dir: &env.state_dir,
                home_dir: Some(&home_neu),
                inspector: &SystemMountInspector,
                cancel: None,
                extra_env: &[("HOME", neu_str.as_str())],
            },
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: execute_backup failed: {e}"));
        assert_eq!(res.exit_code, 0, "{label}");

        // Exclude file still stores `$HOME/Downloads` (not hardcoded to alt or neu).
        assert_eq!(
            fs::read_to_string(&excl_path).expect("read excl after"),
            "$HOME/Downloads\n"
        );

        let paths = env.snapshot_relative_paths(&backup, &home_neu, label);
        assert!(
            paths.iter().any(|p| p == "Documents/report.txt"),
            "{label}: Documents/report.txt missing: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with("Downloads")),
            "{label}: Downloads should be excluded under HOME=/home/neu: {paths:?}"
        );
    }
}

#[test]
fn ac_m2_01_8_expert_rule_with_comma_or_space_preserved_verbatim() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let env = ResticTestEnv::new(&restic_bin);
        let src = env.root().join("src");
        write_file(&src, "folder with spaces/skip me, please.txt", b"skip-1");
        write_file(&src, "folder with spaces/keep me.txt", b"keep-1");
        write_file(&src, "comma,dir/ignored.dat", b"skip-2");
        write_file(&src, "normal.txt", b"keep-2");

        let mut backup = Backup::new("Special Chars", Source::folder(&src), env.dest.id.clone())
            .expect("backup");
        backup.excludes =
            ExcludeRules::from_text("folder with spaces/skip me, please.txt\ncomma,dir");

        let res = execute_backup(
            &backup,
            &env.dest,
            &env.state_dir,
            &env.runtime(),
            None,
            &[],
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: execute_backup failed: {e}"));
        assert_eq!(res.exit_code, 0, "{label}");

        let excl_content =
            fs::read_to_string(exclude_file_path(&env.state_dir, &backup.id)).expect("read excl");
        let excl_lines: Vec<&str> = excl_content.lines().collect();
        assert_eq!(
            excl_lines,
            vec!["folder with spaces/skip me, please.txt", "comma,dir"],
            "{label}: lines must be preserved verbatim"
        );

        let paths = env.snapshot_relative_paths(&backup, &src, label);
        assert!(
            paths.iter().any(|p| p == "folder with spaces/keep me.txt"),
            "{label}: keep me.txt missing: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p == "normal.txt"),
            "{label}: normal.txt missing: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("skip me, please.txt")),
            "{label}: skip me, please.txt should be excluded: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with("comma,dir")),
            "{label}: comma,dir should be excluded: {paths:?}"
        );
    }
}

#[test]
fn ac_m2_03_1_lint_warns_on_negation_under_whole_dir() {
    let rules = ExcludeRules::from_text("dir\n!dir/inner");
    let warnings = lint_exclude_rules(&rules);
    assert_eq!(
        warnings,
        vec![ExcludeLintWarning {
            line_number: 2,
            rule: "!dir/inner".to_string(),
            parent_rule: "dir".to_string(),
            suggested_fix: "dir/*".to_string(),
        }]
    );

    // Also works when whole directory comes from simple folder rules.
    let folder_rules = ExcludeRules {
        folders: vec!["/data/cache".to_string()],
        lines: vec!["!/data/cache/keep".to_string()],
        ..ExcludeRules::default()
    };
    let folder_warnings = lint_exclude_rules(&folder_rules);
    assert_eq!(folder_warnings.len(), 1);
    assert_eq!(folder_warnings[0].line_number, 1);
    assert_eq!(folder_warnings[0].rule, "!/data/cache/keep");
    assert_eq!(folder_warnings[0].parent_rule, "/data/cache");
    assert_eq!(folder_warnings[0].suggested_fix, "/data/cache/*");
}

#[test]
fn ac_m2_03_2_negative_wildcard_dir_star_with_negation_has_zero_warnings() {
    let rules = ExcludeRules::from_text("dir/*\n!dir/inner");
    assert!(lint_exclude_rules(&rules).is_empty());

    let double_star = ExcludeRules::from_text("dir/**\n!dir/inner");
    assert!(lint_exclude_rules(&double_star).is_empty());
}

#[test]
fn ac_m2_03_3_lint_warning_does_not_block_validation_or_saving() {
    let dir = TempDir::new().expect("tempdir");
    let config_file = dir.path().join("config.toml");

    let dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: dir.path().join("repo"),
        },
    )
    .expect("dest");
    let mut backup =
        Backup::new("Warn Save", Source::folder("/tmp/src"), dest.id.clone()).expect("backup");
    backup.excludes = ExcludeRules::from_text("dir\n!dir/inner");

    // Precondition: lint warning is present.
    let warnings = lint_exclude_rules(&backup.excludes);
    assert_eq!(warnings.len(), 1);

    // Must still validate and persist cleanly.
    backup.validate().expect("backup validate succeeds");
    let cfg = AppConfig {
        config_version: CURRENT_CONFIG_VERSION,
        destinations: vec![dest],
        backups: vec![backup],
        ..AppConfig::default()
    };
    cfg.validate().expect("config validate succeeds");
    save_config(&config_file, &cfg).expect("save_config succeeds with lint warning");
    let loaded = load_config(&config_file).expect("load_config");
    assert_eq!(loaded, cfg);
}

#[test]
fn evaluate_exclude_path_covers_folders_trash_negations_and_ignore_case() {
    let home = Path::new("/home/alice");
    let source = Path::new("/home/alice");

    // Working negation vs shadowed negation.
    let working = ExcludeRules::from_text("dir/*\n!dir/inner");
    let res_inner = evaluate_exclude_path(&working, source, Some(home), "dir/inner");
    assert!(res_inner.included);
    assert_eq!(res_inner.matched_rule.as_deref(), Some("!dir/inner"));
    assert_eq!(res_inner.matched_line, Some(2));
    assert_eq!(res_inner.matched_source, "expert");

    let res_other = evaluate_exclude_path(&working, source, Some(home), "dir/other.txt");
    assert!(!res_other.included);
    assert_eq!(res_other.matched_rule.as_deref(), Some("dir/*"));
    assert_eq!(res_other.matched_line, Some(1));

    let shadowed = ExcludeRules::from_text("dir\n!dir/inner");
    let res_shadowed = evaluate_exclude_path(&shadowed, source, Some(home), "dir/inner/file.txt");
    assert!(!res_shadowed.included);
    assert_eq!(res_shadowed.matched_rule.as_deref(), Some("dir"));
    assert_eq!(res_shadowed.matched_line, Some(1));

    // Simple folder + trash + ignore_case.
    let mut complex = ExcludeRules {
        folders: vec!["/home/alice/Downloads".to_string()],
        lines: vec!["*.TMP".to_string()],
        exclude_trash: true,
        ignore_case: true,
        ..ExcludeRules::default()
    };

    let folder_hit = evaluate_exclude_path(
        &complex,
        source,
        Some(home),
        "/home/alice/Downloads/archive.zip",
    );
    assert!(!folder_hit.included);
    assert_eq!(folder_hit.matched_source, "folder");

    let trash_hit = evaluate_exclude_path(
        &complex,
        source,
        Some(home),
        "/home/alice/.local/share/Trash/files/old.doc",
    );
    assert!(!trash_hit.included);
    assert_eq!(trash_hit.matched_source, "trash");

    let vol_trash_hit =
        evaluate_exclude_path(&complex, source, Some(home), "sub/.Trash-1000/deleted.txt");
    assert!(!vol_trash_hit.included);
    assert_eq!(vol_trash_hit.matched_source, "trash");

    let case_hit = evaluate_exclude_path(&complex, source, Some(home), "notes/draft.tmp");
    assert!(!case_hit.included);
    assert_eq!(case_hit.matched_rule.as_deref(), Some("*.TMP"));

    complex.ignore_case = false;
    let case_miss = evaluate_exclude_path(&complex, source, Some(home), "notes/draft.tmp");
    assert!(case_miss.included);
    assert_eq!(case_miss.matched_source, "none");

    let empty = evaluate_exclude_path(&complex, source, Some(home), "   ");
    assert!(empty.included);
    assert_eq!(empty.matched_source, "empty");
}

#[test]
fn exclude_options_validation_and_command_flags() {
    // Size validation.
    assert_eq!(validate_exclude_larger_than("500M").unwrap(), "500M");
    assert_eq!(validate_exclude_larger_than(" 2G ").unwrap(), "2G");
    assert_eq!(validate_exclude_larger_than("1024").unwrap(), "1024");
    assert!(validate_exclude_larger_than("").is_err());
    assert!(validate_exclude_larger_than("0").is_err());
    assert!(validate_exclude_larger_than("0G").is_err());
    assert!(validate_exclude_larger_than("500X").is_err());
    assert!(validate_exclude_larger_than("-10M").is_err());

    let bad_size = ExcludeRules {
        exclude_larger_than: Some("0M".to_string()),
        ..ExcludeRules::default()
    };
    assert!(bad_size.validate().is_err());

    let bad_marker = ExcludeRules {
        exclude_if_present: vec!["   ".to_string()],
        ..ExcludeRules::default()
    };
    assert!(bad_marker.validate().is_err());

    let bad_folder = ExcludeRules {
        folders: vec!["/tmp/a\nb".to_string()],
        ..ExcludeRules::default()
    };
    assert!(bad_folder.validate().is_err());

    // Command flags: `--iexclude-file`, `--exclude-caches`, `--exclude-larger-than`, `--exclude-if-present`.
    let dir = TempDir::new().expect("tempdir");
    let dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: dir.path().join("repo"),
        },
    )
    .expect("dest");
    let mut backup = Backup::new("Flags", Source::folder("/tmp/src"), dest.id.clone()).expect("bk");
    backup.excludes = ExcludeRules {
        folders: Vec::new(),
        lines: vec!["*.tmp".to_string()],
        exclude_caches: true,
        exclude_trash: true,
        exclude_larger_than: Some("10M".to_string()),
        ignore_case: true,
        exclude_if_present: vec![".nobackup".to_string(), ".kestic-ignore".to_string()],
    };

    let spec = build_backup_command_spec_with_home(
        &backup,
        &dest,
        dir.path(),
        &ResolvedRestic {
            binary: PathBuf::from("/usr/bin/restic"),
            version: semver::Version::new(0, 18, 0),
        },
        Path::new("/usr/bin/kestic"),
        Some(Path::new("/home/alice")),
    )
    .expect("build spec");

    let excl_path = exclude_file_path(dir.path(), &backup.id)
        .to_string_lossy()
        .into_owned();
    assert_has_flag_pair(&spec.subcommand_args, "--iexclude-file", &excl_path);
    assert!(!spec.subcommand_args.iter().any(|a| a == "--exclude-file"));
    assert!(spec.subcommand_args.iter().any(|a| a == "--exclude-caches"));
    assert_has_flag_pair(&spec.subcommand_args, "--exclude-larger-than", "10M");
    assert_has_flag_pair(&spec.subcommand_args, "--exclude-if-present", ".nobackup");
    assert_has_flag_pair(
        &spec.subcommand_args,
        "--exclude-if-present",
        ".kestic-ignore",
    );
    assert_has_flag_pair(&spec.subcommand_args, "--", "/tmp/src");

    let excl_content = fs::read_to_string(exclude_file_path(dir.path(), &backup.id)).unwrap();
    for pat in TRASH_EXCLUDE_PATTERNS {
        assert!(excl_content.contains(pat), "missing trash pattern {pat}");
    }
}

#[test]
fn exclude_options_integration_size_trash_ignore_case_and_if_present() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let env = ResticTestEnv::new(&restic_bin);
        let fake_home = env.root().join("home_user");

        write_file(&fake_home, "keep.txt", b"small-keep");
        write_file(&fake_home, "large.bin", &vec![b'A'; 4096]);
        write_file(&fake_home, "skip.TMP", b"upper-tmp");
        write_file(&fake_home, "skip2.Tmp", b"mixed-tmp");
        write_file(&fake_home, ".local/share/Trash/files/old.txt", b"trash-1");
        write_file(&fake_home, "vol/.Trash-1000/trashed.txt", b"trash-2");
        write_file(&fake_home, "marked/.nobackup", b"");
        write_file(&fake_home, "marked/secret.txt", b"marker-skip");
        write_file(&fake_home, "unmarked/visible.txt", b"marker-keep");

        let mut backup = Backup::new(
            "Options Integration",
            Source::folder(&fake_home),
            env.dest.id.clone(),
        )
        .expect("backup");
        backup.excludes = ExcludeRules {
            folders: Vec::new(),
            lines: vec!["*.tmp".to_string()],
            exclude_caches: false,
            exclude_trash: true,
            exclude_larger_than: Some("1k".to_string()),
            ignore_case: true,
            exclude_if_present: vec![".nobackup".to_string()],
        };

        let home_str = fake_home.to_string_lossy().into_owned();
        let res = execute_backup_with_inspector(
            &backup,
            &env.dest,
            &env.runtime(),
            &BackupExecutionContext {
                state_dir: &env.state_dir,
                home_dir: Some(&fake_home),
                inspector: &SystemMountInspector,
                cancel: None,
                extra_env: &[("HOME", home_str.as_str())],
            },
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: execute_backup failed: {e}"));
        assert_eq!(res.exit_code, 0, "{label}");

        let paths = env.snapshot_relative_paths(&backup, &fake_home, label);
        assert!(
            paths.iter().any(|p| p == "keep.txt"),
            "{label}: keep.txt missing: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p == "unmarked/visible.txt"),
            "{label}: unmarked/visible.txt missing: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p == "large.bin"),
            "{label}: large.bin should be excluded by --exclude-larger-than 1k: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p == "skip.TMP"),
            "{label}: case-insensitive *.tmp should exclude skip.TMP: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p == "skip2.Tmp"),
            "{label}: case-insensitive *.tmp should exclude skip2.Tmp: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains(".local/share/Trash")),
            "{label}: the user trash should be excluded: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains(".Trash-1000")),
            "{label}: the per-drive trash should be excluded: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p == "marked/secret.txt"),
            "{label}: marked/secret.txt should be excluded by .nobackup: {paths:?}"
        );
    }
}

#[test]
fn dry_run_preview_uses_isolated_temp_exclude_file_and_cleans_up() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let env = ResticTestEnv::new(&restic_bin);
        let src = env.root().join("src");
        write_file(&src, "keep.txt", b"keep");
        write_file(&src, "draft_only.log", b"draft");

        let mut saved_backup =
            Backup::new("Saved", Source::folder(&src), env.dest.id.clone()).expect("backup");
        saved_backup.excludes = ExcludeRules {
            lines: vec!["*.tmp".to_string()],
            exclude_caches: false,
            exclude_trash: false,
            ..ExcludeRules::default()
        };
        let saved_excl_path =
            write_exclude_file_with_home(&env.state_dir, &saved_backup, Some(env.root()))
                .expect("write saved excl");
        let saved_before = fs::read_to_string(&saved_excl_path).expect("read saved excl");
        assert_eq!(saved_before, "*.tmp\n");

        let mut preview_backup = saved_backup;
        preview_backup.excludes.lines = vec!["*.log".to_string()];

        let dry_res = execute_backup_dry_run_with(
            &preview_backup,
            &env.dest,
            &env.runtime(),
            &BackupExecutionContext {
                state_dir: &env.state_dir,
                home_dir: Some(env.root()),
                inspector: &SystemMountInspector,
                cancel: None,
                extra_env: &[],
            },
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}: dry run failed: {e}"));

        assert_eq!(dry_res.total_files, 1, "{label}");
        assert!(
            dry_res.files.iter().any(|f| f.ends_with("/keep.txt")),
            "{label}: expected keep.txt in {:?}",
            dry_res.files
        );
        assert!(
            !dry_res.files.iter().any(|f| f.ends_with("/draft_only.log")),
            "{label}: draft_only.log should be excluded in preview: {:?}",
            dry_res.files
        );

        let saved_after = fs::read_to_string(&saved_excl_path).expect("read saved excl after");
        assert_eq!(
            saved_after, saved_before,
            "{label}: dry-run preview must not overwrite saved exclude file"
        );

        let leftovers: Vec<_> = fs::read_dir(env.state_dir.join("excludes"))
            .expect("read excludes dir")
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| name.contains(".preview-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "{label}: preview temp exclude files must be deleted: {leftovers:?}"
        );
    }
}

#[test]
fn differential_evaluate_exclude_path_matches_restic_dry_run() {
    for (label, restic_bin) in common::restic_test_binaries() {
        for ignore_case in [false, true] {
            let env = ResticTestEnv::new(&restic_bin);
            let fake_home = env.root().join("home_diff");
            let home_str = fake_home.to_string_lossy().into_owned();

            let rel_files = [
                "docs/readme.txt",
                "docs/draft.tmp",
                "docs/UPPER.TMP",
                "build/app.o",
                "a/b/c/deep/file.log",
                "a/b/c/deep/keep.txt",
                "cache/sub/item.bin",
                "partial/temp1.dat",
                "partial/important.dat",
                "shadowed/nested/keep_me.txt",
                "folder with spaces/skip me, please.txt",
                "folder with spaces/keep me.txt",
                ".local/share/Trash/files/old.doc",
                "vol/.Trash-1000/trashed.txt",
                "exact_anchor/hit.txt",
                "sub/exact_anchor/miss.txt",
                "glob_q/a1.dat",
                "glob_q/a12.dat",
                "glob_cls/x2.log",
                "glob_cls/x9.log",
                "multi/seg/match/z.bak",
                "other/match/z.bak",
                "tilde_or_dollar/secret.key",
                "trailing_slash/inside.txt",
                "case_dir/MixedCase.Log",
            ];
            for rel in &rel_files {
                write_file(&fake_home, rel, b"diff-payload");
            }

            let anchored_exact = format!("{}/exact_anchor/*.txt", fake_home.display());
            let mut backup = Backup::new(
                "Differential",
                Source::folder(&fake_home),
                env.dest.id.clone(),
            )
            .expect("backup");
            backup.excludes = ExcludeRules {
                folders: vec![fake_home.join("cache").to_string_lossy().into_owned()],
                lines: vec![
                    "*.tmp".to_string(),
                    "*.o".to_string(),
                    "**/deep/*.log".to_string(),
                    "partial/*".to_string(),
                    "!partial/important.dat".to_string(),
                    "shadowed".to_string(),
                    "!shadowed/nested/keep_me.txt".to_string(),
                    "folder with spaces/skip me, please.txt".to_string(),
                    anchored_exact,
                    "a?.dat".to_string(),
                    "x[1-3].log".to_string(),
                    "seg/match/*.bak".to_string(),
                    "$HOME/tilde_or_dollar/*.key".to_string(),
                    "trailing_slash/".to_string(),
                    "*mixedcase*".to_string(),
                ],
                exclude_caches: false,
                exclude_trash: true,
                exclude_larger_than: None,
                ignore_case,
                exclude_if_present: Vec::new(),
            };

            let dry = execute_backup_dry_run_with(
                &backup,
                &env.dest,
                &env.runtime(),
                &BackupExecutionContext {
                    state_dir: &env.state_dir,
                    home_dir: Some(&fake_home),
                    inspector: &SystemMountInspector,
                    cancel: None,
                    extra_env: &[("HOME", home_str.as_str())],
                },
                |_| {},
            )
            .unwrap_or_else(|e| panic!("{label} (ignore_case={ignore_case}): dry-run failed: {e}"));

            let included_by_restic: std::collections::HashSet<PathBuf> =
                dry.files.iter().map(PathBuf::from).collect();

            for rel in &rel_files {
                let abs_path = fake_home.join(rel);
                let restic_included = included_by_restic.contains(&abs_path);
                let eval = evaluate_exclude_path(
                    &backup.excludes,
                    &fake_home,
                    Some(&fake_home),
                    &abs_path.to_string_lossy(),
                );
                assert_eq!(
                    eval.included, restic_included,
                    "{label} (ignore_case={ignore_case}): mismatch for '{rel}': evaluate={eval:?}, restic_included={restic_included}, restic_files={:?}",
                    dry.files
                );
            }
        }
    }
}
