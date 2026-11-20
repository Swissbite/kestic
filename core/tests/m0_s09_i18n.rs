// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! NFR-06 (M0-S09): GNU gettext string extraction and untranslated-string lint. AC-M0-09-1
//! (Rust and QML texts in EN and DE) is checked in the product window, see
//! `docs/spec/milestones/m0-spike.md`.

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

/// NFR-06 (M0-S09): Verifies the selected string-lint toolchain (`xgettext` + `msgcmp` +
/// `msgattrib` + `msgfmt --check`) extracts all `i18n(...)` strings from Rust and QML and
/// enforces 100% translation coverage in `po/de/kestic.po`.
#[test]
fn nfr_06_gettext_xgettext_and_po_lint() {
    let root = workspace_root();
    let po_file = root.join("po/de/kestic.po");
    let rust_src = root.join("gui/src/ui_texts.rs");
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
            "--keyword=i18n_noop:1",
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
