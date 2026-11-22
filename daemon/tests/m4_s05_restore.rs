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
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
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
                        "(a) {}: {case}: {err:?}",
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
                let (fresh_id, _) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
                fx.rig.clock.advance(std::time::Duration::from_secs(11 * 60));
                refused(Some(&fresh_id), "older than 10 minutes");
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
                    .start_restore(&["[dir]*"], new_folder(&target), RestoreOverwrite::Never, None)
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
                assert_eq!(detail.run.status, RunStatus::Warning, "(a) {}: {detail:?}", v.label);
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
                let (_, p) = fx.preview(&["a.txt"], Some(new_folder(&link)), RestoreOverwrite::Never);
                assert_eq!(p.target.check.code, RestoreTargetCode::Ok, "(a) {}", v.label);
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
            assert!(state.blocking.is_none(), "(a) no block: {:?}", state.blocking);
            assert!(state.restic_status.valid, "(a) restic usable");
            assert_eq!(
                state.restic_status.version.as_deref(),
                Some("0.18.0"),
                "(a) the 0.18.0 binary is used"
            );
            let target = fx.new_dir("old-restic");
            let run_id = fx
                .start_restore(&["a.txt"], new_folder(&target), RestoreOverwrite::Never, None)
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
                let mut last = None;
                loop {
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
                        last = Some(p);
                        break;
                    }
                    for e in &p.entries.items {
                        assert_eq!(e.outcome, RestoreOutcome::Overwritten, "{}", v.label);
                        seen.insert(e.path.clone());
                    }
                    offset += p.entries.items.len() as u64;
                    last = Some(p);
                }
                let p = last.expect("pages");
                assert_eq!(seen.len(), 300, "(a) {}: all 300 reachable", v.label);
                assert_eq!(p.entries.matched, 300, "(b) {}", v.label);
                assert_eq!(
                    (p.new_files, p.overwritten_files, p.kept_files, p.identical_files),
                    (600, 300, 0, 100),
                    "(c) {}: exact counts",
                    v.label
                );
                assert_eq!(p.entries.total, 1000, "(c) {}: total", v.label);
            });
        },
    );
}
