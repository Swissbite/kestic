// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! The target of a restore: the suggested new folder, the original location, the check of a
//! target folder and the parameters a preview is valid for.

use crate::restore::RestoreTarget;
use crate::snapshot::ResticSnapshot;
use kestic_i18n::codes::{RestoreOverwrite, RestoreTargetCode};
use rustix::fs::{Access, access};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use time::{Duration, OffsetDateTime};

/// How long after its end a preview counts for `StartRestore`.
pub const PREVIEW_MAX_AGE: Duration = Duration::minutes(10);

/// The name of a new folder for a restore: `<label> <YYYY-MM-DD HH-MM>` with the snapshot time,
/// without `/`, control characters or a leading dot.
pub fn new_folder_name(label: &str, snapshot_time: OffsetDateTime) -> String {
    let stamp = format!(
        "{:04}-{:02}-{:02} {:02}-{:02}",
        snapshot_time.year(),
        u8::from(snapshot_time.month()),
        snapshot_time.day(),
        snapshot_time.hour(),
        snapshot_time.minute()
    );
    sanitize_folder_name(&format!("{label} {stamp}"))
}

/// Removes `/`, control characters and leading dots and spaces from a folder name.
pub fn sanitize_folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| *c != '/' && !c.is_control())
        .collect();
    cleaned
        .trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_end()
        .to_string()
}

/// `parent/name`, or with « (2)», « (3)», … when a folder of that name already exists.
pub fn unique_folder(parent: &Path, name: &str) -> PathBuf {
    let first = parent.join(name);
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    (2u32..)
        .map(|n| parent.join(format!("{name} ({n})")))
        .find(|p| std::fs::symlink_metadata(p).is_err())
        .unwrap_or(first)
}

/// The words of the suggested folder in the language of the daemon.
pub struct FolderWords<'a> {
    /// The folder below the home that holds restores («Restored»).
    pub restored: &'a str,
    /// Marks a snapshot without backup («not assigned»).
    pub not_assigned: &'a str,
}

/// What the suggestion of a new folder is made of.
pub struct SuggestionInput<'a> {
    pub home: &'a Path,
    pub words: FolderWords<'a>,
    /// The backup the snapshot belongs to, if any.
    pub backup_name: Option<&'a str>,
    pub destination_name: &'a str,
    pub snapshot_short_id: &'a str,
    /// The snapshot time in the local time zone.
    pub snapshot_time: OffsetDateTime,
}

/// The suggested new folder: `~/<restored>/<backup> <time>`, for a snapshot without backup
/// `~/<restored>/<destination> <not assigned> <short id>`.
pub fn suggested_new_folder(input: &SuggestionInput<'_>) -> PathBuf {
    let name = input.backup_name.map_or_else(
        || {
            sanitize_folder_name(&format!(
                "{} {} {}",
                input.destination_name, input.words.not_assigned, input.snapshot_short_id
            ))
        },
        |backup| new_folder_name(backup, input.snapshot_time),
    );
    let parent = input.home.join(sanitize_folder_name(input.words.restored));
    unique_folder(&parent, &name)
}

/// The original location of a restore of `folder` (the common folder of the selection, from the
/// snapshot tree).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginalLocation {
    pub original_dir: String,
    /// The folder restored into; `None` when the user has to choose one.
    pub mapped_dir: Option<String>,
    /// `alt → neu` when the folder was mapped to the user's home.
    pub mapping: Option<(String, String)>,
    pub needs_user_choice: bool,
}

/// The original location of `folder`: `/` is never a target, and a folder that lies outside
/// the paths the snapshot names (e.g. a snapshot of relative paths) needs a chosen folder.
pub fn original_location(folder: &str, snapshot: &ResticSnapshot) -> OriginalLocation {
    let within_paths = snapshot.paths.iter().any(|p| {
        let p = p.trim_end_matches('/');
        folder == p || folder.starts_with(&format!("{p}/")) || p.starts_with(&format!("{folder}/"))
    });
    let usable = folder != "/" && within_paths;
    OriginalLocation {
        original_dir: folder.to_string(),
        mapped_dir: usable.then(|| folder.to_string()),
        mapping: None,
        needs_user_choice: !usable,
    }
}

/// Result of checking a target folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetCheck {
    pub code: RestoreTargetCode,
    /// The folder with symbolic links resolved (for a folder still to create: its resolved
    /// parent joined with the rest).
    pub resolved: PathBuf,
    /// The folder does not exist yet.
    pub missing: bool,
}

impl TargetCheck {
    /// Whether a restore may write into the folder (with a preview where one is required).
    pub fn usable(&self) -> bool {
        matches!(
            self.code,
            RestoreTargetCode::Ok | RestoreTargetCode::ExistsNotEmpty
        )
    }
}

/// `path` with symbolic links resolved; for a path that does not exist yet, its deepest
/// existing folder resolved and the rest appended.
fn resolve_path(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(resolved) = std::fs::canonicalize(&existing) {
            let mut out = resolved;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Checks the target folder of `target`: a missing original location is never created, a file
/// is no folder, the folder must be writable, must not lie inside the repository of a local
/// destination (`repository`) and needs a preview when it has content.
pub fn check_target(target: &RestoreTarget, repository: Option<&Path>) -> TargetCheck {
    let dir = Path::new(target.dir());
    let resolved = resolve_path(dir);
    let inside_repo = repository
        .map(resolve_path)
        .is_some_and(|repo| resolved.starts_with(&repo));
    let check = |code| TargetCheck {
        code,
        resolved: resolved.clone(),
        missing: false,
    };
    let Ok(meta) = std::fs::metadata(dir) else {
        let code = if inside_repo {
            RestoreTargetCode::InsideRepository
        } else if target.is_original() {
            RestoreTargetCode::MissingMappedDir
        } else {
            creatable_below(dir)
        };
        return TargetCheck {
            code,
            resolved,
            missing: true,
        };
    };
    if !meta.is_dir() {
        return check(RestoreTargetCode::NotADirectory);
    }
    if inside_repo {
        return check(RestoreTargetCode::InsideRepository);
    }
    if access(&resolved, Access::WRITE_OK | Access::EXEC_OK).is_err() {
        return check(RestoreTargetCode::NotWritable);
    }
    let empty = std::fs::read_dir(&resolved).is_ok_and(|mut d| d.next().is_none());
    if empty {
        check(RestoreTargetCode::Ok)
    } else {
        check(RestoreTargetCode::ExistsNotEmpty)
    }
}

/// Whether the missing folder `dir` can be created: its deepest existing ancestor must be a
/// writable folder.
fn creatable_below(dir: &Path) -> RestoreTargetCode {
    let Some(existing) = dir
        .ancestors()
        .skip(1)
        .find(|a| std::fs::metadata(a).is_ok())
    else {
        return RestoreTargetCode::NotWritable;
    };
    if !existing.is_dir() {
        RestoreTargetCode::NotADirectory
    } else if access(existing, Access::WRITE_OK | Access::EXEC_OK).is_err() {
        RestoreTargetCode::NotWritable
    } else {
        RestoreTargetCode::Ok
    }
}

/// Whether a restore into `target` needs a finished, fresh preview: always at the original
/// location, otherwise when the folder has content.
pub fn preview_required(target: &RestoreTarget, check: &TargetCheck) -> bool {
    target.is_original() || check.code == RestoreTargetCode::ExistsNotEmpty
}

/// The user ID of the running process (the user a restore writes as).
pub fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Whether `path` is an absolute path without `..` segments.
pub fn is_clean_absolute(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|c| !matches!(c, Component::ParentDir))
}

/// The parameters a preview was made for; `StartRestore` only accepts a preview whose
/// parameters equal its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFingerprint {
    pub destination_id: String,
    pub snapshot_id: String,
    /// Normalised selection.
    pub selection: Vec<String>,
    pub original: bool,
    /// Resolved target folder.
    pub target: PathBuf,
    pub overwrite: RestoreOverwrite,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_drop_slashes_control_characters_and_leading_dots() {
        assert_eq!(sanitize_folder_name("..a/b\nc "), "abc");
        assert_eq!(sanitize_folder_name(" .x"), "x");
    }

    #[test]
    fn clean_absolute_paths() {
        assert!(is_clean_absolute("/a/b"));
        assert!(!is_clean_absolute("a/b"));
        assert!(!is_clean_absolute("/a/../b"));
        assert!(!is_clean_absolute(""));
        assert!(!is_clean_absolute("/a\0"));
    }
}
