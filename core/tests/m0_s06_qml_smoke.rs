// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

#![forbid(unsafe_code)]

//! Verification test for M0-S06: offscreen QML smoke test and screenshot artifact in `just verify`.

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
    panic!("kestic binary not found under {}", target_dir.display());
}

/// AC-M0-06-1: With `QT_QPA_PLATFORM=offscreen`, `just verify` invokes the GUI smoke test,
/// loads the main window with zero QML errors/warnings, and writes a PNG screenshot artifact.
#[test]
fn ac_m0_06_1_offscreen_qml_smoke_and_screenshot_in_just_verify() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has workspace root parent");

    let justfile = fs::read_to_string(root.join("justfile")).expect("read justfile");
    assert!(
        justfile.contains("just smoke"),
        "justfile verify recipe must run `just smoke`"
    );
    assert!(
        justfile.contains("QT_QPA_PLATFORM=offscreen"),
        "justfile smoke recipe must run with QT_QPA_PLATFORM=offscreen"
    );

    let kestic = find_kestic_binary(root);
    let temp = root.join(format!("target/test-tmp-m0-s06-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    for sub in ["home", "cfg", "data", "cache", "state", "run"] {
        fs::create_dir_all(temp.join(sub)).expect("create isolated XDG dir");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temp.join("run"), fs::Permissions::from_mode(0o700))
            .expect("chmod 0700 XDG_RUNTIME_DIR");
    }
    let _guard = TempDirGuard(temp.clone());

    let screenshot = root.join("target/smoke-artifacts/m0-s06-main.png");
    let output = Command::new(kestic)
        .args(["--smoke-test", "--smoke-lang", "en", "--screenshot"])
        .arg(&screenshot)
        .env("HOME", temp.join("home"))
        .env("XDG_CONFIG_HOME", temp.join("cfg"))
        .env("XDG_DATA_HOME", temp.join("data"))
        .env("XDG_CACHE_HOME", temp.join("cache"))
        .env("XDG_STATE_HOME", temp.join("state"))
        .env("XDG_RUNTIME_DIR", temp.join("run"))
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", "C.UTF-8")
        .env("LANGUAGE", "en")
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .expect("run kestic offscreen smoke test");

    assert!(
        output.status.success(),
        "QML smoke test failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "QML smoke test must not emit QML warnings/errors on stderr, got:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let png = fs::read(&screenshot).expect("read screenshot artifact");
    assert!(
        png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 1024,
        "screenshot artifact must be a valid non-trivial PNG, got {} bytes",
        png.len()
    );
}
