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
    let label = sanitize_folder_name(label);
    let label: String = label
        .char_indices()
        .take_while(|(i, c)| i + c.len_utf8() <= MAX_NAME_BYTES - 17)
        .map(|(_, c)| c)
        .collect();
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

/// Longest folder name Kestic suggests, in bytes: file systems allow 255, and « (n)» may be
/// appended.
const MAX_NAME_BYTES: usize = 240;

/// Removes `/`, control characters and leading dots and spaces from a folder name and cuts it
/// to [`MAX_NAME_BYTES`].
pub fn sanitize_folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| *c != '/' && !c.is_control())
        .collect();
    let mut out = cleaned
        .trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_end()
        .to_string();
    if out.len() > MAX_NAME_BYTES {
        let mut end = MAX_NAME_BYTES;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
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
    /// The folder restored into (mapped to the user's home where needed); `None` when there is
    /// none, `Some` with `needs_user_choice` when it does not exist here.
    pub mapped_dir: Option<String>,
    /// `alt → neu` when the folder was mapped to the user's home.
    pub mapping: Option<(String, String)>,
    pub needs_user_choice: bool,
}

/// Where a folder of a snapshot lands at the original location on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeMapping {
    /// The folder lies outside `/home/<name>`, or in the user's own home: it stays.
    Unchanged,
    /// `/home/<from>/<rest>` becomes `<home>/<rest>`; `to` is the name of the user's home.
    Mapped {
        dir: String,
        from: String,
        to: String,
    },
    /// The folder lies in `/home/<name>`, but it cannot be mapped safely: the snapshot was not
    /// made by `<name>`, the path has an empty, `.` or `..` segment, or the user's home is
    /// unknown. The user chooses a folder instead.
    Unmappable,
}

/// Maps `folder` from another user's home to the user's `home`: `/home/<name>/<rest>` becomes
/// `<home>/<rest>` when the snapshot was made by `<name>` (`snapshot_user`). A folder outside
/// `/home/<name>`, and one in the user's own home, stays where it is.
pub fn map_to_home(folder: &str, snapshot_user: Option<&str>, home: Option<&Path>) -> HomeMapping {
    let Some(rest) = folder.strip_prefix("/home/") else {
        return HomeMapping::Unchanged;
    };
    let (name, sub) = rest.split_once('/').unwrap_or((rest, ""));
    let clean = |segment: &str| !matches!(segment, "" | "." | "..");
    if folder.contains('\0') || !clean(name) || (!sub.is_empty() && !sub.split('/').all(clean)) {
        return HomeMapping::Unmappable;
    }
    let Some(home) = home.filter(|h| is_clean_absolute(&h.to_string_lossy())) else {
        return HomeMapping::Unmappable;
    };
    let old_home = PathBuf::from(format!("/home/{name}"));
    let same = old_home == home
        || std::fs::canonicalize(&old_home)
            .is_ok_and(|old| std::fs::canonicalize(home).is_ok_and(|own| old == own));
    if same {
        return HomeMapping::Unchanged;
    }
    let (Some(home_str), Some(to)) = (home.to_str(), home.file_name().and_then(|n| n.to_str()))
    else {
        return HomeMapping::Unmappable;
    };
    if snapshot_user != Some(name) {
        return HomeMapping::Unmappable;
    }
    let home_str = home_str.trim_end_matches('/');
    HomeMapping::Mapped {
        dir: if sub.is_empty() {
            home_str.to_string()
        } else {
            format!("{home_str}/{sub}")
        },
        from: name.to_string(),
        to: to.to_string(),
    }
}

/// The original location of `folder`: `/` is never a target, and a folder that lies outside
/// the paths the snapshot names (e.g. a snapshot of relative paths) needs a chosen folder. A
/// folder in another user's home is mapped to the user's `home` ([`map_to_home`]). A folder
/// that does not exist here is never created: the user chooses one instead.
pub fn original_location(
    folder: &str,
    snapshot: &ResticSnapshot,
    home: Option<&Path>,
) -> OriginalLocation {
    let within_paths = snapshot.paths.iter().any(|p| {
        let p = p.trim_end_matches('/');
        folder == p || folder.starts_with(&format!("{p}/")) || p.starts_with(&format!("{folder}/"))
    });
    let choose = OriginalLocation {
        original_dir: folder.to_string(),
        mapped_dir: None,
        mapping: None,
        needs_user_choice: true,
    };
    if folder == "/" || !within_paths {
        return choose;
    }
    let (dir, mapping) = match map_to_home(folder, snapshot.username.as_deref(), home) {
        HomeMapping::Unchanged => (folder.to_string(), None),
        HomeMapping::Mapped { dir, from, to } => (dir, Some((from, to))),
        HomeMapping::Unmappable => return choose,
    };
    let exists = std::fs::metadata(&dir).is_ok();
    OriginalLocation {
        original_dir: folder.to_string(),
        mapped_dir: Some(dir),
        mapping,
        needs_user_choice: !exists,
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
        code_usable(self.code)
    }

    /// Whether the real path of the folder is UTF-8; restic takes the target as text, so a
    /// folder whose path is not would become another folder.
    pub fn resolved_is_utf8(&self) -> bool {
        self.resolved.to_str().is_some()
    }
}

/// Whether a restore may write into a folder whose check gave `code`.
pub fn code_usable(code: RestoreTargetCode) -> bool {
    matches!(
        code,
        RestoreTargetCode::Ok | RestoreTargetCode::ExistsNotEmpty
    )
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
/// destination (`repositories`) and needs a preview when it has content.
pub fn check_target(target: &RestoreTarget, repositories: &[PathBuf]) -> TargetCheck {
    let dir = Path::new(target.dir());
    let resolved = resolve_path(dir);
    let inside_repo = repositories
        .iter()
        .map(|repo| resolve_path(repo))
        .any(|repo| resolved.starts_with(&repo));
    let check = |code| TargetCheck {
        code,
        resolved: resolved.clone(),
        missing: false,
    };
    let Ok(meta) = std::fs::metadata(dir) else {
        // A symbolic link to nothing cannot become the folder: restic fails to create it.
        let dangling_link =
            std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink());
        let code = if dangling_link {
            RestoreTargetCode::NotADirectory
        } else if inside_repo {
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

/// Whether the missing folder `dir` can be created: a restore creates it and at most the
/// folder holding it (`~/Restored`), so its parent or grandparent must be a writable folder.
fn creatable_below(dir: &Path) -> RestoreTargetCode {
    let Some((depth, existing)) = dir
        .ancestors()
        .enumerate()
        .skip(1)
        .find(|(_, a)| std::fs::metadata(a).is_ok())
    else {
        return RestoreTargetCode::NotWritable;
    };
    if depth > 2 {
        return RestoreTargetCode::MissingMappedDir;
    }
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
    preview_needed(target, check.code)
}

/// [`preview_required`] for a folder whose check gave `code`.
pub fn preview_needed(target: &RestoreTarget, code: RestoreTargetCode) -> bool {
    target.is_original() || code == RestoreTargetCode::ExistsNotEmpty
}

/// The user ID of the running process (the user a restore writes as).
pub fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// The repositories of `repositories` that lie below the resolved target folder `target`, as
/// paths relative to it starting with `/` (the form of the dry run and of [`SnapshotTree`]
/// keys). A repository that holds the target is caught by [`check_target`] already.
///
/// [`SnapshotTree`]: crate::restore_preview::SnapshotTree
pub fn repositories_below(target: &Path, repositories: &[PathBuf]) -> Vec<String> {
    repositories
        .iter()
        .map(|repo| resolve_path(repo))
        .filter_map(|repo| {
            // restic shows bytes that are not UTF-8 as U+FFFD, like `to_string_lossy`.
            let rest = repo.strip_prefix(target).ok()?.to_string_lossy();
            (!rest.is_empty()).then(|| format!("/{rest}"))
        })
        .collect()
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
    fn long_folder_names_keep_the_time_and_fit_the_file_system() {
        let name = new_folder_name(
            &"ö".repeat(200),
            time::macros::datetime!(2026-11-15 12:00 UTC),
        );
        assert!(name.len() <= MAX_NAME_BYTES, "{} bytes", name.len());
        assert!(name.ends_with(" 2026-11-15 12-00"), "{name}");
    }

    #[test]
    fn repositories_below_the_target_are_named_relative_to_it() {
        let dir = std::env::temp_dir().join(format!("kestic-repos-below-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("home/vault")).expect("mkdir");
        let target = std::fs::canonicalize(dir.join("home")).expect("canonical");
        let below = repositories_below(
            &target,
            &[
                dir.join("home/vault"),
                dir.join("home"),
                dir.join("elsewhere"),
            ],
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");
        assert_eq!(below, vec!["/vault".to_string()]);
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
