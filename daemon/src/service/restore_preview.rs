// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `restore_preview` snapshot query: target suggestion, target check and, at the original
//! location or in a folder with content, the dry runs of `restic restore`. A finished preview
//! keeps the parameters it was made for, so `StartRestore` can tell whether it fits.

use super::error::KesticServiceErr;
use crate::dbus_api::{
    RestorePreviewResultDto, RestorePreviewTargetDto, RestoreSuggestionDto, RestoreTargetCheckDto,
    SnapshotQueryRequest,
};
use crate::i18n::i18n;
use kestic_core::backup::{BackupRunError, SnapshotListAccess};
use kestic_core::config::AppConfig;
use kestic_core::model::{BackupId, Destination, DestinationKind};
use kestic_core::mount::current_home_dir;
use kestic_core::query_list::PageRequest;
use kestic_core::restic::CancelHandle;
use kestic_core::restore::{
    EntryKind, RestorePlan, RestoreTarget, folder_instead_of_unmatchable, normalize_selection,
    plan_restore, probe_path, probe_snapshot_path, selected_paths,
};
use kestic_core::restore_preview::{
    PreviewClassification, PreviewRun, SnapshotTree, execute_preview, walk_folder,
};
use kestic_core::restore_target::{
    FolderWords, PreviewFingerprint, SuggestionInput, TargetCheck, check_target, current_uid,
    original_location, preview_required, repositories_below, suggested_new_folder,
};
use kestic_core::scheduler::{TimezoneSpec, to_local_offset};
use kestic_core::snapshot::ResticSnapshot;
use kestic_core::snapshot_ls::{normalize_snapshot_path, start_dir, validate_snapshot_id};
use kestic_i18n::codes::{RestoreOverwrite, RestoreTargetCode};
use std::path::PathBuf;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A checked restore request: full snapshot ID, normalised selection and target.
#[derive(Debug, Clone)]
pub(crate) struct RestoreParams {
    pub snapshot_id: String,
    pub selection: Vec<String>,
    pub target: Option<RestoreTarget>,
    pub overwrite: RestoreOverwrite,
}

fn invalid(message: impl std::fmt::Display) -> KesticServiceErr {
    KesticServiceErr::InvalidInput(message.to_string())
}

/// Checks the snapshot ID, the selection and the target folder of a restore or its preview;
/// an empty, relative path, one with a `..` segment or a NUL is refused before restic runs.
pub(crate) fn check_params(
    snapshot_id: &str,
    selection: &[String],
    target: Option<&RestoreTarget>,
    overwrite: RestoreOverwrite,
) -> Result<RestoreParams, KesticServiceErr> {
    validate_snapshot_id(snapshot_id).map_err(invalid)?;
    // restic cannot match a name that is not UTF-8; its folder can be restored instead.
    if let Some((path, folder)) = selection
        .iter()
        .find_map(|p| folder_instead_of_unmatchable(p).map(|f| (p, f)))
    {
        return Err(invalid(format!(
            "{path}: the name is not valid UTF-8; restore its folder {folder} instead"
        )));
    }
    let selection = normalize_selection(selection).map_err(invalid)?;
    let target = target
        .map(|t| {
            let dir = normalize_snapshot_path(t.dir()).map_err(invalid)?;
            if dir == "/" {
                return Err(invalid("/ is never a restore target"));
            }
            Ok(match t {
                RestoreTarget::NewFolder { .. } => RestoreTarget::NewFolder { dir },
                RestoreTarget::Original { .. } => RestoreTarget::Original { dir },
            })
        })
        .transpose()?;
    Ok(RestoreParams {
        snapshot_id: snapshot_id.to_string(),
        selection,
        target,
        overwrite,
    })
}

/// The `restore_preview` fields of a snapshot query request.
pub(crate) fn parse_preview_request(
    request: &SnapshotQueryRequest,
) -> Result<RestoreParams, KesticServiceErr> {
    let snapshot_id = request
        .snapshot_id
        .as_deref()
        .ok_or_else(|| invalid("restore_preview query without snapshot_id"))?;
    check_params(
        snapshot_id,
        request.selection.as_deref().unwrap_or_default(),
        request.target.as_ref(),
        request.overwrite.unwrap_or_default(),
    )
}

/// What a preview or a restore needs to know about the configuration.
#[derive(Clone)]
pub(crate) struct RestoreContext {
    pub destination: Destination,
    /// `(id, name)` of every configured backup.
    pub backups: Vec<(String, String)>,
    /// The repository folders of all local destinations; a restore never writes into them.
    pub repositories: Vec<PathBuf>,
    pub timezone: TimezoneSpec,
}

impl RestoreContext {
    pub(crate) fn from_config(
        config: &AppConfig,
        destination: Destination,
        timezone: TimezoneSpec,
    ) -> Self {
        Self {
            backups: config
                .backups
                .iter()
                .map(|b| (b.id.as_str().to_string(), b.name.clone()))
                .collect(),
            repositories: config
                .destinations
                .iter()
                .chain(std::iter::once(&destination))
                .filter_map(|d| match &d.kind {
                    DestinationKind::Local { path } => Some(path.clone()),
                    DestinationKind::Rest { .. } => None,
                })
                .collect(),
            destination,
            timezone,
        }
    }

    /// The configured backup the snapshot belongs to (its only configured `kestic:` tag).
    pub(crate) fn backup_of(&self, snapshot: &ResticSnapshot) -> Option<(BackupId, String)> {
        let configured: Vec<&str> = self.backups.iter().map(|(id, _)| id.as_str()).collect();
        let id = snapshot.assigned_backup(&configured)?;
        let name = self
            .backups
            .iter()
            .find(|(bid, _)| bid == id)
            .map(|(_, name)| name.clone())?;
        Some((BackupId::parse(id).ok()?, name))
    }

    /// The repository folders of all local destinations; a restore never writes into them.
    pub(crate) fn repositories(&self) -> &[PathBuf] {
        &self.repositories
    }

    /// The suggested new folder for a restore of `snapshot`.
    fn suggestion(&self, snapshot: &ResticSnapshot) -> PathBuf {
        let home = current_home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let restored = i18n("Restored").to_string();
        let not_assigned = i18n("not assigned").to_string();
        let backup = self.backup_of(snapshot);
        let time = OffsetDateTime::parse(&snapshot.time, &Rfc3339)
            .map_or(OffsetDateTime::UNIX_EPOCH, |t| {
                to_local_offset(t, &self.timezone)
            });
        let short_id = snapshot
            .short_id
            .clone()
            .unwrap_or_else(|| snapshot.id.chars().take(8).collect());
        suggested_new_folder(&SuggestionInput {
            home: &home,
            words: FolderWords {
                restored: &restored,
                not_assigned: &not_assigned,
            },
            backup_name: backup.as_ref().map(|(_, name)| name.as_str()),
            destination_name: &self.destination.name,
            snapshot_short_id: &short_id,
            snapshot_time: time,
        })
    }
}

/// A finished preview with the parameters it was made for.
pub(crate) struct HeldPreview {
    pub fingerprint: PreviewFingerprint,
    pub plan: RestorePlan,
    pub suggestion: RestoreSuggestionDto,
    pub target: RestoreTarget,
    pub check: TargetCheck,
    pub classification: PreviewClassification,
    /// The dry runs ran (the target needed a preview and could be written to).
    pub dry_run: bool,
    /// The configured backup of the snapshot, if any.
    pub backup: Option<(BackupId, String)>,
    /// A restore was accepted with this preview; it does not count for another one.
    pub used: bool,
}

impl HeldPreview {
    pub(crate) fn held_bytes(&self) -> u64 {
        kestic_core::query_list::HeldResult::held_bytes(&self.classification)
    }

    pub(crate) fn page(&mut self, page: &PageRequest) -> RestorePreviewResultDto {
        let counts = self.classification.counts;
        RestorePreviewResultDto {
            suggestion: self.suggestion.clone(),
            target: RestorePreviewTargetDto {
                target: self.target.clone(),
                check: RestoreTargetCheckDto {
                    code: self.check.code,
                    detail: self.check.resolved.to_string_lossy().into_owned(),
                },
            },
            new_files: counts.new_files,
            overwritten_files: counts.overwritten_files,
            kept_files: counts.kept_files,
            identical_files: counts.identical_files,
            bytes_to_write: counts.bytes_to_write,
            entries: self.classification.page(page),
            notes: self.classification.notes.clone(),
        }
    }
}

/// The plan of a restore: reads the snapshot and the kind of the probed path first.
pub(crate) fn resolve_plan(
    access: &SnapshotListAccess<'_>,
    params: &RestoreParams,
    cancel: Option<&CancelHandle>,
) -> Result<(ResticSnapshot, RestorePlan), BackupRunError> {
    let probe = probe_path(&params.selection);
    let (snapshot, kind) = probe_snapshot_path(access, &params.snapshot_id, &probe, cancel)?;
    let mut plan = plan_restore(&params.snapshot_id, &params.selection, kind)?;
    if params.selection.is_empty() {
        // The whole snapshot: its common folder from the tree, `/` when the tree does not hold
        // the folder the snapshot names (a snapshot of relative paths).
        let common = start_dir(&snapshot.paths);
        if common != "/"
            && probe_snapshot_path(access, &params.snapshot_id, &common, cancel)?.1
                == EntryKind::Dir
        {
            plan.folder = common;
        }
    }
    Ok((snapshot, plan))
}

/// Runs a preview: suggestion and check always, the dry runs only where a preview is required
/// and the target can be written to.
pub(crate) fn execute_restore_preview(
    access: &SnapshotListAccess<'_>,
    ctx: &RestoreContext,
    params: &RestoreParams,
    cancel: Option<&CancelHandle>,
    limit_bytes: u64,
) -> Result<HeldPreview, BackupRunError> {
    let (snapshot, plan) = resolve_plan(access, params, cancel)?;
    let new_folder = ctx.suggestion(&snapshot);
    let suggestion = RestoreSuggestionDto {
        new_folder: new_folder.to_string_lossy().into_owned(),
        original: original_location(&plan.folder, &snapshot, current_home_dir().as_deref()),
    };
    let target = params
        .target
        .clone()
        .unwrap_or_else(|| RestoreTarget::NewFolder {
            dir: suggestion.new_folder.clone(),
        });
    if target.is_original() && suggestion.original.mapped_dir.as_deref() != Some(target.dir()) {
        return Err(BackupRunError::InvalidTarget {
            path: target.dir().to_string(),
            reason: "not the original location of the selection".to_string(),
        });
    }
    let mut check = check_target(&target, ctx.repositories());
    // restic takes the target as text: a folder whose real path is not UTF-8 would become
    // another folder.
    if check.resolved.to_str().is_none() {
        return Err(BackupRunError::InvalidTarget {
            path: check.resolved.to_string_lossy().into_owned(),
            reason: "the real path of the target folder is not valid UTF-8".to_string(),
        });
    }
    let mut dry_run = preview_required(&target, &check) && check.usable();
    let classification = if dry_run {
        let tree = walk_folder(access, &plan, &params.selection, cancel)?;
        if writes_into_repository(&tree, &plan, params, &check, ctx) {
            check.code = RestoreTargetCode::InsideRepository;
            dry_run = false;
            PreviewClassification::empty()
        } else {
            // restic leaves `uid` out of the snapshot when it is 0 (a snapshot made by root).
            let foreign_owner = snapshot.uid.unwrap_or(0) != current_uid();
            let run = PreviewRun {
                plan: &plan,
                tree: &tree,
                target: &check.resolved,
                overwrite: params.overwrite,
                foreign_owner,
                limit_bytes,
            };
            execute_preview(access, &run, cancel)?
        }
    } else {
        PreviewClassification::empty()
    };
    Ok(HeldPreview {
        fingerprint: fingerprint(ctx, params, &target, &check),
        plan,
        suggestion,
        target,
        check,
        classification,
        dry_run,
        backup: ctx.backup_of(&snapshot),
        used: false,
    })
}

/// Whether the restore would write into the repository of a local destination that lies below
/// the target folder: only entries of the selection count, so a snapshot without the
/// repository's folder or a selection outside it restores as usual.
fn writes_into_repository(
    tree: &SnapshotTree,
    plan: &RestorePlan,
    params: &RestoreParams,
    check: &TargetCheck,
    ctx: &RestoreContext,
) -> bool {
    let selected = selected_paths(plan, &params.selection);
    repositories_below(&check.resolved, ctx.repositories())
        .iter()
        .any(|repo| tree.writes_below(selected.as_deref(), repo))
}

/// The parameters a preview or a restore stands for.
pub(crate) fn fingerprint(
    ctx: &RestoreContext,
    params: &RestoreParams,
    target: &RestoreTarget,
    check: &TargetCheck,
) -> PreviewFingerprint {
    PreviewFingerprint {
        destination_id: ctx.destination.id.as_str().to_string(),
        snapshot_id: params.snapshot_id.clone(),
        selection: params.selection.clone(),
        original: target.is_original(),
        target: check.resolved.clone(),
        overwrite: params.overwrite,
    }
}
