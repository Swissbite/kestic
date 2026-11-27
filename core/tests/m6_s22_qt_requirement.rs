// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Unit test for M6-S22: the specification and the showcase site name the same Qt requirement.

use kestic_test_support::workspace::workspace_root;
use std::fs;

/// The Qt versions a text requires: the number after «Qt ≥ » and before « or newer» in
/// «Qt 6.10 or newer».
fn qt_requirements(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (i, _) in text.match_indices("Qt ") {
        let rest = &text[i + 3..];
        let (rest, from_ge) = match rest.strip_prefix("≥ ") {
            Some(r) => (r, true),
            None => (rest, false),
        };
        let version: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let version = version.trim_end_matches('.').to_string();
        if version.is_empty() {
            continue;
        }
        let after = &rest[version.len()..];
        if from_ge || after.starts_with(" or newer") {
            found.push(version);
        }
    }
    found
}

/// Every version-like mention of Qt in a text: «Qt 6.10.2», «Qt ≥ 6.10», «Qt 6.10 or newer».
fn qt_versions_mentioned(text: &str) -> Vec<String> {
    text.match_indices("Qt ")
        .filter_map(|(i, _)| {
            let rest = &text[i + 3..];
            let rest = rest.strip_prefix("≥ ").unwrap_or(rest);
            let version: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let version = version.trim_end_matches('.').to_string();
            version.contains('.').then_some(version)
        })
        .collect()
}

/// AC-M6-22-3 (a): NFR-01, the packaging part of architecture.md and the showcase site name the
/// same Qt requirement, Qt 6.10 or newer; the site names no other Qt version.
#[test]
fn ac_m6_22_3_qt_requirement_same_everywhere() {
    let root = workspace_root();
    let read =
        |rel: &str| fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));

    let requirements = read("docs/spec/requirements.md");
    let nfr_01 = requirements
        .lines()
        .find(|l| l.starts_with("| NFR-01 |"))
        .expect("NFR-01 row in requirements.md");
    let architecture = read("docs/spec/architecture.md");
    let native_packages = architecture
        .split("\n## ")
        .find(|s| s.starts_with("Native Pakete"))
        .expect("architecture.md §Native Pakete");
    let site = read("site/src/pages/index.astro");

    // (a) NFR-01
    assert_eq!(qt_requirements(nfr_01), ["6.10"], "NFR-01: {nfr_01}");
    // (a) architecture.md, packaging of the native packages
    assert_eq!(
        qt_requirements(native_packages),
        ["6.10"],
        "architecture.md §Native Pakete"
    );
    // (a) site: every Qt version it names is the requirement «Qt 6.10 or newer»
    let site_requirements = qt_requirements(&site);
    assert!(
        !site_requirements.is_empty(),
        "the site names no Qt requirement"
    );
    assert_eq!(
        qt_versions_mentioned(&site),
        site_requirements,
        "the site names a Qt version that is not the requirement"
    );
    assert!(
        site_requirements.iter().all(|v| v == "6.10"),
        "the site requires Qt {site_requirements:?}, not 6.10"
    );
}
