// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! One snapshot query of a list model (folder list, search hits, comparison, restore preview):
//! started and fetched off the Qt thread so the window never waits for restic, polled by a
//! timer, fetched page by page, and cancelled when the view leaves it. Also the state machine and the texts
//! every query list shares.

use super::rows::row_count;
use crate::backend::{Lane, call_daemon_on, send_to_daemon};
use crate::state_cache::{daemon_unavailable_text, dbus_error_text, i18n, i18np, tr_text};
use cxx_qt::Threading;
use cxx_qt_lib::{QModelIndex, QString};
use kestic_core::query_list::{DEFAULT_PAGE_LIMIT, PageFilter, QueryState};
use kestic_daemon::dbus_api::{
    KesticDbusError, SnapshotQueryPage, SnapshotQueryRequest, SnapshotQueryResultDto,
    SnapshotQueryStatusDto,
};
use kestic_i18n::codes::ErrorKind;
use std::pin::Pin;

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

/// The query of a list model as far as it has got. The daemon's answers arrive on the Qt thread;
/// an answer for an earlier query, or for a page asked for before the filter changed, is
/// ignored.
#[derive(Default)]
pub(crate) struct QueryRun {
    /// The daemon's ID of the started query.
    query_id: Option<String>,
    /// Counts the starts and abandons: an answer to an earlier start is ignored.
    generation: u64,
    /// Counts the first pages asked for: an answer to an earlier one is ignored.
    page_generation: u64,
    /// The first page is asked for and has not arrived yet (the timer asks no second time).
    first_page_pending: bool,
    /// A further page is asked for and has not arrived yet.
    next_page_pending: bool,
}

impl QueryRun {
    /// The daemon's ID of the started query.
    pub(crate) fn id(&self) -> Option<&str> {
        self.query_id.as_deref()
    }

    /// Whether the first page (after the query ended or the filter changed) is on its way.
    pub(crate) fn loading_first_page(&self) -> bool {
        self.first_page_pending
    }

    /// Cancels the query while it still runs, so restic stops; one whose start has not arrived
    /// yet is cancelled once it does.
    pub(crate) fn abandon(&mut self) {
        self.generation += 1;
        self.first_page_pending = false;
        self.next_page_pending = false;
        if let Some(id) = self.query_id.take() {
            // A query that ended meanwhile needs no cancel; the answer is not needed.
            send_to_daemon(Lane::Alone, move |client| {
                let _ = client.cancel_snapshot_query(&id);
            });
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
pub(super) trait QueryListModel: QueryListView + Threading + Sized + 'static {
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

    /// Starts `request` off the Qt thread; `state` is `running` until the first page arrives.
    fn start_query(mut self: Pin<&mut Self>, request: SnapshotQueryRequest) {
        let generation = {
            let run = &mut self.as_mut().list_mut().run;
            run.abandon();
            run.generation
        };
        self.as_mut().set_query_state(Some(QueryState::Running));
        call_daemon_on(
            &self.qt_thread(),
            Lane::Alone,
            move |client| client.start_snapshot_query(&request),
            |_| false,
            move |mut model: Pin<&mut Self>, started| {
                let current = model.list().run.generation == generation;
                match (current, started) {
                    (false, Ok(Ok(id))) => send_to_daemon(Lane::Alone, move |client| {
                        // Nobody waits for the answer; a query that already ended needs nothing.
                        let _ = client.cancel_snapshot_query(&id);
                    }),
                    (false, _) => {}
                    (true, started) => match started {
                        Ok(Ok(id)) => {
                            model.as_mut().list_mut().run.query_id = Some(id);
                            model.load_first_page();
                        }
                        Ok(Err(err)) => {
                            model.fail_query(&QueryFailure::from_text(dbus_error_text(&err)));
                        }
                        Err(_) => {
                            model.fail_query(&QueryFailure::from_text(daemon_unavailable_text()));
                        }
                    },
                }
            },
        );
    }

    /// Checks the running query (called by a timer while `state` is `running`): asks for the
    /// first page once the query started, unless that answer is still on its way.
    fn poll_query(self: Pin<&mut Self>) {
        let run = &self.list().run;
        if run.query_id.is_some() && !run.first_page_pending {
            self.load_first_page();
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

    /// Asks for the page at `offset` with the current filter; `then` gets what arrived, on the
    /// Qt thread, unless the query or the filter changed meanwhile (`first`: the first page, which
    /// a newer first page replaces).
    fn fetch_at(
        mut self: Pin<&mut Self>,
        offset: u64,
        first: bool,
        then: fn(Pin<&mut Self>, Fetched<Self::Page>),
    ) {
        let Some(query_id) = self.list().run.query_id.clone() else {
            return;
        };
        let page = SnapshotQueryPage {
            offset,
            limit: DEFAULT_PAGE_LIMIT,
            filter: self.list().filter.clone(),
        };
        let (generation, page_generation) = {
            let run = &mut self.as_mut().list_mut().run;
            if first {
                run.page_generation += 1;
                run.first_page_pending = true;
            } else {
                run.next_page_pending = true;
            }
            (run.generation, run.page_generation)
        };
        call_daemon_on(
            &self.qt_thread(),
            Lane::Alone,
            move |client| client.get_snapshot_query_status(&query_id, &page),
            |_| false,
            move |mut model: Pin<&mut Self>, status| {
                if model.list().run.generation != generation {
                    return;
                }
                {
                    let run = &mut model.as_mut().list_mut().run;
                    // A newer first page (the filter changed) replaces this answer; a next page
                    // has arrived either way, so further pages can be asked for.
                    if !first {
                        run.next_page_pending = false;
                    }
                    if run.page_generation != page_generation {
                        return;
                    }
                    if first {
                        run.first_page_pending = false;
                    }
                }
                let fetched = match status {
                    Ok(Ok(status)) => match status_outcome(status) {
                        Fetched::Page(result) => Self::page_of(result).map_or_else(
                            || {
                                Fetched::Failed(QueryFailure::from_text(
                                    "unexpected snapshot query result",
                                ))
                            },
                            Fetched::Page,
                        ),
                        Fetched::Running => Fetched::Running,
                        Fetched::Gone => Fetched::Gone,
                        Fetched::Failed(failure) => Fetched::Failed(failure),
                    },
                    Ok(Err(KesticDbusError::NotFound(_))) => Fetched::Gone,
                    Ok(Err(err)) => Fetched::Failed(QueryFailure::from_text(dbus_error_text(&err))),
                    Err(_) => Fetched::Failed(QueryFailure::from_text(daemon_unavailable_text())),
                };
                then(model, fetched);
            },
        );
    }

    /// Fetches `pages` of the finished query off the Qt thread, one after the other; `then` gets
    /// them on the Qt thread (`Running` when the query has not finished), unless the query
    /// changed meanwhile.
    fn fetch_pages(
        self: Pin<&mut Self>,
        pages: Vec<SnapshotQueryPage>,
        then: impl FnOnce(Pin<&mut Self>, Fetched<Vec<Self::Page>>) + Send + 'static,
    ) {
        let Some(query_id) = self.list().run.query_id.clone() else {
            return;
        };
        let generation = self.list().run.generation;
        call_daemon_on(
            &self.qt_thread(),
            Lane::Alone,
            move |client| {
                let mut results = Vec::with_capacity(pages.len());
                for page in &pages {
                    match client.get_snapshot_query_status(&query_id, page) {
                        Ok(status) => match status_outcome(status) {
                            Fetched::Page(result) => results.push(result),
                            Fetched::Running => return Fetched::Running,
                            Fetched::Gone => return Fetched::Gone,
                            Fetched::Failed(failure) => return Fetched::Failed(failure),
                        },
                        Err(KesticDbusError::NotFound(_)) => return Fetched::Gone,
                        Err(err) => {
                            return Fetched::Failed(QueryFailure::from_text(dbus_error_text(&err)));
                        }
                    }
                }
                Fetched::Page(results)
            },
            |_| false,
            move |model: Pin<&mut Self>, fetched| {
                if model.list().run.generation != generation {
                    return;
                }
                let fetched = match fetched {
                    Ok(Fetched::Page(results)) => results
                        .into_iter()
                        .map(Self::page_of)
                        .collect::<Option<Vec<_>>>()
                        .map_or_else(
                            || {
                                Fetched::Failed(QueryFailure::from_text(
                                    "unexpected snapshot query result",
                                ))
                            },
                            Fetched::Page,
                        ),
                    Ok(Fetched::Running) => Fetched::Running,
                    Ok(Fetched::Gone) => Fetched::Gone,
                    Ok(Fetched::Failed(failure)) => Fetched::Failed(failure),
                    Err(_) => Fetched::Failed(QueryFailure::from_text(daemon_unavailable_text())),
                };
                then(model, fetched);
            },
        );
    }

    /// Asks for the first page; a query that still runs is asked again by the timer.
    fn load_first_page(self: Pin<&mut Self>) {
        self.fetch_at(0, true, |model, fetched| match fetched {
            Fetched::Running => {}
            Fetched::Page(page) => model.show_first_page(page),
            Fetched::Gone => model.restart(),
            Fetched::Failed(failure) => model.fail_query(&failure),
        });
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

    /// Whether the daemon holds further pages of a finished query (also while one is on its way,
    /// so a view that asks again keeps waiting for it; none while the first page of a new filter
    /// is on its way).
    fn can_fetch_page(&self, parent: &QModelIndex) -> bool {
        let (too_large, matched) = self.too_large_and_matched();
        !parent.is_valid()
            && self.query_state() == Some(QueryState::Done)
            && !self.list().run.first_page_pending
            && !too_large
            && row_count(&self.list().rows) < matched
    }

    /// Appends the next page (at most [`DEFAULT_PAGE_LIMIT`] entries) at the end of the list
    /// once it arrived.
    fn fetch_next_page(self: Pin<&mut Self>, parent: &QModelIndex) {
        // One further page at a time; asking again while it is on its way changes nothing.
        if !self.can_fetch_page(parent) || self.list().run.next_page_pending {
            return;
        }
        let offset = self.list().rows.len() as u64;
        self.fetch_at(offset, false, |mut model, fetched| match fetched {
            Fetched::Page(page) => {
                let rows = model.page_rows(&page);
                if !rows.is_empty() {
                    model.as_mut().append_rows(&QModelIndex::default(), rows);
                }
            }
            Fetched::Running => {}
            Fetched::Gone => model.restart(),
            Fetched::Failed(failure) => model.fail_query(&failure),
        });
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
