// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! Commit label of a source tree, as shown in the daemon's startup line, the tray and «About
//! Kestic»: short commit hash, `-dirty` when tracked source files differ from it, `unknown`
//! outside a Git checkout or without `git`.

use std::path::Path;
use std::process::Command;

/// Workspace paths whose content ends up in the `kestic` binary. Only changes there count for the
/// `-dirty` marker, and each change reruns the build script so the marker stays current. A new
/// workspace crate or source directory belongs in this list.
pub const SOURCE_INPUTS: [&str; 15] = [
    "Cargo.toml",
    "Cargo.lock",
    "build-stamp/Cargo.toml",
    "build-stamp/src",
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
pub fn git(dir: &Path, args: &[&str]) -> Option<String> {
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
pub fn is_own_checkout(workspace_root: &Path) -> bool {
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
pub fn commit_label(workspace_root: &Path) -> Option<String> {
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

/// The build state of `workspace_root` as it goes into `KESTIC_BUILD_COMMIT`.
pub fn build_label(workspace_root: &Path) -> String {
    let label = if is_own_checkout(workspace_root) {
        commit_label(workspace_root)
    } else {
        None
    };
    label.unwrap_or_else(|| "unknown".to_owned())
}
