// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Result lists of snapshot queries (folder contents, search hits, changes, preview entries):
//! held in full, handed out page by page with a text filter, and bounded in memory. Lists are
//! never cut short: a list over its memory limit is dropped as a whole while its counts stay
//! exact.

use crate::restic::CancelHandle;
use kestic_i18n::codes::ErrorKind;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use thiserror::Error;
use time::{Duration, OffsetDateTime};

/// Page size when the request names none.
pub const DEFAULT_PAGE_LIMIT: u32 = 200;
/// Largest page a request may ask for.
pub const MAX_PAGE_LIMIT: u32 = 1000;
/// Bytes counted per entry on top of the length of its path.
pub const ENTRY_OVERHEAD_BYTES: u64 = 64;
/// How long a finished query stays available after it was last fetched.
pub const QUERY_RETENTION: Duration = Duration::minutes(10);

const MIB: u64 = 1024 * 1024;

/// Memory limits for the lists of all held queries, counted with [`entry_cost`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryLimits {
    /// Limit for the list of one query.
    pub per_query_bytes: u64,
    /// Limit for the lists of all held queries together.
    pub total_bytes: u64,
}

impl Default for QueryLimits {
    /// 256 MiB per query, 512 MiB in total: about a million paths of medium length per list.
    fn default() -> Self {
        Self {
            per_query_bytes: 256 * MIB,
            total_bytes: 512 * MIB,
        }
    }
}

/// An entry of a query list.
pub trait ListEntry {
    /// The path of the entry; the list is sorted by it and its memory is counted by it.
    fn path(&self) -> &str;

    /// The text the filter searches in; the path unless the list holds one folder only.
    fn filter_text(&self) -> &str {
        self.path()
    }

    /// The kind code the `kinds` filter compares; `None` for lists without kinds.
    fn kind_code(&self) -> Option<&str> {
        None
    }
}

/// Memory counted for `entry`: the length of its path plus [`ENTRY_OVERHEAD_BYTES`].
pub fn entry_cost(entry: &impl ListEntry) -> u64 {
    entry.path().len() as u64 + ENTRY_OVERHEAD_BYTES
}

/// Collects the entries of a running query. Once they exceed the limit, the entries are dropped
/// and only counted, so the finished list says «too large» with an exact total.
pub struct ListBuilder<T> {
    items: Option<Vec<T>>,
    total: u64,
    bytes: u64,
    limit_bytes: u64,
}

impl<T: ListEntry> ListBuilder<T> {
    pub fn new(limit_bytes: u64) -> Self {
        Self {
            items: Some(Vec::new()),
            total: 0,
            bytes: 0,
            limit_bytes,
        }
    }

    pub fn push(&mut self, entry: T) {
        self.total += 1;
        self.bytes = self.bytes.saturating_add(entry_cost(&entry));
        if self.bytes > self.limit_bytes {
            self.items = None;
        }
        if let Some(items) = self.items.as_mut() {
            items.push(entry);
        }
    }

    /// Counts `n` entries that were too large to collect: the list is dropped, its total stays
    /// exact.
    pub fn push_dropped(&mut self, n: u64) {
        if n > 0 {
            self.total += n;
            self.items = None;
        }
    }

    /// The finished list, sorted by path.
    pub fn finish(self) -> QueryList<T> {
        self.finish_sorted_by(|a, b| a.path().cmp(b.path()))
    }

    /// The finished list, sorted by `order`.
    pub fn finish_sorted_by(self, order: impl FnMut(&T, &T) -> Ordering) -> QueryList<T> {
        let items = self.items.map(|mut items| {
            items.sort_by(order);
            items
        });
        QueryList {
            bytes: if items.is_some() { self.bytes } else { 0 },
            items,
            total: self.total,
            filter_cache: None,
        }
    }
}

/// Filter of a page request: a text in the path (any case) and kind codes (empty = all).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PageFilter {
    pub text: String,
    pub kinds: Vec<String>,
}

impl PageFilter {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.kinds.is_empty()
    }
}

/// One page of a query list as requested (`page_json` of `GetSnapshotQueryStatus`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PageRequest {
    pub offset: u64,
    pub limit: u32,
    pub filter: PageFilter,
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: DEFAULT_PAGE_LIMIT,
            filter: PageFilter::default(),
        }
    }
}

/// Why a page request is refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PageRequestError {
    #[error("invalid page JSON: {0}")]
    Json(String),
    #[error("page limit must be between 1 and {MAX_PAGE_LIMIT}, got {0}")]
    Limit(u32),
}

impl PageRequest {
    /// Parses `page_json`; an empty string is the first page without filter.
    pub fn parse(page_json: &str) -> Result<Self, PageRequestError> {
        let request: Self = if page_json.trim().is_empty() {
            Self::default()
        } else {
            serde_json::from_str(page_json).map_err(|e| PageRequestError::Json(e.to_string()))?
        };
        if !(1..=MAX_PAGE_LIMIT).contains(&request.limit) {
            return Err(PageRequestError::Limit(request.limit));
        }
        Ok(request)
    }
}

/// A page of a query list (`ListPageDto`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPage<T> {
    /// The entries of the page.
    pub items: Vec<T>,
    pub offset: u64,
    /// All entries of the query.
    pub total: u64,
    /// Entries that match the filter; without filter the total.
    pub matched: u64,
    /// The list exceeded its memory limit and is not held; `items` is empty.
    pub too_large: bool,
}

/// The complete, sorted list of a finished query.
pub struct QueryList<T> {
    /// `None` once the list exceeded its memory limit.
    items: Option<Vec<T>>,
    total: u64,
    bytes: u64,
    /// Positions of the entries matching the last filter used.
    filter_cache: Option<(PageFilter, Vec<u32>)>,
}

impl<T: ListEntry + Clone> QueryList<T> {
    /// All entries of the query, held or not.
    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn too_large(&self) -> bool {
        self.items.is_none()
    }

    /// Memory the held list counts against the limits; 0 for a dropped list.
    pub fn held_bytes(&self) -> u64 {
        self.bytes
    }

    /// The page `request` asks for, in the fixed order of the list.
    pub fn page(&mut self, request: &PageRequest) -> ListPage<T> {
        let total = self.total;
        let Some(items) = self.items.as_ref() else {
            // The filter cannot be applied to a dropped list; nothing is listed anyway.
            return ListPage {
                items: Vec::new(),
                offset: request.offset,
                total,
                matched: total,
                too_large: true,
            };
        };
        let start = usize::try_from(request.offset).unwrap_or(usize::MAX);
        let limit = request.limit as usize;
        if request.filter.is_empty() {
            return ListPage {
                items: items.iter().skip(start).take(limit).cloned().collect(),
                offset: request.offset,
                total,
                matched: total,
                too_large: false,
            };
        }
        let cached = self
            .filter_cache
            .as_ref()
            .is_some_and(|(filter, _)| *filter == request.filter);
        if !cached {
            let positions = matching_positions(items, &request.filter);
            self.filter_cache = Some((request.filter.clone(), positions));
        }
        let positions = self
            .filter_cache
            .as_ref()
            .map_or(&[][..], |(_, p)| p.as_slice());
        ListPage {
            items: positions
                .iter()
                .skip(start)
                .take(limit)
                .filter_map(|&i| items.get(i as usize).cloned())
                .collect(),
            offset: request.offset,
            total,
            matched: positions.len() as u64,
            too_large: false,
        }
    }
}

fn matching_positions<T: ListEntry>(items: &[T], filter: &PageFilter) -> Vec<u32> {
    let needle = filter.text.to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, e)| needle.is_empty() || e.filter_text().to_lowercase().contains(&needle))
        .filter(|(_, e)| {
            filter.kinds.is_empty()
                || e.kind_code()
                    .is_some_and(|k| filter.kinds.iter().any(|c| c == k))
        })
        .filter_map(|(i, _)| u32::try_from(i).ok())
        .collect()
}

/// A query result that holds a list counted against the memory limits.
pub trait HeldResult {
    fn held_bytes(&self) -> u64;
}

/// State of a held query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryState {
    Running,
    Done,
    Cancelled,
    Error,
}

impl QueryState {
    /// The `state` string of the status DTO.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
        }
    }
}

/// How a query ended.
pub enum QueryOutcome<R> {
    Done(R),
    Cancelled,
    /// Error code and raw detail.
    Error(ErrorKind, String),
}

/// A query held by the daemon.
pub struct HeldQuery<R> {
    pub state: QueryState,
    pub result: Option<R>,
    /// Error code and raw detail while the state is `Error`.
    pub error: Option<(ErrorKind, String)>,
    pub cancel: CancelHandle,
    /// When it ended (independent of later fetches); `None` while it runs.
    pub finished_at: Option<OffsetDateTime>,
    /// When it was last fetched (or finished, before the first fetch).
    last_access: OffsetDateTime,
}

/// The queries of the daemon by ID. Finished queries stay [`QUERY_RETENTION`] after their last
/// fetch; when the lists of all queries would exceed the total limit, the finished queries
/// fetched longest ago are freed first.
pub struct QueryStore<R> {
    queries: HashMap<String, HeldQuery<R>>,
    limits: QueryLimits,
}

impl<R: HeldResult> QueryStore<R> {
    pub fn new(limits: QueryLimits) -> Self {
        Self {
            queries: HashMap::new(),
            limits,
        }
    }

    pub fn limits(&self) -> QueryLimits {
        self.limits
    }

    /// Memory of the lists of all held queries.
    pub fn held_bytes(&self) -> u64 {
        self.queries
            .values()
            .filter_map(|q| q.result.as_ref())
            .map(HeldResult::held_bytes)
            .sum()
    }

    /// Adds the running query `id`.
    pub fn register(&mut self, id: String, cancel: CancelHandle, now: OffsetDateTime) {
        self.expire(now);
        self.queries.insert(
            id,
            HeldQuery {
                state: QueryState::Running,
                result: None,
                error: None,
                cancel,
                finished_at: None,
                last_access: now,
            },
        );
    }

    /// Withdraws query `id` (its worker never started).
    pub fn remove(&mut self, id: &str) {
        self.queries.remove(id);
    }

    /// Records how query `id` ended; frees the finished queries fetched longest ago while the
    /// lists would exceed the total limit. A query that is gone (freed meanwhile) stays gone.
    pub fn finish(&mut self, id: &str, outcome: QueryOutcome<R>, now: OffsetDateTime) {
        if !self.queries.contains_key(id) {
            return;
        }
        if let QueryOutcome::Done(result) = &outcome {
            self.make_room(id, result.held_bytes());
        }
        let Some(query) = self.queries.get_mut(id) else {
            return;
        };
        query.last_access = now;
        query.finished_at = Some(now);
        match outcome {
            QueryOutcome::Done(result) => {
                query.state = QueryState::Done;
                query.result = Some(result);
            }
            QueryOutcome::Cancelled => query.state = QueryState::Cancelled,
            QueryOutcome::Error(kind, detail) => {
                query.state = QueryState::Error;
                query.error = Some((kind, detail));
            }
        }
    }

    fn make_room(&mut self, id: &str, needed: u64) {
        while self.held_bytes().saturating_add(needed) > self.limits.total_bytes {
            let oldest = self
                .queries
                .iter()
                .filter(|(qid, q)| {
                    qid.as_str() != id && q.result.as_ref().is_some_and(|r| r.held_bytes() > 0)
                })
                .min_by_key(|(_, q)| q.last_access)
                .map(|(qid, _)| qid.clone());
            let Some(oldest) = oldest else {
                return;
            };
            self.queries.remove(&oldest);
        }
    }

    /// Query `id` for a fetch at `now`, which counts as its last access; `None` when it does
    /// not exist (never, expired or freed).
    pub fn fetch(&mut self, id: &str, now: OffsetDateTime) -> Option<&mut HeldQuery<R>> {
        self.expire(now);
        let query = self.queries.get_mut(id)?;
        query.last_access = now;
        Some(query)
    }

    /// Query `id` without counting as an access; `None` when it does not exist.
    pub fn peek(&self, id: &str) -> Option<&HeldQuery<R>> {
        self.queries.get(id)
    }

    /// The cancel handle of query `id`.
    pub fn cancel_handle(&self, id: &str) -> Option<&CancelHandle> {
        self.queries.get(id).map(|q| &q.cancel)
    }

    fn expire(&mut self, now: OffsetDateTime) {
        self.queries.retain(|_, q| {
            q.state == QueryState::Running || now - q.last_access <= QUERY_RETENTION
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[derive(Debug, Clone, PartialEq)]
    struct Entry(String);

    impl ListEntry for Entry {
        fn path(&self) -> &str {
            &self.0
        }
    }

    struct Held(u64);

    impl HeldResult for Held {
        fn held_bytes(&self) -> u64 {
            self.0
        }
    }

    fn list(paths: &[&str]) -> QueryList<Entry> {
        let mut b = ListBuilder::new(u64::MAX);
        for p in paths {
            b.push(Entry((*p).to_string()));
        }
        b.finish()
    }

    #[test]
    fn page_request_defaults_and_limits() {
        assert_eq!(
            PageRequest::parse("").expect("empty"),
            PageRequest::default()
        );
        assert_eq!(PageRequest::default().limit, 200);
        assert_eq!(
            PageRequest::parse(r#"{"limit":0}"#),
            Err(PageRequestError::Limit(0))
        );
        assert_eq!(
            PageRequest::parse(r#"{"limit":1001}"#),
            Err(PageRequestError::Limit(1001))
        );
        let p = PageRequest::parse(r#"{"offset":5,"filter":{"text":"a"}}"#).expect("page");
        assert_eq!(p.offset, 5);
        assert_eq!(p.limit, 200);
        assert_eq!(p.filter.text, "a");
    }

    #[test]
    fn list_is_sorted_by_path_and_paged() {
        let mut l = list(&["/c", "/a", "/b"]);
        let page = l.page(&PageRequest {
            offset: 1,
            limit: 1,
            filter: PageFilter::default(),
        });
        assert_eq!(page.items, vec![Entry("/b".into())]);
        assert_eq!(page.total, 3);
        assert_eq!(page.matched, 3);
    }

    #[test]
    fn filter_ignores_case_and_keeps_total() {
        let mut l = list(&["/x/Report", "/x/report.txt", "/x/other"]);
        let page = l.page(&PageRequest {
            filter: PageFilter {
                text: "REPORT".into(),
                kinds: Vec::new(),
            },
            ..PageRequest::default()
        });
        assert_eq!(page.matched, 2);
        assert_eq!(page.total, 3);
        assert_eq!(page.items.len(), 2);
    }

    #[test]
    fn retained_finished_query_expires_ten_minutes_after_last_fetch() {
        let t0 = datetime!(2026-11-15 12:00 UTC);
        let mut store = QueryStore::new(QueryLimits::default());
        store.register("q".into(), CancelHandle::new(), t0);
        store.finish("q", QueryOutcome::Done(Held(1)), t0);
        assert!(store.fetch("q", t0 + Duration::minutes(9)).is_some());
        assert!(store.fetch("q", t0 + Duration::minutes(18)).is_some());
        assert!(store.fetch("q", t0 + Duration::minutes(29)).is_none());
    }
}
