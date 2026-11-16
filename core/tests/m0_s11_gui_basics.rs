// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

#![forbid(unsafe_code)]

//! Integration GUI smoke tests for M0-S11 (AC-M0-11-1, AC-M0-11-7 EN, AC-M0-11-8).

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

fn run_isolated_smoke(
    root: &Path,
    tag: &str,
    lang: &str,
    args: &[&str],
    screenshot_rel: Option<&str>,
) {
    let kestic = find_kestic_binary(root);
    let temp = root.join(format!(
        "target/test-tmp-m0-s11-{tag}-{lang}-{}",
        std::process::id()
    ));
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

    let mut cmd = Command::new(kestic);
    cmd.arg("--smoke-test")
        .args(["--smoke-lang", lang])
        .args(args);
    let screenshot_path = screenshot_rel.map(|rel| root.join(rel));
    if let Some(ref p) = screenshot_path {
        if p.exists() {
            fs::remove_file(p).expect("remove stale screenshot");
        }
        cmd.arg("--screenshot").arg(p);
    }

    let output = cmd
        .env("HOME", temp.join("home"))
        .env("XDG_CONFIG_HOME", temp.join("cfg"))
        .env("XDG_DATA_HOME", temp.join("data"))
        .env("XDG_CACHE_HOME", temp.join("cache"))
        .env("XDG_STATE_HOME", temp.join("state"))
        .env("XDG_RUNTIME_DIR", temp.join("run"))
        .env("QT_QPA_PLATFORM", "offscreen")
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", "en_US.UTF-8")
        .env("LANGUAGE", lang)
        .output()
        .expect("run kestic offscreen smoke check");

    assert!(
        output.status.success(),
        "GUI smoke `{tag}` ({lang}) failed (status={:?}):\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "GUI smoke `{tag}` ({lang}) emitted QML/Qt warnings on stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    if let Some(p) = screenshot_path {
        let png = fs::read(&p).expect("read smoke PNG artifact");
        assert!(
            png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 1024,
            "screenshot {} must be a valid non-trivial PNG, got {} bytes",
            p.display(),
            png.len()
        );
    }
}

/// AC-M0-11-1 & AC-M0-11-7 (EN + DE): Verifies `Kirigami.NavigationTabBar` (5 areas), `pageStack`
/// columns, and `Breadcrumb` header load cleanly offscreen in wide (900x600) and minimum narrow
/// (`26 * gridUnit` width) modes in both English and German (`Wiederherstellen`), with zero
/// truncated tab labels and PNG artifacts saved.
#[test]
fn ac_m0_11_1_and_11_7_navigation_tab_bar_columns_breadcrumb_en_and_de() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has workspace root parent");

    run_isolated_smoke(
        root,
        "wide",
        "en",
        &["--smoke-layout", "wide"],
        Some("target/smoke-artifacts/m0-s11-wide.png"),
    );
    run_isolated_smoke(
        root,
        "narrow",
        "en",
        &["--smoke-layout", "narrow"],
        Some("target/smoke-artifacts/m0-s11-narrow-en.png"),
    );
    run_isolated_smoke(
        root,
        "narrow",
        "de",
        &["--smoke-layout", "narrow"],
        Some("target/smoke-artifacts/m0-s11-narrow-de.png"),
    );
}

/// AC-M0-11-8 (U12): Sends simulated `Ctrl+Shift+B` key press/release to the Kirigami window
/// offscreen and verifies it triggers the "Back Up Now" action.
#[test]
fn ac_m0_11_8_ctrl_shift_b_triggers_back_up_now() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has workspace root parent");

    run_isolated_smoke(root, "shortcut", "en", &["--smoke-shortcut"], None);
}
