// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Browsing a snapshot one folder at a time with `restic ls --json <snapshot> <folder>`: always
//! with a folder argument, so restic lists one level and never the whole tree.

use crate::backup::{BackupRunError, SnapshotListAccess, check_command_succeeded, run_streaming};
use crate::destination::{DEFAULT_CONNECT_TIMEOUT, preflight_check_destination};
use crate::query_list::{HeldResult, ListBuilder, ListEntry, QueryList};
use crate::restic::CancelHandle;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One entry of a folder in a snapshot, as `restic ls --json` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsEntry {
    pub name: String,
    /// Absolute path in the snapshot.
    pub path: String,
    /// `file`, `dir`, `symlink` or another restic node type, unchanged.
    pub entry_type: String,
    /// Size in bytes; restic reports none for folders and links.
    pub size: Option<u64>,
    /// Modification time as restic reports it (RFC 3339).
    pub mtime: Option<String>,
    pub uid: Option<u32>,
    /// Number of hard links, when restic reports it.
    pub hard_links: Option<u64>,
}

impl LsEntry {
    pub fn is_dir(&self) -> bool {
        self.entry_type == "dir"
    }
}

impl ListEntry for LsEntry {
    fn path(&self) -> &str {
        &self.path
    }

    /// Within one folder the filter searches the names.
    fn filter_text(&self) -> &str {
        &self.name
    }
}

/// The content of one folder of a snapshot.
pub struct LsListing {
    /// The listed folder (normalised).
    pub dir: String,
    /// Whether the folder exists in the snapshot; restic lists nothing for a missing one.
    pub dir_exists: bool,
    pub entries: QueryList<LsEntry>,
}

impl HeldResult for LsListing {
    fn held_bytes(&self) -> u64 {
        self.entries.held_bytes()
    }
}

/// Why a snapshot path or ID from the GUI is refused before restic runs.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SnapshotPathError {
    #[error("path is empty")]
    Empty,
    #[error("path is not absolute: {0}")]
    Relative(String),
    #[error("path contains a '..' segment: {0}")]
    ParentSegment(String),
    #[error("path contains a NUL character")]
    Nul,
    #[error("not a full snapshot ID: {0}")]
    SnapshotId(String),
}

/// Normalises an absolute path in a snapshot: repeated `/`, `.` segments and a trailing `/`
/// are removed. Empty, relative paths, paths with a `..` segment or a NUL are refused.
pub fn normalize_snapshot_path(path: &str) -> Result<String, SnapshotPathError> {
    if path.is_empty() {
        return Err(SnapshotPathError::Empty);
    }
    if path.contains('\0') {
        return Err(SnapshotPathError::Nul);
    }
    if !path.starts_with('/') {
        return Err(SnapshotPathError::Relative(path.to_string()));
    }
    let mut out = String::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => return Err(SnapshotPathError::ParentSegment(path.to_string())),
            name => {
                out.push('/');
                out.push_str(name);
            }
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    Ok(out)
}

/// Checks that `id` is a full snapshot ID (64 hexadecimal digits); Kestic never passes a short
/// ID or `latest`, which can be ambiguous.
pub fn validate_snapshot_id(id: &str) -> Result<(), SnapshotPathError> {
    if id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(SnapshotPathError::SnapshotId(id.to_string()))
    }
}

/// Arguments of `restic ls` for one level of `dir` in `snapshot_id` (`--json` comes from the
/// command spec).
pub fn ls_args(snapshot_id: &str, dir: &str) -> Vec<String> {
    vec![
        "ls".to_string(),
        "--no-lock".to_string(),
        snapshot_id.to_string(),
        dir.to_string(),
    ]
}

/// The folder that contains `path` (`/` for an entry at the top).
pub fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

/// The deepest folder that contains or is each of `paths`, `/` without paths.
pub(crate) fn common_dir<'a>(paths: impl IntoIterator<Item = &'a str>) -> String {
    let mut common: Option<Vec<&str>> = None;
    for path in paths {
        let segments = path.split('/').filter(|s| !s.is_empty());
        common = Some(match common {
            None => segments.collect(),
            Some(prev) => prev
                .into_iter()
                .zip(segments)
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a)
                .collect(),
        });
    }
    let segments = common.unwrap_or_default();
    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

/// The folder a snapshot opens in: the deepest folder that contains all its backed-up paths
/// (the backed-up folder itself for a snapshot of one folder), `/` without paths.
pub fn start_dir(paths: &[String]) -> String {
    common_dir(paths.iter().map(String::as_str))
}

/// The folders from `/` down to `dir`, each as (name, path); `/` has an empty name.
pub fn path_segments(dir: &str) -> Vec<(String, String)> {
    let mut out = vec![(String::new(), "/".to_string())];
    let mut path = String::new();
    for name in dir.split('/').filter(|s| !s.is_empty()) {
        path.push('/');
        path.push_str(name);
        out.push((name.to_string(), path.clone()));
    }
    out
}

/// One line of `restic ls --json`: a node (an entry of the snapshot) or another message such
/// as the snapshot. Unknown fields are ignored.
#[derive(Deserialize)]
pub(crate) struct RawLsNode {
    #[serde(default)]
    pub message_type: Option<String>,
    #[serde(default)]
    struct_type: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub node_type: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub mtime: Option<String>,
    #[serde(default)]
    pub uid: Option<u32>,
    #[serde(default)]
    pub links: Option<u64>,
    #[serde(default)]
    pub inode: Option<u64>,
}

impl RawLsNode {
    /// Parses one stdout line; `None` for a line that is no JSON object of this shape.
    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line.trim()).ok()
    }

    /// Whether the line is a node; restic names the kind of a line in `message_type` and in
    /// `struct_type`, either one is enough.
    pub fn is_node(&self) -> bool {
        self.message_type.as_deref() == Some("node") || self.struct_type.as_deref() == Some("node")
    }

    pub fn is_dir(&self) -> bool {
        self.node_type == "dir"
    }
}

/// Turns the output lines of `restic ls --json <snapshot> <dir>` into the folder's entries.
/// restic prints the snapshot first, then the folder itself as the first node (except for `/`),
/// then its entries; for a missing folder it prints no node at all. Unknown lines and fields
/// are ignored.
struct LsCollector {
    dir: String,
    seen_first_node: bool,
    dir_exists: bool,
    builder: ListBuilder<LsEntry>,
}

impl LsCollector {
    /// Collects the entries of `dir` (normalised); the list is dropped once it exceeds
    /// `limit_bytes`.
    fn new(dir: &str, limit_bytes: u64) -> Self {
        Self {
            dir: dir.to_string(),
            seen_first_node: false,
            dir_exists: dir == "/",
            builder: ListBuilder::new(limit_bytes),
        }
    }

    fn push_line(&mut self, line: &str) {
        let Some(node) = RawLsNode::parse(line).filter(RawLsNode::is_node) else {
            return;
        };
        if !self.seen_first_node {
            self.seen_first_node = true;
            if self.dir != "/" && node.path == self.dir {
                self.dir_exists = node.is_dir();
                return;
            }
        }
        // restic's paths are absolute; a node without one is no entry of the folder.
        if self.dir_exists && node.path.starts_with('/') && parent_dir(&node.path) == self.dir {
            self.builder.push(LsEntry {
                name: node.name,
                path: node.path,
                entry_type: node.node_type,
                size: node.size,
                mtime: node.mtime,
                uid: node.uid,
                hard_links: node.links,
            });
        }
    }

    fn finish(self) -> LsListing {
        LsListing {
            dir: self.dir,
            dir_exists: self.dir_exists,
            entries: self.builder.finish(),
        }
    }
}

/// Lists one level of `dir` in snapshot `snapshot_id` (`restic ls --no-lock`), after the
/// reachability check of the destination. `dir` must be normalised and `snapshot_id` full.
pub fn execute_ls(
    access: &SnapshotListAccess<'_>,
    snapshot_id: &str,
    dir: &str,
    cancel: Option<&CancelHandle>,
    limit_bytes: u64,
) -> Result<LsListing, BackupRunError> {
    preflight_check_destination(access.destination, DEFAULT_CONNECT_TIMEOUT)?;
    let mut collector = LsCollector::new(dir, limit_bytes);
    let res = run_streaming(access, ls_args(snapshot_id, dir), cancel, &mut |line| {
        collector.push_line(line)
    })?;
    check_command_succeeded(&res)?;
    Ok(collector.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_paths_are_normalised_or_refused() {
        assert_eq!(
            normalize_snapshot_path("/a//b/./c/").as_deref(),
            Ok("/a/b/c")
        );
        assert_eq!(normalize_snapshot_path("/").as_deref(), Ok("/"));
        assert_eq!(normalize_snapshot_path(""), Err(SnapshotPathError::Empty));
        assert!(matches!(
            normalize_snapshot_path("a/b"),
            Err(SnapshotPathError::Relative(_))
        ));
        assert!(matches!(
            normalize_snapshot_path("/a/../b"),
            Err(SnapshotPathError::ParentSegment(_))
        ));
        assert_eq!(
            normalize_snapshot_path("/a\0b"),
            Err(SnapshotPathError::Nul)
        );
    }

    #[test]
    fn snapshot_ids_must_be_full() {
        assert!(validate_snapshot_id(&"ab12".repeat(16)).is_ok());
        assert!(validate_snapshot_id("ab12cd34").is_err());
        assert!(validate_snapshot_id("latest").is_err());
        assert!(validate_snapshot_id(&format!("-{}", "a".repeat(63))).is_err());
    }

    #[test]
    fn start_dir_is_the_common_folder_of_the_paths() {
        assert_eq!(start_dir(&["/home/u/docs".into()]), "/home/u/docs");
        assert_eq!(
            start_dir(&["/home/u/docs".into(), "/home/u/pics".into()]),
            "/home/u"
        );
        assert_eq!(start_dir(&["/home/u".into(), "/media/x".into()]), "/");
        assert_eq!(start_dir(&[]), "/");
    }

    #[test]
    fn path_segments_run_from_the_top() {
        assert_eq!(
            path_segments("/home/u"),
            vec![
                (String::new(), "/".to_string()),
                ("home".to_string(), "/home".to_string()),
                ("u".to_string(), "/home/u".to_string()),
            ]
        );
        assert_eq!(path_segments("/"), vec![(String::new(), "/".to_string())]);
    }

    fn node(name: &str, ty: &str, path: &str) -> String {
        format!(
            r#"{{"name":"{name}","type":"{ty}","path":"{path}","size":3,"mtime":"2026-11-03T10:00:00Z","uid":1000,"message_type":"node","struct_type":"node","future":1}}"#
        )
    }

    #[test]
    fn collector_skips_the_folder_node_and_reads_entries() {
        let mut c = LsCollector::new("/src", u64::MAX);
        c.push_line(r#"{"id":"x","message_type":"snapshot","struct_type":"snapshot"}"#);
        c.push_line(&node("src", "dir", "/src"));
        c.push_line(&node("b.txt", "file", "/src/b.txt"));
        c.push_line(&node("a", "dir", "/src/a"));
        c.push_line("not json");
        let mut listing = c.finish();
        assert!(listing.dir_exists);
        let page = listing.entries.page(&Default::default());
        let names: Vec<&str> = page.items.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a", "b.txt"]);
        assert_eq!(page.items[1].size, Some(3));
    }

    #[test]
    fn collector_reports_a_missing_folder_and_a_file_argument() {
        let mut missing = LsCollector::new("/nope", u64::MAX);
        missing.push_line(r#"{"id":"x","message_type":"snapshot"}"#);
        assert!(!missing.finish().dir_exists);

        let mut file = LsCollector::new("/src/a.txt", u64::MAX);
        file.push_line(&node("a.txt", "file", "/src/a.txt"));
        let listing = file.finish();
        assert!(!listing.dir_exists);
        assert_eq!(listing.entries.total(), 0);
    }

    #[test]
    fn collector_lists_the_top_folder_without_a_folder_node() {
        let mut c = LsCollector::new("/", u64::MAX);
        c.push_line(&node("tmp", "dir", "/tmp"));
        let listing = c.finish();
        assert!(listing.dir_exists);
        assert_eq!(listing.entries.total(), 1);
    }
}
