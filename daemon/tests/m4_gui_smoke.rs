// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Offscreen GUI smoke tests for Milestone M4 (restore area) against a daemon with a real
//! restic temp repository; screenshots in English and German under `target/smoke-artifacts/`.

mod common;

use common::m3::{
    FakeSetup, M3Rig, ResticVariant, StdFakes, add_local_destination, latest_variant,
};
use common::m4::{RawSnapshot, backup_dir, big_tree, small_tree, write_file, write_snapshot};
use common::{find_kestic_binary, run_isolated_test};
use kestic_core::query_list::QueryLimits;
use kestic_core::scheduler::TimezoneSpec;
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

/// AC-M4-05-24 (c) in the GUI: the GUI started with `--show-restore` opens on the tab «Restore»
/// (scene `m4-show-restore`).
#[test]
fn ac_m4_05_24_gui_started_from_the_tray_opens_the_restore_tab() {
    run_isolated_test(
        "ac_m4_05_24_gui_started_from_the_tray_opens_the_restore_tab",
        "KESTIC_AC_M4_05_24_SMOKE_INNER",
        |root| {
            let (rig, _fakes) = restore_env(root);
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
                &[],
            );
            rig.daemon.stop();
        },
    );
}
