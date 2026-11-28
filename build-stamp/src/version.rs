// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! Version of a build. It comes from Git tags, never from a file in a commit, and has the form of
//! a Debian version so that packages sort the way releases follow each other:
//!
//! 1. `KESTIC_VERSION`, when set (the release workflow sets it from the tag it releases);
//! 2. else `git describe` of the checkout (nearest tag `v<n>.<n>.<n>…`): `v0.4.0` → `0.4.0`,
//!    `v0.4.0-m4.3` → `0.4.0~m4.3` (`~` sorts a pre-release before its release), two commits
//!    after that tag → `0.4.0~m4.3+2.gabc1234`;
//! 3. else `0.0.0~dev+gabc1234` in a checkout without a version tag, `0.0.0~dev` outside one.

use crate::commit::{git, is_own_checkout};
use std::path::Path;

/// Environment variable that sets the version of a build directly.
pub const OVERRIDE_VAR: &str = "KESTIC_VERSION";

/// Version of a build without a version tag.
pub const DEV_VERSION: &str = "0.0.0~dev";

/// The version a build of `workspace_root` gets, `override_value` being the value of
/// [`OVERRIDE_VAR`]. Fails only for an override that is no valid Debian upstream version.
pub fn build_version(
    workspace_root: &Path,
    override_value: Option<&str>,
) -> Result<String, String> {
    match override_value {
        Some(value) if is_debian_version(value) => Ok(value.to_owned()),
        Some(value) => Err(format!(
            "{OVERRIDE_VAR}={value:?} is no Debian version: a digit first, then letters, digits, `.`, `+` or `~`"
        )),
        None => Ok(describe_version(workspace_root)),
    }
}

fn describe_version(workspace_root: &Path) -> String {
    if !is_own_checkout(workspace_root) {
        return DEV_VERSION.to_owned();
    }
    let describe = git(
        workspace_root,
        &[
            "describe",
            "--tags",
            "--long",
            "--abbrev=7",
            "--match",
            "v[0-9]*.[0-9]*.[0-9]*",
            "HEAD",
        ],
    );
    if let Some(version) = describe.as_deref().and_then(from_describe) {
        return version;
    }
    match git(workspace_root, &["rev-parse", "--short=7", "HEAD"]) {
        Some(hash) => format!("{DEV_VERSION}+g{hash}"),
        None => DEV_VERSION.to_owned(),
    }
}

/// Debian version for the output of `git describe --long`: `<tag>-<commits since>-g<hash>`.
/// `None` when the tag is no version tag.
pub fn from_describe(describe: &str) -> Option<String> {
    let mut parts = describe.rsplitn(3, '-');
    let hash = parts.next()?.strip_prefix('g')?;
    let distance: u32 = parts.next()?.parse().ok()?;
    let version = from_tag(parts.next()?)?;
    if hash.is_empty() || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(if distance == 0 {
        version
    } else {
        format!("{version}+{distance}.g{hash}")
    })
}

/// Debian version for a tag `v<major>.<minor>.<patch>[-<pre-release>]`, the pre-release made of
/// dot-separated letters and digits. `None` for any other tag.
pub fn from_tag(tag: &str) -> Option<String> {
    let rest = tag.strip_prefix('v')?;
    let (release, pre_release) = match rest.split_once('-') {
        Some((release, pre_release)) => (release, Some(pre_release)),
        None => (rest, None),
    };
    let numbers: Vec<&str> = release.split('.').collect();
    if numbers.len() != 3 || !numbers.iter().all(|n| is_number(n)) {
        return None;
    }
    match pre_release {
        None => Some(release.to_owned()),
        Some(pre) if pre.split('.').all(is_identifier) => Some(format!("{release}~{pre}")),
        Some(_) => None,
    }
}

/// A number without leading zeros, as in semantic versions.
fn is_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'))
}

fn is_identifier(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A Debian upstream version without epoch and revision, as `cargo deb` gets it before `-1`.
fn is_debian_version(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_digit())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'~'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// architecture.md §Release: (a) a release tag, (b) a pre-release tag of an integration
    /// branch and (c) a release candidate map to Debian versions.
    #[test]
    fn release_tags_map_to_debian_versions() {
        assert_eq!(from_tag("v0.4.0").as_deref(), Some("0.4.0"), "(a)");
        assert_eq!(
            from_tag("v0.4.0-m4.3").as_deref(),
            Some("0.4.0~m4.3"),
            "(b)"
        );
        assert_eq!(
            from_tag("v1.0.0-rc.1").as_deref(),
            Some("1.0.0~rc.1"),
            "(c)"
        );
    }

    /// (a) A tagged commit gets the tag's version; commits after (b) a pre-release tag and (c) a
    /// release tag add their number and the commit.
    #[test]
    fn describe_output_maps_to_debian_versions() {
        assert_eq!(
            from_describe("v0.4.0-m4.3-0-gabc1234").as_deref(),
            Some("0.4.0~m4.3"),
            "(a)"
        );
        assert_eq!(
            from_describe("v0.4.0-m4.3-2-gabc1234").as_deref(),
            Some("0.4.0~m4.3+2.gabc1234"),
            "(b)"
        );
        assert_eq!(
            from_describe("v0.4.0-5-gabc1234").as_deref(),
            Some("0.4.0+5.gabc1234"),
            "(c)"
        );
    }

    /// Tags that are no version tags give no version, so the build falls back to `0.0.0~dev`.
    #[test]
    fn other_tags_give_no_version() {
        for tag in [
            "0.4.0",
            "v0.4",
            "v0.4.0.1",
            "v01.4.0",
            "v0.4.x",
            "v0.4.0-",
            "v0.4.0-m4..3",
            "v0.4.0-m4_3",
        ] {
            assert_eq!(from_tag(tag), None, "{tag}");
        }
        assert_eq!(
            from_describe("v0.4.0-m4.3-2-xabc1234"),
            None,
            "hash without g"
        );
        assert_eq!(
            from_describe("v0.4.0-m4.3-two-gabc1234"),
            None,
            "distance not a number"
        );
        assert_eq!(
            from_describe("v0.4.0-m4.3-2-gxyz"),
            None,
            "hash not hexadecimal"
        );
    }

    /// `KESTIC_VERSION` is taken as it is when it is (a) a release or (b) a pre-release version,
    /// and refused (c) with a revision, (d) with the tag's `v` and (e) empty.
    #[test]
    fn override_must_be_a_debian_version() {
        let root = Path::new("/nonexistent");
        assert_eq!(
            build_version(root, Some("0.4.0")),
            Ok("0.4.0".to_owned()),
            "(a)"
        );
        assert_eq!(
            build_version(root, Some("0.4.0~m4.3")),
            Ok("0.4.0~m4.3".to_owned()),
            "(b)"
        );
        assert!(build_version(root, Some("0.4.0-1")).is_err(), "(c)");
        assert!(build_version(root, Some("v0.4.0")).is_err(), "(d)");
        assert!(build_version(root, Some("")).is_err(), "(e)");
    }

    /// Without override and outside a Git checkout the version is `0.0.0~dev`.
    #[test]
    fn no_checkout_is_dev_version() {
        assert_eq!(
            build_version(Path::new("/nonexistent"), None),
            Ok(DEV_VERSION.to_owned())
        );
    }

    fn dpkg_less_than(a: &str, b: &str) -> bool {
        Command::new("dpkg")
            .args([
                "--compare-versions",
                &format!("{a}-1"),
                "lt",
                &format!("{b}-1"),
            ])
            .status()
            .expect("run dpkg --compare-versions (Debian/Ubuntu tool)")
            .success()
    }

    /// Packages (with revision `-1`) sort in the order of the builds: a development build before
    /// everything, pre-releases of an integration branch by number and with their follow-up
    /// commits, a release candidate after them, the release after its pre-releases, and commits
    /// after a release before the next pre-release.
    #[test]
    fn debian_versions_sort_like_releases() {
        let ordered = [
            "0.0.0~dev+gabc1234".to_owned(),
            from_tag("v0.4.0-m4.3").expect("tag"),
            from_describe("v0.4.0-m4.3-2-gabc1234").expect("describe"),
            from_tag("v0.4.0-m4.9").expect("tag"),
            from_tag("v0.4.0-m4.10").expect("tag"),
            from_tag("v0.4.0-rc.1").expect("tag"),
            from_tag("v0.4.0").expect("tag"),
            from_describe("v0.4.0-3-gabc1234").expect("describe"),
            from_tag("v0.5.0-m5.1").expect("tag"),
        ];
        for pair in ordered.windows(2) {
            assert!(
                dpkg_less_than(&pair[0], &pair[1]),
                "{} < {}",
                pair[0],
                pair[1]
            );
        }
    }
}
