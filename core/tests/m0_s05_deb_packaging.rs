// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Unit and integration tests for M0-S05: `.deb` packaging, QML Depends, notifyrc, and autostart;
//! plus the `Suggests` of M5-S04.

mod common;

use kestic_test_support::tools::short_tempdir;
use kestic_test_support::workspace::{
    file_contains, find_kestic_binary, find_target_dir, workspace_root,
};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn find_deb_package() -> PathBuf {
    let deb_dir = find_target_dir().join("debian");
    let entries = fs::read_dir(&deb_dir).unwrap_or_else(|e| {
        panic!(
            "target/debian not readable at {} ({}); run `just deb` or `just verify` first",
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
            "no kestic_*.deb found in {}; run `just deb` or `just verify` first",
            deb_dir.display()
        )
    })
}

/// AC-M0-05-2: Unit check for autostart desktop entry contents (`TryExec=/usr/bin/kestic`,
/// `Exec=kestic daemon --autostart`), desktop-file validation, and immediate exit without config.
#[test]
fn ac_m0_05_2_autostart_entry_contains_tryexec_and_exec() {
    let root = workspace_root();

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
    let temp = short_tempdir("kestic-m0-s05-bin");
    fs::create_dir_all(temp.path().join("cfg")).expect("create cfg dir");
    fs::create_dir_all(temp.path().join("home")).expect("create home dir");

    let out = Command::new(find_kestic_binary())
        .args(["daemon", "--autostart"])
        .env("HOME", temp.path().join("home"))
        .env("XDG_CONFIG_HOME", temp.path().join("cfg"))
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
    let deb_path = find_deb_package();

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
        "./usr/share/icons/hicolor/scalable/apps/kestic.svg",
        "./usr/share/icons/hicolor/scalable/status/kestic-symbolic.svg",
        "./usr/share/locale/de/LC_MESSAGES/kestic.mo",
    ] {
        assert!(
            contents.contains(required_file),
            ".deb missing packaged file `{required_file}` in:\n{contents}"
        );
    }

    // Verify /etc/xdg/autostart/net.daester.Kestic.daemon.desktop is marked as a dpkg conffile.
    let ctrl = short_tempdir("kestic-m0-s05-ctrl");
    let ctrl_dir = ctrl.path();
    let extract_out = Command::new("dpkg-deb")
        .arg("-e")
        .arg(&deb_path)
        .arg(ctrl_dir)
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

/// Names of harness pieces that must never ship: CLI flags and environment variables of the smoke
/// scenes and `--simulate-status` (strings), and Qt's synthetic input functions (imported symbols).
const HARNESS_STRINGS: [&str; 3] = ["--smoke-m1", "--simulate-status", "KESTIC_SMOKE_"];
const HARNESS_SYMBOLS: [&str; 2] = ["qt_handleKeyEvent", "qt_handleMouseEvent"];

/// Extracts `/usr/bin/kestic` from the `.deb` into a fresh temp directory.
fn extract_shipped_binary(tag: &str) -> (tempfile::TempDir, PathBuf) {
    let deb_path = find_deb_package();
    let extracted = short_tempdir(tag);
    let status = Command::new("dpkg-deb")
        .arg("-x")
        .arg(&deb_path)
        .arg(extracted.path().join("root"))
        .status()
        .expect("run dpkg-deb -x");
    assert!(status.success(), "dpkg-deb -x failed");
    let shipped = extracted.path().join("root/usr/bin/kestic");
    (extracted, shipped)
}

fn undefined_dynamic_symbols(path: &std::path::Path) -> String {
    let out = Command::new("nm")
        .args(["-D", "--undefined-only"])
        .arg(path)
        .output()
        .expect("run nm");
    assert!(out.status.success(), "nm failed on {}", path.display());
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The `kestic` binary in the `.deb` is the product without the test harness (architecture.md
/// §Code-Struktur: test harness only with the `test-harness` feature, never in the `.deb`), and it
/// runs. The same probes find the harness in the test build, so they are proven to work.
#[test]
fn shipped_binary_runs_and_carries_no_test_harness() {
    let (extracted, shipped) = extract_shipped_binary("kestic-m0-s05-shipped");
    let harness = find_kestic_binary();

    // (a) the shipped binary runs: `kestic daemon --autostart` without config exits 0 (NFR-11)
    fs::create_dir_all(extracted.path().join("cfg")).expect("create cfg dir");
    fs::create_dir_all(extracted.path().join("home")).expect("create home dir");
    let out = Command::new(&shipped)
        .args(["daemon", "--autostart"])
        .env("HOME", extracted.path().join("home"))
        .env("XDG_CONFIG_HOME", extracted.path().join("cfg"))
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .expect("run shipped kestic");
    assert!(
        out.status.success(),
        "shipped kestic daemon --autostart failed"
    );

    // (b) no harness strings in the shipped binary, all of them in the harness build
    for text in HARNESS_STRINGS {
        assert!(
            !file_contains(&shipped, text.as_bytes()),
            "shipped kestic contains {text}"
        );
        assert!(
            file_contains(&harness, text.as_bytes()),
            "harness build lacks {text}"
        );
    }

    // (c) no synthetic-input imports in the shipped binary, both in the harness build
    let shipped_syms = undefined_dynamic_symbols(&shipped);
    let harness_syms = undefined_dynamic_symbols(&harness);
    for sym in HARNESS_SYMBOLS {
        assert!(!shipped_syms.contains(sym), "shipped kestic imports {sym}");
        assert!(harness_syms.contains(sym), "harness build lacks {sym}");
    }
}

/// Directory of a tool installation: the variable if set, else the default below `$HOME`. The
/// `deb` recipe in the justfile resolves the same directories for its path remapping.
fn tool_home(var: &str, default_below_home: &str) -> PathBuf {
    std::env::var_os(var).filter(|v| !v.is_empty()).map_or_else(
        || PathBuf::from(std::env::var_os("HOME").expect("HOME set")).join(default_below_home),
        PathBuf::from,
    )
}

/// NFR-01: the `kestic` in the `.deb` carries no path of the build machine: neither (a) the workspace
/// path, (b) the `CARGO_MANIFEST_DIR` of the GUI crate, (c) the Cargo home with the registry
/// sources nor (d) the Rust toolchain directory. Such paths would publish the build machine's
/// layout, and a target system that has them would load icons or translations from there. The
/// debug harness build keeps its source-tree fallbacks and unmapped panic locations, so the same
/// probes find (a), (b) and (c) there. (d) has no such counter-check: the standard library names
/// its sources below the toolchain directory only where the `rust-src` component is installed,
/// elsewhere (as in CI) it uses `/rustc/<commit>/`.
#[test]
fn shipped_binary_carries_no_build_paths() {
    let (_extracted, shipped) = extract_shipped_binary("kestic-m0-s05-paths");
    let harness = find_kestic_binary();
    let workspace = workspace_root();
    let gui_manifest_dir = workspace.join("gui");
    let cargo_home = tool_home("CARGO_HOME", ".cargo");
    let rustup_home = tool_home("RUSTUP_HOME", ".rustup");
    let text = |p: &std::path::Path| p.to_string_lossy().into_owned();

    // (a) workspace path
    assert!(
        !file_contains(&shipped, text(&workspace).as_bytes()),
        "shipped kestic contains the workspace path {}",
        workspace.display()
    );
    assert!(
        file_contains(&harness, text(&workspace).as_bytes()),
        "harness build lacks the workspace path, so the probe proves nothing"
    );

    // (b) CARGO_MANIFEST_DIR of the GUI crate
    assert!(
        !file_contains(&shipped, text(&gui_manifest_dir).as_bytes()),
        "shipped kestic contains CARGO_MANIFEST_DIR {}",
        gui_manifest_dir.display()
    );
    assert!(
        file_contains(&harness, text(&gui_manifest_dir).as_bytes()),
        "harness build lacks CARGO_MANIFEST_DIR, so the probe proves nothing"
    );

    // (c) Cargo home (panic locations of registry crates)
    assert!(
        !file_contains(&shipped, text(&cargo_home).as_bytes()),
        "shipped kestic contains the Cargo home {}",
        cargo_home.display()
    );
    assert!(
        file_contains(&harness, text(&cargo_home).as_bytes()),
        "harness build lacks the Cargo home, so the probe proves nothing"
    );

    // (d) Rust toolchain directory (panic locations of the standard library)
    assert!(
        !file_contains(&shipped, text(&rustup_home).as_bytes()),
        "shipped kestic contains the toolchain directory {}",
        rustup_home.display()
    );
}

/// `Depends`, `Recommends` and `Suggests` of the `.deb` as lists of package names, alternatives
/// (`a | b`) included.
fn control_field(control: &str, field: &str) -> Vec<String> {
    let prefix = format!("{field}: ");
    let Some(value) = control.lines().find_map(|l| l.strip_prefix(&prefix)) else {
        return Vec::new();
    };
    value
        .split([',', '|'])
        .filter_map(|p| p.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// M5-S04: the `.deb` suggests `rclone` and `openssh-client` and neither depends on nor
/// recommends them; a missing program stays «needs installation» at run time.
#[test]
fn m5_s04_deb_suggests_rclone_and_openssh_client_only() {
    let out = Command::new("dpkg-deb")
        .arg("-f")
        .arg(find_deb_package())
        .output()
        .expect("run dpkg-deb -f");
    assert!(out.status.success(), "dpkg-deb -f failed");
    let control = String::from_utf8_lossy(&out.stdout);

    let suggests = control_field(&control, "Suggests");
    assert!(
        suggests.contains(&"rclone".to_string()),
        "Suggests lacks rclone:\n{control}"
    );
    assert!(
        suggests.contains(&"openssh-client".to_string()),
        "Suggests lacks openssh-client:\n{control}"
    );
    for field in ["Depends", "Recommends"] {
        let packages = control_field(&control, field);
        assert!(
            !packages.contains(&"rclone".to_string()),
            "{field} must not name rclone:\n{control}"
        );
        assert!(
            !packages.contains(&"openssh-client".to_string()),
            "{field} must not name openssh-client:\n{control}"
        );
    }
}
