// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

#![forbid(unsafe_code)]

mod common;

use common::execute_backup_dry_run;

use kestic_test_support::repo_template::copy_template_repo;
use kestic_test_support::tools::write_executable;

use kestic_core::backup::{BackupRunError, build_backup_command_spec, list_snapshots_for_backup};
use kestic_core::destination::{
    DEFAULT_CONNECT_TIMEOUT, DestinationError, DestinationSetupMode, setup_destination,
};
use kestic_core::model::{Backup, Destination, DestinationKind, ExcludeRules, Source};
use kestic_core::provider::ResolvedRestic;
use kestic_core::restic::{ResticStreamEvent, parse_json_line};
use kestic_core::secrets::SecretValue;
use kestic_core::snapshot::ResticSnapshot;
use semver::Version;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use time::macros::datetime;

fn count_all_repo_snapshots(
    restic_bin: &Path,
    repo_uri: &str,
    password: &str,
    cache_dir: &Path,
) -> usize {
    let out = Command::new(restic_bin)
        .args(["-r", repo_uri, "snapshots", "--no-lock", "--json"])
        .env("RESTIC_PASSWORD", password)
        .env("RESTIC_CACHE_DIR", cache_dir)
        .output()
        .expect("run restic snapshots");
    assert!(
        out.status.success(),
        "restic snapshots failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str::<Vec<ResticSnapshot>>(stdout.trim())
        .expect("parse snapshots json")
        .len()
}

fn run_direct_restic_dry_run(
    label: &str,
    backup: &Backup,
    dest: &Destination,
    state_dir: &Path,
    rig: &common::ResticRig,
    repo_pw: &str,
) -> (Vec<String>, Option<u64>, Option<u64>) {
    let mut direct_spec =
        build_backup_command_spec(backup, dest, state_dir, &rig.resolved, &rig.helper)
            .expect("build spec");
    let sep_idx = direct_spec
        .subcommand_args
        .iter()
        .position(|a| a == "--")
        .unwrap_or(direct_spec.subcommand_args.len());
    direct_spec.subcommand_args.splice(
        sep_idx..sep_idx,
        ["--dry-run".to_string(), "-vv".to_string()],
    );
    let direct_out = Command::new(&rig.resolved.binary)
        .args(direct_spec.restic_argv())
        .env("RESTIC_PASSWORD", repo_pw)
        .env("RESTIC_CACHE_DIR", &rig.cache_dir)
        .output()
        .expect("run direct restic dry-run");
    assert!(
        direct_out.status.success(),
        "{label}: direct dry-run failed: {}",
        String::from_utf8_lossy(&direct_out.stderr)
    );

    let mut direct_files = Vec::new();
    let mut direct_total_bytes = None;
    let mut direct_total_files = None;
    for line in String::from_utf8_lossy(&direct_out.stdout).lines() {
        if let Ok(ev) = parse_json_line(line) {
            match ev {
                ResticStreamEvent::VerboseStatus(vs)
                    if vs.action != "scan_finished"
                        && !vs.item.is_empty()
                        && !vs.item.ends_with('/') =>
                {
                    direct_files.push(vs.item);
                }
                ResticStreamEvent::Summary(stats) => {
                    direct_total_bytes = Some(stats.total_bytes_processed);
                    direct_total_files = Some(stats.total_files_processed);
                }
                _ => {}
            }
        }
    }
    (direct_files, direct_total_bytes, direct_total_files)
}

#[test]
fn ac_m2_04_1_dry_run_matches_direct_restic_and_creates_zero_snapshots() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let cache_str = cache_dir.to_string_lossy().into_owned();
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        let src_dir = dir.path().join("src");
        let sub_dir = src_dir.join("sub");
        let excluded_dir = src_dir.join("cache_dir");
        fs::create_dir_all(&sub_dir).expect("mkdir sub");
        fs::create_dir_all(&excluded_dir).expect("mkdir excluded");

        fs::write(src_dir.join("alpha.txt"), vec![b'a'; 120]).expect("write alpha");
        fs::write(sub_dir.join("beta.bin"), vec![b'b'; 380]).expect("write beta");
        fs::write(src_dir.join("ignored.tmp"), vec![b't'; 500]).expect("write tmp");
        fs::write(excluded_dir.join("garbage.dat"), vec![b'g'; 900]).expect("write garbage");

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-11-17 08:00:00 UTC));
        let runtime = rig.runtime();

        let (dest, repo_pw) = rig.local_repo("Local DryRun", &repo_dir);

        let mut excludes = ExcludeRules::from_text("*.tmp");
        excludes
            .folders
            .push(excluded_dir.to_string_lossy().into_owned());

        let mut backup =
            Backup::new("DryRun Test", Source::folder(&src_dir), dest.id.clone()).expect("backup");
        backup.excludes = excludes;

        let dry_res =
            execute_backup_dry_run(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
                .unwrap_or_else(|e| panic!("{label}: execute_backup_dry_run failed: {e}"));

        assert!(!dry_res.cancelled, "{label}: should not be cancelled");
        assert!(
            dry_res.transparent_command.contains("--dry-run"),
            "{label}: transparent command missing --dry-run: {}",
            dry_res.transparent_command
        );
        assert!(
            dry_res.transparent_command.contains("-vv"),
            "{label}: transparent command missing -vv: {}",
            dry_res.transparent_command
        );

        let (mut direct_files, direct_total_bytes, direct_total_files) =
            run_direct_restic_dry_run(label, &backup, &dest, &state_dir, &rig, &repo_pw);

        let mut kestic_files = dry_res.files.clone();
        kestic_files.sort();
        direct_files.sort();

        assert_eq!(
            kestic_files, direct_files,
            "{label}: dry-run file list must match direct `restic backup --dry-run -vv`"
        );
        assert_eq!(kestic_files.len(), 2, "{label}: expected 2 included files");
        assert!(
            kestic_files.iter().any(|p| p.ends_with("alpha.txt")),
            "{label}: missing alpha.txt in {kestic_files:?}"
        );
        assert!(
            kestic_files.iter().any(|p| p.ends_with("beta.bin")),
            "{label}: missing beta.bin in {kestic_files:?}"
        );
        assert!(
            !kestic_files.iter().any(|p| p.contains("ignored.tmp")),
            "{label}: ignored.tmp leaked into dry-run list: {kestic_files:?}"
        );
        assert!(
            !kestic_files.iter().any(|p| p.contains("garbage.dat")),
            "{label}: garbage.dat leaked into dry-run list: {kestic_files:?}"
        );

        assert_eq!(
            Some(dry_res.total_bytes),
            direct_total_bytes,
            "{label}: total_bytes mismatch"
        );
        assert_eq!(
            dry_res.total_bytes, 500,
            "{label}: expected 120 + 380 = 500 bytes"
        );
        assert_eq!(
            Some(dry_res.total_files),
            direct_total_files,
            "{label}: total_files mismatch"
        );
        assert_eq!(dry_res.total_files, 2, "{label}: expected 2 total files");

        assert!(
            dry_res
                .largest_excluded_folders
                .iter()
                .any(|e| e.path == excluded_dir.to_string_lossy() && e.bytes == 900),
            "{label}: expected excluded folder estimate for cache_dir (900 B), got {:?}",
            dry_res.largest_excluded_folders
        );

        assert_eq!(
            count_all_repo_snapshots(
                &rig.resolved.binary,
                &dest.kind.repository_uri(),
                &repo_pw,
                &cache_dir
            ),
            0,
            "{label}: dry-run must create zero snapshots"
        );
        let snaps = list_snapshots_for_backup(
            &backup.id,
            &dest,
            &rig.resolved,
            &rig.helper,
            &rig.store,
            &rig.clock,
            &[("RESTIC_CACHE_DIR", cache_str.as_str())],
        )
        .expect("list snapshots");
        assert!(snaps.is_empty(), "{label}: expected 0 snapshots for backup");
    }
}

#[test]
fn ac_m2_04_2_dry_run_with_zero_prior_snapshots_reports_first_backup_estimate() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let cache_dir = common::restic_cache_dir(dir.path());
        let repo_dir = dir.path().join("repo");
        let state_dir = dir.path().join("state");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("mkdir src");

        fs::write(src_dir.join("doc1.txt"), vec![1u8; 1024]).expect("write doc1");
        fs::write(src_dir.join("doc2.txt"), vec![2u8; 2048]).expect("write doc2");

        let rig =
            common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-11-17 08:15:00 UTC));
        let runtime = rig.runtime();

        let (dest, repo_pw) = rig.local_repo("Empty Repo", &repo_dir);

        assert_eq!(
            count_all_repo_snapshots(
                &rig.resolved.binary,
                &dest.kind.repository_uri(),
                &repo_pw,
                &cache_dir
            ),
            0,
            "{label}: precondition failed, repo should have 0 snapshots"
        );

        let backup =
            Backup::new("First DryRun", Source::folder(&src_dir), dest.id.clone()).expect("backup");
        let dry_res =
            execute_backup_dry_run(&backup, &dest, &state_dir, &runtime, None, &[], |_| {})
                .unwrap_or_else(|e| panic!("{label}: dry-run on empty repo failed: {e}"));

        assert!(!dry_res.cancelled, "{label}");
        assert_eq!(dry_res.total_files, 2, "{label}: total_files");
        assert_eq!(dry_res.files_new, 2, "{label}: files_new");
        assert_eq!(dry_res.files_changed, 0, "{label}: files_changed");
        assert_eq!(dry_res.files_unmodified, 0, "{label}: files_unmodified");
        assert_eq!(dry_res.total_bytes, 3072, "{label}: total_bytes");
        assert_eq!(dry_res.files.len(), 2, "{label}: listed files");
        assert_eq!(
            count_all_repo_snapshots(
                &rig.resolved.binary,
                &dest.kind.repository_uri(),
                &repo_pw,
                &cache_dir
            ),
            0,
            "{label}: dry-run must leave 0 snapshots in repo"
        );
    }
}

#[test]
fn ac_m2_04_3_stopped_rest_server_preflight_fails_within_15s_without_spawning_restic() {
    assert_eq!(
        DEFAULT_CONNECT_TIMEOUT,
        Duration::from_secs(10),
        "the preflight connect timeout is 10 s"
    );

    let dir = TempDir::new().expect("tempdir");
    let _cache_dir = common::restic_cache_dir(dir.path());
    let server_dir = dir.path().join("rest-data");
    let state_dir = dir.path().join("state");
    let src_dir = dir.path().join("src");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("file.txt"), b"hello rest").expect("write file");

    let server = kestic_test_support::tools::RestServerGuard::start_with_auth(&server_dir);
    let server_url = server.url.clone();

    let (_label, restic_bin) = common::restic_test_binaries()
        .into_iter()
        .next()
        .expect("restic binary");
    let rig = common::ResticRig::new(dir.path(), &restic_bin, datetime!(2026-11-17 08:30:00 UTC));
    let real_runtime = rig.runtime();

    let dest = Destination::new(
        "REST DryRun",
        DestinationKind::Rest {
            url: server_url,
            username: Some("alice".to_string()),
        },
    )
    .expect("dest");
    // The repository is only setup for the preflight check: a prepared copy served by
    // `rest-server`, connected through the production setup path.
    let repo = copy_template_repo(&rig.resolved.binary, &server.repo_dir());
    let repo_pw = SecretValue::from_str_secret(&repo.password);
    let rest_pw = SecretValue::from_str_secret(server.password());
    setup_destination(
        &dest,
        DestinationSetupMode::ConnectExisting,
        &repo_pw,
        Some(&rest_pw),
        &real_runtime,
    )
    .expect("connect REST repo");

    // Stop `rest-server` so the destination becomes unreachable.
    drop(server);

    // Replace restic binary with a spy script to prove zero restic processes are spawned.
    let marker_file = dir.path().join("restic-was-spawned.marker");
    let spy_bin = dir.path().join("spy-restic");
    let spy_script = format!(
        "#!/bin/sh\ntouch '{}'\nexit 99\n",
        marker_file.to_string_lossy()
    );
    write_executable(&spy_bin, &spy_script);

    let spy_resolved = ResolvedRestic {
        binary: spy_bin,
        version: Version::new(0, 18, 0),
    };
    let spy_runtime = rig.runtime_with(&spy_resolved);

    let backup =
        Backup::new("REST DryRun", Source::folder(&src_dir), dest.id.clone()).expect("backup");

    let start = Instant::now();
    let err = execute_backup_dry_run(&backup, &dest, &state_dir, &spy_runtime, None, &[], |_| {})
        .expect_err("dry-run against stopped rest-server must fail preflight");
    let elapsed = start.elapsed();

    assert!(
        elapsed <= Duration::from_secs(15),
        "preflight failure exceeded 15s SLA: {elapsed:?}"
    );
    assert!(
        matches!(
            err,
            BackupRunError::Destination(DestinationError::Unreachable(_))
        ),
        "expected DestinationError::Unreachable, got: {err:?}"
    );
    assert!(
        err.to_string().contains("destination unreachable"),
        "error must report destination unreachable per FR-37, got: {err}"
    );
    assert!(
        !marker_file.exists(),
        "zero restic processes must be spawned when preflight fails"
    );
}
