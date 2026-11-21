// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Integration tests for M1-S16 (AC-M1-16-1, AC-M1-16-2: full EN/DE localization in GUI & daemon)
//! and NFR-06 (GNU gettext string extraction & Swiss-German catalog lint across all M1 sources).

mod common;

use common::gui::{
    DaemonProcessGuard, IsolatedEnv, PrivateDbusGuard, SmokeContext, find_kestic_binary,
    workspace_root,
};
use common::tools::{built_german_catalog, short_tempdir};
use common::workspace::files_with_ext;
use kestic_core::config::{AppConfig, save_config};
use kestic_core::model::ResticProvider;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// NFR-06 (M1-S16): Extracts every `i18n(...)` string from all `daemon/src/*.rs`,
/// `gui/src/*.rs`, and `gui/src/qml/*.qml` files, plus every `i18n_noop(...)` msgid of the
/// shared texts in `i18n/src/*.rs`, and enforces 100% Swiss-German coverage
/// (0 untranslated, 0 fuzzy, 0 missing, 0 `ß`) in `po/de/kestic.po`.
#[test]
fn nfr_06_m1_all_rust_and_qml_i18n_strings_extracted_and_translated() {
    let root = workspace_root();
    let po_file = root.join("po/de/kestic.po");

    let mut rust_files = files_with_ext(&root.join("daemon/src"), "rs");
    rust_files.extend(files_with_ext(&root.join("gui/src"), "rs"));
    // The shared texts of `kestic-i18n` are marked with `i18n_noop`.
    rust_files.extend(files_with_ext(&root.join("i18n/src"), "rs"));
    let qml_files = files_with_ext(&root.join("gui/src/qml"), "qml");
    assert!(!rust_files.is_empty(), "expected Rust files");
    assert!(!qml_files.is_empty(), "expected QML files");

    let temp = short_tempdir("kestic-m1-s16-lint");
    let pot_file = temp.path().join("kestic.pot");

    let rust_out = Command::new("xgettext")
        .args([
            "--from-code=UTF-8",
            "--language=C",
            "--keyword=i18n:1",
            "--keyword=i18n_noop:1",
            "-o",
        ])
        .arg(&pot_file)
        .args(&rust_files)
        .output()
        .expect("run xgettext on M1 Rust sources");
    assert!(
        rust_out.status.success(),
        "xgettext (Rust) failed:\n{}",
        String::from_utf8_lossy(&rust_out.stderr)
    );

    let qml_out = Command::new("xgettext")
        .args([
            "--from-code=UTF-8",
            "--language=JavaScript",
            "--keyword=i18n:1",
            "-j",
            "-o",
        ])
        .arg(&pot_file)
        .args(&qml_files)
        .output()
        .expect("run xgettext on M1 QML sources");
    assert!(
        qml_out.status.success(),
        "xgettext (QML) failed:\n{}",
        String::from_utf8_lossy(&qml_out.stderr)
    );

    let cmp_out = Command::new("msgcmp")
        .arg(&po_file)
        .arg(&pot_file)
        .output()
        .expect("run msgcmp on po/de/kestic.po");
    assert!(
        cmp_out.status.success(),
        "msgcmp found missing or untranslated M1 msgids in {}:\n{}",
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

    let po_text = fs::read_to_string(&po_file).expect("read po/de/kestic.po");
    assert!(
        !po_text.contains('ß'),
        "po/de/kestic.po must follow Swiss German orthography (no `ß`, use `ss`)"
    );
}

fn run_lang_smoke(lang: &str, screenshot_rel: &str) {
    let built_mo = built_german_catalog();
    assert!(
        built_mo.is_file(),
        "compiled German catalog missing at {}",
        built_mo.display()
    );
    assert!(
        fs::metadata(&built_mo).expect("stat kestic.mo").len() > 0,
        "compiled German catalog at {} is empty",
        built_mo.display()
    );

    let iso = IsolatedEnv::create(&format!("s16-{lang}"), None);
    iso.seed_probe_secret();
    let cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: PathBuf::from("/nonexistent/kestic-missing-restic"),
        },
        ..AppConfig::default()
    };
    save_config(&iso.config_path(), &cfg).expect("save config");

    let kestic = find_kestic_binary();
    let dbus = PrivateDbusGuard::start();
    let _daemon = DaemonProcessGuard::spawn(&kestic, &iso, &dbus.address, true, lang);
    SmokeContext {
        kestic_bin: &kestic,
        iso: &iso,
        dbus_addr: &dbus.address,
        use_secret_file: true,
    }
    .run(
        "blocking-restic_unavailable",
        "wide",
        lang,
        screenshot_rel,
        &[],
    );
}

/// AC-M1-16-1 & AC-M1-16-2: Verifies both GUI and daemon KI18n translation in EN and DE.
#[test]
fn ac_m1_16_1_and_16_2_ki18n_en_and_de_in_gui_and_daemon() {
    run_lang_smoke("en", "target/smoke-artifacts/m1-s16-en.png");
    run_lang_smoke("de", "target/smoke-artifacts/m1-s16-de.png");
}
