// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Workspace lookups: workspace root, target directory, source file listing and the compiled
//! `kestic` binary.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// The Cargo workspace root (every crate sits directly below it).
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has workspace root parent")
        .to_path_buf()
}

/// The Cargo target directory (`CARGO_TARGET_DIR` or `<workspace>/target`).
pub fn find_target_dir() -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"))
}

/// Locates the compiled `kestic` binary (`CARGO_BIN_EXE_kestic`, `target/debug/kestic`, or
/// `target/release/kestic`).
pub fn find_kestic_binary() -> PathBuf {
    if let Some(p) = env::var_os("CARGO_BIN_EXE_kestic").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(p);
        if path.is_file() {
            return path;
        }
    }
    let target_dir = find_target_dir();
    for profile in ["debug", "release"] {
        let candidate = target_dir.join(profile).join("kestic");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "kestic binary not found under {}; build it with `cargo build --bin kestic`",
        target_dir.display()
    );
}

/// All files with extension `ext` below `dir`, sorted.
pub fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("read source dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, ext, out);
            } else if path.extension().is_some_and(|e| e == ext) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, ext, &mut out);
    out.sort();
    out
}
