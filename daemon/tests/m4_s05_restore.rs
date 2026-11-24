// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S05 restoring files against real restic (0.18.0 and latest) with temp repositories:
//! the preview (`StartSnapshotQuery` kind `restore_preview`), `StartRestore` and the run.

mod common;

use common::m3::{ResticVariant, restic_variants};
use common::m4::{big_page, small_tree};
use common::m4_restore::{RestoreFixture, for_each_variant, set_mtime, tree_contents};
use common::run_isolated_test;
use kestic_core::model::{RunKind, RunStatus};
use kestic_daemon::dbus_api::{
    KesticDbusError, PageFilter, RestoreOutcome, RestoreOverwrite, RestorePreviewResultDto,
    RestoreRequest, RestoreTarget, RestoreTargetCode, SnapshotQueryPage,
};
use kestic_test_support::tools::find_lock_file;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Outcome of every entry of a preview by its file name.
fn outcomes(p: &RestorePreviewResultDto) -> BTreeMap<String, RestoreOutcome> {
    p.entries
        .items
        .iter()
        .map(|e| {
            let name = Path::new(&e.path)
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            (name, e.outcome)
        })
        .collect()
}

/// AC-M4-05-2: a restore into a new, empty folder (a) writes `a.txt` with identical content
/// there and (b) ends with `ok`.
#[test]
fn ac_m4_05_2_restore_into_new_folder_writes_identical_file() {
    run_isolated_test(
        "ac_m4_05_2_restore_into_new_folder_writes_identical_file",
        "KESTIC_AC_M4_05_2_INNER",
        |root| {
            for_each_variant(root, "M4R2", small_tree, |fx, v| {
                let target = fx.new_dir("new");
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        RestoreTarget::NewFolder {
                            dir: target.to_string_lossy().into_owned(),
                        },
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                let detail = fx.wait_run(&run_id);
                assert_eq!(
                    fs::read_to_string(target.join("a.txt")).expect("a.txt restored"),
                    "hello",
                    "(a) {}: identical content",
                    v.label
                );
                assert_eq!(
                    detail.run.status,
                    RunStatus::Ok,
                    "(b) {}: {detail:?}",
                    v.label
                );
            });
        },
    );
}

/// AC-M4-05-3: a restore to the original location with a differing `a.txt` and the mode left
/// alone (a) uses `never`, (b) leaves `a.txt` unchanged and (c) the preview calls it «not
/// overwritten» (`kept`).
#[test]
fn ac_m4_05_3_original_location_keeps_differing_file_by_default() {
    run_isolated_test(
        "ac_m4_05_3_original_location_keeps_differing_file_by_default",
        "KESTIC_AC_M4_05_3_INNER",
        |root| {
            for_each_variant(root, "M4R3", small_tree, |fx, v| {
                fs::write(fx.src.join("a.txt"), "changed locally").expect("change a.txt");
                let (preview_id, preview) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::default());
                assert_eq!(
                    outcomes(&preview).get("a.txt"),
                    Some(&RestoreOutcome::Kept),
                    "(c) {}: {preview:?}",
                    v.label
                );
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        fx.original(),
                        RestoreOverwrite::default(),
                        Some(&preview_id),
                    )
                    .expect("StartRestore");
                let detail = fx.wait_run(&run_id);
                let args = &detail.run.commands.last().expect("restore command").args;
                let mode = args
                    .iter()
                    .position(|a| a == "--overwrite")
                    .and_then(|i| args.get(i + 1));
                assert_eq!(
                    mode.map(String::as_str),
                    Some("never"),
                    "(a) {}: {args:?}",
                    v.label
                );
                // AC-M4-05-17 (c): at the original location `--target` is the common folder.
                let target = args
                    .iter()
                    .position(|a| a == "--target")
                    .and_then(|i| args.get(i + 1));
                let src = fs::canonicalize(&fx.src).expect("canonical src");
                assert_eq!(
                    target.map(String::as_str),
                    Some(src.to_string_lossy().as_ref()),
                    "AC-M4-05-17 (c) {}: {args:?}",
                    v.label
                );
                assert_eq!(
                    fs::read_to_string(fx.src.join("a.txt")).expect("read a.txt"),
                    "changed locally",
                    "(b) {}: a.txt unchanged",
                    v.label
                );
            });
        },
    );
}

/// AC-M4-05-8: with only `docs/` selected, (a) only files below `docs/` are written.
#[test]
fn ac_m4_05_8_only_the_selected_folder_is_restored() {
    run_isolated_test(
        "ac_m4_05_8_only_the_selected_folder_is_restored",
        "KESTIC_AC_M4_05_8_INNER",
        |root| {
            for_each_variant(root, "M4R8", small_tree, |fx, v| {
                let target = fx.new_dir("docs-only");
                let run_id = fx
                    .start_restore(
                        &["docs"],
                        RestoreTarget::NewFolder {
                            dir: target.to_string_lossy().into_owned(),
                        },
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
                let written: Vec<String> = tree_contents(&target).into_keys().collect();
                assert_eq!(
                    written,
                    vec!["deep/y.txt".to_string(), "x.txt".to_string()],
                    "(a) {}: only the files of docs/",
                    v.label
                );
            });
        },
    );
}

fn new_folder(dir: &Path) -> RestoreTarget {
    RestoreTarget::NewFolder {
        dir: dir.to_string_lossy().into_owned(),
    }
}

/// Number of `restic restore` calls with `--dry-run` so far.
fn dry_runs(fx: &RestoreFixture) -> Vec<String> {
    fx.restore_calls()
        .into_iter()
        .filter(|c| c.contains("--dry-run"))
        .collect()
}

/// The local tree for AC-M4-05-4: in the snapshot `a.txt`, `b.bin`, `docs/x.txt` and
/// `docs/deep/y.txt`; afterwards locally `a.txt` older and different (overwritten with
/// `if-newer`), `docs/deep/y.txt` newer and different (kept), `b.bin` as backed up (unchanged)
/// and `docs/x.txt` gone (new).
fn four_outcomes(fx: &RestoreFixture) {
    fs::write(fx.src.join("a.txt"), "older local").expect("write a.txt");
    set_mtime(&fx.src.join("a.txt"), 1_000_000);
    fs::write(fx.src.join("docs/deep/y.txt"), "newer local").expect("write y.txt");
    set_mtime(&fx.src.join("docs/deep/y.txt"), 4_000_000_000);
    fs::remove_file(fx.src.join("docs/x.txt")).expect("remove x.txt");
}

/// AC-M4-05-4: at the original location (a) `restore --dry-run` runs first, (b) the preview
/// counts and lists every file in one of the four outcomes, (c) nothing is written before
/// `StartRestore`, and (d) a second dry run with `--overwrite always` separates «not
/// overwritten» from «unchanged» only when the mode is not `always`.
#[test]
fn ac_m4_05_4_preview_dry_runs_first_and_lists_four_outcomes() {
    run_isolated_test(
        "ac_m4_05_4_preview_dry_runs_first_and_lists_four_outcomes",
        "KESTIC_AC_M4_05_4_INNER",
        |root| {
            for_each_variant(root, "M4R4", small_tree, |fx, v| {
                four_outcomes(fx);
                let before = tree_contents(&fx.src);
                let (id, p) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::IfNewer);
                assert!(
                    !dry_runs(fx).is_empty() && fx.restore_calls().len() == dry_runs(fx).len(),
                    "(a) {}: only dry runs before the start: {:?}",
                    v.label,
                    fx.restore_calls()
                );
                let o = outcomes(&p);
                assert_eq!(
                    o.get("x.txt"),
                    Some(&RestoreOutcome::New),
                    "(b) {}",
                    v.label
                );
                assert_eq!(
                    o.get("a.txt"),
                    Some(&RestoreOutcome::Overwritten),
                    "(b) {}",
                    v.label
                );
                assert_eq!(
                    o.get("y.txt"),
                    Some(&RestoreOutcome::Kept),
                    "(b) {}",
                    v.label
                );
                assert_eq!(
                    o.get("b.bin"),
                    Some(&RestoreOutcome::Identical),
                    "(b) {}",
                    v.label
                );
                assert_eq!(
                    (
                        p.new_files,
                        p.overwritten_files,
                        p.kept_files,
                        p.identical_files
                    ),
                    (1, 1, 1, 1),
                    "(b) {}: counts",
                    v.label
                );
                assert_eq!(tree_contents(&fx.src), before, "(c) {}: unchanged", v.label);
                let run_id = fx
                    .start_restore(&[], fx.original(), RestoreOverwrite::IfNewer, Some(&id))
                    .expect("StartRestore");
                assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
                assert_eq!(
                    fs::read_to_string(fx.src.join("a.txt")).expect("a.txt"),
                    "hello",
                    "(c) {}: written after the start",
                    v.label
                );
                let calls = dry_runs(fx);
                assert_eq!(calls.len(), 2, "(d) {}: two dry runs: {calls:?}", v.label);
                assert!(
                    calls.iter().any(|c| c.contains("--overwrite always")),
                    "(d) {}: second dry run with always: {calls:?}",
                    v.label
                );
                let seen = dry_runs(fx).len();
                fx.preview(&[], Some(fx.original()), RestoreOverwrite::Always);
                assert_eq!(
                    dry_runs(fx).len() - seen,
                    1,
                    "(d) {}: one dry run with always",
                    v.label
                );
            });
        },
    );
}

/// Restores `a.txt` with `restic restore --overwrite <mode>` directly into `target`.
fn restic_restore(fx: &RestoreFixture, target: &Path, mode: &str) {
    let out = std::process::Command::new(&fx.rig.variant.bin)
        .arg("-r")
        .arg(&fx.repo)
        .args(["restore", "--no-cache"])
        .arg(format!("{}:{}", fx.snapshot_id, fx.path("")))
        .arg("--target")
        .arg(target)
        .args(["--overwrite", mode, "--include=/a.txt"])
        .env("RESTIC_PASSWORD", &fx.password)
        .output()
        .expect("run restic restore");
    assert!(out.status.success(), "restic restore --overwrite {mode}");
}

/// AC-M4-05-5: at the original location with an older, a newer and a changed `a.txt`,
/// restoring with `if-newer`, `if-changed` and `always` (a) gives the result of
/// `restic restore --overwrite <mode>`.
#[test]
fn ac_m4_05_5_modes_match_restic_overwrite() {
    run_isolated_test(
        "ac_m4_05_5_modes_match_restic_overwrite",
        "KESTIC_AC_M4_05_5_INNER",
        |root| {
            for_each_variant(root, "M4R5", small_tree, |fx, v| {
                for (mode, arg, mtime) in [
                    (RestoreOverwrite::IfNewer, "if-newer", 1_000_000),
                    (RestoreOverwrite::IfNewer, "if-newer", 4_000_000_000),
                    (RestoreOverwrite::IfChanged, "if-changed", 4_000_000_000),
                    (RestoreOverwrite::Always, "always", 4_000_000_000),
                ] {
                    let reference = fx.new_dir(&format!("ref-{arg}-{mtime}"));
                    fs::create_dir_all(&reference).expect("mkdir reference");
                    for dir in [&fx.src, &reference] {
                        fs::write(dir.join("a.txt"), "local").expect("write a.txt");
                        set_mtime(&dir.join("a.txt"), mtime);
                    }
                    restic_restore(fx, &reference, arg);
                    let (id, _) = fx.preview(&["a.txt"], Some(fx.original()), mode);
                    let run_id = fx
                        .start_restore(&["a.txt"], fx.original(), mode, Some(&id))
                        .expect("StartRestore");
                    assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
                    assert_eq!(
                        fs::read_to_string(fx.src.join("a.txt")).expect("a.txt"),
                        fs::read_to_string(reference.join("a.txt")).expect("reference"),
                        "(a) {}: {arg} with local mtime {mtime}",
                        v.label
                    );
                }
            });
        },
    );
}

/// AC-M4-05-7: when the user cancels at the confirmation (the preview ran, no `StartRestore`),
/// (a) no file at the target is changed.
#[test]
fn ac_m4_05_7_cancelled_confirmation_changes_nothing() {
    run_isolated_test(
        "ac_m4_05_7_cancelled_confirmation_changes_nothing",
        "KESTIC_AC_M4_05_7_INNER",
        |root| {
            for_each_variant(root, "M4R7", small_tree, |fx, v| {
                four_outcomes(fx);
                let before = tree_contents(&fx.src);
                let (id, _) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Always);
                fx.rig
                    .daemon
                    .client
                    .cancel_snapshot_query(&id)
                    .expect("CancelSnapshotQuery");
                assert_eq!(tree_contents(&fx.src), before, "(a) {}", v.label);
            });
        },
    );
}

/// AC-M4-05-14: at the original location, `StartRestore` without `preview_id`, with the preview
/// of other parameters, with an unfinished preview or with one older than 10 minutes by the
/// clock (a) fails with `PreviewRequired`, (b) starts no restic process and (c) creates no run;
/// with a matching, finished, fresh preview (d) the run starts.
#[test]
fn ac_m4_05_14_restore_without_fitting_fresh_preview_is_refused() {
    run_isolated_test(
        "ac_m4_05_14_restore_without_fitting_fresh_preview_is_refused",
        "KESTIC_AC_M4_05_14_INNER",
        |root| {
            for_each_variant(root, "M4R14", small_tree, |fx, v| {
                fs::write(fx.src.join("a.txt"), "local").expect("write a.txt");
                let (never_id, _) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Never);
                fx.rig.wrapper.hold("restore");
                let running_id =
                    fx.start_preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
                fx.rig
                    .wrapper
                    .wait_for('S', "restore", 3, std::time::Duration::from_secs(30));
                let refused = |preview: Option<&str>, case: &str| {
                    let started = fx.rig.wrapper.started_repo_count();
                    let runs = restore_runs(fx);
                    let err = fx
                        .start_restore(&["a.txt"], fx.original(), RestoreOverwrite::Always, preview)
                        .expect_err("StartRestore must be refused");
                    assert_eq!(
                        err.code(),
                        kestic_i18n::codes::ServiceErrorCode::PreviewRequired,
                        "(a) {}: {case}",
                        v.label
                    );
                    assert_eq!(
                        fx.rig.wrapper.started_repo_count(),
                        started,
                        "(b) {}: {case}: no restic process",
                        v.label
                    );
                    assert_eq!(restore_runs(fx), runs, "(c) {}: {case}: no run", v.label);
                };
                refused(None, "without preview");
                refused(Some(&never_id), "other mode");
                refused(Some(&running_id), "unfinished");
                fx.rig.wrapper.release("restore");
                let (whole_id, _) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Always);
                refused(Some(&whole_id), "other selection");
                let second = common::m4::backup_dir(
                    &v.bin,
                    &fx.repo,
                    &fx.password,
                    &fx.src,
                    &common::m4::RawSnapshot {
                        tag: &format!("kestic:{}", fx.backup.id),
                        time: "2026-11-15 13:00:00",
                        host: "laptop",
                    },
                );
                let other_snapshot = fx
                    .rig
                    .daemon
                    .client
                    .start_snapshot_query(
                        &kestic_daemon::dbus_api::SnapshotQueryRequest::restore_preview(
                            fx.dest.id.as_str(),
                            &second,
                            &[fx.path("a.txt")],
                            Some(fx.original()),
                            RestoreOverwrite::Always,
                        ),
                    )
                    .expect("preview of the second snapshot");
                fx.preview_page(&other_snapshot, &big_page());
                refused(Some(&other_snapshot), "other snapshot");
                let (fresh_id, _) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
                fx.rig.clock.advance(std::time::Duration::from_mins(11));
                refused(Some(&fresh_id), "older than 10 minutes");
                // A preview of a folder that was empty ran no dry run; it does not count once
                // the folder has content.
                let folder = fx.new_dir("filled-later");
                fs::create_dir_all(&folder).expect("mkdir folder");
                let (empty_id, _) = fx.preview(
                    &["a.txt"],
                    Some(new_folder(&folder)),
                    RestoreOverwrite::Always,
                );
                fs::write(folder.join("a.txt"), "local").expect("fill folder");
                let started = fx.rig.wrapper.started_repo_count();
                let runs = restore_runs(fx);
                let err = fx
                    .start_restore(
                        &["a.txt"],
                        new_folder(&folder),
                        RestoreOverwrite::Always,
                        Some(&empty_id),
                    )
                    .expect_err("a preview without dry run must not count");
                assert_eq!(
                    err.code(),
                    kestic_i18n::codes::ServiceErrorCode::PreviewRequired,
                    "(a) {}: preview without dry run",
                    v.label
                );
                assert_eq!(
                    fx.rig.wrapper.started_repo_count(),
                    started,
                    "(b) {}",
                    v.label
                );
                assert_eq!(restore_runs(fx), runs, "(c) {}", v.label);
                let (other_target, _) = fx.preview(
                    &["a.txt"],
                    Some(new_folder(&folder)),
                    RestoreOverwrite::Always,
                );
                refused(Some(&other_target), "other target");
                let (ok_id, _) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        fx.original(),
                        RestoreOverwrite::Always,
                        Some(&ok_id),
                    )
                    .expect("(d) StartRestore with a fitting preview");
                assert_eq!(
                    fx.wait_run(&run_id).run.status,
                    RunStatus::Ok,
                    "(d) {}",
                    v.label
                );
            });
        },
    );
}

/// Number of restore runs the daemon knows.
fn restore_runs(fx: &RestoreFixture) -> usize {
    fx.rig
        .daemon
        .client
        .get_state()
        .expect("GetState")
        .recent_runs
        .iter()
        .filter(|r| matches!(r.kind, RunKind::Restore { .. }))
        .count()
}

/// AC-M4-05-15: at the default target (a new, empty folder) the preview (a) gives only the
/// suggestion and the check, without `restore --dry-run`, with empty counts and no notes, and
/// (b) `StartRestore` needs no `preview_id`. For an existing folder with content (d) the
/// preview is mandatory. A file as target is (f) `not_a_directory`, a folder without write
/// permission (g) `not_writable`, a folder inside the repository of a local destination
/// (h) `inside_repository`, each as status, not as error.
#[test]
fn ac_m4_05_15_default_target_needs_no_dry_run_and_targets_are_checked() {
    run_isolated_test(
        "ac_m4_05_15_default_target_needs_no_dry_run_and_targets_are_checked",
        "KESTIC_AC_M4_05_15_INNER",
        |root| {
            for_each_variant(root, "M4R15", small_tree, |fx, v| {
                let (_, p) = fx.preview(&["a.txt"], None, RestoreOverwrite::Never);
                assert!(dry_runs(fx).is_empty(), "(a) {}: no dry run", v.label);
                assert_eq!(
                    (
                        p.new_files,
                        p.overwritten_files,
                        p.kept_files,
                        p.identical_files
                    ),
                    (0, 0, 0, 0),
                    "(a) {}: empty counts",
                    v.label
                );
                assert_eq!(p.entries.total, 0, "(a) {}: no entries", v.label);
                assert!(p.notes.is_empty(), "(a) {}: no notes", v.label);
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::Ok,
                    "(a) {}",
                    v.label
                );
                assert_eq!(
                    p.target.target.dir(),
                    p.suggestion.new_folder,
                    "(a) {}: suggested folder",
                    v.label
                );
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        p.target.target.clone(),
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("(b) StartRestore without preview");
                assert_eq!(
                    fx.wait_run(&run_id).run.status,
                    RunStatus::Ok,
                    "(b) {}",
                    v.label
                );

                let full = fx.new_dir("full");
                fs::create_dir_all(&full).expect("mkdir full");
                fs::write(full.join("other.txt"), "x").expect("write other");
                let err = fx
                    .start_restore(&["a.txt"], new_folder(&full), RestoreOverwrite::Never, None)
                    .expect_err("(d) preview required");
                assert_eq!(
                    err.code(),
                    kestic_i18n::codes::ServiceErrorCode::PreviewRequired,
                    "(d) {}",
                    v.label
                );
                let (_, p) =
                    fx.preview(&["a.txt"], Some(new_folder(&full)), RestoreOverwrite::Never);
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::ExistsNotEmpty,
                    "(d) {}",
                    v.label
                );

                let file = fx.new_dir("a-file");
                fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
                fs::write(&file, "x").expect("write file");
                let (_, p) =
                    fx.preview(&["a.txt"], Some(new_folder(&file)), RestoreOverwrite::Never);
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::NotADirectory,
                    "(f) {}",
                    v.label
                );

                let locked = fx.new_dir("locked");
                fs::create_dir_all(&locked).expect("mkdir locked");
                set_mode(&locked, 0o555);
                let (_, p) = fx.preview(
                    &["a.txt"],
                    Some(new_folder(&locked)),
                    RestoreOverwrite::Never,
                );
                set_mode(&locked, 0o755);
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::NotWritable,
                    "(g) {}",
                    v.label
                );

                let in_repo = fx.repo.join("restored-here");
                let (_, p) = fx.preview(
                    &["a.txt"],
                    Some(new_folder(&in_repo)),
                    RestoreOverwrite::Never,
                );
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::InsideRepository,
                    "(h) {}",
                    v.label
                );
            });
        },
    );
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
}

/// A folder with a file and a second hard link to it, and a symbolic link to `a.txt`.
fn linked_tree(work: &Path) -> std::path::PathBuf {
    let src = small_tree(work);
    fs::hard_link(src.join("a.txt"), src.join("hard.txt")).expect("hard link");
    std::os::unix::fs::symlink("a.txt", src.join("link")).expect("symlink");
    src
}

/// AC-M4-05-16: a preview with dry run of a snapshot of another uid (a) has the note
/// `ownership_not_restored`, one of the user's own uid (b) has not; a file with further hard
/// links that the preview counts as overwritten (c) gives `hard_links_affected`, without
/// overwriting (d) not; (e) a symbolic link in the snapshot is restored as a link.
#[test]
fn ac_m4_05_16_notes_for_owner_hard_links_and_symlinks() {
    run_isolated_test(
        "ac_m4_05_16_notes_for_owner_hard_links_and_symlinks",
        "KESTIC_AC_M4_05_16_INNER",
        |root| {
            for variant in restic_variants() {
                let mut fx = RestoreFixture::start(root, &variant, "M4R16", linked_tree, |_| {});
                let v = &variant;
                fs::write(fx.src.join("a.txt"), "changed").expect("change a.txt");
                let (_, own) = fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Never);
                assert!(
                    !own.notes
                        .contains(&kestic_daemon::dbus_api::RestoreNoteCode::OwnershipNotRestored),
                    "(b) {}: {:?}",
                    v.label,
                    own.notes
                );
                assert!(
                    !own.notes
                        .contains(&kestic_daemon::dbus_api::RestoreNoteCode::HardLinksAffected),
                    "(d) {}: nothing overwritten: {:?}",
                    v.label,
                    own.notes
                );
                let (_, always) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
                assert!(
                    always
                        .notes
                        .contains(&kestic_daemon::dbus_api::RestoreNoteCode::HardLinksAffected),
                    "(c) {}: {:?}",
                    v.label,
                    always.notes
                );
                // Linked in the snapshot only: the second name is gone at the target and
                // restored with the first.
                fs::remove_file(fx.src.join("hard.txt")).expect("unlink hard.txt");
                let (_, in_snapshot) = fx.preview(
                    &["a.txt", "hard.txt"],
                    Some(fx.original()),
                    RestoreOverwrite::Always,
                );
                assert!(
                    in_snapshot
                        .notes
                        .contains(&kestic_daemon::dbus_api::RestoreNoteCode::HardLinksAffected),
                    "(c) {}: linked in the snapshot: {:?}",
                    v.label,
                    in_snapshot.notes
                );
                // Linked at the target only: b.bin has one name in the snapshot.
                let outside = fx.new_dir("outside-link");
                fs::create_dir_all(outside.parent().expect("parent")).expect("mkdir");
                fs::hard_link(fx.src.join("b.bin"), &outside).expect("link b.bin");
                fs::write(fx.src.join("b.bin"), "changed").expect("change b.bin");
                let (_, at_target) =
                    fx.preview(&["b.bin"], Some(fx.original()), RestoreOverwrite::Always);
                assert!(
                    at_target
                        .notes
                        .contains(&kestic_daemon::dbus_api::RestoreNoteCode::HardLinksAffected),
                    "(c) {}: linked at the target: {:?}",
                    v.label,
                    at_target.notes
                );
                fx.snapshot_id = kestic_test_support::repo_template::set_snapshot_owner(
                    &v.bin,
                    &fx.repo,
                    &fx.password,
                    &fx.snapshot_id,
                    (4242, "someone"),
                );
                let (_, foreign) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Never);
                assert!(
                    foreign
                        .notes
                        .contains(&kestic_daemon::dbus_api::RestoreNoteCode::OwnershipNotRestored),
                    "(a) {}: {:?}",
                    v.label,
                    foreign.notes
                );
                // A snapshot made by root: restic leaves `uid` out.
                fx.snapshot_id = kestic_test_support::repo_template::set_snapshot_owner(
                    &v.bin,
                    &fx.repo,
                    &fx.password,
                    &fx.snapshot_id,
                    (0, "root"),
                );
                let (_, by_root) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Never);
                assert!(
                    by_root
                        .notes
                        .contains(&kestic_daemon::dbus_api::RestoreNoteCode::OwnershipNotRestored),
                    "(a) {}: snapshot of root: {:?}",
                    v.label,
                    by_root.notes
                );
                let target = fx.new_dir("links");
                let run_id = fx
                    .start_restore(
                        &["link"],
                        new_folder(&target),
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
                let meta = fs::symlink_metadata(target.join("link")).expect("link restored");
                assert!(
                    meta.file_type().is_symlink(),
                    "(e) {}: still a link",
                    v.label
                );
                assert_eq!(
                    fs::read_link(target.join("link")).expect("read link"),
                    Path::new("a.txt"),
                    "(e) {}: same target",
                    v.label
                );
                fx.rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-05-17 (integration part): (a) a path with a `..` segment, a relative, an empty one and
/// one with NUL, as selection, folder or target, is refused with `InvalidInput` without calling
/// restic; (d) a folder missing in the snapshot is the error `path_not_in_snapshot`, not a
/// success with 0 files.
#[test]
fn ac_m4_05_17_invalid_paths_refused_and_missing_folder_is_an_error() {
    run_isolated_test(
        "ac_m4_05_17_invalid_paths_refused_and_missing_folder_is_an_error",
        "KESTIC_AC_M4_05_17_INNER",
        |root| {
            for_each_variant(root, "M4R17", small_tree, |fx, v| {
                let client = &fx.rig.daemon.client;
                let started = fx.rig.wrapper.started_repo_count();
                let src = fx.path("");
                for bad in [
                    format!("{src}/../x"),
                    "rel/x".to_string(),
                    String::new(),
                    format!("{src}/a\0b"),
                ] {
                    let as_selection = client.start_snapshot_query(
                        &kestic_daemon::dbus_api::SnapshotQueryRequest::restore_preview(
                            fx.dest.id.as_str(),
                            &fx.snapshot_id,
                            std::slice::from_ref(&bad),
                            None,
                            RestoreOverwrite::Never,
                        ),
                    );
                    let as_target = fx.start_restore(
                        &["a.txt"],
                        RestoreTarget::NewFolder { dir: bad.clone() },
                        RestoreOverwrite::Never,
                        None,
                    );
                    let as_folder = client.start_snapshot_query(
                        &kestic_daemon::dbus_api::SnapshotQueryRequest::ls(
                            fx.dest.id.as_str(),
                            &fx.snapshot_id,
                            &bad,
                        ),
                    );
                    for (what, res) in [
                        ("selection", as_selection),
                        ("target", as_target),
                        ("folder", as_folder),
                    ] {
                        let err = res.expect_err("must be refused");
                        assert_eq!(
                            err.code(),
                            kestic_i18n::codes::ServiceErrorCode::InvalidInput,
                            "(a) {}: {what} {bad:?}",
                            v.label
                        );
                    }
                }
                assert_eq!(
                    fx.rig.wrapper.started_repo_count(),
                    started,
                    "(a) {}: no restic call",
                    v.label
                );

                let id = fx.start_preview(&["nope"], None, RestoreOverwrite::Never);
                let st = fx.wait_query(&id, &big_page());
                assert_eq!(st.state, "error", "(d) {}: {st:?}", v.label);
                assert_eq!(
                    st.error_kind,
                    Some(kestic_i18n::codes::ErrorKind::PathNotInSnapshot),
                    "(d) {}",
                    v.label
                );
                let run_id = fx
                    .start_restore(
                        &["nope"],
                        new_folder(&fx.new_dir("missing")),
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                let detail = fx.wait_run(&run_id);
                assert_eq!(
                    detail.run.status,
                    RunStatus::Error,
                    "(d) {}: {detail:?}",
                    v.label
                );
                assert_eq!(
                    detail.error_kind,
                    Some(kestic_i18n::codes::ErrorKind::PathNotInSnapshot),
                    "(d) {}: {detail:?}",
                    v.label
                );
            });
        },
    );
}

/// The names of AC-M4-05-18, each a file with its own content.
const SPECIAL_NAMES: [&str; 7] = [
    "[x].txt",
    "*star?.txt",
    "back\\slash",
    "-dash",
    "new\nline.txt",
    "with space.txt",
    "ümlaut.txt",
];

/// A folder with the files of [`SPECIAL_NAMES`] and a sub-folder `[dir]*` with one file.
fn special_tree(work: &Path) -> std::path::PathBuf {
    let src = work.join("special");
    fs::create_dir_all(src.join("[dir]*")).expect("mkdir special");
    for name in SPECIAL_NAMES {
        fs::write(src.join(name), format!("content of {name}")).expect("write special");
    }
    fs::write(src.join("[dir]*/inner.txt"), "inner").expect("write inner");
    src
}

/// The arguments of the `restic restore` call of finished run `run_id`.
fn restore_args(fx: &RestoreFixture, run_id: &str) -> Vec<String> {
    let detail = fx.wait_run(run_id);
    assert_eq!(detail.run.status, RunStatus::Ok, "{detail:?}");
    detail
        .run
        .commands
        .iter()
        .find(|c| c.args.iter().any(|a| a == "restore"))
        .map(|c| c.args.clone())
        .expect("restore command recorded")
}

/// AC-M4-05-18: each of the special names chosen alone and restored (a) brings exactly that one
/// file; (b) a single chosen folder is its own folder `<snapshot>:<folder>` (literally) in the
/// call, and (c) every chosen entry is its own `--include=<escaped>` argument (`\ * ? [`
/// escaped, relative to the folder).
#[test]
fn ac_m4_05_18_special_names_restore_exactly_the_chosen_file() {
    run_isolated_test(
        "ac_m4_05_18_special_names_restore_exactly_the_chosen_file",
        "KESTIC_AC_M4_05_18_INNER",
        |root| {
            for_each_variant(root, "M4R18", special_tree, |fx, v| {
                for (i, name) in SPECIAL_NAMES.iter().enumerate() {
                    let target = fx.new_dir(&format!("special-{i}"));
                    let run_id = fx
                        .start_restore(&[name], new_folder(&target), RestoreOverwrite::Never, None)
                        .expect("StartRestore");
                    let args = restore_args(fx, &run_id);
                    let restored = tree_contents(&target);
                    assert_eq!(
                        restored.keys().collect::<Vec<_>>(),
                        vec![*name],
                        "(a) {}: only {name:?}",
                        v.label
                    );
                    assert_eq!(
                        restored.get(*name).map(String::as_str),
                        Some(format!("content of {name}").as_str()),
                        "(a) {}: content of {name:?}",
                        v.label
                    );
                    let escaped = kestic_core::restore::escape_glob(name);
                    assert!(
                        args.contains(&format!("--include=/{escaped}")),
                        "(c) {}: {name:?} as include in {args:?}",
                        v.label
                    );
                }
                let target = fx.new_dir("special-dir");
                let run_id = fx
                    .start_restore(
                        &["[dir]*"],
                        new_folder(&target),
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                let args = restore_args(fx, &run_id);
                assert!(
                    args.contains(&format!("{}:{}", fx.snapshot_id, fx.path("[dir]*"))),
                    "(b) {}: folder literally in {args:?}",
                    v.label
                );
                assert!(
                    !args.iter().any(|a| a.starts_with("--include")),
                    "(b) {}: no include for a single folder: {args:?}",
                    v.label
                );
                assert_eq!(
                    tree_contents(&target).keys().collect::<Vec<_>>(),
                    vec!["inner.txt"],
                    "(b) {}",
                    v.label
                );
            });
        },
    );
}

/// AC-M4-05-19: a restore whose selection matches nothing (restic: exit 0 without numbers)
/// (a) ends with `Warnung` («Nothing restored») and (b) never with `ok`.
#[test]
fn ac_m4_05_19_selection_matching_nothing_ends_with_warning() {
    run_isolated_test(
        "ac_m4_05_19_selection_matching_nothing_ends_with_warning",
        "KESTIC_AC_M4_05_19_INNER",
        |root| {
            for_each_variant(root, "M4R19", small_tree, |fx, v| {
                let run_id = fx
                    .start_restore(
                        &["gone-1.txt", "gone-2.txt"],
                        new_folder(&fx.new_dir("nothing")),
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                let detail = fx.wait_run(&run_id);
                assert_eq!(
                    detail.run.status,
                    RunStatus::Warning,
                    "(a) {}: {detail:?}",
                    v.label
                );
                assert_eq!(
                    detail.run.summary_message.as_deref(),
                    Some("Nothing restored"),
                    "(a) {}",
                    v.label
                );
                assert_ne!(detail.run.status, RunStatus::Ok, "(b) {}", v.label);
            });
        },
    );
}

/// AC-M4-05-21: a target folder that is a symbolic link to another folder (a) is allowed, and
/// (b) the preview names the resolved folder.
#[test]
fn ac_m4_05_21_symlinked_target_folder_is_allowed_and_resolved() {
    run_isolated_test(
        "ac_m4_05_21_symlinked_target_folder_is_allowed_and_resolved",
        "KESTIC_AC_M4_05_21_INNER",
        |root| {
            for_each_variant(root, "M4R21", small_tree, |fx, v| {
                let real = fx.new_dir("real");
                fs::create_dir_all(&real).expect("mkdir real");
                let link = fx.new_dir("link");
                std::os::unix::fs::symlink(&real, &link).expect("symlink");
                let (_, p) =
                    fx.preview(&["a.txt"], Some(new_folder(&link)), RestoreOverwrite::Never);
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::Ok,
                    "(a) {}",
                    v.label
                );
                assert_eq!(
                    Path::new(&p.target.check.detail),
                    fs::canonicalize(&real).expect("canonical"),
                    "(b) {}",
                    v.label
                );
            });
        },
    );
}

/// AC-M4-05-22 (integration part): with restic 0.18.0 (a) a restore runs without block,
/// warning or download request.
#[test]
fn ac_m4_05_22_restore_runs_with_restic_0_18_0() {
    run_isolated_test(
        "ac_m4_05_22_restore_runs_with_restic_0_18_0",
        "KESTIC_AC_M4_05_22_INNER",
        |root| {
            let variant = restic_variants()
                .into_iter()
                .find(|v| v.label.contains("0.18.0"))
                .expect("restic 0.18.0 variant");
            let fx = RestoreFixture::start(root, &variant, "M4R22", small_tree, |_| {});
            let state = fx.rig.daemon.client.get_state().expect("GetState");
            assert!(
                state.blocking.is_none(),
                "(a) no block: {:?}",
                state.blocking
            );
            assert!(state.restic_status.valid, "(a) restic usable");
            assert_eq!(
                state.restic_status.version.as_deref(),
                Some("0.18.0"),
                "(a) the 0.18.0 binary is used"
            );
            let target = fx.new_dir("old-restic");
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&target),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            let detail = fx.wait_run(&run_id);
            assert_eq!(detail.run.status, RunStatus::Ok, "(a) {detail:?}");
            assert!(target.join("a.txt").is_file(), "(a) restored");
            fx.rig.daemon.stop();
        },
    );
}

/// 600 files that are new at the original location, 300 that differ (overwritten with
/// `always`) and 100 identical ones.
fn thousand_tree(work: &Path) -> std::path::PathBuf {
    let src = work.join("thousand");
    fs::create_dir_all(&src).expect("mkdir thousand");
    for i in 0..1000 {
        fs::write(src.join(format!("f{i:04}.txt")), format!("file {i}")).expect("write");
    }
    src
}

/// AC-M4-05-23: a preview at the original location with 600 new, 300 overwritten and 100
/// unchanged files, filtered by «overwritten», (a) reaches all 300 page by page, (b) has
/// `matched` 300 and (c) keeps the four counts exact.
#[test]
fn ac_m4_05_23_large_preview_filters_page_by_page_with_exact_counts() {
    run_isolated_test(
        "ac_m4_05_23_large_preview_filters_page_by_page_with_exact_counts",
        "KESTIC_AC_M4_05_23_INNER",
        |root| {
            for_each_variant(root, "M4R23", thousand_tree, |fx, v| {
                for i in 0..600 {
                    fs::remove_file(fx.src.join(format!("f{i:04}.txt"))).expect("remove");
                }
                for i in 600..900 {
                    fs::write(fx.src.join(format!("f{i:04}.txt")), "changed").expect("change");
                }
                let id = fx.start_preview(&[], Some(fx.original()), RestoreOverwrite::Always);
                let mut seen = std::collections::BTreeSet::new();
                let mut offset = 0;
                let p = loop {
                    let page = SnapshotQueryPage {
                        offset,
                        limit: 100,
                        filter: PageFilter {
                            text: String::new(),
                            kinds: vec!["overwritten".to_string()],
                        },
                    };
                    let p = fx.preview_page(&id, &page);
                    if p.entries.items.is_empty() {
                        break p;
                    }
                    for e in &p.entries.items {
                        assert_eq!(e.outcome, RestoreOutcome::Overwritten, "{}", v.label);
                        seen.insert(e.path.clone());
                    }
                    offset += p.entries.items.len() as u64;
                };
                assert_eq!(seen.len(), 300, "(a) {}: all 300 reachable", v.label);
                assert_eq!(p.entries.matched, 300, "(b) {}", v.label);
                assert_eq!(
                    (
                        p.new_files,
                        p.overwritten_files,
                        p.kept_files,
                        p.identical_files
                    ),
                    (600, 300, 0, 100),
                    "(c) {}: exact counts",
                    v.label
                );
                assert_eq!(p.entries.total, 1000, "(c) {}: total", v.label);
            });
        },
    );
}

/// AC-M4-05-9: with a backup running on the destination, a restore (a) runs right away beside it
/// (non-exclusive lock) and (b) is a run of the kind `Restore`.
#[test]
fn ac_m4_05_9_restore_runs_beside_a_running_backup() {
    run_isolated_test(
        "ac_m4_05_9_restore_runs_beside_a_running_backup",
        "KESTIC_AC_M4_05_9_INNER",
        |root| {
            for_each_variant(root, "M4R9", small_tree, |fx, v| {
                let client = &fx.rig.daemon.client;
                fx.rig.wrapper.hold("backup");
                client
                    .start_backup(fx.backup.id.as_str())
                    .expect("StartBackup");
                fx.rig
                    .wrapper
                    .wait_for('S', "backup", 1, std::time::Duration::from_secs(30));
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        new_folder(&fx.new_dir("beside-backup")),
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                let detail = fx.wait_run(&run_id);
                assert_eq!(
                    detail.run.status,
                    RunStatus::Ok,
                    "(a) {}: {detail:?}",
                    v.label
                );
                assert!(
                    fx.rig.wrapper.of('E', "backup").is_empty(),
                    "(a) {}: the backup still runs while the restore ended",
                    v.label
                );
                assert!(
                    matches!(detail.run.kind, RunKind::Restore { .. }),
                    "(b) {}: {:?}",
                    v.label,
                    detail.run.kind
                );
                fx.rig.wrapper.release("backup");
            });
        },
    );
}

/// Sunday 2026-11-22 12:00:05 Europe/Zurich: the weekly `prune` of the destination is due.
const SUNDAY_PRUNE: time::OffsetDateTime = time::macros::datetime!(2026-11-22 11:00:05 UTC);

/// The fixture with the scheduler on (maintenance runs on its schedule) and the desktop fakes.
fn scheduled_fixture(root: &Path, variant: &ResticVariant, base: &str) -> RestoreFixture {
    let fx = RestoreFixture::start(root, variant, base, small_tree, |o| {
        o.enable_scheduler = true
    });
    // Maintenance becomes due only for slots after the baseline of the new destination.
    common::m3::wait_until("maintenance baseline recorded", || {
        kestic_core::history::RunHistory::open(&fx.rig.daemon.state_dir)
            .expect("open history")
            .get_destination_maintenance_state(&fx.dest.id)
            .expect("maintenance state")
            .baseline_at
            .is_some()
    });
    fx
}

/// Position of the first wrapper log entry of `kind` and `sub`.
fn log_position(fx: &RestoreFixture, kind: char, sub: &str) -> Option<usize> {
    fx.rig
        .wrapper
        .entries()
        .iter()
        .position(|e| e.kind == kind && e.sub == sub)
}

/// AC-M4-05-10: with maintenance (`prune`) running on the destination, a restore (a) is
/// `ausstehend` with the reason «queue» and (b) starts only after the maintenance.
#[test]
fn ac_m4_05_10_restore_waits_for_running_maintenance() {
    run_isolated_test(
        "ac_m4_05_10_restore_waits_for_running_maintenance",
        "KESTIC_AC_M4_05_10_INNER",
        |root| {
            let _fakes = common::m3::StdFakes::start();
            for variant in restic_variants() {
                let fx = scheduled_fixture(root, &variant, "M4R10");
                let v = &variant;
                fx.rig.wrapper.hold("prune");
                fx.rig.clock.set(SUNDAY_PRUNE);
                fx.rig
                    .wrapper
                    .wait_for('S', "prune", 1, std::time::Duration::from_secs(30));
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        new_folder(&fx.new_dir("after-maintenance")),
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                common::m3::settle();
                let waiting = fx
                    .rig
                    .daemon
                    .client
                    .get_run_detail(&run_id)
                    .expect("detail");
                assert_eq!(waiting.run.status, RunStatus::Pending, "(a) {}", v.label);
                assert_eq!(
                    waiting.run.pending_reason,
                    Some(kestic_core::model::PendingReason::Queued),
                    "(a) {}",
                    v.label
                );
                assert!(
                    fx.restore_calls().is_empty(),
                    "(b) {}: no restore while maintenance runs",
                    v.label
                );
                fx.rig.wrapper.release("prune");
                let detail = fx.wait_run(&run_id);
                assert_eq!(
                    detail.run.status,
                    RunStatus::Ok,
                    "(b) {}: {detail:?}",
                    v.label
                );
                let prune_end = log_position(&fx, 'E', "prune").expect("prune ended");
                let restore_start = log_position(&fx, 'S', "restore").expect("restore started");
                assert!(
                    restore_start > prune_end,
                    "(b) {}: restore after maintenance",
                    v.label
                );
                fx.rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-05-20: with a restore running on the destination, due maintenance (a) is `ausstehend`
/// with the reason «queue», (b) starts only after the restore and (c) no restic call ends with
/// exit 11.
#[test]
fn ac_m4_05_20_maintenance_waits_for_running_restore() {
    run_isolated_test(
        "ac_m4_05_20_maintenance_waits_for_running_restore",
        "KESTIC_AC_M4_05_20_INNER",
        |root| {
            let _fakes = common::m3::StdFakes::start();
            for variant in restic_variants() {
                let fx = scheduled_fixture(root, &variant, "M4R20");
                let v = &variant;
                let locks = fx.repo.join("locks");
                fx.rig.wrapper.hold_with_lock("restore", &locks);
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        new_folder(&fx.new_dir("before-maintenance")),
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                fx.rig
                    .wrapper
                    .wait_for('L', "restore", 1, std::time::Duration::from_secs(30));
                assert!(
                    find_lock_file(&locks).is_some(),
                    "{}: the restore holds its repository lock",
                    v.label
                );
                fx.rig.clock.set(SUNDAY_PRUNE);
                let state = common::m3::wait_for_state(
                    &fx.rig.daemon.client,
                    std::time::Duration::from_secs(15),
                    "maintenance waits",
                    |s| {
                        common::m3::dest_summary(s, &fx.dest.id).maintenance_pending_reason
                            == Some(kestic_i18n::codes::PendingReasonCode::Queue)
                    },
                );
                assert!(
                    !common::m3::dest_summary(&state, &fx.dest.id).maintenance_running,
                    "(a) {}: maintenance waits",
                    v.label
                );
                common::m3::settle();
                assert!(
                    fx.rig.wrapper.of('S', "prune").is_empty(),
                    "(b) {}: no prune while the restore runs",
                    v.label
                );
                fx.rig.wrapper.release("restore");
                assert_eq!(
                    fx.wait_run(&run_id).run.status,
                    RunStatus::Ok,
                    "{}",
                    v.label
                );
                fx.rig
                    .wrapper
                    .wait_for('S', "prune", 1, std::time::Duration::from_secs(30));
                let restore_end = log_position(&fx, 'E', "restore").expect("restore ended");
                let prune_start = log_position(&fx, 'S', "prune").expect("prune started");
                assert!(
                    prune_start > restore_end,
                    "(b) {}: prune after restore",
                    v.label
                );
                let exit_11 = fx
                    .rig
                    .wrapper
                    .entries()
                    .into_iter()
                    .filter(|e| e.kind == 'E' && e.detail.trim() == "11")
                    .count();
                assert_eq!(exit_11, 0, "(c) {}: no exit 11", v.label);
                fx.rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-05-13 (integration part, temp home): the preview suggests
/// `~/Restored/<backup> <snapshot time>` and (e) the daemon creates `~/Restored` only with
/// «Restore Now» (`StartRestore`).
#[test]
fn ac_m4_05_13_restored_folder_is_created_only_by_start_restore() {
    run_isolated_test(
        "ac_m4_05_13_restored_folder_is_created_only_by_start_restore",
        "KESTIC_AC_M4_05_13_INNER",
        |root| {
            for_each_variant(root, "M4R13", small_tree, |fx, v| {
                let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
                let parent = home.join("Restored");
                let (_, p) = fx.preview(&["a.txt"], None, RestoreOverwrite::Never);
                assert_eq!(
                    Path::new(&p.suggestion.new_folder),
                    parent.join("Home 2026-11-15 13-00"),
                    "(a) {}: name from backup and local snapshot time",
                    v.label
                );
                assert!(
                    !parent.exists(),
                    "(e) {}: not created by the preview",
                    v.label
                );
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        p.target.target.clone(),
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
                assert!(
                    Path::new(&p.suggestion.new_folder).join("a.txt").is_file(),
                    "(e) {}: created by StartRestore",
                    v.label
                );
                fs::remove_dir_all(&parent).expect("clean up for the next variant");
            });
        },
    );
}

/// AC-M4-05-12 (on the daemon): a restore from a snapshot with `kestic:b1`, started with the
/// dialog's preview, (c) carries `b1` from its creation on and (d) shows in the runs of `b1`
/// and of the destination.
#[test]
fn ac_m4_05_12_restore_run_carries_its_backup_from_the_start() {
    run_isolated_test(
        "ac_m4_05_12_restore_run_carries_its_backup_from_the_start",
        "KESTIC_AC_M4_05_12_INNER",
        |root| {
            for_each_variant(root, "M4R12", small_tree, |fx, v| {
                fx.rig.wrapper.hold("restore");
                let (id, p) = fx.preview(&["a.txt"], None, RestoreOverwrite::Never);
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        p.target.target.clone(),
                        RestoreOverwrite::Never,
                        Some(&id),
                    )
                    .expect("StartRestore");
                let first = fx
                    .rig
                    .daemon
                    .client
                    .get_run_detail(&run_id)
                    .expect("detail");
                assert_eq!(
                    first.run.backup_id(),
                    Some(&fx.backup.id),
                    "(c) {}: backup from the start",
                    v.label
                );
                fx.rig.wrapper.release("restore");
                assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
                let state = fx.rig.daemon.client.get_state().expect("GetState");
                let of_backup = state
                    .recent_runs
                    .iter()
                    .filter(|r| r.backup_id() == Some(&fx.backup.id))
                    .any(|r| r.id.as_str() == run_id);
                let of_dest = state
                    .recent_runs
                    .iter()
                    .filter(|r| r.destination_id == fx.dest.id)
                    .any(|r| r.id.as_str() == run_id);
                assert!(of_backup, "(d) {}: in the runs of the backup", v.label);
                assert!(of_dest, "(d) {}: in the runs of the destination", v.label);
            });
        },
    );
}

/// A restore waiting for maintenance can be cancelled: it ends `cancelled` and never starts.
#[test]
fn m4_s05_waiting_restore_can_be_cancelled() {
    run_isolated_test(
        "m4_s05_waiting_restore_can_be_cancelled",
        "KESTIC_M4_S05_CANCEL_WAITING_INNER",
        |root| {
            let _fakes = common::m3::StdFakes::start();
            let variant = common::m3::latest_variant();
            let fx = scheduled_fixture(root, &variant, "M4RC");
            fx.rig.wrapper.hold("prune");
            fx.rig.clock.set(SUNDAY_PRUNE);
            fx.rig
                .wrapper
                .wait_for('S', "prune", 1, std::time::Duration::from_secs(30));
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&fx.new_dir("cancelled")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            fx.rig.daemon.client.cancel_run(&run_id).expect("CancelRun");
            assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Cancelled);
            fx.rig.wrapper.release("prune");
            fx.rig
                .wrapper
                .wait_for('E', "prune", 1, std::time::Duration::from_secs(30));
            common::m3::settle();
            assert!(
                fx.restore_calls().is_empty(),
                "the cancelled restore never starts"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// The repository password never shows in the run of a restore, also when restic prints it.
#[test]
fn m4_s05_restore_never_shows_the_password() {
    run_isolated_test(
        "m4_s05_restore_never_shows_the_password",
        "KESTIC_M4_S05_REDACT_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RX", small_tree, |_| {});
            fx.rig
                .wrapper
                .fail("restore", 1, &format!("Fatal: echo {} failed", fx.password));
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&fx.new_dir("redacted")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            let detail = fx.wait_run(&run_id);
            assert_eq!(detail.run.status, RunStatus::Error, "{detail:?}");
            let shown = format!(
                "{:?} {:?} {:?}",
                detail.log_lines, detail.error_raw_detail, detail.run.summary_message
            );
            assert!(
                !shown.contains(&fx.password),
                "password in the run: {shown}"
            );
            assert!(
                detail
                    .log_lines
                    .iter()
                    .any(|l| l.contains("***REDACTED***")),
                "the restic error is in the log, redacted: {:?}",
                detail.log_lines
            );
            fx.rig.daemon.stop();
        },
    );
}

/// A backup that ends while a restore runs leaves its `forget` (exclusive lock) for later: no
/// restic call ends with exit 11, the backup is not a warning, and `forget` stays pending.
#[test]
fn m4_s05_backup_retention_waits_for_a_running_restore() {
    run_isolated_test(
        "m4_s05_backup_retention_waits_for_a_running_restore",
        "KESTIC_M4_S05_FORGET_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RF", small_tree, |_| {});
            let locks = fx.repo.join("locks");
            fx.rig.wrapper.hold_with_lock("restore", &locks);
            let restore_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&fx.new_dir("during-backup")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            fx.rig
                .wrapper
                .wait_for('L', "restore", 1, std::time::Duration::from_secs(30));
            assert!(
                find_lock_file(&locks).is_some(),
                "the restore holds its repository lock"
            );
            let backup_run = fx
                .rig
                .daemon
                .client
                .start_backup(fx.backup.id.as_str())
                .expect("StartBackup");
            let backup = fx.wait_run(&backup_run);
            assert_eq!(backup.run.status, RunStatus::Ok, "{:?}", backup.run.status);
            assert!(
                fx.rig.wrapper.of('S', "forget").is_empty(),
                "no forget beside the restore"
            );
            let state = fx.rig.daemon.client.get_state().expect("GetState");
            let summary = common::m3::backup_summary(&state, &fx.backup.id);
            assert!(summary.pending_forget, "retention stays pending");
            fx.rig.wrapper.release("restore");
            assert_eq!(fx.wait_run(&restore_id).run.status, RunStatus::Ok);
            let exit_11 = fx
                .rig
                .wrapper
                .entries()
                .into_iter()
                .filter(|e| e.kind == 'E' && e.detail.trim() == "11")
                .count();
            assert_eq!(exit_11, 0, "no exit 11");
            fx.rig.daemon.stop();
        },
    );
}

/// With `never`, restic still puts a folder of the snapshot in place of a file at the target;
/// the preview counts that file as overwritten instead of reporting nothing.
#[test]
fn m4_s05_folder_replacing_a_file_counts_as_overwritten() {
    run_isolated_test(
        "m4_s05_folder_replacing_a_file_counts_as_overwritten",
        "KESTIC_M4_S05_TYPE_CONFLICT_INNER",
        |root| {
            for_each_variant(root, "M4RT", small_tree, |fx, v| {
                fs::remove_dir(fx.src.join("empty")).expect("remove empty folder");
                fs::write(fx.src.join("empty"), "precious").expect("file in its place");
                let (_, p) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Never);
                assert_eq!(
                    outcomes(&p).get("empty"),
                    Some(&RestoreOutcome::Overwritten),
                    "{}: the file is replaced: {p:?}",
                    v.label
                );
                assert!(p.overwritten_files >= 1, "{}: counted", v.label);
                assert!(
                    !p.notes.contains(
                        &kestic_daemon::dbus_api::RestoreNoteCode::NothingSelectedMatches
                    ),
                    "{}: something matches",
                    v.label
                );
            });
        },
    );
}

/// A selected name that is not valid UTF-8 (restic shows U+FFFD) is refused before restic runs
/// instead of being skipped silently.
#[test]
fn m4_s05_name_restic_cannot_match_is_refused() {
    run_isolated_test(
        "m4_s05_name_restic_cannot_match_is_refused",
        "KESTIC_M4_S05_NON_UTF8_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RU", small_tree, |_| {});
            let started = fx.rig.wrapper.started_repo_count();
            let err = fx
                .start_restore(
                    &["a.txt", "bad\u{FFFD}name.txt"],
                    new_folder(&fx.new_dir("non-utf8")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect_err("refused");
            assert_eq!(
                err.code(),
                kestic_i18n::codes::ServiceErrorCode::InvalidInput
            );
            assert_eq!(
                fx.rig.wrapper.started_repo_count(),
                started,
                "no restic call"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// A restore to the original location that waits for maintenance longer than ten minutes still
/// starts after it: the age of its preview counted when it was accepted.
#[test]
fn m4_s05_waiting_restore_keeps_its_preview() {
    run_isolated_test(
        "m4_s05_waiting_restore_keeps_its_preview",
        "KESTIC_M4_S05_WAIT_LONG_INNER",
        |root| {
            let _fakes = common::m3::StdFakes::start();
            let variant = common::m3::latest_variant();
            let fx = scheduled_fixture(root, &variant, "M4RW");
            fs::write(fx.src.join("a.txt"), "local").expect("change a.txt");
            fx.rig.wrapper.hold("prune");
            fx.rig.clock.set(SUNDAY_PRUNE);
            fx.rig
                .wrapper
                .wait_for('S', "prune", 1, std::time::Duration::from_secs(30));
            let (preview_id, _) =
                fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    fx.original(),
                    RestoreOverwrite::Always,
                    Some(&preview_id),
                )
                .expect("StartRestore");
            fx.rig.clock.advance(std::time::Duration::from_mins(11));
            // Another query meanwhile frees the old preview from the query store.
            let other = fx.start_preview(&["b.bin"], None, RestoreOverwrite::Never);
            fx.preview_page(&other, &big_page());
            fx.rig.wrapper.release("prune");
            let detail = fx.wait_run(&run_id);
            assert_eq!(
                detail.run.status,
                RunStatus::Ok,
                "{:?}",
                detail.run.summary_message
            );
            assert_eq!(
                fs::read_to_string(fx.src.join("a.txt")).expect("a.txt"),
                "hello",
                "restored after the maintenance"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// AC-M4-05-12 (d) without side effects: a restore of a snapshot of `b1` shows in b1's history,
/// but a failed restore never becomes b1's last run nor gives b1 an error.
#[test]
fn m4_s05_failed_restore_leaves_the_backup_status_alone() {
    run_isolated_test(
        "m4_s05_failed_restore_leaves_the_backup_status_alone",
        "KESTIC_M4_S05_BACKUP_STATUS_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RS", small_tree, |_| {});
            let client = &fx.rig.daemon.client;
            let backup_run = client
                .start_backup(fx.backup.id.as_str())
                .expect("StartBackup");
            assert_eq!(fx.wait_run(&backup_run).run.status, RunStatus::Ok);
            let (preview_id, p) = fx.preview(&["a.txt"], None, RestoreOverwrite::Never);
            fx.rig.wrapper.fail("restore", 1, "Fatal: boom");
            let restore_run = fx
                .start_restore(
                    &["a.txt"],
                    p.target.target.clone(),
                    RestoreOverwrite::Never,
                    Some(&preview_id),
                )
                .expect("StartRestore");
            let restore = fx.wait_run(&restore_run);
            assert_eq!(restore.run.status, RunStatus::Error);
            assert_eq!(
                restore.run.backup_id(),
                Some(&fx.backup.id),
                "the restore belongs to b1"
            );
            let state = client.get_state().expect("GetState");
            let summary = common::m3::backup_summary(&state, &fx.backup.id);
            assert_eq!(
                summary.last_run.as_ref().map(|r| r.id.as_str().to_string()),
                Some(backup_run),
                "the last run of the backup stays its backup run"
            );
            assert!(!summary.has_error, "a failed restore is no backup error");
            assert_eq!(summary.error_kind, None, "no error kind from the restore");
            fx.rig.daemon.stop();
        },
    );
}

/// A symbolic link at the target where the snapshot has a folder is replaced by a real folder,
/// also with `never`: the preview counts it as overwritten, adds the note `symlink_in_target`,
/// and does not judge the files behind the link.
#[test]
fn m4_s05_symlink_replaced_by_folder_counts_as_overwritten() {
    run_isolated_test(
        "m4_s05_symlink_replaced_by_folder_counts_as_overwritten",
        "KESTIC_M4_S05_SYMLINK_DIR_INNER",
        |root| {
            for_each_variant(root, "M4RL", small_tree, |fx, v| {
                let elsewhere = fx.new_dir("elsewhere");
                fs::create_dir_all(&elsewhere).expect("mkdir elsewhere");
                fs::write(elsewhere.join("x.txt"), "x").expect("same content behind the link");
                fs::remove_dir_all(fx.src.join("docs")).expect("remove docs");
                std::os::unix::fs::symlink(&elsewhere, fx.src.join("docs")).expect("symlink");
                let (_, p) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Never);
                let o = outcomes(&p);
                assert_eq!(
                    o.get("docs"),
                    Some(&RestoreOutcome::Overwritten),
                    "{}: {p:?}",
                    v.label
                );
                assert_eq!(
                    o.get("x.txt"),
                    Some(&RestoreOutcome::New),
                    "{}: not judged through the link",
                    v.label
                );
                assert!(
                    p.notes
                        .contains(&kestic_daemon::dbus_api::RestoreNoteCode::SymlinkInTarget),
                    "{}: note",
                    v.label
                );
            });
        },
    );
}

/// `/` is never a restore target, neither as new folder nor as original location; it is
/// refused before restic runs.
#[test]
fn m4_s05_root_is_never_a_target() {
    run_isolated_test(
        "m4_s05_root_is_never_a_target",
        "KESTIC_M4_S05_ROOT_TARGET_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RR", small_tree, |_| {});
            let started = fx.rig.wrapper.started_repo_count();
            for target in [
                RestoreTarget::NewFolder { dir: "/".into() },
                RestoreTarget::Original { dir: "/".into() },
            ] {
                let err = fx
                    .start_restore(&["a.txt"], target.clone(), RestoreOverwrite::Never, None)
                    .expect_err("refused");
                assert_eq!(
                    err.code(),
                    kestic_i18n::codes::ServiceErrorCode::InvalidInput
                );
                let query = fx.rig.daemon.client.start_snapshot_query(
                    &kestic_daemon::dbus_api::SnapshotQueryRequest::restore_preview(
                        fx.dest.id.as_str(),
                        &fx.snapshot_id,
                        &[fx.path("a.txt")],
                        Some(target),
                        RestoreOverwrite::Never,
                    ),
                );
                assert_eq!(
                    query.err().map(|e| e.code()),
                    Some(kestic_i18n::codes::ServiceErrorCode::InvalidInput)
                );
            }
            assert_eq!(
                fx.rig.wrapper.started_repo_count(),
                started,
                "no restic call"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// A private folder stays private: the new folder Kestic creates for a restore has no
/// permissions for group and others.
#[test]
fn m4_s05_new_folder_is_private() {
    run_isolated_test(
        "m4_s05_new_folder_is_private",
        "KESTIC_M4_S05_PRIVATE_INNER",
        |root| {
            use std::os::unix::fs::PermissionsExt;
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RP", small_tree, |_| {});
            let target = fx.new_dir("private");
            let run_id = fx
                .start_restore(
                    &["docs"],
                    new_folder(&target),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
            let mode = fs::metadata(&target).expect("target").permissions().mode();
            assert_eq!(mode & 0o077, 0, "the new folder is private: {mode:o}");
            fx.rig.daemon.stop();
        },
    );
}

/// A folder with content in the snapshot where the target has a file: the dry run of «Never»
/// cannot look below the file, but the preview still ends and counts the file as overwritten
/// and the entries below as new.
#[test]
fn m4_s05_folder_with_content_over_a_file_previews() {
    run_isolated_test(
        "m4_s05_folder_with_content_over_a_file_previews",
        "KESTIC_M4_S05_DIR_OVER_FILE_INNER",
        |root| {
            for_each_variant(root, "M4RD", small_tree, |fx, v| {
                fs::remove_dir_all(fx.src.join("docs")).expect("remove docs");
                fs::write(fx.src.join("docs"), "a file now").expect("file in its place");
                for mode in [RestoreOverwrite::Never, RestoreOverwrite::IfNewer] {
                    let (_, p) = fx.preview(&[], Some(fx.original()), mode);
                    let o = outcomes(&p);
                    assert_eq!(
                        o.get("docs"),
                        Some(&RestoreOutcome::Overwritten),
                        "{} {mode:?}",
                        v.label
                    );
                    assert_eq!(
                        o.get("x.txt"),
                        Some(&RestoreOutcome::New),
                        "{} {mode:?}",
                        v.label
                    );
                }
            });
        },
    );
}

/// A folder with content at the target where the snapshot has a file: where restic would write
/// the file (`always`, `if-changed`) the run fails and gives the folder the file's mode, so
/// (a) the preview refuses the target; with `never` restic leaves the folder alone, so (b) the
/// preview counts the file as kept and (c) the restore ends `ok` with (d) the folder, its mode
/// and its content unchanged.
#[test]
fn m4_s05_file_over_a_folder_with_content_is_refused() {
    run_isolated_test(
        "m4_s05_file_over_a_folder_with_content_is_refused",
        "KESTIC_M4_S05_FILE_OVER_DIR_INNER",
        |root| {
            use std::os::unix::fs::PermissionsExt;
            for_each_variant(root, "M4RB", small_tree, |fx, v| {
                fs::remove_file(fx.src.join("a.txt")).expect("remove a.txt");
                fs::create_dir_all(fx.src.join("a.txt/inner")).expect("folder in its place");
                fs::write(fx.src.join("a.txt/inner/f"), "keep").expect("content");
                let mode = || {
                    fs::metadata(fx.src.join("a.txt"))
                        .expect("folder")
                        .permissions()
                        .mode()
                };
                let before = mode();
                for overwrite in [RestoreOverwrite::Always, RestoreOverwrite::IfChanged] {
                    let id = fx.start_preview(&["a.txt"], Some(fx.original()), overwrite);
                    let st = fx.wait_query(&id, &big_page());
                    assert_eq!(
                        st.state, "error",
                        "(a) {} {overwrite:?}: refused: {st:?}",
                        v.label
                    );
                }
                let (id, p) = fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Never);
                assert_eq!(
                    outcomes(&p).get("a.txt"),
                    Some(&RestoreOutcome::Kept),
                    "(b) {}",
                    v.label
                );
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        fx.original(),
                        RestoreOverwrite::Never,
                        Some(&id),
                    )
                    .expect("StartRestore");
                let detail = fx.wait_run(&run_id);
                assert_eq!(
                    detail.run.status,
                    RunStatus::Ok,
                    "(c) {}: {detail:?}",
                    v.label
                );
                assert_eq!(mode(), before, "(d) {}: the folder keeps its mode", v.label);
                assert_eq!(
                    fs::read_to_string(fx.src.join("a.txt/inner/f")).expect("content"),
                    "keep",
                    "(d) {}: content kept",
                    v.label
                );
            });
        },
    );
}

/// Files `a\xff`, `a\xfe` and `b\xff`: names that are not UTF-8, which restic shows alike.
fn unmatchable_tree(work: &Path) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let src = work.join("src");
    fs::create_dir_all(&src).expect("mkdir src");
    for (name, content) in [
        (&b"a\xff"[..], "one"),
        (&b"a\xfe"[..], "two"),
        (&b"b\xff"[..], "file"),
    ] {
        fs::write(src.join(std::ffi::OsStr::from_bytes(name)), content).expect("write");
    }
    src
}

/// Names that restic shows alike (bytes that are not UTF-8 become U+FFFD) are still counted one
/// by one and found at the target, so a folder with content where the snapshot has such a file
/// is refused like any other.
#[test]
fn m4_s05_names_shown_alike_are_counted_and_checked_at_the_target() {
    run_isolated_test(
        "m4_s05_names_shown_alike_are_counted_and_checked_at_the_target",
        "KESTIC_M4_S05_SHOWN_ALIKE_INNER",
        |root| {
            use std::os::unix::ffi::OsStrExt;
            use std::os::unix::fs::PermissionsExt;
            let real =
                |fx: &RestoreFixture, name: &[u8]| fx.src.join(std::ffi::OsStr::from_bytes(name));
            for_each_variant(root, "M4RX", unmatchable_tree, |fx, v| {
                fs::write(real(fx, b"a\xff"), "local").expect("change a\\xff");
                fs::remove_file(real(fx, b"a\xfe")).expect("remove a\\xfe");
                let (_, p) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Always);
                assert_eq!(p.new_files, 1, "{}: a\\xfe is new", v.label);
                assert_eq!(p.overwritten_files, 1, "{}: a\\xff is overwritten", v.label);
                assert_eq!(p.identical_files, 1, "{}: b\\xff is unchanged", v.label);
                assert_eq!(p.entries.total, 3, "{}: three files listed", v.label);

                let folder = real(fx, b"b\xff");
                fs::remove_file(&folder).expect("remove b\\xff");
                fs::create_dir_all(folder.join("inner")).expect("folder in its place");
                fs::write(folder.join("inner/f"), "keep").expect("content");
                let mode = fs::metadata(&folder).expect("folder").permissions().mode();
                let id = fx.start_preview(&[], Some(fx.original()), RestoreOverwrite::Always);
                let st = fx.wait_query(&id, &big_page());
                assert_eq!(st.state, "error", "{}: refused: {st:?}", v.label);
                assert_eq!(
                    fs::metadata(&folder).expect("folder").permissions().mode(),
                    mode,
                    "{}: the folder keeps its mode",
                    v.label
                );
                assert!(
                    folder.join("inner/f").is_file(),
                    "{}: content kept",
                    v.label
                );
            });
        },
    );
}

/// A folder with a file named «not a directory».
fn not_a_directory_tree(work: &Path) -> std::path::PathBuf {
    let src = work.join("src");
    fs::create_dir_all(src.join("locked")).expect("mkdir locked");
    fs::write(src.join("locked/not a directory"), "x").expect("write");
    src
}

/// A dry run that fails for another reason is an error, also when the name of the file reads
/// like the failure the preview tolerates («not a directory»).
#[test]
fn m4_s05_dry_run_error_is_not_mistaken_by_the_file_name() {
    run_isolated_test(
        "m4_s05_dry_run_error_is_not_mistaken_by_the_file_name",
        "KESTIC_M4_S05_NOT_A_DIR_NAME_INNER",
        |root| {
            use std::os::unix::fs::PermissionsExt;
            for_each_variant(root, "M4RN", not_a_directory_tree, |fx, v| {
                let locked = fx.src.join("locked");
                fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
                let id = fx.start_preview(&[], Some(fx.original()), RestoreOverwrite::Never);
                let st = fx.wait_query(&id, &big_page());
                fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
                assert_eq!(st.state, "error", "{}: not hidden: {st:?}", v.label);
            });
        },
    );
}

/// The restore run is a restic run with low CPU and IO priority (`nice` 10, IO class idle),
/// like backups and maintenance.
#[test]
fn m4_s05_restore_run_has_low_priority() {
    run_isolated_test(
        "m4_s05_restore_run_has_low_priority",
        "KESTIC_M4_S05_LOW_PRIORITY_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RP", small_tree, |_| {});
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&fx.new_dir("priority")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            let detail = fx.wait_run(&run_id);
            let command = detail.run.commands.last().expect("restore command");
            let invocation =
                kestic_core::restic::parse_recorded_invocation(&command.executable, &command.args);
            assert_eq!(
                invocation.subcommand_args.first().map(String::as_str),
                Some("restore"),
                "the restore call: {invocation:?}"
            );
            assert!(invocation.low_priority, "low priority: {invocation:?}");
            fx.rig.daemon.stop();
        },
    );
}

/// A target inside the repository of any local destination is refused, not only inside the one
/// the snapshot comes from.
#[test]
fn m4_s05_target_inside_another_local_repository_is_refused() {
    run_isolated_test(
        "m4_s05_target_inside_another_local_repository_is_refused",
        "KESTIC_M4_S05_OTHER_REPO_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RO", small_tree, |_| {});
            let other = root.join("repo-other");
            common::m3::add_local_destination(
                &fx.rig.daemon.client,
                &variant.bin,
                &other,
                "Other",
                |_| {},
            );
            let (_, p) = fx.preview(
                &["a.txt"],
                Some(new_folder(&other.join("restored-here"))),
                RestoreOverwrite::Never,
            );
            assert_eq!(
                p.target.check.code,
                RestoreTargetCode::InsideRepository,
                "{p:?}"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// A restore stopped because the system goes to sleep is `unterbrochen`, not cancelled by the
/// user, like backups and maintenance.
#[test]
fn m4_s05_restore_stopped_by_sleep_is_interrupted() {
    run_isolated_test(
        "m4_s05_restore_stopped_by_sleep_is_interrupted",
        "KESTIC_M4_S05_SLEEP_INNER",
        |root| {
            let _fakes = common::m3::StdFakes::start();
            let logind = common::fakes::FakeLogindHandle::start();
            let _portal = common::fakes::FakePortalInhibitHandle::start();
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RZ", small_tree, |_| {});
            fx.rig.wrapper.hold("restore");
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&fx.new_dir("sleep")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            fx.rig
                .wrapper
                .wait_for('S', "restore", 1, std::time::Duration::from_secs(15));
            logind.emit_prepare_for_sleep(true);
            let detail = fx.wait_run(&run_id);
            fx.rig.wrapper.release("restore");
            assert_eq!(detail.run.status, RunStatus::Interrupted, "{detail:?}");
            fx.rig.daemon.stop();
        },
    );
}

/// A target folder whose real path (behind a symbolic link) is not UTF-8 is refused: restic
/// takes the target as text and would write into another folder.
#[test]
fn m4_s05_target_with_a_real_path_that_is_not_utf8_is_refused() {
    run_isolated_test(
        "m4_s05_target_with_a_real_path_that_is_not_utf8_is_refused",
        "KESTIC_M4_S05_TARGET_NON_UTF8_INNER",
        |root| {
            use std::os::unix::ffi::OsStrExt;
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RT", small_tree, |_| {});
            let base = fx.new_dir("latin1");
            fs::create_dir_all(&base).expect("mkdir base");
            let real = base.join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
            fs::create_dir(&real).expect("mkdir caf\\xe9");
            let link = base.join("link");
            std::os::unix::fs::symlink(&real, &link).expect("symlink");
            let id = fx.start_preview(&["a.txt"], Some(new_folder(&link)), RestoreOverwrite::Never);
            let st = fx.wait_query(&id, &big_page());
            assert_eq!(st.state, "error", "preview refused: {st:?}");
            let err = fx
                .start_restore(&["a.txt"], new_folder(&link), RestoreOverwrite::Never, None)
                .expect_err("StartRestore refused");
            assert_eq!(
                err.code(),
                kestic_i18n::codes::ServiceErrorCode::InvalidInput
            );
            assert!(
                !base.join("caf\u{FFFD}").exists(),
                "no other folder was written"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// A restore waiting for maintenance does not start while the system goes to sleep (the
/// maintenance ends then); it starts after waking up.
#[test]
fn m4_s05_waiting_restore_starts_after_waking_up() {
    run_isolated_test(
        "m4_s05_waiting_restore_starts_after_waking_up",
        "KESTIC_M4_S05_WAKE_INNER",
        |root| {
            let _fakes = common::m3::StdFakes::start();
            let logind = common::fakes::FakeLogindHandle::start();
            let _portal = common::fakes::FakePortalInhibitHandle::start();
            let variant = common::m3::latest_variant();
            let fx = scheduled_fixture(root, &variant, "M4RK");
            fx.rig.wrapper.hold("prune");
            fx.rig.clock.set(SUNDAY_PRUNE);
            fx.rig
                .wrapper
                .wait_for('S', "prune", 1, std::time::Duration::from_secs(30));
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&fx.new_dir("after-wake")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            common::m3::settle();
            logind.emit_prepare_for_sleep(true);
            fx.rig
                .wrapper
                .wait_for('E', "prune", 1, std::time::Duration::from_secs(30));
            common::m3::settle();
            assert!(
                fx.restore_calls().is_empty(),
                "no restore while the system goes to sleep"
            );
            fx.rig.wrapper.release("prune");
            logind.emit_prepare_for_sleep(false);
            let detail = fx.wait_run(&run_id);
            assert_eq!(detail.run.status, RunStatus::Ok, "{detail:?}");
            fx.rig.daemon.stop();
        },
    );
}

/// Backs up the fixture's folder again after a second local destination was set up in its
/// subfolder `vault`, and returns the ID of that snapshot (it holds the repository's files).
fn snapshot_with_a_repository(fx: &RestoreFixture, variant: &ResticVariant) -> String {
    common::m3::add_local_destination(
        &fx.rig.daemon.client,
        &variant.bin,
        &fx.src.join("vault"),
        "Vault",
        |_| {},
    );
    common::m4::backup_dir(
        &variant.bin,
        &fx.repo,
        &fx.password,
        &fx.src,
        &common::m4::RawSnapshot {
            tag: &format!("kestic:{}", fx.backup.id),
            time: "2026-11-15 13:00:00",
            host: "laptop",
        },
    )
}

/// AC-M4-05-15: a target that holds the repository of a local destination below it is refused
/// with `inside_repository` when (i) the selected entries reach that repository (no dry run, no
/// restore, `StartRestore` refused); it restores as usual when (j) the selection lies outside
/// the repository or (k) the snapshot has no entries there.
#[test]
fn ac_m4_05_15_target_holding_a_repository_reached_by_the_selection_is_refused() {
    run_isolated_test(
        "ac_m4_05_15_target_holding_a_repository_reached_by_the_selection_is_refused",
        "KESTIC_AC_M4_05_15_REPO_BELOW_INNER",
        |root| {
            for variant in restic_variants() {
                let mut fx = RestoreFixture::start(root, &variant, "M4RH", small_tree, |_| {});
                let v = &variant;
                let before = fx.snapshot_id.clone();
                fx.snapshot_id = snapshot_with_a_repository(&fx, v);

                let (id, p) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Always);
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::InsideRepository,
                    "(i) {}: {p:?}",
                    v.label
                );
                let err = fx
                    .start_restore(&[], fx.original(), RestoreOverwrite::Always, Some(&id))
                    .expect_err("(i) StartRestore refused");
                assert_eq!(
                    err.code(),
                    kestic_i18n::codes::ServiceErrorCode::InvalidInput,
                    "(i) {}",
                    v.label
                );
                assert_eq!(
                    fx.restore_calls(),
                    Vec::<String>::new(),
                    "(i) {}: no dry run and no restore",
                    v.label
                );
                let (_, p) = fx.preview(
                    &["a.txt", "vault/config"],
                    Some(new_folder(&fx.src)),
                    RestoreOverwrite::Never,
                );
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::InsideRepository,
                    "(i) {}: a selected entry inside the repository",
                    v.label
                );

                let (_, p) = fx.preview(
                    &["a.txt", "docs"],
                    Some(fx.original()),
                    RestoreOverwrite::Never,
                );
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::ExistsNotEmpty,
                    "(j) {}: selection outside the repository",
                    v.label
                );
                assert_eq!(p.identical_files, 3, "(j) {}: dry run ran", v.label);

                fx.snapshot_id = before;
                let (_, p) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Never);
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::ExistsNotEmpty,
                    "(k) {}: snapshot without the repository",
                    v.label
                );
                fx.rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-05-16: a preview where (f) a folder of the snapshot already exists at the target has
/// the note `folder_metadata_reset` (restic sets the folder's mode from the snapshot, even with
/// `never`), and one where (g) no folder of the snapshot exists there has not.
#[test]
fn ac_m4_05_16_existing_folders_get_the_snapshot_metadata() {
    run_isolated_test(
        "ac_m4_05_16_existing_folders_get_the_snapshot_metadata",
        "KESTIC_AC_M4_05_16_FOLDERS_INNER",
        |root| {
            use kestic_daemon::dbus_api::RestoreNoteCode;
            use std::os::unix::fs::PermissionsExt;
            for_each_variant(root, "M4RM", small_tree, |fx, v| {
                let docs = fx.src.join("docs");
                set_mode(&docs, 0o700);
                let (_, p) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Never);
                assert!(
                    p.notes.contains(&RestoreNoteCode::FolderMetadataReset),
                    "(f) {}: {:?}",
                    v.label,
                    p.notes
                );
                let (id, _) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Never);
                let run_id = fx
                    .start_restore(&[], fx.original(), RestoreOverwrite::Never, Some(&id))
                    .expect("StartRestore");
                fx.wait_run(&run_id);
                assert_ne!(
                    fs::metadata(&docs).expect("docs").permissions().mode() & 0o777,
                    0o700,
                    "(f) {}: restic set the folder's mode from the snapshot",
                    v.label
                );

                let other = fx.new_dir("only-a-file");
                fs::create_dir_all(&other).expect("mkdir");
                fs::write(other.join("other.txt"), "x").expect("write other");
                let (_, p) = fx.preview(&[], Some(new_folder(&other)), RestoreOverwrite::Never);
                assert_eq!(
                    p.target.check.code,
                    RestoreTargetCode::ExistsNotEmpty,
                    "(g) {}",
                    v.label
                );
                assert!(
                    !p.notes.contains(&RestoreNoteCode::FolderMetadataReset),
                    "(g) {}: {:?}",
                    v.label,
                    p.notes
                );
            });
        },
    );
}

/// AC-M4-05-25: a preview that refuses its target ends with (a) `error_kind` `invalid_target`
/// and (b) `error_detail` the folder concerned, for a file over a folder with content and for a
/// target whose real path is not UTF-8; (c) its text is a translatable message naming that
/// folder, never English raw text.
#[test]
fn ac_m4_05_25_refused_target_has_its_own_error_kind() {
    run_isolated_test(
        "ac_m4_05_25_refused_target_has_its_own_error_kind",
        "KESTIC_AC_M4_05_25_INNER",
        |root| {
            use kestic_i18n::codes::ErrorKind;
            use kestic_i18n::messages::UiText;
            use std::os::unix::ffi::OsStrExt;
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RK", small_tree, |_| {});
            fs::remove_file(fx.src.join("a.txt")).expect("remove a.txt");
            fs::create_dir_all(fx.src.join("a.txt/inner")).expect("folder in its place");
            fs::write(fx.src.join("a.txt/inner/f"), "keep").expect("content");
            let id = fx.start_preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
            let st = fx.wait_query(&id, &big_page());
            assert_eq!(
                st.error_kind,
                Some(ErrorKind::InvalidTarget),
                "(a) file over a folder: {st:?}"
            );
            let blocked = fs::canonicalize(fx.src.join("a.txt")).expect("canonical");
            assert_eq!(
                st.error_detail.as_deref(),
                Some(blocked.to_str().expect("utf-8")),
                "(b) file over a folder"
            );
            match ErrorKind::InvalidTarget.text(st.error_detail.as_deref()) {
                UiText::Message(m) => assert_eq!(
                    m.args,
                    vec![blocked.to_string_lossy().into_owned()],
                    "(c) names the folder"
                ),
                UiText::Raw(raw) => panic!("(c) raw text: {raw}"),
            }

            let base = fx.new_dir("latin1");
            fs::create_dir_all(&base).expect("mkdir base");
            let real = base.join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
            fs::create_dir(&real).expect("mkdir caf\\xe9");
            let link = base.join("link");
            std::os::unix::fs::symlink(&real, &link).expect("symlink");
            let id = fx.start_preview(&["b.bin"], Some(new_folder(&link)), RestoreOverwrite::Never);
            let st = fx.wait_query(&id, &big_page());
            assert_eq!(
                st.error_kind,
                Some(ErrorKind::InvalidTarget),
                "(a) target not UTF-8: {st:?}"
            );
            let real = fs::canonicalize(&real).expect("canonical");
            assert_eq!(
                st.error_detail.as_deref(),
                Some(real.to_string_lossy().as_ref()),
                "(b) target not UTF-8"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// A restore that waited for maintenance and finds its target unusable when it would start ends
/// (a) with the target check's text in the daemon's language in the run history, not with a code
/// or English raw text, (b) with the error code `invalid_target` in its run detail, and (c) with
/// the notification that the restore ended.
#[test]
fn m4_s05_waiting_restore_names_why_its_target_is_unusable() {
    run_isolated_test(
        "m4_s05_waiting_restore_names_why_its_target_is_unusable",
        "KESTIC_M4_S05_WAITING_UNUSABLE_INNER",
        |root| {
            let fakes = common::m3::StdFakes::start();
            common::po_translator::set_daemon_language("de");
            let variant = common::m3::latest_variant();
            let fx = scheduled_fixture(root, &variant, "M4RU");
            fx.rig.wrapper.hold("prune");
            fx.rig.clock.set(SUNDAY_PRUNE);
            fx.rig
                .wrapper
                .wait_for('S', "prune", 1, std::time::Duration::from_secs(30));
            let target = fx.new_dir("becomes-a-file");
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&target),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            let resolved = fs::canonicalize(&target).expect("created by StartRestore");
            fs::remove_dir(&target).expect("remove the new folder");
            fs::write(&target, "a file now").expect("file in its place");
            fx.rig.wrapper.release("prune");
            let detail = fx.wait_run(&run_id);
            assert_eq!(detail.run.status, RunStatus::Error, "{detail:?}");
            assert_eq!(
                detail.run.summary_message.as_deref(),
                Some(format!("{} ist kein Ordner", resolved.display()).as_str()),
                "(a) the reason in the daemon's language"
            );
            assert_eq!(
                detail.error_kind,
                Some(kestic_i18n::codes::ErrorKind::InvalidTarget),
                "(b)"
            );
            let notes = common::m3::wait_for_notifications(
                &fakes.notif,
                "RestoreFinished",
                1,
                std::time::Duration::from_secs(5),
            );
            assert!(
                notes[0].body.contains("ist kein Ordner"),
                "(c) the notification says why: {notes:?}"
            );
            assert!(fx.restore_calls().is_empty(), "no restore ran");
            fx.rig.daemon.stop();
        },
    );
}

/// Real (not dry-run) `restic restore` calls so far.
fn real_restores(fx: &RestoreFixture) -> usize {
    fx.restore_calls()
        .iter()
        .filter(|c| !c.contains("--dry-run"))
        .count()
}

/// Waits until a `restic version` call is held: started, but not ended.
fn wait_for_held_version(fx: &RestoreFixture, ended_before: usize) {
    common::m3::wait_until("a held restic version call", || {
        fx.rig.wrapper.of('S', "version").len() > fx.rig.wrapper.of('E', "version").len()
            && fx.rig.wrapper.of('E', "version").len() == ended_before
    });
}

/// A second `StartRestore` into the target of a restore that (a) waits for maintenance or
/// (b) runs is refused with `AlreadyRunning`, so two restic runs never write the same files;
/// (c) the first restore still ends `ok`. While the first waits, a restore (d) into a folder
/// below its target and (e) into the folder holding it is refused the same way.
#[test]
fn m4_s05_second_restore_into_the_same_target_is_refused() {
    run_isolated_test(
        "m4_s05_second_restore_into_the_same_target_is_refused",
        "KESTIC_M4_S05_SAME_TARGET_INNER",
        |root| {
            let _fakes = common::m3::StdFakes::start();
            let variant = common::m3::latest_variant();
            let fx = scheduled_fixture(root, &variant, "M4RS");
            let target = new_folder(&fx.new_dir("twice"));
            fx.rig.wrapper.hold("prune");
            fx.rig.clock.set(SUNDAY_PRUNE);
            fx.rig
                .wrapper
                .wait_for('S', "prune", 1, std::time::Duration::from_secs(30));
            let run_id = fx
                .start_restore(&["a.txt"], target.clone(), RestoreOverwrite::Never, None)
                .expect("first StartRestore");
            let waiting =
                fx.start_restore(&["a.txt"], target.clone(), RestoreOverwrite::Never, None);
            assert!(
                matches!(waiting, Err(KesticDbusError::AlreadyRunning(_))),
                "(a) refused while the first waits: {waiting:?}"
            );
            let below = new_folder(&fx.new_dir("twice").join("below"));
            let nested = fx.start_restore(&["a.txt"], below, RestoreOverwrite::Never, None);
            assert!(
                matches!(nested, Err(KesticDbusError::AlreadyRunning(_))),
                "(d) refused below the waiting target: {nested:?}"
            );
            let holding = new_folder(fx.new_dir("twice").parent().expect("targets"));
            let (preview_id, _) =
                fx.preview(&["a.txt"], Some(holding.clone()), RestoreOverwrite::Never);
            let above = fx.start_restore(
                &["a.txt"],
                holding,
                RestoreOverwrite::Never,
                Some(&preview_id),
            );
            assert!(
                matches!(above, Err(KesticDbusError::AlreadyRunning(_))),
                "(e) refused above the waiting target: {above:?}"
            );
            fx.rig.wrapper.hold("restore");
            fx.rig.wrapper.release("prune");
            fx.rig
                .wrapper
                .wait_for('S', "restore", 1, std::time::Duration::from_secs(30));
            let running =
                fx.start_restore(&["b.bin"], target.clone(), RestoreOverwrite::Never, None);
            assert!(
                matches!(running, Err(KesticDbusError::AlreadyRunning(_))),
                "(b) refused while the first runs: {running:?}"
            );
            fx.rig.wrapper.release("restore");
            let detail = fx.wait_run(&run_id);
            assert_eq!(detail.run.status, RunStatus::Ok, "(c) {detail:?}");
            assert_eq!(real_restores(&fx), 1, "(c) one restic restore only");
            fx.rig.daemon.stop();
        },
    );
}

/// A waiting restore can be cancelled while it starts (its target is checked and restic is
/// probed after the maintenance): (a) `CancelRun` finds it, (b) it ends cancelled and (c)
/// restic never restores.
#[test]
fn m4_s05_waiting_restore_can_be_cancelled_while_it_starts() {
    run_isolated_test(
        "m4_s05_waiting_restore_can_be_cancelled_while_it_starts",
        "KESTIC_M4_S05_CANCEL_STARTING_INNER",
        |root| {
            let _fakes = common::m3::StdFakes::start();
            let variant = common::m3::latest_variant();
            let fx = scheduled_fixture(root, &variant, "M4RX");
            fx.rig.wrapper.hold("prune");
            fx.rig.clock.set(SUNDAY_PRUNE);
            fx.rig
                .wrapper
                .wait_for('S', "prune", 1, std::time::Duration::from_secs(30));
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&fx.new_dir("starting")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            let ended = fx.rig.wrapper.of('E', "version").len();
            fx.rig.wrapper.hold("version");
            fx.rig.wrapper.release("prune");
            wait_for_held_version(&fx, ended);
            let cancelled = fx.rig.daemon.client.cancel_run(&run_id);
            assert!(cancelled.is_ok(), "(a) {cancelled:?}");
            fx.rig.wrapper.release("version");
            assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Cancelled, "(b)");
            common::m3::settle();
            assert_eq!(
                real_restores(&fx),
                0,
                "(c) the cancelled restore never runs"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// The target is checked once more right before restic writes: an original location that
/// disappears while restic is probed (a) ends the run with `Fehler` and `invalid_target`, and
/// (b) is not created by restic.
#[test]
fn m4_s05_target_is_checked_again_right_before_the_run() {
    run_isolated_test(
        "m4_s05_target_is_checked_again_right_before_the_run",
        "KESTIC_M4_S05_RECHECK_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RY", small_tree, |_| {});
            let (preview_id, _) =
                fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Never);
            let ended = fx.rig.wrapper.of('E', "version").len();
            fx.rig.wrapper.hold("version");
            let request = RestoreRequest {
                destination_id: fx.dest.id.as_str().to_string(),
                snapshot_id: fx.snapshot_id.clone(),
                selection: vec![fx.path("a.txt")],
                target: fx.original(),
                overwrite: RestoreOverwrite::Never,
                preview_id: Some(preview_id),
            };
            let moved = fx.new_dir("moved-away");
            fs::create_dir_all(moved.parent().expect("parent")).expect("mkdir targets");
            let started = std::thread::scope(|s| {
                let call = s.spawn(|| fx.rig.daemon.client.start_restore(&request));
                wait_for_held_version(&fx, ended);
                fs::rename(&fx.src, &moved).expect("move the original folder away");
                fx.rig.wrapper.release("version");
                call.join().expect("StartRestore thread")
            });
            let run_id = started.expect("StartRestore accepted");
            let detail = fx.wait_run(&run_id);
            assert_eq!(detail.run.status, RunStatus::Error, "(a) {detail:?}");
            assert_eq!(
                detail.error_kind,
                Some(kestic_i18n::codes::ErrorKind::InvalidTarget),
                "(a)"
            );
            assert!(!fx.src.exists(), "(b) restic did not create the folder");
            assert_eq!(real_restores(&fx), 0, "(b) no restic restore ran");
            fx.rig.daemon.stop();
        },
    );
}

/// The walk of a preview reads only the selected entries: the recursive `restic ls` names
/// (a) the one selected file and (b) both selected entries of a selection of two, never their
/// common folder, and (c) the whole folder only when nothing is selected.
#[test]
fn m4_s05_preview_walks_only_the_selected_entries() {
    run_isolated_test(
        "m4_s05_preview_walks_only_the_selected_entries",
        "KESTIC_M4_S05_WALK_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RW", small_tree, |_| {});
            let walks = || -> Vec<String> {
                fx.rig
                    .wrapper
                    .of('S', "ls")
                    .into_iter()
                    .map(|e| e.detail)
                    .filter(|d| d.contains("--recursive"))
                    .collect()
            };
            let walked = |before: usize| -> Vec<String> {
                let all = walks();
                assert_eq!(all.len(), before + 1, "one walk per preview: {all:?}");
                all[before]
                    .split(' ')
                    .skip_while(|a| *a != fx.snapshot_id)
                    .skip(1)
                    .map(str::to_string)
                    .collect()
            };
            let before = walks().len();
            fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Never);
            assert_eq!(walked(before), vec![fx.path("a.txt")], "(a)");
            let before = walks().len();
            fx.preview(
                &["a.txt", "docs"],
                Some(fx.original()),
                RestoreOverwrite::Never,
            );
            assert_eq!(
                walked(before),
                vec![fx.path("a.txt"), fx.path("docs")],
                "(b)"
            );
            let before = walks().len();
            fx.preview(&[], Some(fx.original()), RestoreOverwrite::Never);
            assert_eq!(walked(before), vec![fx.path("")], "(c)");
            fx.rig.daemon.stop();
        },
    );
}

/// The new folder of a restore is created only after restic was probed and removed again
/// while it is empty: (a) a start that fails at the probe leaves no folder, (b) a cancelled
/// restore and (c) a restore that restores nothing leave no empty folder, (d) neither does the
/// folder holding it (`~/Restored`) that the restore created.
#[test]
fn m4_s05_new_folder_is_left_only_with_restored_files() {
    run_isolated_test(
        "m4_s05_new_folder_is_left_only_with_restored_files",
        "KESTIC_M4_S05_NO_EMPTY_FOLDER_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RE", small_tree, |_| {});
            let probe_failed = fx.new_dir("probe-failed");
            fx.rig.wrapper.fail("version", 1, "restic: broken");
            let refused = fx.start_restore(
                &["a.txt"],
                new_folder(&probe_failed),
                RestoreOverwrite::Never,
                None,
            );
            assert!(refused.is_err(), "(a) {refused:?}");
            assert!(!probe_failed.exists(), "(a) no folder after a failed probe");
            fx.rig.wrapper.clear_fail("version");

            let cancelled = fx.new_dir("cancelled");
            fx.rig.wrapper.hold("restore");
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&cancelled),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            fx.rig
                .wrapper
                .wait_for('S', "restore", 1, std::time::Duration::from_secs(30));
            fx.rig.daemon.client.cancel_run(&run_id).expect("CancelRun");
            assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Cancelled);
            fx.rig.wrapper.release("restore");
            assert!(!cancelled.exists(), "(b) no empty folder after a cancel");

            let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
            let holder = home.join("Restored");
            let (_, p) = fx.preview(&["empty"], None, RestoreOverwrite::Never);
            let run_id = fx
                .start_restore(
                    &["empty"],
                    p.target.target.clone(),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            let detail = fx.wait_run(&run_id);
            assert_eq!(detail.run.status, RunStatus::Warning, "(c) {detail:?}");
            assert!(
                !Path::new(&p.suggestion.new_folder).exists(),
                "(c) no empty new folder"
            );
            assert!(!holder.exists(), "(d) no empty ~/Restored");
            fx.rig.daemon.stop();
        },
    );
}

/// While a restore runs from a destination, (a) `RemoveDestination` and (b)
/// `UpdateDestination` are refused with `RestoreRunning`; (c) the restore ends `ok`, and (d)
/// afterwards the destination can be removed (the restore no longer holds it).
#[test]
fn m4_s05_destination_of_a_running_restore_cannot_change() {
    run_isolated_test(
        "m4_s05_destination_of_a_running_restore_cannot_change",
        "KESTIC_M4_S05_DEST_IN_USE_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RU", small_tree, |_| {});
            // A backup on the destination would refuse its removal anyway.
            fx.rig
                .daemon
                .client
                .remove_backup(fx.backup.id.as_str())
                .expect("RemoveBackup");
            fx.rig.wrapper.hold("restore");
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&fx.new_dir("in-use")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            fx.rig
                .wrapper
                .wait_for('S', "restore", 1, std::time::Duration::from_secs(30));
            let removed = fx
                .rig
                .daemon
                .client
                .remove_destination(fx.dest.id.as_str())
                .expect_err("(a) RemoveDestination refused");
            common::m3::assert_dbus_error_name(&removed, "net.daester.Kestic.Error.RestoreRunning");
            let mut renamed = fx.dest.clone();
            renamed.name = "NAS 2".to_string();
            let updated = fx
                .rig
                .daemon
                .client
                .update_destination(&renamed, "connect", "", "", false)
                .expect_err("(b) UpdateDestination refused");
            common::m3::assert_dbus_error_name(&updated, "net.daester.Kestic.Error.RestoreRunning");
            fx.rig.wrapper.release("restore");
            let detail = fx.wait_run(&run_id);
            assert_eq!(detail.run.status, RunStatus::Ok, "(c) {detail:?}");
            let removed = fx.rig.daemon.client.remove_destination(fx.dest.id.as_str());
            assert!(removed.is_ok(), "(d) {removed:?}");
            fx.rig.daemon.stop();
        },
    );
}

/// The folders `e1` and `e2` without files, and a file `f.txt`.
fn folders_only_tree(work: &Path) -> std::path::PathBuf {
    let src = work.join("src");
    fs::create_dir_all(src.join("e1")).expect("mkdir e1");
    fs::create_dir_all(src.join("e2")).expect("mkdir e2");
    fs::write(src.join("f.txt"), "f").expect("write f.txt");
    src
}

/// A selection of folders without files is not «nothing selected matches»: (a) the preview has
/// no such note, (b) the restore ends `ok` and (c) restic created the folders.
#[test]
fn m4_s05_selection_of_folders_only_is_restored() {
    run_isolated_test(
        "m4_s05_selection_of_folders_only_is_restored",
        "KESTIC_M4_S05_FOLDERS_ONLY_INNER",
        |root| {
            for_each_variant(root, "M4RF", folders_only_tree, |fx, v| {
                let target = fx.new_dir("with-content");
                fs::create_dir_all(&target).expect("mkdir target");
                fs::write(target.join("other.txt"), "other").expect("content");
                let (id, p) = fx.preview(
                    &["e1", "e2"],
                    Some(new_folder(&target)),
                    RestoreOverwrite::Never,
                );
                assert!(
                    !p.notes.contains(
                        &kestic_daemon::dbus_api::RestoreNoteCode::NothingSelectedMatches
                    ),
                    "(a) {}: {:?}",
                    v.label,
                    p.notes
                );
                let run_id = fx
                    .start_restore(
                        &["e1", "e2"],
                        new_folder(&target),
                        RestoreOverwrite::Never,
                        Some(&id),
                    )
                    .expect("StartRestore");
                let detail = fx.wait_run(&run_id);
                assert_eq!(
                    detail.run.status,
                    RunStatus::Ok,
                    "(b) {}: {detail:?}",
                    v.label
                );
                assert!(
                    target.join("e1").is_dir() && target.join("e2").is_dir(),
                    "(c) {}",
                    v.label
                );
            });
        },
    );
}

/// AC-M4-05-4 (b), AC-M4-05-16 (f) and AC-M4-05-25 for a selection that spans folders (`a.txt`
/// and `docs/deep/y.txt`): restic creates the folders
/// in between, and the preview treats them as folders: at the original location (a) the
/// preview ends without refusing its target, (b) counts the two files only and (c) notes that
/// the existing folders get the snapshot's metadata; into a folder with content (d) both files
/// are new and nothing else is counted.
#[test]
fn m4_s05_selection_across_folders_previews_its_folders_as_folders() {
    run_isolated_test(
        "m4_s05_selection_across_folders_previews_its_folders_as_folders",
        "KESTIC_M4_S05_ACROSS_FOLDERS_INNER",
        |root| {
            for_each_variant(root, "M4RA", small_tree, |fx, v| {
                let selection = ["a.txt", "docs/deep/y.txt"];
                let id = fx.start_preview(&selection, Some(fx.original()), RestoreOverwrite::Never);
                let st = fx.wait_query(&id, &big_page());
                assert_eq!(st.state, "done", "(a) {}: {st:?}", v.label);
                let (_, p) = fx.preview(&selection, Some(fx.original()), RestoreOverwrite::Never);
                assert_eq!(p.entries.total, 2, "(b) {}: {:?}", v.label, p.entries.items);
                assert_eq!(p.identical_files, 2, "(b) {}", v.label);
                assert!(
                    p.notes
                        .contains(&kestic_daemon::dbus_api::RestoreNoteCode::FolderMetadataReset),
                    "(c) {}: {:?}",
                    v.label,
                    p.notes
                );
                let target = fx.new_dir("with-content");
                fs::create_dir_all(&target).expect("mkdir target");
                fs::write(target.join("other.txt"), "other").expect("content");
                let (_, into) = fx.preview(
                    &selection,
                    Some(new_folder(&target)),
                    RestoreOverwrite::Never,
                );
                assert_eq!(
                    into.new_files, 2,
                    "(d) {}: {:?}",
                    v.label, into.entries.items
                );
                assert_eq!(into.entries.total, 2, "(d) {}", v.label);
            });
        },
    );
}

/// AC-M4-05-25 for a symbolic link at a folder above the selected entries: restic follows a
/// link to a folder further up (and writes into another folder), replaces one at the folder
/// holding a selected entry and fails at a link loop, so the preview refuses the target with
/// (a) `invalid_target` and (b) the link as `error_detail`, for (1) a link at `docs` above
/// `docs/deep/y.txt`, (2) a link at `docs/deep`, (3) a link loop at `docs` and (4) a link to
/// nothing at `docs`; (c) nothing behind the links changes.
#[test]
fn m4_s05_symlink_above_the_selection_is_refused() {
    run_isolated_test(
        "m4_s05_symlink_above_the_selection_is_refused",
        "KESTIC_M4_S05_LINK_ABOVE_INNER",
        |root| {
            for_each_variant(root, "M4RL", small_tree, |fx, v| {
                let out = fx.new_dir("linked-docs");
                fs::create_dir_all(out.join("deep")).expect("mkdir out/deep");
                fs::write(out.join("deep/y.txt"), "different").expect("file behind the link");
                fs::remove_dir_all(fx.src.join("docs")).expect("remove docs");
                let selection = ["a.txt", "docs/deep/y.txt"];
                let refused_at = |link: &str, case: &str| {
                    for mode in [RestoreOverwrite::Never, RestoreOverwrite::Always] {
                        let id = fx.start_preview(&selection, Some(fx.original()), mode);
                        let st = fx.wait_query(&id, &big_page());
                        assert_eq!(
                            st.error_kind,
                            Some(kestic_i18n::codes::ErrorKind::InvalidTarget),
                            "(a) {} {case} {mode:?}: {st:?}",
                            v.label
                        );
                        let at = fs::canonicalize(&fx.src).expect("src").join(link);
                        assert_eq!(
                            st.error_detail.as_deref(),
                            Some(at.to_str().expect("utf-8")),
                            "(b) {} {case} {mode:?}",
                            v.label
                        );
                    }
                };
                std::os::unix::fs::symlink(&out, fx.src.join("docs")).expect("docs -> out");
                refused_at("docs", "(1)");
                fs::remove_file(fx.src.join("docs")).expect("remove link");
                fs::create_dir(fx.src.join("docs")).expect("real docs");
                std::os::unix::fs::symlink(out.join("deep"), fx.src.join("docs/deep"))
                    .expect("docs/deep -> out/deep");
                refused_at("docs/deep", "(2)");
                fs::remove_dir_all(fx.src.join("docs")).expect("remove docs");
                std::os::unix::fs::symlink("docs", fx.src.join("docs")).expect("docs -> docs");
                refused_at("docs", "(3)");
                fs::remove_file(fx.src.join("docs")).expect("remove loop");
                std::os::unix::fs::symlink(fx.new_dir("nowhere"), fx.src.join("docs"))
                    .expect("docs -> nothing");
                refused_at("docs", "(4)");
                assert_eq!(
                    fs::read_to_string(out.join("deep/y.txt")).expect("y.txt"),
                    "different",
                    "(c) {}",
                    v.label
                );
            });
        },
    );
}

/// AC-M4-05-25 for a file at the target where a folder above the selected entries belongs:
/// restic cannot create the folder there and the run fails, so the preview refuses the target
/// with (a) `invalid_target`, (b) that file as `error_detail` and (c) the translated text naming
/// it; (d) the file stays as it was.
#[test]
fn m4_s05_file_above_the_selection_is_refused() {
    run_isolated_test(
        "m4_s05_file_above_the_selection_is_refused",
        "KESTIC_M4_S05_FILE_ABOVE_INNER",
        |root| {
            for_each_variant(root, "M4RG", small_tree, |fx, v| {
                fs::remove_dir_all(fx.src.join("docs")).expect("remove docs");
                fs::write(fx.src.join("docs"), "a file").expect("file in its place");
                let id = fx.start_preview(
                    &["a.txt", "docs/deep/y.txt"],
                    Some(fx.original()),
                    RestoreOverwrite::Never,
                );
                let st = fx.wait_query(&id, &big_page());
                assert_eq!(
                    st.error_kind,
                    Some(kestic_i18n::codes::ErrorKind::InvalidTarget),
                    "(a) {}: {st:?}",
                    v.label
                );
                let blocked = fs::canonicalize(fx.src.join("docs")).expect("canonical");
                let blocked = blocked.to_str().expect("utf-8");
                assert_eq!(st.error_detail.as_deref(), Some(blocked), "(b) {}", v.label);
                match kestic_i18n::codes::ErrorKind::InvalidTarget.text(st.error_detail.as_deref())
                {
                    kestic_i18n::messages::UiText::Message(m) => {
                        assert_eq!(m.args, vec![blocked.to_string()], "(c) {}", v.label);
                    }
                    other => panic!("(c) {}: translatable message expected: {other:?}", v.label),
                }
                assert_eq!(
                    fs::read_to_string(fx.src.join("docs")).expect("docs"),
                    "a file",
                    "(d) {}",
                    v.label
                );
            });
        },
    );
}

/// The refusal of AC-M4-05-25, repeated right before the run: a folder above the selected
/// entries that becomes a symbolic link after the preview is caught before restic writes; the
/// run ends with `Fehler` and `invalid_target`, and the folder behind the link is unchanged.
#[test]
fn m4_s05_link_above_the_selection_after_the_preview_ends_the_run() {
    run_isolated_test(
        "m4_s05_link_above_the_selection_after_the_preview_ends_the_run",
        "KESTIC_M4_S05_LINK_LATER_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RZ", small_tree, |_| {});
            let selection = ["a.txt", "docs/deep/y.txt"];
            let (id, _) = fx.preview(&selection, Some(fx.original()), RestoreOverwrite::Always);
            let out = fx.new_dir("linked-later");
            fs::create_dir_all(out.join("deep")).expect("mkdir out/deep");
            fs::write(out.join("deep/y.txt"), "different").expect("file behind the link");
            fs::remove_dir_all(fx.src.join("docs")).expect("remove docs");
            std::os::unix::fs::symlink(&out, fx.src.join("docs")).expect("docs -> out");
            let run_id = fx
                .start_restore(
                    &selection,
                    fx.original(),
                    RestoreOverwrite::Always,
                    Some(&id),
                )
                .expect("StartRestore");
            let detail = fx.wait_run(&run_id);
            assert_eq!(detail.run.status, RunStatus::Error, "Fehler: {detail:?}");
            assert_eq!(
                detail.error_kind,
                Some(kestic_i18n::codes::ErrorKind::InvalidTarget),
                "invalid_target"
            );
            assert_eq!(
                fs::read_to_string(out.join("deep/y.txt")).expect("y.txt"),
                "different",
                "the folder behind the link is unchanged"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// While a restore waits for maintenance of its destination, (a) `RemoveDestination` and (b)
/// `UpdateDestination` are refused with `RestoreRunning`; (c) after it is cancelled the
/// destination is free again.
#[test]
fn m4_s05_destination_of_a_waiting_restore_cannot_change() {
    run_isolated_test(
        "m4_s05_destination_of_a_waiting_restore_cannot_change",
        "KESTIC_M4_S05_DEST_WAITING_INNER",
        |root| {
            let _fakes = common::m3::StdFakes::start();
            let variant = common::m3::latest_variant();
            let fx = scheduled_fixture(root, &variant, "M4RQ");
            fx.rig.wrapper.hold("prune");
            fx.rig.clock.set(SUNDAY_PRUNE);
            fx.rig
                .wrapper
                .wait_for('S', "prune", 1, std::time::Duration::from_secs(30));
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    new_folder(&fx.new_dir("waiting")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            let client = &fx.rig.daemon.client;
            let removed = client
                .remove_destination(fx.dest.id.as_str())
                .expect_err("(a) RemoveDestination refused");
            common::m3::assert_dbus_error_name(&removed, "net.daester.Kestic.Error.RestoreRunning");
            let updated = client
                .update_destination(&fx.dest, "connect", "", "", false)
                .expect_err("(b) UpdateDestination refused");
            common::m3::assert_dbus_error_name(&updated, "net.daester.Kestic.Error.RestoreRunning");
            client.cancel_run(&run_id).expect("CancelRun");
            assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Cancelled);
            let renamed = client.update_destination(&fx.dest, "connect", "", "", false);
            assert!(renamed.is_ok(), "(c) {renamed:?}");
            fx.rig.wrapper.release("prune");
            fx.rig.daemon.stop();
        },
    );
}

/// `UpdateDestination` with a connection check that is still checking when a restore of the
/// destination starts (a) is refused with `RestoreRunning` and (b) leaves the stored repository
/// password unchanged, although restic accepted the new one; (c) the restore ends `ok`.
#[test]
fn m4_s05_update_racing_a_restore_keeps_the_stored_password() {
    run_isolated_test(
        "m4_s05_update_racing_a_restore_keeps_the_stored_password",
        "KESTIC_M4_S05_UPDATE_RACE_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RW", small_tree, |_| {});
            let new_password_file = root.join(&fx.rig.sub).join("second-password");
            fs::write(&new_password_file, "second key").expect("write password file");
            let added = std::process::Command::new(&variant.bin)
                .arg("-r")
                .arg(&fx.repo)
                .args(["key", "add", "--new-password-file"])
                .arg(&new_password_file)
                .env("RESTIC_PASSWORD", &fx.password)
                .output()
                .expect("restic key add");
            assert!(added.status.success(), "restic key add: {added:?}");
            fx.rig.wrapper.hold("cat");
            let updated = std::thread::scope(|s| {
                let call = s.spawn(|| {
                    fx.rig.daemon.client.update_destination(
                        &fx.dest,
                        "connect",
                        "second key",
                        "",
                        true,
                    )
                });
                fx.rig
                    .wrapper
                    .wait_for('S', "cat", 1, std::time::Duration::from_secs(30));
                fx.rig.wrapper.hold("restore");
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        new_folder(&fx.new_dir("racing")),
                        RestoreOverwrite::Never,
                        None,
                    )
                    .expect("StartRestore");
                fx.rig
                    .wrapper
                    .wait_for('S', "restore", 1, std::time::Duration::from_secs(30));
                fx.rig.wrapper.release("cat");
                let updated = call.join().expect("UpdateDestination thread");
                fx.rig.wrapper.release("restore");
                let detail = fx.wait_run(&run_id);
                assert_eq!(detail.run.status, RunStatus::Ok, "(c) {detail:?}");
                updated
            });
            let err = updated.expect_err("(a) UpdateDestination refused");
            common::m3::assert_dbus_error_name(&err, "net.daester.Kestic.Error.RestoreRunning");
            let stored = kestic_core::secrets::SecretStore::get_secret(
                &common::isolated_secret_store(),
                fx.dest.id.as_str(),
                kestic_core::restic::SECRET_KIND_REPO,
            )
            .expect("read the stored password")
            .expect("a stored password");
            assert_eq!(
                stored.as_str().expect("UTF-8 password"),
                fx.password,
                "(b) the stored password is unchanged"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// A preview counts for one restore: after a restore with it was accepted, (a) a second
/// `StartRestore` with the same preview is refused with `PreviewRequired`, and (b) restic
/// restored once only.
#[test]
fn m4_s05_preview_counts_for_one_restore() {
    run_isolated_test(
        "m4_s05_preview_counts_for_one_restore",
        "KESTIC_M4_S05_PREVIEW_ONCE_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RO", small_tree, |_| {});
            let (preview_id, _) =
                fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    fx.original(),
                    RestoreOverwrite::Always,
                    Some(&preview_id),
                )
                .expect("first StartRestore");
            assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
            fs::write(fx.src.join("a.txt"), "edited after the restore").expect("edit a.txt");
            let again = fx.start_restore(
                &["a.txt"],
                fx.original(),
                RestoreOverwrite::Always,
                Some(&preview_id),
            );
            assert!(
                matches!(again, Err(KesticDbusError::PreviewRequired(_))),
                "(a) the used preview is refused: {again:?}"
            );
            assert_eq!(real_restores(&fx), 1, "(b) one restic restore only");
            fx.rig.daemon.stop();
        },
    );
}

/// A restore that fails midway (restic cannot write into a read-only folder, but restores the
/// other files) ends `Fehler` and still (a) counts the files it restored, (b) records its restic
/// call, and (c) keeps restic's output with the file that failed in its log.
#[test]
fn m4_s05_restore_failing_midway_keeps_what_it_did() {
    run_isolated_test(
        "m4_s05_restore_failing_midway_keeps_what_it_did",
        "KESTIC_M4_S05_PARTIAL_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RP", small_tree, |_| {});
            fs::remove_file(fx.src.join("a.txt")).expect("remove a.txt");
            fs::remove_file(fx.src.join("docs/x.txt")).expect("remove x.txt");
            set_mode(&fx.src.join("docs"), 0o555);
            let (preview_id, _) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Never);
            let run_id = fx
                .start_restore(
                    &[],
                    fx.original(),
                    RestoreOverwrite::Never,
                    Some(&preview_id),
                )
                .expect("StartRestore");
            let detail = fx.wait_run(&run_id);
            set_mode(&fx.src.join("docs"), 0o755);
            assert_eq!(detail.run.status, RunStatus::Error, "{detail:?}");
            assert_eq!(
                fs::read_to_string(fx.src.join("a.txt")).ok().as_deref(),
                Some("hello"),
                "restic restored a.txt"
            );
            let stats = detail.run.restore.as_ref().expect("restore stats");
            assert_eq!(stats.files_new, 1, "(a) {stats:?}");
            assert_eq!(
                detail.run.commands.len(),
                1,
                "(b) {:?}",
                detail.run.commands
            );
            assert!(
                detail.log_lines.iter().any(|l| l.contains("x.txt")),
                "(c) {:?}",
                detail.log_lines
            );
            fx.rig.daemon.stop();
        },
    );
}

/// When the daemon stops while a restore waits for maintenance, (a) the waiting restore does not
/// start on the maintenance's end and (b) its empty new folder is removed.
#[test]
fn m4_s05_waiting_restore_leaves_no_folder_when_the_daemon_stops() {
    run_isolated_test(
        "m4_s05_waiting_restore_leaves_no_folder_when_the_daemon_stops",
        "KESTIC_M4_S05_STOP_WAITING_INNER",
        |root| {
            let _fakes = common::m3::StdFakes::start();
            let variant = common::m3::latest_variant();
            let fx = scheduled_fixture(root, &variant, "M4RQ");
            fx.rig.wrapper.hold("prune");
            fx.rig.clock.set(SUNDAY_PRUNE);
            fx.rig
                .wrapper
                .wait_for('S', "prune", 1, std::time::Duration::from_secs(30));
            let target = fx.new_dir("waiting");
            fx.start_restore(
                &["a.txt"],
                new_folder(&target),
                RestoreOverwrite::Never,
                None,
            )
            .expect("StartRestore");
            assert!(target.is_dir(), "the new folder exists while it waits");
            fx.rig.daemon.stop();
            let started: Vec<String> = fx
                .rig
                .wrapper
                .of('S', "restore")
                .into_iter()
                .map(|e| e.detail)
                .filter(|c| !c.contains("--dry-run"))
                .collect();
            assert!(started.is_empty(), "(a) no restore started: {started:?}");
            assert!(!target.exists(), "(b) the empty new folder is removed");
        },
    );
}

/// A preview whose walk of the snapshot would hold more than the memory limit of one query stops
/// there: (a) its list is too large and empty, (b) no dry run runs, and (c) `StartRestore` with
/// it is refused with `PreviewRequired`, since it classified nothing.
#[test]
fn m4_s05_preview_over_the_memory_limit_stops() {
    run_isolated_test(
        "m4_s05_preview_over_the_memory_limit_stops",
        "KESTIC_M4_S05_PREVIEW_LIMIT_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RL", thousand_tree, |o| {
                o.query_limits = kestic_core::query_list::QueryLimits {
                    per_query_bytes: 20_000,
                    total_bytes: 40_000,
                };
            });
            let (preview_id, p) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Never);
            assert!(p.entries.too_large, "(a) too large: {:?}", p.entries.total);
            assert!(p.entries.items.is_empty(), "(a) no items");
            assert!(dry_runs(&fx).is_empty(), "(b) {:?}", dry_runs(&fx));
            let refused = fx.start_restore(
                &[],
                fx.original(),
                RestoreOverwrite::Never,
                Some(&preview_id),
            );
            assert!(
                matches!(refused, Err(KesticDbusError::PreviewRequired(_))),
                "(c) {refused:?}"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// A preview whose walk fits the memory limit of one query but whose dry run does not stops
/// at the dry run: (a) its list is too large and nothing is counted, (b) the second dry run
/// does not run, and (c) `StartRestore` with it is refused with `PreviewRequired`.
#[test]
fn m4_s05_preview_stops_when_the_dry_run_exceeds_the_memory_limit() {
    run_isolated_test(
        "m4_s05_preview_stops_when_the_dry_run_exceeds_the_memory_limit",
        "KESTIC_M4_S05_DRY_RUN_LIMIT_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            // The walk of 1000 files holds about 74 000 bytes, each dry run about as much.
            let fx = RestoreFixture::start(root, &variant, "M4RD", thousand_tree, |o| {
                o.query_limits = kestic_core::query_list::QueryLimits {
                    per_query_bytes: 100_000,
                    total_bytes: 200_000,
                };
            });
            let (preview_id, p) = fx.preview(&[], Some(fx.original()), RestoreOverwrite::Never);
            assert!(p.entries.too_large, "(a) too large: {:?}", p.entries.total);
            assert_eq!(
                (
                    p.new_files,
                    p.overwritten_files,
                    p.kept_files,
                    p.identical_files
                ),
                (0, 0, 0, 0),
                "(a) nothing counted"
            );
            assert_eq!(dry_runs(&fx).len(), 1, "(b) {:?}", dry_runs(&fx));
            let refused = fx.start_restore(
                &[],
                fx.original(),
                RestoreOverwrite::Never,
                Some(&preview_id),
            );
            assert!(
                matches!(refused, Err(KesticDbusError::PreviewRequired(_))),
                "(c) {refused:?}"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// [`unmatchable_tree`] plus a folder whose name is not UTF-8, with a file and an empty folder.
fn unmatchable_folder_tree(work: &Path) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let src = unmatchable_tree(work);
    let folder = src.join(std::ffi::OsStr::from_bytes(b"d\xff"));
    fs::create_dir_all(folder.join("sub")).expect("mkdir d/sub");
    fs::write(folder.join("x"), "x").expect("write x");
    src
}

/// A restore of names that are not UTF-8 (restic reports them with U+FFFD) counts every file it
/// wrote and no folder: (a) `files_new` is 4 (three files and one in such a folder) and (b) the
/// summary says so.
#[test]
fn m4_s05_restore_counts_names_that_are_not_utf8() {
    run_isolated_test(
        "m4_s05_restore_counts_names_that_are_not_utf8",
        "KESTIC_M4_S05_COUNT_UNMATCHABLE_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RN", unmatchable_folder_tree, |_| {});
            let run_id = fx
                .start_restore(
                    &[],
                    new_folder(&fx.new_dir("names")),
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore");
            let detail = fx.wait_run(&run_id);
            assert_eq!(detail.run.status, RunStatus::Ok, "{detail:?}");
            let stats = detail.run.restore.as_ref().expect("restore stats");
            assert_eq!(stats.files_new, 4, "(a) {stats:?}");
            assert_eq!(
                detail.run.summary_message.as_deref(),
                Some("Restored 4 files"),
                "(b)"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// A restore cancelled after restic reported a written file keeps that count: (a) it ends
/// `abgebrochen` and (b) `files_new` is 1.
#[test]
fn m4_s05_cancelled_restore_keeps_its_counts() {
    run_isolated_test(
        "m4_s05_cancelled_restore_keeps_its_counts",
        "KESTIC_M4_S05_CANCEL_COUNTS_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RC", small_tree, |_| {});
            let (preview_id, _) =
                fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
            // The held run reports `a.txt`, which is at the original location, as written, then
            // a progress line that shows once the daemon has read the line before it.
            fx.rig.wrapper.hold_with_output(
                "restore",
                "{\"message_type\":\"verbose_status\",\"action\":\"restored\",\
                 \"item\":\"/a.txt\",\"size\":5}\n\
                 {\"message_type\":\"status\",\"percent_done\":0.5,\"total_bytes\":10}\n",
            );
            let run_id = fx
                .start_restore(
                    &["a.txt"],
                    fx.original(),
                    RestoreOverwrite::Always,
                    Some(&preview_id),
                )
                .expect("StartRestore");
            fx.rig
                .wrapper
                .wait_for('S', "restore", 2, std::time::Duration::from_secs(30));
            common::m3::wait_for_state(
                &fx.rig.daemon.client,
                std::time::Duration::from_secs(10),
                "the progress after the reported file",
                |s| {
                    s.overall
                        .active_run_progress
                        .as_ref()
                        .is_some_and(|p| p.percent_done >= 50.0)
                },
            );
            fx.rig.daemon.client.cancel_run(&run_id).expect("CancelRun");
            let detail = fx.wait_run(&run_id);
            fx.rig.wrapper.release("restore");
            assert_eq!(detail.run.status, RunStatus::Cancelled, "(a) {detail:?}");
            let stats = detail.run.restore.as_ref().expect("restore stats");
            assert_eq!(stats.files_new, 1, "(b) {stats:?}");
            fx.rig.daemon.stop();
        },
    );
}

/// AC-M4-05-25 (FIFO): a preview at the original location where the target has a FIFO in
/// place of a file of the snapshot (a) ends with `invalid_target`, (b) names the FIFO, and
/// (c) runs no dry run, where restic would hang.
#[test]
fn ac_m4_05_25_fifo_at_a_file_is_refused_before_any_dry_run() {
    run_isolated_test(
        "ac_m4_05_25_fifo_at_a_file_is_refused_before_any_dry_run",
        "KESTIC_AC_M4_05_25_FIFO_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let fx = RestoreFixture::start(root, &variant, "M4RF", small_tree, |_| {});
            let fifo = fx.src.join("a.txt");
            fs::remove_file(&fifo).expect("remove a.txt");
            let made = std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .expect("mkfifo");
            assert!(made.success(), "mkfifo");
            let id = fx.start_preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
            let st = fx.wait_query(&id, &big_page());
            assert_eq!(
                st.error_kind,
                Some(kestic_i18n::codes::ErrorKind::InvalidTarget),
                "(a) {st:?}"
            );
            let resolved = fs::canonicalize(&fx.src).expect("canonical").join("a.txt");
            assert_eq!(
                st.error_detail.as_deref(),
                Some(resolved.to_str().expect("utf-8")),
                "(b)"
            );
            assert!(dry_runs(&fx).is_empty(), "(c) {:?}", dry_runs(&fx));
            fx.rig.daemon.stop();
        },
    );
}
