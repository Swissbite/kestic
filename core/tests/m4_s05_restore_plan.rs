// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S05 restore without restic: the arguments of `restic restore`, the plan of a selection,
//! glob escaping, the suggested new folder and the restore run in the history.

use kestic_core::clock::FakeClock;
use kestic_core::history::RunHistory;
use kestic_core::model::{BackupId, DestinationId, RestoreStats, Run, RunKind, RunStatus};
use kestic_core::restore::{
    EntryKind, ProbeCollector, RestoreCall, escape_glob, folder_instead_of_unmatchable,
    normalize_selection, plan_restore, probe_path, restore_args, selection_with_folders_instead,
};
use kestic_core::restore_target::{
    FolderWords, SuggestionInput, new_folder_name, original_location, suggested_new_folder,
};
use kestic_core::snapshot::ResticSnapshot;
use kestic_i18n::codes::RestoreOverwrite;
use std::path::Path;
use time::macros::datetime;

const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn sel(paths: &[&str]) -> Vec<String> {
    normalize_selection(&paths.iter().map(|p| (*p).to_string()).collect::<Vec<_>>())
        .expect("valid selection")
}

/// AC-M4-05-6: every restore call (a) names `--overwrite` explicitly, (b) never relies on
/// restic's implicit default `always`, and (c) carries neither `--sparse` nor `--verify`.
#[test]
fn ac_m4_05_6_every_restore_call_names_the_overwrite_mode() {
    let plan = plan_restore(ID, &sel(&["/home/u/a.txt"]), EntryKind::Other).expect("plan");
    for mode in RestoreOverwrite::ALL {
        for call in [RestoreCall::DryRun, RestoreCall::Run] {
            let args = restore_args(&plan, Path::new("/t"), mode, call);
            let pos = args.iter().position(|a| a == "--overwrite");
            assert!(pos.is_some(), "(a) {mode:?} {call:?}: {args:?}");
            let value = pos.and_then(|i| args.get(i + 1)).map(String::as_str);
            let expected = match mode {
                RestoreOverwrite::Never => "never",
                RestoreOverwrite::IfNewer => "if-newer",
                RestoreOverwrite::IfChanged => "if-changed",
                RestoreOverwrite::Always => "always",
            };
            assert_eq!(value, Some(expected), "(b) {mode:?} {call:?}: {args:?}");
            assert!(
                !args
                    .iter()
                    .any(|a| a.starts_with("--sparse") || a.starts_with("--verify")),
                "(c) {mode:?} {call:?}: {args:?}"
            );
        }
    }
    assert_eq!(
        RestoreOverwrite::default(),
        RestoreOverwrite::Never,
        "(b) default"
    );
}

/// AC-M4-05-12: a restore of a snapshot with `kestic:b1` (a) is a run of the kind `Restore`,
/// (b) belongs to the destination, (c) carries the backup `b1` and (d) appears in the history
/// of `b1` and of the destination.
#[test]
fn ac_m4_05_12_restore_run_belongs_to_destination_and_backup() {
    let clock = FakeClock::new(datetime!(2026-11-15 12:00 UTC));
    let dest = DestinationId::generate().expect("destination id");
    let b1 = BackupId::generate().expect("backup id");
    let mut run = Run::new_restore(&clock, dest.clone(), Some(b1.clone())).expect("run");
    run.status = RunStatus::Ok;
    run.restore = Some(RestoreStats {
        files_new: 2,
        ..RestoreStats::default()
    });
    assert!(matches!(run.kind, RunKind::Restore { .. }), "(a)");
    assert_eq!(run.destination_id, dest, "(b)");
    assert_eq!(run.backup_id(), Some(&b1), "(c)");
    let dir = tempfile::tempdir().expect("tempdir");
    let mut history = RunHistory::open(dir.path()).expect("history");
    history.upsert_run(&run, &[]).expect("store run");
    let runs = history.list_runs().expect("runs");
    let of_backup: Vec<&Run> = runs.iter().filter(|r| r.backup_id() == Some(&b1)).collect();
    let of_dest: Vec<&Run> = runs.iter().filter(|r| r.destination_id == dest).collect();
    assert_eq!(of_backup.len(), 1, "(d) history of b1");
    assert_eq!(of_dest.len(), 1, "(d) history of the destination");
    assert_eq!(of_backup[0].restore, run.restore, "(d) stats stored");
}

fn input<'a>(home: &'a Path, backup: Option<&'a str>) -> SuggestionInput<'a> {
    SuggestionInput {
        home,
        words: FolderWords {
            restored: "Wiederhergestellt",
            not_assigned: "nicht zugeordnet",
        },
        backup_name: backup,
        destination_name: "NAS",
        snapshot_short_id: "e3d4e5f6",
        snapshot_time: datetime!(2026-11-15 12:00 +01:00),
    }
}

/// AC-M4-05-13 (unit part): the suggested new folder (a) is
/// `~/Wiederhergestellt/Persönlicher Ordner 2026-11-15 12-00`, (b) gets « (2)» when that folder
/// exists, (c) is `<destination> nicht zugeordnet <short id>` without a backup, and (d) has no
/// `/`, control characters or leading dot.
#[test]
fn ac_m4_05_13_suggested_new_folder_name() {
    let home = tempfile::tempdir().expect("home");
    let first = suggested_new_folder(&input(home.path(), Some("Persönlicher Ordner")));
    assert_eq!(
        first,
        home.path()
            .join("Wiederhergestellt/Persönlicher Ordner 2026-11-15 12-00"),
        "(a)"
    );
    std::fs::create_dir_all(&first).expect("existing folder");
    assert_eq!(
        suggested_new_folder(&input(home.path(), Some("Persönlicher Ordner"))),
        home.path()
            .join("Wiederhergestellt/Persönlicher Ordner 2026-11-15 12-00 (2)"),
        "(b)"
    );
    assert_eq!(
        suggested_new_folder(&input(home.path(), None)),
        home.path()
            .join("Wiederhergestellt/NAS nicht zugeordnet e3d4e5f6"),
        "(c)"
    );
    assert_eq!(
        new_folder_name("..a/b\u{7}c", datetime!(2026-11-15 12:00 UTC)),
        "abc 2026-11-15 12-00",
        "(d)"
    );
}

fn root_snapshot() -> ResticSnapshot {
    let raw = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m4/snapshot_of_root.json"),
    )
    .expect("fixture");
    serde_json::from_str(&raw).expect("snapshot json")
}

/// AC-M4-05-17 (unit part): (b) every restore runs as `<snapshot>:<common folder of the
/// selection>` (a single chosen folder is its own), and (c) `--target` is the new or chosen
/// folder, at the original location the folder of the selection, never `/`, also for a snapshot
/// of `/` (JSON fixture).
#[test]
fn ac_m4_05_17_restore_runs_from_the_common_folder_never_into_root() {
    let several = sel(&["/home/u/docs/a.txt", "/home/u/docs/sub/b.txt", "/home/u/x"]);
    assert_eq!(probe_path(&several), "/home/u", "(b) probe of several");
    let plan = plan_restore(ID, &several, EntryKind::Dir).expect("plan");
    assert_eq!(plan.folder, "/home/u", "(b) common folder");
    let folder = plan_restore(ID, &sel(&["/home/u/docs"]), EntryKind::Dir).expect("plan");
    assert_eq!(folder.folder, "/home/u/docs", "(b) a folder is its own");
    assert!(folder.includes.is_empty(), "(b) no include for a folder");
    let args = restore_args(
        &folder,
        Path::new("/new"),
        RestoreOverwrite::Never,
        RestoreCall::Run,
    );
    assert!(args.contains(&format!("{ID}:/home/u/docs")), "(b) {args:?}");
    let target = args
        .iter()
        .position(|a| a == "--target")
        .and_then(|i| args.get(i + 1));
    assert_eq!(target.map(String::as_str), Some("/new"), "(c) new folder");

    let snapshot = root_snapshot();
    let mut probe = ProbeCollector::new("/");
    probe.push_line(
        &serde_json::to_string(&serde_json::json!({
            "message_type": "snapshot", "paths": ["/"], "time": snapshot.time, "id": snapshot.id,
        }))
        .expect("json"),
    );
    assert_eq!(probe.finish().1, EntryKind::Dir, "(c) `/` always exists");
    let across = sel(&["/etc/x", "/home/u/y"]);
    let plan = plan_restore(ID, &across, EntryKind::Dir).expect("plan");
    assert_eq!(plan.folder, "/", "(c) common folder of top-level entries");
    let original = original_location(&plan.folder, &snapshot);
    assert!(original.needs_user_choice, "(c) `/` needs a chosen folder");
    assert_eq!(original.mapped_dir, None, "(c) `/` is never a target");
    let inside = original_location("/etc", &snapshot);
    assert_eq!(
        inside.mapped_dir.as_deref(),
        Some("/etc"),
        "(c) original folder"
    );
}

/// AC-M4-05-18 (unit part): `escape_glob` escapes `\`, `*`, `?` and `[` with `\` and leaves
/// everything else (spaces, umlauts, line breaks, leading `-`, `]`) alone.
#[test]
fn ac_m4_05_18_escape_glob_escapes_glob_characters_only() {
    assert_eq!(escape_glob("[x].txt"), "\\[x].txt");
    assert_eq!(escape_glob("*star?.txt"), "\\*star\\?.txt");
    assert_eq!(escape_glob("back\\slash"), "back\\\\slash");
    assert_eq!(escape_glob("-dash"), "-dash");
    assert_eq!(escape_glob("new\nline ü.txt"), "new\nline ü.txt");
    let plan = plan_restore(ID, &sel(&["/d/[x].txt"]), EntryKind::Other).expect("plan");
    assert_eq!(plan.folder, "/d");
    assert_eq!(plan.includes, vec!["--include=/\\[x].txt".to_string()]);
}

/// AC-M4-05-22 (b, c): «outdated» means older than the newest version the tests run against,
/// `RESTIC_LATEST_VERSION` in `scripts/bootstrap.sh`; 0.18.0 is outdated, that version not.
#[test]
fn ac_m4_05_22_outdated_is_older_than_the_tested_version() {
    let bootstrap = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/bootstrap.sh"),
    )
    .expect("bootstrap.sh");
    let latest = bootstrap
        .lines()
        .find_map(|l| l.strip_prefix("RESTIC_LATEST_VERSION="))
        .map(|v| v.trim_matches('"').to_string())
        .expect("RESTIC_LATEST_VERSION");
    assert_eq!(
        kestic_core::provider::TESTED_RESTIC_VERSION.to_string(),
        latest,
        "the tested version follows bootstrap.sh"
    );
    assert!(kestic_core::provider::is_outdated_text("0.18.0"), "(b)");
    assert!(!kestic_core::provider::is_outdated_text(&latest), "(c)");
}

/// A restore waiting for maintenance lives only in the daemon: at the next start it is no
/// longer `pending`.
#[test]
fn m4_s05_waiting_restore_ends_at_daemon_start() {
    let clock = FakeClock::new(datetime!(2026-11-15 12:00 UTC));
    let mut run =
        Run::new_restore(&clock, DestinationId::generate().expect("id"), None).expect("run");
    run.status = RunStatus::Pending;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut history = RunHistory::open(dir.path()).expect("history");
    history.upsert_run(&run, &[]).expect("store run");
    history
        .mark_interrupted_on_startup(&clock)
        .expect("startup");
    let runs = history.list_runs().expect("runs");
    assert_eq!(runs[0].status, RunStatus::Interrupted);
}

/// A missing new folder is checked at its deepest existing ancestor.
#[test]
fn m4_s05_missing_folder_is_checked_at_its_parent() {
    use kestic_core::restore::RestoreTarget;
    use kestic_core::restore_target::check_target;
    use kestic_i18n::codes::RestoreTargetCode;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("file");
    std::fs::write(&file, "x").expect("file");
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).expect("mkdir");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    let target = |p: std::path::PathBuf| RestoreTarget::NewFolder {
        dir: p.to_string_lossy().into_owned(),
    };
    let below_file = check_target(&target(file.join("new")), &[]).code;
    let below_locked = check_target(&target(locked.join("a")), &[]).code;
    let too_deep = check_target(&target(dir.path().join("x/y/z")), &[]).code;
    let fresh = check_target(&target(dir.path().join("new/x")), &[]).code;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    assert_eq!(below_file, RestoreTargetCode::NotADirectory);
    assert_eq!(below_locked, RestoreTargetCode::NotWritable);
    assert_eq!(fresh, RestoreTargetCode::Ok);
    assert_eq!(
        too_deep,
        RestoreTargetCode::MissingMappedDir,
        "only two levels are created"
    );
}

/// A name restic cannot match (not UTF-8, shown as U+FFFD) is replaced by the nearest folder
/// above it without U+FFFD; names restic can match stay.
#[test]
fn m4_s05_unmatchable_name_gives_way_to_its_folder() {
    assert_eq!(
        folder_instead_of_unmatchable("/home/u/docs/bad\u{FFFD}.txt").as_deref(),
        Some("/home/u/docs")
    );
    assert_eq!(
        folder_instead_of_unmatchable("/home/u/b\u{FFFD}d/a.txt").as_deref(),
        Some("/home/u"),
        "a folder with U+FFFD gives way to its own folder"
    );
    assert_eq!(
        folder_instead_of_unmatchable("/b\u{FFFD}d").as_deref(),
        Some("/")
    );
    assert_eq!(folder_instead_of_unmatchable("/home/u/a.txt"), None);
    assert_eq!(
        selection_with_folders_instead(&strings(&[
            "/home/u/docs/a.txt",
            "/home/u/docs/x\u{FFFD}.txt",
            "/home/u/docs/y\u{FFFD}.txt",
        ])),
        strings(&["/home/u/docs/a.txt", "/home/u/docs"])
    );
}

fn strings(paths: &[&str]) -> Vec<String> {
    paths.iter().map(ToString::to_string).collect()
}

/// A symbolic link to nothing is no target folder: restic could not create the folder there.
#[test]
fn m4_s05_dangling_symlink_is_no_target_folder() {
    use kestic_core::restore::RestoreTarget;
    use kestic_core::restore_target::check_target;
    use kestic_i18n::codes::RestoreTargetCode;
    let dir = tempfile::tempdir().expect("tempdir");
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(dir.path().join("gone"), &link).expect("symlink");
    let target = RestoreTarget::NewFolder {
        dir: link.to_string_lossy().into_owned(),
    };
    assert_eq!(
        check_target(&target, &[]).code,
        RestoreTargetCode::NotADirectory
    );
}

/// A restore cancelled before restic starts (while the destination is checked) (a) ends as
/// cancelled and (b) starts no restic process.
#[test]
fn m4_s05_restore_cancelled_before_the_start_runs_no_restic() {
    use kestic_core::backup::{BackupRunError, SnapshotListAccess};
    use kestic_core::model::{Destination, DestinationKind};
    use kestic_core::provider::ResolvedRestic;
    use kestic_core::restic::CancelHandle;
    use kestic_core::restore::execute_restore;
    use kestic_core::secrets::InMemorySecretStore;

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let calls = tmp.path().join("calls.log");
    let bin = tmp.path().join("spy-restic");
    kestic_test_support::tools::write_executable(
        &bin,
        &format!("#!/bin/sh\necho \"$*\" >> '{}'\nexit 0\n", calls.display()),
    );
    let resolved = ResolvedRestic {
        binary: bin,
        version: semver::Version::new(0, 18, 0),
    };
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir repo");
    let dest = Destination::new("NAS", DestinationKind::Local { path: repo }).expect("dest");
    let store = InMemorySecretStore::default();
    let clock = FakeClock::new(datetime!(2026-11-15 14:32:00 UTC));
    let kestic_bin = tmp.path().join("kestic");
    let access = SnapshotListAccess {
        destination: &dest,
        resolved: &resolved,
        kestic_binary: &kestic_bin,
        secret_store: &store,
        clock: &clock,
        extra_env: &[],
    };
    let plan = plan_restore(ID, &sel(&["/home/u/a.txt"]), EntryKind::Other).expect("plan");
    let cancel = CancelHandle::new();
    cancel.cancel();
    let target = tmp.path().join("target");
    let result = execute_restore(
        &access,
        &plan,
        &target,
        RestoreOverwrite::Never,
        Some(&cancel),
        &mut |_| {},
    );
    assert!(
        matches!(result, Err(BackupRunError::Cancelled)),
        "(a) {:?}",
        result.map(|(r, _)| r)
    );
    assert!(!calls.exists(), "(b) restic never started");
}
