// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! What a build stamps into the `kestic` binary: the version, derived from Git tags, and the
//! commit of the source tree. Used by the daemon's build script and, through the binary of this
//! crate, by `just deb` for the package version and the release entry and screenshots of the AppStream
//! metainfo.

pub mod commit;
pub mod metainfo;
pub mod screenshots;
pub mod version;
