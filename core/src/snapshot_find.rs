// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Searching snapshots with `restic find --json`: in the snapshots of one backup, the
//! unassigned or the orphaned snapshots of a destination. The snapshots come from one
//! `restic snapshots` call sorted like the snapshot lists, so a metadata snapshot or one of
//! another backup is never searched; restic then searches exactly these snapshot IDs.

use crate::backup::{
    BackupRunError, SnapshotListAccess, check_command_succeeded, list_destination_snapshots,
};
use crate::query_list::{ENTRY_OVERHEAD_BYTES, HeldResult, ListBuilder, ListEntry, QueryList};
use crate::restic::{CancelHandle, ResticCommandSpec, run_restic_reading};
use crate::snapshot::ResticSnapshot;
use crate::snapshot_list::sort_destination_snapshots;
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::io::BufRead;
use thiserror::Error;

/// Which snapshots of a destination a search covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FindScope {
    /// The snapshots of a configured backup.
    Backup { backup_id: String },
    /// The snapshots without a Kestic tag of a configured backup.
    Unassigned,
    /// The snapshots of a backup that is neither configured nor described in the destination.
    Orphaned { backup_id: String },
}

impl FindScope {
    /// The backup the hits belong to; none for unassigned snapshots.
    pub fn backup_id(&self) -> Option<&str> {
        match self {
            Self::Backup { backup_id } | Self::Orphaned { backup_id } => Some(backup_id),
            Self::Unassigned => None,
        }
    }
}

/// One hit of a search: an entry of one snapshot whose path contains the search text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindHit {
    /// Full ID of the snapshot.
    pub snapshot_id: String,
    /// Time of the snapshot as restic reports it (RFC 3339).
    pub snapshot_time: String,
    /// Absolute path in the snapshot.
    pub path: String,
    /// `file`, `dir`, `symlink` or another restic node type, unchanged.
    pub entry_type: String,
    /// Size in bytes; restic reports none for folders.
    pub size: Option<u64>,
    /// The backup of the snapshot; none for unassigned snapshots.
    pub backup_id: Option<String>,
}

impl FindHit {
    /// The folder that shows the hit in the folder list: the folder that contains it.
    pub fn folder(&self) -> &str {
        parent_dir(&self.path)
    }
}

impl ListEntry for FindHit {
    fn path(&self) -> &str {
        &self.path
    }
}

/// The folder that contains `path` (`/` for an entry at the top).
pub fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

/// The hits of a search, newest snapshot first and by path within a snapshot.
pub struct FindResult {
    pub hits: QueryList<FindHit>,
    /// Number of snapshots with at least one hit.
    pub snapshots_matched: u64,
}

impl HeldResult for FindResult {
    fn held_bytes(&self) -> u64 {
        self.hits.held_bytes()
    }
}

/// Why a search text is refused before restic runs.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FindTextError {
    #[error("search text is empty")]
    Empty,
    #[error("search text contains a NUL character")]
    Nul,
}

/// Checks the search text: not empty or blank, no NUL.
pub fn validate_search_text(text: &str) -> Result<(), FindTextError> {
    if text.trim().is_empty() {
        return Err(FindTextError::Empty);
    }
    if text.contains('\0') {
        return Err(FindTextError::Nul);
    }
    Ok(())
}

/// The restic pattern that finds `text` anywhere in a path: the text taken literally (`*`, `?`,
/// `[` and `\` masked) between two `*`. restic matches a pattern without a leading `/` against
/// every run of consecutive path segments, so a folder hit also finds the entries below it.
pub fn find_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('*');
    for c in text.chars() {
        if matches!(c, '*' | '?' | '[' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('*');
    pattern
}

/// Arguments of `restic find` for `pattern` in exactly the snapshots `snapshot_ids` (full IDs;
/// `--json` comes from the command spec). Case is ignored.
pub fn find_args(snapshot_ids: &[&str], pattern: &str) -> Vec<String> {
    let mut args = vec![
        "find".to_string(),
        "--no-lock".to_string(),
        "--ignore-case".to_string(),
    ];
    for id in snapshot_ids {
        args.push("--snapshot".to_string());
        args.push((*id).to_string());
    }
    args.push(pattern.to_string());
    args
}

/// The snapshots of `scope` among all snapshots of a destination, newest first, sorted like the
/// snapshot lists: never a metadata snapshot, never one of another backup.
pub fn scope_snapshots(
    snapshots: Vec<ResticSnapshot>,
    configured_backup_ids: &[&str],
    scope: &FindScope,
) -> Vec<ResticSnapshot> {
    let sorted = sort_destination_snapshots(snapshots, configured_backup_ids);
    let find_group = |groups: Vec<(String, Vec<ResticSnapshot>)>, id: &str| {
        groups
            .into_iter()
            .find(|(group, _)| group == id)
            .map(|(_, snaps)| snaps)
            .unwrap_or_default()
    };
    match scope {
        FindScope::Backup { backup_id } => find_group(sorted.backups, backup_id),
        FindScope::Unassigned => sorted.unassigned,
        FindScope::Orphaned { backup_id } => find_group(sorted.orphaned, backup_id),
    }
}

/// A match as `restic find --json` reports it; other fields are ignored.
#[derive(Deserialize)]
struct RawMatch {
    #[serde(default)]
    path: String,
    #[serde(default, rename = "type")]
    node_type: String,
    #[serde(default)]
    size: Option<u64>,
}

/// The matches of one snapshot. Beyond `limit_bytes` they are only counted.
#[derive(Default)]
struct RawGroup {
    snapshot: String,
    matches: Vec<RawMatch>,
    /// All matches of the group, held or not.
    count: u64,
    dropped: bool,
}

/// Collects the hits of the searched snapshots from the output of `restic find --json`, an
/// array of `{"matches":[…],"hits":n,"snapshot":"<id>"}`, read as a stream: one very long line
/// is never held as a whole. Unknown fields are ignored.
pub struct FindCollector {
    /// Position (newest first) and time of each searched snapshot, by ID.
    snapshots: HashMap<String, (usize, String)>,
    backup_id: Option<String>,
    limit_bytes: u64,
    builder: ListBuilder<FindHit>,
    snapshots_matched: u64,
}

impl FindCollector {
    /// Collects the hits of `snapshots` (newest first) for the backup `backup_id`; the list is
    /// dropped once it exceeds `limit_bytes`.
    pub fn new(snapshots: &[ResticSnapshot], backup_id: Option<&str>, limit_bytes: u64) -> Self {
        Self {
            snapshots: snapshots
                .iter()
                .enumerate()
                .map(|(i, s)| (s.id.clone(), (i, s.time.clone())))
                .collect(),
            backup_id: backup_id.map(str::to_string),
            limit_bytes,
            builder: ListBuilder::new(limit_bytes),
            snapshots_matched: 0,
        }
    }

    /// Reads the whole output of `restic find --json`.
    pub fn read(&mut self, reader: &mut dyn BufRead) -> Result<(), serde_json::Error> {
        let mut de = serde_json::Deserializer::from_reader(reader);
        GroupsSeed { collector: self }.deserialize(&mut de)?;
        de.end()
    }

    fn add_group(&mut self, group: RawGroup) {
        let Some((_, time)) = self.snapshots.get(&group.snapshot) else {
            return;
        };
        if group.count == 0 {
            return;
        }
        self.snapshots_matched += 1;
        if group.dropped {
            self.builder.push_dropped(group.count);
            return;
        }
        let time = time.clone();
        for m in group.matches {
            self.builder.push(FindHit {
                snapshot_id: group.snapshot.clone(),
                snapshot_time: time.clone(),
                path: m.path,
                entry_type: m.node_type,
                size: m.size,
                backup_id: self.backup_id.clone(),
            });
        }
    }

    pub fn finish(self) -> FindResult {
        let order = self.snapshots;
        let position = |hit: &FindHit| order.get(&hit.snapshot_id).map_or(usize::MAX, |p| p.0);
        let hits = self.builder.finish_sorted_by(|a, b| {
            position(a)
                .cmp(&position(b))
                .then_with(|| a.path.cmp(&b.path))
        });
        FindResult {
            hits,
            snapshots_matched: self.snapshots_matched,
        }
    }
}

struct GroupsSeed<'a> {
    collector: &'a mut FindCollector,
}

impl<'de> DeserializeSeed<'de> for GroupsSeed<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for GroupsSeed<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of snapshots with matches")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let limit_bytes = self.collector.limit_bytes;
        while let Some(group) = seq.next_element_seed(GroupSeed { limit_bytes })? {
            self.collector.add_group(group);
        }
        Ok(())
    }
}

struct GroupSeed {
    limit_bytes: u64,
}

impl<'de> DeserializeSeed<'de> for GroupSeed {
    type Value = RawGroup;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<RawGroup, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for GroupSeed {
    type Value = RawGroup;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a snapshot with matches")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawGroup, A::Error> {
        let mut group = RawGroup::default();
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "snapshot" => group.snapshot = map.next_value()?,
                "matches" => {
                    let (matches, count, dropped) = map.next_value_seed(MatchesSeed {
                        limit_bytes: self.limit_bytes,
                    })?;
                    group.matches = matches;
                    group.count = count;
                    group.dropped = dropped;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(group)
    }
}

/// The matches of one snapshot (`null` counts as none): the held matches, their number, and
/// whether they exceeded the limit and were dropped.
struct MatchesSeed {
    limit_bytes: u64,
}

impl<'de> DeserializeSeed<'de> for MatchesSeed {
    type Value = (Vec<RawMatch>, u64, bool);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_option(self)
    }
}

impl<'de> Visitor<'de> for MatchesSeed {
    type Value = (Vec<RawMatch>, u64, bool);

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of matches or null")
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok((Vec::new(), 0, false))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok((Vec::new(), 0, false))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut matches = Vec::new();
        let mut count = 0u64;
        let mut bytes = 0u64;
        let mut dropped = false;
        while let Some(m) = seq.next_element::<RawMatch>()? {
            count += 1;
            bytes = bytes.saturating_add(m.path.len() as u64 + ENTRY_OVERHEAD_BYTES);
            if bytes > self.limit_bytes {
                dropped = true;
                matches = Vec::new();
            }
            if !dropped {
                matches.push(m);
            }
        }
        Ok((matches, count, dropped))
    }
}

/// What a search looks for and where.
pub struct FindRequest<'a> {
    pub scope: &'a FindScope,
    /// IDs of the configured backups, to sort the snapshots like the snapshot lists.
    pub configured_backup_ids: &'a [&'a str],
    /// The search text, taken literally.
    pub text: &'a str,
}

/// Searches the snapshots of `request.scope` for paths that contain `request.text` (any case),
/// after the reachability check of the destination: one `restic snapshots` call, then
/// `restic find --no-lock` over exactly these snapshots; without snapshots restic does not run.
pub fn execute_find(
    access: &SnapshotListAccess<'_>,
    request: &FindRequest<'_>,
    cancel: Option<&CancelHandle>,
    limit_bytes: u64,
) -> Result<FindResult, BackupRunError> {
    let all = list_destination_snapshots(access)?;
    let snapshots = scope_snapshots(all, request.configured_backup_ids, request.scope);
    let mut collector = FindCollector::new(&snapshots, request.scope.backup_id(), limit_bytes);
    if cancel.is_some_and(CancelHandle::is_cancelled) {
        return Err(BackupRunError::Cancelled);
    }
    if snapshots.is_empty() {
        return Ok(collector.finish());
    }
    let ids: Vec<&str> = snapshots.iter().map(|s| s.id.as_str()).collect();
    let mut spec = ResticCommandSpec::for_destination(
        &access.resolved.binary,
        access.kestic_binary,
        access.destination,
        find_args(&ids, &find_pattern(request.text)),
    );
    spec.low_priority = false;
    let mut parsed = Ok(());
    let res = run_restic_reading(
        &spec,
        access.destination,
        Some(access.secret_store),
        access.clock,
        cancel,
        access.extra_env,
        &mut |reader| parsed = collector.read(reader),
    )?;
    if res.cancelled {
        return Err(BackupRunError::Cancelled);
    }
    check_command_succeeded(&res)?;
    parsed?;
    Ok(collector.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_list::PageRequest;

    fn snap(id: char, time: &str, tags: &[&str]) -> ResticSnapshot {
        serde_json::from_value(serde_json::json!({
            "id": id.to_string().repeat(64),
            "time": time,
            "tags": tags,
        }))
        .expect("snapshot JSON")
    }

    fn ids(snaps: &[ResticSnapshot]) -> Vec<char> {
        snaps
            .iter()
            .map(|s| s.id.chars().next().unwrap_or(' '))
            .collect()
    }

    #[test]
    fn pattern_takes_the_text_literally_between_wildcards() {
        assert_eq!(find_pattern("bericht.pdf"), "*bericht.pdf*");
        assert_eq!(find_pattern(r"a*b?c[d]\e"), r"*a\*b\?c\[d]\\e*");
        assert_eq!(find_pattern("Steuern/2026"), "*Steuern/2026*");
    }

    #[test]
    fn search_text_must_not_be_blank_or_hold_nul() {
        assert_eq!(validate_search_text("  "), Err(FindTextError::Empty));
        assert_eq!(validate_search_text("a\0b"), Err(FindTextError::Nul));
        assert_eq!(validate_search_text(" a "), Ok(()));
    }

    #[test]
    fn find_args_name_every_snapshot_and_ignore_case() {
        assert_eq!(
            find_args(&["aa", "bb"], "*x*"),
            [
                "find",
                "--no-lock",
                "--ignore-case",
                "--snapshot",
                "aa",
                "--snapshot",
                "bb",
                "*x*"
            ]
        );
    }

    #[test]
    fn scope_takes_the_snapshots_of_the_list_it_names() {
        let all = || {
            vec![
                snap('a', "2026-11-10T12:00:00Z", &["kestic:b1"]),
                snap('b', "2026-11-12T12:00:00Z", &["kestic:b1"]),
                snap('c', "2026-11-11T12:00:00Z", &["kestic-meta:b1"]),
                snap('d', "2026-11-11T12:00:00Z", &["kestic:b2"]),
                snap('e', "2026-11-11T12:00:00Z", &["kestic:b1", "kestic:b2"]),
                snap('f', "2026-11-11T12:00:00Z", &["daily"]),
                snap('g', "2026-11-11T12:00:00Z", &["kestic:x9"]),
            ]
        };
        let configured = ["b1", "b2"];
        let backup = |id: &str| FindScope::Backup {
            backup_id: id.to_string(),
        };
        assert_eq!(
            ids(&scope_snapshots(all(), &configured, &backup("b1"))),
            ['b', 'a']
        );
        assert_eq!(
            ids(&scope_snapshots(all(), &configured, &backup("b2"))),
            ['d']
        );
        assert_eq!(
            ids(&scope_snapshots(all(), &configured, &FindScope::Unassigned)),
            ['f', 'e']
        );
        let orphaned = FindScope::Orphaned {
            backup_id: "x9".to_string(),
        };
        assert_eq!(ids(&scope_snapshots(all(), &configured, &orphaned)), ['g']);
        assert!(scope_snapshots(all(), &configured, &backup("zz")).is_empty());
    }

    fn collect(output: &str, limit_bytes: u64) -> FindResult {
        let snaps = vec![
            snap('b', "2026-11-12T12:00:00Z", &["kestic:b1"]),
            snap('a', "2026-11-10T12:00:00Z", &["kestic:b1"]),
        ];
        let mut c = FindCollector::new(&snaps, Some("b1"), limit_bytes);
        c.read(&mut output.as_bytes()).expect("find output parses");
        c.finish()
    }

    #[test]
    fn collector_sorts_newest_snapshot_first_then_by_path() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let x = "x".repeat(64);
        let output = format!(
            r#"[{{"matches":[{{"path":"/s/z.pdf","type":"file","size":3,"future":1}}],"hits":1,"snapshot":"{a}"}},
               {{"matches":[{{"path":"/s/y.pdf","type":"file","size":4}},{{"path":"/s/d","type":"dir"}}],"hits":2,"snapshot":"{b}","new":[1]}},
               {{"matches":[{{"path":"/s/q.pdf","type":"file"}}],"hits":1,"snapshot":"{x}"}},
               {{"matches":null,"hits":0,"snapshot":"{a}"}}]"#
        );
        let mut result = collect(&output, u64::MAX);
        assert_eq!(result.snapshots_matched, 2);
        let page = result.hits.page(&PageRequest::default());
        let seen: Vec<(char, &str)> = page
            .items
            .iter()
            .map(|h| (h.snapshot_id.chars().next().unwrap_or(' '), h.path.as_str()))
            .collect();
        assert_eq!(seen, [('b', "/s/d"), ('b', "/s/y.pdf"), ('a', "/s/z.pdf")]);
        assert_eq!(page.items[0].size, None);
        assert_eq!(page.items[1].size, Some(4));
        assert_eq!(page.items[2].snapshot_time, "2026-11-10T12:00:00Z");
        assert_eq!(page.items[2].backup_id.as_deref(), Some("b1"));
    }

    #[test]
    fn collector_reads_no_hits_and_counts_a_list_over_the_limit() {
        let mut none = collect("[]\n", u64::MAX);
        assert_eq!(none.hits.total(), 0);
        assert_eq!(none.snapshots_matched, 0);
        assert!(!none.hits.page(&PageRequest::default()).too_large);

        let a = "a".repeat(64);
        let output = format!(
            r#"[{{"matches":[{{"path":"/1"}},{{"path":"/2"}},{{"path":"/3"}}],"snapshot":"{a}"}}]"#
        );
        let mut over = collect(&output, 2 * (ENTRY_OVERHEAD_BYTES + 2));
        assert_eq!(over.hits.total(), 3);
        assert_eq!(over.snapshots_matched, 1);
        assert!(over.hits.too_large());
        assert!(over.hits.page(&PageRequest::default()).items.is_empty());
    }

    #[test]
    fn collector_refuses_output_that_is_no_array() {
        let snaps = vec![snap('a', "2026-11-10T12:00:00Z", &["kestic:b1"])];
        let mut c = FindCollector::new(&snaps, Some("b1"), u64::MAX);
        assert!(c.read(&mut "{\"x\":1}".as_bytes()).is_err());
    }

    #[test]
    fn hits_open_in_the_folder_that_contains_them() {
        let hit = |path: &str| FindHit {
            snapshot_id: String::new(),
            snapshot_time: String::new(),
            path: path.to_string(),
            entry_type: "file".to_string(),
            size: None,
            backup_id: None,
        };
        assert_eq!(hit("/home/u/a/bericht.pdf").folder(), "/home/u/a");
        assert_eq!(hit("/top").folder(), "/");
    }
}
