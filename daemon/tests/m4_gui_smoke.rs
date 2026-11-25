// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Offscreen GUI smoke tests for Milestone M4 (restore area) against a daemon with a real
//! restic temp repository; screenshots in English and German under `target/smoke-artifacts/`.

mod common;

use common::m3::{
    FakeSetup, M3Rig, ResticVariant, StdFakes, add_local_destination, latest_variant,
};
use common::m4::{
    RawSnapshot, backup_dir, backup_stdin, big_tree, small_tree, snapshot_username, write_file,
    write_snapshot,
};
use common::{
    click_dbusmenu_item, fetch_dbusmenu_items, find_kestic_binary, run_isolated_test,
    wait_for_sni_service,
};
use kestic_core::query_list::QueryLimits;
use kestic_core::scheduler::TimezoneSpec;
use kestic_daemon::dbus_api::{KESTIC_BUS_NAME, KESTIC_INTERFACE_V1};
use std::path::Path;
use std::sync::mpsc;
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
    restore_env_with(
        root,
        &[
            ("2026-11-10 12:00:00", "desktop"),
            ("2026-11-14 12:00:00", "laptop"),
            ("2026-11-15 12:00:00", "laptop"),
        ],
    )
}

/// Like [`restore_env`], with the snapshots `(time, host)` of «Home».
fn restore_env_with(root: &Path, snapshots: &[(&str, &str)]) -> (M3Rig, StdFakes) {
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
    for &(time, host) in snapshots {
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
    browse_env_in(root, &root.join("trees"), limits)
}

/// [`browse_env`] with both trees below `trees` instead of `<root>/trees`.
fn browse_env_in(root: &Path, trees: &Path, limits: QueryLimits) -> (M3Rig, StdFakes) {
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
    for (src, time) in [
        (big_tree(trees), "2026-11-14 12:00:00"),
        (small_tree(trees), "2026-11-15 12:00:00"),
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

/// AC-M4-03-1 (a), breadcrumb: every folder of the path row is drawn once (no second name or
/// icon from the style beneath the button content), folder names stay literal plain text (the
/// path holds a folder «R&D <b>x<b>») and the top folder keeps its accessible name.
#[test]
fn ac_m4_03_1_breadcrumb_draws_each_folder_once() {
    run_isolated_test(
        "ac_m4_03_1_breadcrumb_draws_each_folder_once",
        "KESTIC_AC_M4_03_1_PATH_ROW_INNER",
        |root| {
            let trees = root.join("R&D <b>x<b>");
            let (rig, _fakes) = browse_env_in(root, &trees, QueryLimits::default());
            for lang in ["en", "de"] {
                run_m4_gui_smoke(
                    root,
                    "m4-browse-path-row",
                    "wide",
                    lang,
                    &smoke_artifact("restore-browse-path-row", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// A daemon on the real bus name with one destination «NAS» and the backup «Home» with three
/// snapshots of three folders: `bericht.pdf` is in the two older ones under different paths
/// (`t1/src/Dokumente`, `t2/src/Archiv/2026`), the newest has none.
fn search_env(root: &Path) -> (M3Rig, StdFakes) {
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
    let tag = format!("kestic:{}", backup.id);
    let trees = root.join("trees");
    write_file(&trees.join("t1/src/Dokumente"), "bericht.pdf", "first");
    write_file(&trees.join("t2/src/Archiv/2026"), "bericht.pdf", "second");
    write_file(&trees.join("t3/src"), "other.txt", "other");
    for (src, time) in [
        ("t1/src", "2026-11-13 12:00:00"),
        ("t2/src", "2026-11-14 12:00:00"),
        ("t3/src", "2026-11-15 12:00:00"),
    ] {
        backup_dir(
            &variant.bin,
            &repo,
            &password,
            &trees.join(src),
            &RawSnapshot {
                tag: &tag,
                time,
                host: "laptop",
            },
        );
    }
    (rig, fakes)
}

/// AC-M4-04-1 (smoke part): Ctrl+F, «bericht.pdf» and Return list (a) two hits with path and
/// snapshot date, and (b) Return on a hit opens it selected in the folder list of its snapshot.
#[test]
fn ac_m4_04_1_search_hits_open_in_the_folder_list() {
    run_isolated_test(
        "ac_m4_04_1_search_hits_open_in_the_folder_list",
        "KESTIC_AC_M4_04_1_INNER",
        |root| {
            let (rig, _fakes) = search_env(root);
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m4_gui_smoke(
                    root,
                    "m4-search",
                    layout,
                    lang,
                    &smoke_artifact("restore-search", layout, lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M4-04-2 (smoke part): a search without hits shows (a) «No hits for ‹…›» and (b) no error.
#[test]
fn ac_m4_04_2_search_without_hits_shows_no_hits() {
    run_isolated_test(
        "ac_m4_04_2_search_without_hits_shows_no_hits",
        "KESTIC_AC_M4_04_2_INNER",
        |root| {
            let (rig, _fakes) = search_env(root);
            for lang in ["en", "de"] {
                run_m4_gui_smoke(
                    root,
                    "m4-search-none",
                    "wide",
                    lang,
                    &smoke_artifact("restore-search-none", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// M4-S04: «Cancel» on a running search stops `restic find`, and the page says so.
#[test]
fn m4_s04_cancelling_a_search_stops_restic_find() {
    run_isolated_test(
        "m4_s04_cancelling_a_search_stops_restic_find",
        "KESTIC_M4_S04_CANCEL_INNER",
        |root| {
            let (rig, _fakes) = search_env(root);
            rig.wrapper.hold("find");
            run_m4_gui_smoke(
                root,
                "m4-search-cancel",
                "wide",
                "en",
                &smoke_artifact("restore-search-cancelled", "wide", "en"),
            );
            let deadline = Instant::now() + Duration::from_secs(15);
            while rig.wrapper.of('T', "find").is_empty() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            rig.wrapper.release("find");
            assert_eq!(
                rig.wrapper.of('T', "find").len(),
                1,
                "restic find got SIGTERM"
            );
            rig.daemon.stop();
        },
    );
}

/// A daemon on the real bus name with one destination «NAS» and the backup «Home» with three
/// snapshots of one folder: from the first (2026-11-13) to the second (2026-11-14) `docs/a.txt`
/// and three more files change, `docs/b.txt` and two more files are new, `docs/c.txt` and one
/// more file are gone (3 new, 2 removed, 4 changed); the third (2026-11-15) equals the second.
fn compare_env(root: &Path) -> (M3Rig, StdFakes) {
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
    let tag = format!("kestic:{}", backup.id);
    let src = root.join("trees/home");
    let docs = src.join("docs");
    write_file(&docs, "a.txt", "a, first");
    write_file(&docs, "c.txt", "c");
    for i in 0..3 {
        write_file(&src, &format!("changed{i}.txt"), "before");
    }
    write_file(&src, "removed0.txt", "gone soon");
    let snapshot = |time: &str| {
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
    };
    snapshot("2026-11-13 12:00:00");
    write_file(&docs, "a.txt", "a, second version");
    write_file(&docs, "b.txt", "b");
    std::fs::remove_file(docs.join("c.txt")).expect("remove c.txt");
    for i in 0..3 {
        write_file(&src, &format!("changed{i}.txt"), "after, longer");
    }
    std::fs::remove_file(src.join("removed0.txt")).expect("remove removed0.txt");
    for i in 0..2 {
        write_file(&src, &format!("added{i}.txt"), "new");
    }
    snapshot("2026-11-14 12:00:00");
    snapshot("2026-11-15 12:00:00");
    (rig, fakes)
}

/// AC-M4-02-1 and AC-M4-02-4 (smoke part): two snapshots chosen with «Compare…» show (a)
/// `a.txt` as changed, `b.txt` as new and `c.txt` as removed; the chip «New» leaves exactly the
/// 3 added files while the numbers of the other kinds stay.
#[test]
fn ac_m4_02_1_compare_shows_changed_new_and_removed_files() {
    run_isolated_test(
        "ac_m4_02_1_compare_shows_changed_new_and_removed_files",
        "KESTIC_AC_M4_02_1_SMOKE_INNER",
        |root| {
            let (rig, _fakes) = compare_env(root);
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m4_gui_smoke(
                    root,
                    "m4-compare",
                    layout,
                    lang,
                    &smoke_artifact("restore-compare", layout, lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M4-02-3 (smoke part): comparing two identical snapshots shows (a) «No differences»
/// rather than an empty list.
#[test]
fn ac_m4_02_3_identical_snapshots_show_no_differences() {
    run_isolated_test(
        "ac_m4_02_3_identical_snapshots_show_no_differences",
        "KESTIC_AC_M4_02_3_SMOKE_INNER",
        |root| {
            let (rig, _fakes) = compare_env(root);
            for lang in ["en", "de"] {
                run_m4_gui_smoke(
                    root,
                    "m4-compare-identical",
                    "wide",
                    lang,
                    &smoke_artifact("restore-compare-identical", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// A daemon on the real bus name with one destination «NAS» and the backup «Home» with one
/// snapshot of a small folder, whose `a.txt` differs locally afterwards. The wrapper holds
/// `restore`; the smoke scene releases it by removing the returned marker file.
fn restore_dialog_env(root: &Path, variant: &ResticVariant) -> (M3Rig, StdFakes, String) {
    restore_dialog_env_with(root, variant, |_| {})
}

/// [`restore_dialog_env`] with `extra` run on the backed-up folder before the backup.
fn restore_dialog_env_with(
    root: &Path,
    variant: &ResticVariant,
    extra: impl FnOnce(&Path),
) -> (M3Rig, StdFakes, String) {
    let fakes = StdFakes::start_with(FakeSetup {
        active_network: Some("HomeWifi"),
        ..FakeSetup::default()
    });
    let rig = M3Rig::start_on(
        root,
        variant,
        "",
        KESTIC_BUS_NAME,
        datetime!(2026-11-15 14:32:00 UTC),
        |o| {
            o.enable_scheduler = false;
            // The window shows times in the system time zone; the suggested folder name too.
            o.timezone = TimezoneSpec::System;
        },
    );
    let repo = root.join("repo-NAS");
    let (dest, password) =
        add_local_destination(&rig.daemon.client, &variant.bin, &repo, "NAS", |_| {});
    let backup = rig.add_backup_to(&dest, "Home", |_| {});
    let src = small_tree(&root.join("trees"));
    extra(&src);
    backup_dir(
        &variant.bin,
        &repo,
        &password,
        &src,
        &RawSnapshot {
            tag: &format!("kestic:{}", backup.id),
            time: "2026-11-15 12:00:00",
            host: "laptop",
        },
    );
    std::fs::write(src.join("a.txt"), "changed locally").expect("change a.txt");
    let release = format!("{}.restore", rig.wrapper.hold.display());
    (rig, fakes, release)
}

/// AC-M4-05-1, AC-M4-05-4 (smoke part), AC-M4-05-11 and AC-M4-05-15 (c, e): the restore dialog
/// preselects a new folder, names the outcomes and the consequence of overwriting, keeps «Restore
/// Now» disabled until the preview is done, and starts with «Never» every time (scene
/// `m4-restore-dialog` in `smoke_m4.cpp`).
#[test]
fn ac_m4_05_1_4_11_15_restore_dialog() {
    run_isolated_test(
        "ac_m4_05_1_4_11_15_restore_dialog",
        "KESTIC_AC_M4_05_DIALOG_INNER",
        |root| {
            for (layout, lang) in [
                ("wide", "en"),
                ("wide", "de"),
                ("narrow", "en"),
                ("narrow", "de"),
            ] {
                let sub = root.join(format!("{layout}-{lang}"));
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                common::po_translator::set_daemon_language(lang);
                let (rig, _fakes, release) = restore_dialog_env(&sub, &latest_variant());
                rig.wrapper.hold("restore");
                run_m4_gui_smoke_env(
                    root,
                    "m4-restore-dialog",
                    layout,
                    lang,
                    &smoke_artifact("restore-dialog", layout, lang),
                    &[("KESTIC_SMOKE_RELEASE_FILE", &release)],
                );
                rig.daemon.stop();
            }
        },
    );
}

/// The list of files of a preview at the original location with new, overwritten and unchanged
/// files shows a filter chip per outcome, and a focused chip its focus ring (scene
/// `m4-restore-chips`).
#[test]
fn m4_s05_restore_dialog_shows_outcome_chips() {
    run_isolated_test(
        "m4_s05_restore_dialog_shows_outcome_chips",
        "KESTIC_M4_S05_CHIPS_SMOKE_INNER",
        |root| {
            for (layout, lang) in [
                ("wide", "en"),
                ("wide", "de"),
                ("narrow", "en"),
                ("narrow", "de"),
            ] {
                let sub = root.join(format!("{layout}-{lang}"));
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                common::po_translator::set_daemon_language(lang);
                let (rig, _fakes, _) = restore_dialog_env(&sub, &latest_variant());
                let src = sub.join("trees").join("src");
                std::fs::remove_file(src.join("docs").join("x.txt")).expect("remove docs/x.txt");
                run_m4_gui_smoke(
                    root,
                    "m4-restore-chips",
                    layout,
                    lang,
                    &smoke_artifact("restore-chips", layout, lang),
                );
                rig.daemon.stop();
            }
        },
    );
}

/// A selected name that is not valid UTF-8 is refused in the dialog with its reason, and
/// «Restore Folder» restores its folder instead (scene `m4-restore-unmatchable`).
#[test]
fn m4_s05_restore_dialog_offers_the_folder_of_an_unmatchable_name() {
    run_isolated_test(
        "m4_s05_restore_dialog_offers_the_folder_of_an_unmatchable_name",
        "KESTIC_M4_S05_UNMATCHABLE_SMOKE_INNER",
        |root| {
            for lang in ["en", "de"] {
                let sub = root.join(lang);
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                let (rig, _fakes, _) = restore_dialog_env_with(&sub, &latest_variant(), |src| {
                    use std::os::unix::ffi::OsStrExt;
                    let name = std::ffi::OsStr::from_bytes(b"<b>bad<b>\xffname.txt");
                    std::fs::write(src.join(name), "x").expect("write a name that is not UTF-8");
                });
                run_m4_gui_smoke(
                    root,
                    "m4-restore-unmatchable",
                    "wide",
                    lang,
                    &smoke_artifact("restore-unmatchable", "wide", lang),
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-05-16 (f) and AC-M4-05-25 (c), smoke part: the dialog shows the note on the folders
/// that exist at the original location, and the translated refusal of a target where the
/// snapshot has a file and the target a folder with content (scene `m4-restore-target-texts`).
#[test]
fn ac_m4_05_16_25_restore_dialog_shows_folder_note_and_refusal() {
    run_isolated_test(
        "ac_m4_05_16_25_restore_dialog_shows_folder_note_and_refusal",
        "KESTIC_AC_M4_05_16_25_SMOKE_INNER",
        |root| {
            for lang in ["en", "de"] {
                let sub = root.join(lang);
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                common::po_translator::set_daemon_language(lang);
                let (rig, _fakes, _) = restore_dialog_env(&sub, &latest_variant());
                let src = sub.join("trees").join("src");
                std::fs::remove_file(src.join("b.bin")).expect("remove b.bin");
                std::fs::create_dir_all(src.join("b.bin")).expect("folder in its place");
                std::fs::write(src.join("b.bin").join("keep.txt"), "keep").expect("content");
                run_m4_gui_smoke(
                    root,
                    "m4-restore-target-texts",
                    "wide",
                    lang,
                    &smoke_artifact("restore-target-texts", "wide", lang),
                );
                rig.daemon.stop();
            }
        },
    );
}

/// A daemon on the real bus name with one destination «NAS» and the backup «Home» with one
/// snapshot of `/home/<user>/Dokumente/a.txt` (written with `--stdin`, `<user>` as restic records
/// it), and `Dokumente` in the isolated home, so the original location maps to it.
fn restore_mapping_env(root: &Path) -> (M3Rig, StdFakes) {
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
            o.timezone = TimezoneSpec::System;
        },
    );
    let repo = root.join("repo-NAS");
    let (dest, password) =
        add_local_destination(&rig.daemon.client, &variant.bin, &repo, "NAS", |_| {});
    let backup = rig.add_backup_to(&dest, "Home", |_| {});
    // A snapshot without Kestic tag tells the user name restic records.
    let probe = backup_stdin(
        &variant.bin,
        &repo,
        &password,
        "probe.txt",
        "x",
        &RawSnapshot {
            tag: "probe",
            time: "2026-11-15 11:00:00",
            host: "old-laptop",
        },
    );
    let user = snapshot_username(&variant.bin, &repo, &password, &probe);
    backup_stdin(
        &variant.bin,
        &repo,
        &password,
        &format!("home/{user}/Dokumente/a.txt"),
        "hello",
        &RawSnapshot {
            tag: &format!("kestic:{}", backup.id),
            time: "2026-11-15 12:00:00",
            host: "old-laptop",
        },
    );
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME set"));
    std::fs::create_dir_all(home.join("Dokumente")).expect("mkdir Dokumente");
    (rig, fakes)
}

/// AC-M4-06-1 (smoke part): for a snapshot of `/home/<user>/Dokumente/a.txt` the restore dialog
/// (a) offers `Dokumente` in the user's home as the original location and (b) shows the mapping
/// «Original: /home/<user>/Dokumente (<user> → <home>)» (scene `m4-restore-mapping`).
#[test]
fn ac_m4_06_1_restore_dialog_shows_the_mapping() {
    run_isolated_test(
        "ac_m4_06_1_restore_dialog_shows_the_mapping",
        "KESTIC_AC_M4_06_1_SMOKE_INNER",
        |root| {
            for (layout, lang) in [
                ("wide", "en"),
                ("wide", "de"),
                ("narrow", "en"),
                ("narrow", "de"),
            ] {
                let sub = root.join(format!("{layout}-{lang}"));
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                common::po_translator::set_daemon_language(lang);
                let (rig, _fakes) = restore_mapping_env(&sub);
                run_m4_gui_smoke(
                    root,
                    "m4-restore-mapping",
                    layout,
                    lang,
                    &smoke_artifact("restore-mapping", layout, lang),
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-05-22 (b, c): the setting «restic source» marks restic 0.18.0 «(outdated)» and the
/// newest tested version not (scene `m4-restic-outdated`).
#[test]
fn ac_m4_05_22_settings_mark_outdated_restic() {
    run_isolated_test(
        "ac_m4_05_22_settings_mark_outdated_restic",
        "KESTIC_AC_M4_05_22_SMOKE_INNER",
        |root| {
            for variant in common::m3::restic_variants() {
                let outdated = if variant.label.contains("0.18.0") {
                    "1"
                } else {
                    "0"
                };
                for lang in ["en", "de"] {
                    let sub = root.join(format!("{}-{lang}", variant.index));
                    std::fs::create_dir_all(&sub).expect("mkdir scene root");
                    let (rig, _fakes, _) = restore_dialog_env(&sub, &variant);
                    run_m4_gui_smoke_env(
                        root,
                        "m4-restic-outdated",
                        "wide",
                        lang,
                        &smoke_artifact(
                            &format!("settings-restic-{}", variant.index),
                            "wide",
                            lang,
                        ),
                        &[("KESTIC_SMOKE_EXPECT_OUTDATED", outdated)],
                    );
                    rig.daemon.stop();
                }
            }
        },
    );
}

/// Watches the test bus for the GUI's `RegisterGui` call to the daemon; the receiver gets one
/// message once the call passed the bus. The GUI makes that call only after the bus confirmed its
/// match rule for the daemon's signals, so every signal the daemon sends afterwards reaches the
/// GUI. Returns once the bus monitor is active.
fn watch_gui_registration() -> mpsc::Receiver<()> {
    use zbus::export::ordered_stream::OrderedStreamExt;

    let (tx_ready, rx_ready) = mpsc::channel::<()>();
    let (tx_registered, rx_registered) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        zbus::block_on(async move {
            let conn = zbus::Connection::session()
                .await
                .expect("monitor session conn");
            let mut stream = zbus::MessageStream::from(&conn);
            let rule = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::MethodCall)
                .interface(KESTIC_INTERFACE_V1)
                .member("RegisterGui")
                .build()
                .expect("match rule for RegisterGui");
            zbus::fdo::MonitoringProxy::new(&conn)
                .await
                .expect("monitoring proxy")
                .become_monitor(&[rule], 0)
                .await
                .expect("BecomeMonitor");
            tx_ready.send(()).expect("monitor ready");
            while let Some(msg) = stream.next().await {
                let Ok(msg) = msg else {
                    continue;
                };
                if msg.header().member().map(|m| m.as_str()) == Some("RegisterGui") {
                    tx_registered.send(()).expect("report RegisterGui");
                    return;
                }
            }
        });
    });
    rx_ready
        .recv_timeout(Duration::from_secs(10))
        .expect("bus monitor active");
    rx_registered
}

/// Clicks «Restore Files…» in the tray menu of the daemon at `sni` once the GUI is subscribed to
/// the daemon's signals (`registered`, see [`watch_gui_registration`]) and the smoke scene wrote
/// the marker file `marker` (its window shows another tab); returns whether it clicked (false
/// when either did not happen within 60 s).
fn click_restore_files_when_marked(
    sni: &str,
    registered: &mpsc::Receiver<()>,
    marker: &Path,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(60);
    if registered.recv_timeout(Duration::from_secs(60)).is_err() {
        return false;
    }
    while !marker.exists() {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let conn = zbus::block_on(zbus::Connection::session()).expect("session conn");
    let entry = fetch_dbusmenu_items(&conn, sni)
        .into_iter()
        .find(|i| i.label == "Restore Files…" && i.enabled)
        .expect("an enabled «Restore Files…» in the tray menu");
    click_dbusmenu_item(&conn, sni, entry.id);
    true
}

/// AC-M4-05-24 (c) in the GUI: the GUI started with `--show-restore` opens on the tab «Restore»,
/// and a running GUI on another tab switches to it when «Restore Files…» is clicked in the
/// daemon's tray menu (scene `m4-show-restore`).
#[test]
fn ac_m4_05_24_gui_started_from_the_tray_opens_the_restore_tab() {
    run_isolated_test(
        "ac_m4_05_24_gui_started_from_the_tray_opens_the_restore_tab",
        "KESTIC_AC_M4_05_24_SMOKE_INNER",
        |root| {
            let (rig, fakes) = restore_env(root);
            let sni = wait_for_sni_service(&fakes.sni_conn, Duration::from_secs(5));
            let marker = root.join("tray-click-marker");
            let registered = watch_gui_registration();
            let click = {
                let marker = marker.clone();
                std::thread::spawn(move || {
                    click_restore_files_when_marked(&sni, &registered, &marker)
                })
            };
            let iso = kestic_test_support::gui::IsolatedEnv::adopt(root);
            let dbus_addr =
                std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("isolated session bus");
            kestic_test_support::gui::SmokeContext {
                kestic_bin: &find_kestic_binary(),
                iso: &iso,
                dbus_addr: &dbus_addr,
            }
            .run_with_env(
                "m4-show-restore",
                "wide",
                "en",
                &smoke_artifact("show-restore", "wide", "en"),
                &["--show-restore"],
                &[("KESTIC_SMOKE_TRAY_MARKER", &marker.to_string_lossy())],
            );
            assert!(
                click.join().expect("tray click thread"),
                "AC-M4-05-24 (c): «Restore Files…» was clicked in the tray menu"
            );
            rig.daemon.stop();
        },
    );
}

/// A daemon with one destination «NAS», the backup «Home» with one snapshot and, written with
/// the restic CLI, a snapshot without a Kestic tag, one of the unknown backup `x9` (orphaned) and
/// one of the backup `y7` that is not configured here but has a metadata snapshot.
fn foreign_env(root: &Path) -> (M3Rig, StdFakes) {
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
    let home_tag = format!("kestic:{}", backup.id);
    for (tag, time, host) in [
        (home_tag.as_str(), "2026-11-15 12:00:00", "laptop"),
        ("", "2026-11-08 21:14:00", "notebook"),
        ("kestic:x9", "2026-10-02 18:00:00", "notebook-alt"),
        ("kestic:y7", "2026-09-12 09:30:00", "desktop-alt"),
        ("kestic-meta:y7", "2026-09-12 09:31:00", "desktop-alt"),
    ] {
        let work = root.join(format!("work-{}", time.replace([' ', ':'], "-")));
        std::fs::create_dir_all(&work).expect("mkdir work");
        write_snapshot(
            &variant.bin,
            &repo,
            &password,
            &work,
            &RawSnapshot { tag, time, host },
        );
    }
    (rig, fakes)
}

/// AC-M4-07-8: foreign and orphaned snapshots in the list (a) each carry the hint «Not managed
/// by Kestic»; snapshots of a backup not configured here say so instead.
#[test]
fn ac_m4_07_8_foreign_and_orphaned_snapshots_say_not_managed_by_kestic() {
    run_isolated_test(
        "ac_m4_07_8_foreign_and_orphaned_snapshots_say_not_managed_by_kestic",
        "KESTIC_AC_M4_07_8_INNER",
        |root| {
            let (rig, _fakes) = foreign_env(root);
            for (layout, lang) in [
                ("wide", "en"),
                ("wide", "de"),
                ("narrow", "en"),
                ("narrow", "de"),
            ] {
                run_m4_gui_smoke(
                    root,
                    "m4-foreign",
                    layout,
                    lang,
                    &smoke_artifact("restore-unassigned", layout, lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// A daemon with one destination «NAS», the backup «Home» without snapshots and, written with the
/// restic CLI, a snapshot without a Kestic tag of a small folder ([`small_tree`]) whose `a.txt`
/// differs locally afterwards.
fn unassigned_restore_env(root: &Path) -> (M3Rig, StdFakes) {
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
            // The suggested folder name of the restore dialog uses the system time zone.
            o.timezone = TimezoneSpec::System;
        },
    );
    let repo = root.join("repo-NAS");
    let (dest, password) =
        add_local_destination(&rig.daemon.client, &variant.bin, &repo, "NAS", |_| {});
    rig.add_backup_to(&dest, "Home", |_| {});
    let src = small_tree(&root.join("trees"));
    backup_dir(
        &variant.bin,
        &repo,
        &password,
        &src,
        &RawSnapshot {
            tag: "",
            time: "2026-11-08 21:14:00",
            host: "notebook",
        },
    );
    std::fs::write(src.join("a.txt"), "changed locally").expect("change a.txt");
    (rig, fakes)
}

/// AC-M4-07-2 (a), GUI part, for a snapshot without a Kestic tag: browsing it shows the folder
/// list whose path row draws each folder once (AC-M4-03-1 (a)); its restore dialog preselects a
/// new folder, not the original location (AC-M4-05-1); at the original location with «Always» it
/// names the consequence of overwriting and writes nothing before the confirmation
/// (AC-M4-05-4 (c)) (scene `m4-foreign-restore`).
#[test]
fn ac_m4_07_2_unassigned_snapshot_browses_and_restores_like_a_kestic_one() {
    run_isolated_test(
        "ac_m4_07_2_unassigned_snapshot_browses_and_restores_like_a_kestic_one",
        "KESTIC_AC_M4_07_2_SMOKE_INNER",
        |root| {
            for lang in ["en", "de"] {
                let sub = root.join(lang);
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                common::po_translator::set_daemon_language(lang);
                let (rig, _fakes) = unassigned_restore_env(&sub);
                run_m4_gui_smoke(
                    root,
                    "m4-foreign-restore",
                    "wide",
                    lang,
                    &smoke_artifact("restore-unassigned-browse", "wide", lang),
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-01-7: every list is one Tab stop, from the backup and snapshot lists to a folder of
/// 1 000 entries, and no row of any list on the tabs is a Tab stop of its own (scene
/// `m4-list-tab-stop`).
#[test]
fn ac_m4_01_7_every_list_is_one_tab_stop() {
    run_isolated_test(
        "ac_m4_01_7_every_list_is_one_tab_stop",
        "KESTIC_AC_M4_01_7_INNER",
        |root| {
            let (rig, _fakes) = browse_env(root, QueryLimits::default());
            for lang in ["en", "de"] {
                run_m4_gui_smoke(
                    root,
                    "m4-list-tab-stop",
                    "wide",
                    lang,
                    &smoke_artifact("list-tab-stop", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M4-01-4 (b): the Tab order follows the visible order: navigation, the content, the action
/// bar at the bottom of the page, then the navigation again (scene `m4-tab-order`, «Restore»
/// and «Backups»).
#[test]
fn ac_m4_01_4_b_tab_order_follows_the_visible_order() {
    run_isolated_test(
        "ac_m4_01_4_b_tab_order_follows_the_visible_order",
        "KESTIC_AC_M4_01_4_B_INNER",
        |root| {
            let (rig, _fakes) = restore_env(root);
            for lang in ["en", "de"] {
                run_m4_gui_smoke(
                    root,
                    "m4-tab-order",
                    "wide",
                    lang,
                    &smoke_artifact("tab-order", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M4-01-7 (b): in a snapshot list with more rows than fit, the current row stays in view
/// after Down to the last row, and Page Down moves by about one visible page (scene
/// `m4-list-page-keys`, 30 snapshots).
#[test]
fn ac_m4_01_7_b_snapshot_list_keeps_the_current_row_in_view() {
    run_isolated_test(
        "ac_m4_01_7_b_snapshot_list_keeps_the_current_row_in_view",
        "KESTIC_AC_M4_01_7_B_INNER",
        |root| {
            let times: Vec<String> = (1..=30)
                .map(|day| format!("2026-10-{day:02} 12:00:00"))
                .collect();
            let snapshots: Vec<(&str, &str)> =
                times.iter().map(|t| (t.as_str(), "laptop")).collect();
            let (rig, _fakes) = restore_env_with(root, &snapshots);
            for lang in ["en", "de"] {
                run_m4_gui_smoke(
                    root,
                    "m4-list-page-keys",
                    "wide",
                    lang,
                    &smoke_artifact("list-page-keys", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M4-01-8: a right click opens the context menu of a snapshot on the press, at the
/// pointer, also after a long press or a move, on every row every time (scene
/// `m4-snapshot-right-click`).
#[test]
fn ac_m4_01_8_right_click_opens_the_snapshot_menu_every_time() {
    run_isolated_test(
        "ac_m4_01_8_right_click_opens_the_snapshot_menu_every_time",
        "KESTIC_AC_M4_01_8_INNER",
        |root| {
            let (rig, _fakes) = restore_env(root);
            for lang in ["en", "de"] {
                run_m4_gui_smoke(
                    root,
                    "m4-snapshot-right-click",
                    "wide",
                    lang,
                    &smoke_artifact("snapshot-context-menu", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M4-01-9: the tab bar and the columns switch at the same width, and a column at that width
/// shows short names of the folder list in full (scene `m4-breakpoint`).
#[test]
fn ac_m4_01_9_tab_bar_and_columns_switch_at_one_width() {
    run_isolated_test(
        "ac_m4_01_9_tab_bar_and_columns_switch_at_one_width",
        "KESTIC_AC_M4_01_9_INNER",
        |root| {
            let (rig, _fakes) = browse_env(root, QueryLimits::default());
            for lang in ["en", "de"] {
                run_m4_gui_smoke(
                    root,
                    "m4-breakpoint",
                    "wide",
                    lang,
                    &smoke_artifact("restore-browse-breakpoint", "wide", lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M4-03-8: the header check box and Ctrl+A select every entry of the folder shown, also
/// those not loaded yet, and with a filter the matching ones (scene `m4-browse-select-all`).
#[test]
fn ac_m4_03_8_select_all_entries_of_the_folder_shown() {
    run_isolated_test(
        "ac_m4_03_8_select_all_entries_of_the_folder_shown",
        "KESTIC_AC_M4_03_8_INNER",
        |root| {
            let (rig, _fakes) = browse_env(root, QueryLimits::default());
            for (layout, lang) in [("wide", "en"), ("wide", "de"), ("narrow", "en")] {
                run_m4_gui_smoke(
                    root,
                    "m4-browse-select-all",
                    layout,
                    lang,
                    &smoke_artifact("restore-browse-select-all", layout, lang),
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M4-05-26: with every entry of the top folder selected, «Restore All…» restores the whole
/// snapshot: one `restic restore` of `<snapshot>:<top folder>` without `--include`, and the new
/// folder holds every file of the snapshot (scene `m4-restore-all`).
#[test]
fn ac_m4_05_26_restore_all_restores_the_whole_snapshot() {
    run_isolated_test(
        "ac_m4_05_26_restore_all_restores_the_whole_snapshot",
        "KESTIC_AC_M4_05_26_INNER",
        |root| {
            for lang in ["en", "de"] {
                let sub = root.join(lang);
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                common::po_translator::set_daemon_language(lang);
                let (rig, _fakes, _release) = restore_dialog_env(&sub, &latest_variant());
                let src = sub.join("trees/src");
                run_m4_gui_smoke(
                    root,
                    "m4-restore-all",
                    "wide",
                    lang,
                    &smoke_artifact("restore-all", "wide", lang),
                );
                let state = rig.daemon.client.get_state().expect("GetState");
                let restores: Vec<_> = state
                    .recent_runs
                    .iter()
                    .filter(|r| matches!(r.kind, kestic_core::model::RunKind::Restore { .. }))
                    .collect();
                assert_eq!(restores.len(), 1, "(a) {lang}: one restore run");
                let args = &restores[0].commands.last().expect("restore command").args;
                assert!(
                    args.iter()
                        .any(|a| a.ends_with(&format!(":{}", src.display()))),
                    "(a) {lang}: restores the top folder of the snapshot: {args:?}"
                );
                assert!(
                    !args.iter().any(|a| a.starts_with("--include")),
                    "(a) {lang}: the whole snapshot, without --include: {args:?}"
                );
                let target = args
                    .iter()
                    .position(|a| a == "--target")
                    .and_then(|i| args.get(i + 1))
                    .map(std::path::PathBuf::from)
                    .expect("--target");
                for file in ["a.txt", "b.bin", "docs/x.txt", "docs/deep/y.txt", "empty"] {
                    assert!(
                        target.join(file).exists(),
                        "(a) {lang}: {file} restored into {}",
                        target.display()
                    );
                }
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-05-26 (a), browser opened from a search hit: a snapshot of one file is restored from
/// `/`, and every entry of `/` selected reads «Restore All…» with an empty selection, opened
/// from the hit of the file (scene `m4-restore-all-from-search`) and from a folder hit above it
/// (scene `m4-restore-all-from-folder-hit`).
#[test]
fn ac_m4_05_26_restore_all_from_a_search_hit() {
    run_isolated_test(
        "ac_m4_05_26_restore_all_from_a_search_hit",
        "KESTIC_AC_M4_05_26_SEARCH_INNER",
        |root| {
            for lang in ["en", "de"] {
                let sub = root.join(lang);
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                common::po_translator::set_daemon_language(lang);
                let (rig, _fakes) = restore_mapping_env(&sub);
                run_m4_gui_smoke(
                    root,
                    "m4-restore-all-from-search",
                    "wide",
                    lang,
                    &smoke_artifact("restore-all-from-search", "wide", lang),
                );
                run_m4_gui_smoke(
                    root,
                    "m4-restore-all-from-folder-hit",
                    "wide",
                    lang,
                    &smoke_artifact("restore-all-from-folder-hit", "wide", lang),
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-05-27: the Tab order of the restore dialog, forward and backward, stays in the dialog,
/// and Esc returns the focus to the folder list (scene `m4-restore-dialog-tab`).
#[test]
fn ac_m4_05_27_restore_dialog_tab_order() {
    run_isolated_test(
        "ac_m4_05_27_restore_dialog_tab_order",
        "KESTIC_AC_M4_05_27_INNER",
        |root| {
            for lang in ["en", "de"] {
                let sub = root.join(lang);
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                common::po_translator::set_daemon_language(lang);
                let (rig, _fakes, _release) = restore_dialog_env(&sub, &latest_variant());
                run_m4_gui_smoke(
                    root,
                    "m4-restore-dialog-tab",
                    "wide",
                    lang,
                    &smoke_artifact("restore-dialog-tab", "wide", lang),
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-05-29: the restore banner shows a running restore with an icon of the theme, a
/// cancelled one as neutral information, and again the next restore after it was closed;
/// AC-M4-05-30 (d): the running message names the speed (scene `m4-restore-banner`).
#[test]
fn ac_m4_05_29_restore_banner_shows_running_and_cancelled() {
    run_isolated_test(
        "ac_m4_05_29_restore_banner_shows_running_and_cancelled",
        "KESTIC_AC_M4_05_29_INNER",
        |root| {
            for lang in ["en", "de"] {
                let sub = root.join(lang);
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                common::po_translator::set_daemon_language(lang);
                let (rig, _fakes, _release) = restore_dialog_env(&sub, &latest_variant());
                const MIB: u64 = 1024 * 1024;
                let line = |bytes: u64| {
                    format!(
                        "{{\"message_type\":\"status\",\"percent_done\":0.25,\
                         \"files_restored\":1,\"bytes_restored\":{bytes},\"total_bytes\":{}}}\n",
                        40 * MIB
                    )
                };
                rig.wrapper.hold_with_repeated_output("restore", &line(MIB));
                std::thread::scope(|scope| {
                    // AC-M4-05-30 (d): once restic runs, 4 MiB more after 1 s on the daemon's clock.
                    // The daemon reads a 1 MiB line first; the 5 MiB line then comes at the same
                    // moment, which changes no speed, and the clock moves once it has been read.
                    scope.spawn(|| {
                        let deadline = Instant::now() + Duration::from_secs(90);
                        let wait_for_bytes = |bytes: u64| {
                            while Instant::now() < deadline {
                                let read = rig
                                    .daemon
                                    .client
                                    .get_state()
                                    .ok()
                                    .and_then(|state| state.overall.active_run_progress)
                                    .is_some_and(|p| p.bytes_done == bytes);
                                if read {
                                    return;
                                }
                                std::thread::sleep(Duration::from_millis(100));
                            }
                        };
                        wait_for_bytes(MIB);
                        std::fs::write(
                            format!("{}.restore.repeat", rig.wrapper.hold.display()),
                            line(5 * MIB),
                        )
                        .expect("write the next status line");
                        wait_for_bytes(5 * MIB);
                        rig.clock.advance(Duration::from_secs(1));
                    });
                    run_m4_gui_smoke(
                        root,
                        "m4-restore-banner",
                        "wide",
                        lang,
                        &smoke_artifact("restore-banner", "wide", lang),
                    );
                });
                rig.wrapper.release("restore");
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-05-28: a network folder from the folder chooser keeps the target and says that only
/// local folders can be chosen (scene `m4-restore-network-folder`).
#[test]
fn ac_m4_05_28_network_folder_is_no_restore_target() {
    run_isolated_test(
        "ac_m4_05_28_network_folder_is_no_restore_target",
        "KESTIC_AC_M4_05_28_INNER",
        |root| {
            for lang in ["en", "de"] {
                let sub = root.join(lang);
                std::fs::create_dir_all(&sub).expect("mkdir scene root");
                common::po_translator::set_daemon_language(lang);
                let (rig, _fakes, _release) = restore_dialog_env(&sub, &latest_variant());
                run_m4_gui_smoke(
                    root,
                    "m4-restore-network-folder",
                    "wide",
                    lang,
                    &smoke_artifact("restore-network-folder", "wide", lang),
                );
                rig.daemon.stop();
            }
        },
    );
}
