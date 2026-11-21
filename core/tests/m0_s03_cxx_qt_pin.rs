// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

mod common;

use common::workspace::workspace_root;
use std::fs;

/// AC-M0-03-1: CXX-Qt version is pinned with `=0.10.0` in Cargo.toml,
/// resolved to `0.10.0` in Cargo.lock, and justified in ADR-0001.
#[test]
fn ac_m0_03_1_cxx_qt_is_pinned_and_documented() {
    let root = workspace_root();

    let root_cargo = fs::read_to_string(root.join("Cargo.toml")).expect("read root Cargo.toml");
    assert!(
        root_cargo.contains("cxx-qt = \"=0.10.0\""),
        "Cargo.toml must pin cxx-qt to =0.10.0"
    );
    assert!(
        root_cargo.contains("cxx-qt-build = \"=0.10.0\""),
        "Cargo.toml must pin cxx-qt-build to =0.10.0"
    );

    let lockfile = fs::read_to_string(root.join("Cargo.lock")).expect("read Cargo.lock");
    assert!(
        lockfile.contains("name = \"cxx-qt\"\nversion = \"0.10.0\""),
        "Cargo.lock must resolve cxx-qt 0.10.0"
    );

    let adr =
        fs::read_to_string(root.join("docs/spec/adr/0001-tech-stack-rust-cxx-qt-kirigami.md"))
            .expect("read ADR-0001");
    assert!(
        adr.contains("0.8.1"),
        "ADR-0001 must name the rejected 0.8.1"
    );
    assert!(
        adr.contains("0.10.0"),
        "ADR-0001 must justify the chosen 0.10.0"
    );
}
