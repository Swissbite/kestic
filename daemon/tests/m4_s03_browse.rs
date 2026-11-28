// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S03 browsing a snapshot against real restic (0.18.0 and latest) with temp repos:
//! `StartSnapshotQuery` (kind `ls`), `GetSnapshotQueryStatus` and its pages over D-Bus.

mod common;

use common::m3::{M3Rig, ResticVariant, add_local_destination};
use common::m4::{
    BIG_TREE_MATCHES, BIG_TREE_OTHER_CASE, RawSnapshot, VariantFixture, backup_dir, big_page,
    big_tree, for_each_variant_with, restic_ls_nodes, small_tree, wait_query,
};
use common::run_isolated_test;
use kestic_core::model::Destination;
use kestic_daemon::dbus_api::{
    LsEntryDto, LsResultDto, PageFilter, SnapshotQueryPage, SnapshotQueryRequest,
    SnapshotQueryResultDto, SnapshotQueryStatusDto,
};
use kestic_test_support::wait::{try_poll_every, wait_until_within};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use time::macros::datetime;

/// A daemon behind the restic wrapper with one local destination holding one snapshot of a
/// folder.
struct Fixture {
    rig: M3Rig,
    dest: Destination,
    password: String,
    repo: PathBuf,
    /// The backed-up folder; also its path in the snapshot.
    src: PathBuf,
    snapshot_id: String,
}

impl Fixture {
    fn start(root: &Path, variant: &ResticVariant, make_tree: fn(&Path) -> PathBuf) -> Self {
        let rig = M3Rig::start(
            root,
            variant,
            "M4S03",
            datetime!(2026-11-15 14:32:00 UTC),
            |o| o.enable_scheduler = false,
        );
        let work = root.join(&rig.sub);
        let repo = work.join("repo");
        let (dest, password) =
            add_local_destination(&rig.daemon.client, &variant.bin, &repo, "NAS", |_| {});
        let backup = rig.add_backup_to(&dest, "Home", |_| {});
        let src = make_tree(&work);
        let tag = format!("kestic:{}", backup.id);
        let snapshot_id = backup_dir(
            &variant.bin,
            &repo,
            &password,
            &src,
            &RawSnapshot {
                tag: &tag,
                time: "2026-11-15 12:00:00",
                host: "laptop",
            },
        );
        Self {
            rig,
            dest,
            password,
            repo,
            src,
            snapshot_id,
        }
    }

    /// A fixture whose snapshot holds the folder of [`small_tree`].
    fn start_small(root: &Path, variant: &ResticVariant) -> Self {
        Self::start(root, variant, small_tree)
    }

    /// A fixture whose snapshot holds the folder of [`big_tree`].
    fn start_big(root: &Path, variant: &ResticVariant) -> Self {
        Self::start(root, variant, big_tree)
    }

    fn src_dir(&self) -> String {
        self.src.to_string_lossy().into_owned()
    }

    fn start_ls(&self, dir: &str) -> String {
        self.rig
            .daemon
            .client
            .start_snapshot_query(&SnapshotQueryRequest::ls(
                self.dest.id.as_str(),
                &self.snapshot_id,
                dir,
            ))
            .expect("StartSnapshotQuery")
    }

    fn status(&self, query_id: &str, page: &SnapshotQueryPage) -> SnapshotQueryStatusDto {
        self.rig
            .daemon
            .client
            .get_snapshot_query_status(query_id, page)
            .expect("GetSnapshotQueryStatus")
    }

    /// Waits until query `query_id` is done and returns the `ls` result of `page`.
    fn wait_ls(&self, query_id: &str, page: &SnapshotQueryPage) -> LsResultDto {
        let st = wait_query(&self.rig.daemon.client, query_id, page);
        assert_eq!(st.state, "done", "query ended with {st:?}");
        let Some(SnapshotQueryResultDto::Ls(ls)) = st.result else {
            panic!("ls result expected: {st:?}");
        };
        ls
    }

    /// Lists `dir` and returns its first page of up to 1000 entries.
    fn list(&self, dir: &str) -> LsResultDto {
        let id = self.start_ls(dir);
        self.wait_ls(&id, &big_page())
    }

    /// Arguments of every `restic ls` call so far, split at spaces.
    fn ls_calls(&self) -> Vec<Vec<String>> {
        self.rig
            .wrapper
            .of('S', "ls")
            .into_iter()
            .map(|e| e.detail.split(' ').map(str::to_string).collect())
            .collect()
    }
}

impl VariantFixture for Fixture {
    fn stop(self) {
        self.rig.daemon.stop();
    }
}

/// Name, type, size and modification time of an entry as `restic ls --json` prints them.
fn restic_view(node: &serde_json::Value) -> (String, String, Option<u64>, Option<String>) {
    (
        node["name"].as_str().unwrap_or_default().to_string(),
        node["type"].as_str().unwrap_or_default().to_string(),
        node["size"].as_u64(),
        node["mtime"].as_str().map(str::to_string),
    )
}

fn kestic_view(e: &LsEntryDto) -> (String, String, Option<u64>, Option<String>) {
    (
        e.name.clone(),
        e.entry_type.clone(),
        e.size,
        e.mtime.clone(),
    )
}

/// AC-M4-03-1: browsing a snapshot of a folder with sub-folders shows (a) the folders and files
/// with name, size and modification time as `restic ls --json` reports them, on every level,
/// and (b) never uses `restic mount`.
#[test]
fn ac_m4_03_1_folder_list_matches_restic_ls_without_mount() {
    run_isolated_test(
        "ac_m4_03_1_folder_list_matches_restic_ls_without_mount",
        "KESTIC_AC_M4_03_1_INNER",
        |root| {
            for_each_variant_with(root, Fixture::start_small, |fx, v| {
                let label = v.label;
                let top = fx.src_dir();
                for dir in [
                    top.clone(),
                    format!("{top}/docs"),
                    format!("{top}/docs/deep"),
                ] {
                    let ls = fx.list(&dir);
                    assert!(ls.dir_exists, "{label}: {dir} exists");
                    let mut ours: Vec<_> = ls.entries.items.iter().map(kestic_view).collect();
                    let mut theirs: Vec<_> = restic_ls_nodes(
                        &fx.rig.variant.bin,
                        &fx.repo,
                        &fx.password,
                        &fx.snapshot_id,
                        &dir,
                    )
                    .iter()
                    .map(restic_view)
                    .collect();
                    ours.sort();
                    theirs.sort();
                    assert!(!theirs.is_empty(), "{label}: {dir} has entries");
                    assert_eq!(ours, theirs, "(a) {label}: {dir} as restic ls --json");
                    assert_eq!(ls.entries.total, theirs.len() as u64);
                }
                let top_names: Vec<String> = fx
                    .list(&top)
                    .entries
                    .items
                    .iter()
                    .map(|e| format!("{}:{}", e.name, e.entry_type))
                    .collect();
                assert_eq!(
                    top_names,
                    ["a.txt:file", "b.bin:file", "docs:dir", "empty:dir"],
                    "(a) {label}: folders and files of the top folder, by path"
                );
                assert!(
                    fx.rig.wrapper.of('S', "mount").is_empty(),
                    "(b) {label}: restic mount is never used"
                );
            });
        },
    );
}

/// AC-M4-03-2: opening the top folder of a snapshot with 100 000 files in 1 000 folders
/// (a) calls `restic ls` with the snapshot ID and exactly one folder argument, without
/// recursion, and (b) loads only the 1 000 entries of that level.
#[test]
fn ac_m4_03_2_top_folder_lists_one_level_with_one_folder_argument() {
    run_isolated_test(
        "ac_m4_03_2_top_folder_lists_one_level_with_one_folder_argument",
        "KESTIC_AC_M4_03_2_INNER",
        |root| {
            for_each_variant_with(root, Fixture::start_big, |fx, v| {
                let label = v.label;
                let top = fx.src_dir();
                let ls = fx.list(&top);
                let calls = fx.ls_calls();
                assert_eq!(calls.len(), 1, "(a) {label}: one restic ls call");
                let args = &calls[0];
                let after_ls: Vec<&str> = args
                    .iter()
                    .skip_while(|a| *a != "ls")
                    .skip(1)
                    .map(String::as_str)
                    .collect();
                let positional: Vec<&str> = after_ls
                    .iter()
                    .copied()
                    .filter(|a| !a.starts_with('-'))
                    .collect();
                assert_eq!(
                    positional,
                    [fx.snapshot_id.as_str(), top.as_str()],
                    "(a) {label}: snapshot ID and exactly one folder: {args:?}"
                );
                assert!(
                    !args.iter().any(|a| a == "--recursive" || a == "-R"),
                    "(a) {label}: no recursion: {args:?}"
                );
                assert_eq!(
                    ls.entries.total, 1000,
                    "(b) {label}: one level, not 100 000"
                );
                assert_eq!(ls.entries.items.len(), 1000, "(b) {label}: entries loaded");
                assert!(
                    ls.entries.items.iter().all(|e| e.is_dir()),
                    "(b) {label}: the level holds the 1 000 folders"
                );
            });
        },
    );
}

/// AC-M4-03-5 (integration part): the 1 000 entries of the top folder come in pages of at most
/// 200 with a rising `offset`; (b) each entry is reached exactly once, and (c) the total of
/// 1 000 is known with the first page.
#[test]
fn ac_m4_03_5_pages_of_200_reach_every_entry_exactly_once() {
    run_isolated_test(
        "ac_m4_03_5_pages_of_200_reach_every_entry_exactly_once",
        "KESTIC_AC_M4_03_5_INNER",
        |root| {
            for_each_variant_with(root, Fixture::start_big, |fx, v| {
                let label = v.label;
                let id = fx.start_ls(&fx.src_dir());
                let first = fx.wait_ls(&id, &SnapshotQueryPage::default());
                assert_eq!(
                    first.entries.total, 1000,
                    "(c) {label}: total with the first page"
                );
                assert_eq!(
                    first.entries.items.len(),
                    200,
                    "(a) {label}: default page of 200"
                );
                let mut seen = HashSet::new();
                let mut offset = 0u64;
                let mut pages = 0;
                while offset < first.entries.total {
                    let page = SnapshotQueryPage {
                        offset,
                        limit: 200,
                        filter: PageFilter::default(),
                    };
                    let ls = fx.wait_ls(&id, &page);
                    assert_eq!(ls.entries.offset, offset, "(a) {label}: offset echoed");
                    assert!(ls.entries.items.len() <= 200, "(a) {label}: at most 200");
                    for e in ls.entries.items {
                        assert!(seen.insert(e.path), "(b) {label}: entry twice");
                    }
                    offset += 200;
                    pages += 1;
                }
                assert_eq!(pages, 5, "(a) {label}: five pages");
                assert_eq!(seen.len(), 1000, "(b) {label}: every entry reached");
                assert_eq!(
                    fx.ls_calls().len(),
                    1,
                    "{label}: pages come from the held list"
                );
            });
        },
    );
}

/// AC-M4-03-6: filtering the top folder by a text found in 37 names (5 in another case)
/// returns (a) exactly these 37 entries, with (b) `matched` 37 and (c) `total` still 1 000.
#[test]
fn ac_m4_03_6_filter_finds_37_names_in_any_case() {
    run_isolated_test(
        "ac_m4_03_6_filter_finds_37_names_in_any_case",
        "KESTIC_AC_M4_03_6_INNER",
        |root| {
            for_each_variant_with(root, Fixture::start_big, |fx, v| {
                let label = v.label;
                let id = fx.start_ls(&fx.src_dir());
                let page = SnapshotQueryPage {
                    offset: 0,
                    limit: 1000,
                    filter: PageFilter {
                        text: "bericht".to_string(),
                        kinds: Vec::new(),
                    },
                };
                let ls = fx.wait_ls(&id, &page);
                let names: Vec<&str> = ls.entries.items.iter().map(|e| e.name.as_str()).collect();
                assert_eq!(names.len(), BIG_TREE_MATCHES, "(a) {label}: {names:?}");
                assert!(
                    names.iter().all(|n| n.to_lowercase().contains("bericht")),
                    "(a) {label}: only matching names"
                );
                assert_eq!(
                    names.iter().filter(|n| n.contains("BeRiCHT")).count(),
                    BIG_TREE_OTHER_CASE,
                    "(a) {label}: names in another case included"
                );
                assert_eq!(ls.entries.matched, 37, "(b) {label}: matched");
                assert_eq!(ls.entries.total, 1000, "(c) {label}: total unchanged");
            });
        },
    );
}

/// M4-S03: a folder the GUI leaves while it still loads is cancelled: `CancelSnapshotQuery`
/// stops `restic ls` and the query ends as `cancelled`.
#[test]
fn m4_s03_cancel_snapshot_query_stops_restic_ls() {
    run_isolated_test(
        "m4_s03_cancel_snapshot_query_stops_restic_ls",
        "KESTIC_M4_S03_CANCEL_INNER",
        |root| {
            for_each_variant_with(root, Fixture::start_small, |fx, v| {
                let label = v.label;
                fx.rig.wrapper.hold("ls");
                let id = fx.start_ls(&fx.src_dir());
                wait_until_within(
                    Duration::from_secs(30),
                    &format!("{label}: restic ls started"),
                    || !fx.rig.wrapper.of('S', "ls").is_empty(),
                );
                fx.rig
                    .daemon
                    .client
                    .cancel_snapshot_query(&id)
                    .expect("CancelSnapshotQuery");
                // Polled until the query leaves «running», then checked below with the hold
                // released.
                let state =
                    try_poll_every(Duration::from_secs(30), Duration::from_millis(50), || {
                        let st = fx.status(&id, &SnapshotQueryPage::default());
                        (st.state != "running").then_some(st.state)
                    })
                    .unwrap_or_else(|| "running".to_string());
                fx.rig.wrapper.release("ls");
                assert_eq!(state, "cancelled", "{label}: query cancelled");
                assert_eq!(
                    fx.rig.wrapper.of('T', "ls").len(),
                    1,
                    "{label}: restic ls got SIGTERM"
                );
            });
        },
    );
}
