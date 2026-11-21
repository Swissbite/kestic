// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Workspace lookups: workspace root, target directory, source file listing and the compiled
//! `kestic` binary.

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
        .map_or_else(|| workspace_root().join("target"), PathBuf::from)
}

/// Locates the compiled `kestic` binary with the test harness (`CARGO_BIN_EXE_kestic` or
/// `target/debug/kestic`, built with `--features kestic-gui/test-harness`). Panics with a build
/// hint when it is missing or was built without the harness, instead of letting GUI smoke tests
/// start the plain product and hang.
pub fn find_kestic_binary() -> PathBuf {
    static FOUND: OnceLock<PathBuf> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let path = env::var_os("CARGO_BIN_EXE_kestic")
                .filter(|v| !v.is_empty())
                .map_or_else(
                    || find_target_dir().join("debug").join("kestic"),
                    PathBuf::from,
                );
            let hint =
                "build it with `cargo build --bin kestic --features kestic-gui/test-harness`";
            assert!(
                path.is_file(),
                "kestic binary not found at {}; {hint}",
                path.display()
            );
            assert!(
                file_contains(&path, b"--smoke-m1"),
                "{} was built without the test harness; {hint}",
                path.display()
            );
            path
        })
        .clone()
}

/// Whether the file at `path` contains `needle`, read in chunks.
pub fn file_contains(path: &Path, needle: &[u8]) -> bool {
    let mut file = fs::File::open(path).expect("open binary");
    let mut window: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut chunk).expect("read binary");
        if n == 0 {
            return false;
        }
        window.extend_from_slice(&chunk[..n]);
        if window.windows(needle.len()).any(|w| w == needle) {
            return true;
        }
        let keep = window.len().min(needle.len() - 1);
        window.drain(..window.len() - keep);
    }
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

#[cfg(test)]
mod tests {
    use super::file_contains;
    use std::fs;

    #[test]
    fn file_contains_finds_needle_at_start_end_and_across_chunk_boundary() {
        let dir = tempfile::tempdir().expect("tmp");
        let needle = b"--smoke-m1";
        let write = |name: &str, at: Option<usize>| {
            let mut data = vec![b'x'; (1 << 20) + 64];
            if let Some(pos) = at {
                data[pos..pos + needle.len()].copy_from_slice(needle);
            }
            let path = dir.path().join(name);
            fs::write(&path, data).expect("write");
            path
        };
        let last = (1 << 20) + 64 - needle.len();
        // (a) at the start of the file
        assert!(file_contains(&write("start", Some(0)), needle));
        // (b) straddling the first 1 MiB chunk boundary
        assert!(file_contains(
            &write("boundary", Some((1 << 20) - 4)),
            needle
        ));
        // (c) at the very end of the file
        assert!(file_contains(&write("end", Some(last)), needle));
        // (d) absent
        assert!(!file_contains(&write("absent", None), needle));
    }
}
