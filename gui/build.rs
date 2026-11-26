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
        "src/qml/MainHeaderArea.qml",
        "src/qml/MainAboutPage.qml",
        "src/qml/AboutWindow.qml",
        "src/qml/PageFocusManager.qml",
        "src/qml/ShowCommandDialog.qml",
        "src/qml/UpdatePasswordDialog.qml",
        "src/qml/RenameBackupDialog.qml",
        "src/qml/RemoveBackupDialog.qml",
        "src/qml/RemoveDestinationDialog.qml",
        "src/qml/QuitConfirmDialog.qml",
        "src/qml/ConditionOverrideDialog.qml",
        "src/qml/UnsavedChangesDialog.qml",
        "src/qml/UnsavedTabSwitchDialog.qml",
        "src/qml/OverviewPage.qml",
        "src/qml/BackupsPage.qml",
        "src/qml/BackupDetailPage.qml",
        "src/qml/BackupSourceEditPage.qml",
        "src/qml/BackupExcludesEditPage.qml",
        "src/qml/BackupRetentionPage.qml",
        "src/qml/BackupPreviewPage.qml",
        "src/qml/BackupReassignPage.qml",
        "src/qml/BackupSnapshotsPage.qml",
        "src/qml/RestorePage.qml",
        "src/qml/RestoreSnapshotsPage.qml",
        "src/qml/RestoreBrowsePage.qml",
        "src/qml/RestoreSearchPage.qml",
        "src/qml/RestoreComparePage.qml",
        "src/qml/RestoreDialog.qml",
        "src/qml/BackupFullConfigPage.qml",
        "src/qml/BackupScheduleEditPage.qml",
        "src/qml/BackupConditionsEditPage.qml",
        "src/qml/DestinationsPage.qml",
        "src/qml/DestinationDetailPage.qml",
        "src/qml/DestinationEditPage.qml",
        "src/qml/DestinationMaintenancePage.qml",
        "src/qml/HistoryPage.qml",
        "src/qml/RunDetailPage.qml",
        "src/qml/DestinationDraft.qml",
        "src/qml/DestinationTypeChooser.qml",
        "src/qml/DestinationDetailsForm.qml",
        "src/qml/DestinationPasswordForm.qml",
        "src/qml/WizardPage.qml",
        "src/qml/EditorPage.qml",
        "src/qml/FocusRing.qml",
        "src/qml/KeyboardList.qml",
        "src/qml/RowIcon.qml",
        "src/qml/FieldError.qml",
        "src/qml/TitleSubtitleDelegate.qml",
        "src/qml/QueryStatus.qml",
        "src/qml/FilterField.qml",
        "src/qml/KindChips.qml",
        "src/qml/LiteralMessage.qml",
        "src/qml/NavRow.qml",
        "src/qml/TabNavTextArea.qml",
        "src/qml/WizardProgress.qml",
        "src/qml/AddDestinationWizard.qml",
        "src/qml/NewBackupWizard.qml",
        "src/qml/SettingsDialog.qml",
    ];
    for qml in qml_files {
        println!("cargo:rerun-if-changed={qml}");
    }
    for cpp in [
        "src/qt_bridge.h",
        "src/qt_bridge.cpp",
        "src/smoke_common.h",
        "src/smoke_helper.h",
        "src/smoke_helper.cpp",
        "src/smoke_m1.cpp",
        "src/smoke_m2.cpp",
        "src/smoke_m3.cpp",
        "src/smoke_m3_editors.cpp",
        "src/smoke_m3_pages.cpp",
        "src/smoke_m4.cpp",
        "src/smoke_main_menu.cpp",
        "src/smoke_m5.cpp",
        "src/smoke_m3_util.h",
    ] {
        println!("cargo:rerun-if-changed={cpp}");
    }

    remove_stale_qmltypes("net/daester/kestic");

    let mut builder =
        CxxQtBuilder::new_qml_module(QmlModule::new("net.daester.kestic").qml_files(qml_files))
            .file("src/backend.rs")
            .file("src/build_info.rs")
            .file("src/models.rs")
            .file("src/qt_bridge.rs")
            .cpp_file("src/qt_bridge.cpp")
            .qt_module("Widgets")
            .qt_module("Quick");
    if env::var_os("CARGO_FEATURE_TEST_HARNESS").is_some() {
        builder = builder
            .file("src/smoke.rs")
            .cpp_file("src/smoke_helper.cpp")
            .cpp_file("src/smoke_m1.cpp")
            .cpp_file("src/smoke_m2.cpp")
            .cpp_file("src/smoke_m3.cpp")
            .cpp_file("src/smoke_m3_editors.cpp")
            .cpp_file("src/smoke_m3_pages.cpp")
            .cpp_file("src/smoke_m4.cpp")
            .cpp_file("src/smoke_main_menu.cpp")
            .cpp_file("src/smoke_m5.cpp");
    }

    // SAFETY: Configures an additional C++ include path during build-script compilation.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.flag("-isystem/usr/include/KF6/KI18n");
        })
    };
    builder.build();
}

/// Deletes the module's `plugin.qmltypes` left in `OUT_DIR` by a previous build.
///
/// qt-build-utils 0.10 runs qmlcachegen *before* qmltyperegistrar writes the
/// current `plugin.qmltypes`, while the generated `qmldir` already points at it.
/// On a rebuild qmlcachegen therefore compiles QML against the C++ types of the
/// previous build; when the bridges changed in between (e.g. a CI cache from
/// another commit), the AOT-compiled bindings crash with SIGSEGV. Without the
/// file every build behaves like a clean one.
///
/// Upstream: KDAB/cxx-qt#1508, fixed by #1509 (not yet released as of 0.10.0).
/// Remove this workaround once the cxx-qt version in use contains that fix.
fn remove_stale_qmltypes(module_dirs: &str) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let qmltypes = out_dir
        .join("qt-build-utils/qml_modules")
        .join(module_dirs)
        .join("plugin.qmltypes");
    match fs::remove_file(&qmltypes) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("cannot remove stale {}: {e}", qmltypes.display()),
    }
}
