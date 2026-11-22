// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! One snapshot query of a list model (folder list, search hits): started on a worker thread so
//! the window never waits for restic, polled by a timer, fetched page by page, and cancelled
//! when the view leaves it. Also the texts every query list shares.

use crate::state_cache::{
    daemon_unavailable_text, dbus_error_text, i18n, i18np, make_client, tr_text,
};
use kestic_core::query_list::{DEFAULT_PAGE_LIMIT, PageFilter};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{
    KesticClient, KesticDbusError, SnapshotQueryPage, SnapshotQueryRequest, SnapshotQueryResultDto,
};
use std::sync::{Arc, Mutex};

/// The start of a query, as far as it has got.
enum QueryStart {
    /// `StartSnapshotQuery` is still on its way.
    Pending,
    Started(String),
    Failed(String),
    /// The query was left before it started; the start cancels it right away.
    Abandoned,
}

/// What [`QueryRun::poll_start`] found.
pub(crate) enum StartPoll {
    /// The query is not started yet.
    Waiting,
    /// The query runs (or has ended) in the daemon.
    Started,
    /// The query could not be started; the error text.
    Failed(String),
}

/// The query of a list model. Each [`QueryRun::start`] gets its own slot, so a late answer for
/// an earlier query is ignored.
#[derive(Default)]
pub(crate) struct QueryRun {
    start: Option<Arc<Mutex<QueryStart>>>,
    query_id: Option<String>,
    client: Option<KesticClient>,
}

impl QueryRun {
    /// Starts `request` on a worker thread (starting a query checks restic first).
    pub(crate) fn start(&mut self, request: SnapshotQueryRequest) {
        self.query_id = None;
        let slot = Arc::new(Mutex::new(QueryStart::Pending));
        self.start = Some(Arc::clone(&slot));
        std::thread::spawn(move || {
            let Ok(client) = make_client() else {
                *slot.lock_unpoisoned() = QueryStart::Failed(daemon_unavailable_text());
                return;
            };
            let started = match client.start_snapshot_query(&request) {
                Ok(id) => QueryStart::Started(id),
                Err(err) => QueryStart::Failed(dbus_error_text(&err)),
            };
            let mut guard = slot.lock_unpoisoned();
            match (&*guard, started) {
                (QueryStart::Abandoned, QueryStart::Started(id)) => {
                    drop(guard);
                    // Nobody waits for the answer; a query that already ended needs nothing.
                    let _ = client.cancel_snapshot_query(&id);
                }
                (QueryStart::Abandoned, _) => {}
                (_, started) => *guard = started,
            }
        });
    }

    /// Picks up the start of the query once it arrived.
    pub(crate) fn poll_start(&mut self) -> StartPoll {
        if self.query_id.is_some() {
            return StartPoll::Started;
        }
        let started = self
            .start
            .as_ref()
            .map(|slot| std::mem::replace(&mut *slot.lock_unpoisoned(), QueryStart::Pending));
        match started {
            Some(QueryStart::Started(id)) => {
                self.query_id = Some(id);
                StartPoll::Started
            }
            Some(QueryStart::Failed(text)) => StartPoll::Failed(text),
            Some(QueryStart::Pending | QueryStart::Abandoned) | None => StartPoll::Waiting,
        }
    }

    fn client(&mut self) -> Result<&KesticClient, String> {
        let client = match self.client.take() {
            Some(client) => client,
            None => make_client()?,
        };
        Ok(self.client.insert(client))
    }

    /// Fetches the page at `offset` filtered by `filter` (any case): `Ok(None)` while the query
    /// runs. A result the daemon freed, or a query that was cancelled, is `NotFound`; any other
    /// error carries its text as `Internal`.
    pub(crate) fn fetch(
        &mut self,
        offset: u64,
        filter: &str,
    ) -> Result<Option<SnapshotQueryResultDto>, KesticDbusError> {
        self.fetch_page(&SnapshotQueryPage {
            offset,
            limit: DEFAULT_PAGE_LIMIT,
            filter: PageFilter {
                text: filter.to_string(),
                kinds: Vec::new(),
            },
        })
    }

    /// Fetches `page` (offset, limit, text and kinds filter); errors as for [`Self::fetch`].
    pub(crate) fn fetch_page(
        &mut self,
        page: &SnapshotQueryPage,
    ) -> Result<Option<SnapshotQueryResultDto>, KesticDbusError> {
        let Some(query_id) = self.query_id.clone() else {
            return Ok(None);
        };
        let client = self.client().map_err(KesticDbusError::Internal)?;
        let status = client.get_snapshot_query_status(&query_id, page)?;
        match (status.state.as_str(), status.result) {
            ("running", _) => Ok(None),
            ("done", Some(result)) => Ok(Some(result)),
            // Only a query its model left is cancelled; treat it like a freed result.
            ("cancelled", _) => Err(KesticDbusError::NotFound(query_id)),
            _ => {
                let detail = status.error_detail.unwrap_or_default();
                let text = match status.error_kind {
                    Some(kind) => tr_text(&kind.text(Some(&detail))),
                    None => detail,
                };
                Err(KesticDbusError::Internal(text))
            }
        }
    }

    /// Cancels the query while it still runs, so restic stops; also one whose start has not
    /// arrived yet.
    pub(crate) fn abandon(&mut self) {
        let mut started = None;
        if let Some(slot) = self.start.take() {
            let mut guard = slot.lock_unpoisoned();
            match std::mem::replace(&mut *guard, QueryStart::Abandoned) {
                QueryStart::Started(id) => started = Some(id),
                QueryStart::Pending | QueryStart::Failed(_) | QueryStart::Abandoned => {}
            }
        }
        let query_id = self.query_id.take().or(started);
        if let Some(id) = query_id
            && let Ok(client) = self.client()
        {
            // A query that ended meanwhile needs no cancel; the answer is not needed.
            let _ = client.cancel_snapshot_query(&id);
        }
    }
}

/// The text of an error from [`QueryRun::fetch`].
pub(crate) fn fetch_error_text(err: KesticDbusError) -> String {
    match err {
        KesticDbusError::Internal(text) => text,
        other => dbus_error_text(&other),
    }
}

/// Number of entries line: «1000 entries», with a filter «37 of 1000».
pub(crate) fn count_text(total: u64, matched: u64, filtered: bool) -> String {
    if filtered {
        i18n("%1 of %2").subs(&[&format_count(matched), &format_count(total)])
    } else {
        i18np("%1 entry", "%1 entries", total, &[])
    }
}

/// Shown instead of a list too large to hold, with the number of its entries.
pub(crate) fn too_large_text(total: u64) -> String {
    i18n("Too many entries to show (%1). Choose a subfolder or narrow the search.")
        .subs(&[&format_count(total)])
}

/// `n` with the digit grouping of the locale.
pub(crate) fn format_count(n: u64) -> String {
    crate::qt_bridge::ffi::format_count(i64::try_from(n).unwrap_or(i64::MAX))
}

pub(crate) fn to_i32(n: u64) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}
