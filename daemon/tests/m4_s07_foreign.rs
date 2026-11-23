// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S07 foreign and orphaned snapshots against real restic (0.18.0 and latest) with temp
//! repositories: their place in `ListDestinationSnapshots`, browsing, comparing, searching and
//! restoring them, and the restic commands Kestic runs on them.

mod common;

use common::m3::{ResticVariant, restic_variants};
use common::m4::{RawSnapshot, backup_dir, restic_ls_nodes, small_tree};
use common::m4_restore::{RestoreFixture, big_page};
use common::run_isolated_test;
use kestic_core::model::{RunKind, RunStatus};
use kestic_core::snapshot_find::FindScope;
use kestic_daemon::dbus_api::{
    DestinationSnapshotsDto, RestoreOutcome, RestoreOverwrite, RestoreTarget, SnapshotQueryRequest,
    SnapshotQueryResultDto, SnapshotQueryStatusDto,
};
use kestic_i18n::codes::ServiceErrorCode;
use std::fs;
use std::path::Path;

/// The restic subcommands Kestic may run on a foreign or orphaned snapshot.
const READ_ONLY_COMMANDS: [&str; 5] = ["snapshots", "ls", "find", "diff", "restore"];

/// Starts a fixture per restic version whose one snapshot of `small_tree` carries `tag` (empty:
/// none) instead of the tag of the configured backup «Home».
fn for_each_variant(
    root: &Path,
    base: &str,
    tag: &str,
    body: impl Fn(&RestoreFixture, &ResticVariant),
) {
    for variant in restic_variants() {
        let fx = RestoreFixture::start_tagged(
            root,
            &variant,
            base,
            small_tree,
            |_| {},
            |_| tag.to_string(),
        );
        body(&fx, &variant);
        fx.rig.daemon.stop();
    }
}

fn list_destination(fx: &RestoreFixture) -> DestinationSnapshotsDto {
    let dto = fx
        .rig
        .daemon
        .client
        .list_destination_snapshots(fx.dest.id.as_str())
        .expect("ListDestinationSnapshots");
    assert!(dto.error_kind.is_none(), "destination listed: {dto:?}");
    dto
}

fn unassigned_ids(dto: &DestinationSnapshotsDto) -> Vec<String> {
    dto.unassigned.iter().map(|s| s.id.clone()).collect()
}

fn orphaned_ids(dto: &DestinationSnapshotsDto, backup_id: &str) -> Vec<String> {
    dto.orphaned
        .iter()
        .filter(|o| o.backup_id == backup_id)
        .flat_map(|o| o.snapshots.iter().map(|s| s.id.clone()))
        .collect()
}

fn described_ids(dto: &DestinationSnapshotsDto, backup_id: &str) -> Vec<String> {
    dto.described
        .iter()
        .filter(|d| d.backup_id == backup_id)
        .flat_map(|d| d.snapshots.iter().map(|s| s.id.clone()))
        .collect()
}

/// Writes a further snapshot of a small folder into the fixture's repository with `tag`.
fn extra_snapshot(fx: &RestoreFixture, tag: &str, time: &str) -> String {
    let src = fx
        .rig
        .root
        .join(&fx.rig.sub)
        .join(format!("extra-{}", time.replace([' ', ':'], "-")));
    fs::create_dir_all(&src).expect("mkdir extra source");
    fs::write(src.join("note.txt"), format!("{tag} {time}")).expect("write extra source");
    backup_dir(
        &fx.rig.variant.bin,
        &fx.repo,
        &fx.password,
        &src,
        &RawSnapshot {
            tag,
            time,
            host: "old-laptop",
        },
    )
}

/// Runs the query `request` to its end and returns its first page of up to 1000 entries.
fn run_query(fx: &RestoreFixture, request: &SnapshotQueryRequest) -> SnapshotQueryStatusDto {
    let id = fx
        .rig
        .daemon
        .client
        .start_snapshot_query(request)
        .expect("StartSnapshotQuery");
    let status = fx.wait_query(&id, &big_page());
    assert_eq!(status.state, "done", "query ended with {status:?}");
    status
}

/// Name, type, size and modification time of every entry of the backed-up folder, as Kestic
/// lists them and as `restic ls --json` prints them.
type EntryView = (String, String, Option<u64>, Option<String>);

fn listed_by_kestic(fx: &RestoreFixture) -> Vec<EntryView> {
    let request = SnapshotQueryRequest::ls(fx.dest.id.as_str(), &fx.snapshot_id, &fx.path(""));
    let Some(SnapshotQueryResultDto::Ls(ls)) = run_query(fx, &request).result else {
        panic!("ls result expected");
    };
    let mut entries: Vec<EntryView> = ls
        .entries
        .items
        .iter()
        .map(|e| {
            (
                e.name.clone(),
                e.entry_type.clone(),
                e.size,
                e.mtime.clone(),
            )
        })
        .collect();
    entries.sort();
    entries
}

fn listed_by_restic(fx: &RestoreFixture) -> Vec<EntryView> {
    let mut entries: Vec<EntryView> = restic_ls_nodes(
        &fx.rig.variant.bin,
        &fx.repo,
        &fx.password,
        &fx.snapshot_id,
        &fx.path(""),
    )
    .iter()
    .map(|n| {
        (
            n["name"].as_str().unwrap_or_default().to_string(),
            n["type"].as_str().unwrap_or_default().to_string(),
            n["size"].as_u64(),
            n["mtime"].as_str().map(str::to_string),
        )
    })
    .collect();
    entries.sort();
    entries
}

/// Restores `a.txt` of the fixture's snapshot into a new folder and returns the run's detail.
fn restore_into_new_folder(
    fx: &RestoreFixture,
    name: &str,
) -> (std::path::PathBuf, kestic_daemon::dbus_api::RunDetailDto) {
    let target = fx.new_dir(name);
    let run_id = fx
        .start_restore(
            &["a.txt"],
            RestoreTarget::NewFolder {
                dir: target.to_string_lossy().into_owned(),
            },
            RestoreOverwrite::Never,
            None,
        )
        .expect("StartRestore");
    let detail = fx.wait_run(&run_id);
    (target, detail)
}

/// AC-M4-07-1: a snapshot made with the restic CLI without a Kestic tag (a) is listed under
/// «unassigned» and (b) in no backup list.
#[test]
fn ac_m4_07_1_snapshot_without_kestic_tag_is_unassigned() {
    run_isolated_test(
        "ac_m4_07_1_snapshot_without_kestic_tag_is_unassigned",
        "KESTIC_AC_M4_07_1_INNER",
        |root| {
            for_each_variant(root, "M4F1", "", |fx, v| {
                let dto = list_destination(fx);
                assert_eq!(
                    unassigned_ids(&dto),
                    std::slice::from_ref(&fx.snapshot_id),
                    "(a) {}: unassigned",
                    v.label
                );
                assert_eq!(
                    dto.backups.iter().map(|b| b.count).sum::<u64>(),
                    0,
                    "(b) {}: counted for no backup: {:?}",
                    v.label,
                    dto.backups
                );
                let home = fx
                    .rig
                    .daemon
                    .client
                    .list_backup_snapshots(fx.backup.id.as_str())
                    .expect("ListBackupSnapshots");
                assert!(
                    home.current_snapshots.is_empty(),
                    "(b) {}: not in the list of «Home»: {:?}",
                    v.label,
                    home.current_snapshots
                );
            });
        },
    );
}

/// AC-M4-07-2: a foreign snapshot (a) is browsed and restored under the rules of AC-M4-03-1 and
/// AC-M4-05-1 to -05-7, and (b) its restore is a run of kind `Restore` of the destination
/// without a backup.
#[test]
fn ac_m4_07_2_foreign_snapshot_is_browsed_and_restored_like_any_other() {
    run_isolated_test(
        "ac_m4_07_2_foreign_snapshot_is_browsed_and_restored_like_any_other",
        "KESTIC_AC_M4_07_2_INNER",
        |root| {
            for_each_variant(root, "M4F2", "", |fx, v| {
                let label = v.label;
                // AC-M4-03-1: the folder list as `restic ls --json`, without `restic mount`.
                let theirs = listed_by_restic(fx);
                assert!(!theirs.is_empty(), "{label}: the folder has entries");
                assert_eq!(listed_by_kestic(fx), theirs, "(a) AC-M4-03-1 {label}");
                assert!(
                    fx.rig.wrapper.of('S', "mount").is_empty(),
                    "(a) AC-M4-03-1 {label}: no restic mount"
                );

                // AC-M4-05-1: the default target is a new folder, not the original location.
                let (_, suggested) = fx.preview(&["a.txt"], None, RestoreOverwrite::Never);
                assert!(
                    matches!(suggested.target.target, RestoreTarget::NewFolder { .. }),
                    "(a) AC-M4-05-1 {label}: new folder preselected: {:?}",
                    suggested.target.target
                );

                // AC-M4-05-2: into a new, empty folder.
                let (target, detail) = restore_into_new_folder(fx, "new");
                assert_eq!(
                    fs::read_to_string(target.join("a.txt")).expect("a.txt restored"),
                    "hello",
                    "(a) AC-M4-05-2 {label}: identical content"
                );
                assert_eq!(
                    detail.run.status,
                    RunStatus::Ok,
                    "(a) AC-M4-05-2 {label}: {detail:?}"
                );

                // AC-M4-05-4: the original location needs a dry run first.
                fs::write(fx.src.join("a.txt"), "changed locally").expect("change a.txt");
                let refused = fx
                    .start_restore(&["a.txt"], fx.original(), RestoreOverwrite::Never, None)
                    .expect_err("the original location needs a preview");
                assert_eq!(
                    refused.code(),
                    ServiceErrorCode::PreviewRequired,
                    "(a) AC-M4-05-4 {label}"
                );
                let (preview_id, preview) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Never);
                assert!(
                    fx.restore_calls().iter().any(|c| c.contains("--dry-run")),
                    "(a) AC-M4-05-4 {label}: restore --dry-run ran: {:?}",
                    fx.restore_calls()
                );

                // AC-M4-05-7: a preview without confirmation changes nothing.
                assert_eq!(
                    fs::read_to_string(fx.src.join("a.txt")).expect("a.txt"),
                    "changed locally",
                    "(a) AC-M4-05-7 {label}: unchanged without confirmation"
                );

                // AC-M4-05-3: `never` keeps the differing file.
                assert_eq!(
                    preview
                        .entries
                        .items
                        .iter()
                        .find(|e| e.path.ends_with("/a.txt"))
                        .map(|e| e.outcome),
                    Some(RestoreOutcome::Kept),
                    "(a) AC-M4-05-3 {label}: «not overwritten»"
                );
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        fx.original(),
                        RestoreOverwrite::Never,
                        Some(&preview_id),
                    )
                    .expect("StartRestore with preview");
                assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
                assert_eq!(
                    fs::read_to_string(fx.src.join("a.txt")).expect("a.txt"),
                    "changed locally",
                    "(a) AC-M4-05-3 {label}: kept with never"
                );

                // AC-M4-05-5: `always` writes the snapshot's file as restic does.
                let (always_id, _) =
                    fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Always);
                let run_id = fx
                    .start_restore(
                        &["a.txt"],
                        fx.original(),
                        RestoreOverwrite::Always,
                        Some(&always_id),
                    )
                    .expect("StartRestore always");
                assert_eq!(fx.wait_run(&run_id).run.status, RunStatus::Ok);
                assert_eq!(
                    fs::read_to_string(fx.src.join("a.txt")).expect("a.txt"),
                    "hello",
                    "(a) AC-M4-05-5 {label}: overwritten with always"
                );

                // AC-M4-05-6: every restore call names the mode, never --sparse or --verify.
                for call in fx.restore_calls() {
                    assert!(
                        call.contains("--overwrite"),
                        "(a) AC-M4-05-6 {label}: {call}"
                    );
                    assert!(
                        !call.contains("--sparse") && !call.contains("--verify"),
                        "(a) AC-M4-05-6 {label}: {call}"
                    );
                }

                // (b) The run belongs to the destination, without a backup.
                assert!(
                    matches!(detail.run.kind, RunKind::Restore { .. }),
                    "(b) {label}: kind Restore: {:?}",
                    detail.run.kind
                );
                assert_eq!(
                    detail.run.destination_id, fx.dest.id,
                    "(b) {label}: run of the destination"
                );
                assert_eq!(
                    detail.run.backup_id(),
                    None,
                    "(b) {label}: without a backup"
                );
            });
        },
    );
}

/// AC-M4-07-3: metadata snapshots `kestic-meta:b1` and snapshots with `kestic:b1` are (a) never
/// listed under «unassigned».
#[test]
fn ac_m4_07_3_metadata_and_backup_snapshots_are_not_unassigned() {
    run_isolated_test(
        "ac_m4_07_3_metadata_and_backup_snapshots_are_not_unassigned",
        "KESTIC_AC_M4_07_3_INNER",
        |root| {
            for variant in restic_variants() {
                let fx = RestoreFixture::start(root, &variant, "M4F3", small_tree, |_| {});
                let meta_tag = format!("kestic-meta:{}", fx.backup.id);
                let meta = extra_snapshot(&fx, &meta_tag, "2026-11-15 12:05:00");
                let foreign = extra_snapshot(&fx, "", "2026-11-14 12:00:00");
                let dto = list_destination(&fx);
                let unassigned = unassigned_ids(&dto);
                assert!(
                    !unassigned.contains(&meta),
                    "(a) {}: metadata snapshot not unassigned: {unassigned:?}",
                    variant.label
                );
                assert!(
                    !unassigned.contains(&fx.snapshot_id),
                    "(a) {}: snapshot of the backup not unassigned: {unassigned:?}",
                    variant.label
                );
                assert_eq!(
                    unassigned,
                    [foreign],
                    "{}: only the foreign snapshot",
                    variant.label
                );
                fx.rig.daemon.stop();
            }
        },
    );
}

/// AC-M4-07-4: a snapshot with `kestic:x9`, where `x9` is neither configured nor described by
/// a metadata snapshot, is (a) orphaned and (b) not unassigned.
#[test]
fn ac_m4_07_4_snapshot_of_an_unknown_backup_is_orphaned() {
    run_isolated_test(
        "ac_m4_07_4_snapshot_of_an_unknown_backup_is_orphaned",
        "KESTIC_AC_M4_07_4_INNER",
        |root| {
            for_each_variant(root, "M4F4", "kestic:x9", |fx, v| {
                let dto = list_destination(fx);
                assert_eq!(
                    orphaned_ids(&dto, "x9"),
                    std::slice::from_ref(&fx.snapshot_id),
                    "(a) {}: orphaned under x9: {:?}",
                    v.label,
                    dto.orphaned
                );
                assert!(
                    !unassigned_ids(&dto).contains(&fx.snapshot_id),
                    "(b) {}: not unassigned",
                    v.label
                );
            });
        },
    );
}

/// AC-M4-07-5: a snapshot with `kestic:x9` and a metadata snapshot `kestic-meta:x9` (x9 not
/// configured) is (a) not orphaned but listed with its described backup; (b) orphaned snapshots
/// are browsed and restored like foreign ones.
#[test]
fn ac_m4_07_5_described_backup_is_not_orphaned_and_orphaned_ones_restore() {
    run_isolated_test(
        "ac_m4_07_5_described_backup_is_not_orphaned_and_orphaned_ones_restore",
        "KESTIC_AC_M4_07_5_INNER",
        |root| {
            // The fixture's snapshot is orphaned (y7 has no metadata snapshot).
            for_each_variant(root, "M4F5", "kestic:y7", |fx, v| {
                let label = v.label;
                let described = extra_snapshot(fx, "kestic:x9", "2026-11-14 12:00:00");
                extra_snapshot(fx, "kestic-meta:x9", "2026-11-14 12:01:00");
                let dto = list_destination(fx);
                assert!(
                    orphaned_ids(&dto, "x9").is_empty(),
                    "(a) {label}: x9 is not orphaned: {:?}",
                    dto.orphaned
                );
                assert_eq!(
                    described_ids(&dto, "x9"),
                    std::slice::from_ref(&described),
                    "(a) {label}: listed with its described backup x9: {:?}",
                    dto.described
                );
                assert!(
                    !unassigned_ids(&dto).contains(&described),
                    "(a) {label}: not unassigned either"
                );
                assert_eq!(
                    orphaned_ids(&dto, "y7"),
                    std::slice::from_ref(&fx.snapshot_id),
                    "{label}: y7 is orphaned"
                );

                assert_eq!(
                    listed_by_kestic(fx),
                    listed_by_restic(fx),
                    "(b) {label}: an orphaned snapshot is browsed"
                );
                let (target, detail) = restore_into_new_folder(fx, "orphaned");
                assert_eq!(
                    fs::read_to_string(target.join("a.txt")).expect("a.txt restored"),
                    "hello",
                    "(b) {label}: an orphaned snapshot is restored"
                );
                assert_eq!(detail.run.status, RunStatus::Ok, "(b) {label}: {detail:?}");
                assert_eq!(
                    detail.run.backup_id(),
                    None,
                    "(b) {label}: like a foreign one, without a backup"
                );
            });
        },
    );
}

/// AC-M4-07-6: showing, browsing, comparing, searching and restoring a foreign and an orphaned
/// snapshot (a) runs only `snapshots`, `ls`, `find`, `diff` and `restore`, and (b) never `tag`,
/// `forget` or `rewrite`.
#[test]
fn ac_m4_07_6_foreign_and_orphaned_snapshots_are_only_read() {
    run_isolated_test(
        "ac_m4_07_6_foreign_and_orphaned_snapshots_are_only_read",
        "KESTIC_AC_M4_07_6_INNER",
        |root| {
            for_each_variant(root, "M4F6", "", |fx, v| {
                let label = v.label;
                let orphaned = extra_snapshot(fx, "kestic:x9", "2026-11-14 12:00:00");
                let older_foreign = extra_snapshot(fx, "", "2026-11-13 12:00:00");
                let entries_before = fx.rig.wrapper.entries().len();
                let dest = fx.dest.id.as_str();

                list_destination(fx);
                listed_by_kestic(fx);
                run_query(fx, &SnapshotQueryRequest::ls(dest, &orphaned, "/"));
                run_query(
                    fx,
                    &SnapshotQueryRequest::diff(dest, &older_foreign, &fx.snapshot_id),
                );
                run_query(
                    fx,
                    &SnapshotQueryRequest::find(dest, FindScope::Unassigned, "a.txt"),
                );
                run_query(
                    fx,
                    &SnapshotQueryRequest::find(
                        dest,
                        FindScope::Orphaned {
                            backup_id: "x9".to_string(),
                        },
                        "note",
                    ),
                );
                fx.preview(&["a.txt"], Some(fx.original()), RestoreOverwrite::Never);
                restore_into_new_folder(fx, "read-only");

                let started: Vec<String> = fx.rig.wrapper.entries()[entries_before..]
                    .iter()
                    // `restic version` only probes the binary and reaches no repository.
                    .filter(|e| e.kind == 'S' && e.sub != "version")
                    .map(|e| e.sub.clone())
                    .collect();
                for (sub, wanted) in [
                    ("snapshots", "show"),
                    ("ls", "browse"),
                    ("diff", "compare"),
                    ("find", "search"),
                    ("restore", "restore"),
                ] {
                    assert!(
                        started.iter().any(|s| s == sub),
                        "{label}: {wanted} ran `{sub}`: {started:?}"
                    );
                }
                assert!(
                    started
                        .iter()
                        .all(|s| READ_ONLY_COMMANDS.contains(&s.as_str())),
                    "(a) {label}: only snapshots, ls, find, diff and restore: {started:?}"
                );
                for forbidden in ["tag", "forget", "rewrite"] {
                    assert!(
                        !started.iter().any(|s| s == forbidden),
                        "(b) {label}: never `{forbidden}`: {started:?}"
                    );
                }
            });
        },
    );
}
