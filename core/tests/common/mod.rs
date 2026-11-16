// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

#![allow(dead_code)]

use std::env;
use std::path::PathBuf;

/// Resolves the `kestic-dev/bin` directory populated by `scripts/bootstrap.sh`.
pub fn dev_tools_bin_dir() -> PathBuf {
    if let Some(dir) = env::var_os("KESTIC_TOOLS_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(data) = env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(data).join("kestic-dev").join("bin");
    }
    let home = env::var_os("HOME").expect("HOME must be set for integration tests");
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("kestic-dev")
        .join("bin")
}

/// Returns `(label, binary_path)` pairs for both restic 0.18.0 and latest (`AC-M1-Q-2`).
pub fn restic_test_binaries() -> Vec<(&'static str, PathBuf)> {
    let dir = dev_tools_bin_dir();
    let v018 = env::var_os("KESTIC_RESTIC_0_18")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("restic-0.18.0"));
    let vlatest = env::var_os("KESTIC_RESTIC_LATEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("restic-latest"));

    assert!(
        v018.exists(),
        "restic-0.18.0 missing at {} (run scripts/bootstrap.sh)",
        v018.display()
    );
    assert!(
        vlatest.exists(),
        "restic-latest missing at {} (run scripts/bootstrap.sh)",
        vlatest.display()
    );

    vec![("restic-0.18.0", v018), ("restic-latest", vlatest)]
}

/// Returns the `rest-server` test binary installed by `scripts/bootstrap.sh`.
pub fn rest_server_binary() -> PathBuf {
    let bin = env::var_os("KESTIC_REST_SERVER")
        .map(PathBuf::from)
        .unwrap_or_else(|| dev_tools_bin_dir().join("rest-server"));
    assert!(
        bin.exists(),
        "rest-server missing at {} (run scripts/bootstrap.sh)",
        bin.display()
    );
    bin
}
