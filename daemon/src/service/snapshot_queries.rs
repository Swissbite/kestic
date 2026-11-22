// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Snapshot queries (`StartSnapshotQuery`, `GetSnapshotQueryStatus`, `CancelSnapshotQuery`):
//! restic runs on a worker thread, the daemon holds the complete result and hands its list out
//! page by page.

use super::error::{KesticServiceErr, backup_run_error_kind};
use super::state::{DaemonCore, probe_for_run};
use crate::dbus_api::{
    FindResultDto, LsResultDto, SnapshotQueryRequest, SnapshotQueryResultDto,
    SnapshotQueryStatusDto,
};
use kestic_core::backup::{BackupRunError, SnapshotListAccess};
use kestic_core::model::{DestinationId, RunId};
use kestic_core::query_list::{HeldResult, PageRequest, QueryOutcome};
use kestic_core::restic::CancelHandle;
use kestic_core::snapshot_find::{
    FindRequest, FindResult, FindScope, execute_find, validate_search_text,
};
use kestic_core::snapshot_ls::{
    LsListing, execute_ls, normalize_snapshot_path, validate_snapshot_id,
};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::SnapshotQueryKind;
use std::sync::{Arc, Mutex};
use std::thread;

/// The finished result of a snapshot query.
pub(crate) enum QueryResult {
    Ls(LsListing),
    Find(FindResult),
}

impl HeldResult for QueryResult {
    fn held_bytes(&self) -> u64 {
        match self {
            Self::Ls(listing) => listing.held_bytes(),
            Self::Find(found) => found.held_bytes(),
        }
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
                hits: found.hits.page(page),
                snapshots_matched: found.snapshots_matched,
            }),
        }
    }
}

/// What a validated request asks for.
enum QueryWork {
    Ls { snapshot_id: String, dir: String },
    Find { scope: FindScope, text: String },
}

/// A validated request.
struct ParsedQuery {
    destination_id: DestinationId,
    work: QueryWork,
}

fn invalid(message: impl std::fmt::Display) -> KesticServiceErr {
    KesticServiceErr::InvalidInput(message.to_string())
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
        other => {
            return Err(invalid(format!(
                "snapshot query kind not supported yet: {}",
                other.as_str()
            )));
        }
    };
    Ok(ParsedQuery {
        destination_id,
        work,
    })
}

fn outcome_of(res: Result<QueryResult, BackupRunError>) -> QueryOutcome<QueryResult> {
    match res {
        Ok(result) => QueryOutcome::Done(result),
        Err(BackupRunError::Cancelled) => QueryOutcome::Cancelled,
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
    let query_id = RunId::generate()
        .map(|r| r.as_str().to_string())
        .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
    let cancel = CancelHandle::new();
    let (dest, deps, limit_bytes, configured) = {
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
        guard
            .snapshot_queries
            .register(query_id.clone(), cancel.clone(), now);
        (dest, guard.restic_deps(), limit_bytes, configured)
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
        };
        let outcome = outcome_of(res);
        let now = deps.clock.now_utc();
        shared_worker
            .lock_unpoisoned()
            .snapshot_queries
            .finish(&id, outcome, now);
    });
    Ok(query_id)
}

impl DaemonCore {
    /// State of query `query_id` and, once done, the page `page_json` asks for (empty = the
    /// first page without filter).
    pub fn get_snapshot_query_status(
        &mut self,
        query_id: &str,
        page_json: &str,
    ) -> Result<SnapshotQueryStatusDto, KesticServiceErr> {
        let page = PageRequest::parse(page_json).map_err(invalid)?;
        let now = self.clock.now_utc();
        let query = self
            .snapshot_queries
            .fetch(query_id, now)
            .ok_or_else(|| KesticServiceErr::NotFound(query_id.to_string()))?;
        if let (Some(QueryResult::Ls(_) | QueryResult::Find(_)), false) =
            (&query.result, page.filter.kinds.is_empty())
        {
            return Err(invalid(
                "folder lists and search hits have no kinds to filter by",
            ));
        }
        Ok(SnapshotQueryStatusDto {
            state: query.state.wire_name().to_string(),
            error_kind: query.error.as_ref().map(|(kind, _)| *kind),
            error_detail: query.error.as_ref().map(|(_, detail)| detail.clone()),
            result: query.result.as_mut().map(|r| r.page(&page)),
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
