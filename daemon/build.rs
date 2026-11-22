// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! Records the version in `KESTIC_VERSION` (from `KESTIC_VERSION` or the Git tags, see
//! `kestic_build_stamp::version`) and the Git commit the binary is built from in
//! `KESTIC_BUILD_COMMIT`: the short hash, with `-dirty` when tracked files differ from it, or
//! `unknown` outside a Git checkout (e.g. a source tarball) or without `git`. No timestamps, so
//! equal sources give equal binaries.

use kestic_build_stamp::commit::{SOURCE_INPUTS, build_label, git, is_own_checkout};
use kestic_build_stamp::version::{OVERRIDE_VAR, build_version};
use std::env;
use std::path::{Path, PathBuf};

fn rerun_if_exists(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// Reruns the build script when `HEAD` moves or a tag appears: a commit, checkout or reset
/// rewrites `HEAD`, its reflog or the branch ref, a tag adds a file below `refs/tags`. In a linked worktree `.git` is a file, so the directories come from
/// `git rev-parse` instead of being assumed.
fn watch_head(workspace_root: &Path) {
    let Some(git_dir) = git(workspace_root, &["rev-parse", "--absolute-git-dir"]) else {
        return;
    };
    let git_dir = PathBuf::from(git_dir);
    rerun_if_exists(&git_dir.join("HEAD"));
    rerun_if_exists(&git_dir.join("logs/HEAD"));

    let Some(common_dir) = git(
        workspace_root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ) else {
        return;
    };
    let common_dir = PathBuf::from(common_dir);
    rerun_if_exists(&common_dir.join("packed-refs"));
    rerun_if_exists(&common_dir.join("refs/tags"));
    rerun_if_exists(&common_dir.join("reftable"));
    if let Some(head_ref) = git(workspace_root, &["symbolic-ref", "-q", "HEAD"]) {
        // A branch kept only in `packed-refs` gets its loose ref file with the next commit; the
        // closest existing directory notices that file appearing.
        let ref_path = common_dir.join(head_ref);
        if let Some(watched) = ref_path.ancestors().find(|p| p.exists()) {
            rerun_if_exists(watched);
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("daemon crate has workspace root parent");

    for input in SOURCE_INPUTS {
        rerun_if_exists(&workspace_root.join(input));
    }

    if is_own_checkout(workspace_root) {
        watch_head(workspace_root);
    }
    println!(
        "cargo:rustc-env=KESTIC_BUILD_COMMIT={}",
        build_label(workspace_root)
    );

    println!("cargo:rerun-if-env-changed={OVERRIDE_VAR}");
    let override_value = env::var(OVERRIDE_VAR).ok();
    let version = build_version(workspace_root, override_value.as_deref())
        .unwrap_or_else(|message| panic!("{message}"));
    println!("cargo:rustc-env=KESTIC_VERSION={version}");
}
