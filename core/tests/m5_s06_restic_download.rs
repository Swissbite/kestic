// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Integration tests for M5-S06a: downloading, checking and installing the app-managed restic
//! from a local release mock signed with a test key (feature `test-trust-anchor`).

use kestic_core::restic_install::{
    InstallOutcome, InstallRequest, PreparedInstall, TEMP_PREFIX, app_managed_binary,
    install_latest_with_anchor, kestic_data_dir, prepare_install_with_anchor,
};
use kestic_core::restic_release::{
    Arch, InstallError, RELEASE_INFO_LIMIT, SIGNATURE_LIMIT, Transport, TrustAnchor, UreqTransport,
};
use kestic_i18n::codes::ResticInstallCode;
use kestic_test_support::release_mock::{
    LATEST_PATH, ReleaseFiles, ReleaseMock, Route, TestSigningKey, asset_path, bzip2_compress,
    keyring_of, restic_stand_in_script,
};
use kestic_test_support::tools::write_executable;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const RELEASE: &str = "0.19.1";

/// A release mock, the key the test anchor trusts and a data folder below a temp
/// `XDG_DATA_HOME`.
struct Setup {
    trusted: TestSigningKey,
    mock: ReleaseMock,
    files: ReleaseFiles,
    _root: tempfile::TempDir,
    data_dir: PathBuf,
}

impl Setup {
    /// Release 0.19.1 signed by the trusted key, or by `signer` if given.
    fn new(signer: Option<&TestSigningKey>) -> Self {
        let trusted = TestSigningKey::generate("kestic-test-release");
        let files = ReleaseFiles::signed(RELEASE, signer.unwrap_or(&trusted));
        Self::serve(trusted, files)
    }

    fn serve(trusted: TestSigningKey, files: ReleaseFiles) -> Self {
        Self::serve_with(trusted, files, Vec::new())
    }

    fn serve_with(
        trusted: TestSigningKey,
        files: ReleaseFiles,
        routes: Vec<(String, Route)>,
    ) -> Self {
        let mock = ReleaseMock::start_with(&files, routes);
        let root = tempfile::tempdir().expect("temp root");
        let xdg_data_home = root.path().join("data");
        let data_dir = kestic_data_dir(
            Some(root.path().join("home").as_os_str()),
            Some(xdg_data_home.as_os_str()),
        )
        .expect("data dir from XDG_DATA_HOME");
        Self {
            trusted,
            mock,
            files,
            _root: root,
            data_dir,
        }
    }

    fn anchor(&self) -> TrustAnchor {
        self.anchor_with_keyring(self.trusted.public_key_file())
    }

    /// Test anchor trusting the fingerprint of the trusted key, with `keyring` as key export.
    fn anchor_with_keyring(&self, keyring: &Path) -> TrustAnchor {
        let url = self.mock.base_url();
        let key = keyring.to_string_lossy().into_owned();
        let fpr = self.trusted.fingerprint().to_string();
        TrustAnchor::from_lookup(|name| match name {
            "KESTIC_TEST_RELEASE_URL" => Some(url.clone()),
            "KESTIC_TEST_SIGNING_KEY" => Some(key.clone()),
            "KESTIC_TEST_SIGNING_FPR" => Some(fpr.clone()),
            _ => None,
        })
        .expect("test anchor")
    }

    fn request<'a>(&'a self, path: &'a Option<OsString>) -> InstallRequest<'a> {
        InstallRequest {
            data_dir: &self.data_dir,
            arch: Arch::Amd64,
            search_path: path.as_deref(),
        }
    }

    /// Puts an app-managed restic 0.18.0 stand-in in place; returns its bytes and inode.
    fn existing_binary(&self) -> (Vec<u8>, u64) {
        fs::create_dir_all(&self.data_dir).expect("data dir");
        let binary = app_managed_binary(&self.data_dir);
        write_executable(
            &binary,
            "#!/bin/sh\necho '{\"message_type\":\"version\",\"version\":\"0.18.0\"}'\n",
        );
        let meta = fs::metadata(&binary).expect("old binary");
        (fs::read(&binary).expect("read old binary"), meta.ino())
    }

    fn entries(&self) -> Vec<String> {
        let Ok(dir) = fs::read_dir(&self.data_dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = dir
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn archive_path(&self) -> String {
        asset_path(RELEASE, &self.files.archive_name("amd64"))
    }
}

fn path_env() -> Option<OsString> {
    std::env::var_os("PATH")
}

fn prepare(setup: &Setup, anchor: &TrustAnchor) -> Result<PreparedInstall, InstallError> {
    let path = path_env();
    prepare_install_with_anchor(&setup.request(&path), anchor)
        .map(|p| p.into_ready().expect("a new binary, not already current"))
}

/// Version a restic binary reports with `version --json`.
fn reported_version(binary: &Path) -> String {
    let out = Command::new(binary)
        .args(["version", "--json"])
        .output()
        .expect("run restic version --json");
    assert!(
        out.status.success(),
        "{} version --json failed",
        binary.display()
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("version JSON");
    json["version"].as_str().expect("version field").to_string()
}

/// The old binary is unchanged: same bytes, same inode, still executable and runnable.
fn assert_old_binary_kept(setup: &Setup, old: &(Vec<u8>, u64)) {
    let binary = app_managed_binary(&setup.data_dir);
    assert_eq!(
        fs::read(&binary).expect("read binary"),
        old.0,
        "bytes changed"
    );
    let meta = fs::metadata(&binary).expect("binary");
    assert_eq!(meta.ino(), old.1, "binary was replaced");
    assert_eq!(meta.permissions().mode() & 0o777, 0o755);
    assert_eq!(reported_version(&binary), "0.18.0");
}

/// AC-M5-06-1: a valid release ends up as an executable restic in `$XDG_DATA_HOME/kestic/`.
#[test]
fn ac_m5_06_1_download_installs_verified_binary() {
    let setup = Setup::new(None);
    let path = path_env();
    let installed = install_latest_with_anchor(&setup.request(&path), &setup.anchor())
        .expect("install from the release mock");
    assert_eq!(
        installed,
        InstallOutcome::Installed(RELEASE.parse().expect("version"))
    );

    // (a) an executable binary in the data folder below XDG_DATA_HOME
    let binary = app_managed_binary(&setup.data_dir);
    assert!(
        setup.data_dir.ends_with("data/kestic"),
        "{}",
        setup.data_dir.display()
    );
    let meta = fs::symlink_metadata(&binary).expect("installed binary");
    assert!(meta.file_type().is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o755);
    assert_eq!(
        fs::metadata(&setup.data_dir)
            .expect("data dir")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(setup.entries(), vec!["restic".to_string()]);
    // (b)
    assert_eq!(reported_version(&binary), RELEASE);
}

/// M5-S06: only the request for the release information asks for GitHub's JSON media type; the
/// downloads of the release's files do not.
#[test]
fn m5_06_only_the_release_information_asks_for_json() {
    let setup = Setup::new(None);
    let path = path_env();
    install_latest_with_anchor(&setup.request(&path), &setup.anchor())
        .expect("install from the release mock");
    assert_eq!(
        setup.mock.json_requests(),
        vec!["/repos/restic/restic/releases/latest".to_string()]
    );
    assert!(
        setup.mock.requests().len() > 1,
        "{:?}",
        setup.mock.requests()
    );
}

/// AC-M5-06-4: `SHA256SUMS.asc` signed by another key stops the download, whether that key is
/// unknown to `gpgv` or in the keyring with a valid signature.
#[test]
fn ac_m5_06_4_foreign_signature_aborts_and_keeps_binary() {
    let foreign = TestSigningKey::generate("kestic-test-foreign");
    let setup = Setup::new(Some(&foreign));
    let old = setup.existing_binary();
    let keys = tempfile::tempdir().expect("keyring dir");
    let both = keyring_of(keys.path(), &[&setup.trusted, &foreign]);

    for anchor in [setup.anchor(), setup.anchor_with_keyring(&both)] {
        let error = prepare(&setup, &anchor).expect_err("a foreign signature must fail");
        // (a) aborted with a message
        assert_eq!(error.code(), Some(ResticInstallCode::SignatureInvalid));
        assert!(error.to_string().contains("signature"), "{error}");
        // (b) no new binary, no temporary file, the archive was never fetched
        assert_eq!(setup.entries(), vec!["restic".to_string()]);
        assert!(!setup.mock.requests().contains(&setup.archive_path()));
        // (c)
        assert_old_binary_kept(&setup, &old);
    }
}

/// AC-M5-06-5: an archive that does not match `SHA256SUMS` stops the download.
#[test]
fn ac_m5_06_5_checksum_mismatch_aborts_and_keeps_binary() {
    let trusted = TestSigningKey::generate("kestic-test-release");
    let mut files = ReleaseFiles::signed(RELEASE, &trusted);
    let archive = files.archive_name("amd64");
    // Not bzip2 at all: had it been unpacked before the checksum check, the error would name
    // an unusable archive instead of the checksum.
    *files.assets.get_mut(&archive).expect("archive") = b"tampered, not bzip2".to_vec();
    let setup = Setup::serve(trusted, files);
    let old = setup.existing_binary();
    let error = prepare(&setup, &setup.anchor()).expect_err("a tampered archive must fail");

    // (a)
    assert_eq!(error, InstallError::ChecksumMismatch(archive));
    assert!(error.to_string().contains("checksum"), "{error}");
    // (b)
    assert_eq!(setup.entries(), vec!["restic".to_string()]);
    assert!(setup.mock.requests().contains(&setup.archive_path()));
    // (c)
    assert_old_binary_kept(&setup, &old);
}

/// AC-M5-06-7: the new binary is written and checked as a temporary file next to the old one,
/// then moved over it with `rename`; an abort before that keeps the old one.
#[test]
fn ac_m5_06_7_temp_file_then_rename_abort_keeps_old() {
    let setup = Setup::new(None);
    let old = setup.existing_binary();

    // (a) temporary file in the same folder, already checked; the old binary untouched
    let aborted = prepare(&setup, &setup.anchor()).expect("prepare");
    let temp = aborted.temp_path().to_path_buf();
    assert_eq!(temp.parent(), Some(setup.data_dir.as_path()));
    let temp_name = temp
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned();
    assert!(temp_name.starts_with(TEMP_PREFIX), "{temp_name}");
    assert_eq!(aborted.version().to_string(), RELEASE);
    assert_eq!(reported_version(&temp), RELEASE);
    assert_eq!(
        fs::read(app_managed_binary(&setup.data_dir)).expect("old"),
        old.0
    );

    // (c) abort before the rename: the old binary stays and runs, the temp file is gone
    drop(aborted);
    assert!(!temp.exists(), "temp file left after abort");
    assert_eq!(setup.entries(), vec!["restic".to_string()]);
    assert_old_binary_kept(&setup, &old);

    // (b) commit moves the temp file itself (same inode) to the target name
    let prepared = prepare(&setup, &setup.anchor()).expect("prepare again");
    let temp = prepared.temp_path().to_path_buf();
    let temp_inode = fs::metadata(&temp).expect("temp file").ino();
    assert_eq!(
        prepared.commit().map(|v| v.to_string()),
        Ok(RELEASE.to_string())
    );
    let binary = app_managed_binary(&setup.data_dir);
    assert_eq!(fs::metadata(&binary).expect("new binary").ino(), temp_inode);
    assert!(!temp.exists());
    assert_eq!(setup.entries(), vec!["restic".to_string()]);
    assert_eq!(reported_version(&binary), RELEASE);
}

/// AC-M5-06-13 (b): without `gpgv` in `PATH` nothing is fetched and nothing written.
#[test]
fn ac_m5_06_13_without_gpgv_no_request() {
    let setup = Setup::new(None);
    let empty_bin = tempfile::tempdir().expect("PATH without gpgv");
    let path = Some(empty_bin.path().as_os_str().to_owned());
    let error = install_latest_with_anchor(&setup.request(&path), &setup.anchor())
        .expect_err("no gpgv must fail");

    assert_eq!(error, InstallError::GpgvMissing);
    assert!(error.to_string().contains("gpgv"), "{error}");
    assert_eq!(setup.mock.requests(), Vec::<String>::new());
    assert_eq!(setup.entries(), Vec::<String>::new());

    // a gpgv that cannot be run counts as missing
    let fake = empty_bin.path().join("gpgv");
    fs::write(&fake, b"#!/bin/sh\nexit 0\n").expect("fake gpgv");
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o644)).expect("chmod");
    let error = install_latest_with_anchor(&setup.request(&path), &setup.anchor())
        .expect_err("a non-executable gpgv must fail");
    assert_eq!(error, InstallError::GpgvMissing);
    assert_eq!(setup.mock.requests(), Vec::<String>::new());
    // the mock answers when asked, so 0 requests is not an unreachable mock
    let probe = install_latest_with_anchor(&setup.request(&path_env()), &setup.anchor());
    assert!(probe.is_ok(), "{probe:?}");
    assert_eq!(
        setup.mock.requests().first().map(String::as_str),
        Some(LATEST_PATH)
    );
}

/// AC-M5-06-1: an existing data folder with 0755 ends up 0700, and a release archive of
/// realistic size installs, with and without `Content-Length`.
#[test]
fn ac_m5_06_1_existing_folder_narrowed_and_large_archive() {
    // restic's archives are about 10 MiB: pad the stand-in with bytes bzip2 cannot shrink
    let mut program = restic_stand_in_script(RELEASE);
    let mut noise = 0x2545_f491_u32;
    program.extend((0..12 << 20).map(|_| {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        noise.to_le_bytes()[0]
    }));
    let archive = bzip2_compress(&program);
    assert!(
        archive.len() > 11 << 20,
        "archive only {} bytes",
        archive.len()
    );

    for until_close in [false, true] {
        let trusted = TestSigningKey::generate("kestic-test-release");
        let mut files = ReleaseFiles::signed(RELEASE, &trusted);
        let archive_name = files.archive_name("amd64");
        files.assets.insert(archive_name.clone(), archive.clone());
        files.resign(&trusted);
        let route = if until_close {
            Route::BodyUntilClose(archive.clone())
        } else {
            Route::Body(archive.clone())
        };
        let setup = Setup::serve_with(
            trusted,
            files,
            vec![(asset_path(RELEASE, &archive_name), route)],
        );
        fs::create_dir_all(&setup.data_dir).expect("data dir");
        fs::set_permissions(&setup.data_dir, fs::Permissions::from_mode(0o755)).expect("chmod");

        let path = path_env();
        let outcome = install_latest_with_anchor(&setup.request(&path), &setup.anchor());
        assert_eq!(
            outcome,
            Ok(InstallOutcome::Installed(RELEASE.parse().expect("version")))
        );
        // (a)
        let binary = app_managed_binary(&setup.data_dir);
        assert_eq!(fs::read(&binary).expect("installed"), program);
        assert_eq!(
            fs::metadata(&setup.data_dir)
                .expect("dir")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        // (b)
        assert_eq!(reported_version(&binary), RELEASE);
    }
}

/// AC-M5-06-7: whatever sits at the target name is replaced by `rename` and never written
/// through; a failed `rename` or an unwritable folder leaves no temporary file and the old
/// binary as it was.
#[test]
fn ac_m5_06_7_foreign_target_and_failed_rename() {
    let setup = Setup::new(None);
    fs::create_dir_all(&setup.data_dir).expect("data dir");
    let binary = app_managed_binary(&setup.data_dir);

    // a symlink at the target: replaced by a regular file, its target untouched
    let elsewhere = setup._root.path().join("system-restic");
    fs::write(&elsewhere, b"not kestic's").expect("link target");
    std::os::unix::fs::symlink(&elsewhere, &binary).expect("symlink");
    let prepared = prepare(&setup, &setup.anchor()).expect("prepare");
    assert!(prepared.commit().is_ok());
    let meta = fs::symlink_metadata(&binary).expect("binary");
    assert!(meta.file_type().is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o755);
    assert_eq!(fs::read(&elsewhere).expect("link target"), b"not kestic's");

    // a non-empty directory at the target: the rename fails, nothing is left over
    fs::remove_file(&binary).expect("remove binary");
    fs::create_dir(&binary).expect("dir at target");
    fs::write(binary.join("inside"), b"x").expect("file in dir");
    let prepared = prepare(&setup, &setup.anchor()).expect("prepare");
    let error = prepared
        .commit()
        .expect_err("rename over a directory must fail");
    assert_eq!(error.code(), Some(ResticInstallCode::WriteFailed));
    assert_eq!(setup.entries(), vec!["restic".to_string()]);
    assert!(binary.join("inside").exists());
    fs::remove_dir_all(&binary).expect("remove dir");

    // a folder Kestic cannot write to: refused, the old binary as it was
    let old = setup.existing_binary();
    fs::set_permissions(&setup.data_dir, fs::Permissions::from_mode(0o500)).expect("chmod");
    let error = prepare(&setup, &setup.anchor()).expect_err("read-only folder must fail");
    fs::set_permissions(&setup.data_dir, fs::Permissions::from_mode(0o700)).expect("chmod");
    assert_eq!(error.code(), Some(ResticInstallCode::WriteFailed));
    assert_eq!(setup.entries(), vec!["restic".to_string()]);
    assert_old_binary_kept(&setup, &old);
}

/// AC-M5-06-7: when the app-managed restic already is the release's version, nothing is
/// downloaded or written.
#[test]
fn ac_m5_06_7_same_version_is_already_current() {
    let setup = Setup::new(None);
    fs::create_dir_all(&setup.data_dir).expect("data dir");
    let binary = app_managed_binary(&setup.data_dir);
    write_executable(
        &binary,
        &format!("#!/bin/sh\necho '{{\"message_type\":\"version\",\"version\":\"{RELEASE}\"}}'\n"),
    );
    let inode = fs::metadata(&binary).expect("binary").ino();
    let path = path_env();
    let outcome = install_latest_with_anchor(&setup.request(&path), &setup.anchor());
    assert_eq!(
        outcome,
        Ok(InstallOutcome::AlreadyCurrent(
            RELEASE.parse().expect("version")
        ))
    );
    assert_eq!(setup.mock.requests(), vec![LATEST_PATH.to_string()]);
    assert_eq!(fs::metadata(&binary).expect("binary").ino(), inode);
    assert_eq!(setup.entries(), vec!["restic".to_string()]);
}

/// A release older than the installed app-managed restic is refused before any download, and
/// the installed binary stays.
#[test]
fn older_release_than_installed_is_refused() {
    let setup = Setup::new(None);
    fs::create_dir_all(&setup.data_dir).expect("data dir");
    let binary = app_managed_binary(&setup.data_dir);
    write_executable(
        &binary,
        "#!/bin/sh\necho '{\"message_type\":\"version\",\"version\":\"0.20.0\"}'\n",
    );
    let inode = fs::metadata(&binary).expect("binary").ino();
    let path = path_env();
    let outcome = install_latest_with_anchor(&setup.request(&path), &setup.anchor());
    // (a) refused with the release and the installed version
    assert_eq!(
        outcome,
        Err(InstallError::OlderThanInstalled {
            release: RELEASE.to_string(),
            installed: "0.20.0".to_string(),
        })
    );
    assert_eq!(
        InstallError::OlderThanInstalled {
            release: RELEASE.to_string(),
            installed: "0.20.0".to_string(),
        }
        .code(),
        Some(ResticInstallCode::TooOld)
    );
    // (b) nothing but the release information was fetched
    assert_eq!(setup.mock.requests(), vec![LATEST_PATH.to_string()]);
    // (c) the installed binary stays
    assert_eq!(fs::metadata(&binary).expect("binary").ino(), inode);
    assert_eq!(setup.entries(), vec!["restic".to_string()]);
}

/// AC-M5-06-11: the real transport does not follow a redirect itself; one to another host is
/// refused before that host gets a request.
#[test]
fn ac_m5_06_11_real_transport_refuses_foreign_redirect() {
    let other = ReleaseMock::empty();
    let trusted = TestSigningKey::generate("kestic-test-release");
    let files = ReleaseFiles::signed(RELEASE, &trusted);
    let sums = asset_path(RELEASE, "SHA256SUMS");
    let setup = Setup::serve_with(
        trusted,
        files,
        vec![(
            sums.clone(),
            Route::Redirect(format!("{}/SHA256SUMS", other.base_url())),
        )],
    );
    let error = prepare(&setup, &setup.anchor()).expect_err("foreign redirect must fail");
    assert_eq!(error.code(), Some(ResticInstallCode::HostNotAllowed));
    assert_eq!(other.requests(), Vec::<String>::new());
    assert_eq!(setup.mock.requests(), vec![LATEST_PATH.to_string(), sums]);
    assert_eq!(setup.entries(), Vec::<String>::new());

    // a redirect back to the mock itself is followed
    let trusted = TestSigningKey::generate("kestic-test-release");
    let files = ReleaseFiles::signed(RELEASE, &trusted);
    let moved = "/moved/SHA256SUMS".to_string();
    let sums_body = files.assets["SHA256SUMS"].clone();
    let setup = Setup::serve_with(
        trusted,
        files,
        vec![
            (
                asset_path(RELEASE, "SHA256SUMS"),
                Route::Redirect(moved.clone()),
            ),
            (moved.clone(), Route::Body(sums_body)),
        ],
    );
    assert!(prepare(&setup, &setup.anchor()).is_ok());
    assert!(setup.mock.requests().contains(&moved));
}

/// AC-M5-06-11: a server that stops sending after the headers ends the download.
#[test]
fn ac_m5_06_11_stalled_body_times_out() {
    let mock = ReleaseMock::start_with(
        &ReleaseFiles {
            version: RELEASE.to_string(),
            assets: Default::default(),
            latest_json: String::new(),
        },
        vec![("/stall".to_string(), Route::StallAfterHeaders)],
    );
    let transport =
        UreqTransport::with_timeouts(Duration::from_secs(5), Duration::from_millis(500));
    let url = url::Url::parse(&format!("{}/stall", mock.base_url())).expect("url");
    let started = Instant::now();
    let result = transport.get(&url, RELEASE_INFO_LIMIT);
    assert!(
        matches!(result, Err(InstallError::Network(_))),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

/// AC-M5-06-11 (size caps): the real transport stops reading a body above its limit, with and
/// without `Content-Length`, and still reads a body of exactly the limit.
#[test]
fn ac_m5_06_11_real_transport_stops_at_size_limit() {
    let limit = usize::try_from(SIGNATURE_LIMIT).expect("limit fits");
    let mock = ReleaseMock::start_with(
        &ReleaseFiles {
            version: RELEASE.to_string(),
            assets: Default::default(),
            latest_json: String::new(),
        },
        vec![
            ("/exact".to_string(), Route::Body(vec![b'x'; limit])),
            ("/over".to_string(), Route::Body(vec![b'x'; limit + 1])),
            (
                "/over-until-close".to_string(),
                Route::BodyUntilClose(vec![b'x'; limit + 1]),
            ),
        ],
    );
    let transport = UreqTransport::new();
    let get = |path: &str| {
        let url = url::Url::parse(&format!("{}{path}", mock.base_url())).expect("url");
        transport.get(&url, SIGNATURE_LIMIT)
    };

    let exact = get("/exact");
    assert!(
        matches!(&exact, Ok(answer) if answer.body.len() == limit),
        "{exact:?}"
    );
    let over = get("/over");
    assert!(matches!(over, Err(InstallError::Network(_))), "{over:?}");
    let over_until_close = get("/over-until-close");
    assert!(
        matches!(over_until_close, Err(InstallError::Network(_))),
        "{over_until_close:?}"
    );
}
