// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S03 memory limit of query lists with synthetic paths (AC-M4-03-7, unit part).

use kestic_core::query_list::{
    ENTRY_OVERHEAD_BYTES, HeldResult, ListBuilder, ListEntry, MAX_PAGE_LIMIT, PageFilter,
    PageRequest, QueryLimits, QueryOutcome, QueryStore,
};
use kestic_core::restic::CancelHandle;
use time::Duration;
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

/// AC-M4-03-7 (a), (b): a list over the limit is dropped as a whole; `too_large` is set,
/// no entry is handed out, and the total stays exact.
#[test]
fn ac_m4_03_7_a_b_list_over_the_limit_is_too_large_with_exact_total() {
    let mut builder = ListBuilder::new(10 * (100 + ENTRY_OVERHEAD_BYTES));
    for i in 0..25u32 {
        builder.push(Entry(format!("/{i:099}")));
    }
    let mut l = builder.finish();
    let page = l.page(&PageRequest::default());
    assert!(page.too_large, "(a) too_large");
    assert!(page.items.is_empty(), "(a) no items");
    assert_eq!(page.total, 25, "(b) exact total");
    assert_eq!(l.held_bytes(), 0, "a dropped list holds no memory");
}

/// AC-M4-03-7 (d): when the total limit is full, the finished query fetched longest ago is
/// freed first.
#[test]
fn ac_m4_03_7_d_full_total_limit_frees_the_query_fetched_longest_ago() {
    let t0 = datetime!(2026-11-15 12:00 UTC);
    let mut store = QueryStore::new(QueryLimits {
        per_query_bytes: 100,
        total_bytes: 250,
    });
    for (id, at) in [("a", 0), ("b", 1)] {
        store.register(id.into(), CancelHandle::new(), t0 + Duration::seconds(at));
        store.finish(
            id,
            QueryOutcome::Done(Held(100)),
            t0 + Duration::seconds(at),
        );
    }
    // «a» was finished first but fetched again later than «b».
    assert!(store.fetch("a", t0 + Duration::seconds(5)).is_some());
    store.register("c".into(), CancelHandle::new(), t0 + Duration::seconds(6));
    store.finish(
        "c",
        QueryOutcome::Done(Held(100)),
        t0 + Duration::seconds(7),
    );
    let now = t0 + Duration::seconds(8);
    assert!(store.fetch("b", now).is_none(), "(d) «b» freed first");
    assert!(store.fetch("a", now).is_some(), "(d) «a» kept");
    assert!(store.fetch("c", now).is_some(), "(d) the new query is held");
    assert_eq!(store.held_bytes(), 200);
}

/// AC-M4-03-7 (e): a list of 10⁶ paths of 100 bytes each stays below the default limit and
/// every entry can be fetched exactly once.
#[test]
fn ac_m4_03_7_e_million_paths_of_100_bytes_stay_below_the_limit() {
    let limits = QueryLimits::default();
    let mut builder = ListBuilder::new(limits.per_query_bytes);
    for i in 0..1_000_000u32 {
        builder.push(Entry(format!("/{i:099}")));
    }
    let mut l = builder.finish();
    assert!(!l.too_large(), "(e) below the per-query limit");
    assert!(l.held_bytes() <= limits.per_query_bytes);
    let mut seen = 0u64;
    let mut previous: Option<String> = None;
    loop {
        let page = l.page(&PageRequest {
            offset: seen,
            limit: MAX_PAGE_LIMIT,
            filter: PageFilter::default(),
        });
        if page.items.is_empty() {
            break;
        }
        for e in page.items {
            // Strictly ascending paths: no entry twice, none skipped between pages.
            assert!(previous.as_deref().is_none_or(|p| p < e.0.as_str()));
            previous = Some(e.0);
            seen += 1;
        }
    }
    assert_eq!(seen, 1_000_000, "(e) every entry fetched exactly once");
}

/// AC-M4-03-7 (d): freeing room drops only finished lists that hold memory; a cancelled query
/// fetched long ago holds nothing and stays readable.
#[test]
fn ac_m4_03_7_d_freeing_room_keeps_records_without_a_list() {
    let t0 = datetime!(2026-11-15 12:00 UTC);
    let mut store = QueryStore::<Held>::new(QueryLimits {
        per_query_bytes: 100,
        total_bytes: 150,
    });
    store.register("cancelled".into(), CancelHandle::new(), t0);
    store.finish("cancelled", QueryOutcome::Cancelled, t0);
    store.register("old".into(), CancelHandle::new(), t0 + Duration::seconds(1));
    store.finish(
        "old",
        QueryOutcome::Done(Held(100)),
        t0 + Duration::seconds(1),
    );
    store.register("new".into(), CancelHandle::new(), t0 + Duration::seconds(2));
    store.finish(
        "new",
        QueryOutcome::Done(Held(100)),
        t0 + Duration::seconds(2),
    );
    let now = t0 + Duration::seconds(3);
    assert!(
        store.fetch("old", now).is_none(),
        "(d) the list fetched longest ago is freed"
    );
    assert!(
        store.fetch("cancelled", now).is_some(),
        "(d) a record without a list stays"
    );
    assert!(
        store.fetch("new", now).is_some(),
        "(d) the new list is held"
    );
}
