// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

mod common;

use kestic_test_support::workspace::workspace_root;
use std::collections::BTreeSet;
use std::fs;

const CXX_QT_CRATES: [&str; 4] = ["cxx-qt", "cxx-qt-lib", "cxx-qt-lib-extras", "cxx-qt-build"];

/// Version requirement of a workspace dependency, written as `"x"` or `{ version = "x" }`.
fn workspace_requirement(dependencies: &toml::Table, name: &str) -> String {
    let entry = dependencies
        .get(name)
        .unwrap_or_else(|| panic!("{name} missing in [workspace.dependencies]"));
    let requirement = match entry {
        toml::Value::Table(table) => table.get("version"),
        other => Some(other),
    };
    requirement
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("{name} has no version requirement"))
        .to_owned()
}

/// AC-M0-03-1: the CXX-Qt crates share one version in Cargo.toml (a), Cargo.lock
/// resolves them to one version (b), and ADR-0001 justifies 0.10.0 over 0.8.1 (c).
#[test]
fn ac_m0_03_1_cxx_qt_crates_share_one_version_and_are_documented() {
    let root = workspace_root();

    let manifest: toml::Table = fs::read_to_string(root.join("Cargo.toml"))
        .expect("read root Cargo.toml")
        .parse()
        .expect("parse root Cargo.toml");
    let dependencies = manifest["workspace"]["dependencies"]
        .as_table()
        .expect("[workspace.dependencies]");
    let requirements: BTreeSet<String> = CXX_QT_CRATES
        .iter()
        .map(|name| workspace_requirement(dependencies, name))
        .collect();
    assert_eq!(
        requirements.len(),
        1,
        "(a) the CXX-Qt crates must require one version, got {requirements:?}"
    );

    let lockfile: toml::Table = fs::read_to_string(root.join("Cargo.lock"))
        .expect("read Cargo.lock")
        .parse()
        .expect("parse Cargo.lock");
    let locked: BTreeSet<(String, String)> = lockfile["package"]
        .as_array()
        .expect("[[package]] in Cargo.lock")
        .iter()
        .filter_map(|package| {
            let name = package["name"].as_str()?;
            let version = package["version"].as_str()?;
            CXX_QT_CRATES
                .contains(&name)
                .then(|| (name.to_owned(), version.to_owned()))
        })
        .collect();
    let locked_names: BTreeSet<&str> = locked.iter().map(|(name, _)| name.as_str()).collect();
    let locked_versions: BTreeSet<&str> =
        locked.iter().map(|(_, version)| version.as_str()).collect();
    assert_eq!(
        locked_names,
        BTreeSet::from(CXX_QT_CRATES),
        "(b) Cargo.lock must contain every CXX-Qt crate"
    );
    assert_eq!(
        locked_versions.len(),
        1,
        "(b) Cargo.lock must resolve the CXX-Qt crates to one version, got {locked:?}"
    );

    let adr =
        fs::read_to_string(root.join("docs/spec/adr/0001-tech-stack-rust-cxx-qt-kirigami.md"))
            .expect("read ADR-0001");
    assert!(
        adr.contains("0.8.1"),
        "(c) ADR-0001 must name the rejected 0.8.1"
    );
    assert!(
        adr.contains("0.10.0"),
        "(c) ADR-0001 must justify the chosen 0.10.0"
    );
}
