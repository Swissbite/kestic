// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

#![forbid(unsafe_code)]

//! Unit and integration tests for M0-S05: `.deb` packaging, QML Depends, notifyrc, and autostart.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn find_deb_package(workspace_root: &Path) -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"));
    let deb_dir = target_dir.join("debian");
    let entries = fs::read_dir(&deb_dir).unwrap_or_else(|e| {
        panic!(
            "target/debian not readable at {} ({}); run `cargo deb -p kestic-gui --fast` or `just verify` first",
            deb_dir.display(),
            e
        )
    });

    let mut debs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("kestic_") && n.ends_with(".deb"))
        })
        .collect();
    debs.sort();
    debs.pop().unwrap_or_else(|| {
        panic!(
            "no kestic_*.deb found in {}; run `cargo deb -p kestic-gui --fast` or `just verify` first",
            deb_dir.display()
        )
    })
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

/// AC-M0-05-2: Unit check for autostart desktop entry contents (`TryExec=/usr/bin/kestic`,
/// `Exec=kestic daemon --autostart`), desktop-file validation, and immediate exit without config.
#[test]
fn ac_m0_05_2_autostart_entry_contains_tryexec_and_exec() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has workspace root parent");

    let autostart_path = root.join("packaging/net.daester.Kestic.daemon.desktop");
    let gui_desktop_path = root.join("packaging/net.daester.Kestic.desktop");

    let autostart = fs::read_to_string(&autostart_path).expect("read daemon autostart desktop");
    assert!(
        autostart
            .lines()
            .any(|l| l == "Exec=kestic daemon --autostart"),
        "autostart entry must contain Exec=kestic daemon --autostart"
    );
    assert!(
        autostart.lines().any(|l| l == "TryExec=/usr/bin/kestic"),
        "autostart entry must contain TryExec=/usr/bin/kestic"
    );

    let validate = Command::new("desktop-file-validate")
        .arg(&autostart_path)
        .arg(&gui_desktop_path)
        .output()
        .expect("run desktop-file-validate");
    assert!(
        validate.status.success(),
        "desktop-file-validate failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&validate.stdout),
        String::from_utf8_lossy(&validate.stderr)
    );

    // Also verify `kestic daemon --autostart` exits 0 immediately when no config exists (NFR-11).
    let temp = root.join(format!("target/test-tmp-m0-s05-bin-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("cfg")).expect("create cfg dir");
    fs::create_dir_all(temp.join("home")).expect("create home dir");
    let _guard = TempDirGuard(temp.clone());

    let kestic = find_kestic_binary(root);
    let out = Command::new(kestic)
        .args(["daemon", "--autostart"])
        .env("HOME", temp.join("home"))
        .env("XDG_CONFIG_HOME", temp.join("cfg"))
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .expect("run kestic daemon --autostart");
    assert!(
        out.status.success(),
        "kestic daemon --autostart without config must exit 0, got {:?}",
        out.status.code()
    );
}

/// AC-M0-05-1 & AC-M0-05-2: Inspect built `.deb` via `dpkg-deb -I`, `dpkg-deb -c`, and `dpkg-deb -e`
/// for QML Depends, restic Recommends, notifyrc, GUI desktop, and `/etc/xdg/autostart` Conffile.
#[test]
fn ac_m0_05_1_deb_contains_qml_depends_recommends_notifyrc_and_autostart() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has workspace root parent");
    let deb_path = find_deb_package(root);

    let info_out = Command::new("dpkg-deb")
        .arg("-I")
        .arg(&deb_path)
        .output()
        .expect("run dpkg-deb -I");
    assert!(info_out.status.success(), "dpkg-deb -I failed");
    let info = String::from_utf8_lossy(&info_out.stdout);

    for required_dep in [
        "qml6-module-org-kde-kirigami (>= 6.24)",
        "qml6-module-org-kde-kirigamiaddons-datetime (>= 1.11)",
        "qml6-module-org-kde-kirigamiaddons-tableview (>= 1.11)",
    ] {
        assert!(
            info.contains(required_dep),
            ".deb Depends missing `{required_dep}` in:\n{info}"
        );
    }
    assert!(
        info.contains("Recommends: restic (>= 0.18.0)"),
        ".deb missing `Recommends: restic (>= 0.18.0)` in:\n{info}"
    );

    let contents_out = Command::new("dpkg-deb")
        .arg("-c")
        .arg(&deb_path)
        .output()
        .expect("run dpkg-deb -c");
    assert!(contents_out.status.success(), "dpkg-deb -c failed");
    let contents = String::from_utf8_lossy(&contents_out.stdout);

    for required_file in [
        "./usr/bin/kestic",
        "./usr/share/knotifications6/kestic.notifyrc",
        "./usr/share/applications/net.daester.Kestic.desktop",
        "./etc/xdg/autostart/net.daester.Kestic.daemon.desktop",
        "./usr/share/icons/hicolor/scalable/status/kestic-symbolic.svg",
    ] {
        assert!(
            contents.contains(required_file),
            ".deb missing packaged file `{required_file}` in:\n{contents}"
        );
    }

    // Verify /etc/xdg/autostart/net.daester.Kestic.daemon.desktop is marked as a dpkg conffile.
    let ctrl_dir = root.join(format!(
        "target/test-tmp-m0-s05-ctrl-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&ctrl_dir);
    let _guard = TempDirGuard(ctrl_dir.clone());
    let extract_out = Command::new("dpkg-deb")
        .arg("-e")
        .arg(&deb_path)
        .arg(&ctrl_dir)
        .output()
        .expect("run dpkg-deb -e");
    assert!(extract_out.status.success(), "dpkg-deb -e failed");

    let conffiles = fs::read_to_string(ctrl_dir.join("conffiles")).expect("read debian conffiles");
    assert!(
        conffiles
            .lines()
            .any(|l| l.trim() == "/etc/xdg/autostart/net.daester.Kestic.daemon.desktop"),
        "autostart desktop file must be listed in conffiles, got:\n{conffiles}"
    );
}
