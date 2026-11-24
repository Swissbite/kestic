// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared fixture of the M4-S05 restore tests: a daemon behind the restic wrapper with one local
//! destination, one backup «Home» and one snapshot of a folder, written directly with restic.

use super::m3::{M3DaemonOpts, M3Rig, ResticVariant, add_local_destination};
use super::m4::{
    RawSnapshot, VariantFixture, backup_dir, big_page, for_each_variant_with, small_tree,
    wait_query,
};
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
        Self::start_tagged(root, variant, base, make_tree, configure, |backup| {
            format!("kestic:{}", backup.id)
        })
    }

    /// Like [`Self::start`], with the snapshot's tag from `tag_of` (empty: no tag).
    pub fn start_tagged(
        root: &Path,
        variant: &ResticVariant,
        base: &str,
        make_tree: impl FnOnce(&Path) -> PathBuf,
        configure: impl FnOnce(&mut M3DaemonOpts<'_>),
        tag_of: impl FnOnce(&Backup) -> String,
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
        let tag = tag_of(&backup);
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
        wait_query(&self.rig.daemon.client, query_id, page)
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

    /// AC-M4-05-7: a preview of the whole snapshot at the original location with `always`
    /// whose confirmation is cancelled (`CancelSnapshotQuery`, no `StartRestore`) leaves the
    /// target as it was: (a) no entry is added or removed anywhere below it, and every entry,
    /// the target folder included, keeps its type, content, permissions and mtime. `docs` and
    /// `b.bin` first get other permissions than in the snapshot, so a restore would reset them.
    pub fn cancelled_confirmation_leaves_the_target_as_it_was(&self, label: &str) {
        use std::os::unix::fs::PermissionsExt;
        for (rel, mode) in [("docs", 0o700), ("b.bin", 0o600)] {
            fs::set_permissions(self.src.join(rel), fs::Permissions::from_mode(mode))
                .expect("chmod");
        }
        let before = tree_state(&self.src);
        let calls = self.restore_calls().len();
        let (id, _) = self.preview(&[], Some(self.original()), RestoreOverwrite::Always);
        assert!(
            self.restore_calls().len() > calls,
            "AC-M4-05-7 {label}: the preview ran its dry run: {:?}",
            self.restore_calls()
        );
        self.rig
            .daemon
            .client
            .cancel_snapshot_query(&id)
            .expect("CancelSnapshotQuery");
        super::m3::settle();
        let after = tree_state(&self.src);
        assert_eq!(
            after.keys().collect::<Vec<_>>(),
            before.keys().collect::<Vec<_>>(),
            "(a) AC-M4-05-7 {label}: no file or folder added or removed below the target"
        );
        for (rel, was) in &before {
            assert_eq!(
                after.get(rel),
                Some(was),
                "(a) AC-M4-05-7 {label}: `{rel}` keeps type, content, permissions and mtime"
            );
        }
    }
}

impl VariantFixture for RestoreFixture {
    fn stop(self) {
        self.rig.daemon.stop();
    }
}

/// Runs `body` once per restic version with a [`RestoreFixture`] (scheduler off) whose snapshot
/// of the folder `make_tree` creates carries the tag of the backup «Home».
pub fn for_each_variant(
    root: &Path,
    base: &str,
    make_tree: fn(&Path) -> PathBuf,
    body: impl Fn(&RestoreFixture, &ResticVariant),
) {
    for_each_variant_with(
        root,
        |root, variant| RestoreFixture::start(root, variant, base, make_tree, |_| {}),
        body,
    );
}

/// Like [`for_each_variant`] with the folder of [`small_tree`], but its snapshot carries `tag`
/// (empty: none) instead of the tag of the backup «Home».
pub fn for_each_tagged_variant(
    root: &Path,
    base: &str,
    tag: &str,
    body: impl Fn(&RestoreFixture, &ResticVariant),
) {
    for_each_variant_with(
        root,
        |root, variant| {
            RestoreFixture::start_tagged(
                root,
                variant,
                base,
                small_tree,
                |_| {},
                |_| tag.to_string(),
            )
        },
        body,
    );
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

/// What [`tree_state`] records of one entry: its type, permission bits, modification time, and
/// the length and hash of its content (the file's bytes, a link's target, nothing for a folder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryState {
    pub kind: &'static str,
    pub mode: u32,
    pub mtime: std::time::SystemTime,
    pub content_len: usize,
    pub content_hash: u64,
}

/// Relative path → [`EntryState`] of `dir` itself (as `""`) and of every entry below it, folders
/// included, without following symbolic links.
pub fn tree_state(dir: &Path) -> BTreeMap<String, EntryState> {
    let mut out = BTreeMap::new();
    record_state(dir, dir, &mut out);
    out
}

fn record_state(root: &Path, path: &Path, out: &mut BTreeMap<String, EntryState>) {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let meta = fs::symlink_metadata(path).expect("metadata");
    let t = meta.file_type();
    let (kind, content) = if t.is_symlink() {
        let target = fs::read_link(path).expect("read link");
        ("symlink", target.into_os_string().into_encoded_bytes())
    } else if t.is_dir() {
        ("dir", Vec::new())
    } else if t.is_file() {
        ("file", fs::read(path).expect("read file"))
    } else if t.is_fifo() {
        ("fifo", Vec::new())
    } else if t.is_socket() {
        ("socket", Vec::new())
    } else {
        ("other", Vec::new())
    };
    let rel = path
        .strip_prefix(root)
        .expect("below root")
        .to_string_lossy()
        .into_owned();
    let mut hasher = std::hash::DefaultHasher::new();
    std::hash::Hash::hash(&content, &mut hasher);
    out.insert(
        rel,
        EntryState {
            kind,
            mode: meta.permissions().mode(),
            mtime: meta.modified().expect("mtime"),
            content_len: content.len(),
            content_hash: std::hash::Hasher::finish(&hasher),
        },
    );
    if kind == "dir" {
        for entry in fs::read_dir(path).expect("read dir") {
            record_state(root, &entry.expect("dir entry").path(), out);
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
