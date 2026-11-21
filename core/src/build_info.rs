// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Version and build commit of a Kestic process, as shown in «About Kestic» and the tray.
//!
//! The commit is the label the build script records: a short hash, the hash with `-dirty`, or
//! `unknown`. GUI and background service can run different builds, e.g. after a package update
//! replaced the binary and only the background service restarted into it.

/// Public source repository; commit links point into it.
pub const REPOSITORY_URL: &str = "https://github.com/Swissbite/kestic";

/// Project homepage.
pub const HOMEPAGE_URL: &str = "https://swissbite.github.io/kestic/";

/// Where users report bugs.
pub const ISSUES_URL: &str = "https://github.com/Swissbite/kestic/issues";

/// Version and build commit of one running Kestic process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildId<'a> {
    pub version: &'a str,
    pub commit: &'a str,
}

impl BuildId<'_> {
    /// The commit as a plain Git hash, or `None` for `unknown`, `-dirty` or an empty label (an
    /// older background service that does not report one).
    fn clean_commit(&self) -> Option<&str> {
        let is_hash = (7..=40).contains(&self.commit.len())
            && self.commit.bytes().all(|b| b.is_ascii_hexdigit());
        is_hash.then_some(self.commit)
    }

    /// The commit as it is shown, clean or `-dirty`; `None` for `unknown` or an empty label.
    pub fn shown_commit(&self) -> Option<&str> {
        let known = !self.commit.is_empty() && self.commit != "unknown";
        known.then_some(self.commit)
    }

    /// Link to the commit in the public repository. Only a clean hash gets one: a `-dirty` build
    /// contains changes that are in no commit, and `unknown` names none.
    pub fn commit_url(&self) -> Option<String> {
        self.clean_commit()
            .map(|hash| format!("{REPOSITORY_URL}/commit/{hash}"))
    }

    /// `0.1.0 · a561018`; only the version when the commit is not known.
    pub fn line(&self) -> String {
        match self.shown_commit() {
            Some(commit) => format!("{} · {commit}", self.version),
            None => self.version.to_owned(),
        }
    }

    /// The product name in front of [`Self::line`]: `Kestic 0.1.0 · a561018`. A name and two
    /// identifiers, the same in every language.
    pub fn product_line(&self) -> String {
        format!("Kestic {}", self.line())
    }

    /// Whether `other` is a different build: another version, or another commit when both
    /// name one. An empty version (background service not reachable) never differs, and a
    /// build without a known commit is compared by version only.
    pub fn differs_from(&self, other: &BuildId<'_>) -> bool {
        if self.version.is_empty() || other.version.is_empty() {
            return false;
        }
        if self.version != other.version {
            return true;
        }
        match (self.shown_commit(), other.shown_commit()) {
            (Some(mine), Some(theirs)) => mine != theirs,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build<'a>(version: &'a str, commit: &'a str) -> BuildId<'a> {
        BuildId { version, commit }
    }

    /// AC-M1-13-9 (b): a clean short hash links to its commit page.
    #[test]
    fn clean_hash_links_to_the_commit() {
        assert_eq!(
            build("0.1.0", "a561018").commit_url().as_deref(),
            Some("https://github.com/Swissbite/kestic/commit/a561018")
        );
    }

    /// AC-M1-13-9 (c): (a) `-dirty`, (b) `unknown`, (c) an empty label and (d) a non-hex label get
    /// no link.
    #[test]
    fn dirty_unknown_and_empty_commits_get_no_link() {
        assert_eq!(build("0.1.0", "a561018-dirty").commit_url(), None, "(a)");
        assert_eq!(build("0.1.0", "unknown").commit_url(), None, "(b)");
        assert_eq!(build("0.1.0", "").commit_url(), None, "(c)");
        assert_eq!(build("0.1.0", "zzzzzzz").commit_url(), None, "(d)");
    }

    /// AC-M1-11-6 (b), AC-M1-13-9: the line shows (a) version and commit, (b) a `-dirty` commit as it is, and (c) only the
    /// version without a known commit.
    #[test]
    fn line_names_version_and_known_commit() {
        assert_eq!(build("0.1.0", "a561018").line(), "0.1.0 · a561018", "(a)");
        assert_eq!(
            build("0.1.0", "a561018-dirty").line(),
            "0.1.0 · a561018-dirty",
            "(b)"
        );
        assert_eq!(build("0.1.0", "unknown").line(), "0.1.0", "(c)");
    }

    /// AC-M1-11-6 (b): the tray line puts «Kestic» in front of the version line.
    #[test]
    fn product_line_names_kestic() {
        assert_eq!(
            build("0.1.0", "a561018").product_line(),
            "Kestic 0.1.0 · a561018"
        );
    }

    /// AC-M1-13-9 (a): (a) another version differs; (b) another commit of the same version differs;
    /// (c) the same build does not.
    #[test]
    fn other_version_or_commit_is_a_different_build() {
        let gui = build("0.1.0", "a561018");
        assert!(gui.differs_from(&build("0.2.0", "a561018")), "(a)");
        assert!(gui.differs_from(&build("0.1.0", "b7c8d9e")), "(b)");
        assert!(!gui.differs_from(&build("0.1.0", "a561018")), "(c)");
    }

    /// AC-M1-13-9 (a): no difference is claimed (a) while the background service is not reachable (empty
    /// version), (b) when one side has no known commit and the versions match, or (c) when an
    /// older background service reports no commit; (d) a version change still counts then.
    #[test]
    fn missing_information_is_not_a_difference() {
        let gui = build("0.1.0", "a561018");
        assert!(!gui.differs_from(&build("", "")), "(a)");
        assert!(!gui.differs_from(&build("0.1.0", "unknown")), "(b)");
        assert!(!gui.differs_from(&build("0.1.0", "")), "(c)");
        assert!(gui.differs_from(&build("0.2.0", "")), "(d)");
    }

    /// AC-M1-13-9 (a): a `-dirty` build and the clean build of the same commit are different builds.
    #[test]
    fn dirty_build_differs_from_clean_build() {
        assert!(build("0.1.0", "a561018").differs_from(&build("0.1.0", "a561018-dirty")));
    }
}
