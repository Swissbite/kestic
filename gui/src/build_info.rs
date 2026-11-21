// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `BuildInfo` QML singleton: version and build commit of this GUI, commit links and whether
//! the background service runs another build. The logic lives in `kestic_core::build_info`.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        /// Build of the running GUI. No daemon connection.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type BuildInfo = super::BuildInfoRust;
    }

    extern "RustQt" {
        /// Version of this GUI, e.g. `0.1.0`.
        #[qinvokable]
        #[cxx_name = "appVersion"]
        fn app_version(self: &BuildInfo) -> QString;

        /// Build commit of this GUI: short hash, hash with `-dirty`, or `unknown`.
        #[qinvokable]
        #[cxx_name = "appCommit"]
        fn app_commit(self: &BuildInfo) -> QString;

        /// The commit as it is shown, or an empty text when it names none.
        #[qinvokable]
        #[cxx_name = "shownCommit"]
        fn shown_commit(self: &BuildInfo, commit: &QString) -> QString;

        /// Link to the commit in the public repository, or an empty text without one.
        #[qinvokable]
        #[cxx_name = "commitUrl"]
        fn commit_url(self: &BuildInfo, commit: &QString) -> QString;

        /// Whether a process with `version` and `commit` runs another build than this GUI.
        #[qinvokable]
        #[cxx_name = "differsFromApp"]
        fn differs_from_app(self: &BuildInfo, version: &QString, commit: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "homepageUrl"]
        fn homepage_url(self: &BuildInfo) -> QString;

        #[qinvokable]
        #[cxx_name = "issuesUrl"]
        fn issues_url(self: &BuildInfo) -> QString;
    }
}

use cxx_qt_lib::QString;
use kestic_core::build_info::{BuildId, HOMEPAGE_URL, ISSUES_URL};

/// Backing struct of the `BuildInfo` QML singleton.
#[derive(Default)]
pub struct BuildInfoRust;

/// The GUI and the daemon are one binary: the daemon crate records the build commit for both.
fn app_build() -> BuildId<'static> {
    kestic_daemon::build_id()
}

/// Calls `f` with a build that has only `commit`; the commit rules need no version.
fn with_commit<R>(commit: &QString, f: impl FnOnce(&BuildId<'_>) -> R) -> R {
    let commit = commit.to_string();
    f(&BuildId {
        version: "",
        commit: &commit,
    })
}

impl qobject::BuildInfo {
    pub fn app_version(&self) -> QString {
        QString::from(app_build().version)
    }

    pub fn app_commit(&self) -> QString {
        QString::from(app_build().commit)
    }

    pub fn shown_commit(&self, commit: &QString) -> QString {
        with_commit(commit, |build| {
            QString::from(build.shown_commit().unwrap_or_default())
        })
    }

    pub fn commit_url(&self, commit: &QString) -> QString {
        with_commit(commit, |build| {
            QString::from(&build.commit_url().unwrap_or_default())
        })
    }

    pub fn differs_from_app(&self, version: &QString, commit: &QString) -> bool {
        let (version, commit) = (version.to_string(), commit.to_string());
        app_build().differs_from(&BuildId {
            version: &version,
            commit: &commit,
        })
    }

    pub fn homepage_url(&self) -> QString {
        QString::from(HOMEPAGE_URL)
    }

    pub fn issues_url(&self) -> QString {
        QString::from(ISSUES_URL)
    }
}
