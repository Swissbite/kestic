// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Unit and integration tests for M0-S05: `.deb` packaging, QML Depends, notifyrc, and autostart;
//! plus the `Suggests` of M5-S04, of M5-S06 the `gpgv` dependency and the trust anchor that only
//! test builds can replace, of M6-S22 the `.deb` without an exact Qt version, and of M6-S23 the
//! AppStream metainfo (`m6_s23_appstream`) and of M6-S36 Qt Test only in the test harness
//! (`m6_s36_qt_test`).

mod common;
#[path = "m0_s05_deb_packaging/m6_s23_appstream.rs"]
mod m6_s23_appstream;
#[path = "m0_s05_deb_packaging/m6_s36_qt_test.rs"]
mod m6_s36_qt_test;

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

/// AC-M5-06-13 (a): the `.deb` depends on `gpgv`, which checks the signature of a downloaded
/// restic.
#[test]
fn m5_s06_deb_depends_on_gpgv() {
    let out = Command::new("dpkg-deb")
        .arg("-f")
        .arg(find_deb_package())
        .output()
        .expect("run dpkg-deb -f");
    assert!(out.status.success(), "dpkg-deb -f failed");
    let control = String::from_utf8_lossy(&out.stdout);

    // (a)
    assert!(
        control_field(&control, "Depends").contains(&"gpgv".to_string()),
        "Depends lacks gpgv:\n{control}"
    );
}

/// One alternative of a `Depends` entry: package name and the version relation, if any
/// (`("libqt6qml6", Some((">=", "6.10")))`).
type Relation = (String, Option<(String, String)>);

/// All alternatives of the `Depends` field, `a (>= 1) | b` split into its parts.
fn depends_relations(control: &str) -> Vec<Relation> {
    let Some(value) = control.lines().find_map(|l| l.strip_prefix("Depends: ")) else {
        return Vec::new();
    };
    value
        .split([',', '|'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|part| {
            let (name, rest) = part.split_once(' ').unwrap_or((part, ""));
            let version = rest
                .trim()
                .strip_prefix('(')
                .and_then(|r| r.strip_suffix(')'))
                .and_then(|r| r.trim().split_once(' '))
                .map(|(op, v)| (op.to_string(), v.trim().to_string()));
            (name.to_string(), version)
        })
        .collect()
}

/// `6.10.2` → `[6, 10, 2]`; a Debian revision or epoch is not expected in the Qt bounds.
fn version_parts(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|p| p.parse().unwrap_or_else(|_| panic!("version {version}")))
        .collect()
}

fn deb_control() -> String {
    let out = Command::new("dpkg-deb")
        .arg("-f")
        .arg(find_deb_package())
        .output()
        .expect("run dpkg-deb -f");
    assert!(out.status.success(), "dpkg-deb -f failed");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// AC-M6-22-1: the `.deb` installs next to any Qt from 6.10 on: (a) `Depends` names no
/// `qt6-*-private-abi` package (they pin one exact Qt patch version), (b) the shipped binary
/// imports no symbol of Qt's private API, and (c) the Qt libraries are bounded only from below,
/// Qt core and QML from exactly 6.10 on, the requirement of NFR-01.
#[test]
fn ac_m6_22_1_deb_needs_qt_6_10_without_private_abi() {
    let control = deb_control();
    let depends = depends_relations(&control);
    let (_extracted, shipped) = extract_shipped_binary("kestic-m6-s22-abi");

    // (a)
    let private_abi: Vec<&Relation> = depends
        .iter()
        .filter(|(name, _)| name.ends_with("-private-abi"))
        .collect();
    assert!(
        private_abi.is_empty(),
        "Depends pins a private Qt ABI {private_abi:?}:\n{control}"
    );

    // (b)
    let objdump = Command::new("objdump")
        .arg("-T")
        .arg(&shipped)
        .output()
        .expect("run objdump -T");
    assert!(objdump.status.success(), "objdump -T failed");
    let private_symbols: Vec<String> = String::from_utf8_lossy(&objdump.stdout)
        .lines()
        .filter(|l| l.contains("Qt_6_PRIVATE_API"))
        .map(str::to_string)
        .collect();
    assert!(
        private_symbols.is_empty(),
        "shipped kestic imports private Qt symbols:\n{}",
        private_symbols.join("\n")
    );

    // (c) no Qt package with an upper bound or an exact version
    let qt: Vec<&Relation> = depends
        .iter()
        .filter(|(name, _)| name.starts_with("libqt6") || name.starts_with("qt6-"))
        .collect();
    let bounded_above: Vec<&&Relation> = qt
        .iter()
        .filter(|(_, v)| v.as_ref().is_some_and(|(op, _)| op != ">=" && op != ">>"))
        .collect();
    assert!(
        bounded_above.is_empty(),
        "Qt packages with an upper bound or exact version {bounded_above:?}:\n{control}"
    );
    // (c) Qt core and QML from 6.10 on: the highest lower bound, manual or from shlibdeps
    for library in ["libqt6core6", "libqt6qml6"] {
        let floor = qt
            .iter()
            .filter(|(name, _)| name.starts_with(library))
            .filter_map(|(_, v)| v.as_ref())
            .filter(|(op, _)| op == ">=")
            .map(|(_, v)| version_parts(v))
            .max();
        assert_eq!(
            floor,
            Some(vec![6, 10]),
            "Depends must require {library}* from 6.10 on, no lower or higher:\n{control}"
        );
    }
}

/// Mangled names of the functions that QML compiled ahead of time to C++ calls in QtQml.
const AOT_SYMBOL: &str = "AOTCompiledContext";

/// AC-M6-22-2 (a): the QML of the GUI is compiled to bytecode only, without the C++ that
/// qmlcachegen generates by default: neither the shipped binary nor the test-harness build
/// imports a function of `QQmlPrivate::AOTCompiledContext`, which every compiled binding calls.
/// (b) is the GUI smoke of all milestones in `just verify`; (c) is manual.
#[test]
fn ac_m6_22_2_qml_compiled_to_bytecode_only() {
    let (_extracted, shipped) = extract_shipped_binary("kestic-m6-s22-aot");
    let harness = find_kestic_binary();

    // (a) shipped binary
    let shipped_aot: Vec<String> = undefined_dynamic_symbols(&shipped)
        .lines()
        .filter(|l| l.contains(AOT_SYMBOL))
        .map(str::to_string)
        .collect();
    assert!(
        shipped_aot.is_empty(),
        "shipped kestic carries AOT-compiled QML:\n{}",
        shipped_aot.join("\n")
    );
    // (a) test-harness build, the one the GUI smoke runs
    let harness_aot: Vec<String> = undefined_dynamic_symbols(&harness)
        .lines()
        .filter(|l| l.contains(AOT_SYMBOL))
        .map(str::to_string)
        .collect();
    assert!(
        harness_aot.is_empty(),
        "test-harness kestic carries AOT-compiled QML:\n{}",
        harness_aot.join("\n")
    );
}

/// The `[features]` and dependency tables of a workspace crate's `Cargo.toml`.
fn manifest(crate_dir: &str) -> toml::Table {
    let path = workspace_root().join(crate_dir).join("Cargo.toml");
    fs::read_to_string(&path)
        .expect("read Cargo.toml")
        .parse()
        .expect("parse Cargo.toml")
}

/// Features a dependency table enables on `kestic-core`.
fn core_features(manifest: &toml::Table, table: &str) -> Vec<String> {
    manifest
        .get(table)
        .and_then(|t| t.get("kestic-core"))
        .and_then(|dep| dep.get("features"))
        .and_then(toml::Value::as_array)
        .map_or_else(Vec::new, |features| {
            features
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_string)
                .collect()
        })
}

/// AC-M5-06-10: release URL, signing key and fingerprint can only be replaced with the feature
/// `test-trust-anchor` of `kestic-core`, which only the integration tests switch on and the
/// `.deb` build never does.
#[test]
fn ac_m5_06_10_trust_anchor_only_with_test_feature() {
    use kestic_core::restic_release::TrustAnchor;
    const FEATURE: &str = "test-trust-anchor";
    let names = [
        TrustAnchor::RELEASE_URL_VAR,
        TrustAnchor::SIGNING_KEY_VAR,
        TrustAnchor::SIGNING_FPR_VAR,
    ];
    assert_eq!(
        names,
        [
            "KESTIC_TEST_RELEASE_URL",
            "KESTIC_TEST_SIGNING_KEY",
            "KESTIC_TEST_SIGNING_FPR"
        ]
    );

    // (a) a build of kestic-core without the feature ignores the variables: with all three set,
    // its `for_this_build` is the production anchor; this test binary, built with the feature,
    // takes the release URL from them
    let probe = FeaturelessProbe::new();
    let mock_url = "http://127.0.0.1:9/";
    assert_eq!(
        probe.run_anchor_bin(&[
            (TrustAnchor::RELEASE_URL_VAR, mock_url),
            (TrustAnchor::SIGNING_KEY_VAR, &probe.manifest_path()),
            (
                TrustAnchor::SIGNING_FPR_VAR,
                "0000000000000000000000000000000000000000"
            ),
        ]),
        "production",
        "a build without {FEATURE} took the anchor from the variables"
    );
    let with_feature = TrustAnchor::from_lookup(|name| {
        (name == TrustAnchor::RELEASE_URL_VAR).then(|| mock_url.to_string())
    })
    .expect("anchor from the variables");
    assert_ne!(
        with_feature,
        TrustAnchor::production(),
        "the feature build ignores the variables, so the probe proves nothing"
    );
    // the shipped binary does not carry the variable names either
    let (_extracted, shipped) = extract_shipped_binary("kestic-m5-s06-anchor");
    let test_binary = std::env::current_exe().expect("test binary");
    for name in names {
        assert!(
            !file_contains(&shipped, name.as_bytes()),
            "shipped kestic contains {name}"
        );
        assert!(
            file_contains(&test_binary, name.as_bytes()),
            "the feature build lacks {name}, so the probe proves nothing"
        );
    }

    // (b) without the feature the anchor has no constructor that takes a URL, key or fingerprint
    let refused = probe.build_lookup_bin();
    assert!(
        refused.contains("error[E0599]"),
        "the probe without {FEATURE} failed for another reason than a missing item:\n{refused}"
    );
    assert!(
        refused.contains("from_lookup"),
        "the probe without {FEATURE} failed on something else than from_lookup:\n{refused}"
    );
    // the feature is declared by kestic-core, off by default, and only kestic-core's own
    // dev-dependency (the integration tests) switches it on
    let core = manifest("core");
    let features = core.get("features").and_then(toml::Value::as_table);
    assert!(
        features.is_some_and(|f| f.contains_key(FEATURE)),
        "kestic-core declares no feature {FEATURE}"
    );
    let default = features
        .and_then(|f| f.get("default"))
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !default.iter().any(|f| f.as_str() == Some(FEATURE)),
        "{FEATURE} is a default feature"
    );
    assert_eq!(
        core_features(&core, "dev-dependencies"),
        vec![FEATURE.to_string()]
    );
    for crate_dir in [
        "build-stamp",
        "core",
        "i18n",
        "daemon",
        "gui",
        "test-support",
    ] {
        let manifest = manifest(crate_dir);
        for table in ["dependencies", "build-dependencies"] {
            assert!(
                !core_features(&manifest, table).contains(&FEATURE.to_string()),
                "{crate_dir} enables {FEATURE} in [{table}]"
            );
        }
        let enabled_by_features = manifest
            .get("features")
            .and_then(toml::Value::as_table)
            .is_some_and(|f| {
                f.values()
                    .filter_map(toml::Value::as_array)
                    .flatten()
                    .any(|v| v.as_str() == Some(&format!("kestic-core/{FEATURE}")))
            });
        assert!(
            !enabled_by_features,
            "a feature of {crate_dir} switches on kestic-core/{FEATURE}"
        );
    }

    // (c) `just deb` builds the package without any feature
    let justfile = fs::read_to_string(workspace_root().join("justfile")).expect("read justfile");
    let deb_builds: Vec<&str> = justfile
        .lines()
        .filter(|l| l.contains("cargo deb") && !l.trim_start().starts_with('#'))
        .collect();
    assert_eq!(deb_builds.len(), 1, "{deb_builds:?}");
    for flag in ["--features", "-F", "--all-features"] {
        assert!(
            !deb_builds[0]
                .split_whitespace()
                .any(|w| w.starts_with(flag)),
            "`cargo deb` in the justfile passes {flag}: {}",
            deb_builds[0]
        );
    }
}

/// A crate outside the workspace that depends on `kestic-core` without any feature, as the `.deb`
/// build does. Its target folder lies below the workspace's, so later runs reuse the build.
struct FeaturelessProbe {
    dir: tempfile::TempDir,
}

impl FeaturelessProbe {
    const ANCHOR_BIN: &str = r#"
use kestic_core::restic_release::TrustAnchor;
fn main() {
    let anchor = TrustAnchor::for_this_build().expect("anchor of this build");
    print!("{}", if anchor == TrustAnchor::production() { "production" } else { "replaced" });
}
"#;
    const LOOKUP_BIN: &str = r#"
use kestic_core::restic_release::TrustAnchor;
fn main() {
    let anchor = TrustAnchor::from_lookup(|_| None);
    print!("{anchor:?}");
}
"#;

    fn new() -> Self {
        let dir = tempfile::tempdir().expect("probe dir");
        let core = workspace_root().join("core");
        let manifest = format!(
            "[package]\nname = \"featureless-core-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             publish = false\n\n[dependencies]\nkestic-core = {{ path = {core:?} }}\n\n[workspace]\n",
            core = core.to_string_lossy()
        );
        fs::write(dir.path().join("Cargo.toml"), manifest).expect("write probe manifest");
        fs::copy(
            workspace_root().join("Cargo.lock"),
            dir.path().join("Cargo.lock"),
        )
        .expect("copy Cargo.lock");
        let bin = dir.path().join("src/bin");
        fs::create_dir_all(&bin).expect("probe src/bin");
        fs::write(bin.join("anchor.rs"), Self::ANCHOR_BIN).expect("write anchor probe");
        fs::write(bin.join("lookup.rs"), Self::LOOKUP_BIN).expect("write lookup probe");
        Self { dir }
    }

    fn manifest_path(&self) -> String {
        self.dir
            .path()
            .join("Cargo.toml")
            .to_string_lossy()
            .into_owned()
    }

    /// `cargo build` of one probe binary, run from the workspace so that its toolchain applies.
    fn cargo_build(&self, bin: &str) -> std::process::Output {
        Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .current_dir(workspace_root())
            .env("CARGO_HOME", tool_home("CARGO_HOME", ".cargo"))
            .args(["build", "--quiet", "--bin", bin, "--manifest-path"])
            .arg(self.manifest_path())
            .arg("--target-dir")
            .arg(find_target_dir().join("featureless-core-probe"))
            .output()
            .expect("run cargo build for the probe")
    }

    /// Output of the anchor probe, run with `vars` set.
    fn run_anchor_bin(&self, vars: &[(&str, &str)]) -> String {
        let built = self.cargo_build("anchor");
        assert!(
            built.status.success(),
            "probe build failed:\n{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let exe = find_target_dir().join("featureless-core-probe/debug/anchor");
        let out = Command::new(exe)
            .env_clear()
            .envs(vars.iter().copied())
            .output()
            .expect("run anchor probe");
        assert!(out.status.success(), "anchor probe failed: {out:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Compiler output of the probe that calls `TrustAnchor::from_lookup`; the build must fail.
    fn build_lookup_bin(&self) -> String {
        let built = self.cargo_build("lookup");
        assert!(
            !built.status.success(),
            "a build without the feature compiled TrustAnchor::from_lookup"
        );
        String::from_utf8_lossy(&built.stderr).into_owned()
    }
}
