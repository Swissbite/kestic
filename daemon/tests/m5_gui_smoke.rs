// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Offscreen GUI smoke tests for Milestone M5 against an in-process daemon on an isolated
//! session bus. The daemon searches `ssh` and `rclone` in a temp directory of its own (`PATH`
//! fake), so the tests never depend on the programs installed on the machine.

mod common;

use common::m3::{FakeSetup, M3Rig, StdFakes, latest_variant};
use common::run_isolated_test;
use kestic_daemon::dbus_api::{DestinationTypeDto, KESTIC_BUS_NAME};
use kestic_i18n::codes::DestinationTypeCode;
use kestic_test_support::tools::write_executable;
use std::fs;
use std::path::{Path, PathBuf};
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
        |o| o.program_search_path = Some(search_path),
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
