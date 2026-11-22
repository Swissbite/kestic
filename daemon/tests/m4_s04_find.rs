// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S04 searching the snapshots of a backup against real restic (0.18.0 and latest) with temp
//! repos: `StartSnapshotQuery` (kind `find`) and `GetSnapshotQueryStatus` over D-Bus.

mod common;

use common::m3::{M3Rig, ResticVariant, add_local_destination, restic_variants};
use common::m4::{RawSnapshot, backup_dir, restic_find_paths, write_file};
use common::run_isolated_test;
use kestic_core::model::Destination;
use kestic_core::snapshot_ls::path_segments;
use kestic_daemon::dbus_api::{
    FindResultDto, FindScope, LsResultDto, SnapshotQueryPage, SnapshotQueryRequest,
    SnapshotQueryResultDto, SnapshotQueryStatusDto,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use time::macros::datetime;

/// A daemon behind the restic wrapper with one local destination and two backups:
/// - «Home» (`b1`): three snapshots of three folders; `bericht.pdf` is in the first two, under
///   different paths, the newest has none;
/// - two metadata snapshots `kestic-meta:<b1>` of a folder with `bericht.pdf`;
/// - «Photos» (`b2`): one snapshot of a folder with `bericht.pdf`.
struct Fixture {
    rig: M3Rig,
    dest: Destination,
    password: String,
    repo: PathBuf,
    b1: String,
    b2: String,
    /// The `b1` snapshots with `bericht.pdf`, oldest first, with its path there.
    b1_hits: Vec<(String, String)>,
    meta_snapshots: Vec<String>,
    b2_snapshot: String,
}

impl Fixture {
    fn start(root: &Path, variant: &ResticVariant) -> Self {
        let rig = M3Rig::start(
            root,
            variant,
            "M4S04",
            datetime!(2026-11-15 14:32:00 UTC),
            |o| o.enable_scheduler = false,
        );
        let work = root.join(&rig.sub);
        let repo = work.join("repo");
        let (dest, password) =
            add_local_destination(&rig.daemon.client, &variant.bin, &repo, "NAS", |_| {});
        let b1 = rig.add_backup_to(&dest, "Home", |_| {}).id.to_string();
        let b2 = rig.add_backup_to(&dest, "Photos", |_| {}).id.to_string();
        let backup = |src: &Path, tag: &str, time: &str| {
            backup_dir(
                &variant.bin,
                &repo,
                &password,
                src,
                &RawSnapshot {
                    tag,
                    time,
                    host: "laptop",
                },
            )
        };

        let tag1 = format!("kestic:{b1}");
        let first = work.join("t1/src");
        write_file(&first.join("Dokumente"), "bericht.pdf", "first");
        write_file(&first, "notes.txt", "n");
        let second = work.join("t2/src");
        write_file(&second.join("Archiv/2026"), "bericht.pdf", "second");
        let third = work.join("t3/src");
        write_file(&third, "other.txt", "o");
        let mut b1_hits = Vec::new();
        for (src, time, rel) in [
            (&first, "2026-11-13 12:00:00", Some("Dokumente/bericht.pdf")),
            (
                &second,
                "2026-11-14 12:00:00",
                Some("Archiv/2026/bericht.pdf"),
            ),
            (&third, "2026-11-15 12:00:00", None),
        ] {
            let id = backup(src, &tag1, time);
            if let Some(rel) = rel {
                b1_hits.push((id, format!("{}/{rel}", src.display())));
            }
        }

        let meta_src = work.join("meta/src");
        write_file(&meta_src, "bericht.pdf", "meta");
        let meta_tag = format!("kestic-meta:{b1}");
        let meta_snapshots = ["2026-11-14 18:00:00", "2026-11-15 18:00:00"]
            .iter()
            .map(|time| backup(&meta_src, &meta_tag, time))
            .collect();

        let b2_src = work.join("photos/src");
        write_file(&b2_src, "bericht.pdf", "photos");
        let b2_snapshot = backup(&b2_src, &format!("kestic:{b2}"), "2026-11-15 13:00:00");

        Self {
            rig,
            dest,
            password,
            repo,
            b1,
            b2,
            b1_hits,
            meta_snapshots,
            b2_snapshot,
        }
    }

    fn start_query(&self, request: &SnapshotQueryRequest) -> String {
        self.rig
            .daemon
            .client
            .start_snapshot_query(request)
            .expect("StartSnapshotQuery")
    }

    /// Waits until query `query_id` has ended and returns its last status.
    fn wait(&self, query_id: &str) -> SnapshotQueryStatusDto {
        let page = SnapshotQueryPage {
            limit: 1000,
            ..SnapshotQueryPage::default()
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let st = self
                .rig
                .daemon
                .client
                .get_snapshot_query_status(query_id, &page)
                .expect("GetSnapshotQueryStatus");
            if st.state != "running" {
                return st;
            }
            assert!(Instant::now() < deadline, "query still running");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Searches the snapshots of backup `backup_id` for `text` and returns the finished result.
    fn search(&self, backup_id: &str, text: &str) -> FindResultDto {
        let scope = FindScope::Backup {
            backup_id: backup_id.to_string(),
        };
        let id = self.start_query(&SnapshotQueryRequest::find(
            self.dest.id.as_str(),
            scope,
            text,
        ));
        let st = self.wait(&id);
        assert_eq!(st.state, "done", "search ended with {st:?}");
        assert_eq!(st.error_kind, None, "search without error: {st:?}");
        match st.result {
            Some(SnapshotQueryResultDto::Find(found)) => found,
            other => panic!("find result expected: {other:?}"),
        }
    }

    /// Lists `dir` of snapshot `snapshot_id`.
    fn list(&self, snapshot_id: &str, dir: &str) -> LsResultDto {
        let id = self.start_query(&SnapshotQueryRequest::ls(
            self.dest.id.as_str(),
            snapshot_id,
            dir,
        ));
        let st = self.wait(&id);
        assert_eq!(st.state, "done", "ls ended with {st:?}");
        match st.result {
            Some(SnapshotQueryResultDto::Ls(ls)) => ls,
            other => panic!("ls result expected: {other:?}"),
        }
    }

    /// What restic itself finds for `bericht.pdf` in snapshot `snapshot_id`.
    fn restic_finds_bericht(&self, snapshot_id: &str) -> Vec<String> {
        restic_find_paths(
            &self.rig.variant.bin,
            &self.repo,
            &self.password,
            snapshot_id,
            "bericht.pdf",
        )
    }
}

fn for_each_variant(root: &Path, body: impl Fn(&Fixture, &str)) {
    for variant in restic_variants() {
        let fixture = Fixture::start(root, &variant);
        body(&fixture, variant.label);
        fixture.rig.daemon.stop();
    }
}

/// AC-M4-04-1: `bericht.pdf` is in two of the three snapshots of `b1` under different paths;
/// searching for it gives (a) exactly two hits, each with its snapshot and path (newest
/// snapshot first), and (b) a hit opens directly in the folder list: its folder lists it, and
/// the breadcrumb leads from the top down to that folder.
#[test]
fn ac_m4_04_1_two_hits_with_snapshot_and_path_open_in_the_folder_list() {
    run_isolated_test(
        "ac_m4_04_1_two_hits_with_snapshot_and_path_open_in_the_folder_list",
        "KESTIC_AC_M4_04_1_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                let found = fx.search(&fx.b1, "bericht.pdf");
                let hits: Vec<(String, String)> = found
                    .hits
                    .items
                    .iter()
                    .map(|h| (h.snapshot_id.clone(), h.path.clone()))
                    .collect();
                let expected: Vec<(String, String)> = fx.b1_hits.iter().rev().cloned().collect();
                assert_eq!(found.hits.total, 2, "(a) {label}: exactly two hits");
                assert_eq!(hits, expected, "(a) {label}: snapshot and path of each hit");
                assert_ne!(
                    hits[0].1, hits[1].1,
                    "(a) {label}: the hits are under different paths"
                );
                assert_eq!(
                    found.hits.items[0].snapshot_time.get(..10),
                    Some("2026-11-14"),
                    "(a) {label}: the hit carries the time of its snapshot"
                );
                assert_eq!(found.snapshots_matched, 2, "(a) {label}: two snapshots");

                for hit in &found.hits.items {
                    let folder = hit.folder();
                    let ls = fx.list(&hit.snapshot_id, folder);
                    assert!(ls.dir_exists, "(b) {label}: the folder of the hit exists");
                    assert!(
                        ls.entries.items.iter().any(|e| e.path == hit.path),
                        "(b) {label}: the folder list shows the hit {}",
                        hit.path
                    );
                    let crumbs = path_segments(folder);
                    assert_eq!(
                        crumbs.first().map(|c| c.1.as_str()),
                        Some("/"),
                        "(b) {label}: the breadcrumb starts at the top"
                    );
                    assert_eq!(
                        crumbs.last().map(|c| c.1.as_str()),
                        Some(folder),
                        "(b) {label}: the breadcrumb ends at the folder of the hit"
                    );
                }
            });
        },
    );
}

/// AC-M4-04-2: a search text without hits gives (a) «no hits» (`total` 0, no hit listed), and
/// (b) no error: the search ends as `done` after `restic find` ran.
#[test]
fn ac_m4_04_2_search_without_hits_ends_without_error() {
    run_isolated_test(
        "ac_m4_04_2_search_without_hits_ends_without_error",
        "KESTIC_AC_M4_04_2_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                let found = fx.search(&fx.b1, "nirgends-zu-finden");
                assert_eq!(found.hits.total, 0, "(a) {label}: no hits");
                assert!(found.hits.items.is_empty(), "(a) {label}: nothing listed");
                assert_eq!(
                    found.snapshots_matched, 0,
                    "(a) {label}: no snapshot matched"
                );
                let ends = fx.rig.wrapper.of('E', "find");
                assert_eq!(ends.len(), 1, "(b) {label}: restic find ran once");
                assert_eq!(
                    ends[0].detail, "0",
                    "(b) {label}: restic find ended without error"
                );
            });
        },
    );
}

/// AC-M4-04-3: the metadata snapshots `kestic-meta:<b1>` hold `bericht.pdf` too, yet (a) they
/// give no hits when the snapshots of `b1` are searched.
#[test]
fn ac_m4_04_3_metadata_snapshots_give_no_hits() {
    run_isolated_test(
        "ac_m4_04_3_metadata_snapshots_give_no_hits",
        "KESTIC_AC_M4_04_3_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                for meta in &fx.meta_snapshots {
                    assert_eq!(
                        fx.restic_finds_bericht(meta).len(),
                        1,
                        "{label}: the metadata snapshot holds bericht.pdf"
                    );
                }
                let found = fx.search(&fx.b1, "bericht.pdf");
                assert!(
                    found
                        .hits
                        .items
                        .iter()
                        .all(|h| !fx.meta_snapshots.contains(&h.snapshot_id)),
                    "(a) {label}: no hit from a metadata snapshot: {:?}",
                    found.hits.items
                );
                assert_eq!(found.hits.total, 2, "(a) {label}: only the two backup hits");
            });
        },
    );
}

/// AC-M4-04-4: `bericht.pdf` is in snapshots of `b1` and `b2` of the same destination;
/// searching in `b1` gives (a) only hits from snapshots tagged `kestic:<b1>`.
#[test]
fn ac_m4_04_4_search_in_b1_gives_only_hits_of_b1() {
    run_isolated_test(
        "ac_m4_04_4_search_in_b1_gives_only_hits_of_b1",
        "KESTIC_AC_M4_04_4_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                assert_eq!(
                    fx.restic_finds_bericht(&fx.b2_snapshot).len(),
                    1,
                    "{label}: the b2 snapshot holds bericht.pdf"
                );
                let found = fx.search(&fx.b1, "bericht.pdf");
                let b1_ids: Vec<&String> = fx.b1_hits.iter().map(|(id, _)| id).collect();
                assert!(
                    found
                        .hits
                        .items
                        .iter()
                        .all(|h| b1_ids.contains(&&h.snapshot_id)),
                    "(a) {label}: only snapshots of b1: {:?}",
                    found.hits.items
                );
                assert!(
                    found
                        .hits
                        .items
                        .iter()
                        .all(|h| h.backup_id.as_deref() == Some(fx.b1.as_str())),
                    "(a) {label}: every hit belongs to b1"
                );
                assert_eq!(found.hits.total, 2, "(a) {label}: the b2 hit is left out");
                let in_b2 = fx.search(&fx.b2, "bericht.pdf");
                assert_eq!(
                    in_b2
                        .hits
                        .items
                        .iter()
                        .map(|h| &h.snapshot_id)
                        .collect::<Vec<_>>(),
                    [&fx.b2_snapshot],
                    "{label}: searching b2 finds its own snapshot"
                );
            });
        },
    );
}

/// M4-S04: a text that starts with `/` starts a path segment, so a pasted absolute path and a
/// folder at the top of the snapshot are found.
#[test]
fn m4_s04_absolute_path_text_finds_the_hit() {
    run_isolated_test(
        "m4_s04_absolute_path_text_finds_the_hit",
        "KESTIC_M4_S04_ABSOLUTE_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                let (snapshot, path) = &fx.b1_hits[0];
                let found = fx.search(&fx.b1, path);
                let hits: Vec<(&String, &String)> = found
                    .hits
                    .items
                    .iter()
                    .map(|h| (&h.snapshot_id, &h.path))
                    .collect();
                assert_eq!(
                    hits,
                    [(snapshot, path)],
                    "{label}: the absolute path is found"
                );

                let top = format!(
                    "/{}",
                    path.trim_start_matches('/')
                        .split('/')
                        .next()
                        .unwrap_or_default()
                );
                let found = fx.search(&fx.b1, &top);
                assert!(
                    found.hits.items.iter().any(|h| &h.path == path),
                    "{label}: {top} finds the entries below the top folder"
                );
            });
        },
    );
}

/// M4-S04: the text is taken literally; restic would clean `Dokumente/../Archiv` to `Archiv`,
/// but no snapshot path contains that text, so the search finds nothing.
#[test]
fn m4_s04_text_with_parent_segment_finds_nothing() {
    run_isolated_test(
        "m4_s04_text_with_parent_segment_finds_nothing",
        "KESTIC_M4_S04_PARENT_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                assert!(
                    fx.search(&fx.b1, "Archiv").hits.total > 0,
                    "{label}: «Archiv» alone has hits"
                );
                let found = fx.search(&fx.b1, "Dokumente/../Archiv");
                assert_eq!(found.hits.total, 0, "{label}: no path contains the text");
            });
        },
    );
}
