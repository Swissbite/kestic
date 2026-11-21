// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! Records the Git commit the binary is built from in `KESTIC_BUILD_COMMIT`: the short hash,
//! with `-dirty` when tracked files differ from it, or `unknown` outside a Git checkout (e.g. a
//! source tarball) or without `git`. No timestamps, so equal sources give equal binaries.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Workspace paths whose content ends up in the `kestic` binary. Only changes there count for the
/// `-dirty` marker, and each change reruns the build script so the marker stays current. A new
/// workspace crate or source directory belongs in this list.
const SOURCE_INPUTS: [&str; 13] = [
    "Cargo.toml",
    "Cargo.lock",
    "core/Cargo.toml",
    "core/src",
    "i18n/Cargo.toml",
    "i18n/src",
    "daemon/Cargo.toml",
    "daemon/src",
    "gui/Cargo.toml",
    "gui/build.rs",
    "gui/src",
    "po",
    "packaging",
];

/// Runs `git` in `dir` and returns its trimmed standard output, or `None` if it fails. Variables
/// that point Git at another repository or index (set e.g. inside Git hooks) are removed.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.trim().to_owned())
}

/// Whether `workspace_root` is the top level of a Git work tree. Guards against a source tarball
/// unpacked inside some other repository, whose commit would be wrong here.
fn is_own_checkout(workspace_root: &Path) -> bool {
    let Some(top) = git(workspace_root, &["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    let canonical = |p: &Path| p.canonicalize().ok();
    canonical(Path::new(&top)).is_some_and(|top| Some(top) == canonical(workspace_root))
}

/// Short commit hash plus `-dirty` when tracked files below [`SOURCE_INPUTS`] have uncommitted
/// changes. Untracked files do not count: they only reach the binary through a tracked file that
/// references them, and that file is then modified. `--no-optional-locks` keeps `git status` from
/// rewriting the index while a build runs.
fn commit_label(workspace_root: &Path) -> Option<String> {
    let hash = git(workspace_root, &["rev-parse", "--short=7", "HEAD"])?;
    let mut status = vec![
        "--no-optional-locks",
        "status",
        "--porcelain",
        "--untracked-files=no",
        "--",
    ];
    status.extend(SOURCE_INPUTS);
    let changes = git(workspace_root, &status)?;
    Some(if changes.is_empty() {
        hash
    } else {
        format!("{hash}-dirty")
    })
}

fn rerun_if_exists(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// Reruns the build script when `HEAD` moves: a commit, checkout or reset rewrites `HEAD`, its
/// reflog or the branch ref. In a linked worktree `.git` is a file, so the directories come from
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

    let label = if is_own_checkout(workspace_root) {
        watch_head(workspace_root);
        commit_label(workspace_root)
    } else {
        None
    };
    println!(
        "cargo:rustc-env=KESTIC_BUILD_COMMIT={}",
        label.as_deref().unwrap_or("unknown")
    );
}
