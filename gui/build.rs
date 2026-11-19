// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

use cxx_qt_build::{CxxQtBuilder, QmlModule};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn compile_mo(po_file: &Path, locale_root: &Path) {
    let out_dir = locale_root.join("de/LC_MESSAGES");
    fs::create_dir_all(&out_dir).expect("create locale output directory");
    let mo_file = out_dir.join("kestic.mo");
    let status = Command::new("msgfmt")
        .arg("--check")
        .arg("-o")
        .arg(&mo_file)
        .arg(po_file)
        .status()
        .expect("run msgfmt to compile po/de/kestic.po");
    assert!(status.success(), "msgfmt failed on {}", po_file.display());
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("gui crate has workspace root parent");
    let po_file = workspace_root.join("po/de/kestic.po");
    println!("cargo:rerun-if-changed={}", po_file.display());

    let default_target_locale = workspace_root.join("target/locale");
    compile_mo(&po_file, &default_target_locale);

    // cargo-deb only maps asset paths starting with `target/release/` to the
    // profile directory it actually builds; place the catalog there as well.
    // OUT_DIR = <target>/<profile>/build/…/out (depth below `build` varies by
    // Cargo version), so the profile directory is the parent of `build`.
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "build"))
        .and_then(Path::parent)
        .expect("OUT_DIR lies below <target>/<profile>/build");
    compile_mo(&po_file, &profile_dir.join("locale"));

    let custom_locale = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .filter(|d| *d != workspace_root.join("target"))
        .map(|d| d.join("locale"));
    if let Some(ref extra_locale) = custom_locale {
        compile_mo(&po_file, extra_locale);
    }

    let dev_locale = custom_locale.unwrap_or(default_target_locale);
    println!(
        "cargo:rustc-env=KESTIC_DEV_LOCALE_DIR={}",
        dev_locale.display()
    );

    let qml_files = [
        "src/qml/Main.qml",
        "src/qml/SpikeMain.qml",
        "src/qml/OverviewPage.qml",
        "src/qml/BackupsPage.qml",
        "src/qml/BackupDetailPage.qml",
        "src/qml/BackupSourceEditPage.qml",
        "src/qml/BackupExcludesEditPage.qml",
        "src/qml/BackupRetentionPage.qml",
        "src/qml/BackupPreviewPage.qml",
        "src/qml/BackupReassignPage.qml",
        "src/qml/BackupSnapshotsPage.qml",
        "src/qml/BackupFullConfigPage.qml",
        "src/qml/BackupScheduleEditPage.qml",
        "src/qml/BackupConditionsEditPage.qml",
        "src/qml/DestinationsPage.qml",
        "src/qml/DestinationDetailPage.qml",
        "src/qml/DestinationEditPage.qml",
        "src/qml/DestinationMaintenancePage.qml",
        "src/qml/HistoryPage.qml",
        "src/qml/RunDetailPage.qml",
        "src/qml/AddDestinationWizard.qml",
        "src/qml/NewBackupWizard.qml",
        "src/qml/SettingsDialog.qml",
    ];
    for qml in qml_files {
        println!("cargo:rerun-if-changed={qml}");
    }
    for cpp in [
        "src/smoke_common.h",
        "src/smoke_helper.h",
        "src/smoke_helper.cpp",
        "src/smoke_m1.cpp",
        "src/smoke_m2.cpp",
        "src/smoke_m3.cpp",
    ] {
        println!("cargo:rerun-if-changed={cpp}");
    }

    let builder =
        CxxQtBuilder::new_qml_module(QmlModule::new("net.daester.kestic").qml_files(qml_files))
            .file("src/backend.rs")
            .file("src/models.rs")
            .file("src/smoke.rs")
            .cpp_file("src/smoke_helper.cpp")
            .cpp_file("src/smoke_m1.cpp")
            .cpp_file("src/smoke_m2.cpp")
            .cpp_file("src/smoke_m3.cpp")
            .qt_module("Widgets")
            .qt_module("Quick");

    // SAFETY: Configures an additional C++ include path during build-script compilation.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.flag("-isystem/usr/include/KF6/KI18n");
        })
    };
    builder.build();
}
