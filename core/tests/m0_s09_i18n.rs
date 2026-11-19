// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Integration tests for M0-S09 (AC-M0-09-1: KI18n EN/DE in Rust & QML)
//! and NFR-06 (GNU gettext string-extraction & untranslated-string lint).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has workspace root parent")
}

fn find_target_dir(workspace_root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"))
}

fn find_kestic_binary(workspace_root: &Path) -> PathBuf {
    let target_dir = find_target_dir(workspace_root);
    for profile in ["debug", "release"] {
        let candidate = target_dir.join(profile).join("kestic");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("kestic binary not found under {}", target_dir.display());
}

fn run_i18n_smoke(root: &Path, lang: &str, screenshot_rel: &str) {
    let kestic = find_kestic_binary(root);
    let target_dir = find_target_dir(root);
    let built_mo = target_dir.join("locale/de/LC_MESSAGES/kestic.mo");
    assert!(
        built_mo.is_file() && fs::metadata(&built_mo).expect("stat kestic.mo").len() > 0,
        "compiled German catalog missing or empty at {}",
        built_mo.display()
    );

    let temp = root.join(format!(
        "target/test-tmp-m0-s09-{lang}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    for sub in [
        "home",
        "cfg",
        "data/locale/de/LC_MESSAGES",
        "cache",
        "state",
        "run",
    ] {
        fs::create_dir_all(temp.join(sub)).expect("create isolated XDG dir");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temp.join("run"), fs::Permissions::from_mode(0o700))
            .expect("chmod 0700 XDG_RUNTIME_DIR");
    }
    fs::copy(&built_mo, temp.join("data/locale/de/LC_MESSAGES/kestic.mo"))
        .expect("copy kestic.mo into isolated XDG_DATA_HOME");
    let _guard = TempDirGuard(temp.clone());

    let screenshot_path = root.join(screenshot_rel);
    if screenshot_path.exists() {
        fs::remove_file(&screenshot_path).expect("remove stale i18n screenshot");
    }

    let output = Command::new(kestic)
        .args([
            "--smoke-test",
            "--smoke-layout",
            "wide",
            "--smoke-lang",
            lang,
            "--screenshot",
        ])
        .arg(&screenshot_path)
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", "en_US.UTF-8")
        .env("LANGUAGE", lang)
        .env("HOME", temp.join("home"))
        .env("XDG_CONFIG_HOME", temp.join("cfg"))
        .env("XDG_DATA_HOME", temp.join("data"))
        .env("XDG_CACHE_HOME", temp.join("cache"))
        .env("XDG_STATE_HOME", temp.join("state"))
        .env("XDG_RUNTIME_DIR", temp.join("run"))
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .expect("run kestic i18n smoke check");

    assert!(
        output.status.success(),
        "i18n smoke `{lang}` failed (status={}):\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "i18n smoke `{lang}` emitted QML/Qt warnings:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let png = fs::read(&screenshot_path).expect("read i18n smoke PNG");
    assert!(
        png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 1024,
        "i18n screenshot {} must be valid PNG",
        screenshot_path.display()
    );
}

/// AC-M0-09-1 (E12, NFR-06): Verifies KI18n translates both Rust (`SpikeListModel`) and QML
/// (`NavigationTabBar` / `Main.qml`) strings under `LANGUAGE=en` and `LANGUAGE=de` offscreen.
#[test]
fn ac_m0_09_1_ki18n_translates_rust_and_qml_strings_en_and_de() {
    let root = workspace_root();
    run_i18n_smoke(root, "en", "target/smoke-artifacts/m0-s09-en.png");
    run_i18n_smoke(root, "de", "target/smoke-artifacts/m0-s09-de.png");
}

/// NFR-06 (M0-S09): Verifies the selected string-lint toolchain (`xgettext` + `msgcmp` +
/// `msgattrib` + `msgfmt --check`) extracts all `i18n(...)` strings from Rust and QML and
/// enforces 100% translation coverage in `po/de/kestic.po`.
#[test]
fn nfr_06_gettext_xgettext_and_po_lint() {
    let root = workspace_root();
    let po_file = root.join("po/de/kestic.po");
    let rust_src = root.join("gui/src/list_model.rs");
    let qml_src = root.join("gui/src/qml/Main.qml");

    let temp = root.join(format!(
        "target/test-tmp-m0-s09-lint-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).expect("create temp dir for pot extraction");
    let _guard = TempDirGuard(temp.clone());

    let pot_file = temp.join("kestic.pot");

    let extract_rust = Command::new("xgettext")
        .args([
            "--from-code=UTF-8",
            "--language=C",
            "--keyword=i18n:1",
            "-o",
        ])
        .arg(&pot_file)
        .arg(&rust_src)
        .output()
        .expect("run xgettext on Rust sources");
    assert!(
        extract_rust.status.success(),
        "xgettext (Rust) failed:\n{}",
        String::from_utf8_lossy(&extract_rust.stderr)
    );

    let extract_qml = Command::new("xgettext")
        .args([
            "--from-code=UTF-8",
            "--language=JavaScript",
            "--keyword=i18n:1",
            "-j",
            "-o",
        ])
        .arg(&pot_file)
        .arg(&qml_src)
        .output()
        .expect("run xgettext on QML sources");
    assert!(
        extract_qml.status.success(),
        "xgettext (QML) failed:\n{}",
        String::from_utf8_lossy(&extract_qml.stderr)
    );

    let cmp_out = Command::new("msgcmp")
        .arg(&po_file)
        .arg(&pot_file)
        .output()
        .expect("run msgcmp");
    assert!(
        cmp_out.status.success(),
        "msgcmp found missing or untranslated msgids in {}:\n{}",
        po_file.display(),
        String::from_utf8_lossy(&cmp_out.stderr)
    );

    let untranslated = Command::new("msgattrib")
        .arg("--untranslated")
        .arg(&po_file)
        .output()
        .expect("run msgattrib --untranslated");
    assert!(untranslated.status.success(), "msgattrib failed");
    assert!(
        String::from_utf8_lossy(&untranslated.stdout)
            .trim()
            .is_empty(),
        "po/de/kestic.po still has untranslated entries:\n{}",
        String::from_utf8_lossy(&untranslated.stdout)
    );

    let fuzzy = Command::new("msgattrib")
        .arg("--only-fuzzy")
        .arg(&po_file)
        .output()
        .expect("run msgattrib --only-fuzzy");
    assert!(fuzzy.status.success(), "msgattrib --only-fuzzy failed");
    assert!(
        String::from_utf8_lossy(&fuzzy.stdout).trim().is_empty(),
        "po/de/kestic.po still has fuzzy entries:\n{}",
        String::from_utf8_lossy(&fuzzy.stdout)
    );

    let msgfmt_out = Command::new("msgfmt")
        .args(["--check", "-o", "/dev/null"])
        .arg(&po_file)
        .output()
        .expect("run msgfmt --check");
    assert!(
        msgfmt_out.status.success(),
        "msgfmt --check failed:\n{}",
        String::from_utf8_lossy(&msgfmt_out.stderr)
    );
}
