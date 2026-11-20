// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Integration smoke test for M0-S04: Kirigami window with Rust `SpikeListModel`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn find_kestic_binary(workspace_root: &Path) -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"));

    for profile in ["debug", "release"] {
        let candidate = target_dir.join(profile).join("kestic");
        if candidate.is_file() {
            return candidate;
        }
    }

    panic!(
        "kestic binary not found under {}; run `cargo build --bin kestic` or `just verify` first",
        target_dir.display()
    );
}

/// AC-M0-04-1: Launches the Kirigami window offscreen with `SpikeListModel` (3 entries),
/// verifies QML warnings/count/titles inside the app smoke check, captures a PNG screenshot
/// artifact, and checks that CXX-Qt is pinned (AC-M0-03-1).
#[test]
fn ac_m0_04_1_kirigami_window_displays_three_rust_list_model_entries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has workspace root parent");
    let root_cargo = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    assert!(
        root_cargo.contains("cxx-qt = \"=0.10.0\""),
        "AC-M0-03-1 / AC-M0-04-1: cxx-qt must be pinned to =0.10.0"
    );

    let kestic_bin = find_kestic_binary(root);
    let temp = root.join(format!("target/test-tmp-m0-s04-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).expect("create temp root");
    let _guard = TempDirGuard(temp.clone());

    let home = temp.join("home");
    let xdg_cfg = temp.join("cfg");
    let xdg_data = temp.join("data");
    let xdg_cache = temp.join("cache");
    let xdg_state = temp.join("state");
    let xdg_run = temp.join("run");
    for dir in [&home, &xdg_cfg, &xdg_data, &xdg_cache, &xdg_state, &xdg_run] {
        fs::create_dir_all(dir).expect("create isolated XDG directory");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&xdg_run, fs::Permissions::from_mode(0o700))
            .expect("chmod 0700 XDG_RUNTIME_DIR");
    }

    let screenshot_path = root.join("target/smoke-artifacts/m0-s04-listmodel.png");
    if screenshot_path.exists() {
        fs::remove_file(&screenshot_path).expect("remove stale screenshot");
    }

    let output = Command::new(&kestic_bin)
        .args(["--smoke-test", "--smoke-lang", "en", "--screenshot"])
        .arg(&screenshot_path)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &xdg_cfg)
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_CACHE_HOME", &xdg_cache)
        .env("XDG_STATE_HOME", &xdg_state)
        .env("XDG_RUNTIME_DIR", &xdg_run)
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", "C.UTF-8")
        .env("LANGUAGE", "en")
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .expect("run kestic --smoke-test offscreen");

    assert!(
        output.status.success(),
        "kestic --smoke-test failed (status={})\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let png_bytes = fs::read(&screenshot_path).expect("read captured PNG screenshot");
    assert!(
        png_bytes.starts_with(b"\x89PNG\r\n\x1a\n") && png_bytes.len() > 1024,
        "screenshot must be a valid non-trivial PNG image, got {} bytes",
        png_bytes.len()
    );
}
