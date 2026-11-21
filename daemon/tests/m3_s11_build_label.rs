// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! AC-M3-11-4 (c), (d): the build state in the daemon's startup line comes from `build.rs`, which
//! labels a source tree through `daemon/build_commit.rs`. These tests run that logic against
//! temporary Git repositories.

mod common;

#[path = "../build_commit.rs"]
mod build_commit;

use build_commit::build_label;
use common::run_isolated_test;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Runs `git` in `dir` with an empty configuration; fails the test if it fails.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Kestic Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .trim()
        .to_owned()
}

/// A Git checkout with one committed source file below `core/src`.
fn checkout(root: &Path) -> std::path::PathBuf {
    let repo = root.join("checkout");
    fs::create_dir_all(repo.join("core/src")).expect("mkdir");
    fs::write(repo.join("core/src/lib.rs"), "// v1\n").expect("write source");
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    repo
}

/// AC-M3-11-4 (c): the label of a clean checkout is its short hash; after a change to a tracked
/// source file it carries `-dirty`, while an untracked file alone does not mark it.
#[test]
fn ac_m3_11_4_c_modified_checkout_is_dirty() {
    run_isolated_test(
        "ac_m3_11_4_c_modified_checkout_is_dirty",
        "KESTIC_AC_M3_11_4_C_INNER",
        |root| {
            let repo = checkout(root);
            let hash = git(&repo, &["rev-parse", "--short=7", "HEAD"]);
            assert_eq!(build_label(&repo), hash, "clean checkout: short hash");
            fs::write(repo.join("core/src/untracked.rs"), "// new\n").expect("write untracked");
            assert_eq!(build_label(&repo), hash, "untracked file does not count");
            fs::write(repo.join("core/src/lib.rs"), "// v2\n").expect("modify source");
            assert_eq!(
                build_label(&repo),
                format!("{hash}-dirty"),
                "(c) modified tracked source file"
            );
        },
    );
}

/// AC-M3-11-4 (d): `unknown` for a source tree that is no Git checkout, and for one unpacked
/// inside another repository (its commit would be wrong).
#[test]
fn ac_m3_11_4_d_no_checkout_is_unknown() {
    run_isolated_test(
        "ac_m3_11_4_d_no_checkout_is_unknown",
        "KESTIC_AC_M3_11_4_D_INNER",
        |root| {
            let plain = root.join("tarball");
            fs::create_dir_all(plain.join("core/src")).expect("mkdir");
            assert_eq!(build_label(&plain), "unknown", "(d) outside a checkout");

            let repo = checkout(root);
            let nested = repo.join("vendor/kestic");
            fs::create_dir_all(nested.join("core/src")).expect("mkdir nested");
            assert_eq!(
                build_label(&nested),
                "unknown",
                "(d) inside another repository"
            );
        },
    );
}
