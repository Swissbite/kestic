// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Installs the app-managed restic in Kestic's data folder: download and check the release
//! ([`crate::restic_release`]), unpack it into a temporary file next to the binary, check the
//! version it reports, then `rename` it over the old binary. Until that `rename` the old binary
//! is never touched, and every error removes the temporary file.

use crate::bounded_command::output_within;
use crate::provider::{MIN_RESTIC_VERSION, parse_version_json};
use crate::restic_gate::ResticBinaryGate;
use crate::restic_release::{
    ARCHIVE_LIMIT, Arch, DownloadPlan, HttpAnswer, InstallError, RELEASE_INFO_LIMIT,
    SIGNATURE_LIMIT, Transport, TrustAnchor, UreqTransport, check_sha256, expected_sha256,
    fetch_asset, fetch_release, find_gpgv, verify_signature,
};
use kestic_i18n::codes::ResticInstallPhase;
use semver::Version;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use url::Url;

/// File name of the app-managed restic in Kestic's data folder.
pub const BINARY_NAME: &str = "restic";
/// Prefix of the temporary file a download is written to, in the same folder.
pub const TEMP_PREFIX: &str = ".restic-download-";
/// restic's Linux binaries are about 30 MiB.
pub const UNPACKED_LIMIT: u64 = 256 << 20;
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// What an installation needs from its caller.
#[derive(Debug, Clone, Copy)]
pub struct InstallRequest<'a> {
    /// Kestic's data folder, `$XDG_DATA_HOME/kestic`.
    pub data_dir: &'a Path,
    pub arch: Arch,
    /// The `PATH` in which `gpgv` is looked up.
    pub search_path: Option<&'a OsStr>,
}

/// Kestic's data folder, `$XDG_DATA_HOME/kestic`, or `$HOME/.local/share/kestic` when
/// `XDG_DATA_HOME` is unset, empty or relative (as the XDG base directory rules say).
pub fn kestic_data_dir(home: Option<&OsStr>, xdg_data_home: Option<&OsStr>) -> Option<PathBuf> {
    let base = match xdg_data_home.map(Path::new).filter(|p| p.is_absolute()) {
        Some(data) => data.to_path_buf(),
        None => {
            let home = Path::new(home?);
            if !home.is_absolute() {
                return None;
            }
            home.join(".local").join("share")
        }
    };
    Some(base.join("kestic"))
}

/// Result of preparing an installation ([`prepare_with`]).
#[derive(Debug)]
pub enum Preparation {
    /// The app-managed binary already is the release's version; nothing was downloaded.
    AlreadyCurrent(Version),
    /// The new binary waits in its temporary file.
    Ready(PreparedInstall),
}

impl Preparation {
    /// The prepared binary, unless the installed one is already current.
    pub fn into_ready(self) -> Option<PreparedInstall> {
        match self {
            Self::Ready(prepared) => Some(prepared),
            Self::AlreadyCurrent(_) => None,
        }
    }
}

/// Result of an installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed(Version),
    AlreadyCurrent(Version),
}

/// Path of the app-managed restic in `data_dir`.
pub fn app_managed_binary(data_dir: &Path) -> PathBuf {
    data_dir.join(BINARY_NAME)
}

/// Why the app-managed binary was not removed.
#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    /// A restic process may run it, or an installation replaces it.
    #[error("the restic managed by Kestic is in use")]
    InUse,
    #[error("cannot remove the restic managed by Kestic: {0}")]
    Io(#[from] std::io::Error),
}

/// Deletes the app-managed restic in `data_dir` at once, unless a restic process may run it or
/// an installation replaces it ([`ResticBinaryGate::remove_if_unused`]); restic processes that
/// start meanwhile wait. Returns whether there was a binary to delete.
pub fn remove_app_managed(data_dir: &Path, gate: &ResticBinaryGate) -> Result<bool, RemoveError> {
    let binary = app_managed_binary(data_dir);
    let _removal = gate.remove_if_unused(&binary).ok_or(RemoveError::InUse)?;
    match fs::remove_file(&binary) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}

/// A downloaded and checked restic in its temporary file, ready to replace the app-managed
/// binary. Dropping it without [`PreparedInstall::commit`] deletes the temporary file.
#[derive(Debug)]
pub struct PreparedInstall {
    temp: PathBuf,
    target: PathBuf,
    dir: PathBuf,
    version: Version,
    committed: bool,
}

impl PreparedInstall {
    /// Version the new binary reported.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// The temporary file that holds the new binary.
    pub fn temp_path(&self) -> &Path {
        &self.temp
    }

    /// Makes the new binary executable for everyone, writes it to disk and moves it over the
    /// app-managed binary with `rename`.
    pub fn commit(mut self) -> Result<Version, InstallError> {
        fs::set_permissions(&self.temp, fs::Permissions::from_mode(0o755)).map_err(write_failed)?;
        File::open(&self.temp)
            .and_then(|file| file.sync_all())
            .map_err(write_failed)?;
        fs::rename(&self.temp, &self.target).map_err(write_failed)?;
        self.committed = true;
        // The binary is in place; a failed sync of the folder only delays when the rename
        // survives a power loss.
        if let Ok(dir) = File::open(&self.dir) {
            let _ = dir.sync_all();
        }
        Ok(self.version.clone())
    }
}

impl Drop for PreparedInstall {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.temp);
        }
    }
}

fn finish(preparation: Preparation) -> Result<InstallOutcome, InstallError> {
    replace_when_idle(preparation, ResticBinaryGate::global(), &Unobserved)
}

/// Downloads the latest restic release with an anchor of the test build, checks it and writes
/// it to a temporary file in `request.data_dir`; the app-managed binary stays as it is.
#[cfg(feature = "test-trust-anchor")]
pub fn prepare_install_with_anchor(
    request: &InstallRequest<'_>,
    anchor: &TrustAnchor,
) -> Result<Preparation, InstallError> {
    prepare_with(
        request,
        anchor,
        &crate::restic_release::UreqTransport::new(),
    )
}

/// Downloads, checks and installs the latest restic release with an anchor of the test build.
#[cfg(feature = "test-trust-anchor")]
pub fn install_latest_with_anchor(
    request: &InstallRequest<'_>,
    anchor: &TrustAnchor,
) -> Result<InstallOutcome, InstallError> {
    finish(prepare_install_with_anchor(request, anchor)?)
}

/// What a running installation reports, and whether it should stop. It is called while restic
/// processes may wait for the installation, so it must never wait for anything that runs restic.
pub trait InstallObserver: Sync {
    fn phase(&self, phase: ResticInstallPhase);
    /// The release version, once it is known.
    fn version(&self, version: &str);
    /// Bytes of the file being downloaded so far, and its size if the server says.
    fn bytes(&self, done: u64, total: Option<u64>);
    /// True once the installation should stop; it then ends before the `rename`.
    fn cancelled(&self) -> bool;
    /// Called under the replace lease right before the `rename`: true moves to the phase
    /// `replacing`, after which a cancel comes too late; false (cancelled meanwhile) stops the
    /// installation. Deciding both in one step keeps a cancel and the `rename` apart.
    fn begin_replace(&self) -> bool;
}

struct Unobserved;

impl InstallObserver for Unobserved {
    fn phase(&self, _phase: ResticInstallPhase) {}
    fn version(&self, _version: &str) {}
    fn bytes(&self, _done: u64, _total: Option<u64>) {}
    fn cancelled(&self) -> bool {
        false
    }
    fn begin_replace(&self) -> bool {
        true
    }
}

/// [`UreqTransport`] that reports each download to an [`InstallObserver`] and stops it once
/// the observer is cancelled.
struct ObservedTransport<'a> {
    inner: UreqTransport,
    observer: &'a dyn InstallObserver,
}

impl Transport for ObservedTransport<'_> {
    fn get(&self, url: &Url, limit: u64) -> Result<HttpAnswer, InstallError> {
        self.inner.get_observed(url, limit, &|done, total| {
            self.observer.bytes(done, total);
            !self.observer.cancelled()
        })
    }
}

/// Downloads, checks and installs the latest restic release with this build's trust anchor:
/// download and checks run at once, the `rename` only under `gate`'s replace lease, so never
/// while restic runs. Reports its progress to `observer` and stops before the `rename` once
/// the observer is cancelled ([`InstallError::Cancelled`]).
pub fn install_latest_observed(
    request: &InstallRequest<'_>,
    gate: &ResticBinaryGate,
    observer: &dyn InstallObserver,
) -> Result<InstallOutcome, InstallError> {
    let anchor = TrustAnchor::for_this_build()?;
    let transport = ObservedTransport {
        inner: UreqTransport::new(),
        observer,
    };
    let preparation = prepare_observed(request, &anchor, &transport, observer)?;
    replace_when_idle(preparation, gate, observer)
}

/// Moves a prepared binary into place under `gate`'s replace lease.
pub(crate) fn replace_when_idle(
    preparation: Preparation,
    gate: &ResticBinaryGate,
    observer: &dyn InstallObserver,
) -> Result<InstallOutcome, InstallError> {
    let prepared = match preparation {
        Preparation::AlreadyCurrent(version) => {
            return Ok(InstallOutcome::AlreadyCurrent(version));
        }
        Preparation::Ready(prepared) => prepared,
    };
    observer.phase(ResticInstallPhase::WaitingForRuns);
    let lease = gate
        .replace_when_idle(&|| observer.cancelled())
        .ok_or(InstallError::Cancelled)?;
    if !observer.begin_replace() {
        return Err(InstallError::Cancelled);
    }
    let version = prepared.commit()?;
    drop(lease);
    Ok(InstallOutcome::Installed(version))
}

pub(crate) fn prepare_with(
    request: &InstallRequest<'_>,
    anchor: &TrustAnchor,
    transport: &dyn Transport,
) -> Result<Preparation, InstallError> {
    prepare_observed(request, anchor, transport, &Unobserved)
}

fn stop_if_cancelled(observer: &dyn InstallObserver) -> Result<(), InstallError> {
    if observer.cancelled() {
        Err(InstallError::Cancelled)
    } else {
        Ok(())
    }
}

fn prepare_observed(
    request: &InstallRequest<'_>,
    anchor: &TrustAnchor,
    transport: &dyn Transport,
    observer: &dyn InstallObserver,
) -> Result<Preparation, InstallError> {
    let gpgv = find_gpgv(request.search_path).ok_or(InstallError::GpgvMissing)?;
    observer.phase(ResticInstallPhase::FetchingRelease);
    let version = fetch_release(transport, anchor)?;
    observer.version(&version.to_string());
    stop_if_cancelled(observer)?;
    let target = app_managed_binary(request.data_dir);
    match installed_version(&target) {
        Some(installed) if installed == version => {
            return Ok(Preparation::AlreadyCurrent(version));
        }
        // The release information is not signed; an older release is never installed over a
        // newer one.
        Some(installed) if installed > version => {
            return Err(InstallError::OlderThanInstalled {
                release: version.to_string(),
                installed: installed.to_string(),
            });
        }
        _ => {}
    }
    let plan = DownloadPlan::new(anchor, &version, request.arch);
    observer.phase(ResticInstallPhase::Downloading);
    let sums = fetch_asset(
        transport,
        anchor,
        &plan.sums_url,
        RELEASE_INFO_LIMIT,
        "SHA256SUMS",
    )?;
    let signature = fetch_asset(
        transport,
        anchor,
        &plan.signature_url,
        SIGNATURE_LIMIT,
        "SHA256SUMS.asc",
    )?;
    prepare_data_dir(request.data_dir)?;
    observer.phase(ResticInstallPhase::Verifying);
    verify_signature(&gpgv, anchor, request.data_dir, &sums, &signature)?;
    let expected = expected_sha256(&String::from_utf8_lossy(&sums), &plan.archive_name)?;
    stop_if_cancelled(observer)?;
    observer.phase(ResticInstallPhase::Downloading);
    let archive = fetch_asset(
        transport,
        anchor,
        &plan.archive_url,
        ARCHIVE_LIMIT,
        &plan.archive_name,
    )?;
    observer.phase(ResticInstallPhase::Verifying);
    check_sha256(&archive, &expected, &plan.archive_name)?;
    stop_if_cancelled(observer)?;

    let temp = request.data_dir.join(temp_name());
    let mut file = create_exclusive(&temp).map_err(write_failed)?;
    let mut prepared = PreparedInstall {
        temp,
        target,
        dir: request.data_dir.to_path_buf(),
        version: version.clone(),
        committed: false,
    };
    unpack(&archive, &mut file, &plan.archive_name)?;
    file.sync_all().map_err(write_failed)?;
    // The file must be closed before it runs (Linux refuses to run a file open for writing).
    drop(file);
    prepared.version = check_binary_version(&prepared.temp, &version, VERSION_TIMEOUT)?;
    stop_if_cancelled(observer)?;
    Ok(Preparation::Ready(prepared))
}

fn write_failed(err: std::io::Error) -> InstallError {
    InstallError::WriteFailed(err.to_string())
}

/// Creates Kestic's data folder (0700) if needed; an existing one loses any access for group
/// and others, and is never widened. A symbolic link in its place is refused, so the
/// permissions of its target stay as they are.
fn prepare_data_dir(dir: &Path) -> Result<(), InstallError> {
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent).map_err(write_failed)?;
    }
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => return Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(write_failed(err)),
    }
    let meta = fs::symlink_metadata(dir).map_err(write_failed)?;
    if !meta.is_dir() {
        return Err(InstallError::WriteFailed(format!(
            "{} is not a folder",
            dir.display()
        )));
    }
    let mode = meta.permissions().mode() & 0o7777;
    if mode & 0o077 != 0 {
        fs::set_permissions(dir, fs::Permissions::from_mode(mode & !0o077))
            .map_err(write_failed)?;
    }
    Ok(())
}

/// What [`remove_leftovers`] did.
#[derive(Debug, Default)]
pub struct LeftoverCleanup {
    pub removed: usize,
    /// Entries that could not be removed, or the folder that could not be read.
    pub failed: Vec<(PathBuf, std::io::Error)>,
}

/// Removes what an interrupted installation left in `data_dir`: temporary files and `gpgv`
/// folders whose names start with [`TEMP_PREFIX`]. Symbolic links are removed, never followed;
/// an entry that cannot be removed does not stop the others.
pub fn remove_leftovers(data_dir: &Path) -> LeftoverCleanup {
    let mut cleanup = LeftoverCleanup::default();
    match fs::symlink_metadata(data_dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return cleanup,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return cleanup,
        Err(err) => {
            cleanup.failed.push((data_dir.to_path_buf(), err));
            return cleanup;
        }
    }
    let entries = match fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(err) => {
            cleanup.failed.push((data_dir.to_path_buf(), err));
            return cleanup;
        }
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .as_encoded_bytes()
            .starts_with(TEMP_PREFIX.as_bytes())
        {
            continue;
        }
        let path = entry.path();
        let removed = match entry.file_type() {
            Ok(kind) if kind.is_dir() => fs::remove_dir_all(&path),
            Ok(_) => fs::remove_file(&path),
            Err(err) => Err(err),
        };
        match removed {
            Ok(()) => cleanup.removed += 1,
            Err(err) => cleanup.failed.push((path, err)),
        }
    }
    cleanup
}

/// Version the installed app-managed binary reports, if it runs and says.
///
/// Runs without a read lease of the run gate: only the worker of an installation that still
/// runs can replace this binary (`begin_replace` refuses a cancelled one, which may still be
/// downloading), at most one installation runs, and its worker calls this before it replaces
/// anything.
fn installed_version(binary: &Path) -> Option<Version> {
    if !binary.is_file() {
        return None;
    }
    reported_version(binary, VERSION_TIMEOUT).ok()
}

/// The version `binary version --json` reports, or why there is none. Without a read lease of
/// the run gate: it runs the new binary in its temp file, which no restic process uses, or the
/// installed one before this worker replaces it (see [`installed_version`]).
fn reported_version(binary: &Path, limit: Duration) -> Result<Version, String> {
    let mut attempts = 0;
    let out = loop {
        let mut cmd = Command::new(binary);
        cmd.env_clear().args(["version", "--json"]);
        match output_within(cmd, limit) {
            // Another thread of this process may still hold the just-written file open in a
            // child between fork and exec; the kernel then refuses to run it for a moment.
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 5 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            other => break other.map_err(|err| err.to_string())?,
        }
    };
    if !out.status.success() {
        return Err(format!("`version --json` ended with {}", out.status));
    }
    parse_version_json(&out.stdout)
}

/// Runs `binary version --json` (at most `limit`) and checks the version against `release`.
pub(crate) fn check_binary_version(
    binary: &Path,
    release: &Version,
    limit: Duration,
) -> Result<Version, InstallError> {
    let found =
        reported_version(binary, limit).map_err(|detail| InstallError::VersionMismatch {
            expected: release.to_string(),
            found: format!("no version ({detail})"),
        })?;
    check_version(&found, release)?;
    Ok(found)
}

/// Unpacks the bzip2 `archive` into `out`, at most [`UNPACKED_LIMIT`] bytes; returns the size.
pub(crate) fn unpack(archive: &[u8], out: &mut File, name: &str) -> Result<u64, InstallError> {
    let mut decoder = bzip2::read::BzDecoder::new(archive);
    let mut buf = vec![0_u8; 64 << 10];
    let mut written: u64 = 0;
    loop {
        let n = decoder
            .read(&mut buf)
            .map_err(|err| InstallError::AssetMissing(format!("{name} ({err})")))?;
        if n == 0 {
            break;
        }
        written += n as u64;
        if written > UNPACKED_LIMIT {
            return Err(InstallError::WriteFailed(format!(
                "{name} unpacks to more than {UNPACKED_LIMIT} bytes"
            )));
        }
        out.write_all(&buf[..n]).map_err(write_failed)?;
    }
    if written == 0 {
        return Err(InstallError::AssetMissing(format!("{name} (empty)")));
    }
    Ok(written)
}

/// A fresh random name for the temporary file.
pub(crate) fn temp_name() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut random = [0_u8; 8];
    if getrandom::fill(&mut random).is_err() {
        // Without entropy the name only has to differ within this process; creating the file
        // with O_EXCL keeps it from ever reusing an existing one.
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        random = (u64::from(std::process::id()) << 32 ^ n).to_le_bytes();
    }
    let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
    format!("{TEMP_PREFIX}{hex}")
}

/// Creates `path` as a new file only this user may read, write and run; fails if anything,
/// a symlink included, is already there.
pub(crate) fn create_exclusive(path: &Path) -> std::io::Result<File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(path)
}

/// Checks the version a new binary reported against the release it came from.
pub fn check_version(found: &Version, release: &Version) -> Result<(), InstallError> {
    if *found < MIN_RESTIC_VERSION {
        return Err(InstallError::TooOld {
            found: found.to_string(),
            min: MIN_RESTIC_VERSION.to_string(),
        });
    }
    if found != release {
        return Err(InstallError::VersionMismatch {
            expected: release.to_string(),
            found: found.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "restic_install_tests.rs"]
mod tests;
