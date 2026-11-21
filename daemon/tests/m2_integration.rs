// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

mod common;

use common::load_config;
use common::m3::{M3Daemon, M3DaemonOpts, ResticWrapper, assert_dbus_error_name, spawn_m3_daemon};
use common::tools::{
    RestServerGuard, assert_has_flag_pair, find_lock_file, install_args_spy, install_marker_spy,
};
use common::{
    CapturedKesticSignal, KesticSignalCollector, fetch_dbusmenu_items, restic_test_binaries,
    run_isolated_test, start_fake_notifications, start_fake_sni_watcher, wait_for_sni_service,
};
use kestic_core::backup::exclude_file_path;
use kestic_core::history::RunHistory;
use kestic_core::model::{
    Backup, Destination, DestinationKind, ExcludeRules, KeepCount, ResticProvider, RetentionPolicy,
    Run, RunKind, RunStatus, Source, SourceIdentity, SourceKind,
};
use kestic_core::mount::{FakeMountInspector, FakeMountState};
use kestic_daemon::dbus_api::{
    KESTIC_BUS_NAME, KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH, KesticClient,
};
use kestic_i18n::codes::BlockedReason;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use zbus::{Connection, block_on};

fn retention_to_explicit_json(ret: &RetentionPolicy) -> serde_json::Value {
    serde_json::json!({
        "keep_last": ret.keep_last,
        "keep_hourly": ret.keep_hourly,
        "keep_daily": ret.keep_daily,
        "keep_weekly": ret.keep_weekly,
        "keep_monthly": ret.keep_monthly,
        "keep_yearly": ret.keep_yearly,
        "keep_within": ret.keep_within,
        "keep_within_hourly": ret.keep_within_hourly,
        "keep_within_daily": ret.keep_within_daily,
        "keep_within_weekly": ret.keep_within_weekly,
        "keep_within_monthly": ret.keep_within_monthly,
        "keep_within_yearly": ret.keep_within_yearly,
    })
}

fn backup_to_explicit_json(backup: &Backup) -> String {
    let mut val = serde_json::to_value(backup).expect("serialize backup");
    val["retention"] = retention_to_explicit_json(&backup.retention);
    serde_json::to_string(&val).expect("serialize backup json")
}

fn add_backup_explicit(
    client: &KesticClient,
    bus_name: &str,
    backup: &Backup,
) -> Result<String, zbus::Error> {
    let json = backup_to_explicit_json(backup);
    let conn = client.connection().clone();
    let bus = bus_name.to_string();
    block_on(async move {
        let proxy =
            zbus::Proxy::new(&conn, bus.as_str(), KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1).await?;
        proxy.call("AddBackup", &(json,)).await
    })
}

fn update_backup_explicit(
    client: &KesticClient,
    bus_name: &str,
    backup: &Backup,
) -> Result<(), zbus::Error> {
    let json = backup_to_explicit_json(backup);
    let conn = client.connection().clone();
    let bus = bus_name.to_string();
    block_on(async move {
        let proxy =
            zbus::Proxy::new(&conn, bus.as_str(), KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1).await?;
        proxy.call("UpdateBackup", &(json,)).await
    })
}

fn spawn_daemon_on_bus(
    root: &Path,
    sub_dir: &Path,
    bus_name: &str,
    restic_path: PathBuf,
    mount_inspector: Option<Arc<dyn kestic_core::mount::MountInspector>>,
    assume_sni_available: bool,
) -> M3Daemon {
    // `sub_dir` is absolute, so it replaces `root` in the sandbox path.
    let sub = sub_dir.to_string_lossy();
    spawn_m3_daemon(
        root,
        M3DaemonOpts {
            mount_inspector,
            assume_sni_available,
            ..M3DaemonOpts::on_request(&sub, bus_name, &restic_path)
        },
    )
}

fn wait_for_run_finished(signals: &KesticSignalCollector, run_id: &str, timeout: Duration) -> Run {
    let captured = signals.wait_until(timeout, |sigs| {
        sigs.iter().any(|s| {
            if let CapturedKesticSignal::RunUpdated(json) = s
                && let Ok(run) = serde_json::from_str::<Run>(json)
            {
                return run.id.as_str() == run_id && run.status != RunStatus::Running;
            }
            false
        })
    });
    captured
        .iter()
        .rev()
        .find_map(|s| {
            if let CapturedKesticSignal::RunUpdated(json) = s
                && let Ok(run) = serde_json::from_str::<Run>(json)
                && run.id.as_str() == run_id
                && run.status != RunStatus::Running
            {
                return Some(run);
            }
            None
        })
        .expect("finished RunUpdated signal")
}

struct NonExclusiveLockHolder {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl NonExclusiveLockHolder {
    fn start(restic_bin: &Path, repo_dir: &Path, password: &str) -> Self {
        let cache_dir = repo_dir.parent().unwrap_or(repo_dir).join("restic-cache");
        let mut child = Command::new(restic_bin)
            .args([
                "-r",
                &repo_dir.to_string_lossy(),
                "backup",
                "--stdin",
                "--stdin-filename",
                "lock-holder.txt",
            ])
            .env("RESTIC_PASSWORD", password)
            .env("RESTIC_CACHE_DIR", cache_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn restic backup --stdin");

        let mut stdin = child.stdin.take().expect("piped stdin");
        stdin
            .write_all(b"holding non-exclusive lock\n")
            .expect("write initial stdin byte");
        stdin.flush().expect("flush stdin");

        let locks_dir = repo_dir.join("locks");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if find_lock_file(&locks_dir).is_some() {
                // Give restic a brief moment to finish flushing the lock file.
                thread::sleep(Duration::from_millis(30));
                break;
            }
            if let Some(status) = child.try_wait().expect("try_wait lock holder") {
                panic!("restic backup --stdin exited prematurely with {status}");
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for non-exclusive lock in {}",
                locks_dir.display()
            );
            thread::sleep(Duration::from_millis(10));
        }

        Self {
            child,
            stdin: Some(stdin),
        }
    }

    fn release(mut self, repo_dir: &Path) {
        drop(self.stdin.take());
        let status = self.child.wait().expect("wait restic backup --stdin");
        assert!(status.success(), "lock holder exited with {status}");

        let locks_dir = repo_dir.join("locks");
        let deadline = Instant::now() + Duration::from_secs(5);
        while find_lock_file(&locks_dir).is_some() {
            assert!(
                Instant::now() < deadline,
                "lock file remained after lock holder exited"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for NonExclusiveLockHolder {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn is_running_as_root() -> bool {
    if let Ok(status) = fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("Uid:") {
                let mut fields = rest.split_whitespace();
                let _real = fields.next();
                if let Some(euid) = fields.next() {
                    return euid == "0";
                }
            }
        }
    }
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
}

#[test]
fn ac_m2_02_5_and_02_6_update_backup_validates_writes_config_and_regenerates_excludes() {
    run_isolated_test(
        "ac_m2_02_5_and_02_6_update_backup_validates_writes_config_and_regenerates_excludes",
        "KESTIC_M2_S02_EXCLUDES",
        |root| {
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let daemon = spawn_daemon_on_bus(root, root, KESTIC_BUS_NAME, restic_bin, None, true);
            let client = &daemon.client;

            let cfg_path = root.join("cfg").join("kestic").join("config.toml");
            let state_dir = root.join("state").join("kestic");
            let repo_dir = root.join("repo-excl");
            let src_dir = root.join("home").join("docs");
            let home_sub_excl = root.join("home").join("Downloads");
            let outside_excl = root.join("external-cache");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            fs::create_dir_all(&home_sub_excl).expect("mkdir home downloads");
            fs::create_dir_all(&outside_excl).expect("mkdir outside");

            let dest = Destination::new(
                "Local Excludes Repo",
                DestinationKind::Local {
                    path: repo_dir.clone(),
                },
            )
            .expect("dest");
            client
                .add_destination(&dest, "init", "excl-pw-123", "")
                .expect("AddDestination");

            let mut backup = Backup::new("Docs", Source::folder(&src_dir), dest.id.clone())
                .expect("valid backup");
            backup.excludes = ExcludeRules::from_text("*.log");
            let backup_id_str = client.add_backup(&backup).expect("AddBackup");
            let backup_id = kestic_core::model::BackupId::parse(&backup_id_str).expect("backup id");
            let excl_file = exclude_file_path(&state_dir, &backup_id);

            assert_eq!(
                fs::read_to_string(&excl_file).expect("initial exclude file"),
                "*.log\n"
            );

            // AC-M2-02-5: Valid UpdateBackup writes config.toml and regenerates `<state>/excludes/<backup-id>.txt`.
            let mut updated = backup.clone();
            updated.excludes = ExcludeRules {
                folders: vec![
                    home_sub_excl.to_string_lossy().into_owned(),
                    outside_excl.to_string_lossy().into_owned(),
                ],
                lines: vec!["*.tmp".to_string(), "**/build/**".to_string()],
                exclude_caches: true,
                exclude_trash: true,
                exclude_larger_than: Some("250M".to_string()),
                ignore_case: false,
                exclude_if_present: vec![".nobackup".to_string()],
            };
            client
                .update_backup(&updated)
                .expect("UpdateBackup valid excludes");

            let persisted = load_config(&cfg_path).expect("reload config.toml");
            assert_eq!(persisted.backups.len(), 1);
            assert_eq!(
                persisted.backups[0].excludes.folders,
                vec![
                    "$HOME/Downloads".to_string(),
                    outside_excl.to_string_lossy().into_owned(),
                ]
            );
            assert_eq!(
                persisted.backups[0].excludes.lines,
                vec!["*.tmp".to_string(), "**/build/**".to_string()]
            );
            assert!(persisted.backups[0].excludes.exclude_caches);
            assert_eq!(
                persisted.backups[0].excludes.exclude_larger_than.as_deref(),
                Some("250M")
            );
            assert_eq!(
                persisted.backups[0].excludes.exclude_if_present,
                vec![".nobackup".to_string()]
            );

            let regenerated = fs::read_to_string(&excl_file).expect("regenerated exclude file");
            assert!(regenerated.contains("$HOME/Downloads\n"), "{regenerated}");
            assert!(
                regenerated.contains(&format!("{}\n", outside_excl.display())),
                "{regenerated}"
            );
            assert!(
                regenerated.contains("$HOME/.local/share/Trash\n"),
                "{regenerated}"
            );
            assert!(regenerated.contains("**/.Trash-*\n"), "{regenerated}");
            assert!(regenerated.contains("*.tmp\n"), "{regenerated}");
            assert!(regenerated.contains("**/build/**\n"), "{regenerated}");
            assert!(!regenerated.contains("*.log\n"), "{regenerated}");

            // Verify TestExcludePath & LintExcludes D-Bus helpers.
            let test_res = client
                .test_exclude_path(&backup_id_str, &updated.excludes, "proj/build/out.o")
                .expect("TestExcludePath");
            assert!(!test_res.included, "build artifact must be excluded");

            let shadowed = ExcludeRules::from_text("dir\n!dir/inner");
            let lints = client.lint_excludes(&shadowed).expect("LintExcludes");
            assert_eq!(lints.len(), 1);
            assert_eq!(lints[0].suggested_fix, "dir/*");

            // AC-M2-02-6: Invalid UpdateBackup rejected by core, config.toml & exclude file untouched.
            let cfg_before = fs::read(&cfg_path).expect("read config bytes");
            let excl_before = fs::read(&excl_file).expect("read exclude bytes");

            let mut invalid_size = updated.clone();
            invalid_size.excludes.exclude_larger_than = Some("0M".to_string());
            let err1 = client
                .update_backup(&invalid_size)
                .expect_err("zero exclude_larger_than must fail");
            assert_dbus_error_name(&err1, "net.daester.Kestic.Error.InvalidInput");
            assert_eq!(fs::read(&cfg_path).expect("cfg unchanged"), cfg_before);
            assert_eq!(fs::read(&excl_file).expect("excl unchanged"), excl_before);

            let mut invalid_marker = updated.clone();
            invalid_marker.excludes.exclude_if_present = vec!["   ".to_string()];
            let err2 = client
                .update_backup(&invalid_marker)
                .expect_err("blank exclude_if_present must fail");
            assert_dbus_error_name(&err2, "net.daester.Kestic.Error.InvalidInput");
            assert_eq!(fs::read(&cfg_path).expect("cfg unchanged"), cfg_before);
            assert_eq!(fs::read(&excl_file).expect("excl unchanged"), excl_before);

            let mut invalid_newline_folder = updated;
            invalid_newline_folder.excludes.folders = vec!["bad\nfolder".to_string()];
            let err3 = client
                .update_backup(&invalid_newline_folder)
                .expect_err("newline in folder rule must fail");
            assert_dbus_error_name(&err3, "net.daester.Kestic.Error.InvalidInput");
            assert_eq!(fs::read(&cfg_path).expect("cfg unchanged"), cfg_before);
            assert_eq!(fs::read(&excl_file).expect("excl unchanged"), excl_before);

            daemon.stop();
        },
    );
}

fn verify_dry_run_with_excludes(client: &KesticClient, backup_id: &str, label: &str) {
    // AC-M2-04-1: StartBackupDryRun against real repo with excludes -> matches dry-run, 0 snapshots.
    let preview_id = client
        .start_backup_dry_run(backup_id, None)
        .expect("StartBackupDryRun");
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        let st = client
            .get_backup_dry_run_status(&preview_id)
            .expect("GetBackupDryRunStatus");
        if st.state != "running" {
            break st;
        }
        assert!(
            Instant::now() < deadline,
            "[{label}] timed out waiting for dry-run"
        );
        thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(status.state, "done", "[{label}] dry-run failed: {status:?}");
    let res = status.result.expect("dry-run result");
    assert!(!res.cancelled, "[{label}]");
    assert_eq!(res.total_files, 2, "[{label}] res={res:?}");
    assert!(res.total_bytes >= 22, "[{label}] res={res:?}");
    assert!(
        res.files.iter().any(|f| f.ends_with("keep.txt")),
        "[{label}] files={:?}",
        res.files
    );
    assert!(
        res.files.iter().any(|f| f.ends_with("doc.txt")),
        "[{label}] files={:?}",
        res.files
    );
    assert!(
        !res.files.iter().any(|f| f.ends_with("skip.tmp")),
        "[{label}] excluded file leaked into dry-run: {:?}",
        res.files
    );

    // Verify draft exclude override works without mutating config.toml.
    let draft_excludes = ExcludeRules::from_text("*.tmp\nsub");
    let draft_preview_id = client
        .start_backup_dry_run(backup_id, Some(&draft_excludes))
        .expect("StartBackupDryRun with draft excludes");
    let deadline = Instant::now() + Duration::from_secs(20);
    let draft_status = loop {
        let st = client
            .get_backup_dry_run_status(&draft_preview_id)
            .expect("GetBackupDryRunStatus draft");
        if st.state != "running" {
            break st;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(draft_status.state, "done", "[{label}] {draft_status:?}");
    let draft_res = draft_status.result.expect("draft dry-run result");
    assert_eq!(draft_res.total_files, 1, "[{label}] {draft_res:?}");

    // No snapshot was created by dry-runs.
    let snaps = client
        .list_backup_snapshots(backup_id)
        .expect("ListBackupSnapshots");
    assert!(
        snaps.current_snapshots.is_empty(),
        "[{label}] dry-run must create 0 snapshots: {:?}",
        snaps.current_snapshots
    );
}

fn verify_stopped_rest_server_preflight(
    sub: &Path,
    restic_bin: &Path,
    client: &KesticClient,
    server: &mut RestServerGuard,
    backup_id: &str,
    label: &str,
) {
    // AC-M2-04-3: Stop rest-server, install restic spy -> preflight fails <= 15s, unreachable, 0 restic.
    let (spy_script, spy_marker) = install_marker_spy(sub, "restic-dryrun-spy", restic_bin);
    let prov = client
        .set_restic_provider(&ResticProvider::Custom { path: spy_script })
        .expect("install restic spy");
    assert!(prov.valid);

    server.stop();
    let start = Instant::now();
    let fail_preview_id = client
        .start_backup_dry_run(backup_id, None)
        .expect("StartBackupDryRun against stopped server");
    let deadline = Instant::now() + Duration::from_secs(15);
    let fail_status = loop {
        let st = client
            .get_backup_dry_run_status(&fail_preview_id)
            .expect("GetBackupDryRunStatus stopped");
        if st.state != "running" {
            break st;
        }
        assert!(
            Instant::now() < deadline,
            "[{label}] stopped rest-server preflight exceeded 15s"
        );
        thread::sleep(Duration::from_millis(25));
    };
    let elapsed = start.elapsed();
    assert!(
        elapsed <= Duration::from_secs(15),
        "[{label}] preflight took {elapsed:?} (> 15s)"
    );
    assert_eq!(
        fail_status.state, "error",
        "[{label}] expected error on stopped server: {fail_status:?}"
    );
    assert_eq!(
        fail_status.error_kind.map(|k| k.as_str()),
        Some("destination_unreachable"),
        "[{label}] unexpected error: {fail_status:?}"
    );
    assert!(
        !spy_marker.exists(),
        "[{label}] zero restic process must be spawned on unreachable preflight"
    );
}

fn run_dry_run_and_stopped_server_case(root: &Path, label: &str, restic_bin: &Path) {
    let sub = root.join(label);
    let rest_data = sub.join("rest-data");
    let src_dir = sub.join("src-dry");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::create_dir_all(src_dir.join("sub")).expect("mkdir sub");
    fs::write(src_dir.join("keep.txt"), b"alpha 12345\n").expect("write keep");
    fs::write(src_dir.join("sub").join("doc.txt"), b"beta 67890\n").expect("write doc");
    fs::write(src_dir.join("skip.tmp"), b"should be excluded\n").expect("write tmp");

    let mut server = RestServerGuard::start_with_auth(&rest_data);
    let bus_name = format!(
        "net.daester.Kestic.M2DryRun_{}",
        label.replace(['.', '-'], "_")
    );
    let daemon = spawn_daemon_on_bus(root, &sub, &bus_name, restic_bin.to_path_buf(), None, true);
    let client = &daemon.client;

    let dest = Destination::new(
        format!("REST DryRun ({label})"),
        DestinationKind::Rest {
            url: server.url.clone(),
            username: Some("alice".to_string()),
        },
    )
    .expect("dest");
    client
        .add_destination(&dest, "init", "dry-repo-pw", "secret")
        .expect("AddDestination REST");

    let mut backup = Backup::new(
        format!("DryRun Docs ({label})"),
        Source::folder(&src_dir),
        dest.id.clone(),
    )
    .expect("backup");
    backup.excludes = ExcludeRules::from_text("*.tmp");
    let backup_id = client.add_backup(&backup).expect("AddBackup");

    verify_dry_run_with_excludes(client, &backup_id, label);
    verify_stopped_rest_server_preflight(&sub, restic_bin, client, &mut server, &backup_id, label);

    daemon.stop();
}

#[test]
fn ac_m2_04_1_and_04_3_dry_run_over_dbus_and_stopped_rest_server_preflight_dual_restic() {
    run_isolated_test(
        "ac_m2_04_1_and_04_3_dry_run_over_dbus_and_stopped_rest_server_preflight_dual_restic",
        "KESTIC_M2_S04_DRYRUN",
        |root| {
            for (label, restic_bin) in restic_test_binaries() {
                run_dry_run_and_stopped_server_case(root, label, &restic_bin);
            }
        },
    );
}

fn verify_retention_preview_under_lock(
    sub: &Path,
    restic_bin: &Path,
    client: &KesticClient,
    backup_id: &str,
    label: &str,
) {
    let repo_dir = sub.join("repo");
    // Install spy wrapper logging argv for non-`version` restic invocations to verify `--no-lock`.
    let (spy_script, spy_log) = install_args_spy(sub, "restic-args-spy", restic_bin);
    client
        .set_restic_provider(&ResticProvider::Custom { path: spy_script })
        .expect("install spy");

    // AC-M2-06-6: Hold a non-exclusive lock via `restic backup --stdin`.
    let lock_holder = NonExclusiveLockHolder::start(restic_bin, &repo_dir, "ret-pw-123");

    let candidate_policy = RetentionPolicy {
        keep_last: Some(KeepCount::Count(1)),
        ..RetentionPolicy::empty()
    };
    let preview_id = client
        .start_retention_preview(backup_id, &candidate_policy)
        .expect("StartRetentionPreview");
    let deadline = Instant::now() + Duration::from_secs(15);
    let dto = loop {
        let st = client
            .get_retention_preview_status(&preview_id)
            .expect("GetRetentionPreviewStatus");
        if st.state != "running" {
            break st;
        }
        assert!(
            Instant::now() < deadline,
            "[{label}] retention preview timed out"
        );
        thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(dto.state, "done", "[{label}] {dto:?}");
    let preview = dto.result.expect("retention preview result");
    assert_eq!(preview.keep_count, 1, "[{label}] {preview:?}");
    assert_eq!(preview.remove_count, 1, "[{label}] {preview:?}");
    let kept = preview
        .snapshots
        .iter()
        .find(|s| s.action == "keep")
        .expect("kept snapshot");
    assert!(
        !kept.reasons.is_empty(),
        "[{label}] kept snapshot must carry reasons: {kept:?}"
    );
    assert!(
        preview.snapshots.iter().any(|s| s.action == "remove"),
        "[{label}] expected one remove entry: {preview:?}"
    );

    let logged = fs::read_to_string(&spy_log).expect("read spy log");
    assert!(
        logged.lines().any(|line| line.contains("forget")
            && line.contains("--dry-run")
            && line.contains("--no-lock")),
        "[{label}] the retention preview must pass --dry-run and --no-lock:\n{logged}"
    );

    // Dry-run preview must not delete snapshots.
    let snaps = client
        .list_backup_snapshots(backup_id)
        .expect("ListBackupSnapshots");
    assert_eq!(snaps.current_snapshots.len(), 2, "[{label}]");

    lock_holder.release(&repo_dir);
}

fn verify_retention_update_validation(
    cfg_path: &Path,
    client: &KesticClient,
    bus_name: &str,
    backup: &Backup,
) {
    // AC-M2-06-7: Invalid retention update rejected by core, config.toml untouched; valid updates config.toml.
    let cfg_before = fs::read(cfg_path).expect("read config bytes");

    let mut invalid_empty = backup.clone();
    invalid_empty.retention = RetentionPolicy::empty();
    let err_empty = update_backup_explicit(client, bus_name, &invalid_empty)
        .expect_err("empty retention must fail");
    assert_dbus_error_name(&err_empty, "net.daester.Kestic.Error.InvalidInput");
    assert_eq!(fs::read(cfg_path).expect("cfg unchanged"), cfg_before);

    let mut invalid_dur = backup.clone();
    invalid_dur.retention = RetentionPolicy {
        keep_within: Some("30x".to_string()),
        ..RetentionPolicy::empty()
    };
    let err_dur = update_backup_explicit(client, bus_name, &invalid_dur)
        .expect_err("invalid duration 30x must fail");
    assert_dbus_error_name(&err_dur, "net.daester.Kestic.Error.InvalidInput");
    assert_eq!(fs::read(cfg_path).expect("cfg unchanged"), cfg_before);

    let mut valid_upd = backup.clone();
    valid_upd.retention = RetentionPolicy::long();
    update_backup_explicit(client, bus_name, &valid_upd).expect("UpdateBackup valid retention");
    let reloaded = load_config(cfg_path).expect("reload config.toml");
    assert_eq!(reloaded.backups[0].retention, RetentionPolicy::long());
    let state = client.get_state().expect("GetState");
    assert_eq!(state.backups[0].backup.retention, RetentionPolicy::long());
}

fn run_retention_preview_and_update_case(root: &Path, label: &str, restic_bin: &Path) {
    let sub = root.join(label);
    let repo_dir = sub.join("repo");
    let src_dir = sub.join("src");
    let cfg_path = sub.join("cfg").join("kestic").join("config.toml");
    fs::create_dir_all(&repo_dir).expect("mkdir repo");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("file1.txt"), b"snapshot 1\n").expect("write file1");

    let bus_name = format!(
        "net.daester.Kestic.M2Retention_{}",
        label.replace(['.', '-'], "_")
    );
    let daemon = spawn_daemon_on_bus(root, &sub, &bus_name, restic_bin.to_path_buf(), None, true);
    let client = &daemon.client;
    let signals = KesticSignalCollector::start(&bus_name, KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1);

    let dest = Destination::new(
        format!("Retention Repo ({label})"),
        DestinationKind::Local { path: repo_dir },
    )
    .expect("dest");
    client
        .add_destination(&dest, "init", "ret-pw-123", "")
        .expect("AddDestination");

    let mut backup = Backup::new(
        format!("Retention Docs ({label})"),
        Source::folder(&src_dir),
        dest.id.clone(),
    )
    .expect("backup");
    backup.retention = RetentionPolicy::standard();
    let backup_id = client.add_backup(&backup).expect("AddBackup");

    // Produce 2 distinct snapshots so retention preview has keep + remove items.
    let run1 = client.start_backup(&backup_id).expect("StartBackup 1");
    assert_eq!(
        wait_for_run_finished(&signals, &run1, Duration::from_secs(20)).status,
        RunStatus::Ok
    );

    fs::write(src_dir.join("file2.txt"), b"snapshot 2\n").expect("write file2");
    let run2 = client.start_backup(&backup_id).expect("StartBackup 2");
    assert_eq!(
        wait_for_run_finished(&signals, &run2, Duration::from_secs(20)).status,
        RunStatus::Ok
    );

    verify_retention_preview_under_lock(&sub, restic_bin, client, &backup_id, label);
    verify_retention_update_validation(&cfg_path, client, &bus_name, &backup);

    daemon.stop();
}

#[test]
fn ac_m2_06_6_and_06_7_preview_retention_with_non_exclusive_lock_and_update_dual_restic() {
    run_isolated_test(
        "ac_m2_06_6_and_06_7_preview_retention_with_non_exclusive_lock_and_update_dual_restic",
        "KESTIC_M2_S06_RETENTION",
        |root| {
            let _notif_guard = start_fake_notifications();
            for (label, restic_bin) in restic_test_binaries() {
                run_retention_preview_and_update_case(root, label, &restic_bin);
            }
        },
    );
}

#[test]
fn ac_m2_07_1_backup_exit_0_runs_automatic_forget_and_prunes_dual_restic() {
    run_isolated_test(
        "ac_m2_07_1_backup_exit_0_runs_automatic_forget_and_prunes_dual_restic",
        "KESTIC_M2_S07_EXIT0",
        |root| {
            let _notif_guard = start_fake_notifications();
            for (label, restic_bin) in restic_test_binaries() {
                let sub = root.join(label);
                let repo_dir = sub.join("repo");
                let src_dir = sub.join("src");
                fs::create_dir_all(&repo_dir).expect("mkdir repo");
                fs::create_dir_all(&src_dir).expect("mkdir src");

                let bus_name = format!(
                    "net.daester.Kestic.M2ForgetOk_{}",
                    label.replace(['.', '-'], "_")
                );
                let daemon = spawn_daemon_on_bus(root, &sub, &bus_name, restic_bin, None, true);
                let client = &daemon.client;
                let signals = KesticSignalCollector::start(
                    &bus_name,
                    KESTIC_OBJECT_PATH,
                    KESTIC_INTERFACE_V1,
                );

                let dest = Destination::new(
                    format!("Forget Ok Repo ({label})"),
                    DestinationKind::Local {
                        path: repo_dir.clone(),
                    },
                )
                .expect("dest");
                client
                    .add_destination(&dest, "init", "ok-pw-123", "")
                    .expect("AddDestination");

                let mut backup = Backup::new(
                    format!("Pruned Docs ({label})"),
                    Source::folder(&src_dir),
                    dest.id.clone(),
                )
                .expect("backup");
                backup.retention = RetentionPolicy {
                    keep_last: Some(KeepCount::Count(2)),
                    ..RetentionPolicy::empty()
                };
                let backup_id = add_backup_explicit(client, &bus_name, &backup).expect("AddBackup");

                fs::write(src_dir.join("a.txt"), b"v1\n").expect("write a");
                let r1 = client.start_backup(&backup_id).expect("StartBackup 1");
                assert_eq!(
                    wait_for_run_finished(&signals, &r1, Duration::from_secs(20)).status,
                    RunStatus::Ok
                );

                fs::write(src_dir.join("b.txt"), b"v2\n").expect("write b");
                let r2 = client.start_backup(&backup_id).expect("StartBackup 2");
                assert_eq!(
                    wait_for_run_finished(&signals, &r2, Duration::from_secs(20)).status,
                    RunStatus::Ok
                );
                assert_eq!(
                    client
                        .list_backup_snapshots(&backup_id)
                        .expect("snaps 2")
                        .current_snapshots
                        .len(),
                    2
                );

                // Tightening retention to keep_last = 1 + running backup Exit 0 automatically prunes to 1.
                backup.retention = RetentionPolicy {
                    keep_last: Some(KeepCount::Count(1)),
                    ..RetentionPolicy::empty()
                };
                update_backup_explicit(client, &bus_name, &backup)
                    .expect("UpdateBackup keep_last=1");

                fs::write(src_dir.join("c.txt"), b"v3\n").expect("write c");
                let r3 = client.start_backup(&backup_id).expect("StartBackup 3");
                let finished = wait_for_run_finished(&signals, &r3, Duration::from_secs(20));
                assert_eq!(finished.status, RunStatus::Ok, "[{label}] {finished:?}");
                assert_eq!(finished.exit_code, Some(0), "[{label}]");

                let detail = client.get_run_detail(&r3).expect("GetRunDetail");
                assert_eq!(
                    detail.run.commands.len(),
                    2,
                    "[{label}] expected backup + forget commands: {:?}",
                    detail.run.commands
                );
                let forget_cmd = &detail.run.commands[1];
                assert!(
                    forget_cmd.args.contains(&"forget".to_string()),
                    "[{label}] {:?}",
                    forget_cmd.args
                );
                assert_has_flag_pair(&forget_cmd.args, "--tag", &format!("kestic:{backup_id}"));
                assert_has_flag_pair(&forget_cmd.args, "--group-by", "tags");
                assert_eq!(forget_cmd.exit_code, 0, "[{label}]");

                let snaps_after = client
                    .list_backup_snapshots(&backup_id)
                    .expect("ListBackupSnapshots after forget");
                assert_eq!(
                    snaps_after.current_snapshots.len(),
                    1,
                    "[{label}] expected snapshots pruned to 1: {:?}",
                    snaps_after.current_snapshots
                );

                let state = client.get_state().expect("GetState");
                assert!(!state.backups[0].pending_forget, "[{label}]");

                daemon.stop();
            }
        },
    );
}

/// A backup that just ended with Exit 3 because of an unreadable file.
struct Exit3Run<'a> {
    sub: &'a Path,
    client: &'a KesticClient,
    signals: &'a KesticSignalCollector,
    backup_id_str: &'a str,
    label: &'a str,
    unreadable: PathBuf,
    detail: kestic_daemon::dbus_api::RunDetailDto,
}

impl Drop for Exit3Run<'_> {
    fn drop(&mut self) {
        // Restore permissions so directory cleanup never hits permission snags.
        let _ = fs::set_permissions(&self.unreadable, fs::Permissions::from_mode(0o644));
    }
}

/// Makes a file unreadable (`chmod 000`) and runs the backup: restic exits 3.
fn run_backup_with_unreadable_file<'a>(
    sub: &'a Path,
    client: &'a KesticClient,
    signals: &'a KesticSignalCollector,
    backup_id_str: &'a str,
    label: &'a str,
) -> Exit3Run<'a> {
    let unreadable = sub.join("src").join("secret_unreadable.txt");
    fs::write(&unreadable, b"cannot read this\n").expect("write unreadable");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmod 000");

    let r2 = client.start_backup(backup_id_str).expect("StartBackup 2");
    let fin2 = wait_for_run_finished(signals, &r2, Duration::from_secs(20));
    assert_eq!(fin2.status, RunStatus::Warning, "[{label}] {fin2:?}");
    assert_eq!(fin2.exit_code, Some(3), "[{label}]");
    let detail = client.get_run_detail(&r2).expect("GetRunDetail 2");
    Exit3Run {
        sub,
        client,
        signals,
        backup_id_str,
        label,
        unreadable,
        detail,
    }
}

/// AC-M2-07-2: after Exit 3 `forget` still runs and prunes down to `keep_last = 1`.
fn verify_forget_runs_after_exit_3(run: &Exit3Run<'_>) {
    let label = run.label;
    let detail2 = &run.detail;
    assert_eq!(detail2.run.status, RunStatus::Warning, "[{label}]");
    assert_eq!(detail2.run.exit_code, Some(3), "[{label}]");
    assert_eq!(
        detail2.run.commands.len(),
        2,
        "[{label}] forget must still run after Exit 3: {:?}",
        detail2.run.commands
    );
    assert!(
        detail2.run.commands[1].args.contains(&"forget".to_string()),
        "[{label}]"
    );
    assert_eq!(detail2.run.commands[1].exit_code, 0, "[{label}]");
    assert_eq!(
        run.client
            .list_backup_snapshots(run.backup_id_str)
            .expect("snaps after exit 3")
            .current_snapshots
            .len(),
        1,
        "[{label}]"
    );
}

/// AC-M2-08-1: the run detail and the state list exactly the unreadable path and offer
/// «add to exclude rules». AC-M2-08-2: adding the rule excludes the path, the next run is `ok`.
fn verify_unreadable_files_are_listed_and_excludable(run: &Exit3Run<'_>) {
    let label = run.label;
    let client = run.client;
    let detail2 = &run.detail;
    let state_dir = run.sub.join("state").join("kestic");
    let cfg_path = run.sub.join("cfg").join("kestic").join("config.toml");
    let backup_id = kestic_core::model::BackupId::parse(run.backup_id_str).expect("backup id");

    assert!(
        detail2
            .unreadable_files
            .iter()
            .any(|p| p.ends_with("secret_unreadable.txt")),
        "[{label}] expected unreadable path in {:?}",
        detail2.unreadable_files
    );
    assert_eq!(
        detail2.error_action.map(|c| c.as_str()),
        Some("add_to_excludes"),
        "[{label}]"
    );
    let state2 = client.get_state().expect("GetState after exit 3");
    assert!(
        state2.backups[0]
            .unreadable_files
            .iter()
            .any(|p| p.ends_with("secret_unreadable.txt")),
        "[{label}] {:?}",
        state2.backups[0].unreadable_files
    );

    let unreadable_item = detail2.unreadable_files[0].clone();
    let preview = client
        .add_exclude_rule_for_item(run.backup_id_str, &unreadable_item)
        .expect("AddExcludeRuleForItem");
    assert!(!preview.is_dir, "[{label}]");
    assert_eq!(preview.rule, unreadable_item, "[{label}]");

    let reloaded = load_config(&cfg_path).expect("reload config");
    assert!(
        reloaded.backups[0]
            .excludes
            .lines
            .contains(&unreadable_item),
        "[{label}]"
    );
    let excl_content =
        fs::read_to_string(exclude_file_path(&state_dir, &backup_id)).expect("read exclude file");
    assert!(
        excl_content.contains(&unreadable_item),
        "[{label}] {excl_content}"
    );

    let r3 = client
        .start_backup(run.backup_id_str)
        .expect("StartBackup 3");
    let fin3 = wait_for_run_finished(run.signals, &r3, Duration::from_secs(20));
    assert_eq!(fin3.status, RunStatus::Ok, "[{label}] {fin3:?}");
    assert_eq!(fin3.exit_code, Some(0), "[{label}]");
    let detail3 = client.get_run_detail(&r3).expect("GetRunDetail 3");
    assert!(
        detail3.unreadable_files.is_empty(),
        "[{label}] {:?}",
        detail3.unreadable_files
    );
}

fn run_exit_3_case(
    root: &Path,
    label: &str,
    restic_bin: &Path,
    verify: impl FnOnce(&Exit3Run<'_>),
) {
    let sub = root.join(label);
    let repo_dir = sub.join("repo");
    let src_dir = sub.join("src");
    fs::create_dir_all(&repo_dir).expect("mkdir repo");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("readable.txt"), b"readable content\n").expect("write readable");

    let bus_name = format!(
        "net.daester.Kestic.M2Exit3_{}",
        label.replace(['.', '-'], "_")
    );
    let daemon = spawn_daemon_on_bus(root, &sub, &bus_name, restic_bin.to_path_buf(), None, true);
    let client = &daemon.client;
    let signals = KesticSignalCollector::start(&bus_name, KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1);

    let dest = Destination::new(
        format!("Exit3 Repo ({label})"),
        DestinationKind::Local { path: repo_dir },
    )
    .expect("dest");
    client
        .add_destination(&dest, "init", "exit3-pw-123", "")
        .expect("AddDestination");

    let mut backup = Backup::new(
        format!("Exit3 Docs ({label})"),
        Source::folder(&src_dir),
        dest.id.clone(),
    )
    .expect("backup");
    backup.retention = RetentionPolicy {
        keep_last: Some(KeepCount::Count(1)),
        ..RetentionPolicy::empty()
    };
    let backup_id_str = add_backup_explicit(client, &bus_name, &backup).expect("AddBackup");

    // Run 1 produces 1 clean snapshot.
    let r1 = client.start_backup(&backup_id_str).expect("StartBackup 1");
    assert_eq!(
        wait_for_run_finished(&signals, &r1, Duration::from_secs(20)).status,
        RunStatus::Ok
    );

    verify(&run_backup_with_unreadable_file(
        &sub,
        client,
        &signals,
        &backup_id_str,
        label,
    ));

    daemon.stop();
}

#[test]
fn ac_m2_07_2_exit_3_still_runs_forget_dual_restic() {
    if is_running_as_root() {
        eprintln!("skipping chmod 000 Exit 3 test because running as root (euid == 0)");
        return;
    }

    run_isolated_test(
        "ac_m2_07_2_exit_3_still_runs_forget_dual_restic",
        "KESTIC_M2_S07_EXIT3",
        |root| {
            let _notif_guard = start_fake_notifications();
            for (label, restic_bin) in restic_test_binaries() {
                run_exit_3_case(root, label, &restic_bin, verify_forget_runs_after_exit_3);
            }
        },
    );
}

#[test]
fn ac_m2_08_1_08_2_exit_3_lists_unreadable_files_and_add_to_excludes_dual_restic() {
    if is_running_as_root() {
        eprintln!("skipping chmod 000 Exit 3 test because running as root (euid == 0)");
        return;
    }

    run_isolated_test(
        "ac_m2_08_1_08_2_exit_3_lists_unreadable_files_and_add_to_excludes_dual_restic",
        "KESTIC_M2_S08_EXIT3",
        |root| {
            let _notif_guard = start_fake_notifications();
            for (label, restic_bin) in restic_test_binaries() {
                run_exit_3_case(
                    root,
                    label,
                    &restic_bin,
                    verify_unreadable_files_are_listed_and_excludable,
                );
            }
        },
    );
}

struct Lock11Ctx<'a> {
    sub: &'a Path,
    restic_bin: &'a Path,
    client: &'a KesticClient,
    signals: &'a KesticSignalCollector,
    dest_id: &'a str,
    backup_id_str: &'a str,
    label: &'a str,
}

fn verify_exit_11_under_non_exclusive_lock(ctx: &Lock11Ctx<'_>) {
    let repo_dir = ctx.sub.join("repo");
    let src_dir = ctx.sub.join("src");
    let state_dir = ctx.sub.join("state").join("kestic");
    let backup_id = kestic_core::model::BackupId::parse(ctx.backup_id_str).expect("backup id");
    let label = ctx.label;

    // AC-M2-07-3: Start non-exclusive `restic backup --stdin` lock holder, then run Kestic backup.
    fs::write(src_dir.join("two.txt"), b"second\n").expect("write two");
    let lock_holder = NonExclusiveLockHolder::start(ctx.restic_bin, &repo_dir, "lock11-pw-123");

    let r2 = ctx
        .client
        .start_backup(ctx.backup_id_str)
        .expect("StartBackup 2");
    let fin2 = wait_for_run_finished(ctx.signals, &r2, Duration::from_secs(20));
    assert_eq!(fin2.status, RunStatus::Warning, "[{label}] {fin2:?}");
    assert_eq!(fin2.exit_code, Some(0), "[{label}] backup exited 0");

    let detail2 = ctx.client.get_run_detail(&r2).expect("GetRunDetail 2");
    assert_eq!(detail2.run.commands.len(), 2, "[{label}]");
    assert_eq!(detail2.run.commands[0].exit_code, 0, "[{label}]");
    assert_eq!(
        detail2.run.commands[1].exit_code, 11,
        "[{label}] forget must hit Exit 11 under non-exclusive lock: {:?}",
        detail2.run.commands[1]
    );
    let lock_summary = fin2.summary_message.as_deref().unwrap_or("");
    assert!(
        lock_summary.contains("locked"),
        "[{label}] expected locked summary_message on Exit 11 forget, got: {:?}",
        fin2.summary_message
    );

    let state2 = ctx.client.get_state().expect("GetState pending_forget");
    assert!(
        state2.backups[0].pending_forget,
        "[{label}] pending_forget must be true"
    );
    let hist = RunHistory::open(&state_dir).expect("open RunHistory");
    assert!(
        hist.is_pending_forget(&backup_id)
            .expect("check persisted pending_forget"),
        "[{label}] pending_forget must be persisted in history.db"
    );
    drop(hist);

    // 2 snapshots exist because forget failed with Exit 11.
    assert_eq!(
        ctx.client
            .list_backup_snapshots(ctx.backup_id_str)
            .expect("snaps before maint")
            .current_snapshots
            .len(),
        2,
        "[{label}]"
    );

    lock_holder.release(&repo_dir);
}

fn verify_exit_11_catchup_paths(ctx: &Lock11Ctx<'_>) {
    let repo_dir = ctx.sub.join("repo");
    let src_dir = ctx.sub.join("src");
    let label = ctx.label;

    // AC-M2-07-4, catch-up path 1: RunDestinationMaintenance executes catch-up forget & clears pending_forget.
    let maint_id = ctx
        .client
        .run_destination_maintenance(ctx.dest_id)
        .expect("RunDestinationMaintenance");
    let maint_fin = wait_for_run_finished(ctx.signals, &maint_id, Duration::from_secs(20));
    assert_eq!(maint_fin.status, RunStatus::Ok, "[{label}] {maint_fin:?}");
    assert!(
        matches!(maint_fin.kind, RunKind::Maintenance),
        "[{label}] {:?}",
        maint_fin.kind
    );

    let state_after_maint = ctx.client.get_state().expect("GetState after maint");
    assert!(
        !state_after_maint.backups[0].pending_forget,
        "[{label}] pending_forget must clear after maintenance"
    );
    assert_eq!(
        ctx.client
            .list_backup_snapshots(ctx.backup_id_str)
            .expect("snaps after maint")
            .current_snapshots
            .len(),
        1,
        "[{label}] maintenance forget must prune back to keep_last=1"
    );

    // AC-M2-07-4, catch-up path 2: trigger pending_forget again, release lock, then next backup clears it.
    fs::write(src_dir.join("three.txt"), b"third\n").expect("write three");
    let lock_holder2 = NonExclusiveLockHolder::start(ctx.restic_bin, &repo_dir, "lock11-pw-123");
    let r3 = ctx
        .client
        .start_backup(ctx.backup_id_str)
        .expect("StartBackup 3");
    let fin3 = wait_for_run_finished(ctx.signals, &r3, Duration::from_secs(20));
    assert_eq!(fin3.status, RunStatus::Warning, "[{label}]");
    assert!(
        ctx.client.get_state().expect("GetState").backups[0].pending_forget,
        "[{label}]"
    );
    lock_holder2.release(&repo_dir);

    fs::write(src_dir.join("four.txt"), b"fourth\n").expect("write four");
    let r4 = ctx
        .client
        .start_backup(ctx.backup_id_str)
        .expect("StartBackup 4");
    let fin4 = wait_for_run_finished(ctx.signals, &r4, Duration::from_secs(20));
    assert_eq!(fin4.status, RunStatus::Ok, "[{label}] {fin4:?}");
    assert!(
        !ctx.client
            .get_state()
            .expect("GetState after next backup")
            .backups[0]
            .pending_forget,
        "[{label}] next backup must clear pending_forget"
    );
    assert_eq!(
        ctx.client
            .list_backup_snapshots(ctx.backup_id_str)
            .expect("snaps after r4")
            .current_snapshots
            .len(),
        1,
        "[{label}]"
    );
}

fn run_lock11_case(root: &Path, label: &str, restic_bin: &Path) {
    let sub = root.join(label);
    let repo_dir = sub.join("repo");
    let src_dir = sub.join("src");
    fs::create_dir_all(&repo_dir).expect("mkdir repo");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("one.txt"), b"first\n").expect("write one");

    let bus_name = format!(
        "net.daester.Kestic.M2Lock11_{}",
        label.replace(['.', '-'], "_")
    );
    let daemon = spawn_daemon_on_bus(root, &sub, &bus_name, restic_bin.to_path_buf(), None, true);
    let client = &daemon.client;
    let signals = KesticSignalCollector::start(&bus_name, KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1);

    let dest = Destination::new(
        format!("Lock11 Repo ({label})"),
        DestinationKind::Local { path: repo_dir },
    )
    .expect("dest");
    let dest_id = client
        .add_destination(&dest, "init", "lock11-pw-123", "")
        .expect("AddDestination");

    let mut backup = Backup::new(
        format!("Lock11 Docs ({label})"),
        Source::folder(&src_dir),
        dest.id.clone(),
    )
    .expect("backup");
    backup.retention = RetentionPolicy {
        keep_last: Some(KeepCount::Count(1)),
        ..RetentionPolicy::empty()
    };
    let backup_id_str = add_backup_explicit(client, &bus_name, &backup).expect("AddBackup");

    // Initial backup -> 1 snapshot, Ok.
    let r1 = client.start_backup(&backup_id_str).expect("StartBackup 1");
    assert_eq!(
        wait_for_run_finished(&signals, &r1, Duration::from_secs(20)).status,
        RunStatus::Ok
    );

    let ctx = Lock11Ctx {
        sub: &sub,
        restic_bin,
        client,
        signals: &signals,
        dest_id: &dest_id,
        backup_id_str: &backup_id_str,
        label,
    };
    verify_exit_11_under_non_exclusive_lock(&ctx);
    verify_exit_11_catchup_paths(&ctx);

    daemon.stop();
}

#[test]
fn ac_m2_07_3_and_07_4_non_exclusive_lock_forget_exit_11_pending_forget_and_catchup_dual_restic() {
    run_isolated_test(
        "ac_m2_07_3_and_07_4_non_exclusive_lock_forget_exit_11_pending_forget_and_catchup_dual_restic",
        "KESTIC_M2_S07_LOCK11",
        |root| {
            let _notif_guard = start_fake_notifications();
            for (label, restic_bin) in restic_test_binaries() {
                run_lock11_case(root, label, &restic_bin);
            }
        },
    );
}

#[test]
fn ac_m2_07_5_backup_exit_1_does_not_run_forget_and_ends_error_dual_restic() {
    run_isolated_test(
        "ac_m2_07_5_backup_exit_1_does_not_run_forget_and_ends_error_dual_restic",
        "KESTIC_M2_S07_EXIT1",
        |root| {
            let _notif_guard = start_fake_notifications();
            for (label, restic_bin) in restic_test_binaries() {
                let sub = root.join(label);
                let repo_dir = sub.join("repo");
                let src_dir = sub.join("src");
                fs::create_dir_all(&repo_dir).expect("mkdir repo");
                fs::create_dir_all(&src_dir).expect("mkdir src");
                fs::write(src_dir.join("data.txt"), b"hello\n").expect("write data");

                let bus_name = format!(
                    "net.daester.Kestic.M2Exit1_{}",
                    label.replace(['.', '-'], "_")
                );
                let wrapper = ResticWrapper::create(&sub.join("wrapper"), &restic_bin);
                let daemon =
                    spawn_daemon_on_bus(root, &sub, &bus_name, wrapper.path.clone(), None, true);
                let client = &daemon.client;
                let signals = KesticSignalCollector::start(
                    &bus_name,
                    KESTIC_OBJECT_PATH,
                    KESTIC_INTERFACE_V1,
                );

                let dest = Destination::new(
                    format!("Exit1 Repo ({label})"),
                    DestinationKind::Local {
                        path: repo_dir.clone(),
                    },
                )
                .expect("dest");
                client
                    .add_destination(&dest, "init", "exit1-pw-123", "")
                    .expect("AddDestination");

                let mut backup = Backup::new(
                    format!("Exit1 Docs ({label})"),
                    Source::folder(&src_dir),
                    dest.id.clone(),
                )
                .expect("backup");
                backup.retention = RetentionPolicy {
                    keep_last: Some(KeepCount::Count(1)),
                    ..RetentionPolicy::empty()
                };
                let backup_id = client.add_backup(&backup).expect("AddBackup");

                wrapper.fail(
                    "backup",
                    1,
                    r#"{"message_type":"exit_error","code":1,"message":"simulated fatal backup failure"}"#,
                );

                let r1 = client.start_backup(&backup_id).expect("StartBackup");
                let fin = wait_for_run_finished(&signals, &r1, Duration::from_secs(20));
                assert_eq!(fin.status, RunStatus::Error, "[{label}] {fin:?}");
                assert_eq!(fin.exit_code, Some(1), "[{label}]");

                let detail = client.get_run_detail(&r1).expect("GetRunDetail");
                assert_eq!(detail.run.status, RunStatus::Error, "[{label}]");
                assert_eq!(
                    detail.run.commands.len(),
                    1,
                    "[{label}] forget must NOT run on backup Exit 1: {:?}",
                    detail.run.commands
                );
                assert!(
                    detail.run.commands[0].args.contains(&"backup".to_string()),
                    "[{label}]"
                );

                // `init` belongs to the setup of the destination, not to the run.
                let subcmds: Vec<String> = wrapper
                    .entries()
                    .into_iter()
                    .filter(|e| e.kind == 'S' && e.sub != "version" && e.sub != "init")
                    .map(|e| e.sub)
                    .collect();
                assert_eq!(
                    subcmds,
                    vec!["backup"],
                    "[{label}] only backup must be invoked, never forget: {subcmds:?}"
                );

                daemon.stop();
            }
        },
    );
}

struct DriveTestCtx<'a> {
    client: &'a KesticClient,
    client_conn: &'a Connection,
    sni_svc: &'a str,
    signals: &'a KesticSignalCollector,
    notif_capture: &'a common::NotificationCapture,
    fake_inspector: &'a FakeMountInspector,
    drive_mount: &'a Path,
    root_only_mountinfo: &'a str,
    mounted_mountinfo: &'a str,
    backup_id: &'a str,
    spy_marker: &'a Path,
}

fn install_drive_restic_spy(root: &Path, real_restic: &Path, client: &KesticClient) -> PathBuf {
    let (spy_script, spy_marker) = install_marker_spy(root, "drive-restic-spy", real_restic);
    client
        .set_restic_provider(&ResticProvider::Custom { path: spy_script })
        .expect("install spy");
    spy_marker
}

fn verify_drive_available_and_unmounted_block(ctx: &DriveTestCtx<'_>) {
    // Initially mounted & non-empty -> available and tray item enabled.
    let avail_ok = ctx
        .client
        .check_availability(true)
        .expect("CheckAvailability");
    assert!(avail_ok.backups[0].source_available);
    assert!(avail_ok.backups[0].available);
    assert_eq!(avail_ok.backups[0].blocked_reason, None);
    let menu_ok = fetch_dbusmenu_items(ctx.client_conn, ctx.sni_svc);
    let item_ok = menu_ok
        .iter()
        .find(|m| m.label.starts_with("USB Photos"))
        .expect("tray item USB Photos");
    assert!(item_ok.enabled);

    // AC-M2-09-1: Unmount drive -> CheckAvailability marks unavailable, tray item disabled.
    ctx.fake_inspector.update(|st| {
        st.mountinfo = ctx.root_only_mountinfo.to_string();
    });
    let avail_unmounted = ctx
        .client
        .check_availability(true)
        .expect("CheckAvailability unmounted");
    assert!(!avail_unmounted.backups[0].source_available);
    assert!(!avail_unmounted.backups[0].available);
    assert_eq!(
        avail_unmounted.backups[0]
            .blocked_reason
            .map(|c| c.as_str()),
        Some(BlockedReason::SourceUnavailable.as_str())
    );
    assert_eq!(
        avail_unmounted.backups[0].source_status.map(|c| c.as_str()),
        Some("drive_not_mounted"),
        "{:?}",
        avail_unmounted.backups[0]
    );
    let menu_unmounted = fetch_dbusmenu_items(ctx.client_conn, ctx.sni_svc);
    let item_unmounted = menu_unmounted
        .iter()
        .find(|m| m.label.starts_with("USB Photos"))
        .expect("tray item USB Photos");
    assert!(
        !item_unmounted.enabled,
        "tray menu item must be disabled when drive is unmounted"
    );

    // AC-M2-09-1 / 09-2: StartBackup returns ManualRunBlocked(source_unavailable) & 0 restic process.
    ctx.signals.clear();
    ctx.notif_capture.clear();
    let err_unmounted = ctx
        .client
        .start_backup(ctx.backup_id)
        .expect_err("StartBackup while unmounted must fail");
    assert_dbus_error_name(&err_unmounted, "net.daester.Kestic.Error.ManualRunBlocked");
    assert!(
        !ctx.spy_marker.exists(),
        "zero restic process when unmounted"
    );
    ctx.signals.wait_until(Duration::from_secs(3), |sigs| {
        sigs.iter().any(|s| {
            matches!(
                s,
                CapturedKesticSignal::ManualRunBlocked { backup_id: b, reason }
                if b == ctx.backup_id && reason == BlockedReason::SourceUnavailable.as_str()
            )
        })
    });
}

fn verify_drive_mismatch_empty_and_race_recovery(ctx: &DriveTestCtx<'_>) {
    // Also verify DriveMismatch (different UUID mounted) & DriveEmpty block StartBackup with 0 restic.
    ctx.fake_inspector.update(|st| {
        st.mountinfo = ctx.mounted_mountinfo.to_string();
        st.device_uuids.insert(
            "/dev/sdb1".to_string(),
            ("UUID-OTHER-999".to_string(), Some("OtherUSB".to_string())),
        );
    });
    let avail_mismatch = ctx
        .client
        .check_availability(true)
        .expect("CheckAvailability mismatch");
    assert!(!avail_mismatch.backups[0].available);
    assert_eq!(
        avail_mismatch.backups[0].blocked_reason.map(|c| c.as_str()),
        Some(BlockedReason::SourceUnavailable.as_str())
    );
    let err_mismatch = ctx
        .client
        .start_backup(ctx.backup_id)
        .expect_err("StartBackup on wrong drive must fail");
    assert_dbus_error_name(&err_mismatch, "net.daester.Kestic.Error.ManualRunBlocked");
    assert!(
        !ctx.spy_marker.exists(),
        "zero restic process on wrong drive"
    );

    ctx.fake_inspector.update(|st| {
        st.device_uuids.insert(
            "/dev/sdb1".to_string(),
            ("UUID-DRIVE-001".to_string(), Some("MyUSB".to_string())),
        );
        st.empty_dirs.insert(ctx.drive_mount.to_path_buf(), true);
    });
    let avail_empty = ctx
        .client
        .check_availability(true)
        .expect("CheckAvailability empty");
    assert!(!avail_empty.backups[0].available);
    assert_eq!(
        avail_empty.backups[0].blocked_reason.map(|c| c.as_str()),
        Some(BlockedReason::SourceUnavailable.as_str())
    );
    let err_empty = ctx
        .client
        .start_backup(ctx.backup_id)
        .expect_err("StartBackup on empty drive mount must fail");
    assert_dbus_error_name(&err_empty, "net.daester.Kestic.Error.ManualRunBlocked");
    assert!(
        !ctx.spy_marker.exists(),
        "zero restic process on empty drive"
    );

    // AC-M2-09-2: Mounted at display time (cached available), then unmounted right before click ->
    // StartBackup still performs live check, returns ManualRunBlocked(source_unavailable), 0 restic.
    ctx.fake_inspector.update(|st| {
        st.mountinfo = ctx.mounted_mountinfo.to_string();
        st.empty_dirs.clear();
    });
    let avail_remounted = ctx
        .client
        .check_availability(true)
        .expect("CheckAvailability remounted");
    assert!(avail_remounted.backups[0].available);

    // Unmount silently without calling CheckAvailability.
    ctx.fake_inspector.update(|st| {
        st.mountinfo = ctx.root_only_mountinfo.to_string();
    });
    ctx.signals.clear();
    let err_race = ctx
        .client
        .start_backup(ctx.backup_id)
        .expect_err("StartBackup after unmount before click must fail");
    assert_dbus_error_name(&err_race, "net.daester.Kestic.Error.ManualRunBlocked");
    assert!(
        !ctx.spy_marker.exists(),
        "zero restic process on pre-click unmount"
    );
    ctx.signals.wait_until(Duration::from_secs(3), |sigs| {
        sigs.iter().any(|s| {
            matches!(
                s,
                CapturedKesticSignal::ManualRunBlocked { backup_id: b, reason }
                if b == ctx.backup_id && reason == BlockedReason::SourceUnavailable.as_str()
            )
        })
    });

    // Once remounted, StartBackup runs normally and completes Ok.
    ctx.fake_inspector.update(|st| {
        st.mountinfo = ctx.mounted_mountinfo.to_string();
    });
    let run_id = ctx
        .client
        .start_backup(ctx.backup_id)
        .expect("StartBackup once remounted");
    let fin = wait_for_run_finished(ctx.signals, &run_id, Duration::from_secs(20));
    assert_eq!(fin.status, RunStatus::Ok);
    assert!(
        ctx.spy_marker.exists(),
        "restic must run when drive is mounted"
    );
}

#[test]
fn ac_m2_09_1_and_09_2_drive_source_availability_and_manual_run_blocked() {
    run_isolated_test(
        "ac_m2_09_1_and_09_2_drive_source_availability_and_manual_run_blocked",
        "KESTIC_M2_S09_DRIVE",
        |root| {
            let watcher_conn = start_fake_sni_watcher();
            let (_notif_conn, notif_capture) = start_fake_notifications();
            let client_conn = block_on(Connection::session()).expect("session conn");

            let (_, real_restic) = restic_test_binaries().pop().expect("restic binary");

            let repo_dir = root.join("repo-drive");
            let drive_mount = root.join("mnt").join("usb_drive");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&drive_mount).expect("mkdir drive mount");
            fs::write(drive_mount.join("photo.jpg"), b"jpeg-data\n").expect("write drive file");

            let root_only_mountinfo = "20 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw\n";
            let mounted_mountinfo = format!(
                "{root_only_mountinfo}35 20 8:17 / {} rw,relatime shared:2 - ext4 /dev/sdb1 rw\n",
                drive_mount.display()
            );

            let mut uuids = HashMap::new();
            uuids.insert(
                "/dev/sdb1".to_string(),
                ("UUID-DRIVE-001".to_string(), Some("MyUSB".to_string())),
            );
            let fake_inspector = Arc::new(FakeMountInspector::new(FakeMountState {
                mountinfo: mounted_mountinfo.clone(),
                device_uuids: uuids,
                empty_dirs: HashMap::new(),
            }));

            let daemon = spawn_daemon_on_bus(
                root,
                root,
                KESTIC_BUS_NAME,
                real_restic.clone(),
                Some(fake_inspector.clone()),
                false,
            );
            let client = &daemon.client;
            let sni_svc = wait_for_sni_service(&watcher_conn, Duration::from_secs(5));
            let signals = KesticSignalCollector::start(
                KESTIC_BUS_NAME,
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );

            let dest = Destination::new(
                "Local Drive Repo",
                DestinationKind::Local { path: repo_dir },
            )
            .expect("dest");
            client
                .add_destination(&dest, "init", "drive-pw-123", "")
                .expect("AddDestination");

            // DetectSource recognizes mounted external filesystem as Drive with FS-UUID.
            let detected = client
                .detect_source(&drive_mount.to_string_lossy())
                .expect("DetectSource");
            assert_eq!(detected.kind, SourceKind::Drive);
            assert_eq!(
                detected.identity,
                Some(SourceIdentity::FsUuid {
                    fs_uuid: "UUID-DRIVE-001".to_string()
                })
            );
            assert_eq!(detected.drive_label.as_deref(), Some("MyUSB"));

            let backup =
                Backup::new("USB Photos", detected, dest.id.clone()).expect("drive backup");
            let backup_id = client.add_backup(&backup).expect("AddBackup");

            // Install restic spy AFTER init so any blocked StartBackup guarantees 0 restic invocations.
            let spy_marker = install_drive_restic_spy(root, &real_restic, client);
            let ctx = DriveTestCtx {
                client,
                client_conn: &client_conn,
                sni_svc: &sni_svc,
                signals: &signals,
                notif_capture: &notif_capture,
                fake_inspector: &fake_inspector,
                drive_mount: &drive_mount,
                root_only_mountinfo,
                mounted_mountinfo: &mounted_mountinfo,
                backup_id: &backup_id,
                spy_marker: &spy_marker,
            };

            verify_drive_available_and_unmounted_block(&ctx);
            verify_drive_mismatch_empty_and_race_recovery(&ctx);

            daemon.stop();
        },
    );
}
