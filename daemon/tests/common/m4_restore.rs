// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared fixture of the M4-S05 restore tests: a daemon behind the restic wrapper with one local
//! destination, one backup «Home» and one snapshot of a folder, written directly with restic.

use super::m3::{M3DaemonOpts, M3Rig, ResticVariant, add_local_destination};
use super::m4::{RawSnapshot, backup_dir};
use kestic_core::model::{Backup, Destination, RunStatus, Schedule};
use kestic_daemon::dbus_api::{
    KesticDbusError, RestoreOverwrite, RestorePreviewResultDto, RestoreRequest, RestoreTarget,
    RunDetailDto, SnapshotQueryPage, SnapshotQueryRequest, SnapshotQueryResultDto,
    SnapshotQueryStatusDto,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use time::macros::datetime;

pub struct RestoreFixture {
    pub rig: M3Rig,
    pub dest: Destination,
    pub password: String,
    pub repo: PathBuf,
    pub backup: Backup,
    /// The backed-up folder; also its path in the snapshot.
    pub src: PathBuf,
    pub snapshot_id: String,
}

impl RestoreFixture {
    /// Starts the daemon (scheduler off unless `configure` turns it on) and writes one snapshot
    /// of the folder `make_tree` creates below the sandbox.
    pub fn start(
        root: &Path,
        variant: &ResticVariant,
        base: &str,
        make_tree: impl FnOnce(&Path) -> PathBuf,
        configure: impl FnOnce(&mut M3DaemonOpts<'_>),
    ) -> Self {
        let rig = M3Rig::start(
            root,
            variant,
            base,
            datetime!(2026-11-15 14:32:00 UTC),
            |o| {
                o.enable_scheduler = false;
                configure(o);
            },
        );
        let work = root.join(&rig.sub);
        let repo = work.join("repo");
        let (dest, password) =
            add_local_destination(&rig.daemon.client, &variant.bin, &repo, "NAS", |_| {});
        let backup = rig.add_backup_to(&dest, "Home", |b| b.schedule = Schedule::ManualOnly);
        let src = make_tree(&work);
        let tag = format!("kestic:{}", backup.id);
        let snapshot_id = backup_dir(
            &variant.bin,
            &repo,
            &password,
            &src,
            &RawSnapshot {
                tag: &tag,
                time: "2026-11-15 12:00:00",
                host: "laptop",
            },
        );
        Self {
            rig,
            dest,
            password,
            repo,
            backup,
            src,
            snapshot_id,
        }
    }

    pub fn path(&self, rel: &str) -> String {
        if rel.is_empty() {
            self.src.to_string_lossy().into_owned()
        } else {
            format!("{}/{rel}", self.src.display())
        }
    }

    pub fn original(&self) -> RestoreTarget {
        RestoreTarget::Original { dir: self.path("") }
    }

    /// Starts a preview of `selection` (paths relative to the backed-up folder).
    pub fn start_preview(
        &self,
        selection: &[&str],
        target: Option<RestoreTarget>,
        overwrite: RestoreOverwrite,
    ) -> String {
        let selection: Vec<String> = selection.iter().map(|r| self.path(r)).collect();
        self.rig
            .daemon
            .client
            .start_snapshot_query(&SnapshotQueryRequest::restore_preview(
                self.dest.id.as_str(),
                &self.snapshot_id,
                &selection,
                target,
                overwrite,
            ))
            .expect("StartSnapshotQuery restore_preview")
    }

    pub fn status(&self, query_id: &str, page: &SnapshotQueryPage) -> SnapshotQueryStatusDto {
        self.rig
            .daemon
            .client
            .get_snapshot_query_status(query_id, page)
            .expect("GetSnapshotQueryStatus")
    }

    /// Waits until query `query_id` has ended and returns its status with `page`.
    pub fn wait_query(&self, query_id: &str, page: &SnapshotQueryPage) -> SnapshotQueryStatusDto {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let st = self.status(query_id, page);
            if st.state != "running" {
                return st;
            }
            assert!(Instant::now() < deadline, "preview still running");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The result of the finished preview `query_id` for `page`.
    pub fn preview_page(
        &self,
        query_id: &str,
        page: &SnapshotQueryPage,
    ) -> RestorePreviewResultDto {
        let st = self.wait_query(query_id, page);
        assert_eq!(st.state, "done", "preview ended with {st:?}");
        match st.result {
            Some(SnapshotQueryResultDto::RestorePreview(p)) => *p,
            other => panic!("restore preview result expected: {other:?}"),
        }
    }

    /// Runs a preview and returns its ID and its first page of up to 1000 entries.
    pub fn preview(
        &self,
        selection: &[&str],
        target: Option<RestoreTarget>,
        overwrite: RestoreOverwrite,
    ) -> (String, RestorePreviewResultDto) {
        let id = self.start_preview(selection, target, overwrite);
        let result = self.preview_page(&id, &big_page());
        (id, result)
    }

    pub fn start_restore(
        &self,
        selection: &[&str],
        target: RestoreTarget,
        overwrite: RestoreOverwrite,
        preview_id: Option<&str>,
    ) -> Result<String, KesticDbusError> {
        self.rig.daemon.client.start_restore(&RestoreRequest {
            destination_id: self.dest.id.as_str().to_string(),
            snapshot_id: self.snapshot_id.clone(),
            selection: selection.iter().map(|r| self.path(r)).collect(),
            target,
            overwrite,
            preview_id: preview_id.map(str::to_string),
        })
    }

    /// Waits until run `run_id` has ended and returns its detail.
    pub fn wait_run(&self, run_id: &str) -> RunDetailDto {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let detail = self
                .rig
                .daemon
                .client
                .get_run_detail(run_id)
                .expect("GetRunDetail");
            if !matches!(detail.run.status, RunStatus::Running | RunStatus::Pending) {
                return detail;
            }
            assert!(
                Instant::now() < deadline,
                "restore still running: {detail:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Arguments of every `restic restore` call so far, as the wrapper logged them.
    pub fn restore_calls(&self) -> Vec<String> {
        self.rig
            .wrapper
            .of('S', "restore")
            .into_iter()
            .map(|e| e.detail)
            .collect()
    }

    /// A fresh, not yet existing folder below the sandbox.
    pub fn new_dir(&self, name: &str) -> PathBuf {
        self.rig.root.join(&self.rig.sub).join("targets").join(name)
    }
}

/// A page of up to 1000 entries without filter.
pub fn big_page() -> SnapshotQueryPage {
    SnapshotQueryPage {
        limit: 1000,
        ..SnapshotQueryPage::default()
    }
}

/// Relative path → content of every file below `dir` (symlinks as `-> target`).
pub fn tree_contents(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    collect(dir, dir, &mut out);
    out
}

fn collect(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("dir entry").path();
        let rel = path
            .strip_prefix(root)
            .expect("below root")
            .to_string_lossy()
            .into_owned();
        let meta = fs::symlink_metadata(&path).expect("metadata");
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&path).expect("read link");
            out.insert(rel, format!("-> {}", target.display()));
        } else if meta.is_dir() {
            collect(root, &path, out);
        } else {
            out.insert(
                rel,
                String::from_utf8_lossy(&fs::read(&path).expect("read file")).into_owned(),
            );
        }
    }
}

/// Sets the modification time of `path` to `secs` seconds after the Unix epoch.
pub fn set_mtime(path: &Path, secs: u64) {
    let file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open for mtime");
    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(secs))
        .expect("set mtime");
}
