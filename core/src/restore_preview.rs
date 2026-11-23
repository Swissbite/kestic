// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Preview of a restore: which files would be new, overwritten, kept or unchanged. Two dry runs
//! of `restic restore` (the chosen mode and, unless it is `always`, `always`) are joined with a
//! recursive `restic ls` of the restored folder, which gives the types, sizes and inodes the dry
//! run leaves out.

use crate::backup::{BackupRunError, SnapshotListAccess, check_command_succeeded, run_streaming};
use crate::query_list::{
    ENTRY_OVERHEAD_BYTES, HeldResult, ListBuilder, ListEntry, ListPage, PageRequest, QueryList,
};
use crate::restic::CancelHandle;
use crate::restore::{
    RestoreCall, RestoreLine, RestorePlan, check_restore_succeeded, parse_restore_line,
    restore_args,
};
use crate::snapshot_ls::RawLsNode;
use kestic_i18n::codes::{RestoreNoteCode, RestoreOutcome, RestoreOverwrite};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

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
    /// Folders with content at the target where the snapshot has a file the restore would
    /// write: restic cannot put the file there, fails, and changes the folder's mode.
    pub blocked: Vec<String>,
}

impl HeldResult for PreviewClassification {
    fn held_bytes(&self) -> u64 {
        self.entries.held_bytes()
    }
}

impl PreviewClassification {
    /// A preview over the memory limit of a query: nothing classified, the list is too large
    /// with `files` entries.
    pub fn too_large(files: u64) -> Self {
        let mut entries = ListBuilder::new(0);
        entries.push_dropped(files.max(1));
        Self {
            counts: PreviewCounts::default(),
            entries: entries.finish(),
            notes: Vec::new(),
            blocked: Vec::new(),
        }
    }

    /// Whether the preview stopped at the memory limit instead of classifying the files.
    pub fn is_too_large(&self) -> bool {
        self.entries.too_large()
    }

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
    pub size: u64,
    pub inode: Option<u64>,
    /// A folder above the selected entries, not selected itself: restic creates it where it is
    /// missing; anything else than a real folder there is refused (see
    /// [`blocked_folders_above`]).
    pub above: bool,
}

/// The entries of the restored folder by their path relative to it (starting with `/`, as the
/// dry run names them). restic shows the bytes of a name that are not UTF-8 as U+FFFD, so
/// several entries can share one path; they are kept in the order restic lists them.
#[derive(Debug, Default)]
pub struct SnapshotTree {
    pub nodes: HashMap<String, Vec<TreeNode>>,
    /// Memory of `nodes`, counted like a list entry (path plus overhead).
    pub held_bytes: u64,
    /// The walk exceeded the memory limit of a query: `nodes` is empty, the preview cannot
    /// classify anything.
    pub too_large: bool,
    /// Entries that are no folders, also those dropped for the limit.
    pub files: u64,
}

impl SnapshotTree {
    /// Collects the output lines of `restic ls --json --recursive <id> <folder>`, holding at
    /// most `limit_bytes`.
    pub fn collector(folder: &str, limit_bytes: u64) -> TreeCollector {
        TreeCollector {
            folder: folder.to_string(),
            tree: SnapshotTree::default(),
            limit_bytes,
        }
    }

    /// Whether the restore of `selected` (paths relative to the restored folder, each starting
    /// with `/`; `None` for the whole folder) writes an entry at `below` or under it. restic
    /// restores a selected folder with everything in it.
    pub fn writes_below(&self, selected: Option<&[String]>, below: &str) -> bool {
        let under = |path: &str, top: &str| {
            path == top
                || path
                    .strip_prefix(top)
                    .is_some_and(|rest| rest.starts_with('/'))
        };
        self.nodes.keys().any(|path| {
            under(path, below) && selected.is_none_or(|sel| sel.iter().any(|s| under(path, s)))
        })
    }

    /// Adds the folders between the restored folder and each selected entry (`selected`, paths
    /// relative to it): the walk lists only the selected entries, but restic creates the folders
    /// above them, and the dry run reports them. Folders above an entry the snapshot lacks are
    /// added too, so the preview checks them like the run does; they never count as written
    /// below a repository, because only entries below a selected one do.
    pub fn add_folders_above(&mut self, selected: Option<&[String]>) {
        for path in selected.unwrap_or_default() {
            let mut end = path.len();
            while let Some(slash) = path[..end].rfind('/').filter(|&i| i > 0) {
                end = slash;
                self.nodes
                    .entry(path[..end].to_string())
                    .or_insert_with(|| {
                        vec![TreeNode {
                            is_dir: true,
                            size: 0,
                            inode: None,
                            above: true,
                        }]
                    });
            }
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
    limit_bytes: u64,
}

impl TreeCollector {
    pub fn push_line(&mut self, line: &str) {
        let Some(node) = RawLsNode::parse(line).filter(RawLsNode::is_node) else {
            return;
        };
        let relative = if self.folder == "/" {
            node.path.as_str()
        } else {
            match node.path.strip_prefix(self.folder.as_str()) {
                Some(rest) if rest.starts_with('/') => rest,
                _ => return,
            }
        };
        let is_dir = node.is_dir();
        if !is_dir {
            self.tree.files += 1;
        }
        if self.tree.too_large {
            return;
        }
        self.tree.held_bytes += relative.len() as u64 + ENTRY_OVERHEAD_BYTES;
        if self.tree.held_bytes > self.limit_bytes {
            self.tree.too_large = true;
            self.tree.nodes = HashMap::new();
            return;
        }
        self.tree
            .nodes
            .entry(relative.to_string())
            .or_default()
            .push(TreeNode {
                is_dir,
                size: node.size.unwrap_or(0),
                inode: node.inode,
                above: false,
            });
    }

    pub fn finish(self) -> SnapshotTree {
        self.tree
    }
}

/// What a dry run reports for one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DryRunAction {
    /// restic writes the entry where nothing of its kind is (`restored`).
    Restored,
    /// restic replaces the content of an existing entry (`updated`).
    Updated,
    /// restic leaves the entry as it is (`unchanged` or anything else).
    Unchanged,
}

impl DryRunAction {
    /// The action of a `verbose_status` line.
    pub fn parse(action: &str) -> Self {
        match action {
            "restored" => Self::Restored,
            "updated" => Self::Updated,
            _ => Self::Unchanged,
        }
    }

    fn writes(self) -> bool {
        self != Self::Unchanged
    }
}

/// The actions of one dry run by the path of the entry relative to the target; entries whose
/// names restic shows alike (U+FFFD) share a path and keep the order of the dry run.
pub type DryRunActions = HashMap<String, Vec<DryRunAction>>;

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
    let mut items: Vec<(&String, usize, DryRunAction)> = input
        .chosen
        .iter()
        .flat_map(|(item, actions)| actions.iter().enumerate().map(move |(i, a)| (item, i, *a)))
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
        if node.is_some_and(|n| n.above) {
            // Never counted: restic creates it, or keeps the real folder that is there and sets
            // the snapshot's mode and time on it.
            let at = input.target.join(item.trim_start_matches('/'));
            if std::fs::symlink_metadata(&at).is_ok_and(|m| m.is_dir()) {
                note(RestoreNoteCode::FolderMetadataReset, &mut notes);
            }
            continue;
        }
        let restored = action == DryRunAction::Restored;
        let rank = input.chosen[item][..index]
            .iter()
            .filter(|a| (**a == DryRunAction::Restored) == restored)
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
        if found
            .entries
            .iter()
            .any(|(_, m)| m.file_type().is_symlink())
        {
            note(RestoreNoteCode::SymlinkInTarget, &mut notes);
        }
        if node.is_some_and(|n| n.is_dir) {
            // A folder is not counted, unless restic puts it in place of a file or a symbolic
            // link at the target (it does so even with `never`): that entry is overwritten.
            let replaces_file = restored && existing.as_ref().is_some_and(|m| !m.is_dir());
            // restic sets mode and mtime of a folder that exists, even with `never`.
            if existing.as_ref().is_some_and(std::fs::Metadata::is_dir) {
                note(RestoreNoteCode::FolderMetadataReset, &mut notes);
            }
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
        let outcome = match action {
            // The dry run looks through the link; the run writes into a new, real folder.
            _ if behind_link => RestoreOutcome::New,
            DryRunAction::Restored if existing.is_some() => RestoreOutcome::Overwritten,
            DryRunAction::Restored => RestoreOutcome::New,
            DryRunAction::Updated => RestoreOutcome::Overwritten,
            DryRunAction::Unchanged if input.overwrite == RestoreOverwrite::Always => {
                RestoreOutcome::Identical
            }
            DryRunAction::Unchanged => match input
                .always
                .and_then(|a| a.get(item.as_str()))
                .and_then(|actions| actions.get(index))
            {
                Some(a) if a.writes() => RestoreOutcome::Kept,
                _ => RestoreOutcome::Identical,
            },
        };
        let linked_in_snapshot = node
            .and_then(|n| n.inode)
            .is_some_and(|i| shared_inodes.contains(&i));
        // restic cannot put a file where the target has a folder with content: the run fails
        // and gives the folder the file's mode and time. It tries only where the dry run
        // writes the file, or for a file hard-linked in the snapshot (another name links to
        // it). Entries whose names restic shows alike cannot be told apart at the target: any
        // of them that is such a folder counts.
        let tried = action.writes() || linked_in_snapshot;
        if let Some((dir, _)) = found.entries.iter().find(|(p, m)| {
            tried
                && !behind_link
                && m.is_dir()
                && std::fs::read_dir(p).is_ok_and(|mut d| d.next().is_some())
        }) {
            blocked.push(dir.to_string_lossy().into_owned());
            continue;
        }
        let size = node.map_or(0, |n| n.size);
        if outcome == RestoreOutcome::Overwritten {
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
    // A selection of folders alone counts no file, but restic still creates the folders.
    if input.chosen.is_empty() {
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

/// The folders above the selection where the target has something else than a real folder: a
/// file, or a symbolic link of any kind. With `--include`, restic follows a link to a folder
/// there and writes into another folder, replaces a link or a file at the folder holding a
/// selected entry, and fails at a file, a dangling link or a link loop further up; the dry run
/// tells none of this apart.
pub fn blocked_folders_above(tree: &SnapshotTree, target: &Path) -> Vec<String> {
    let mut blocked: Vec<String> = tree
        .nodes
        .iter()
        .filter(|(_, nodes)| nodes.iter().any(|n| n.above))
        .map(|(path, _)| target.join(path.trim_start_matches('/')))
        .filter(|at| std::fs::symlink_metadata(at).is_ok_and(|m| !m.is_dir()))
        .map(|at| at.to_string_lossy().into_owned())
        .collect();
    blocked.sort();
    blocked
}

/// The folders above the selected entries of `plan` (`selection`) where the target has
/// something else than a real folder; see [`blocked_folders_above`]. Used right before the
/// run, without the walked tree: the selected entries are never below one another, so every
/// folder above one of them is a folder restic creates, not a selected one.
pub fn blocked_above_selection(
    plan: &RestorePlan,
    selection: &[String],
    target: &Path,
) -> Vec<String> {
    let mut tree = SnapshotTree::default();
    let selected = crate::restore::selected_paths(plan, selection);
    for path in selected.as_deref().unwrap_or_default() {
        tree.nodes.entry(path.clone()).or_default().push(TreeNode {
            is_dir: false,
            size: 0,
            inode: None,
            above: false,
        });
    }
    tree.add_folders_above(selected.as_deref());
    blocked_folders_above(&tree, target)
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

/// Walks what the restore of `plan` writes (`restic ls --json --recursive --no-lock`): the
/// selected entries of `selection` with everything below them, or the restored folder when the
/// whole folder is restored. Nothing else of the snapshot is read or held.
pub fn walk_folder(
    access: &SnapshotListAccess<'_>,
    plan: &RestorePlan,
    selection: &[String],
    cancel: Option<&CancelHandle>,
    limit_bytes: u64,
) -> Result<SnapshotTree, BackupRunError> {
    let mut args = vec![
        "ls".to_string(),
        "--no-lock".to_string(),
        "--recursive".to_string(),
        plan.snapshot_id.clone(),
    ];
    args.extend(walk_paths(plan, selection));
    let mut collector = SnapshotTree::collector(&plan.folder, limit_bytes);
    let res = run_streaming(access, args, cancel, &mut |line| collector.push_line(line))?;
    check_command_succeeded(&res)?;
    let mut tree = collector.finish();
    if !tree.too_large {
        tree.add_folders_above(crate::restore::selected_paths(plan, selection).as_deref());
    }
    Ok(tree)
}

/// The paths `restic ls` lists for the walk of `plan`: the selected entries when the plan
/// restores only them (`--include`), otherwise its folder.
pub fn walk_paths(plan: &RestorePlan, selection: &[String]) -> Vec<String> {
    if plan.includes.is_empty() || selection.is_empty() {
        vec![plan.folder.clone()]
    } else {
        selection.to_vec()
    }
}

/// One dry run of `plan` into `target` with `overwrite` (`--dry-run --no-lock -vv`); `None`
/// when what it reports would hold more than `limit_bytes`.
pub fn dry_run(
    access: &SnapshotListAccess<'_>,
    plan: &RestorePlan,
    target: &Path,
    overwrite: RestoreOverwrite,
    cancel: Option<&CancelHandle>,
    limit_bytes: u64,
) -> Result<Option<(DryRunActions, u64)>, BackupRunError> {
    let args = restore_args(plan, target, overwrite, RestoreCall::DryRun);
    let mut actions = DryRunActions::new();
    let mut held: u64 = 0;
    let mut too_large = false;
    let res = run_streaming(access, args, cancel, &mut |line| {
        if too_large {
            return;
        }
        if let RestoreLine::Item(item) = parse_restore_line(line) {
            held += item.item.len() as u64 + ENTRY_OVERHEAD_BYTES;
            if held > limit_bytes {
                too_large = true;
                actions = DryRunActions::new();
                return;
            }
            actions
                .entry(item.item)
                .or_default()
                .push(DryRunAction::parse(&item.action));
        }
    })?;
    if too_large {
        return Ok(None);
    }
    // Where the target has a file and the snapshot a folder with content, the dry run cannot
    // look below the file and fails («not a directory»); the run itself puts the folder in
    // place of the file, so the entries below it are new.
    if let Some(items) = below_replaced_files(&res) {
        for item in items {
            actions.insert(item, vec![DryRunAction::Restored]);
        }
        return Ok(Some((actions, held)));
    }
    check_restore_succeeded(&res, plan)?;
    Ok(Some((actions, held)))
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

/// What the dry runs of a preview need.
pub struct PreviewRun<'a> {
    pub plan: &'a RestorePlan,
    /// The restored folder from [`walk_folder`].
    pub tree: &'a SnapshotTree,
    /// The checked target folder.
    pub target: &'a Path,
    pub overwrite: RestoreOverwrite,
    /// Whether the snapshot belongs to another user than the one restoring.
    pub foreign_owner: bool,
    pub limit_bytes: u64,
}

/// Runs the dry runs of a preview and classifies them.
pub fn execute_preview(
    access: &SnapshotListAccess<'_>,
    run: &PreviewRun<'_>,
    cancel: Option<&CancelHandle>,
) -> Result<PreviewClassification, BackupRunError> {
    let PreviewRun {
        plan,
        tree,
        target,
        overwrite,
        foreign_owner,
        limit_bytes,
    } = *run;
    // Checked before any dry run: a link loop there makes the dry run itself fail.
    if let Some(first) = blocked_folders_above(tree, target).first() {
        return Err(BackupRunError::InvalidTarget {
            path: first.clone(),
            reason: "not a real folder where restic needs one above the selection".to_string(),
        });
    }
    // The walk and both dry runs together hold at most the limit of one query; beyond it the
    // preview stops with a list that is too large instead of growing without bound.
    let files = tree.files;
    if tree.too_large {
        return Ok(PreviewClassification::too_large(files));
    }
    let budget = limit_bytes.saturating_sub(tree.held_bytes);
    let Some((chosen, held)) = dry_run(access, plan, target, overwrite, cancel, budget)? else {
        return Ok(PreviewClassification::too_large(files));
    };
    let always = if overwrite == RestoreOverwrite::Always {
        None
    } else {
        let budget = budget.saturating_sub(held);
        let always = dry_run(
            access,
            plan,
            target,
            RestoreOverwrite::Always,
            cancel,
            budget,
        )?;
        let Some((always, _)) = always else {
            return Ok(PreviewClassification::too_large(files));
        };
        Some(always)
    };
    let classification = classify_preview(&ClassifyInput {
        tree,
        chosen: &chosen,
        always: always.as_ref(),
        overwrite,
        target,
        foreign_owner,
        limit_bytes,
    });
    // Such a target would fail and lose its folder's mode: the restore is refused up front.
    if let Some(first) = classification.blocked.first() {
        return Err(BackupRunError::InvalidTarget {
            path: first.clone(),
            reason: "a folder with content where the snapshot has a file".to_string(),
        });
    }
    Ok(classification)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(paths: &[&str]) -> SnapshotTree {
        let mut tree = SnapshotTree::default();
        for p in paths {
            tree.nodes
                .entry((*p).to_string())
                .or_default()
                .push(TreeNode {
                    is_dir: false,
                    size: 0,
                    inode: None,
                    above: false,
                });
        }
        tree
    }

    #[test]
    fn walk_reads_only_the_selected_entries() {
        let plan = |folder: &str, includes: &[&str]| RestorePlan {
            snapshot_id: "abc".to_string(),
            folder: folder.to_string(),
            includes: includes.iter().map(|i| (*i).to_string()).collect(),
        };
        let sel = ["/h/u/a.txt".to_string(), "/h/u/docs".to_string()];
        assert_eq!(
            walk_paths(
                &plan("/h/u", &["--include=/a.txt", "--include=/docs"]),
                &sel
            ),
            sel.to_vec(),
            "the selected entries, not their common folder"
        );
        assert_eq!(
            walk_paths(&plan("/h/u/docs", &[]), &["/h/u/docs".to_string()]),
            vec!["/h/u/docs".to_string()],
            "a selected folder restored as a whole"
        );
        assert_eq!(
            walk_paths(&plan("/h/u", &[]), &[]),
            vec!["/h/u".to_string()],
            "nothing selected: the whole folder"
        );
    }

    #[test]
    fn folders_above_selected_entries_are_folders() {
        let mut t = tree(&["/a.txt", "/docs/deep/y.txt"]);
        t.add_folders_above(Some(&[
            "/a.txt".to_string(),
            "/docs/deep/y.txt".to_string(),
        ]));
        let is_dir = |p: &str| t.nodes.get(p).map(|n| n[0].is_dir);
        assert_eq!(is_dir("/docs"), Some(true), "the folder in between");
        assert_eq!(
            is_dir("/docs/deep"),
            Some(true),
            "the folder holding the file"
        );
        assert_eq!(
            is_dir("/docs/deep/y.txt"),
            Some(false),
            "the file stays a file"
        );
        assert_eq!(is_dir("/a.txt"), Some(false), "a file at the top");
        assert_eq!(t.nodes.len(), 4, "nothing else is added");
    }

    #[test]
    fn folders_above_a_missing_entry_write_nothing_below_it() {
        let selected = ["/docs/deep/y.txt".to_string(), "/docs/ghost/q".to_string()];
        let mut t = tree(&["/docs/deep/y.txt"]);
        t.add_folders_above(Some(&selected));
        assert!(t.nodes.contains_key("/docs/deep"), "above a found entry");
        assert!(
            t.nodes.contains_key("/docs/ghost"),
            "checked like in the run"
        );
        assert!(
            !t.writes_below(Some(&selected), "/docs/ghost"),
            "a folder above the selection is no write below a repository there"
        );
    }

    #[test]
    fn folders_above_the_selection_must_be_real_folders() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let t = tmp.path();
        std::fs::create_dir_all(t.join("real/sub")).expect("real folders");
        std::fs::write(t.join("file"), "x").expect("file");
        std::fs::create_dir(t.join("other")).expect("other");
        std::os::unix::fs::symlink(t.join("other"), t.join("link")).expect("link");
        std::os::unix::fs::symlink(t.join("nowhere"), t.join("dangling")).expect("dangling");
        let plan = RestorePlan {
            snapshot_id: "abc".to_string(),
            folder: "/s".to_string(),
            includes: vec!["--include=/x".to_string(), "--include=/y".to_string()],
        };
        let blocked = |entries: &[&str]| {
            let selection: Vec<String> = entries.iter().map(|e| format!("/s{e}")).collect();
            blocked_above_selection(&plan, &selection, t)
        };
        let at = |p: &str| vec![t.join(p).to_string_lossy().into_owned()];
        assert!(blocked(&["/a", "/real/sub/f"]).is_empty(), "real folders");
        assert!(
            blocked(&["/a", "/missing/f"]).is_empty(),
            "a folder restic creates"
        );
        assert_eq!(blocked(&["/a", "/file/f"]), at("file"), "a file");
        assert_eq!(
            blocked(&["/a", "/link/f"]),
            at("link"),
            "a link to a folder"
        );
        assert_eq!(
            blocked(&["/a", "/dangling/f"]),
            at("dangling"),
            "a dangling link"
        );
    }

    #[test]
    fn only_selected_entries_below_a_folder_count() {
        let t = tree(&["/a.txt", "/vault", "/vault/config", "/vaulty/x"]);
        assert!(t.writes_below(None, "/vault"), "everything selected");
        let outside = ["/a.txt".to_string(), "/vaulty".to_string()];
        assert!(
            !t.writes_below(Some(&outside), "/vault"),
            "selection outside"
        );
        let inside = ["/vault/config".to_string()];
        assert!(t.writes_below(Some(&inside), "/vault"), "entry inside");
        assert!(
            !tree(&["/a.txt"]).writes_below(None, "/vault"),
            "no entries there"
        );
    }
}
