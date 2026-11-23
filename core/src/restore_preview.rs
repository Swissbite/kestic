// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Preview of a restore: which files would be new, overwritten, kept or unchanged. Two dry runs
//! of `restic restore` (the chosen mode and, unless it is `always`, `always`) are joined with a
//! recursive `restic ls` of the restored folder, which gives the types, sizes and inodes the dry
//! run leaves out.

use crate::backup::{BackupRunError, SnapshotListAccess, check_command_succeeded};
use crate::query_list::{HeldResult, ListBuilder, ListEntry, ListPage, PageRequest, QueryList};
use crate::restic::CancelHandle;
use crate::restore::{
    RestoreCall, RestoreLine, RestorePlan, check_restore_succeeded, parse_restore_line,
    restore_args, run_streaming,
};
use kestic_i18n::codes::{RestoreNoteCode, RestoreOutcome, RestoreOverwrite};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// One file of a preview (`RestorePreviewEntryDto`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePreviewEntry {
    /// Path the file gets at the target.
    pub path: String,
    pub outcome: RestoreOutcome,
    pub size: u64,
}

impl ListEntry for RestorePreviewEntry {
    fn path(&self) -> &str {
        &self.path
    }

    fn kind_code(&self) -> Option<&str> {
        Some(self.outcome.as_str())
    }
}

/// Counts of a preview; they cover the whole restore, independent of any filter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewCounts {
    pub new_files: u64,
    pub overwritten_files: u64,
    pub kept_files: u64,
    pub identical_files: u64,
    /// Bytes of the new and overwritten files.
    pub bytes_to_write: u64,
}

impl PreviewCounts {
    fn add(&mut self, outcome: RestoreOutcome, size: u64) {
        match outcome {
            RestoreOutcome::New => self.new_files += 1,
            RestoreOutcome::Overwritten => self.overwritten_files += 1,
            RestoreOutcome::Kept => self.kept_files += 1,
            RestoreOutcome::Identical => self.identical_files += 1,
        }
        if matches!(outcome, RestoreOutcome::New | RestoreOutcome::Overwritten) {
            self.bytes_to_write += size;
        }
    }

    /// Number of files with `outcome`.
    pub fn of(&self, outcome: RestoreOutcome) -> u64 {
        match outcome {
            RestoreOutcome::New => self.new_files,
            RestoreOutcome::Overwritten => self.overwritten_files,
            RestoreOutcome::Kept => self.kept_files,
            RestoreOutcome::Identical => self.identical_files,
        }
    }
}

/// The classified files of a preview with its notes.
pub struct PreviewClassification {
    pub counts: PreviewCounts,
    pub entries: QueryList<RestorePreviewEntry>,
    pub notes: Vec<RestoreNoteCode>,
    /// Folders with content at the target where the snapshot has a file: restic cannot put the
    /// file there, fails, and changes the folder's mode.
    pub blocked: Vec<String>,
}

impl HeldResult for PreviewClassification {
    fn held_bytes(&self) -> u64 {
        self.entries.held_bytes()
    }
}

impl PreviewClassification {
    /// An empty preview (the default target: everything is new there anyway).
    pub fn empty() -> Self {
        Self {
            counts: PreviewCounts::default(),
            entries: ListBuilder::new(0).finish(),
            notes: Vec::new(),
            blocked: Vec::new(),
        }
    }

    /// The page `request` asks for. For a list too large to hold, `matched` of a filter by
    /// kinds only still comes from the exact counts.
    pub fn page(&mut self, request: &PageRequest) -> ListPage<RestorePreviewEntry> {
        let mut page = self.entries.page(request);
        if page.too_large && request.filter.text.is_empty() && !request.filter.kinds.is_empty() {
            page.matched = RestoreOutcome::ALL
                .into_iter()
                .filter(|o| request.filter.kinds.iter().any(|k| k == o.as_str()))
                .map(|o| self.counts.of(o))
                .sum();
        }
        page
    }
}

/// An entry of the restored folder as `restic ls --json --recursive` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeNode {
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub inode: Option<u64>,
    pub mtime: Option<OffsetDateTime>,
}

/// The entries of the restored folder by their path relative to it (starting with `/`, as the
/// dry run names them). restic shows the bytes of a name that are not UTF-8 as U+FFFD, so
/// several entries can share one path; they are kept in the order restic lists them.
#[derive(Debug, Default)]
pub struct SnapshotTree {
    pub nodes: HashMap<String, Vec<TreeNode>>,
}

impl SnapshotTree {
    /// Collects the output lines of `restic ls --json --recursive <id> <folder>`.
    pub fn collector(folder: &str) -> TreeCollector {
        TreeCollector {
            folder: folder.to_string(),
            tree: SnapshotTree::default(),
        }
    }

    /// Inodes that more than one file of the folder shares (hard links within the snapshot).
    fn shared_inodes(&self) -> BTreeSet<u64> {
        let mut seen = BTreeSet::new();
        let mut shared = BTreeSet::new();
        for node in self.nodes.values().flatten().filter(|n| !n.is_dir) {
            if let Some(inode) = node.inode
                && !seen.insert(inode)
            {
                shared.insert(inode);
            }
        }
        shared
    }
}

/// Collector of [`SnapshotTree::collector`].
pub struct TreeCollector {
    folder: String,
    tree: SnapshotTree,
}

#[derive(Deserialize)]
struct RawTreeNode {
    #[serde(default)]
    message_type: Option<String>,
    #[serde(default, rename = "type")]
    node_type: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    inode: Option<u64>,
    #[serde(default)]
    mtime: Option<String>,
}

impl TreeCollector {
    pub fn push_line(&mut self, line: &str) {
        let Ok(node) = serde_json::from_str::<RawTreeNode>(line.trim()) else {
            return;
        };
        if node.message_type.as_deref() != Some("node") {
            return;
        }
        let relative = if self.folder == "/" {
            node.path.as_str()
        } else {
            match node.path.strip_prefix(self.folder.as_str()) {
                Some(rest) if rest.starts_with('/') => rest,
                _ => return,
            }
        };
        self.tree
            .nodes
            .entry(relative.to_string())
            .or_default()
            .push(TreeNode {
                is_dir: node.node_type == "dir",
                is_symlink: node.node_type == "symlink",
                size: node.size.unwrap_or(0),
                inode: node.inode,
                mtime: node
                    .mtime
                    .and_then(|t| OffsetDateTime::parse(&t, &Rfc3339).ok()),
            });
    }

    pub fn finish(self) -> SnapshotTree {
        self.tree
    }
}

/// The actions of one dry run by the path of the entry relative to the target; entries whose
/// names restic shows alike (U+FFFD) share a path and keep the order of the dry run.
pub type DryRunActions = HashMap<String, Vec<String>>;

/// What the preview needs besides the dry runs.
pub struct ClassifyInput<'a> {
    pub tree: &'a SnapshotTree,
    /// Actions of the dry run with the chosen mode.
    pub chosen: &'a DryRunActions,
    /// Actions of the dry run with `always`; `None` when the chosen mode is `always`.
    pub always: Option<&'a DryRunActions>,
    pub overwrite: RestoreOverwrite,
    /// The checked target folder; entries are looked up below it.
    pub target: &'a Path,
    /// Whether the snapshot belongs to another user than the one restoring.
    pub foreign_owner: bool,
    pub limit_bytes: u64,
}

/// Joins the dry runs with the tree into one outcome per file and the notes. Folders are not
/// counted; an entry the dry run reports as `restored` where the target already has something
/// replaces it and counts as overwritten.
pub fn classify_preview(input: &ClassifyInput<'_>) -> PreviewClassification {
    let shared_inodes = input.tree.shared_inodes();
    let mut counts = PreviewCounts::default();
    let mut builder = ListBuilder::new(input.limit_bytes);
    let mut notes: Vec<RestoreNoteCode> = Vec::new();
    let note = |code: RestoreNoteCode, notes: &mut Vec<RestoreNoteCode>| {
        if !notes.contains(&code) {
            notes.push(code);
        }
    };
    let mut blocked: Vec<String> = Vec::new();
    let mut items: Vec<(&String, usize, &String)> = input
        .chosen
        .iter()
        .flat_map(|(item, actions)| actions.iter().enumerate().map(move |(i, a)| (item, i, a)))
        .collect();
    items.sort();
    for (item, index, action) in items {
        // Below a symbolic link at the target, restic puts a real folder in place of the link
        // and never writes through it: what lies behind the link does not count.
        let found = find_at_target(input.target, item);
        let behind_link = found.behind_link;
        let node = input
            .tree
            .nodes
            .get(item.as_str())
            .and_then(|nodes| nodes.get(index).or(nodes.first()));
        let restored = action == "restored";
        let rank = input.chosen[item][..index]
            .iter()
            .filter(|a| (*a == "restored") == restored)
            .count();
        let picked = if behind_link {
            None
        } else {
            pick_existing(
                &found.entries,
                item.contains('\u{FFFD}'),
                node.is_some_and(|n| n.is_dir),
                restored,
                rank,
            )
        };
        let at_target = picked.map_or_else(
            || input.target.join(item.trim_start_matches('/')),
            |(p, _)| p.clone(),
        );
        let existing = picked.map(|(_, m)| m.clone());
        // A symbolic link where the snapshot has one too is no surprise.
        if !node.is_some_and(|n| n.is_symlink)
            && found
                .entries
                .iter()
                .any(|(_, m)| m.file_type().is_symlink())
        {
            note(RestoreNoteCode::SymlinkInTarget, &mut notes);
        }
        if node.is_some_and(|n| n.is_dir) {
            // A folder is not counted, unless restic puts it in place of a file or a symbolic
            // link at the target (it does so even with `never`): that entry is overwritten.
            let replaces_file =
                action == "restored" && existing.as_ref().is_some_and(|m| !m.is_dir());
            if !replaces_file {
                continue;
            }
            counts.add(RestoreOutcome::Overwritten, 0);
            builder.push(RestorePreviewEntry {
                path: at_target.to_string_lossy().into_owned(),
                outcome: RestoreOutcome::Overwritten,
                size: 0,
            });
            continue;
        }
        // restic reports a symbolic link or a hard-linked file as `restored` with `always` and
        // `if-changed` even when the target already holds the same entry; the same type, size
        // and modification time as in the snapshot count as unchanged.
        let same_link = node.is_some_and(|n| {
            let linked = n.is_symlink || n.inode.is_some_and(|i| shared_inodes.contains(&i));
            linked && existing.as_ref().is_some_and(|m| same_link_entry(n, m))
        });
        let outcome = match action.as_str() {
            // The dry run looks through the link; the run writes into a new, real folder.
            _ if behind_link => RestoreOutcome::New,
            "restored" | "unchanged" if same_link => RestoreOutcome::Identical,
            "restored" if existing.is_some() => RestoreOutcome::Overwritten,
            "restored" => RestoreOutcome::New,
            "updated" => RestoreOutcome::Overwritten,
            _ if input.overwrite == RestoreOverwrite::Always => RestoreOutcome::Identical,
            _ => match input
                .always
                .and_then(|a| a.get(item.as_str()))
                .and_then(|actions| actions.get(index))
            {
                Some(a) if a == "updated" || a == "restored" => RestoreOutcome::Kept,
                _ => RestoreOutcome::Identical,
            },
        };
        // Entries whose names restic shows alike cannot be told apart at the target: any of
        // them that is a folder with content blocks the file.
        if let Some((dir, _)) = found.entries.iter().find(|(p, m)| {
            !behind_link && m.is_dir() && std::fs::read_dir(p).is_ok_and(|mut d| d.next().is_some())
        }) {
            blocked.push(dir.to_string_lossy().into_owned());
            continue;
        }
        let size = node.map_or(0, |n| n.size);
        if outcome == RestoreOutcome::Overwritten {
            let linked_in_snapshot = node
                .and_then(|n| n.inode)
                .is_some_and(|i| shared_inodes.contains(&i));
            let linked_at_target = !behind_link
                && found
                    .entries
                    .iter()
                    .any(|(_, m)| m.is_file() && m.nlink() > 1);
            if linked_in_snapshot || linked_at_target {
                note(RestoreNoteCode::HardLinksAffected, &mut notes);
            }
        }
        counts.add(outcome, size);
        builder.push(RestorePreviewEntry {
            path: at_target.to_string_lossy().into_owned(),
            outcome,
            size,
        });
    }
    let entries = builder.finish();
    if input.foreign_owner {
        note(RestoreNoteCode::OwnershipNotRestored, &mut notes);
    }
    if entries.total() == 0 {
        note(RestoreNoteCode::NothingSelectedMatches, &mut notes);
    }
    PreviewClassification {
        counts,
        entries,
        blocked,
        notes: RestoreNoteCode::ALL
            .into_iter()
            .filter(|c| notes.contains(c))
            .collect(),
    }
}

/// Whether `existing` at the target is the link-like snapshot entry `node`: a symbolic link for
/// a symbolic link, a file of the same size for a hard-linked file, with the same modification
/// time (restic restores it, also for symbolic links).
fn same_link_entry(node: &TreeNode, existing: &std::fs::Metadata) -> bool {
    let same_type = if node.is_symlink {
        existing.file_type().is_symlink()
    } else {
        existing.is_file() && existing.len() == node.size
    };
    let modified = existing.modified().ok().map(OffsetDateTime::from);
    same_type && node.mtime.is_some() && modified == node.mtime
}

/// The existing entry at the target that belongs to the `rank`-th action of its kind
/// (`restored` or not) for one path. For a plain name that is the one entry. Names restic shows
/// alike (`shown_alike`, U+FFFD) cannot be paired by name: an entry of another type (or a
/// symbolic link) is what a `restored` action replaces, an entry of the same type is what the
/// other actions look at.
fn pick_existing(
    entries: &[(PathBuf, std::fs::Metadata)],
    shown_alike: bool,
    node_is_dir: bool,
    restored: bool,
    rank: usize,
) -> Option<&(PathBuf, std::fs::Metadata)> {
    if !shown_alike {
        return entries.first();
    }
    let replaced = |m: &std::fs::Metadata| m.file_type().is_symlink() || m.is_dir() != node_is_dir;
    entries
        .iter()
        .filter(|(_, m)| replaced(m) == restored)
        .nth(rank)
}

/// What lies at the target for one entry of a dry run.
struct AtTarget {
    /// Whether a folder above the entry is a symbolic link at the target.
    behind_link: bool,
    /// The existing entries (`lstat`) with that name, sorted by their real name. A name with
    /// U+FFFD matches every real name that restic shows that way, so there can be several.
    entries: Vec<(PathBuf, std::fs::Metadata)>,
}

/// Looks up `item` (relative to `target`, starting with `/`) at the target. A segment with
/// U+FFFD is matched against the real names of its folder, shown the way restic shows them.
fn find_at_target(target: &Path, item: &str) -> AtTarget {
    let parts: Vec<&str> = item.split('/').filter(|s| !s.is_empty()).collect();
    let mut candidates = vec![target.to_path_buf()];
    for (i, part) in parts.iter().enumerate() {
        let mut next = Vec::new();
        for dir in &candidates {
            if part.contains('\u{FFFD}') {
                let Ok(read) = std::fs::read_dir(dir) else {
                    continue;
                };
                let mut found: Vec<PathBuf> = read
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy() == *part)
                    .map(|e| e.path())
                    .collect();
                found.sort();
                next.extend(found);
            } else {
                let path = dir.join(part);
                if std::fs::symlink_metadata(&path).is_ok() {
                    next.push(path);
                }
            }
        }
        let last = i + 1 == parts.len();
        if !last
            && next
                .iter()
                .any(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()))
        {
            return AtTarget {
                behind_link: true,
                entries: Vec::new(),
            };
        }
        candidates = next;
    }
    AtTarget {
        behind_link: false,
        entries: candidates
            .into_iter()
            .filter_map(|p| std::fs::symlink_metadata(&p).ok().map(|m| (p, m)))
            .collect(),
    }
}

/// Walks the restored folder of `plan` (`restic ls --json --recursive --no-lock`).
pub fn walk_folder(
    access: &SnapshotListAccess<'_>,
    plan: &RestorePlan,
    cancel: Option<&CancelHandle>,
) -> Result<SnapshotTree, BackupRunError> {
    let args = vec![
        "ls".to_string(),
        "--no-lock".to_string(),
        "--recursive".to_string(),
        plan.snapshot_id.clone(),
        plan.folder.clone(),
    ];
    let mut collector = SnapshotTree::collector(&plan.folder);
    let res = run_streaming(access, args, cancel, &mut |line| collector.push_line(line))?;
    check_command_succeeded(&res)?;
    Ok(collector.finish())
}

/// One dry run of `plan` into `target` with `overwrite` (`--dry-run --no-lock -vv`).
pub fn dry_run(
    access: &SnapshotListAccess<'_>,
    plan: &RestorePlan,
    target: &Path,
    overwrite: RestoreOverwrite,
    cancel: Option<&CancelHandle>,
) -> Result<DryRunActions, BackupRunError> {
    let args = restore_args(plan, target, overwrite, RestoreCall::DryRun);
    let mut actions = DryRunActions::new();
    let res = run_streaming(access, args, cancel, &mut |line| {
        if let RestoreLine::Item(item) = parse_restore_line(line) {
            actions.entry(item.item).or_default().push(item.action);
        }
    })?;
    // Where the target has a file and the snapshot a folder with content, the dry run cannot
    // look below the file and fails («not a directory»); the run itself puts the folder in
    // place of the file, so the entries below it are new.
    if let Some(items) = below_replaced_files(&res) {
        for item in items {
            actions.insert(item, vec!["restored".to_string()]);
        }
        return Ok(actions);
    }
    check_restore_succeeded(&res, plan)?;
    Ok(actions)
}

/// The items of a dry run that failed only because entries lie below a file at the target.
fn below_replaced_files(res: &crate::restic::ResticExecutionResult) -> Option<Vec<String>> {
    if res.exit_code != 1 || res.errors.is_empty() {
        return None;
    }
    let error_lines: Vec<&String> = res
        .log_lines
        .iter()
        .filter(|l| l.contains("\"message_type\":\"error\""))
        .collect();
    // The error text (`error.message`) ends with the cause; the path in front of it and the
    // item may contain anything.
    let cause_is_not_a_directory = |line: &str| {
        serde_json::from_str::<serde_json::Value>(line).is_ok_and(|v| {
            v["error"]["message"]
                .as_str()
                .is_some_and(|m| m.trim_end().ends_with(": not a directory"))
        })
    };
    let only_these = error_lines.len() == res.errors.len()
        && error_lines.iter().all(|l| cause_is_not_a_directory(l));
    if !only_these {
        return None;
    }
    res.errors.iter().map(|e| e.item.clone()).collect()
}

/// Runs the dry runs of a preview and classifies them.
pub fn execute_preview(
    access: &SnapshotListAccess<'_>,
    plan: &RestorePlan,
    target: &Path,
    overwrite: RestoreOverwrite,
    foreign_owner: bool,
    cancel: Option<&CancelHandle>,
    limit_bytes: u64,
) -> Result<PreviewClassification, BackupRunError> {
    let tree = walk_folder(access, plan, cancel)?;
    let chosen = dry_run(access, plan, target, overwrite, cancel)?;
    let always = if overwrite == RestoreOverwrite::Always {
        None
    } else {
        Some(dry_run(
            access,
            plan,
            target,
            RestoreOverwrite::Always,
            cancel,
        )?)
    };
    let classification = classify_preview(&ClassifyInput {
        tree: &tree,
        chosen: &chosen,
        always: always.as_ref(),
        overwrite,
        target,
        foreign_owner,
        limit_bytes,
    });
    // Such a target would fail and lose its folder's mode: the restore is refused up front.
    if let Some(first) = classification.blocked.first() {
        return Err(BackupRunError::InvalidTarget(format!(
            "{first} is a folder with content where the snapshot has a file; restore into a new folder instead"
        )));
    }
    Ok(classification)
}
