// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S07 without restic: the commands Kestic builds for a foreign or orphaned snapshot.

use kestic_core::restore::{
    EntryKind, RestoreCall, normalize_selection, plan_restore, restore_args,
};
use kestic_core::snapshot_diff::diff_args;
use kestic_core::snapshot_find::find_args;
use kestic_core::snapshot_ls::ls_args;
use kestic_i18n::codes::RestoreOverwrite;
use std::path::Path;

const FOREIGN: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const ORPHANED: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// AC-M4-07-6: browsing, comparing, searching and restoring a foreign or orphaned snapshot
/// (a) builds only `ls`, `diff`, `find` and `restore` calls, and (b) never `tag`, `forget` or
/// `rewrite`.
#[test]
fn ac_m4_07_6_commands_on_foreign_snapshots_only_read() {
    let selection = normalize_selection(&["/home/user/a.txt".to_string()]).expect("selection");
    let mut calls = Vec::new();
    for id in [FOREIGN, ORPHANED] {
        calls.push(("ls", ls_args(id, "/home/user")));
        calls.push(("find", find_args(&[id], "*a.txt*")));
        let plan = plan_restore(id, &selection, EntryKind::Other).expect("plan");
        for call in [RestoreCall::DryRun, RestoreCall::Run] {
            calls.push((
                "restore",
                restore_args(&plan, Path::new("/t"), RestoreOverwrite::Never, call),
            ));
        }
    }
    calls.push(("diff", diff_args(FOREIGN, ORPHANED)));

    for (expected, args) in &calls {
        assert_eq!(
            args.first().map(String::as_str),
            Some(*expected),
            "(a) subcommand of {args:?}"
        );
        for forbidden in ["tag", "forget", "rewrite"] {
            assert!(
                !args.iter().any(|a| a == forbidden),
                "(b) never `{forbidden}`: {args:?}"
            );
        }
    }
}
