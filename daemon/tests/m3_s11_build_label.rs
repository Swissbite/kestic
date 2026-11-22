// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! AC-M3-11-4 (c), (d): the build state in the daemon's startup line comes from `build.rs`, which
//! labels a source tree through `kestic_build_stamp`. These tests run that logic, and the version
//! taken from Git tags, against temporary Git repositories.

mod common;

use common::run_isolated_test;
use kestic_build_stamp::commit::build_label;
use kestic_build_stamp::version::build_version;
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

/// architecture.md §Release: the version of a build comes from the nearest version tag: (a)
/// `0.0.0~dev+g<hash>` without one (another tag such as `m4-start` does not count), (b) the tag's
/// version on the tagged commit, (c) commits after it add their number and the commit, and (d) a
/// source tree outside a checkout is `0.0.0~dev`.
#[test]
fn build_version_follows_version_tags() {
    run_isolated_test(
        "build_version_follows_version_tags",
        "KESTIC_BUILD_VERSION_TAGS_INNER",
        |root| {
            let repo = checkout(root);
            git(&repo, &["tag", "m4-start"]);
            let first = git(&repo, &["rev-parse", "--short=7", "HEAD"]);
            assert_eq!(
                build_version(&repo, None),
                Ok(format!("0.0.0~dev+g{first}")),
                "(a) no version tag"
            );

            git(&repo, &["tag", "v0.4.0-m4.3"]);
            assert_eq!(
                build_version(&repo, None),
                Ok("0.4.0~m4.3".to_owned()),
                "(b) tagged commit"
            );

            for n in 1..=2 {
                fs::write(repo.join("core/src/lib.rs"), format!("// after {n}\n")).expect("write");
                git(&repo, &["commit", "-q", "-am", "next"]);
            }
            let head = git(&repo, &["rev-parse", "--short=7", "HEAD"]);
            assert_eq!(
                build_version(&repo, None),
                Ok(format!("0.4.0~m4.3+2.g{head}")),
                "(c) two commits after the tag"
            );

            let plain = root.join("tarball");
            fs::create_dir_all(&plain).expect("mkdir");
            assert_eq!(
                build_version(&plain, None),
                Ok("0.0.0~dev".to_owned()),
                "(d) outside a checkout"
            );
        },
    );
}
