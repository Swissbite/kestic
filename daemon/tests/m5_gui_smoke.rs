// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Offscreen GUI smoke tests for Milestone M5 against an in-process daemon on an isolated
//! session bus. The daemon searches `ssh` and `rclone` in a temp directory of its own (`PATH`
//! fake), so the tests never depend on the programs installed on the machine.

mod common;

use common::fakes::{FakeUDisks2Filesystem, FakeUDisks2Handle};
use common::m3::{FakeSetup, M3DaemonOpts, M3Rig, StdFakes, latest_variant};
use common::{SYSTEM_PROGRAMS_DIR, run_isolated_test, run_isolated_test_with_system_programs};
use kestic_core::config::{AppConfig, save_config};
use kestic_core::model::{DestinationKind, ResticProvider};
use kestic_core::mount::{FakeMountInspector, FakeMountState, MountInspector};
use kestic_core::restic_install::{app_managed_binary, kestic_data_dir};
use kestic_core::restic_release::{TrustAnchor, set_test_anchor};
use kestic_daemon::dbus_api::{DestinationTypeDto, KESTIC_BUS_NAME};
use kestic_i18n::codes::DestinationTypeCode;
use kestic_test_support::release_mock::{
    ReleaseFiles, ReleaseMock, Route, TestSigningKey, asset_path,
};
use kestic_test_support::s3_server::S3ServerGuard;
use kestic_test_support::sftp_server::SftpServerGuard;
use kestic_test_support::tools::{free_loopback_port, write_executable};
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
/// (a) shows what to do and the action «Check Keys», and the right key passes the test and names
/// the bucket that does not exist yet. (b) No destination is saved by the tests. Screenshots of
/// the path hint, the failed test and the missing bucket in English and German, wide and narrow.
#[test]
fn m5_02_3_gui_s3_form_wrong_secret_key_shows_action() {
    run_isolated_test(
        "m5_02_3_gui_s3_form_wrong_secret_key_shows_action",
        "KESTIC_M5_GUI_S3_INNER",
        |root| {
            let (rig, _fakes, _programs) = start_with_ssh_only(root);
            // The buckets `kestic-en` and `kestic-de` of the scenario do not exist yet.
            let server = S3ServerGuard::start(&root.join("s3-data"), "other");
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m5_gui_smoke(
                    root,
                    "m5-s3-form",
                    layout,
                    lang,
                    &smoke_artifact("s3-form-wrong-key", layout, lang),
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

/// M5-S03 (GUI parts of AC-M5-03-1 (c), AC-M5-03-4 and AC-M5-03-7 (b)): «Cloud Storage» offers
/// B2, Azure, Google Cloud and Swift; B2 opens the generic form with `b2:`; an unknown scheme and
/// a reserved variable keep «Test Connection» locked with `core`'s message; «Geheim» starts as the
/// name suggests and can be switched. No destination is saved. Screenshots of the filled form and
/// of the refused variable in English and German, wide and narrow.
#[test]
fn m5_03_gui_generic_form() {
    run_isolated_test(
        "m5_03_gui_generic_form",
        "KESTIC_M5_GUI_GENERIC_INNER",
        |root| {
            let (rig, _fakes, _programs) = start_with_ssh_only(root);
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m5_gui_smoke(
                    root,
                    "m5-generic-form",
                    layout,
                    lang,
                    &smoke_artifact("generic-form", layout, lang),
                    &[],
                );
            }
            let state = rig.daemon.client.get_state().expect("GetState");
            assert!(state.destinations.is_empty(), "no destination saved");
            rig.daemon.stop();
        },
    );
}

/// M5-S03 (GUI): with `rclone` installed, the rclone card opens the generic form asking for the
/// rclone remote only. Screenshots in English and German, wide and narrow.
#[test]
fn m5_03_gui_rclone_form() {
    run_isolated_test(
        "m5_03_gui_rclone_form",
        "KESTIC_M5_GUI_RCLONE_INNER",
        |root| {
            let (rig, _fakes, programs) = start_with_ssh_only(root);
            write_executable(&programs.join("rclone"), "#!/bin/sh\nexit 0\n");
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m5_gui_smoke(
                    root,
                    "m5-rclone-form",
                    layout,
                    lang,
                    &smoke_artifact("rclone-form", layout, lang),
                    &[],
                );
            }
            rig.daemon.stop();
        },
    );
}

/// M5-S01 (GUI parts of AC-M5-01-1 and AC-M5-01-4 (a)): the SFTP form with host, port, user,
/// path and the hint on the SSH key and the ssh-agent; the connection test against an unknown
/// server asks with key type and SHA256 fingerprint, and «Trust Server» lets the test pass. The
/// C++ scenario checks each part; Kestic's `known_hosts` is removed before every run, so each
/// language meets an unknown server.
#[test]
fn m5_01_gui_sftp_form_asks_to_trust_an_unknown_server() {
    run_isolated_test(
        "m5_01_gui_sftp_form_asks_to_trust_an_unknown_server",
        "KESTIC_M5_GUI_SFTP_INNER",
        |root| {
            let _fakes = StdFakes::start_with(FakeSetup::default());
            let rig = M3Rig::start_on(
                root,
                &latest_variant(),
                "",
                KESTIC_BUS_NAME,
                datetime!(2026-12-01 10:00:00 UTC),
                |_| {},
            );
            let server = SftpServerGuard::start(&root.join("sshd"));
            kestic_core::sftp::set_test_ssh_config(Some(server.ssh_config.clone()));
            let paths = kestic_core::sftp::KnownHostsPaths::from_env().expect("known_hosts paths");
            let listed = std::process::Command::new("ssh-keygen")
                .args(["-l", "-E", "sha256", "-f"])
                .arg(root.join("sshd").join("host_key.pub"))
                .output()
                .expect("ssh-keygen -l");
            let fingerprint = String::from_utf8_lossy(&listed.stdout)
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let port = server.port.to_string();
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                let _ = fs::remove_file(&paths.kestic);
                let repo = root.join(format!("sftp-repo-{layout}-{lang}"));
                run_m5_gui_smoke(
                    root,
                    "m5-sftp-form",
                    layout,
                    lang,
                    &smoke_artifact("sftp-form", layout, lang),
                    &[
                        ("KESTIC_SMOKE_SFTP_PORT", &port),
                        ("KESTIC_SMOKE_SFTP_USER", &server.user),
                        ("KESTIC_SMOKE_SFTP_PATH", &repo.to_string_lossy()),
                        ("KESTIC_SMOKE_SFTP_FINGERPRINT", &fingerprint),
                    ],
                );
                assert_eq!(
                    fs::read_to_string(&paths.kestic).expect("trusted key written"),
                    format!("{}\n", server.known_hosts_line()),
                    "{layout}/{lang}: the trusted key"
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M5-05-7 (GUI smoke): a local destination whose path is changed in «Edit Connection» to a
/// folder with a repository of its own `init` (the same password): the connection test shows (b)
/// «There is another backup destination at this address.» and (a) «Save» stays disabled; (c)
/// «Add as New Destination…» opens the wizard at its details with the entered type, path and
/// name, without the edited destination's ID or any password. The C++ scenario checks each part;
/// screenshots in English and German, wide and narrow.
#[test]
fn ac_m5_05_7_gui_other_repository_message_and_add_as_new() {
    run_isolated_test(
        "ac_m5_05_7_gui_other_repository_message_and_add_as_new",
        "KESTIC_M5_GUI_EDIT_OTHER_INNER",
        |root| {
            let _fakes = StdFakes::start_with(FakeSetup::default());
            let rig = M3Rig::start_on(
                root,
                &latest_variant(),
                "",
                KESTIC_BUS_NAME,
                datetime!(2026-12-01 10:00:00 UTC),
                |_| {},
            );
            let (dest, password) = common::m3::add_local_destination(
                &rig.daemon.client,
                &rig.variant.bin,
                &root.join("repo"),
                "Disk",
                |_| {},
            );
            let other = root.join("other");
            let out = std::process::Command::new(&rig.variant.bin)
                .args(["-r", &other.to_string_lossy(), "--no-cache", "init"])
                .env("RESTIC_PASSWORD", &password)
                .output()
                .expect("restic init");
            assert!(out.status.success(), "restic init: {out:?}");
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m5_gui_smoke(
                    root,
                    "m5-edit-other-repository",
                    layout,
                    lang,
                    &smoke_artifact("edit-other-repository", layout, lang),
                    &[
                        ("KESTIC_SMOKE_DEST_ID", dest.id.as_str()),
                        ("KESTIC_SMOKE_DEST_NAME", &dest.name),
                        ("KESTIC_SMOKE_OTHER_PATH", &other.to_string_lossy()),
                    ],
                );
            }
            let config = common::load_config(&rig.daemon.config_path).expect("config");
            assert_eq!(
                config.destination(&dest.id).map(|d| &d.kind),
                Some(&dest.kind),
                "(a) the edited path was not saved"
            );
            rig.daemon.stop();
        },
    );
}

/// Newer than every real restic the tests run.
const RELEASE: &str = "0.99.0";

/// Like the real restic 0.16.4: no `version --json`, only the text of `restic version`.
const RESTIC_0_16_4: &str = "#!/bin/sh\nif [ \"$1\" = version ] && [ -z \"$2\" ]; then\n  \
     echo 'restic 0.16.4 compiled with go1.22.2 on linux/amd64'\n  exit 0\nfi\nexit 1\n";

/// Makes the daemon of this process trust `key` and download from `base_url`.
fn trust(base_url: &str, key: &TestSigningKey) {
    let key_path = key.public_key_file().to_string_lossy().into_owned();
    let fpr = key.fingerprint().to_string();
    let anchor = TrustAnchor::from_lookup(|name| match name {
        "KESTIC_TEST_RELEASE_URL" => Some(base_url.to_string()),
        "KESTIC_TEST_SIGNING_KEY" => Some(key_path.clone()),
        "KESTIC_TEST_SIGNING_FPR" => Some(fpr.clone()),
        _ => None,
    })
    .expect("test anchor");
    set_test_anchor(Some(anchor));
}

/// A release mock for `files`, trusted by the daemon of this process.
fn serve(files: &ReleaseFiles, key: &TestSigningKey, routes: Vec<(String, Route)>) -> ReleaseMock {
    let mock = ReleaseMock::start_with(files, routes);
    trust(&mock.base_url(), key);
    mock
}

/// The app-managed restic of this isolated test (`$XDG_DATA_HOME/kestic/restic`).
fn managed_restic() -> PathBuf {
    app_managed_binary(
        &kestic_data_dir(
            std::env::var_os("HOME").as_deref(),
            std::env::var_os("XDG_DATA_HOME").as_deref(),
        )
        .expect("data dir"),
    )
}

/// A daemon on the GUI's bus name with the restic source `source`; `None` keeps the default
/// configuration (system restic from `PATH`).
fn start_with_source(root: &Path, source: Option<ResticProvider>) -> M3Rig {
    if let Some(restic_provider) = source {
        let (config_path, _) = common::m3::daemon_paths(root, "");
        fs::create_dir_all(config_path.parent().expect("cfg dir")).expect("cfg dir");
        let config = AppConfig {
            restic_provider,
            ..Default::default()
        };
        save_config(&config_path, &config).expect("save config");
    }
    M3Rig::start_on(
        root,
        &latest_variant(),
        "",
        KESTIC_BUS_NAME,
        datetime!(2026-12-01 10:00:00 UTC),
        |o| {
            o.enable_monitors = false;
            o.enable_scheduler = false;
            o.set_restic_provider = false;
        },
    )
}

/// System restic 0.16.4 first in `PATH`, no app-managed restic, the default source.
fn start_with_old_system_restic(root: &Path) -> M3Rig {
    write_executable(
        &root.join(SYSTEM_PROGRAMS_DIR).join("restic"),
        RESTIC_0_16_4,
    );
    let rig = start_with_source(root, None);
    let status = rig.daemon.client.get_state().expect("state").restic_status;
    assert!(!status.valid, "precondition: {status:?}");
    assert_eq!(status.found_version.as_deref(), Some("0.16.4"));
    rig
}

/// Runs `m5-restic-install-error` with a click on `button`, in English and German; each run
/// expects the text of its language.
fn run_install_error(root: &Path, name: &str, button: &str, texts: [(&str, &str); 2]) {
    for (lang, expected) in texts {
        run_m5_gui_smoke(
            root,
            "m5-restic-install-error",
            "wide",
            lang,
            &smoke_artifact(name, "wide", lang),
            &[
                ("KESTIC_SMOKE_RESTIC_BUTTON", button),
                ("KESTIC_SMOKE_EXPECT_ERROR", expected),
            ],
        );
    }
}

/// AC-M5-06-12 (b): system restic 0.16.4 and no app-managed restic; the window offers (b1)
/// «Download restic…» in the blocking bar and (b2) «Download restic» in the restic section of
/// the settings (the C++ scenario checks each); the release server was not asked (precondition
/// (a) holds while the GUI shows the offer). Screenshots in English and German, wide and narrow.
#[test]
fn ac_m5_06_12_b_gui_offers_download() {
    run_isolated_test_with_system_programs(
        "ac_m5_06_12_b_gui_offers_download",
        "KESTIC_M5_GUI_RESTIC_OFFER_INNER",
        |root| {
            let key = TestSigningKey::generate("kestic-test-release");
            let mock = serve(&ReleaseFiles::signed(RELEASE, &key), &key, Vec::new());
            let rig = start_with_old_system_restic(root);
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                let settings_shot = if layout == "wide" {
                    smoke_artifact("restic-download-offer-settings", layout, lang)
                } else {
                    String::new()
                };
                run_m5_gui_smoke(
                    root,
                    "m5-restic-download-offer",
                    layout,
                    lang,
                    &smoke_artifact("restic-download-offer", layout, lang),
                    &[("KESTIC_SMOKE_SETTINGS_SHOT", &settings_shot)],
                );
            }
            assert_eq!(
                mock.requests(),
                Vec::<String>::new(),
                "(a) no release access"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M5-06-4 (a), GUI: `SHA256SUMS.asc` signed with another key; after «Download restic» the
/// settings show the message for an invalid signature (English and German) with the raw detail
/// under «Details»; no binary is placed.
#[test]
fn ac_m5_06_4_gui_signature_message() {
    run_isolated_test_with_system_programs(
        "ac_m5_06_4_gui_signature_message",
        "KESTIC_M5_GUI_RESTIC_SIGNATURE_INNER",
        |root| {
            let trusted = TestSigningKey::generate("kestic-test-release");
            let other = TestSigningKey::generate("kestic-other-key");
            let _mock = serve(&ReleaseFiles::signed(RELEASE, &other), &trusted, Vec::new());
            let rig = start_with_old_system_restic(root);
            run_install_error(
                root,
                "restic-signature-error",
                "downloadResticButton",
                [
                    (
                        "en",
                        "The signature of the restic release is not valid. Nothing was installed.",
                    ),
                    (
                        "de",
                        "Die Signatur des restic-Release ist nicht gültig. Es wurde nichts installiert.",
                    ),
                ],
            );
            assert!(!managed_restic().exists(), "no binary placed");
            rig.daemon.stop();
        },
    );
}

/// AC-M5-06-5 (a), GUI: the app-managed restic 0.18.0 is the source and the archive does not
/// match `SHA256SUMS`; after «Update Now» the settings show the checksum message (English and
/// German); the binary and the source stay as they were.
#[test]
fn ac_m5_06_5_gui_checksum_message() {
    run_isolated_test(
        "ac_m5_06_5_gui_checksum_message",
        "KESTIC_M5_GUI_RESTIC_CHECKSUM_INNER",
        |root| {
            let binary = managed_restic();
            fs::create_dir_all(binary.parent().expect("data dir")).expect("data dir");
            write_executable(
                &binary,
                "#!/bin/sh\necho '{\"message_type\":\"version\",\"version\":\"0.18.0\"}'\n",
            );
            let before = fs::read(&binary).expect("binary");
            let key = TestSigningKey::generate("kestic-test-release");
            let mut files = ReleaseFiles::signed(RELEASE, &key);
            for arch in ["amd64", "arm64"] {
                *files
                    .assets
                    .get_mut(&files.archive_name(arch))
                    .expect("archive") = b"tampered".to_vec();
            }
            let _mock = serve(&files, &key, Vec::new());
            let source = ResticProvider::AppManaged {
                path: binary.clone(),
            };
            let rig = start_with_source(root, Some(source.clone()));
            run_install_error(
                root,
                "restic-checksum-error",
                "updateResticButton",
                [
                    (
                        "en",
                        "The downloaded restic does not match its checksum. Nothing was installed.",
                    ),
                    (
                        "de",
                        "Das heruntergeladene restic passt nicht zu seiner Prüfsumme. Es wurde nichts installiert.",
                    ),
                ],
            );
            assert_eq!(
                fs::read(&binary).expect("binary"),
                before,
                "binary unchanged"
            );
            let state = rig.daemon.client.get_state().expect("state");
            assert_eq!(state.config.restic_provider, source, "source unchanged");
            rig.daemon.stop();
        },
    );
}

/// AC-M5-06-6 (a), (b), GUI: the release server cannot be reached; after «Download restic»
/// the settings show (a) the network message (English and German), and (b) the restic source
/// is the same as before.
#[test]
fn ac_m5_06_6_gui_network_message() {
    run_isolated_test_with_system_programs(
        "ac_m5_06_6_gui_network_message",
        "KESTIC_M5_GUI_RESTIC_NETWORK_INNER",
        |root| {
            let key = TestSigningKey::generate("kestic-test-release");
            trust(&format!("http://127.0.0.1:{}", free_loopback_port()), &key);
            let rig = start_with_old_system_restic(root);
            let before = rig.daemon.client.get_state().expect("state").config;
            run_install_error(
                root,
                "restic-network-error",
                "downloadResticButton",
                [
                    (
                        "en",
                        "Could not download restic. Check the internet connection and try again.",
                    ),
                    (
                        "de",
                        "restic konnte nicht heruntergeladen werden. Prüfen Sie die Internetverbindung und versuchen Sie es erneut.",
                    ),
                ],
            );
            let after = rig.daemon.client.get_state().expect("state").config;
            // (b)
            assert_eq!(after.restic_provider, before.restic_provider);
            assert!(!managed_restic().exists(), "no binary placed");
            rig.daemon.stop();
        },
    );
}

/// Progress, cancel and result in the settings: the app-managed restic is the source but
/// missing. A stalled download shows its step, a progress bar and «Cancel»; the cancel ends it
/// without a binary (English and German). Then «Download restic…» in the blocking bar opens
/// the settings at «Download restic» without starting anything; after the click there the
/// release is installed while the settings are closed: the bar disappears and the settings
/// show the installed version.
#[test]
fn restic_install_gui_progress_cancel_and_result() {
    run_isolated_test(
        "restic_install_gui_progress_cancel_and_result",
        "KESTIC_M5_GUI_RESTIC_PROGRESS_INNER",
        |root| {
            let key = TestSigningKey::generate("kestic-test-release");
            let files = ReleaseFiles::signed(RELEASE, &key);
            let archive = asset_path(RELEASE, &files.archive_name("amd64"));
            let stalled = serve(&files, &key, vec![(archive, Route::StallAfterHeaders)]);
            let rig = start_with_source(
                root,
                Some(ResticProvider::AppManaged {
                    path: managed_restic(),
                }),
            );
            for lang in ["en", "de"] {
                run_m5_gui_smoke(
                    root,
                    "m5-restic-install-cancel",
                    "wide",
                    lang,
                    &smoke_artifact("restic-install-progress", "wide", lang),
                    &[],
                );
            }
            assert!(!managed_restic().exists(), "no binary after the cancel");
            drop(stalled);

            let _mock = serve(&files, &key, Vec::new());
            run_m5_gui_smoke(
                root,
                "m5-restic-install-result",
                "wide",
                "en",
                &smoke_artifact("restic-install-result", "wide", "en"),
                &[("KESTIC_SMOKE_RELEASE", RELEASE)],
            );
            assert!(managed_restic().exists(), "the binary is installed");
            rig.daemon.stop();
        },
    );
}
