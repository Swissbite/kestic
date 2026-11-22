// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Offscreen GUI smoke tests for Milestone M4 (restore area) against a daemon with a real
//! restic temp repository; screenshots in English and German under `target/smoke-artifacts/`.

mod common;

use common::m3::{FakeSetup, M3Rig, StdFakes, add_local_destination, latest_variant};
use common::m4::{RawSnapshot, backup_dir, big_tree, small_tree, write_snapshot};
use common::{find_kestic_binary, run_isolated_test};
use kestic_core::query_list::QueryLimits;
use kestic_daemon::dbus_api::KESTIC_BUS_NAME;
use std::path::Path;
use std::time::{Duration, Instant};
use time::macros::datetime;

fn run_m4_gui_smoke(root: &Path, scenario: &str, layout: &str, lang: &str, screenshot_rel: &str) {
    run_m4_gui_smoke_env(root, scenario, layout, lang, screenshot_rel, &[]);
}

fn run_m4_gui_smoke_env(
    root: &Path,
    scenario: &str,
    layout: &str,
    lang: &str,
    screenshot_rel: &str,
    env: &[(&str, &str)],
) {
    let iso = kestic_test_support::gui::IsolatedEnv::adopt(root);
    let dbus_addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("isolated session bus");
    kestic_test_support::gui::SmokeContext {
        kestic_bin: &find_kestic_binary(),
        iso: &iso,
        dbus_addr: &dbus_addr,
    }
    .run_with_env(scenario, layout, lang, screenshot_rel, &[], env);
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

/// A daemon on the real bus name with one destination «NAS» and the backup «Home» with two
/// snapshots: the newest of a small folder with sub-folders ([`small_tree`]), the one before of
/// 1 000 folders with 100 files each ([`big_tree`]). `limits` bounds the query lists.
fn browse_env(root: &Path, limits: QueryLimits) -> (M3Rig, StdFakes) {
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
        |o| {
            o.enable_scheduler = false;
            o.query_limits = limits;
        },
    );
    let repo = root.join("repo-NAS");
    let (dest, password) =
        add_local_destination(&rig.daemon.client, &variant.bin, &repo, "NAS", |_| {});
    let backup = rig.add_backup_to(&dest, "Home", |_| {});
    let tag = format!("kestic:{}", backup.id);
    let trees = root.join("trees");
    for (src, time) in [
        (big_tree(&trees), "2026-11-14 12:00:00"),
        (small_tree(&trees), "2026-11-15 12:00:00"),
    ] {
        backup_dir(
            &variant.bin,
            &repo,
            &password,
            &src,
            &RawSnapshot {
                tag: &tag,
                time,
                host: "laptop",
            },
        );
    }
    (rig, fakes)
}

/// AC-M4-03-3 (smoke part) and AC-M4-03-1 (a) on screen: with the keyboard alone the folder
/// list (a) opens sub-folders, (b) the path row leads back to every folder above (no tree), and
/// (c) entries can be marked for restore; the list shows name, size and modification time.
#[test]
fn ac_m4_03_3_browse_folders_and_mark_entries_with_the_keyboard() {
    run_isolated_test(
        "ac_m4_03_3_browse_folders_and_mark_entries_with_the_keyboard",
        "KESTIC_AC_M4_03_3_INNER",
        |root| {
            let (rig, _fakes) = browse_env(root, QueryLimits::default());
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m4_gui_smoke(
                    root,
                    "m4-browse-keyboard",
                    layout,
                    lang,
                    &smoke_artifact("restore-browse", layout, lang),
                );
            }
            assert!(
                rig.wrapper.of('S', "mount").is_empty(),
                "AC-M4-03-1 (b): browsing never uses restic mount"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M4-03-2 (c) and AC-M4-03-5 (smoke part): the view keeps reacting to input while the top
/// folder with 1 000 entries loads; it names 1 000 before scrolling, fetches pages of at most
/// 200 while scrolling and reaches every entry exactly once.
#[test]
fn ac_m4_03_5_folder_list_loads_page_by_page_while_scrolling() {
    run_isolated_test(
        "ac_m4_03_5_folder_list_loads_page_by_page_while_scrolling",
        "KESTIC_AC_M4_03_5_INNER",
        |root| {
            let (rig, _fakes) = browse_env(root, QueryLimits::default());
            for lang in ["en", "de"] {
                // Holds `restic ls` until the scene has typed into the window while it loads.
                rig.wrapper.hold("ls");
                let calls_before = rig.wrapper.of('E', "ls").len();
                let typed = rig.wrapper.log.with_file_name("browse-typed");
                let release = {
                    let hold_prefix = rig.wrapper.hold.clone();
                    let typed = typed.clone();
                    std::thread::spawn(move || release_ls_when_typed(&hold_prefix, &typed))
                };
                run_m4_gui_smoke_env(
                    root,
                    "m4-browse-paging",
                    "wide",
                    lang,
                    &smoke_artifact("restore-browse-paging", "wide", lang),
                    &[("KESTIC_SMOKE_TYPED_MARKER", &typed.to_string_lossy())],
                );
                release.join().expect("release thread");
                rig.wrapper.release("ls");
                assert_eq!(
                    rig.wrapper.of('E', "ls").len() - calls_before,
                    1,
                    "AC-M4-03-5: one restic ls for the folder, all pages from the daemon"
                );
            }
            rig.daemon.stop();
        },
    );
}

/// Releases a held `restic ls` once the smoke scene wrote the marker file `typed` (it typed
/// into the window while the folder loaded), or after 60 s.
fn release_ls_when_typed(hold_prefix: &Path, typed: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !typed.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(typed);
    let _ = std::fs::remove_file(format!("{}.ls", hold_prefix.display()));
}

/// AC-M4-03-7 (c): a folder list over the memory limit (a smaller limit for the test) shows
/// «Too many entries to show» instead of a partial list.
#[test]
fn ac_m4_03_7_list_over_the_limit_shows_too_many_entries() {
    run_isolated_test(
        "ac_m4_03_7_list_over_the_limit_shows_too_many_entries",
        "KESTIC_AC_M4_03_7_INNER",
        |root| {
            let limits = QueryLimits {
                per_query_bytes: 20_000,
                total_bytes: 40_000,
            };
            let (rig, _fakes) = browse_env(root, limits);
            for lang in ["en", "de"] {
                run_m4_gui_smoke(
                    root,
                    "m4-browse-too-large",
                    "wide",
                    lang,
                    &smoke_artifact("restore-browse-too-large", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// M4-S03: going back from a folder that still loads cancels its query, so `restic ls` stops.
#[test]
fn m4_s03_leaving_a_loading_folder_stops_restic_ls() {
    run_isolated_test(
        "m4_s03_leaving_a_loading_folder_stops_restic_ls",
        "KESTIC_M4_S03_LEAVE_INNER",
        |root| {
            let (rig, _fakes) = browse_env(root, QueryLimits::default());
            rig.wrapper.hold("ls");
            run_m4_gui_smoke(
                root,
                "m4-browse-leave",
                "wide",
                "en",
                &smoke_artifact("restore-browse-leave", "wide", "en"),
            );
            let deadline = Instant::now() + Duration::from_secs(15);
            while rig.wrapper.of('T', "ls").is_empty() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            rig.wrapper.release("ls");
            assert_eq!(rig.wrapper.of('T', "ls").len(), 1, "restic ls got SIGTERM");
            rig.daemon.stop();
        },
    );
}
