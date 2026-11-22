// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Offscreen GUI smoke tests for Milestone M5 against an in-process daemon on an isolated
//! session bus. The daemon searches `ssh` and `rclone` in a temp directory of its own (`PATH`
//! fake), so the tests never depend on the programs installed on the machine.

mod common;

use common::fakes::{FakeUDisks2Filesystem, FakeUDisks2Handle};
use common::m3::{FakeSetup, M3DaemonOpts, M3Rig, StdFakes, latest_variant};
use common::run_isolated_test;
use kestic_core::model::DestinationKind;
use kestic_core::mount::{FakeMountInspector, FakeMountState, MountInspector};
use kestic_daemon::dbus_api::{DestinationTypeDto, KESTIC_BUS_NAME};
use kestic_i18n::codes::DestinationTypeCode;
use kestic_test_support::s3_server::S3ServerGuard;
use kestic_test_support::tools::write_executable;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use time::macros::datetime;

fn smoke_artifact(name: &str, layout: &str, lang: &str) -> String {
    format!("target/smoke-artifacts/m5-{name}-{layout}-{lang}.png")
}

fn run_m5_gui_smoke(
    root: &Path,
    scenario: &str,
    layout: &str,
    lang: &str,
    screenshot_rel: &str,
    extra_env: &[(&str, &str)],
) {
    let iso = kestic_test_support::gui::IsolatedEnv::adopt(root);
    let dbus_addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("isolated session bus");
    kestic_test_support::gui::SmokeContext {
        kestic_bin: &common::find_kestic_binary(),
        iso: &iso,
        dbus_addr: &dbus_addr,
    }
    .run_with_env(scenario, layout, lang, screenshot_rel, &[], extra_env);
}

/// A program directory holding only a stand-in for `ssh`, and a daemon that searches it.
fn start_with_ssh_only(root: &Path) -> (M3Rig, StdFakes, PathBuf) {
    start_with_ssh_only_and(root, |_| {})
}

/// [`start_with_ssh_only`] with further daemon options set by `configure`.
fn start_with_ssh_only_and(
    root: &Path,
    configure: impl FnOnce(&mut M3DaemonOpts<'_>),
) -> (M3Rig, StdFakes, PathBuf) {
    let programs = root.join("programs");
    fs::create_dir_all(&programs).expect("mkdir programs");
    write_executable(&programs.join("ssh"), "#!/bin/sh\nexit 0\n");
    let fakes = StdFakes::start_with(FakeSetup::default());
    let search_path = programs.clone().into_os_string();
    let rig = M3Rig::start_on(
        root,
        &latest_variant(),
        "",
        KESTIC_BUS_NAME,
        datetime!(2026-12-01 10:00:00 UTC),
        |o| {
            o.program_search_path = Some(search_path);
            configure(o);
        },
    );
    (rig, fakes, programs)
}

fn ready(types: &[DestinationTypeDto], kind: DestinationTypeCode) -> bool {
    types
        .iter()
        .find(|t| t.kind == kind)
        .unwrap_or_else(|| panic!("{} missing in {types:?}", kind.as_str()))
        .ready
}

/// AC-M5-04-1 and AC-M5-04-3 (GUI smoke): with `ssh` and without `rclone` in the daemon's
/// `PATH`, `ListDestinationTypes` reports (a) SFTP ready and (b) rclone needing installation;
/// the wizard shows (a) the ready cards under «Ready», (b) rclone under «Needs Installation» and
/// (c) the ready cards first; the opened rclone entry shows (AC-M5-04-3 a) the package `rclone`,
/// (b) no «Open in Discover», as rclone has no AppStream component, and (c)
/// `sudo apt install rclone`.
/// The C++ scenario checks each part; screenshots in English and German, wide and narrow.
#[test]
fn ac_m5_04_1_and_3_wizard_lists_ready_types_first_and_install_help() {
    run_isolated_test(
        "ac_m5_04_1_and_3_wizard_lists_ready_types_first_and_install_help",
        "KESTIC_M5_GUI_TYPES_INNER",
        |root| {
            let (rig, _fakes, _programs) = start_with_ssh_only(root);
            let types = rig
                .daemon
                .client
                .list_destination_types()
                .expect("ListDestinationTypes");
            // (a)
            assert!(
                ready(&types, DestinationTypeCode::Sftp),
                "(a) SFTP ready over D-Bus"
            );
            // (b)
            assert!(
                !ready(&types, DestinationTypeCode::Rclone),
                "(b) rclone needs installation over D-Bus"
            );
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m5_gui_smoke(
                    root,
                    "m5-destination-types",
                    layout,
                    lang,
                    &smoke_artifact("destination-types", layout, lang),
                    &[],
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M5-04-4: rclone put into the daemon's `PATH` afterwards; (a) the next
/// `ListDestinationTypes` reports it ready, and in the wizard «Check Again» moves the open
/// rclone entry under «Ready».
#[test]
fn ac_m5_04_4_check_again_finds_rclone_added_later() {
    run_isolated_test(
        "ac_m5_04_4_check_again_finds_rclone_added_later",
        "KESTIC_M5_GUI_CHECK_AGAIN_INNER",
        |root| {
            let (rig, _fakes, programs) = start_with_ssh_only(root);
            let client = &rig.daemon.client;
            assert!(
                !ready(
                    &client.list_destination_types().expect("list"),
                    DestinationTypeCode::Rclone
                ),
                "precondition: rclone missing"
            );
            write_executable(&programs.join("rclone"), "#!/bin/sh\nexit 0\n");
            // (a) over D-Bus
            assert!(
                ready(
                    &client.list_destination_types().expect("list again"),
                    DestinationTypeCode::Rclone
                ),
                "(a) rclone ready on the next call"
            );

            fs::remove_file(programs.join("rclone")).expect("remove rclone again");
            let program_dir = programs.to_str().expect("utf-8 path");
            for lang in ["en", "de"] {
                run_m5_gui_smoke(
                    root,
                    "m5-destination-types-check-again",
                    "wide",
                    lang,
                    &smoke_artifact("destination-types-check-again", "wide", lang),
                    &[("KESTIC_SMOKE_PROGRAM_DIR", program_dir)],
                );
                fs::remove_file(programs.join("rclone")).expect("the scenario wrote rclone");
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M5-05-1 (GUI part of the D-Bus test in `m5_s05_drives`): the UDisks2 fake reports the
/// removable drive «USB-Disk» next to an internal disk; the type «External Drive / USB» offers
/// (a) the drive with name and mount point, and choosing it sets the path. Screenshots in
/// English and German, wide and narrow.
#[test]
fn m5_05_1_gui_drive_type_offers_the_removable_drive() {
    run_isolated_test(
        "m5_05_1_gui_drive_type_offers_the_removable_drive",
        "KESTIC_M5_GUI_DRIVES_INNER",
        |root| {
            let udisks = FakeUDisks2Handle::start();
            for (device, label, mount_point, removable) in [
                ("sdb1", "USB-Disk", "/media/user/USB-Disk", true),
                ("nvme0n1p2", "System", "/", false),
            ] {
                udisks.add_filesystem(FakeUDisks2Filesystem {
                    device_name: device.to_string(),
                    label: label.to_string(),
                    uuid: format!("uuid-{device}"),
                    mount_points: vec![mount_point.to_string()],
                    drive_name: format!("drive_{device}"),
                    size_bytes: 64_000_000_000,
                    removable,
                    connection_bus: if removable { "usb" } else { "" }.to_string(),
                });
            }
            let (rig, _fakes, _programs) = start_with_ssh_only(root);
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m5_gui_smoke(
                    root,
                    "m5-drive-list",
                    layout,
                    lang,
                    &smoke_artifact("drive-list", layout, lang),
                    &[],
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M5-05-2 (GUI part; detection in `core`, D-Bus in `m5_s05_drives`): a folder on a CIFS
/// mount (mountinfo fake); the wizard (a) warns that restic advises against repositories on
/// CIFS/SMB shares and keeps «Next» disabled until the warning is confirmed, and (b) saves the
/// destination after the confirmation. Screenshots of the warning in English and German.
#[test]
fn m5_05_2_gui_cifs_warning_needs_confirmation() {
    run_isolated_test(
        "m5_05_2_gui_cifs_warning_needs_confirmation",
        "KESTIC_M5_GUI_CIFS_INNER",
        |root| {
            let share = root.join("mnt").join("nas");
            fs::create_dir_all(&share).expect("mkdir share");
            let mounts = Arc::new(FakeMountInspector::new(FakeMountState {
                mountinfo: format!(
                    "110 25 0:45 / {} rw,relatime - cifs //nas/backups rw\n",
                    share.display()
                ),
                ..FakeMountState::default()
            }));
            let (rig, _fakes, _programs) = start_with_ssh_only_and(root, |o| {
                o.mount_inspector = Some(Arc::clone(&mounts) as Arc<dyn MountInspector>);
            });
            for lang in ["en", "de"] {
                let repo = share.join(format!("kestic-{lang}"));
                fs::create_dir_all(&repo).expect("mkdir repo");
                let repo_text = repo.to_str().expect("utf-8 path");
                run_m5_gui_smoke(
                    root,
                    "m5-cifs-note",
                    "wide",
                    lang,
                    &smoke_artifact("cifs-note", "wide", lang),
                    &[("KESTIC_SMOKE_CIFS_DIR", repo_text)],
                );
                // (b)
                let state = rig.daemon.client.get_state().expect("GetState");
                assert!(
                    state.destinations.iter().any(|d| matches!(
                        &d.destination.kind,
                        DestinationKind::Local { path } if path == &repo
                    )),
                    "(b) destination on the share saved ({lang}): {:?}",
                    state.destinations
                );
            }
            rig.daemon.stop();
        },
    );
}

/// M5-S02 (GUI part of `AC-M5-02-3`; D-Bus in `m5_s02_s3`): the card «Cloud Storage» offers
/// «S3-Compatible Storage» and opens the S3 form; against `rclone serve s3` a wrong secret key
/// (a) shows what to do and the action «Check Keys», and the right key passes the test. (b) No
/// destination is saved by the tests. Screenshots of the failed test in English and German.
#[test]
fn m5_02_3_gui_s3_form_wrong_secret_key_shows_action() {
    run_isolated_test(
        "m5_02_3_gui_s3_form_wrong_secret_key_shows_action",
        "KESTIC_M5_GUI_S3_INNER",
        |root| {
            let (rig, _fakes, _programs) = start_with_ssh_only(root);
            let server = S3ServerGuard::start(&root.join("s3-data"), "kestic-en");
            fs::create_dir_all(server.bucket_dir("kestic-de")).expect("mkdir bucket");
            for lang in ["en", "de"] {
                run_m5_gui_smoke(
                    root,
                    "m5-s3-form",
                    "wide",
                    lang,
                    &smoke_artifact("s3-form-wrong-key", "wide", lang),
                    &[
                        ("KESTIC_SMOKE_S3_ENDPOINT", &server.endpoint),
                        ("KESTIC_SMOKE_S3_ACCESS_KEY", &server.access_key),
                        ("KESTIC_SMOKE_S3_SECRET_KEY", &server.secret_key),
                    ],
                );
            }
            // (b)
            let state = rig.daemon.client.get_state().expect("GetState");
            assert!(
                state.destinations.is_empty(),
                "(b) {:?}",
                state.destinations
            );
            rig.daemon.stop();
        },
    );
}
