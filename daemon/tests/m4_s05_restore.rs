// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S05 restoring files against real restic (0.18.0 and latest) with temp repositories:
//! the preview (`StartSnapshotQuery` kind `restore_preview`), `StartRestore` and the run.

mod common;

use common::m3::{ResticVariant, restic_variants};
use common::m4::small_tree;
use common::m4_restore::{RestoreFixture, big_page, set_mtime, tree_contents};
use common::run_isolated_test;
use kestic_core::model::{RunKind, RunStatus};
use kestic_daemon::dbus_api::{
    PageFilter, RestoreOutcome, RestoreOverwrite, RestorePreviewResultDto, RestoreTarget,
    RestoreTargetCode, SnapshotQueryPage,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn for_each_variant(
    root: &Path,
    base: &str,
    make_tree: fn(&Path) -> std::path::PathBuf,
    body: impl Fn(&RestoreFixture, &ResticVariant),
) {
    for variant in restic_variants() {
        let fx = RestoreFixture::start(root, &variant, base, make_tree, |_| {});
        body(&fx, &variant);
        fx.rig.daemon.stop();
    }
}

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
                // Linked in the snapshot only: the second name is gone at the target.
                fs::remove_file(fx.src.join("hard.txt")).expect("unlink hard.txt");
                let (_, in_snapshot) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
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
    // One scheduler tick records the maintenance baseline of the new destination.
    std::thread::sleep(std::time::Duration::from_millis(1300));
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
                fx.rig.wrapper.hold("restore");
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
                    .wait_for('S', "restore", 1, std::time::Duration::from_secs(30));
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
            fx.rig.wrapper.hold("restore");
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
                .wait_for('S', "restore", 1, std::time::Duration::from_secs(30));
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

/// A folder with content at the target where the snapshot has a file cannot be restored over
/// (restic fails and changes the folder's mode): the preview refuses it and the folder stays as
/// it was.
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
                let mode = fs::metadata(fx.src.join("a.txt"))
                    .expect("folder")
                    .permissions()
                    .mode();
                for overwrite in [RestoreOverwrite::Never, RestoreOverwrite::Always] {
                    let id = fx.start_preview(&["a.txt"], Some(fx.original()), overwrite);
                    let st = fx.wait_query(&id, &big_page());
                    assert_eq!(
                        st.state, "error",
                        "{} {overwrite:?}: refused: {st:?}",
                        v.label
                    );
                }
                assert_eq!(
                    fs::metadata(fx.src.join("a.txt"))
                        .expect("folder")
                        .permissions()
                        .mode(),
                    mode,
                    "{}: the folder keeps its mode",
                    v.label
                );
                assert!(
                    fx.src.join("a.txt/inner/f").is_file(),
                    "{}: content kept",
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
                let id = fx.start_preview(&[], Some(fx.original()), RestoreOverwrite::Never);
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

/// A symbolic link and a hard-linked file that are the same at the target as in the snapshot
/// count as unchanged in every mode, although restic reports them `restored` with `always`
/// and `if-changed`.
#[test]
fn m4_s05_identical_links_are_unchanged() {
    run_isolated_test(
        "m4_s05_identical_links_are_unchanged",
        "KESTIC_M4_S05_SAME_LINKS_INNER",
        |root| {
            for_each_variant(root, "M4RL", linked_tree, |fx, v| {
                for mode in [
                    RestoreOverwrite::Never,
                    RestoreOverwrite::IfChanged,
                    RestoreOverwrite::Always,
                ] {
                    let (_, p) = fx.preview(&[], Some(fx.original()), mode);
                    let o = outcomes(&p);
                    assert_eq!(
                        o.get("link"),
                        Some(&RestoreOutcome::Identical),
                        "{} {mode:?}: symlink",
                        v.label
                    );
                    assert_eq!(
                        o.get("hard.txt"),
                        Some(&RestoreOutcome::Identical),
                        "{} {mode:?}: hard link",
                        v.label
                    );
                    assert_eq!(p.kept_files, 0, "{} {mode:?}: nothing kept", v.label);
                    assert_eq!(
                        p.overwritten_files, 0,
                        "{} {mode:?}: nothing overwritten",
                        v.label
                    );
                }
            });
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
