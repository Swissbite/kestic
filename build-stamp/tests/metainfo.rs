// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! The `metainfo` command of `kestic-build-stamp` as `just deb` runs it in the release workflow:
//! the version of the tag in `KESTIC_VERSION` becomes the release entry of the AppStream metainfo
//! (AC-M6-23-3).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TEMPLATE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
    <component type=\"desktop-application\">\n  <id>net.daester.Kestic</id>\n</component>\n";

/// An empty directory below Cargo's temp directory for integration tests.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("remove previous test directory");
    }
    fs::create_dir_all(&dir).expect("create test directory");
    dir
}

/// Runs `kestic-build-stamp metainfo` in `dir` with `KESTIC_VERSION=0.6.0` and the given
/// `SOURCE_DATE_EPOCH`, Git configured only by the test.
fn run_metainfo(dir: &Path, source_date_epoch: Option<&str>) -> (Output, String) {
    fs::write(dir.join("template.xml"), TEMPLATE).expect("write template");
    let mut command = Command::new(env!("CARGO_BIN_EXE_kestic-build-stamp"));
    command
        .args(["metainfo", "template.xml", "out.xml"])
        .current_dir(dir)
        .env("KESTIC_VERSION", "0.6.0")
        .env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("SOURCE_DATE_EPOCH");
    if let Some(value) = source_date_epoch {
        command.env("SOURCE_DATE_EPOCH", value);
    }
    let output = command.output().expect("run kestic-build-stamp metainfo");
    let written = fs::read_to_string(dir.join("out.xml")).unwrap_or_default();
    (output, written)
}

/// AC-M6-23-3 (b): a release build (`KESTIC_VERSION` from the tag) writes (a) successfully (b) a
/// release entry for exactly that version.
#[test]
fn ac_m6_23_3_metainfo_release_from_tag_version() {
    let dir = fresh_dir("metainfo-tag-version");
    let (output, written) = run_metainfo(&dir, Some("1795780800"));
    assert!(
        output.status.success(),
        "(a) metainfo failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        written.contains("<release version=\"0.6.0\" date=\"2026-11-27\"/>"),
        "(b) no release entry for 0.6.0:\n{written}"
    );
}

/// AC-M6-23-3 (b): without `SOURCE_DATE_EPOCH` the release is dated with the commit the build
/// comes from.
#[test]
fn ac_m6_23_3_release_date_is_the_commit_date() {
    let dir = fresh_dir("metainfo-commit-date");
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&dir)
            .env("HOME", &dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_DATE", "2026-03-04T10:00:00+00:00")
            .env("GIT_COMMITTER_DATE", "2026-03-04T10:00:00+00:00")
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "--quiet"]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "--quiet",
        "--allow-empty",
        "--no-gpg-sign",
        "-m",
        "release",
    ]);
    let (output, written) = run_metainfo(&dir, None);
    assert!(
        output.status.success(),
        "metainfo failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        written.contains("<release version=\"0.6.0\" date=\"2026-03-04\"/>"),
        "release not dated with the commit:\n{written}"
    );
}
