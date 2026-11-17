// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Detection of replaced executable binaries after package updates (`dpkg` rename, E11).

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Filesystem identity of the running binary captured at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryIdentity {
    exe_path: PathBuf,
    dev: u64,
    ino: u64,
}

/// Reason why a binary replacement was detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceReason {
    /// `/proc/self/exe` symlink target ends with `" (deleted)"`.
    ProcSelfExeDeleted,
    /// Path on disk now resolves to a different `(st_dev, st_ino)`.
    InodeChanged,
    /// Original executable path no longer exists on disk.
    PathMissing,
}

impl ReplaceReason {
    /// Stable identifier for logs and test assertions.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProcSelfExeDeleted => "proc_self_exe_deleted",
            Self::InodeChanged => "inode_changed",
            Self::PathMissing => "path_missing",
        }
    }
}

/// Current status of the running process executable relative to startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryStatus {
    Unchanged,
    Replaced(ReplaceReason),
}

impl BinaryIdentity {
    /// Captures the current executable path (via `/proc/self/exe`) and its device/inode.
    pub fn capture_current() -> io::Result<Self> {
        let exe_path = std::env::current_exe()?;
        Self::from_path(exe_path)
    }

    /// Captures identity for a given executable path.
    pub fn from_path(exe_path: PathBuf) -> io::Result<Self> {
        let meta = fs::metadata(&exe_path)?;
        Ok(Self {
            exe_path,
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }

    /// Path of the executable recorded at startup.
    pub fn exe_path(&self) -> &Path {
        &self.exe_path
    }

    /// Checks whether the executable on disk was replaced (e.g. via `dpkg` `rename`).
    pub fn check(&self) -> BinaryStatus {
        if let Ok(target) = fs::read_link("/proc/self/exe")
            && target.to_string_lossy().ends_with(" (deleted)")
        {
            return BinaryStatus::Replaced(ReplaceReason::ProcSelfExeDeleted);
        }

        match fs::metadata(&self.exe_path) {
            Ok(meta) => {
                if meta.dev() != self.dev || meta.ino() != self.ino {
                    BinaryStatus::Replaced(ReplaceReason::InodeChanged)
                } else {
                    BinaryStatus::Unchanged
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                BinaryStatus::Replaced(ReplaceReason::PathMissing)
            }
            Err(_) => BinaryStatus::Unchanged,
        }
    }
}
