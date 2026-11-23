// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S06 against real restic with temp repositories: a snapshot of another user's home
//! (`/home/<user>/…`, written with `restic backup --stdin --stdin-filename`) is restored at the
//! original location in the isolated home of the test. Folders in `--stdin-filename` need
//! restic 0.20 or newer, so these tests run with the latest restic only; the mapping itself
//! calls no restic command.

mod common;

use common::m3::latest_variant;
use common::m4::{RawSnapshot, backup_stdin, small_tree, snapshot_username};
use common::m4_restore::RestoreFixture;
use common::run_isolated_test;
use kestic_core::model::{RunKind, RunStatus};
use kestic_daemon::dbus_api::{
    KesticDbusError, RestoreOverwrite, RestoreTarget, RestoreTargetCode,
};
use std::fs;
use std::path::{Path, PathBuf};

/// A fixture whose snapshot holds `/home/<user>/<file>` with the content «hello», `<user>`
/// being the user restic records (so the snapshot's `username` matches the folder). Paths
/// passed to the fixture are relative to `/home/<user>`.
fn other_home_fixture(root: &Path, base: &str, file: &str) -> (RestoreFixture, String) {
    let variant = latest_variant();
    let fx = RestoreFixture::start(root, &variant, base, small_tree, |_| {});
    let user = snapshot_username(&variant.bin, &fx.repo, &fx.password, &fx.snapshot_id);
    let snapshot_id = backup_stdin(
        &variant.bin,
        &fx.repo,
        &fx.password,
        &format!("home/{user}/{file}"),
        "hello",
        &RawSnapshot {
            tag: &format!("kestic:{}", fx.backup.id),
            time: "2026-11-15 13:00:00",
            host: "old-laptop",
        },
    );
    let fx = RestoreFixture {
        snapshot_id,
        src: PathBuf::from(format!("/home/{user}")),
        ..fx
    };
    (fx, user)
}

/// The home folder of the test (and of the daemon under test).
fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME set"))
}

fn original(dir: &Path) -> RestoreTarget {
    RestoreTarget::Original {
        dir: dir.to_string_lossy().into_owned(),
    }
}

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

/// AC-M4-06-1 (a, b) end to end: `/home/<user>/Dokumente/a.txt` of another computer is
/// suggested in `Dokumente` of the user's home with the mapping `<user> → <home>`, and a restore
/// there writes `a.txt` into the user's `Dokumente`.
#[test]
fn ac_m4_06_1_restore_lands_in_the_users_home() {
    run_isolated_test(
        "ac_m4_06_1_restore_lands_in_the_users_home",
        "KESTIC_AC_M4_06_1_INNER",
        |root| {
            let (fx, user) = other_home_fixture(root, "M4M1", "Dokumente/a.txt");
            let dokumente = home().join("Dokumente");
            fs::create_dir_all(&dokumente).expect("mkdir Dokumente");
            let (_, suggested) = fx.preview(&["Dokumente/a.txt"], None, RestoreOverwrite::Never);
            let suggestion = &suggested.suggestion.original;
            assert_eq!(
                suggestion.mapped_dir.as_deref(),
                Some(dokumente.to_string_lossy().as_ref()),
                "(a) Dokumente in the user's home"
            );
            assert!(!suggestion.needs_user_choice, "(a) usable");
            let home_name = home()
                .file_name()
                .expect("home name")
                .to_string_lossy()
                .into_owned();
            assert_eq!(
                suggestion.mapping,
                Some((user.clone(), home_name)),
                "(b) mapping old user → user"
            );
            let (preview_id, _) = fx.preview(
                &["Dokumente/a.txt"],
                Some(original(&dokumente)),
                RestoreOverwrite::Never,
            );
            let run_id = fx
                .start_restore(
                    &["Dokumente/a.txt"],
                    original(&dokumente),
                    RestoreOverwrite::Never,
                    Some(&preview_id),
                )
                .expect("StartRestore");
            assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
            assert_eq!(
                fs::read_to_string(dokumente.join("a.txt")).expect("a.txt restored"),
                "hello",
                "(a) a.txt lands in the user's Dokumente"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// AC-M4-06-4: `/home/<user>/Projekte/a.txt` and no `Projekte` in the user's home: (a) the
/// original location needs a chosen folder, (b) `Projekte` is never created, (c) no restore
/// starts until a folder is chosen, and (d) the daemon checks the mapped folder at the
/// suggestion and again right before the start, without a restic process when it is missing
/// then (`missing_mapped_dir`).
#[test]
fn ac_m4_06_4_missing_mapped_folder_is_never_created() {
    run_isolated_test(
        "ac_m4_06_4_missing_mapped_folder_is_never_created",
        "KESTIC_AC_M4_06_4_INNER",
        |root| {
            let (fx, _) = other_home_fixture(root, "M4M4", "Projekte/a.txt");
            let projekte = home().join("Projekte");
            let selection = ["Projekte/a.txt"];
            let (_, suggested) = fx.preview(&selection, None, RestoreOverwrite::Never);
            let suggestion = &suggested.suggestion.original;
            assert!(
                suggestion.needs_user_choice,
                "(a) a folder has to be chosen"
            );
            assert_eq!(
                suggestion.mapped_dir.as_deref(),
                Some(projekte.to_string_lossy().as_ref()),
                "(d) the missing folder is named at the suggestion"
            );
            let (_, checked) = fx.preview(
                &selection,
                Some(original(&projekte)),
                RestoreOverwrite::Never,
            );
            assert_eq!(
                checked.target.check.code,
                RestoreTargetCode::MissingMappedDir,
                "(d) the check of the target"
            );
            assert!(
                fx.restore_calls().is_empty(),
                "(d) no dry run for a missing folder: {:?}",
                fx.restore_calls()
            );

            let refused = |preview_id: Option<&str>, case: &str| {
                let started = fx.rig.wrapper.started_repo_count();
                let runs = restore_runs(&fx);
                let err = fx
                    .start_restore(
                        &selection,
                        original(&projekte),
                        RestoreOverwrite::Never,
                        preview_id,
                    )
                    .expect_err("StartRestore must be refused");
                let KesticDbusError::InvalidInput(message) = &err else {
                    panic!("(d) {case}: InvalidInput expected, got {err:?}");
                };
                assert!(
                    message.contains("missing_mapped_dir"),
                    "(d) {case}: {message}"
                );
                assert_eq!(
                    fx.rig.wrapper.started_repo_count(),
                    started,
                    "(d) {case}: no restic process"
                );
                assert_eq!(restore_runs(&fx), runs, "(c) {case}: no run");
                assert!(
                    fs::symlink_metadata(&projekte).is_err(),
                    "(b) {case}: Projekte is not created"
                );
            };
            refused(None, "missing at the suggestion");

            // The folder exists for the preview and is gone at the start.
            fs::create_dir_all(&projekte).expect("mkdir Projekte");
            let (preview_id, ready) = fx.preview(
                &selection,
                Some(original(&projekte)),
                RestoreOverwrite::Never,
            );
            assert!(
                !ready.suggestion.original.needs_user_choice,
                "the existing folder is usable"
            );
            fs::remove_dir(&projekte).expect("remove Projekte");
            refused(Some(&preview_id), "missing right before the start");
            assert!(
                fx.restore_calls().iter().all(|c| c.contains("--dry-run")),
                "(c) only dry runs: {:?}",
                fx.restore_calls()
            );

            let chosen = fx.new_dir("chosen");
            let run_id = fx
                .start_restore(
                    &selection,
                    RestoreTarget::NewFolder {
                        dir: chosen.to_string_lossy().into_owned(),
                    },
                    RestoreOverwrite::Never,
                    None,
                )
                .expect("StartRestore into the chosen folder");
            assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
            assert_eq!(
                fs::read_to_string(chosen.join("a.txt")).expect("a.txt restored"),
                "hello",
                "(c) the restore runs into the chosen folder"
            );
            assert!(
                fs::symlink_metadata(&projekte).is_err(),
                "(b) Projekte is not created"
            );
            fx.rig.daemon.stop();
        },
    );
}
