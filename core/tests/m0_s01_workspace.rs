// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has workspace root parent")
        .to_path_buf()
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// AC-M0-01-1 (Q1): Workspace layout with core, daemon, gui, rust-toolchain.toml,
/// justfile, deny.toml, and LICENSES/ exists.
#[test]
fn ac_m0_01_1_workspace_layout_exists() {
    let root = workspace_root();
    for rel in [
        "Cargo.toml",
        "rust-toolchain.toml",
        "justfile",
        "deny.toml",
        "core/Cargo.toml",
        "daemon/Cargo.toml",
        "gui/Cargo.toml",
        "LICENSES/GPL-2.0-only.txt",
        "LICENSES/GPL-3.0-only.txt",
        "LICENSES/LicenseRef-KDE-Accepted-GPL.txt",
        "LICENSES/BSD-2-Clause.txt",
        "LICENSES/CC-BY-SA-4.0.txt",
        "LICENSES/CC0-1.0.txt",
    ] {
        assert!(
            root.join(rel).exists(),
            "missing expected workspace file: {rel}"
        );
    }
}

/// AC-M0-01-2: A new file without an SPDX header causes `reuse lint` to fail;
/// adding valid SPDX headers makes `reuse lint` succeed.
#[test]
fn ac_m0_01_2_reuse_lint_fails_without_spdx_header() {
    let root = workspace_root();
    let temp = TempDir::new("kestic-ac-m0-01-2");
    let licenses_dir = temp.path().join("LICENSES");
    fs::create_dir_all(&licenses_dir).expect("create LICENSES dir");
    fs::copy(
        root.join("LICENSES/CC0-1.0.txt"),
        licenses_dir.join("CC0-1.0.txt"),
    )
    .expect("copy CC0-1.0.txt");

    let sample_file = temp.path().join("sample.rs");
    fs::write(&sample_file, "pub fn answer() -> u32 { 42 }\n").expect("write sample file");

    let fail_output = Command::new("reuse")
        .arg("--root")
        .arg(temp.path())
        .arg("lint")
        .output()
        .expect("run reuse lint (expected reuse installed)");

    assert!(
        !fail_output.status.success(),
        "AC-M0-01-2 expected reuse lint to fail without SPDX header; stdout={}, stderr={}",
        String::from_utf8_lossy(&fail_output.stdout),
        String::from_utf8_lossy(&fail_output.stderr)
    );

    let licensed_content = concat!(
        "// SPDX-FileCopyrightText",
        ": 2026 David Däster <david@daester.net>\n",
        "// SPDX-License-Identifier",
        ": CC0-1.0\n",
        "pub fn answer() -> u32 { 42 }\n",
    );
    fs::write(&sample_file, licensed_content).expect("write licensed sample file");

    let ok_output = Command::new("reuse")
        .arg("--root")
        .arg(temp.path())
        .arg("lint")
        .output()
        .expect("run reuse lint after adding header");

    assert!(
        ok_output.status.success(),
        "expected reuse lint to pass with SPDX header; stdout={}, stderr={}",
        String::from_utf8_lossy(&ok_output.stdout),
        String::from_utf8_lossy(&ok_output.stderr)
    );
}
