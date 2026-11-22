// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Offscreen GUI smoke tests for Milestone M4 (restore area) against a daemon with a real
//! restic temp repository; screenshots in English and German under `target/smoke-artifacts/`.

mod common;

use common::m3::{FakeSetup, M3Rig, StdFakes, add_local_destination, latest_variant};
use common::m4::{RawSnapshot, write_snapshot};
use common::{find_kestic_binary, run_isolated_test};
use kestic_daemon::dbus_api::KESTIC_BUS_NAME;
use std::path::Path;
use time::macros::datetime;

fn run_m4_gui_smoke(root: &Path, scenario: &str, layout: &str, lang: &str, screenshot_rel: &str) {
    let iso = kestic_test_support::gui::IsolatedEnv::adopt(root);
    let dbus_addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("isolated session bus");
    kestic_test_support::gui::SmokeContext {
        kestic_bin: &find_kestic_binary(),
        iso: &iso,
        dbus_addr: &dbus_addr,
    }
    .run_with_env(scenario, layout, lang, screenshot_rel, &[], &[]);
}

fn smoke_artifact(name: &str, layout: &str, lang: &str) -> String {
    format!("target/smoke-artifacts/m4-{name}-{layout}-{lang}.png")
}

/// A daemon on the real bus name with one destination «NAS», the backups «Home» (three
/// snapshots: today, yesterday, five days ago) and «Photos» (none). Keep the fakes alive with
/// the rig.
fn restore_env(root: &Path) -> (M3Rig, StdFakes) {
    let fakes = StdFakes::start_with(FakeSetup {
        active_network: Some("HomeWifi"),
        ..FakeSetup::default()
    });
    let variant = latest_variant();
    let rig = M3Rig::start_on(
        root,
        &variant,
        "",
        KESTIC_BUS_NAME,
        datetime!(2026-11-15 14:32:00 UTC),
        |o| o.enable_scheduler = false,
    );
    let repo = root.join("repo-NAS");
    let (dest, password) =
        add_local_destination(&rig.daemon.client, &variant.bin, &repo, "NAS", |_| {});
    let backup = rig.add_backup_to(&dest, "Home", |_| {});
    rig.add_backup_to(&dest, "Photos", |_| {});
    let tag = format!("kestic:{}", backup.id);
    for (time, host) in [
        ("2026-11-10 12:00:00", "desktop"),
        ("2026-11-14 12:00:00", "laptop"),
        ("2026-11-15 12:00:00", "laptop"),
    ] {
        write_snapshot(
            &variant.bin,
            &repo,
            &password,
            root,
            &RawSnapshot {
                tag: &tag,
                time,
                host,
            },
        );
    }
    (rig, fakes)
}

/// AC-M4-01-6: (a) the tab bar shows «Restore» between «Overview» and «Backups» (checked by
/// every scene, wide and narrow), (b) the area opens the snapshot selection.
#[test]
fn ac_m4_01_6_restore_tab_between_overview_and_backups_opens_snapshot_selection() {
    run_isolated_test(
        "ac_m4_01_6_restore_tab_between_overview_and_backups_opens_snapshot_selection",
        "KESTIC_AC_M4_01_6_INNER",
        |root| {
            let (rig, _fakes) = restore_env(root);
            for (layout, lang) in [
                ("wide", "en"),
                ("wide", "de"),
                ("narrow", "en"),
                ("narrow", "de"),
            ] {
                run_m4_gui_smoke(
                    root,
                    "m4-restore-tab",
                    layout,
                    lang,
                    &smoke_artifact("restore-snapshot-list", layout, lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M4-01-4 (smoke part): with the keyboard alone (a) every snapshot can be selected and
/// (b) browsing, comparing and restoring can be started from the snapshot list.
#[test]
fn ac_m4_01_4_snapshot_list_works_with_the_keyboard_alone() {
    run_isolated_test(
        "ac_m4_01_4_snapshot_list_works_with_the_keyboard_alone",
        "KESTIC_AC_M4_01_4_INNER",
        |root| {
            let (rig, _fakes) = restore_env(root);
            for lang in ["en", "de"] {
                run_m4_gui_smoke(
                    root,
                    "m4-restore-keyboard",
                    "wide",
                    lang,
                    &smoke_artifact("restore-keyboard", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}
