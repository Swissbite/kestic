// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S07 without restic: the commands Kestic builds for a foreign or orphaned snapshot.

use kestic_core::backup::{
    SnapshotListAccess, build_forget_command_spec, list_destination_snapshots,
};
use kestic_core::clock::FakeClock;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationKind, KeepCount, RetentionPolicy, Source,
};
use kestic_core::provider::ResolvedRestic;
use kestic_core::restic::parse_recorded_invocation;
use kestic_core::restore::{
    EntryKind, RestoreCall, normalize_selection, plan_restore, restore_args,
};
use kestic_core::secrets::InMemorySecretStore;
use kestic_core::snapshot::ResticSnapshot;
use kestic_core::snapshot_diff::diff_args;
use kestic_core::snapshot_find::find_args;
use kestic_core::snapshot_list::sort_destination_snapshots;
use kestic_core::snapshot_ls::ls_args;
use kestic_i18n::codes::RestoreOverwrite;
use kestic_test_support::tools::write_executable;
use semver::Version;
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use time::macros::datetime;

const FOREIGN: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const ORPHANED: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const OWN: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const ALLOWED: [&str; 5] = ["snapshots", "ls", "find", "diff", "restore"];
const FORBIDDEN: [&str; 3] = ["tag", "forget", "rewrite"];

/// A restic stand-in that logs its arguments to `calls.log` and prints `listing` for
/// `snapshots`.
fn spy_restic(dir: &Path, listing: &str) -> ResolvedRestic {
    let bin = dir.join("spy-restic");
    let fixture = dir.join("snapshots.json");
    fs::write(&fixture, listing).expect("write listing");
    write_executable(
        &bin,
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in *snapshots*) cat '{}'; echo ;; esac\nexit 0\n",
            dir.join("calls.log").display(),
            fixture.display()
        ),
    );
    ResolvedRestic {
        binary: bin,
        version: Version::new(0, 18, 0),
    }
}

/// The restic subcommand and its arguments of each logged call, without `-r <repo> --json`.
fn logged_subcommands(dir: &Path, bin: &Path) -> Vec<Vec<String>> {
    fs::read_to_string(dir.join("calls.log"))
        .expect("calls.log")
        .lines()
        .map(|line| {
            let args: Vec<String> = line.split_whitespace().map(str::to_string).collect();
            parse_recorded_invocation(&bin.to_string_lossy(), &args).subcommand_args
        })
        .collect()
}

/// AC-M4-07-6: listing, browsing, searching, comparing and restoring a foreign or orphaned
/// snapshot (a) builds only `snapshots`, `ls`, `find`, `diff` and `restore` calls, and (b) no
/// command Kestic builds reaches it with `tag`, `forget` or `rewrite`: the only `forget` Kestic
/// builds is the retention of a configured backup, filtered by that backup's tag.
#[test]
fn ac_m4_07_6_commands_on_foreign_snapshots_only_read() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).expect("mkdir repo");
    let dest = Destination::new("NAS", DestinationKind::Local { path: repo }).expect("dest");
    let mut backup =
        Backup::new("Home", Source::folder("/home/user"), dest.id.clone()).expect("backup");
    backup.retention = RetentionPolicy {
        keep_last: Some(KeepCount::Count(3)),
        ..RetentionPolicy::empty()
    };
    let orphaned_id = BackupId::generate().expect("orphaned backup id");
    let listing = serde_json::json!([
        {"id": FOREIGN, "time": "2026-11-09T12:00:00Z", "tags": ["daily"],
         "paths": ["/home/user"], "hostname": "laptop"},
        {"id": ORPHANED, "time": "2026-11-10T12:00:00Z", "tags": [format!("kestic:{orphaned_id}")],
         "paths": ["/home/user"], "hostname": "laptop"},
        {"id": OWN, "time": "2026-11-11T12:00:00Z", "tags": [backup.tag()],
         "paths": ["/home/user"], "hostname": "laptop"},
    ])
    .to_string();
    let resolved = spy_restic(tmp.path(), &listing);
    let store = InMemorySecretStore::default();
    let clock = FakeClock::new(datetime!(2026-11-15 14:32:00 UTC));
    let kestic_bin = tmp.path().join("kestic");

    // Listing: the destination's snapshots come from one `snapshots` call.
    let access = SnapshotListAccess {
        destination: &dest,
        resolved: &resolved,
        kestic_binary: &kestic_bin,
        secret_store: &store,
        clock: &clock,
        extra_env: &[],
    };
    let listed = list_destination_snapshots(&access).expect("listing");
    let mut calls = logged_subcommands(tmp.path(), &resolved.binary);
    assert_eq!(
        calls.len(),
        1,
        "AC-M4-07-6 (a): one listing call: {calls:?}"
    );
    assert_eq!(
        calls[0].first().map(String::as_str),
        Some("snapshots"),
        "AC-M4-07-6 (a): the listing is a `snapshots` call: {calls:?}"
    );

    // The foreign and the orphaned snapshot as the destination sorts them.
    let sorted = sort_destination_snapshots(listed, &[backup.id.as_str()]);
    let foreign: Vec<&ResticSnapshot> = sorted.unassigned.iter().collect();
    let orphaned: Vec<&ResticSnapshot> = sorted.orphaned.iter().flat_map(|(_, s)| s).collect();
    assert_eq!(
        foreign.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        [FOREIGN],
        "the foreign snapshot is unassigned"
    );
    assert_eq!(
        orphaned.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        [ORPHANED],
        "the snapshot of the unconfigured backup is orphaned"
    );

    // Browsing, searching, comparing and restoring them, with the production builders.
    let selection = normalize_selection(&["/home/user/a.txt".to_string()]).expect("selection");
    for snap in foreign.iter().chain(&orphaned) {
        let id = snap.id.as_str();
        calls.push(ls_args(id, "/home/user"));
        calls.push(find_args(&[id], "*a.txt*"));
        let plan = plan_restore(id, &selection, EntryKind::Other).expect("plan");
        for call in [RestoreCall::DryRun, RestoreCall::Run] {
            calls.push(restore_args(
                &plan,
                Path::new("/t"),
                RestoreOverwrite::Never,
                call,
            ));
        }
    }
    calls.push(diff_args(FOREIGN, ORPHANED));

    for args in &calls {
        let sub = args.first().map_or_default(String::as_str);
        assert!(
            ALLOWED.contains(&sub),
            "AC-M4-07-6 (a): `{sub}` is one of {ALLOWED:?}: {args:?}"
        );
        assert!(
            !FORBIDDEN.contains(&sub),
            "AC-M4-07-6 (b): never `{sub}`: {args:?}"
        );
    }
    let subcommands: Vec<&str> = calls
        .iter()
        .filter_map(|a| a.first().map(String::as_str))
        .collect();
    for expected in ALLOWED {
        assert!(
            subcommands.contains(&expected),
            "AC-M4-07-6 (a): `{expected}` is built: {subcommands:?}"
        );
    }

    // The retention `forget` is built for the configured backup only and selects its snapshots
    // by tag, so it cannot reach the foreign or the orphaned snapshot.
    let forget = build_forget_command_spec(&backup, &dest, false, false, &resolved, &kestic_bin)
        .expect("forget spec")
        .subcommand_args;
    let tag_filter = forget
        .iter()
        .position(|a| a == "--tag")
        .and_then(|i| forget.get(i + 1));
    assert_eq!(
        tag_filter,
        Some(&backup.tag()),
        "AC-M4-07-6 (b): `forget` selects the configured backup's tag: {forget:?}"
    );
    for snap in foreign.iter().chain(&orphaned) {
        assert!(
            !snap.tags.contains(&backup.tag()),
            "AC-M4-07-6 (b): {} does not carry the tag `forget` selects",
            snap.id
        );
        assert!(
            !forget.iter().any(|a| a.contains(&snap.id[..8])),
            "AC-M4-07-6 (b): `forget` does not name {}: {forget:?}",
            snap.id
        );
    }
}
