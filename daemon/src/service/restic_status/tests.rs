// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::super::state::probe_restic_and_secrets_with;
use super::*;
use crate::dbus_api::blocking_message;
use kestic_core::secrets::InMemorySecretStore;
use kestic_i18n::codes::BlockingKind;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn write_script(path: &Path, script: &str) -> PathBuf {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, script).expect("write");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
    path.to_path_buf()
}

/// Like the real restic 0.16.4: no `version --json`, a text line instead.
fn stand_in_0_16_4(path: &Path) -> PathBuf {
    write_script(
        path,
        "#!/bin/sh\n\
         if [ \"${2:-}\" = --json ]; then echo 'unknown flag: --json' >&2; exit 1; fi\n\
         echo 'restic 0.16.4 compiled with go1.22.2 on linux/amd64'\n",
    )
}

fn json_stand_in(path: &Path, version: &str) -> PathBuf {
    write_script(
        path,
        &format!("#!/bin/sh\necho '{{\"message_type\":\"version\",\"version\":\"{version}\"}}'\n"),
    )
}

/// AC-M5-07-6 (daemon): «System» chosen explicitly with a system restic 0.16.4 makes restic
/// unusable: runs cannot start, the status names the old version, and the window shows the
/// restic actions.
#[test]
fn ac_m5_07_6_explicit_system_too_old_blocks_runs() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let system = stand_in_0_16_4(&dir.path().join("bin/restic"));
    let app = json_stand_in(&dir.path().join("data/kestic/restic"), "0.19.1");
    let locations = ResticLocations {
        system: Some(system.clone()),
        app_managed: Some(app),
    };

    let ((status, _, blocking), resolved) = probe_restic_and_secrets_with(
        &ResticProvider::System,
        &locations,
        &InMemorySecretStore::new(),
    );
    // (a) no binary, so no run starts
    assert!(resolved.is_err());
    assert!(!status.valid);
    // (b) too old, with the version found and the text naming it
    assert_eq!(status.error_kind, Some(ResticStatusCode::TooOld));
    assert_eq!(status.found_version.as_deref(), Some("0.16.4"));
    assert_eq!(
        status.binary_path.as_deref(),
        Some(system.to_str().expect("utf-8"))
    );
    assert_eq!(status.source, ResticSourceCode::System);
    assert_eq!(
        status.error_message().map(|m| m.english()).as_deref(),
        Some("restic 0.16.4 is too old; Kestic needs at least 0.18.0")
    );
    // (b) the blocking state offers the restic actions
    assert_eq!(
        blocking.as_ref().map(|b| b.kind),
        Some(BlockingKind::ResticUnavailable)
    );
    assert_eq!(
        blocking_message(blocking.as_ref(), &status).map(|m| m.english()),
        status.error_message().map(|m| m.english())
    );
}

/// AC-M5-07-5 (a), (d): the status reports both binaries; the switch to the system restic is
/// offered only while the app-managed source is chosen and the system restic is recent enough.
#[test]
fn ac_m5_07_5_status_reports_both_binaries_and_the_offer() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let system = json_stand_in(&dir.path().join("bin/restic"), "0.18.1");
    let app = json_stand_in(&dir.path().join("data/kestic/restic"), "0.19.1");
    let locations = ResticLocations {
        system: Some(system.clone()),
        app_managed: Some(app.clone()),
    };

    let (managed, _) = restic_status_with(&ResticProvider::AppManaged, &locations);
    // (a) the switch is offered
    assert!(managed.offer_switch_to_system);
    assert_eq!(managed.binary_path.as_deref(), app.to_str());
    assert_eq!(managed.source, ResticSourceCode::AppManaged);

    let (automatic, _) = restic_status_with(&ResticProvider::Automatic, &locations);
    // (d) «Automatic» uses the system restic without asking and still reports the other one
    assert!(!automatic.offer_switch_to_system);
    assert_eq!(automatic.binary_path.as_deref(), system.to_str());
    assert_eq!(
        automatic.system,
        Some(ResticBinaryDto {
            path: system.to_string_lossy().into_owned(),
            version: Some("0.18.1".to_string()),
            too_old: false,
        })
    );
    assert_eq!(
        automatic.app_managed,
        Some(ResticBinaryDto {
            path: app.to_string_lossy().into_owned(),
            version: Some("0.19.1".to_string()),
            too_old: false,
        })
    );

    // a system restic that is a link to the app-managed one is no reason to switch
    let linked = dir.path().join("linked/restic");
    fs::create_dir_all(linked.parent().expect("parent")).expect("mkdir");
    std::os::unix::fs::symlink(&app, &linked).expect("symlink");
    let (managed, _) = restic_status_with(
        &ResticProvider::AppManaged,
        &ResticLocations {
            system: Some(linked),
            app_managed: Some(app.clone()),
        },
    );
    assert!(!managed.offer_switch_to_system);

    let old_system = ResticLocations {
        system: Some(stand_in_0_16_4(&dir.path().join("old/restic"))),
        app_managed: Some(app),
    };
    let (managed, _) = restic_status_with(&ResticProvider::AppManaged, &old_system);
    assert!(!managed.offer_switch_to_system, "0.16.4 is not enough");
    assert!(managed.system.as_ref().is_some_and(|s| s.too_old));
}

/// AC-M5-07-7: an invalid custom path reports why, as a code next to the raw detail.
#[test]
fn ac_m5_07_7_status_reports_the_path_problem() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let missing = dir.path().join("missing");
    let (status, _) = restic_status_with(
        &ResticProvider::Custom {
            path: missing.clone(),
        },
        &ResticLocations::default(),
    );
    assert_eq!(status.error_kind, Some(ResticStatusCode::InvalidCustomPath));
    assert_eq!(
        status.path_problem,
        Some(kestic_i18n::codes::ResticPathProblem::NotFound)
    );
    assert!(status.detail.as_ref().is_some_and(|d| !d.is_empty()));
    assert_eq!(
        status.error_message().map(|m| m.english()),
        Some(format!(
            "Custom restic path is invalid: {} does not exist",
            missing.display()
        ))
    );
}
