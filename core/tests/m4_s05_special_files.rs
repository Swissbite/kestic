// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S05 without restic: a selected file whose place at the target is a device is refused,
//! before the preview's dry runs and again before the run starts restic.

use kestic_core::backup::{BackupRunError, SnapshotListAccess};
use kestic_core::clock::FakeClock;
use kestic_core::model::{Destination, DestinationKind};
use kestic_core::provider::ResolvedRestic;
use kestic_core::restore::{EntryKind, RestorePlan, normalize_selection, plan_restore};
use kestic_core::restore_preview::{
    PreviewRun, SnapshotTree, TreeNode, execute_preview, special_file_in_selection,
};
use kestic_core::secrets::InMemorySecretStore;
use kestic_i18n::codes::RestoreOverwrite;
use semver::Version;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use tempfile::TempDir;
use time::macros::datetime;

const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The plan of a restore of the snapshot file `/home/u/null`: restored from `/home/u`, so it
/// lands at `<target>/null`.
fn plan_of_null() -> (RestorePlan, Vec<String>) {
    let selection = normalize_selection(&["/home/u/null".to_string()]).expect("selection");
    let plan = plan_restore(ID, &selection, EntryKind::Other).expect("plan");
    (plan, selection)
}

/// The snapshot tree of the preview: one file `null` in the restored folder.
fn tree_with_file_null() -> SnapshotTree {
    let mut tree = SnapshotTree::default();
    tree.nodes.insert(
        "null".to_string(),
        vec![TreeNode {
            is_dir: false,
            size: 1,
            inode: None,
            above: false,
        }],
    );
    tree.files = 1;
    tree
}

/// The preview's error for `target`; a refusal comes before any restic call (the restic binary
/// does not exist).
fn preview_error(target: &Path) -> Option<BackupRunError> {
    let tmp = TempDir::new().expect("tempdir");
    let dest = Destination::new(
        "NAS",
        DestinationKind::Local {
            path: tmp.path().join("repo"),
        },
    )
    .expect("dest");
    let resolved = ResolvedRestic {
        binary: tmp.path().join("no-restic"),
        version: Version::new(0, 18, 0),
    };
    let store = InMemorySecretStore::default();
    let clock = FakeClock::new(datetime!(2026-11-15 12:00 UTC));
    let kestic_bin = tmp.path().join("kestic");
    let access = SnapshotListAccess {
        destination: &dest,
        resolved: &resolved,
        kestic_binary: &kestic_bin,
        secret_store: &store,
        clock: &clock,
        extra_env: &[],
    };
    let (plan, _) = plan_of_null();
    let tree = tree_with_file_null();
    let run = PreviewRun {
        plan: &plan,
        tree: &tree,
        target,
        overwrite: RestoreOverwrite::Never,
        foreign_owner: false,
        limit_bytes: 1 << 20,
    };
    execute_preview(&access, &run, None).err()
}

/// AC-M4-05-25, device at the target: a snapshot file that would be written onto the character
/// device `/dev/null` is refused by the preview before any dry run with (a) an invalid target
/// and (b) `/dev/null` as the affected path, and the check of the run before restic (b) names
/// `/dev/null` too; the same file onto a regular file is not refused. No block device can be
/// created without root, so only the character device is covered.
#[test]
fn ac_m4_05_25_file_onto_a_character_device_is_refused() {
    let dev_null = std::fs::symlink_metadata("/dev/null").expect("stat /dev/null");
    assert!(
        dev_null.file_type().is_char_device(),
        "precondition: /dev/null is a character device"
    );
    let (plan, selection) = plan_of_null();

    let refusal = preview_error(Path::new("/dev"));
    assert!(
        matches!(refusal, Some(BackupRunError::InvalidTarget { .. })),
        "AC-M4-05-25 (a): the preview refuses the target as invalid, got {refusal:?}"
    );
    let refused_path = match refusal {
        Some(BackupRunError::InvalidTarget { path, .. }) => Some(path),
        _ => None,
    };
    assert_eq!(
        refused_path.as_deref(),
        Some("/dev/null"),
        "AC-M4-05-25 (b): the preview names the device /dev/null"
    );
    assert_eq!(
        special_file_in_selection(&plan, &selection, Path::new("/dev")),
        Some("/dev/null".to_string()),
        "AC-M4-05-25 (b): the run's check names the device /dev/null"
    );

    let regular = TempDir::new().expect("tempdir");
    std::fs::write(regular.path().join("null"), b"x").expect("regular file");
    assert_eq!(
        special_file_in_selection(&plan, &selection, regular.path()),
        None,
        "AC-M4-05-25: a regular file at the target is no special file"
    );
}
