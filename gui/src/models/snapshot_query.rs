// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! One snapshot query of a list model (folder list, search hits, comparison, restore preview):
//! started on a worker thread so the window never waits for restic, polled by a timer, fetched
//! page by page, and cancelled when the view leaves it. Also the state machine and the texts
//! every query list shares.

use super::rows::row_count;
use crate::state_cache::{
    daemon_unavailable_text, dbus_error_text, i18n, i18np, make_client, tr_text,
};
use cxx_qt_lib::{QModelIndex, QString};
use kestic_core::query_list::{DEFAULT_PAGE_LIMIT, PageFilter, QueryState};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{
    KesticClient, KesticDbusError, SnapshotQueryPage, SnapshotQueryRequest, SnapshotQueryResultDto,
    SnapshotQueryStatusDto,
};
use kestic_i18n::codes::ErrorKind;
use std::pin::Pin;
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

/// What fetching a page of a query found.
pub(crate) enum Fetched<T> {
    /// The query still runs.
    Running,
    Page(T),
    /// The daemon freed the result, or the query was cancelled: it has to start again.
    Gone,
    Failed(QueryFailure),
}

/// Why a query, or the fetch of one of its pages, failed.
#[derive(Debug, PartialEq)]
pub(crate) struct QueryFailure {
    /// The text to show.
    pub(crate) text: String,
    /// The error code of a query that failed in the daemon.
    pub(crate) kind: Option<ErrorKind>,
}

impl QueryFailure {
    /// A failure known only by its text.
    pub(crate) fn from_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: None,
        }
    }
}

/// The state named by the `state` string of the query status.
fn query_state(wire_name: &str) -> Option<QueryState> {
    [
        QueryState::Running,
        QueryState::Done,
        QueryState::Cancelled,
        QueryState::Error,
    ]
    .into_iter()
    .find(|state| state.wire_name() == wire_name)
}

/// What the status of a query says about the requested page.
pub(super) fn status_outcome(status: SnapshotQueryStatusDto) -> Fetched<SnapshotQueryResultDto> {
    match (query_state(&status.state), status.result) {
        (Some(QueryState::Running), _) => Fetched::Running,
        (Some(QueryState::Done), Some(result)) => Fetched::Page(result),
        // Only a query its model left is cancelled; treat it like a freed result.
        (Some(QueryState::Cancelled), _) => Fetched::Gone,
        _ => {
            let detail = status.error_detail.unwrap_or_default();
            let text = match status.error_kind {
                Some(kind) => tr_text(&kind.text(Some(&detail))),
                None => detail,
            };
            Fetched::Failed(QueryFailure {
                text,
                kind: status.error_kind,
            })
        }
    }
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

    /// The daemon's ID of the started query.
    pub(crate) fn id(&self) -> Option<&str> {
        self.query_id.as_deref()
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

    /// Fetches `page` (offset, limit, text and kinds filter).
    pub(crate) fn fetch_page(
        &mut self,
        page: &SnapshotQueryPage,
    ) -> Fetched<SnapshotQueryResultDto> {
        let Some(query_id) = self.query_id.clone() else {
            return Fetched::Running;
        };
        let client = match self.client() {
            Ok(client) => client,
            Err(text) => return Fetched::Failed(QueryFailure::from_text(text)),
        };
        match client.get_snapshot_query_status(&query_id, page) {
            Ok(status) => status_outcome(status),
            Err(KesticDbusError::NotFound(_)) => Fetched::Gone,
            Err(err) => Fetched::Failed(QueryFailure::from_text(dbus_error_text(&err))),
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

/// The rows and the query of a list model that shows a snapshot query page by page.
#[derive(Default)]
pub(crate) struct QueryList<R> {
    pub(crate) rows: Vec<R>,
    /// `None` before the first query, and while a search has no text.
    pub(crate) state: Option<QueryState>,
    /// The filter the daemon applies to the pages.
    pub(crate) filter: PageFilter,
    pub(crate) run: QueryRun,
}

/// The Qt side of a query list model: its rows and the properties the state machine of
/// [`QueryListModel`] sets. Written by `query_list_model!`.
pub(super) trait QueryListView {
    type Row;

    fn list(&self) -> &QueryList<Self::Row>;

    fn list_mut(self: Pin<&mut Self>) -> &mut QueryList<Self::Row>;

    /// Whether the list is too large to hold, and how many entries match the filter.
    fn too_large_and_matched(&self) -> (bool, i32);

    /// Sets the `state` property.
    fn show_state(self: Pin<&mut Self>, state: QString);

    /// Sets the `errorText` property.
    fn show_error(self: Pin<&mut Self>, text: QString);

    /// Replaces all rows.
    fn reset_rows(self: Pin<&mut Self>, rows: Vec<Self::Row>);

    /// Appends `rows` (at least one) at the end.
    fn append_rows(self: Pin<&mut Self>, parent: &QModelIndex, rows: Vec<Self::Row>);
}

/// A list model that shows one snapshot query page by page: the query runs in the daemon, a
/// timer polls it, the first page arrives when it ends, further pages while the view scrolls,
/// and the daemon filters. A result the daemon freed starts the query again.
pub(super) trait QueryListModel: QueryListView {
    /// The result of this model's kind of query.
    type Page;

    /// The page of this model's kind of query; `None` for a result of another kind.
    fn page_of(result: SnapshotQueryResultDto) -> Option<Self::Page>;

    /// The rows of a further page.
    fn page_rows(&self, page: &Self::Page) -> Vec<Self::Row>;

    /// Shows the first page: after the query ended, or after the filter changed.
    fn show_first_page(self: Pin<&mut Self>, page: Self::Page);

    /// Starts the query again because the daemon no longer holds its result.
    fn restart(self: Pin<&mut Self>);

    fn query_state(&self) -> Option<QueryState> {
        self.list().state
    }

    fn set_query_state(mut self: Pin<&mut Self>, state: Option<QueryState>) {
        self.as_mut().list_mut().state = state;
        self.show_state(QString::from(state.map_or("", QueryState::wire_name)));
    }

    /// Empties the list and its filter before a new query.
    fn clear_list(mut self: Pin<&mut Self>) {
        self.as_mut().list_mut().filter = PageFilter::default();
        self.reset_rows(Vec::new());
    }

    /// Starts `request` on a worker thread; `state` is `running` until the first page arrives.
    fn start_query(mut self: Pin<&mut Self>, request: SnapshotQueryRequest) {
        self.as_mut().list_mut().run.start(request);
        self.set_query_state(Some(QueryState::Running));
    }

    /// Checks the running query (called by a timer while `state` is `running`).
    fn poll_query(mut self: Pin<&mut Self>) {
        match self.as_mut().list_mut().run.poll_start() {
            StartPoll::Waiting => {}
            StartPoll::Failed(text) => self.fail_query(&QueryFailure::from_text(text)),
            StartPoll::Started => self.load_first_page(),
        }
    }

    /// Cancels the query while it still runs, so restic stops.
    fn abandon_query(mut self: Pin<&mut Self>) {
        if self.query_state() != Some(QueryState::Running) {
            return;
        }
        self.as_mut().list_mut().run.abandon();
        self.set_query_state(Some(QueryState::Cancelled));
    }

    /// Shows `failure`; `state` becomes `error`.
    fn fail_query(mut self: Pin<&mut Self>, failure: &QueryFailure) {
        self.as_mut().note_failure(failure.kind);
        self.as_mut().show_error(QString::from(&failure.text));
        self.set_query_state(Some(QueryState::Error));
    }

    /// Called with the error code of each failure; a model that tells failures apart keeps it.
    fn note_failure(self: Pin<&mut Self>, _kind: Option<ErrorKind>) {}

    /// Fetches `page` of the query.
    fn fetch(self: Pin<&mut Self>, page: &SnapshotQueryPage) -> Fetched<Self::Page> {
        match self.list_mut().run.fetch_page(page) {
            Fetched::Page(result) => Self::page_of(result).map_or_else(
                || Fetched::Failed(QueryFailure::from_text("unexpected snapshot query result")),
                Fetched::Page,
            ),
            Fetched::Running => Fetched::Running,
            Fetched::Gone => Fetched::Gone,
            Fetched::Failed(failure) => Fetched::Failed(failure),
        }
    }

    /// Fetches the page at `offset` with the current filter.
    fn fetch_at(self: Pin<&mut Self>, offset: u64) -> Fetched<Self::Page> {
        let page = SnapshotQueryPage {
            offset,
            limit: DEFAULT_PAGE_LIMIT,
            filter: self.list().filter.clone(),
        };
        self.fetch(&page)
    }

    fn load_first_page(mut self: Pin<&mut Self>) {
        match self.as_mut().fetch_at(0) {
            Fetched::Running => {}
            Fetched::Page(page) => self.show_first_page(page),
            Fetched::Gone => self.restart(),
            Fetched::Failed(failure) => self.fail_query(&failure),
        }
    }

    fn reload_if_done(self: Pin<&mut Self>) {
        if self.query_state() == Some(QueryState::Done) {
            self.load_first_page();
        }
    }

    /// Filters the entries by `text` (any case); the daemon filters.
    fn set_filter_text(mut self: Pin<&mut Self>, text: String) {
        if text == self.list().filter.text {
            return;
        }
        self.as_mut().list_mut().filter.text = text;
        self.reload_if_done();
    }

    /// Shows only the entries of the kinds `kinds`; none shows all.
    fn set_filter_kinds(mut self: Pin<&mut Self>, kinds: Vec<String>) {
        if kinds == self.list().filter.kinds {
            return;
        }
        self.as_mut().list_mut().filter.kinds = kinds;
        self.reload_if_done();
    }

    /// Whether the daemon holds further pages of a finished query.
    fn can_fetch_page(&self, parent: &QModelIndex) -> bool {
        let (too_large, matched) = self.too_large_and_matched();
        !parent.is_valid()
            && self.query_state() == Some(QueryState::Done)
            && !too_large
            && row_count(&self.list().rows) < matched
    }

    /// Appends the next page (at most [`DEFAULT_PAGE_LIMIT`] entries) at the end of the list.
    fn fetch_next_page(mut self: Pin<&mut Self>, parent: &QModelIndex) {
        if !self.can_fetch_page(parent) {
            return;
        }
        let offset = self.list().rows.len() as u64;
        match self.as_mut().fetch_at(offset) {
            Fetched::Page(page) => {
                let rows = self.page_rows(&page);
                if !rows.is_empty() {
                    self.append_rows(parent, rows);
                }
            }
            Fetched::Running => {}
            Fetched::Gone => self.restart(),
            Fetched::Failed(failure) => self.fail_query(&failure),
        }
    }
}

/// Implements [`QueryListView`] for `qobject::<Model>` (whose Rust struct holds the
/// [`QueryList`] in `list` and the properties `too_large` and `matched`) and the QML-facing
/// functions every query list shares: `poll`, `cancel`, `setFilter`, `canFetchMore`,
/// `fetchMore`, `rowCount`, `roleNames` and `data`.
macro_rules! query_list_model {
    (
        $model:ident, $row:ty,
        begin_reset: $begin_reset:ident,
        end_reset: $end_reset:ident,
        begin_insert: $begin_insert:ident,
        end_insert: $end_insert:ident $(,)?
    ) => {
        impl $crate::models::snapshot_query::QueryListView for qobject::$model {
            type Row = $row;

            fn list(&self) -> &$crate::models::snapshot_query::QueryList<$row> {
                &self.list
            }

            fn list_mut(
                self: ::std::pin::Pin<&mut Self>,
            ) -> &mut $crate::models::snapshot_query::QueryList<$row> {
                &mut ::cxx_qt::CxxQtType::rust_mut(self).get_mut().list
            }

            fn too_large_and_matched(&self) -> (bool, i32) {
                (self.too_large, self.matched)
            }

            fn show_state(self: ::std::pin::Pin<&mut Self>, state: ::cxx_qt_lib::QString) {
                self.set_state(state);
            }

            fn show_error(self: ::std::pin::Pin<&mut Self>, text: ::cxx_qt_lib::QString) {
                self.set_error_text(text);
            }

            fn reset_rows(mut self: ::std::pin::Pin<&mut Self>, rows: Vec<$row>) {
                self.as_mut().$begin_reset();
                self.as_mut().list_mut().rows = rows;
                self.as_mut().$end_reset();
            }

            fn append_rows(
                mut self: ::std::pin::Pin<&mut Self>,
                parent: &::cxx_qt_lib::QModelIndex,
                rows: Vec<$row>,
            ) {
                let first = $crate::models::rows::row_count(&self.list.rows);
                let last = first + $crate::models::rows::row_count(&rows) - 1;
                self.as_mut().$begin_insert(parent, first, last);
                self.as_mut().list_mut().rows.extend(rows);
                self.as_mut().$end_insert();
            }
        }

        impl qobject::$model {
            /// Checks the running query (called by a timer while `state` is `running`).
            pub fn poll(self: ::std::pin::Pin<&mut Self>) {
                $crate::models::snapshot_query::QueryListModel::poll_query(self);
            }

            /// Stops the running query (the user cancels it or the view closes).
            pub fn cancel(self: ::std::pin::Pin<&mut Self>) {
                $crate::models::snapshot_query::QueryListModel::abandon_query(self);
            }

            /// Filters the entries by `text` (any case); the daemon filters.
            pub fn set_filter(self: ::std::pin::Pin<&mut Self>, text: &::cxx_qt_lib::QString) {
                $crate::models::snapshot_query::QueryListModel::set_filter_text(
                    self,
                    text.to_string(),
                );
            }

            pub fn can_fetch_more(&self, parent: &::cxx_qt_lib::QModelIndex) -> bool {
                $crate::models::snapshot_query::QueryListModel::can_fetch_page(self, parent)
            }

            /// Appends the next page at the end of the list.
            pub fn fetch_more(
                self: ::std::pin::Pin<&mut Self>,
                parent: &::cxx_qt_lib::QModelIndex,
            ) {
                $crate::models::snapshot_query::QueryListModel::fetch_next_page(self, parent);
            }

            pub fn row_count(&self, _parent: &::cxx_qt_lib::QModelIndex) -> i32 {
                $crate::models::rows::row_count(&self.list.rows)
            }

            pub fn role_names(
                &self,
            ) -> ::cxx_qt_lib::QHash<::cxx_qt_lib::QHashPair_i32_QByteArray> {
                $crate::models::rows::role_names::<$row>()
            }

            pub fn data(
                &self,
                index: &::cxx_qt_lib::QModelIndex,
                role: i32,
            ) -> ::cxx_qt_lib::QVariant {
                $crate::models::rows::row_data(&self.list.rows, index, role)
            }
        }
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use kestic_daemon::dbus_api::{ListPageDto, LsResultDto};

    fn status(state: &str, result: Option<SnapshotQueryResultDto>) -> SnapshotQueryStatusDto {
        SnapshotQueryStatusDto {
            state: state.to_string(),
            error_kind: None,
            error_detail: None,
            result,
        }
    }

    fn ls_result() -> SnapshotQueryResultDto {
        SnapshotQueryResultDto::Ls(LsResultDto {
            dir: "/home/u".to_string(),
            dir_exists: true,
            entries: ListPageDto {
                items: Vec::new(),
                offset: 0,
                total: 0,
                matched: 0,
                too_large: false,
            },
        })
    }

    /// The query status decides what a fetch found: (a) a running query has no page yet, (b) a
    /// finished one hands over its result, (c) a cancelled one has to start again, and (d) a
    /// failed one carries its text and error code.
    #[test]
    fn query_status_decides_what_a_fetch_found() {
        assert!(
            matches!(status_outcome(status("running", None)), Fetched::Running),
            "(a)"
        );
        assert!(
            matches!(
                status_outcome(status("done", Some(ls_result()))),
                Fetched::Page(SnapshotQueryResultDto::Ls(_))
            ),
            "(b)"
        );
        assert!(
            matches!(status_outcome(status("cancelled", None)), Fetched::Gone),
            "(c)"
        );
        let mut failed = status("error", None);
        failed.error_kind = Some(ErrorKind::Unknown);
        failed.error_detail = Some("restic crashed".to_string());
        let Fetched::Failed(failure) = status_outcome(failed) else {
            panic!("(d) a failed query is a failure");
        };
        assert_eq!(failure.kind, Some(ErrorKind::Unknown), "(d) error code");
        assert_eq!(failure.text, "restic crashed", "(d) text");
    }
}
