// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Searching snapshots with `restic find --json`: in the snapshots of one backup, the
//! unassigned or the orphaned snapshots of a destination. The snapshots come from one
//! `restic snapshots` call sorted like the snapshot lists, so a metadata snapshot or one of
//! another backup is never searched; restic then searches exactly these snapshot IDs.

use crate::backup::{
    BackupRunError, SnapshotListAccess, check_command_succeeded, list_destination_snapshots,
    run_reading,
};
use crate::query_list::{
    ENTRY_OVERHEAD_BYTES, HeldResult, ListBuilder, ListEntry, ListPage, PageRequest, QueryList,
};
use crate::restic::CancelHandle;
use crate::restore::escape_glob;
use crate::snapshot::ResticSnapshot;
use crate::snapshot_list::sort_destination_snapshots;
use crate::snapshot_ls::parent_dir;
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
    /// The unassigned snapshots: without a Kestic tag, or tagged with more than one backup ID.
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

/// A hit as the daemon holds it: its snapshot by position in [`FindResult`], so the ID and time
/// of a snapshot are held once, not once per hit.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HeldHit {
    /// Position of the snapshot, newest first.
    snapshot: usize,
    path: String,
    entry_type: String,
    size: Option<u64>,
}

impl ListEntry for HeldHit {
    fn path(&self) -> &str {
        &self.path
    }
}

/// The hits of a search, newest snapshot first and by path within a snapshot.
pub struct FindResult {
    hits: QueryList<HeldHit>,
    /// ID and time of the searched snapshots, newest first.
    snapshots: Vec<(String, String)>,
    backup_id: Option<String>,
    /// Number of snapshots with at least one hit.
    pub snapshots_matched: u64,
}

impl FindResult {
    /// All hits, held or not.
    pub fn total(&self) -> u64 {
        self.hits.total()
    }

    pub fn too_large(&self) -> bool {
        self.hits.too_large()
    }

    /// The page `request` asks for (the text filter searches the paths).
    pub fn page(&mut self, request: &PageRequest) -> ListPage<FindHit> {
        let page = self.hits.page(request);
        let items = page
            .items
            .into_iter()
            .map(|hit| {
                let (id, time) = self
                    .snapshots
                    .get(hit.snapshot)
                    .cloned()
                    .unwrap_or_default();
                FindHit {
                    snapshot_id: id,
                    snapshot_time: time,
                    path: hit.path,
                    entry_type: hit.entry_type,
                    size: hit.size,
                    backup_id: self.backup_id.clone(),
                }
            })
            .collect();
        ListPage {
            items,
            offset: page.offset,
            total: page.total,
            matched: page.matched,
            too_large: page.too_large,
        }
    }
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
    #[error("search text is longer than {MAX_SEARCH_TEXT_BYTES} bytes")]
    TooLong,
}

/// Longest search text in bytes; restic gets it as one argument.
pub const MAX_SEARCH_TEXT_BYTES: usize = 4096;

/// Most snapshot IDs per `restic find` call, so its arguments stay far below the system limit.
const FIND_BATCH_SNAPSHOTS: usize = 1000;

/// Checks the search text: not empty or blank, no NUL, at most [`MAX_SEARCH_TEXT_BYTES`].
pub fn validate_search_text(text: &str) -> Result<(), FindTextError> {
    if text.trim().is_empty() {
        return Err(FindTextError::Empty);
    }
    if text.contains('\0') {
        return Err(FindTextError::Nul);
    }
    if text.len() > MAX_SEARCH_TEXT_BYTES {
        return Err(FindTextError::TooLong);
    }
    Ok(())
}

/// The restic pattern that finds the paths containing `text`, taken literally (`*`, `?`, `[`, `\`
/// and `!`, which negates a leading pattern, masked), or `None` when no path can contain it.
///
/// restic matches a pattern without a leading `/` segment by segment against every run of
/// consecutive path segments, so a folder hit also finds the entries below it. A text inside a
/// path becomes `*text*`. A text that starts with `/` starts a segment: it becomes `text*`
/// without the `/`, which also finds a segment at the top. restic cleans the pattern like a
/// path, so a text with an empty, `.` or `..` segment between two `/` would match other paths;
/// a snapshot path never holds such a segment, so it finds nothing.
fn find_pattern(text: &str) -> Option<String> {
    let segments: Vec<&str> = text.split('/').collect();
    let inner = segments
        .get(1..segments.len().saturating_sub(1))
        .unwrap_or_default();
    if inner.iter().any(|s| matches!(*s, "" | "." | "..")) {
        return None;
    }
    // A `restic find` pattern starting with `!` is negated: `!` is masked as well.
    let literal = |t: &str| escape_glob(t).replace('!', "\\!");
    Some(match text.strip_prefix('/') {
        Some(rest) => format!("{}*", literal(rest)),
        None => format!("*{}*", literal(text)),
    })
}

/// Arguments of `restic find` for `pattern` in exactly the snapshots `snapshot_ids` (full IDs,
/// one batch of a search; `--json` comes from the command spec). Case is ignored; `--`
/// keeps a pattern that starts with `-` from being read as a flag.
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
    args.push("--".to_string());
    args.push(pattern.to_string());
    args
}

/// The snapshots of `scope` among all snapshots of a destination, newest first, sorted like the
/// snapshot lists: never a metadata snapshot, never one of another backup.
fn scope_snapshots(
    snapshots: Vec<ResticSnapshot>,
    configured_backup_ids: &[&str],
    scope: &FindScope,
) -> Vec<ResticSnapshot> {
    let sorted = sort_destination_snapshots(snapshots, configured_backup_ids);
    let find_group = |groups: Vec<(String, Vec<ResticSnapshot>)>, id: &str| {
        groups
            .into_iter()
            .find(|(group, _)| group == id)
            .map_or_default(|(_, snaps)| snaps)
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
struct FindCollector {
    /// Position (newest first) of each searched snapshot, by ID.
    positions: HashMap<String, usize>,
    /// ID and time of the searched snapshots, newest first.
    snapshots: Vec<(String, String)>,
    backup_id: Option<String>,
    limit_bytes: u64,
    builder: ListBuilder<HeldHit>,
    snapshots_matched: u64,
}

impl FindCollector {
    /// Collects the hits of `snapshots` (newest first) for the backup `backup_id`; the list is
    /// dropped once it exceeds `limit_bytes`.
    fn new(snapshots: &[ResticSnapshot], backup_id: Option<&str>, limit_bytes: u64) -> Self {
        Self {
            positions: snapshots
                .iter()
                .enumerate()
                .map(|(i, s)| (s.id.clone(), i))
                .collect(),
            snapshots: snapshots
                .iter()
                .map(|s| (s.id.clone(), s.time.clone()))
                .collect(),
            backup_id: backup_id.map(str::to_string),
            limit_bytes,
            builder: ListBuilder::new(limit_bytes),
            snapshots_matched: 0,
        }
    }

    /// Reads the whole output of `restic find --json`.
    fn read(&mut self, reader: &mut dyn BufRead) -> Result<(), serde_json::Error> {
        let mut de = serde_json::Deserializer::from_reader(reader);
        GroupsSeed { collector: self }.deserialize(&mut de)?;
        de.end()
    }

    fn add_group(&mut self, group: RawGroup) {
        let Some(&snapshot) = self.positions.get(&group.snapshot) else {
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
        for m in group.matches {
            self.builder.push(HeldHit {
                snapshot,
                path: m.path,
                entry_type: m.node_type,
                size: m.size,
            });
        }
    }

    fn finish(self) -> FindResult {
        let hits = self.builder.finish_sorted_by(|a, b| {
            a.snapshot
                .cmp(&b.snapshot)
                .then_with(|| a.path.cmp(&b.path))
        });
        FindResult {
            hits,
            snapshots: self.snapshots,
            backup_id: self.backup_id,
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

/// The arguments of the `restic find` calls that search `pattern` in `snapshot_ids`, at most
/// [`FIND_BATCH_SNAPSHOTS`] snapshots per call.
fn find_calls(snapshot_ids: &[&str], pattern: &str) -> Vec<Vec<String>> {
    snapshot_ids
        .chunks(FIND_BATCH_SNAPSHOTS)
        .map(|batch| find_args(batch, pattern))
        .collect()
}

/// Searches the snapshots of `request.scope` for paths that contain `request.text` (any case),
/// after the reachability check of the destination: one `restic snapshots` call, then
/// `restic find --no-lock` over exactly these snapshots, in batches. Without snapshots, or
/// for a text no path can contain, `find` does not run.
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
    let Some(pattern) = find_pattern(request.text) else {
        return Ok(collector.finish());
    };
    let ids: Vec<&str> = snapshots.iter().map(|s| s.id.as_str()).collect();
    for args in find_calls(&ids, &pattern) {
        let mut parsed = Ok(());
        let res = run_reading(access, args, cancel, &mut |reader| {
            parsed = collector.read(reader);
        })?;
        check_command_succeeded(&res)?;
        parsed?;
    }
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
        let pattern = |text| find_pattern(text).unwrap_or_default();
        assert_eq!(pattern("bericht.pdf"), "*bericht.pdf*");
        assert_eq!(pattern(r"a*b?c[d]\e"), r"*a\*b\?c\[d]\\e*");
        assert_eq!(
            pattern("/!x"),
            r"\!x*",
            "a leading «!» would negate the pattern"
        );
        assert_eq!(pattern("Steuern/2026"), "*Steuern/2026*");
        assert_eq!(pattern("../x"), "*../x*");
        assert_eq!(pattern("x/.."), "*x/..*");
    }

    #[test]
    fn a_leading_slash_starts_a_segment() {
        let pattern = |text| find_pattern(text).unwrap_or_default();
        assert_eq!(pattern("/home/u/bericht.pdf"), "home/u/bericht.pdf*");
        assert_eq!(pattern("/.."), "..*");
        assert_eq!(pattern("/-dash"), "-dash*");
        assert_eq!(pattern("/!x"), r"\!x*");
        assert_eq!(pattern("a!b"), r"*a\!b*");
        assert_eq!(pattern("/"), "*");
    }

    #[test]
    fn a_text_no_path_can_contain_has_no_pattern() {
        for text in ["a/../b", "a/./b", "a//b", "//a", "/../b"] {
            assert_eq!(find_pattern(text), None, "{text}");
        }
    }

    #[test]
    fn many_snapshots_are_searched_in_batches() {
        let ids: Vec<String> = (0..2500).map(|i| format!("{i:064x}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let calls = find_calls(&refs, "*x*");
        assert_eq!(calls.len(), 3);
        let searched: Vec<&String> = calls
            .iter()
            .flat_map(|args| args.iter().skip_while(|a| *a != "--snapshot"))
            .filter(|a| !matches!(a.as_str(), "--snapshot" | "--" | "*x*"))
            .collect();
        assert_eq!(searched.len(), 2500, "every snapshot once");
        for args in &calls {
            let n = args.iter().filter(|a| *a == "--snapshot").count();
            assert!(n <= FIND_BATCH_SNAPSHOTS, "{n} snapshots in one call");
            assert_eq!(args.last().map(String::as_str), Some("*x*"));
        }
    }

    #[test]
    fn search_text_must_not_be_blank_or_hold_nul() {
        assert_eq!(validate_search_text("  "), Err(FindTextError::Empty));
        assert_eq!(validate_search_text("a\0b"), Err(FindTextError::Nul));
        assert_eq!(validate_search_text(" a "), Ok(()));
        let long = "a".repeat(MAX_SEARCH_TEXT_BYTES);
        assert_eq!(validate_search_text(&long), Ok(()));
        assert_eq!(
            validate_search_text(&format!("{long}a")),
            Err(FindTextError::TooLong)
        );
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
                "--",
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
        let page = result.page(&PageRequest::default());
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
        assert_eq!(none.total(), 0);
        assert_eq!(none.snapshots_matched, 0);
        assert!(!none.page(&PageRequest::default()).too_large);

        let a = "a".repeat(64);
        let output = format!(
            r#"[{{"matches":[{{"path":"/1"}},{{"path":"/2"}},{{"path":"/3"}}],"snapshot":"{a}"}}]"#
        );
        let mut over = collect(&output, 2 * (ENTRY_OVERHEAD_BYTES + 2));
        assert_eq!(over.total(), 3);
        assert_eq!(over.snapshots_matched, 1);
        assert!(over.too_large());
        assert!(over.page(&PageRequest::default()).items.is_empty());
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
