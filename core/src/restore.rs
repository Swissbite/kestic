// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Restoring files from a snapshot with `restic restore`: the selection becomes one folder of
//! the snapshot (`<snapshot>:<folder>`) plus one escaped `--include` per selected entry, and
//! every call names `--overwrite` explicitly. This module is the only place that builds the
//! arguments of `restic restore`.

use crate::backup::{BackupRunError, SnapshotListAccess, check_command_succeeded};
use crate::destination::{DEFAULT_CONNECT_TIMEOUT, preflight_check_destination};
use crate::restic::{
    CancelHandle, ResticCommandSpec, ResticExecutionResult, ResticStatusMessage,
    run_restic_streaming,
};
use crate::snapshot::ResticSnapshot;
use crate::snapshot_ls::{SnapshotPathError, normalize_snapshot_path};
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
pub fn overwrite_arg(mode: RestoreOverwrite) -> &'static str {
    match mode {
        RestoreOverwrite::Never => "never",
        RestoreOverwrite::IfNewer => "if-newer",
        RestoreOverwrite::IfChanged => "if-changed",
        RestoreOverwrite::Always => "always",
    }
}

/// Escapes the glob characters `\`, `*`, `?` and `[` of `name` with `\`, so restic's
/// `--include` matches the name literally.
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
    Some(match path[..cut].rfind('/') {
        Some(i) if i > 0 => path[..i].to_string(),
        _ => "/".to_string(),
    })
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

fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

fn join_segments(segments: &[&str]) -> String {
    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

/// The parent folder of an absolute, normalised path (`/` for a top-level entry).
pub fn parent_folder(path: &str) -> String {
    let segs = segments(path);
    join_segments(&segs[..segs.len().saturating_sub(1)])
}

/// The deepest folder that contains every path of `paths` (normalised, none below another);
/// for several paths that is always a folder above each of them.
pub fn common_folder(paths: &[String]) -> String {
    let mut common: Option<Vec<&str>> = None;
    for path in paths {
        let segs = segments(path);
        common = Some(match common {
            None => segs[..segs.len().saturating_sub(1)].to_vec(),
            Some(prev) => prev
                .iter()
                .zip(&segs)
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| *a)
                .collect(),
        });
    }
    join_segments(&common.unwrap_or_default())
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
            let folder = parent_folder(single);
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

/// The restic call of `args`. Only the restore run itself is a run with low CPU and IO
/// priority; the queries and dry runs of the preview answer the waiting dialog.
fn spec_for(
    access: &SnapshotListAccess<'_>,
    args: Vec<String>,
    priority: Priority,
) -> ResticCommandSpec {
    let mut spec = ResticCommandSpec::for_destination(
        &access.resolved.binary,
        access.kestic_binary,
        access.destination,
        args,
    );
    spec.low_priority = priority == Priority::Low;
    spec
}

/// CPU and IO priority of a restic call of a restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Priority {
    /// A query or a dry run the user waits for.
    Normal,
    /// The restore run.
    Low,
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
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            return;
        };
        match value.get("message_type").and_then(|v| v.as_str()) {
            Some("snapshot") if self.snapshot.is_none() => {
                self.snapshot = serde_json::from_value(value).ok();
            }
            Some("node") if self.kind.is_none() => {
                let path = value.get("path").and_then(|v| v.as_str());
                if path == Some(self.path.as_str()) {
                    let is_dir = value.get("type").and_then(|v| v.as_str()) == Some("dir");
                    self.kind = Some(if is_dir {
                        EntryKind::Dir
                    } else {
                        EntryKind::Other
                    });
                } else {
                    self.kind = Some(EntryKind::Missing);
                }
            }
            _ => {}
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
    let args = vec![
        "ls".to_string(),
        "--no-lock".to_string(),
        snapshot_id.to_string(),
        path.to_string(),
    ];
    let mut collector = ProbeCollector::new(path);
    let res = run_streaming(access, args, cancel, &mut |line| collector.push_line(line))?;
    check_command_succeeded(&res)?;
    let (snapshot, kind) = collector.finish();
    let snapshot = snapshot.ok_or_else(|| {
        BackupRunError::Io(std::io::Error::other("restic ls printed no snapshot"))
    })?;
    Ok((snapshot, kind))
}

/// Runs restic with `args`, handing every stdout line unredacted to `on_line`; a cancelled call
/// is `Cancelled`.
pub(crate) fn run_streaming(
    access: &SnapshotListAccess<'_>,
    args: Vec<String>,
    cancel: Option<&CancelHandle>,
    on_line: &mut dyn FnMut(&str),
) -> Result<ResticExecutionResult, BackupRunError> {
    run_streaming_at(access, args, Priority::Normal, cancel, on_line)
}

fn run_streaming_at(
    access: &SnapshotListAccess<'_>,
    args: Vec<String>,
    priority: Priority,
    cancel: Option<&CancelHandle>,
    on_line: &mut dyn FnMut(&str),
) -> Result<ResticExecutionResult, BackupRunError> {
    // Cancelled before restic started (e.g. while the destination was checked): no process.
    if cancel.is_some_and(CancelHandle::is_cancelled) {
        return Err(BackupRunError::Cancelled);
    }
    let spec = spec_for(access, args, priority);
    let res = run_restic_streaming(
        &spec,
        access.destination,
        Some(access.secret_store),
        access.clock,
        cancel,
        access.extra_env,
        on_line,
    )?;
    if res.cancelled {
        return Err(BackupRunError::Cancelled);
    }
    Ok(res)
}

/// restic's error for a folder of `<snapshot>:<folder>` that the snapshot lacks
/// («path … not found»).
pub fn is_path_not_found(res: &ResticExecutionResult) -> bool {
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

/// Runs the restore of `plan` into `target` and reports progress lines to `on_status`. Folders
/// are told apart from files on disk after restic wrote them.
pub fn execute_restore(
    access: &SnapshotListAccess<'_>,
    plan: &RestorePlan,
    target: &Path,
    overwrite: RestoreOverwrite,
    cancel: Option<&CancelHandle>,
    on_status: &mut dyn FnMut(&ResticStatusMessage),
) -> Result<(RestoreRunResult, ResticExecutionResult), BackupRunError> {
    preflight_check_destination(access.destination, DEFAULT_CONNECT_TIMEOUT)?;
    let args = restore_args(plan, target, overwrite, RestoreCall::Run);
    let mut items: Vec<RestoreItem> = Vec::new();
    let mut summary: Option<RestoreSummary> = None;
    let res =
        run_streaming_at(
            access,
            args,
            Priority::Low,
            cancel,
            &mut |line| match parse_restore_line(line) {
                RestoreLine::Item(item) => items.push(item),
                RestoreLine::Status(status) => on_status(&status),
                RestoreLine::Summary(s) => summary = Some(s),
                RestoreLine::Other => {}
            },
        )?;
    check_restore_succeeded(&res, plan)?;
    let mut result = RestoreRunResult {
        nothing_matched: summary.as_ref().is_none_or(RestoreSummary::is_empty),
        bytes_restored: summary.as_ref().and_then(|s| s.bytes_restored).unwrap_or(0),
        ..RestoreRunResult::default()
    };
    for item in items {
        let written = target.join(item.item.trim_start_matches('/'));
        if std::fs::symlink_metadata(&written).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        match item.action.as_str() {
            "restored" => result.files_new += 1,
            "updated" => result.files_overwritten += 1,
            _ => {}
        }
    }
    Ok((result, res))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_string()).collect()
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
