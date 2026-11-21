// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Mountpoint parsing (`/proc/self/mountinfo`), filesystem UUID lookup, and source drive
//! detection.

use crate::model::{Backup, Source, SourceIdentity, SourceKind};
use crate::sync::LockUnpoisoned;
use kestic_i18n::codes::SourceStatus;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Single parsed entry from `/proc/self/mountinfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub mount_id: u64,
    pub parent_id: u64,
    pub major_minor: String,
    pub root: String,
    pub mount_point: PathBuf,
    pub fs_type: String,
    pub mount_source: String,
}

/// Abstracts OS mountinfo + block-device UUID inspection for deterministic tests.
pub trait MountInspector: Send + Sync {
    /// Returns raw `/proc/self/mountinfo` contents.
    fn read_mountinfo(&self) -> std::io::Result<String>;

    /// Resolves `(fs_uuid, optional_label)` for a mounted block device (udev / `/dev/disk/by-uuid`).
    /// Returns `None` for non-block/virtual filesystems without UUID (e.g. `nfs4`, `sshfs`, `tmpfs`).
    fn fs_uuid_for_device(
        &self,
        mount_source: &str,
        major_minor: &str,
    ) -> Option<(String, Option<String>)>;

    /// Returns whether directory `path` is empty (returns `false` if non-empty or not a readable dir).
    fn is_empty_dir(&self, path: &Path) -> bool {
        dir_is_empty(path)
    }
}

/// Whether `path` is a readable directory without entries.
fn dir_is_empty(path: &Path) -> bool {
    path.is_dir() && fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
}

/// Real Linux inspector using `/proc/self/mountinfo`, udev metadata, and `/dev/disk/by-uuid`
/// (without relying on mountinfo for FS-UUIDs or external subprocesses).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemMountInspector;

impl MountInspector for SystemMountInspector {
    fn read_mountinfo(&self) -> std::io::Result<String> {
        fs::read_to_string("/proc/self/mountinfo")
    }

    fn fs_uuid_for_device(
        &self,
        mount_source: &str,
        major_minor: &str,
    ) -> Option<(String, Option<String>)> {
        // Non-block pseudo-devices always use major 0 in Linux (`0:xx`).
        if major_minor.starts_with("0:") {
            return None;
        }

        // 1. Fast, unprivileged udev database file (`/run/udev/data/b<major>:<minor>`).
        if let Some(res) = read_udev_fs_identity(major_minor) {
            return Some(res);
        }

        // 2. Canonicalized `/dev/disk/by-uuid` and `/dev/disk/by-label` symlinks.
        if let Some(res) = read_dev_disk_by_uuid(mount_source) {
            return Some(res);
        }

        None
    }
}

fn read_udev_fs_identity(major_minor: &str) -> Option<(String, Option<String>)> {
    let udev_path = PathBuf::from(format!("/run/udev/data/b{major_minor}"));
    let content = fs::read_to_string(udev_path).ok()?;
    let mut uuid = None;
    let mut label = None;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("E:ID_FS_UUID=") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                uuid = Some(trimmed.to_string());
            }
        } else if let Some(val) = line.strip_prefix("E:ID_FS_LABEL=") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                label = Some(trimmed.to_string());
            }
        }
    }
    uuid.map(|u| (u, label))
}

fn read_dev_disk_by_uuid(mount_source: &str) -> Option<(String, Option<String>)> {
    if !mount_source.starts_with("/dev/") {
        return None;
    }
    let target = fs::canonicalize(mount_source).ok()?;
    let by_uuid = Path::new("/dev/disk/by-uuid");
    let entries = fs::read_dir(by_uuid).ok()?;
    let mut found_uuid = None;
    for entry in entries.flatten() {
        if let Ok(resolved) = fs::canonicalize(entry.path())
            && resolved == target
        {
            found_uuid = Some(entry.file_name().to_string_lossy().into_owned());
            break;
        }
    }
    let uuid = found_uuid?;

    let mut found_label = None;
    if let Ok(label_entries) = fs::read_dir("/dev/disk/by-label") {
        for entry in label_entries.flatten() {
            if let Ok(resolved) = fs::canonicalize(entry.path())
                && resolved == target
            {
                found_label = Some(entry.file_name().to_string_lossy().into_owned());
                break;
            }
        }
    }
    Some((uuid, found_label))
}

/// Configurable thread-safe fake [`MountInspector`] for unit and D-Bus integration tests.
#[derive(Debug, Clone, Default)]
pub struct FakeMountState {
    pub mountinfo: String,
    /// Map of `mount_source` -> `(fs_uuid, optional_label)`.
    pub device_uuids: HashMap<String, (String, Option<String>)>,
    /// Optional overrides of empty-dir checks; when absent, falls back to filesystem `read_dir`.
    pub empty_dirs: HashMap<PathBuf, bool>,
}

/// Fake [`MountInspector`] backed by shared state (`Arc<Mutex<FakeMountState>>`).
#[derive(Debug, Clone, Default)]
pub struct FakeMountInspector {
    state: Arc<Mutex<FakeMountState>>,
}

impl FakeMountInspector {
    pub fn new(state: FakeMountState) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut FakeMountState),
    {
        let mut guard = self.state.lock_unpoisoned();
        f(&mut guard);
    }
}

impl MountInspector for FakeMountInspector {
    fn read_mountinfo(&self) -> std::io::Result<String> {
        let guard = self.state.lock_unpoisoned();
        Ok(guard.mountinfo.clone())
    }

    fn fs_uuid_for_device(
        &self,
        mount_source: &str,
        _major_minor: &str,
    ) -> Option<(String, Option<String>)> {
        let guard = self.state.lock_unpoisoned();
        guard.device_uuids.get(mount_source).cloned()
    }

    fn is_empty_dir(&self, path: &Path) -> bool {
        let guard = self.state.lock_unpoisoned();
        if let Some(&forced) = guard.empty_dirs.get(path) {
            return forced;
        }
        drop(guard);
        dir_is_empty(path)
    }
}

/// Unescapes Linux procfs octal escapes (`\040`, `\011`, `\012`, `\134`).
pub fn unescape_mountinfo_field(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let oct = &raw[i + 1..i + 4];
            if let Ok(val) = u8::from_str_radix(oct, 8) {
                out.push(val as char);
                i += 4;
                continue;
            }
        }
        let ch = raw[i..].chars().next().expect("valid utf8 boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Parses `/proc/self/mountinfo` contents into structured [`MountEntry`] rows.
pub fn parse_mountinfo(content: &str) -> Vec<MountEntry> {
    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let left_fields: Vec<&str> = left.split_whitespace().collect();
        let right_fields: Vec<&str> = right.split_whitespace().collect();
        if left_fields.len() < 6 || right_fields.len() < 2 {
            continue;
        }
        let Ok(mount_id) = left_fields[0].parse::<u64>() else {
            continue;
        };
        let Ok(parent_id) = left_fields[1].parse::<u64>() else {
            continue;
        };
        let major_minor = left_fields[2].to_string();
        let root = unescape_mountinfo_field(left_fields[3]);
        let mount_point = PathBuf::from(unescape_mountinfo_field(left_fields[4]));
        let fs_type = right_fields[0].to_string();
        let mount_source = unescape_mountinfo_field(right_fields[1]);

        entries.push(MountEntry {
            mount_id,
            parent_id,
            major_minor,
            root,
            mount_point,
            fs_type,
            mount_source,
        });
    }
    entries
}

/// Normalizes a path lexically by resolving `.` and `..` without touching disk symlinks.
pub fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(seg) => out.push(seg),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

fn path_component_count(path: &Path) -> usize {
    path.components().count()
}

/// Resolves the deepest matching mount entry for `path` (component-wise longest prefix).
/// When stacked mounts exist on the same mountpoint, the later line in mountinfo wins.
pub fn resolve_mount_for_path<'a>(
    path: &Path,
    entries: &'a [MountEntry],
) -> Option<&'a MountEntry> {
    let norm = normalize_lexical_path(path);
    let mut best: Option<(usize, &'a MountEntry)> = None;
    for entry in entries {
        let mp = normalize_lexical_path(&entry.mount_point);
        if norm == mp || norm.starts_with(&mp) {
            let depth = path_component_count(&mp);
            match best {
                None => best = Some((depth, entry)),
                Some((best_depth, _)) if depth >= best_depth => best = Some((depth, entry)),
                _ => {}
            }
        }
    }
    best.map(|(_, entry)| entry)
}

fn same_mount(a: &MountEntry, b: &MountEntry) -> bool {
    a.mount_point == b.mount_point
        && a.major_minor == b.major_minor
        && a.mount_source == b.mount_source
        && a.fs_type == b.fs_type
}

/// Detects [`SourceKind`] (`Folder` vs `Drive`) and drive identity for `path` from mountinfo.
pub fn detect_source_with(
    path: &Path,
    home_dir: Option<&Path>,
    inspector: &dyn MountInspector,
) -> Source {
    let entries = match inspector.read_mountinfo() {
        Ok(raw) => parse_mountinfo(&raw),
        Err(_) => return Source::folder(path),
    };
    if entries.is_empty() {
        return Source::folder(path);
    }

    let Some(src_mount) = resolve_mount_for_path(path, &entries) else {
        return Source::folder(path);
    };
    let root_mount = resolve_mount_for_path(Path::new("/"), &entries);
    let home_mount = home_dir.and_then(|h| resolve_mount_for_path(h, &entries));

    let matches_root = root_mount.is_some_and(|r| same_mount(src_mount, r));
    let matches_home = home_mount.is_some_and(|h| same_mount(src_mount, h));

    if matches_root || matches_home {
        return Source::folder(path);
    }

    if let Some((fs_uuid, label)) =
        inspector.fs_uuid_for_device(&src_mount.mount_source, &src_mount.major_minor)
    {
        return Source::drive_with_identity(path, Some(SourceIdentity::FsUuid { fs_uuid }), label);
    }

    Source::drive_with_identity(
        path,
        Some(SourceIdentity::MountSource {
            mount_source: src_mount.mount_source.clone(),
            fs_type: src_mount.fs_type.clone(),
        }),
        None,
    )
}

/// Result of checking whether a backup source is ready for execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceCheckResult {
    Available,
    /// Path does not exist (generic `Folder` failure).
    PathMissing(String),
    /// Drive is not currently mounted (`Laufwerk nicht eingehängt`).
    DriveNotMounted,
    /// Different drive is mounted at the expected mountpoint (`anderes Laufwerk`).
    DriveMismatch,
    /// Drive mountpoint exists and matches, but directory is empty (`Quelle leer`).
    DriveEmpty,
}

impl SourceCheckResult {
    /// Code of an unavailable source (`source_status`); `None` while the source is available.
    pub fn status_code(&self) -> Option<SourceStatus> {
        match self {
            Self::Available => None,
            Self::PathMissing(_) => Some(SourceStatus::PathMissing),
            Self::DriveNotMounted => Some(SourceStatus::DriveNotMounted),
            Self::DriveMismatch => Some(SourceStatus::DriveMismatch),
            Self::DriveEmpty => Some(SourceStatus::DriveEmpty),
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Evaluates source availability using `inspector` and `home_dir`.
pub fn check_source_status_with(
    backup: &Backup,
    home_dir: Option<&Path>,
    inspector: &dyn MountInspector,
) -> SourceCheckResult {
    match backup.source.kind {
        SourceKind::Folder => {
            if !backup.source.path.exists() {
                SourceCheckResult::PathMissing(backup.source.path.display().to_string())
            } else {
                // Empty folder is explicitly allowed for `SourceKind::Folder`.
                SourceCheckResult::Available
            }
        }
        SourceKind::Drive => {
            let entries = inspector
                .read_mountinfo()
                .map(|raw| parse_mountinfo(&raw))
                .unwrap_or_default();

            if entries.is_empty() {
                if !backup.source.path.exists() {
                    return SourceCheckResult::DriveNotMounted;
                }
                if inspector.is_empty_dir(&backup.source.path) {
                    return SourceCheckResult::DriveEmpty;
                }
                return SourceCheckResult::Available;
            }

            let Some(src_mount) = resolve_mount_for_path(&backup.source.path, &entries) else {
                return SourceCheckResult::DriveNotMounted;
            };
            let root_mount = resolve_mount_for_path(Path::new("/"), &entries);
            let home_mount = home_dir.and_then(|h| resolve_mount_for_path(h, &entries));

            let on_root_or_home = root_mount.is_some_and(|r| same_mount(src_mount, r))
                || home_mount.is_some_and(|h| same_mount(src_mount, h));

            // If mount resolves only to `/` or `$HOME`, the external drive isn't mounted.
            if on_root_or_home {
                return SourceCheckResult::DriveNotMounted;
            }

            // Verify drive identity if one was recorded.
            if let Some(expected) = &backup.source.identity {
                let current_identity = match inspector
                    .fs_uuid_for_device(&src_mount.mount_source, &src_mount.major_minor)
                {
                    Some((fs_uuid, _)) => SourceIdentity::FsUuid { fs_uuid },
                    None => SourceIdentity::MountSource {
                        mount_source: src_mount.mount_source.clone(),
                        fs_type: src_mount.fs_type.clone(),
                    },
                };
                if &current_identity != expected {
                    return SourceCheckResult::DriveMismatch;
                }
            }

            if !backup.source.path.exists() {
                // Mounted drive doesn't even contain configured subdir.
                return SourceCheckResult::DriveNotMounted;
            }

            // Reject empty mountpoint / directory for drives.
            if inspector.is_empty_dir(&backup.source.path) {
                return SourceCheckResult::DriveEmpty;
            }

            SourceCheckResult::Available
        }
    }
}

/// Resolves current `HOME` env var for source checks.
pub fn current_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}
