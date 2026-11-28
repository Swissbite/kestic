// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M6-S36: Qt Test sends the smoke's input events and is linked only by the test harness, never
//! by the `kestic` in the `.deb`.

use super::{control_field, extract_shipped_binary, find_deb_package};
use kestic_test_support::workspace::find_kestic_binary;
use std::path::Path;
use std::process::Command;

/// The `NEEDED` entries of the dynamic section of `binary` (`readelf -d`).
fn needed_libraries(binary: &Path) -> Vec<String> {
    let out = Command::new("readelf")
        .arg("-d")
        .arg(binary)
        .output()
        .expect("run readelf -d");
    assert!(
        out.status.success(),
        "readelf -d failed on {}",
        binary.display()
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains("(NEEDED)"))
        .filter_map(|l| l.split('[').nth(1)?.split(']').next().map(str::to_string))
        .collect()
}

fn links_qt_test(binary: &Path) -> bool {
    needed_libraries(binary)
        .iter()
        .any(|lib| lib.starts_with("libQt6Test"))
}

/// AC-M6-36-6 (b): the binary with the test harness links Qt Test (so the check below can see
/// it).
#[test]
fn ac_m6_36_6_b_test_harness_binary_links_qt_test() {
    let binary = find_kestic_binary();
    assert!(
        links_qt_test(&binary),
        "AC-M6-36-6 (b): {} lists libQt6Test as NEEDED: {:?}",
        binary.display(),
        needed_libraries(&binary)
    );
}

/// AC-M6-36-6 (c), (d): the `kestic` in the `.deb` does not link Qt Test, and the package does not
/// depend on it.
#[test]
fn ac_m6_36_6_c_d_deb_without_qt_test() {
    let (_extracted, shipped) = extract_shipped_binary("kestic-m6-s36-qt-test");
    assert!(
        !links_qt_test(&shipped),
        "AC-M6-36-6 (c): the shipped kestic lists libQt6Test: {:?}",
        needed_libraries(&shipped)
    );
    let out = Command::new("dpkg-deb")
        .arg("-f")
        .arg(find_deb_package())
        .output()
        .expect("run dpkg-deb -f");
    assert!(out.status.success(), "dpkg-deb -f failed");
    let control = String::from_utf8_lossy(&out.stdout);
    let depends = control_field(&control, "Depends");
    assert!(!depends.is_empty(), "precondition: Depends is read");
    assert!(
        !depends.iter().any(|p| p.starts_with("libqt6test")),
        "AC-M6-36-6 (d): Depends names no libqt6test package: {depends:?}"
    );
}
