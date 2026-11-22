// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S02 differences between two snapshots against real restic (0.18.0 and latest) with temp
//! repos: `StartSnapshotQuery` (kind `diff`) and `GetSnapshotQueryStatus` over D-Bus.

mod common;

use common::m3::{M3Rig, ResticVariant, add_local_destination, latest_variant, restic_variants};
use common::m4::{RawSnapshot, backup_dir, write_file};
use common::run_isolated_test;
use kestic_core::model::Destination;
use kestic_daemon::dbus_api::{
    DiffResultDto, KesticDbusError, PageFilter, SnapshotChange, SnapshotQueryPage,
    SnapshotQueryRequest, SnapshotQueryResultDto, SnapshotQueryStatusDto,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use time::macros::datetime;

/// Two snapshots of one folder: the first and the one after `change` modified it.
struct Pair {
    from: String,
    to: String,
    /// The backed-up folder.
    src: PathBuf,
}

/// A daemon behind the restic wrapper with one local destination and the backup «Home» (`b1`)
/// with three pairs of snapshots, each of its own folder:
/// - `docs`: `docs/a.txt` and `docs/c.txt`, then `a.txt` changed, `b.txt` new, `c.txt` gone;
/// - `same`: the same folder backed up twice without a change;
/// - `many`: then 3 files added, 2 removed and 4 changed.
struct Fixture {
    rig: M3Rig,
    dest: Destination,
    password: String,
    repo: PathBuf,
    docs: Pair,
    same: Pair,
    many: Pair,
}

impl Fixture {
    fn start(root: &Path, variant: &ResticVariant) -> Self {
        let rig = M3Rig::start(
            root,
            variant,
            "M4S02",
            datetime!(2026-11-15 14:32:00 UTC),
            |o| o.enable_scheduler = false,
        );
        let work = root.join(&rig.sub);
        let repo = work.join("repo");
        let (dest, password) =
            add_local_destination(&rig.daemon.client, &variant.bin, &repo, "NAS", |_| {});
        let b1 = rig.add_backup_to(&dest, "Home", |_| {}).id.to_string();
        let tag = format!("kestic:{b1}");
        let mut hour = 0;
        let mut pair = |name: &str, setup: &dyn Fn(&Path), change: &dyn Fn(&Path)| {
            let src = work.join(name).join("src");
            fs::create_dir_all(&src).expect("mkdir source");
            setup(&src);
            let mut snap = |src: &Path| {
                hour += 1;
                backup_dir(
                    &variant.bin,
                    &repo,
                    &password,
                    src,
                    &RawSnapshot {
                        tag: &tag,
                        time: &format!("2026-11-14 {hour:02}:00:00"),
                        host: "laptop",
                    },
                )
            };
            let from = snap(&src);
            change(&src);
            let to = snap(&src);
            Pair { from, to, src }
        };

        let docs = pair(
            "docs",
            &|src| {
                write_file(&src.join("docs"), "a.txt", "a, first");
                write_file(&src.join("docs"), "c.txt", "c");
            },
            &|src| {
                write_file(&src.join("docs"), "a.txt", "a, second version");
                write_file(&src.join("docs"), "b.txt", "b");
                fs::remove_file(src.join("docs/c.txt")).expect("remove c.txt");
            },
        );
        let same = pair(
            "same",
            &|src| write_file(&src.join("docs"), "a.txt", "unchanged"),
            &|_| {},
        );
        let many = pair(
            "many",
            &|src| {
                for i in 0..4 {
                    write_file(src, &format!("changed{i}.txt"), "before");
                }
                for i in 0..2 {
                    write_file(src, &format!("removed{i}.txt"), "gone soon");
                }
            },
            &|src| {
                for i in 0..4 {
                    write_file(src, &format!("changed{i}.txt"), "after, longer");
                }
                for i in 0..2 {
                    fs::remove_file(src.join(format!("removed{i}.txt"))).expect("remove");
                }
                for i in 0..3 {
                    write_file(src, &format!("added{i}.txt"), "new");
                }
            },
        );
        Self {
            rig,
            dest,
            password,
            repo,
            docs,
            same,
            many,
        }
    }

    fn start_diff(&self, from: &str, to: &str) -> Result<String, KesticDbusError> {
        self.rig
            .daemon
            .client
            .start_snapshot_query(&SnapshotQueryRequest::diff(self.dest.id.as_str(), from, to))
    }

    fn status(
        &self,
        query_id: &str,
        page: &SnapshotQueryPage,
    ) -> Result<SnapshotQueryStatusDto, KesticDbusError> {
        self.rig
            .daemon
            .client
            .get_snapshot_query_status(query_id, page)
    }

    /// Waits until query `query_id` has ended and returns its last status for `page`.
    fn wait(&self, query_id: &str, page: &SnapshotQueryPage) -> SnapshotQueryStatusDto {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let st = self.status(query_id, page).expect("GetSnapshotQueryStatus");
            if st.state != "running" {
                return st;
            }
            assert!(Instant::now() < deadline, "query still running");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Compares `pair` (from → to) and returns the finished result for `page`, with the
    /// query ID for further pages.
    fn diff(&self, pair: &Pair, page: &SnapshotQueryPage) -> (String, DiffResultDto) {
        let id = self
            .start_diff(&pair.from, &pair.to)
            .expect("StartSnapshotQuery");
        let st = self.wait(&id, page);
        assert_eq!(st.state, "done", "diff ended with {st:?}");
        assert_eq!(st.error_kind, None, "diff without error: {st:?}");
        match st.result {
            Some(SnapshotQueryResultDto::Diff(diff)) => (id, diff),
            other => panic!("diff result expected: {other:?}"),
        }
    }

    /// What restic itself reports for `restic diff --json from to`: the changes (path and
    /// modifier) and the statistics line.
    fn restic_diff(&self, pair: &Pair) -> (Vec<(String, String)>, serde_json::Value) {
        let out = Command::new(&self.rig.variant.bin)
            .arg("-r")
            .arg(&self.repo)
            .args(["diff", "--no-cache", "--json", &pair.from, &pair.to])
            .env("RESTIC_PASSWORD", &self.password)
            .output()
            .expect("run restic diff");
        assert!(out.status.success(), "restic diff");
        let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        let changes = lines
            .iter()
            .filter(|v| v["message_type"] == "change")
            .map(|v| {
                (
                    v["path"].as_str().unwrap_or_default().to_string(),
                    v["modifier"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        let stats = lines
            .into_iter()
            .find(|v| v["message_type"] == "statistics")
            .expect("statistics line of restic diff");
        (changes, stats)
    }
}

fn all_page() -> SnapshotQueryPage {
    SnapshotQueryPage {
        limit: 1000,
        ..SnapshotQueryPage::default()
    }
}

fn kinds_page(kinds: &[&str]) -> SnapshotQueryPage {
    SnapshotQueryPage {
        limit: 1000,
        filter: PageFilter {
            text: String::new(),
            kinds: kinds.iter().map(|k| (*k).to_string()).collect(),
        },
        ..SnapshotQueryPage::default()
    }
}

/// The modifier `restic diff` prints for `change`.
fn modifier(change: SnapshotChange) -> &'static str {
    match change {
        SnapshotChange::Added => "+",
        SnapshotChange::Removed => "-",
        SnapshotChange::Modified => "M",
        SnapshotChange::Metadata => "U",
        SnapshotChange::TypeChanged => "T",
        SnapshotChange::Bitrot => "?",
    }
}

fn for_each_variant(root: &Path, body: impl Fn(&Fixture, &str)) {
    for variant in restic_variants() {
        let fixture = Fixture::start(root, &variant);
        body(&fixture, variant.label);
        fixture.rig.daemon.stop();
    }
}

/// AC-M4-02-1: snapshot A has `docs/a.txt` (and `docs/c.txt`); in B `a.txt` is changed,
/// `b.txt` new and `c.txt` removed. Comparing A → B shows (a) `a.txt` as modified, `b.txt` as
/// added and `c.txt` as removed, and (b) the same changes and counts as `restic diff --json A B`.
#[test]
fn ac_m4_02_1_changed_new_and_removed_files_match_restic_diff() {
    run_isolated_test(
        "ac_m4_02_1_changed_new_and_removed_files_match_restic_diff",
        "KESTIC_AC_M4_02_1_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                let (_, diff) = fx.diff(&fx.docs, &all_page());
                let docs = fx.docs.src.join("docs");
                let path = |name: &str| docs.join(name).to_string_lossy().into_owned();
                let changes: Vec<(String, SnapshotChange)> = diff
                    .changes
                    .items
                    .iter()
                    .map(|c| (c.path.clone(), c.change))
                    .collect();
                assert_eq!(
                    changes,
                    [
                        (path("a.txt"), SnapshotChange::Modified),
                        (path("b.txt"), SnapshotChange::Added),
                        (path("c.txt"), SnapshotChange::Removed),
                    ],
                    "(a) {label}: a.txt modified, b.txt added, c.txt removed"
                );
                assert!(!diff.identical, "(a) {label}: the snapshots differ");

                // Kestic lists by path; restic prints the tree order.
                let (mut restic_changes, stats) = fx.restic_diff(&fx.docs);
                restic_changes.sort();
                let ours: Vec<(String, String)> = diff
                    .changes
                    .items
                    .iter()
                    .map(|c| (c.path.clone(), modifier(c.change).to_string()))
                    .collect();
                assert_eq!(ours, restic_changes, "(b) {label}: the changes of restic");
                assert_eq!(
                    diff.changes.total,
                    restic_changes.len() as u64,
                    "(b) {label}: as many changes as restic reports"
                );
                assert_eq!(
                    Some(diff.added_files),
                    stats["added"]["files"].as_u64(),
                    "(b) {label}: added files of restic"
                );
                assert_eq!(
                    Some(diff.removed_files),
                    stats["removed"]["files"].as_u64(),
                    "(b) {label}: removed files of restic"
                );
                assert_eq!(
                    Some(diff.changed_files),
                    stats["changed_files"].as_u64(),
                    "(b) {label}: changed files of restic"
                );
                assert_eq!(
                    Some(diff.bytes_added),
                    stats["added"]["bytes"].as_u64(),
                    "(b) {label}: added bytes of restic"
                );
                assert_eq!(
                    Some(diff.bytes_removed),
                    stats["removed"]["bytes"].as_u64(),
                    "(b) {label}: removed bytes of restic"
                );

                let starts = fx.rig.wrapper.of('S', "diff");
                let last = starts.last().expect("restic diff ran");
                assert!(
                    last.detail.contains("--no-lock"),
                    "{label}: restic diff runs without lock: {}",
                    last.detail
                );
                assert!(
                    last.detail
                        .ends_with(&format!("{} {}", fx.docs.from, fx.docs.to)),
                    "{label}: restic diff compares A with B: {}",
                    last.detail
                );
            });
        },
    );
}

/// AC-M4-02-3: two snapshots with the same content give (a) «no differences» (`identical`)
/// rather than just an empty list.
#[test]
fn ac_m4_02_3_identical_snapshots_give_no_differences() {
    run_isolated_test(
        "ac_m4_02_3_identical_snapshots_give_no_differences",
        "KESTIC_AC_M4_02_3_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                assert_ne!(fx.same.from, fx.same.to, "{label}: two different snapshots");
                let (_, diff) = fx.diff(&fx.same, &all_page());
                assert!(diff.identical, "(a) {label}: «no differences»");
                assert_eq!(diff.changes.total, 0, "(a) {label}: no change listed");
                let (_, changed) = fx.diff(&fx.docs, &all_page());
                assert!(
                    !changed.identical,
                    "(a) {label}: snapshots with changes are not identical"
                );
            });
        },
    );
}

/// AC-M4-02-4: A → B with 3 added, 2 removed and 4 changed files, filtered by the kind
/// «added»: (a) the list holds exactly the 3 added files, (b) `matched` is 3, and (c) the
/// counts `added_files`, `removed_files` and `changed_files` stay 3, 2 and 4.
#[test]
fn ac_m4_02_4_filter_by_kind_keeps_the_counts() {
    run_isolated_test(
        "ac_m4_02_4_filter_by_kind_keeps_the_counts",
        "KESTIC_AC_M4_02_4_INNER",
        |root| {
            for_each_variant(root, |fx, label| {
                let (_, diff) = fx.diff(&fx.many, &kinds_page(&["added"]));
                let src = &fx.many.src;
                let added: Vec<String> = (0..3)
                    .map(|i| {
                        src.join(format!("added{i}.txt"))
                            .to_string_lossy()
                            .into_owned()
                    })
                    .collect();
                let listed: Vec<String> =
                    diff.changes.items.iter().map(|c| c.path.clone()).collect();
                assert_eq!(listed, added, "(a) {label}: exactly the 3 added files");
                assert!(
                    diff.changes
                        .items
                        .iter()
                        .all(|c| c.change == SnapshotChange::Added),
                    "(a) {label}: only added files"
                );
                assert_eq!(diff.changes.matched, 3, "(b) {label}: matched is 3");
                assert_eq!(diff.changes.total, 9, "{label}: 9 changes in total");
                assert_eq!(diff.added_files, 3, "(c) {label}: added_files stays 3");
                assert_eq!(diff.removed_files, 2, "(c) {label}: removed_files stays 2");
                assert_eq!(diff.changed_files, 4, "(c) {label}: changed_files stays 4");
            });
        },
    );
}

/// M4-S02: the kinds filter takes the codes of the changes only; a list is filtered and paged
/// in the daemon, and invalid snapshot IDs are refused without running restic.
#[test]
fn m4_s02_diff_requests_and_pages_are_checked() {
    run_isolated_test(
        "m4_s02_diff_requests_and_pages_are_checked",
        "KESTIC_M4_S02_CHECKS_INNER",
        |root| {
            let variant = latest_variant();
            let fx = Fixture::start(root, &variant);
            let (id, _) = fx.diff(&fx.many, &all_page());
            let removed_and_changed = fx
                .status(&id, &kinds_page(&["removed", "modified"]))
                .expect("two kinds");
            match removed_and_changed.result {
                Some(SnapshotQueryResultDto::Diff(diff)) => {
                    assert_eq!(diff.changes.matched, 6, "removed or modified: 2 + 4");
                }
                other => panic!("diff result expected: {other:?}"),
            }
            let paged = fx
                .status(
                    &id,
                    &SnapshotQueryPage {
                        offset: 2,
                        limit: 3,
                        filter: PageFilter {
                            text: "CHANGED".to_string(),
                            kinds: Vec::new(),
                        },
                    },
                )
                .expect("page with text filter");
            match paged.result {
                Some(SnapshotQueryResultDto::Diff(diff)) => {
                    assert_eq!(diff.changes.matched, 4, "text filter, any case");
                    assert_eq!(diff.changes.items.len(), 2, "the rest after offset 2");
                }
                other => panic!("diff result expected: {other:?}"),
            }
            let unknown = fx.status(&id, &kinds_page(&["new"]));
            assert!(
                matches!(unknown, Err(KesticDbusError::InvalidInput(_))),
                "an unknown kind is refused: {unknown:?}"
            );

            let diffs_before = fx.rig.wrapper.of('S', "diff").len();
            let short = fx.start_diff(&fx.many.from[..8], &fx.many.to);
            assert!(
                matches!(short, Err(KesticDbusError::InvalidInput(_))),
                "a short ID is refused: {short:?}"
            );
            let flag = fx.start_diff(&fx.many.from, &format!("-{}", &fx.many.to[1..]));
            assert!(
                matches!(flag, Err(KesticDbusError::InvalidInput(_))),
                "an ID that looks like a flag is refused: {flag:?}"
            );
            assert_eq!(
                fx.rig.wrapper.of('S', "diff").len(),
                diffs_before,
                "no restic diff for a refused request"
            );
            fx.rig.daemon.stop();
        },
    );
}

/// M4-S02: cancelling a running comparison stops `restic diff`; the query ends as `cancelled`.
#[test]
fn m4_s02_cancelling_a_diff_stops_restic_diff() {
    run_isolated_test(
        "m4_s02_cancelling_a_diff_stops_restic_diff",
        "KESTIC_M4_S02_CANCEL_INNER",
        |root| {
            let variant = latest_variant();
            let fx = Fixture::start(root, &variant);
            fx.rig.wrapper.hold("diff");
            let id = fx
                .start_diff(&fx.docs.from, &fx.docs.to)
                .expect("StartSnapshotQuery");
            let deadline = Instant::now() + Duration::from_secs(30);
            while fx.rig.wrapper.of('S', "diff").is_empty() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            assert_eq!(
                fx.rig.wrapper.of('S', "diff").len(),
                1,
                "restic diff started"
            );
            fx.rig
                .daemon
                .client
                .cancel_snapshot_query(&id)
                .expect("CancelSnapshotQuery");
            let st = fx.wait(&id, &all_page());
            fx.rig.wrapper.release("diff");
            assert_eq!(st.state, "cancelled", "the comparison ends as cancelled");
            assert!(st.result.is_none(), "a cancelled comparison has no result");
            assert_eq!(
                fx.rig.wrapper.of('T', "diff").len(),
                1,
                "restic diff got SIGTERM"
            );
            fx.rig.daemon.stop();
        },
    );
}
