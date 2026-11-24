// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Restoring files from a snapshot with `restic restore`: the selection becomes one folder of
//! the snapshot (`<snapshot>:<folder>`) plus one escaped `--include` per selected entry, and
//! every call names `--overwrite` explicitly. This module is the only place that builds the
//! arguments of `restic restore`.

use crate::backup::{
    BackupRunError, Priority, SnapshotListAccess, check_command_succeeded, run_streaming,
    run_streaming_to_end,
};
use crate::destination::{DEFAULT_CONNECT_TIMEOUT, preflight_check_destination};
use crate::restic::{CancelHandle, ResticExecutionResult, ResticStatusMessage};
use crate::snapshot::ResticSnapshot;
use crate::snapshot_ls::{
    RawLsNode, SnapshotPathError, common_dir, ls_args, normalize_snapshot_path, parent_dir,
    start_dir,
};
use kestic_i18n::codes::RestoreOverwrite;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Where a restore writes: a new (or chosen) folder, or the original location of the
/// selection (its common folder, possibly mapped to the user's home).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RestoreTarget {
    NewFolder { dir: String },
    Original { dir: String },
}

impl RestoreTarget {
    pub fn dir(&self) -> &str {
        match self {
            Self::NewFolder { dir } | Self::Original { dir } => dir,
        }
    }

    pub fn is_original(&self) -> bool {
        matches!(self, Self::Original { .. })
    }
}

/// The value restic expects for `--overwrite`.
fn overwrite_arg(mode: RestoreOverwrite) -> &'static str {
    match mode {
        RestoreOverwrite::Never => "never",
        RestoreOverwrite::IfNewer => "if-newer",
        RestoreOverwrite::IfChanged => "if-changed",
        RestoreOverwrite::Always => "always",
    }
}

/// Escapes the glob characters `\`, `*`, `?` and `[` of `name` with `\`, so a restic pattern
/// (`--include`, `restic find`) matches the name literally.
pub fn escape_glob(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '\\' | '*' | '?' | '[') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// restic shows the bytes of a name that are not UTF-8 as U+FFFD, and `--include` cannot match
/// such a name. For a path with U+FFFD in it, the nearest folder above it without one (`/` at
/// the top); `None` for a path restic can match.
pub fn folder_instead_of_unmatchable(path: &str) -> Option<String> {
    let cut = path.find('\u{FFFD}')?;
    Some(parent_dir(&path[..cut]).to_string())
}

/// `selection` with every path restic cannot match replaced by its folder
/// ([`folder_instead_of_unmatchable`]), without duplicates, in the original order.
pub fn selection_with_folders_instead(selection: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(selection.len());
    for path in selection {
        let entry = folder_instead_of_unmatchable(path).unwrap_or_else(|| path.clone());
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    out
}

/// Normalises the selected snapshot paths, sorts them and drops duplicates and entries that lie
/// below another selected entry. A selection containing `/` is the whole snapshot (empty).
pub fn normalize_selection(selection: &[String]) -> Result<Vec<String>, SnapshotPathError> {
    let mut paths = selection
        .iter()
        .map(|p| normalize_snapshot_path(p))
        .collect::<Result<Vec<_>, _>>()?;
    if paths.iter().any(|p| p == "/") {
        return Ok(Vec::new());
    }
    paths.sort();
    paths.dedup();
    let all: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
    let below_another = |path: &str| {
        let mut rest = path;
        while let Some(i) = rest.rfind('/').filter(|i| *i > 0) {
            rest = &rest[..i];
            if all.contains(rest) {
                return true;
            }
        }
        false
    };
    Ok(paths
        .iter()
        .filter(|p| !below_another(p))
        .cloned()
        .collect())
}

/// Whether `selection` (absolute snapshot paths) restores the same files as the whole snapshot,
/// whose restore runs from its top folder `top` ([`resolve_plan`]): each of the `entries`
/// entries of `top` is selected and nothing outside `top`. Entries below a selected one change
/// nothing; `top` itself selected from the folder above is not counted as its entries.
pub fn selection_is_whole_folder(selection: &[String], top: &str, entries: u64) -> bool {
    let inside = |path: &str| {
        path != top
            && (top == "/"
                || path
                    .strip_prefix(top)
                    .is_some_and(|rest| rest.starts_with('/')))
    };
    if entries == 0 || !selection.iter().all(|p| inside(p)) {
        return false;
    }
    let direct: std::collections::HashSet<&str> = selection
        .iter()
        .map(String::as_str)
        .filter(|p| parent_dir(p) == top)
        .collect();
    direct.len() as u64 == entries
}

/// The deepest folder that contains every path of `paths` (normalised, none below another);
/// for several paths that is always a folder above each of them.
fn common_folder(paths: &[String]) -> String {
    let Some((first, rest)) = paths.split_first() else {
        return "/".to_string();
    };
    common_dir(std::iter::once(parent_dir(first)).chain(rest.iter().map(String::as_str)))
}

/// Most bytes the `--include` arguments of one restore may take together, so the call stays
/// well below the system's limit for arguments.
pub const MAX_INCLUDE_BYTES: usize = 512 * 1024;

/// For a selection whose `--include` arguments could exceed [`MAX_INCLUDE_BYTES`], the folder
/// to restore instead (the folder holding all of it); `None` for a selection that fits. Counted
/// with the absolute paths, which are never shorter than the arguments restic gets.
pub fn folder_instead_of_large(selection: &[String]) -> Option<String> {
    let bytes: usize = selection
        .iter()
        .map(|p| "--include=".len() + escape_glob(p).len())
        .sum();
    (bytes > MAX_INCLUDE_BYTES).then(|| common_folder(selection))
}

/// The path restic needs to know first: the type of a single selected entry decides whether it
/// is its own folder; for several entries (or none) their common folder.
pub fn probe_path(selection: &[String]) -> String {
    match selection {
        [] => "/".to_string(),
        [single] => single.clone(),
        several => common_folder(several),
    }
}

/// What the snapshot holds at the probed path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    /// A file, link or other non-folder entry.
    Other,
    Missing,
}

/// A restore as restic runs it: `<snapshot_id>:<folder>` plus `--include` patterns (escaped,
/// relative to the folder).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePlan {
    pub snapshot_id: String,
    /// The common folder of the selection (a single selected folder is its own).
    pub folder: String,
    pub includes: Vec<String>,
}

/// Builds the plan of a normalised `selection` from the kind of the probed path
/// ([`probe_path`]). A probed path the snapshot lacks is `PathNotInSnapshot`.
pub fn plan_restore(
    snapshot_id: &str,
    selection: &[String],
    probed: EntryKind,
) -> Result<RestorePlan, BackupRunError> {
    let probe = probe_path(selection);
    if probed == EntryKind::Missing {
        return Err(BackupRunError::PathNotInSnapshot(probe));
    }
    let (folder, includes) = match selection {
        [] => ("/".to_string(), Vec::new()),
        [single] if probed == EntryKind::Dir => (single.clone(), Vec::new()),
        [single] => {
            let folder = parent_dir(single).to_string();
            let include = include_pattern(&folder, single);
            (folder, vec![include])
        }
        several => {
            let folder = common_folder(several);
            let includes = several
                .iter()
                .map(|p| include_pattern(&folder, p))
                .collect();
            (folder, includes)
        }
    };
    Ok(RestorePlan {
        snapshot_id: snapshot_id.to_string(),
        folder,
        includes,
    })
}

fn include_pattern(folder: &str, path: &str) -> String {
    format!("--include={}", escape_glob(relative_to(folder, path)))
}

/// `path` relative to `folder`, starting with `/` (the form restic's `--include` and the dry
/// run use).
fn relative_to<'a>(folder: &str, path: &'a str) -> &'a str {
    if folder == "/" {
        path
    } else {
        &path[folder.len()..]
    }
}

/// The selected entries of `plan` relative to its folder (each starting with `/`); `None` when
/// the whole folder is restored.
pub fn selected_paths(plan: &RestorePlan, selection: &[String]) -> Option<Vec<String>> {
    if plan.includes.is_empty() {
        return None;
    }
    Some(
        selection
            .iter()
            .map(|p| relative_to(&plan.folder, p).to_string())
            .collect(),
    )
}

/// The plan of a restore of `selection` from snapshot `snapshot_id`: reads the snapshot and
/// the kind of the probed path first. Without a selection the whole snapshot is restored from
/// its common folder, or from `/` when the tree does not hold the folder the snapshot names (a
/// snapshot of relative paths).
pub fn resolve_plan(
    access: &SnapshotListAccess<'_>,
    snapshot_id: &str,
    selection: &[String],
    cancel: Option<&CancelHandle>,
) -> Result<(ResticSnapshot, RestorePlan), BackupRunError> {
    let probe = probe_path(selection);
    let (snapshot, kind) = probe_snapshot_path(access, snapshot_id, &probe, cancel)?;
    let mut plan = plan_restore(snapshot_id, selection, kind)?;
    if selection.is_empty() {
        let common = start_dir(&snapshot.paths);
        if common != "/"
            && probe_snapshot_path(access, snapshot_id, &common, cancel)?.1 == EntryKind::Dir
        {
            plan.folder = common;
        }
    }
    Ok((snapshot, plan))
}

/// A dry run (preview: nothing is written, no lock) or the restore itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreCall {
    DryRun,
    Run,
}

/// The arguments of `restic restore` (`--json` comes from the command spec). `--overwrite` is
/// always explicit; `--sparse`, `--verify`, `--delete`, `--host`, `--path` and `--exclude` are
/// never passed.
pub fn restore_args(
    plan: &RestorePlan,
    target: &Path,
    overwrite: RestoreOverwrite,
    call: RestoreCall,
) -> Vec<String> {
    let mut args = vec![
        "restore".to_string(),
        format!("{}:{}", plan.snapshot_id, plan.folder),
        "--target".to_string(),
        target.to_string_lossy().into_owned(),
        "--overwrite".to_string(),
        overwrite_arg(overwrite).to_string(),
    ];
    args.extend(plan.includes.iter().cloned());
    if call == RestoreCall::DryRun {
        args.push("--dry-run".to_string());
        args.push("--no-lock".to_string());
    }
    // One line per entry, so a preview can classify each file and a run can count them.
    args.push("-vv".to_string());
    args
}

/// The snapshot and the kind of the probed path, from `restic ls --json <id> <path>`: restic
/// prints the snapshot first, then the path itself as the first node (except for `/`).
#[derive(Default)]
pub struct ProbeCollector {
    path: String,
    snapshot: Option<ResticSnapshot>,
    kind: Option<EntryKind>,
}

impl ProbeCollector {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            ..Self::default()
        }
    }

    pub fn push_line(&mut self, line: &str) {
        let line = line.trim();
        let Some(node) = RawLsNode::parse(line) else {
            return;
        };
        if node.is_node() {
            if self.kind.is_none() {
                self.kind = Some(if node.path != self.path {
                    EntryKind::Missing
                } else if node.is_dir() {
                    EntryKind::Dir
                } else {
                    EntryKind::Other
                });
            }
        } else if node.message_type.as_deref() == Some("snapshot") && self.snapshot.is_none() {
            self.snapshot = serde_json::from_str(line).ok();
        }
    }

    /// The snapshot (if restic printed it) and the kind of the path.
    pub fn finish(self) -> (Option<ResticSnapshot>, EntryKind) {
        let kind = if self.path == "/" {
            EntryKind::Dir
        } else {
            self.kind.unwrap_or(EntryKind::Missing)
        };
        (self.snapshot, kind)
    }
}

/// Reads the snapshot and the kind of `path` in it (`restic ls --no-lock`), after the
/// reachability check of the destination.
pub fn probe_snapshot_path(
    access: &SnapshotListAccess<'_>,
    snapshot_id: &str,
    path: &str,
    cancel: Option<&CancelHandle>,
) -> Result<(ResticSnapshot, EntryKind), BackupRunError> {
    preflight_check_destination(access.destination, DEFAULT_CONNECT_TIMEOUT)?;
    let mut collector = ProbeCollector::new(path);
    let res = run_streaming(access, ls_args(snapshot_id, path), cancel, &mut |line| {
        collector.push_line(line)
    })?;
    check_command_succeeded(&res)?;
    let (snapshot, kind) = collector.finish();
    let snapshot = snapshot.ok_or_else(|| {
        BackupRunError::Io(std::io::Error::other("restic ls printed no snapshot"))
    })?;
    Ok((snapshot, kind))
}

/// restic's error for a folder of `<snapshot>:<folder>` that the snapshot lacks
/// («path … not found»).
fn is_path_not_found(res: &ResticExecutionResult) -> bool {
    res.exit_code == 1
        && res
            .presentation
            .raw_detail
            .as_deref()
            .is_some_and(|d| d.contains("path ") && d.ends_with("not found"))
}

/// Fails with `PathNotInSnapshot` for restic's «path … not found», otherwise like any failed
/// restic call.
pub(crate) fn check_restore_succeeded(
    res: &ResticExecutionResult,
    plan: &RestorePlan,
) -> Result<(), BackupRunError> {
    if is_path_not_found(res) {
        return Err(BackupRunError::PathNotInSnapshot(plan.folder.clone()));
    }
    check_command_succeeded(res)
}

/// One `verbose_status` line of `restic restore -vv --json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RestoreItem {
    pub action: String,
    /// Path relative to the target, starting with `/`.
    pub item: String,
    #[serde(default)]
    pub size: u64,
}

/// The `summary` line of `restic restore --json`; restic leaves the numbers out when nothing
/// matched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RestoreSummary {
    pub total_files: Option<u64>,
    pub files_restored: Option<u64>,
    pub files_skipped: Option<u64>,
    pub total_bytes: Option<u64>,
    pub bytes_restored: Option<u64>,
}

impl RestoreSummary {
    /// restic printed a summary without any number: the selection matched nothing.
    pub fn is_empty(&self) -> bool {
        self.total_files.is_none()
            && self.files_restored.is_none()
            && self.files_skipped.is_none()
            && self.total_bytes.is_none()
            && self.bytes_restored.is_none()
    }
}

/// A line of `restic restore --json`.
#[derive(Debug, Clone, PartialEq)]
pub enum RestoreLine {
    Item(RestoreItem),
    Status(ResticStatusMessage),
    Summary(RestoreSummary),
    Other,
}

/// Parses one stdout line of `restic restore --json`; unknown lines are `Other`.
pub fn parse_restore_line(line: &str) -> RestoreLine {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return RestoreLine::Other;
    };
    match value.get("message_type").and_then(|v| v.as_str()) {
        Some("verbose_status") => {
            serde_json::from_value(value).map_or(RestoreLine::Other, RestoreLine::Item)
        }
        Some("status") => restore_status(&value).map_or(RestoreLine::Other, RestoreLine::Status),
        Some("summary") => {
            serde_json::from_value(value).map_or(RestoreLine::Other, RestoreLine::Summary)
        }
        _ => RestoreLine::Other,
    }
}

/// A restore `status` line in the shape of a backup's, so the same progress code reads it.
fn restore_status(value: &serde_json::Value) -> Option<ResticStatusMessage> {
    let num = |key: &str| value.get(key).and_then(serde_json::Value::as_u64);
    Some(ResticStatusMessage {
        percent_done: value.get("percent_done")?.as_f64()?,
        total_files: num("total_files"),
        files_done: num("files_restored"),
        total_bytes: num("total_bytes"),
        bytes_done: num("bytes_restored"),
        seconds_elapsed: num("seconds_elapsed"),
        seconds_remaining: num("seconds_remaining"),
        current_files: Vec::new(),
    })
}

/// What a finished restore did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreRunResult {
    /// Files (no folders) restic wrote that did not exist before.
    pub files_new: u64,
    /// Files (no folders) restic replaced.
    pub files_overwritten: u64,
    pub bytes_restored: u64,
    /// restic printed a summary without numbers: the selection matched nothing.
    pub nothing_matched: bool,
}

/// A restore restic ran: what it wrote, its call and output, and why it failed, if it did. A
/// restore that fails midway has still written files, so its counts stay.
#[derive(Debug)]
pub struct RestoreRun {
    pub result: RestoreRunResult,
    pub execution: ResticExecutionResult,
    pub failure: Option<BackupRunError>,
}

/// Runs the restore of `plan` into `target` and reports progress lines to `on_status`. Folders
/// are told apart from files on disk as restic reports them. `Err` only when restic did not
/// start; a cancelled run is a `failure` with what it wrote until then.
pub fn execute_restore(
    access: &SnapshotListAccess<'_>,
    plan: &RestorePlan,
    target: &Path,
    overwrite: RestoreOverwrite,
    cancel: Option<&CancelHandle>,
    on_status: &mut dyn FnMut(&ResticStatusMessage),
) -> Result<RestoreRun, BackupRunError> {
    preflight_check_destination(access.destination, DEFAULT_CONNECT_TIMEOUT)?;
    let args = restore_args(plan, target, overwrite, RestoreCall::Run);
    let mut result = RestoreRunResult::default();
    let mut summary: Option<RestoreSummary> = None;
    // Each item is counted when restic reports it (the file is written by then), so a restore
    // of millions of files holds no list of them.
    let mut count = |item: &RestoreItem| {
        // restic reports a file it could not write as restored too: only files that are there
        // count, folders never. A name restic shows with U+FFFD (bytes that are not UTF-8) is
        // looked up among the real names restic shows that way.
        let found = crate::restore_preview::find_at_target(target, &item.item);
        if !found.entries.iter().any(|(_, m)| !m.is_dir()) {
            return;
        }
        match item.action.as_str() {
            "restored" => result.files_new += 1,
            "updated" => result.files_overwritten += 1,
            _ => {}
        }
    };
    let res = run_streaming_to_end(access, args, Priority::Low, cancel, &mut |line| {
        match parse_restore_line(line) {
            RestoreLine::Item(item) => count(&item),
            RestoreLine::Status(status) => on_status(&status),
            RestoreLine::Summary(s) => summary = Some(s),
            RestoreLine::Other => {}
        }
    })?;
    // A cancelled restore keeps the counts of what it wrote until then.
    let failure = if res.cancelled {
        Some(BackupRunError::Cancelled)
    } else {
        check_restore_succeeded(&res, plan).err()
    };
    result.nothing_matched = summary.as_ref().is_none_or(RestoreSummary::is_empty);
    result.bytes_restored = summary.as_ref().and_then(|s| s.bytes_restored).unwrap_or(0);
    Ok(RestoreRun {
        result,
        execution: res,
        failure,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_string()).collect()
    }

    #[test]
    fn includes_keep_an_exclamation_mark() {
        let plan = plan_restore("id", &sel(&["/d/!x", "/d/a!b"]), EntryKind::Dir).expect("plan");
        assert_eq!(plan.folder, "/d");
        assert_eq!(plan.includes, [r"--include=/!x", r"--include=/a!b"]);
    }

    #[test]
    fn a_selection_over_the_argument_limit_offers_its_folder() {
        let fits: Vec<String> = (0..1000).map(|i| format!("/home/u/d/f{i:04}")).collect();
        assert_eq!(
            folder_instead_of_large(&fits),
            None,
            "a thousand short paths fit"
        );
        let long: Vec<String> = (0..40_000).map(|i| format!("/home/u/d/f{i:05}")).collect();
        assert_eq!(
            folder_instead_of_large(&long).as_deref(),
            Some("/home/u/d"),
            "beyond the limit: their folder"
        );
    }

    #[test]
    fn common_folder_lies_above_the_first_path() {
        assert_eq!(common_folder(&sel(&["/a/b"])), "/a");
        assert_eq!(common_folder(&sel(&["/a/b/c", "/a/b/d"])), "/a/b");
        assert_eq!(common_folder(&sel(&["/a/b", "/x"])), "/");
        assert_eq!(common_folder(&sel(&["/a"])), "/");
        assert_eq!(common_folder(&[]), "/");
    }

    #[test]
    fn selection_drops_nested_and_duplicate_entries() {
        assert_eq!(
            normalize_selection(&sel(&["/a/b/", "/a", "/a/b/c", "/x", "/x"])).expect("ok"),
            sel(&["/a", "/x"])
        );
        assert_eq!(
            normalize_selection(&sel(&["/ab", "/a"])).expect("ok"),
            sel(&["/a", "/ab"])
        );
        assert_eq!(
            normalize_selection(&sel(&["/a", "/a b", "/a/c"])).expect("ok"),
            sel(&["/a", "/a b"])
        );
        assert!(
            normalize_selection(&sel(&["/a", "/"]))
                .expect("ok")
                .is_empty()
        );
    }

    #[test]
    fn restore_lines_are_parsed() {
        assert_eq!(
            parse_restore_line(
                r#"{"message_type":"verbose_status","action":"restored","item":"/a","size":2}"#
            ),
            RestoreLine::Item(RestoreItem {
                action: "restored".into(),
                item: "/a".into(),
                size: 2
            })
        );
        let RestoreLine::Summary(s) = parse_restore_line(r#"{"message_type":"summary"}"#) else {
            panic!("summary");
        };
        assert!(s.is_empty());
        let RestoreLine::Status(st) = parse_restore_line(
            r#"{"message_type":"status","percent_done":0.5,"total_bytes":10,"bytes_restored":5}"#,
        ) else {
            panic!("status");
        };
        assert_eq!(st.bytes_done, Some(5));
        assert_eq!(parse_restore_line("noise"), RestoreLine::Other);
    }
}
