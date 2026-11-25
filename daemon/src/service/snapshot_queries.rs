// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Snapshot queries (`StartSnapshotQuery`, `GetSnapshotQueryStatus`, `CancelSnapshotQuery`):
//! restic runs on a worker thread, the daemon holds the complete result and hands its list out
//! page by page.

use super::error::{KesticServiceErr, backup_run_error_kind, invalid};
use super::restore_preview::{
    HeldPreview, RestoreContext, RestoreParams, execute_restore_preview, parse_preview_request,
};
use super::state::{DaemonCore, probe_for_run};
use crate::dbus_api::{
    DiffResultDto, FindResultDto, LsResultDto, SnapshotQueryRequest, SnapshotQueryResultDto,
    SnapshotQueryStatusDto,
};
use kestic_core::backup::{BackupRunError, SnapshotListAccess};
use kestic_core::model::{DestinationId, RunId};
use kestic_core::query_list::{HeldResult, PageRequest, QueryOutcome};
use kestic_core::restic::CancelHandle;
use kestic_core::restic_gate::ResticBinaryGate;
use kestic_core::snapshot_diff::{DiffResult, execute_diff};
use kestic_core::snapshot_find::{
    FindRequest, FindResult, FindScope, execute_find, validate_search_text,
};
use kestic_core::snapshot_ls::{
    LsListing, execute_ls, normalize_snapshot_path, validate_snapshot_id,
};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::{ErrorKind, RestoreOutcome, SnapshotChange, SnapshotQueryKind};
use std::sync::{Arc, Mutex};
use std::thread;

/// The finished result of a snapshot query.
pub(crate) enum QueryResult {
    Ls(LsListing),
    Find(FindResult),
    Diff(DiffResult),
    RestorePreview(Box<HeldPreview>),
}

impl HeldResult for QueryResult {
    fn held_bytes(&self) -> u64 {
        match self {
            Self::Ls(listing) => listing.held_bytes(),
            Self::Find(found) => found.held_bytes(),
            Self::Diff(diff) => diff.held_bytes(),
            Self::RestorePreview(preview) => preview.held_bytes(),
        }
    }
}

/// A finished result as the daemon holds it: shared, so a page is cut without holding the lock
/// of the whole daemon; its memory is counted once, when it is stored.
#[derive(Clone)]
pub(crate) struct SharedResult {
    held: u64,
    inner: Arc<Mutex<QueryResult>>,
}

impl SharedResult {
    fn new(result: QueryResult) -> Self {
        Self {
            held: result.held_bytes(),
            inner: Arc::new(Mutex::new(result)),
        }
    }

    /// The result, locked.
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, QueryResult> {
        self.inner.lock_unpoisoned()
    }
}

impl HeldResult for SharedResult {
    fn held_bytes(&self) -> u64 {
        self.held
    }
}

/// The state of a query taken under the daemon's lock; [`QueryStatus::page`] cuts the page
/// afterwards.
pub(crate) struct QueryStatus {
    page: PageRequest,
    state: &'static str,
    error: Option<(ErrorKind, String)>,
    result: Option<SharedResult>,
}

impl QueryStatus {
    /// The status DTO with the requested page of a finished query.
    pub(crate) fn page(self) -> Result<SnapshotQueryStatusDto, KesticServiceErr> {
        let mut result = self.result.as_ref().map(SharedResult::lock);
        if let Some(r) = result.as_deref() {
            check_kinds(r, &self.page)?;
        }
        Ok(SnapshotQueryStatusDto {
            state: self.state.to_string(),
            error_kind: self.error.as_ref().map(|(kind, _)| *kind),
            error_detail: self.error.map(|(_, detail)| detail),
            result: result.as_deref_mut().map(|r| r.page(&self.page)),
        })
    }
}

/// Refuses a filter by kinds the result does not have.
fn check_kinds(result: &QueryResult, page: &PageRequest) -> Result<(), KesticServiceErr> {
    if page.filter.kinds.is_empty() {
        return Ok(());
    }
    match result {
        QueryResult::Ls(_) | QueryResult::Find(_) => Err(invalid(
            "folder lists and search hits have no kinds to filter by",
        )),
        QueryResult::Diff(_) => match page
            .filter
            .kinds
            .iter()
            .find(|k| SnapshotChange::parse(k).is_none())
        {
            Some(unknown) => Err(invalid(format!("unknown kind of change: {unknown}"))),
            None => Ok(()),
        },
        QueryResult::RestorePreview(_) => match page
            .filter
            .kinds
            .iter()
            .find(|k| RestoreOutcome::parse(k).is_none())
        {
            Some(unknown) => Err(invalid(format!("unknown restore outcome: {unknown}"))),
            None => Ok(()),
        },
    }
}

impl QueryResult {
    fn page(&mut self, page: &PageRequest) -> SnapshotQueryResultDto {
        match self {
            Self::Ls(listing) => SnapshotQueryResultDto::Ls(LsResultDto {
                dir: listing.dir.clone(),
                dir_exists: listing.dir_exists,
                entries: listing.entries.page(page),
            }),
            Self::Find(found) => SnapshotQueryResultDto::Find(FindResultDto {
                hits: found.page(page),
                snapshots_matched: found.snapshots_matched,
            }),
            Self::Diff(diff) => SnapshotQueryResultDto::Diff(DiffResultDto {
                changes: diff.page(page),
                added_files: diff.counts.added_files,
                removed_files: diff.counts.removed_files,
                changed_files: diff.counts.changed_files,
                bytes_added: diff.counts.bytes_added,
                bytes_removed: diff.counts.bytes_removed,
                identical: diff.identical(),
            }),
            Self::RestorePreview(preview) => {
                SnapshotQueryResultDto::RestorePreview(Box::new(preview.page(page)))
            }
        }
    }
}

/// What a validated request asks for.
enum QueryWork {
    Ls { snapshot_id: String, dir: String },
    Find { scope: FindScope, text: String },
    Diff { from: String, to: String },
    RestorePreview(RestoreParams),
}

/// A validated request.
struct ParsedQuery {
    destination_id: DestinationId,
    work: QueryWork,
}

fn parse_request(request_json: &str) -> Result<ParsedQuery, KesticServiceErr> {
    let request: SnapshotQueryRequest = serde_json::from_str(request_json)
        .map_err(|e| invalid(format!("invalid snapshot query JSON: {e}")))?;
    let destination_id = DestinationId::parse(&request.destination_id).map_err(invalid)?;
    let work = match request.kind {
        SnapshotQueryKind::Ls => {
            let snapshot_id = request
                .snapshot_id
                .ok_or_else(|| invalid("ls query without snapshot_id"))?;
            validate_snapshot_id(&snapshot_id).map_err(invalid)?;
            let dir = request.dir.ok_or_else(|| invalid("ls query without dir"))?;
            let dir = normalize_snapshot_path(&dir).map_err(invalid)?;
            QueryWork::Ls { snapshot_id, dir }
        }
        SnapshotQueryKind::Find => {
            let text = request
                .pattern
                .ok_or_else(|| invalid("find query without pattern"))?;
            validate_search_text(&text).map_err(invalid)?;
            let scope = request
                .scope
                .ok_or_else(|| invalid("find query without scope"))?;
            QueryWork::Find { scope, text }
        }
        SnapshotQueryKind::Diff => {
            let from = request
                .from_snapshot_id
                .ok_or_else(|| invalid("diff query without from_snapshot_id"))?;
            let to = request
                .to_snapshot_id
                .ok_or_else(|| invalid("diff query without to_snapshot_id"))?;
            validate_snapshot_id(&from).map_err(invalid)?;
            validate_snapshot_id(&to).map_err(invalid)?;
            QueryWork::Diff { from, to }
        }
        SnapshotQueryKind::RestorePreview => {
            QueryWork::RestorePreview(parse_preview_request(&request)?)
        }
    };
    Ok(ParsedQuery {
        destination_id,
        work,
    })
}

fn outcome_of(res: Result<SharedResult, BackupRunError>) -> QueryOutcome<SharedResult> {
    match res {
        Ok(result) => QueryOutcome::Done(result),
        Err(BackupRunError::Cancelled) => QueryOutcome::Cancelled,
        // The folder is the detail the text of `invalid_target` names.
        Err(BackupRunError::InvalidTarget { path, .. }) => {
            QueryOutcome::Error(ErrorKind::InvalidTarget, path)
        }
        Err(err) => QueryOutcome::Error(backup_run_error_kind(&err), err.to_string()),
    }
}

/// Starts a snapshot query on a worker thread and returns its ID. The request is checked first;
/// an invalid one is refused without running restic.
pub fn start_snapshot_query_async(
    shared: &Arc<Mutex<DaemonCore>>,
    request_json: &str,
) -> Result<String, KesticServiceErr> {
    let query = parse_request(request_json)?;
    // Held until the query has ended, so the app-managed restic is not replaced between its
    // restic processes.
    let run_lease = ResticBinaryGate::global().read();
    let query_id = RunId::generate()
        .map(|r| r.as_str().to_string())
        .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
    let cancel = CancelHandle::new();
    let (dest, deps, limit_bytes, configured, restore_ctx) = {
        let mut guard = shared.lock_unpoisoned();
        let dest = guard
            .config
            .destination(&query.destination_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(query.destination_id.to_string()))?;
        let configured: Vec<String> = guard
            .config
            .backups
            .iter()
            .map(|b| b.id.as_str().to_string())
            .collect();
        let now = guard.clock.now_utc();
        let limit_bytes = guard.snapshot_queries.limits().per_query_bytes;
        let restore_ctx =
            RestoreContext::from_config(&guard.config, dest.clone(), guard.timezone.clone())
                .with_kestic_dirs(&guard.config_path, &guard.state_dir);
        guard
            .snapshot_queries
            .register(query_id.clone(), cancel.clone(), now);
        (
            dest,
            guard.restic_deps(),
            limit_bytes,
            configured,
            restore_ctx,
        )
    };
    let resolved = match probe_for_run(&deps.provider, deps.secret_store.as_ref()).1 {
        Ok(resolved) => resolved,
        Err(err) => {
            shared.lock_unpoisoned().snapshot_queries.remove(&query_id);
            return Err(err);
        }
    };

    let shared_worker = Arc::clone(shared);
    let id = query_id.clone();
    thread::spawn(move || {
        let _run_lease = run_lease;
        let access = SnapshotListAccess {
            destination: &dest,
            resolved: &resolved,
            kestic_binary: &deps.kestic_binary,
            secret_store: deps.secret_store.as_ref(),
            clock: deps.clock.as_ref(),
            extra_env: &[],
        };
        let res = match &query.work {
            QueryWork::Ls { snapshot_id, dir } => {
                execute_ls(&access, snapshot_id, dir, Some(&cancel), limit_bytes)
                    .map(QueryResult::Ls)
            }
            QueryWork::Find { scope, text } => {
                let configured: Vec<&str> = configured.iter().map(String::as_str).collect();
                let request = FindRequest {
                    scope,
                    configured_backup_ids: &configured,
                    text,
                };
                execute_find(&access, &request, Some(&cancel), limit_bytes).map(QueryResult::Find)
            }
            QueryWork::Diff { from, to } => {
                execute_diff(&access, from, to, Some(&cancel), limit_bytes).map(QueryResult::Diff)
            }
            QueryWork::RestorePreview(params) => {
                execute_restore_preview(&access, &restore_ctx, params, Some(&cancel), limit_bytes)
                    .map(|p| QueryResult::RestorePreview(Box::new(p)))
            }
        };
        let outcome = outcome_of(res.map(SharedResult::new));
        let now = deps.clock.now_utc();
        shared_worker
            .lock_unpoisoned()
            .snapshot_queries
            .finish(&id, outcome, now);
    });
    Ok(query_id)
}

impl DaemonCore {
    /// State of query `query_id`, to cut the page `page_json` asks for (empty = the first page
    /// without filter) once the daemon's lock is released.
    pub(crate) fn snapshot_query_status(
        &mut self,
        query_id: &str,
        page_json: &str,
    ) -> Result<QueryStatus, KesticServiceErr> {
        let page = PageRequest::parse(page_json).map_err(invalid)?;
        let now = self.clock.now_utc();
        let query = self
            .snapshot_queries
            .fetch(query_id, now)
            .ok_or_else(|| KesticServiceErr::NotFound(query_id.to_string()))?;
        Ok(QueryStatus {
            page,
            state: query.state.wire_name(),
            error: query.error.clone(),
            result: query.result.clone(),
        })
    }

    /// Cancels the running query `query_id`.
    pub fn cancel_snapshot_query(&self, query_id: &str) -> Result<(), KesticServiceErr> {
        self.snapshot_queries
            .cancel_handle(query_id)
            .ok_or_else(|| KesticServiceErr::NotFound(query_id.to_string()))?
            .cancel();
        Ok(())
    }
}
