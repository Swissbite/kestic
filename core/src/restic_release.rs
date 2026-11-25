// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where the app-managed restic comes from and how its release files are checked: restic's
//! GitHub releases only, redirects only to GitHub's download hosts, `SHA256SUMS` signed by
//! restic's key (checked with `gpgv` against the key embedded here) and the archive's SHA-256.

use crate::bounded_command::output_within;
use crate::provider::MIN_RESTIC_VERSION;
use kestic_i18n::codes::ResticInstallCode;
use semver::Version;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use url::Url;

const RELEASE_API_URL: &str = "https://api.github.com/repos/restic/restic/releases/latest";
const DOWNLOAD_PREFIX: &str = "https://github.com/restic/restic/releases/download/";
/// Binary export of restic's release signing key (primary key and encryption subkey, no
/// third-party signatures).
const SIGNING_KEY: &[u8] = include_bytes!("restic_release/restic-signing-key.pgp");
const SIGNING_FINGERPRINT: &str = "CF8F18F2844575973F79D4E191A6868BD3F7A907";
/// GitHub serves release assets through redirects to subdomains of this domain.
const DOWNLOAD_REDIRECT_DOMAIN: &str = "githubusercontent.com";

/// The release information (about 65 KiB) and `SHA256SUMS` (about 3 KiB) stay far below this.
pub const RELEASE_INFO_LIMIT: u64 = 1 << 20;
pub const SIGNATURE_LIMIT: u64 = 64 << 10;
/// restic's archives are about 10 MiB.
pub const ARCHIVE_LIMIT: u64 = 64 << 20;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_REDIRECTS: usize = 5;
const GPGV_TIMEOUT: Duration = Duration::from_secs(30);
/// The `gpgv` of the system, which the .deb depends on.
pub const SYSTEM_GPGV: &str = "/usr/bin/gpgv";
/// Most characters of text from the network or from `gpgv` kept in an error detail.
pub const DETAIL_LIMIT: usize = 200;

/// Why a restic release could not be installed; [`InstallError::code`] is the code the daemon
/// reports, the text is the raw detail.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    #[error("download from the restic release server failed: {0}")]
    Network(String),
    #[error("the address {0} is not one of restic's release hosts")]
    HostNotAllowed(String),
    #[error("the restic release has no usable {0}")]
    AssetMissing(String),
    #[error("restic offers no Linux build for the processor architecture {0}")]
    UnsupportedArchitecture(String),
    #[error("gpgv is not installed; Kestic needs it to check the signature of restic")]
    GpgvMissing,
    #[error("the signature of SHA256SUMS is not valid for restic's signing key: {0}")]
    SignatureInvalid(String),
    #[error("{0} does not match its checksum in SHA256SUMS")]
    ChecksumMismatch(String),
    #[error("the new restic reports version {found} instead of {expected}")]
    VersionMismatch { expected: String, found: String },
    #[error("restic {found} is too old; Kestic needs at least {min}")]
    TooOld { found: String, min: String },
    #[error("the latest restic release {release} is older than the installed restic {installed}")]
    OlderThanInstalled { release: String, installed: String },
    #[error("could not write the new restic: {0}")]
    WriteFailed(String),
    #[error("the installation was cancelled")]
    Cancelled,
}

impl InstallError {
    /// Language-independent code of this error; a cancelled installation has none.
    pub fn code(&self) -> Option<ResticInstallCode> {
        Some(match self {
            Self::Network(_) => ResticInstallCode::Network,
            Self::HostNotAllowed(_) => ResticInstallCode::HostNotAllowed,
            Self::AssetMissing(_) => ResticInstallCode::AssetMissing,
            Self::UnsupportedArchitecture(_) => ResticInstallCode::UnsupportedArchitecture,
            Self::GpgvMissing => ResticInstallCode::GpgvMissing,
            Self::SignatureInvalid(_) => ResticInstallCode::SignatureInvalid,
            Self::ChecksumMismatch(_) => ResticInstallCode::ChecksumMismatch,
            Self::VersionMismatch { .. } => ResticInstallCode::VersionMismatch,
            Self::TooOld { .. } | Self::OlderThanInstalled { .. } => ResticInstallCode::TooOld,
            Self::WriteFailed(_) => ResticInstallCode::WriteFailed,
            Self::Cancelled => return None,
        })
    }
}

/// Processor architecture of a restic Linux build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    Amd64,
    Arm64,
}

impl Arch {
    /// From a Rust architecture name (`std::env::consts::ARCH`).
    pub fn from_rust(arch: &str) -> Result<Self, InstallError> {
        match arch {
            "x86_64" => Ok(Self::Amd64),
            "aarch64" => Ok(Self::Arm64),
            other => Err(InstallError::UnsupportedArchitecture(other.to_string())),
        }
    }

    /// The architecture this program runs on.
    pub fn host() -> Result<Self, InstallError> {
        Self::from_rust(std::env::consts::ARCH)
    }

    /// Name in restic's asset names.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
        }
    }
}

/// Where releases come from and which key signs them. The release build has only
/// [`TrustAnchor::production`]; other anchors exist only with the feature `test-trust-anchor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustAnchor {
    api_url: String,
    download_prefix: String,
    key: Cow<'static, [u8]>,
    fingerprint: String,
    /// Origin of a release mock that redirects may also lead to (test anchors only).
    mock_origin: Option<url::Origin>,
}

impl TrustAnchor {
    /// restic's GitHub releases and restic's signing key.
    pub fn production() -> Self {
        Self {
            api_url: RELEASE_API_URL.to_string(),
            download_prefix: DOWNLOAD_PREFIX.to_string(),
            key: Cow::Borrowed(SIGNING_KEY),
            fingerprint: SIGNING_FINGERPRINT.to_string(),
            mock_origin: None,
        }
    }

    /// The anchor of this build: the production one, or in a build with `test-trust-anchor` the
    /// one the test variables describe.
    pub fn for_this_build() -> Result<Self, InstallError> {
        #[cfg(feature = "test-trust-anchor")]
        {
            let set = TEST_ANCHOR
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            set.map_or_else(Self::from_env, Ok)
        }
        #[cfg(not(feature = "test-trust-anchor"))]
        {
            Ok(Self::production())
        }
    }

    /// Fingerprint of the signing key's primary key, 40 upper-case hex digits.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Fingerprint in groups of four, as restic's documentation shows it.
    pub fn fingerprint_grouped(&self) -> String {
        let chars: Vec<char> = self.fingerprint.chars().collect();
        chars
            .chunks(4)
            .map(|group| group.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Binary export of the signing key.
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// URL of the release information.
    pub fn release_api_url(&self) -> &str {
        &self.api_url
    }

    /// URL of the asset `name` of release `version`.
    pub fn asset_url(&self, version: &Version, name: &str) -> String {
        format!("{}v{version}/{name}", self.download_prefix)
    }

    /// Whether a redirect may lead to `url`: only `https` on the default port to a subdomain of
    /// `githubusercontent.com` (not the domain itself), or, for a test anchor, back to the
    /// release mock.
    pub fn redirect_allowed(&self, url: &Url) -> bool {
        if self.mock_origin.as_ref() == Some(&url.origin()) {
            return true;
        }
        let Some(url::Host::Domain(host)) = url.host() else {
            return false;
        };
        url.scheme() == "https"
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && host
                .strip_suffix(DOWNLOAD_REDIRECT_DOMAIN)
                .and_then(|sub| sub.strip_suffix('.'))
                .is_some_and(|labels| {
                    labels.split('.').all(|label| {
                        !label.is_empty()
                            && label
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    })
                })
    }
}

/// Anchor that [`TrustAnchor::for_this_build`] returns in a test build, if set.
#[cfg(feature = "test-trust-anchor")]
static TEST_ANCHOR: std::sync::Mutex<Option<TrustAnchor>> = std::sync::Mutex::new(None);

/// Makes [`TrustAnchor::for_this_build`] return `anchor` (`None`: the anchor of the test
/// variables again), for tests that run the installation inside the daemon.
#[cfg(feature = "test-trust-anchor")]
pub fn set_test_anchor(anchor: Option<TrustAnchor>) {
    *TEST_ANCHOR
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = anchor;
}

#[cfg(feature = "test-trust-anchor")]
impl TrustAnchor {
    /// Variable with the base URL of a release mock (`http://127.0.0.1:<port>`).
    pub const RELEASE_URL_VAR: &'static str = "KESTIC_TEST_RELEASE_URL";
    /// Variable with the path of a binary key export.
    pub const SIGNING_KEY_VAR: &'static str = "KESTIC_TEST_SIGNING_KEY";
    /// Variable with the primary key's fingerprint.
    pub const SIGNING_FPR_VAR: &'static str = "KESTIC_TEST_SIGNING_FPR";

    /// [`TrustAnchor::from_lookup`] over the environment of this process.
    pub fn from_env() -> Result<Self, InstallError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// The production anchor with each of the three test variables that `lookup` knows
    /// replacing its part: the release URL (plain `http` only on 127.0.0.1), the key (path of a
    /// binary export) and the fingerprint.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, InstallError> {
        let mut anchor = Self::production();
        if let Some(base) = lookup(Self::RELEASE_URL_VAR) {
            let url = Url::parse(&base)
                .map_err(|err| InstallError::HostNotAllowed(format!("{base}: {err}")))?;
            let loopback = url.scheme() == "http"
                && url.host() == Some(url::Host::Ipv4(std::net::Ipv4Addr::LOCALHOST));
            if !loopback && url.scheme() != "https" {
                return Err(InstallError::HostNotAllowed(base));
            }
            let base = base.trim_end_matches('/');
            anchor.api_url = format!("{base}/repos/restic/restic/releases/latest");
            anchor.download_prefix = format!("{base}/restic/restic/releases/download/");
            anchor.mock_origin = Some(url.origin());
        }
        if let Some(path) = lookup(Self::SIGNING_KEY_VAR) {
            let key = std::fs::read(&path)
                .map_err(|err| InstallError::SignatureInvalid(format!("{path}: {err}")))?;
            anchor.key = Cow::Owned(key);
        }
        if let Some(fingerprint) = lookup(Self::SIGNING_FPR_VAR) {
            // gpgv reports fingerprints in upper case and without spaces.
            anchor.fingerprint = fingerprint
                .chars()
                .filter(|c| !c.is_whitespace())
                .map(|c| c.to_ascii_uppercase())
                .collect();
        }
        Ok(anchor)
    }
}

/// What to download for one release on one architecture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadPlan {
    pub version: Version,
    pub arch: Arch,
    pub archive_name: String,
    pub archive_url: String,
    pub sums_url: String,
    pub signature_url: String,
}

impl DownloadPlan {
    /// The asset names and URLs of release `version` for `arch`.
    pub fn new(anchor: &TrustAnchor, version: &Version, arch: Arch) -> Self {
        let archive_name = format!("restic_{version}_linux_{}.bz2", arch.as_str());
        Self {
            version: version.clone(),
            arch,
            archive_url: anchor.asset_url(version, &archive_name),
            archive_name,
            sums_url: anchor.asset_url(version, "SHA256SUMS"),
            signature_url: anchor.asset_url(version, "SHA256SUMS.asc"),
        }
    }
}

/// One HTTP answer: its status, its `Location` header and, unless it redirects, its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpAnswer {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

/// One `GET` without following redirects; a body longer than `limit` is an error.
pub trait Transport {
    fn get(&self, url: &Url, limit: u64) -> Result<HttpAnswer, InstallError>;
}

/// [`Transport`] over HTTPS with the system's certificates.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqTransport {
    /// Answers must start within 30 s and finish within 10 min.
    pub fn new() -> Self {
        Self::with_timeouts(Duration::from_secs(30), Duration::from_secs(600))
    }

    /// A transport whose answers must start within `response` and finish within `body`.
    pub fn with_timeouts(response: Duration, body: Duration) -> Self {
        let tls_config = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls_config)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(response))
            .timeout_recv_body(Some(body))
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .user_agent("Kestic")
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Transport for UreqTransport {
    fn get(&self, url: &Url, limit: u64) -> Result<HttpAnswer, InstallError> {
        self.get_observed(url, limit, &|_, _| true)
    }
}

/// Reports the bytes of a body read so far and its announced size; `false` stops the download.
pub type BodyProgress<'a> = &'a dyn Fn(u64, Option<u64>) -> bool;

/// How many bytes are read between two progress reports.
const PROGRESS_CHUNK: usize = 64 << 10;

impl UreqTransport {
    /// [`Transport::get`] that reports the body's progress; a `false` from `progress` ends the
    /// download with [`InstallError::Cancelled`].
    pub fn get_observed(
        &self,
        url: &Url,
        limit: u64,
        progress: BodyProgress<'_>,
    ) -> Result<HttpAnswer, InstallError> {
        let network =
            |err: &dyn std::fmt::Display| InstallError::Network(clip(&format!("{url}: {err}")));
        let mut request = self.agent.get(url.as_str());
        // Only GitHub's API answers in its JSON media type; asset downloads get no such wish.
        if url.path().starts_with("/repos/") {
            request = request.header("Accept", "application/vnd.github+json");
        }
        let mut response = request.call().map_err(|err| network(&err))?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        if response.status().is_redirection() {
            return Ok(HttpAnswer {
                status,
                location,
                body: Vec::new(),
            });
        }
        let total = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let mut reader = response.body_mut().with_config().limit(limit).reader();
        let mut body = Vec::new();
        let mut chunk = vec![0_u8; PROGRESS_CHUNK];
        loop {
            if !progress(body.len() as u64, total) {
                return Err(InstallError::Cancelled);
            }
            let n = reader.read(&mut chunk).map_err(|err| network(&err))?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
        Ok(HttpAnswer {
            status,
            location,
            body,
        })
    }
}

/// Asks for the latest release and returns its version; refuses one below restic 0.18.0.
pub fn fetch_release(
    transport: &dyn Transport,
    anchor: &TrustAnchor,
) -> Result<Version, InstallError> {
    let body = fetch_asset(
        transport,
        anchor,
        anchor.release_api_url(),
        RELEASE_INFO_LIMIT,
        "release information",
    )?;
    let missing =
        |detail: &str| InstallError::AssetMissing(format!("release information ({detail})"));
    let info: serde_json::Value =
        serde_json::from_slice(&body).map_err(|err| missing(&err.to_string()))?;
    for flag in ["draft", "prerelease"] {
        if info.get(flag).and_then(serde_json::Value::as_bool) == Some(true) {
            return Err(missing(flag));
        }
    }
    let tag = info
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| missing("no tag_name"))?;
    let version = release_version(tag).ok_or_else(|| missing(&format!("tag {:?}", clip(tag))))?;
    if version < MIN_RESTIC_VERSION {
        return Err(InstallError::TooOld {
            found: version.to_string(),
            min: MIN_RESTIC_VERSION.to_string(),
        });
    }
    Ok(version)
}

/// The version of a tag `v<major>.<minor>.<patch>` (digits only, no leading zeros).
fn release_version(tag: &str) -> Option<Version> {
    let parts: Vec<&str> = tag.strip_prefix('v')?.split('.').collect();
    let [major, minor, patch] = parts.as_slice() else {
        return None;
    };
    let number = |part: &str| {
        let plain = !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'));
        plain.then(|| part.parse::<u64>().ok()).flatten()
    };
    Some(Version::new(number(major)?, number(minor)?, number(patch)?))
}

/// Downloads `url`, following at most [`MAX_REDIRECTS`] redirects the anchor allows; `what`
/// names the file in errors.
pub fn fetch_asset(
    transport: &dyn Transport,
    anchor: &TrustAnchor,
    url: &str,
    limit: u64,
    what: &str,
) -> Result<Vec<u8>, InstallError> {
    let mut url =
        Url::parse(url).map_err(|err| InstallError::HostNotAllowed(format!("{url}: {err}")))?;
    for _ in 0..=MAX_REDIRECTS {
        let answer = transport.get(&url, limit)?;
        match answer.status {
            200 => return Ok(answer.body),
            301 | 302 | 303 | 307 | 308 => {
                let location = answer.location.ok_or_else(|| {
                    InstallError::Network(format!("{url}: redirect without a target"))
                })?;
                let next = url.join(&location).map_err(|err| {
                    InstallError::HostNotAllowed(format!("{}: {err}", clip(&location)))
                })?;
                if !anchor.redirect_allowed(&next) {
                    return Err(InstallError::HostNotAllowed(clip(next.as_str())));
                }
                url = next;
            }
            404 | 410 => return Err(InstallError::AssetMissing(what.to_string())),
            status => {
                return Err(InstallError::Network(format!(
                    "{}: HTTP status {status}",
                    clip(url.as_str())
                )));
            }
        }
    }
    Err(InstallError::Network(format!(
        "{what}: more than {MAX_REDIRECTS} redirects"
    )))
}

/// The `gpgv` that checks restic's signature. The release build takes [`SYSTEM_GPGV`] (the
/// .deb depends on it) and otherwise a `gpgv` from `search_path` only if [`trusted_gpgv`]
/// accepts it: a `gpgv` that this user could change could fake a valid signature. A build with
/// `test-trust-anchor` takes any `gpgv` from `search_path`, so tests can replace it.
pub fn find_gpgv(search_path: Option<&OsStr>) -> Option<PathBuf> {
    #[cfg(feature = "test-trust-anchor")]
    {
        gpgv_in_path(search_path)
    }
    #[cfg(not(feature = "test-trust-anchor"))]
    {
        select_trusted_gpgv(Path::new(SYSTEM_GPGV), search_path)
    }
}

/// `system` if [`trusted_gpgv`] accepts it, else a `gpgv` from `search_path` it accepts.
pub fn select_trusted_gpgv(system: &Path, search_path: Option<&OsStr>) -> Option<PathBuf> {
    trusted_gpgv(system).or_else(|| gpgv_in_path(search_path).and_then(|path| trusted_gpgv(&path)))
}

/// `gpgv` from `search_path` (absolute entries only), if there is one.
pub fn gpgv_in_path(search_path: Option<&OsStr>) -> Option<PathBuf> {
    let absolute: Vec<PathBuf> = std::env::split_paths(search_path?)
        .filter(|dir| dir.is_absolute())
        .collect();
    let absolute = std::env::join_paths(absolute).ok()?;
    which::which_in("gpgv", Some(absolute), "/").ok()
}

/// The resolved path of `path` if it is a file that only root can change: the file and every
/// folder above it belong to root and are not writable by group or others.
pub fn trusted_gpgv(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let resolved = std::fs::canonicalize(path).ok()?;
    let only_root_writes = |meta: &std::fs::Metadata| meta.uid() == 0 && meta.mode() & 0o022 == 0;
    let file = std::fs::metadata(&resolved).ok()?;
    if !file.is_file() || !only_root_writes(&file) {
        return None;
    }
    for dir in resolved.ancestors().skip(1) {
        if !only_root_writes(&std::fs::metadata(dir).ok()?) {
            return None;
        }
    }
    Some(resolved)
}

/// Checks that `signature` is a valid signature of `sums` by the anchor's key, with `gpgv`
/// and a private `GNUPGHOME`: a new folder in `work_dir` that is removed afterwards.
pub fn verify_signature(
    gpgv: &Path,
    anchor: &TrustAnchor,
    work_dir: &Path,
    sums: &[u8],
    signature: &[u8],
) -> Result<(), InstallError> {
    let write_failed = |err: std::io::Error| InstallError::WriteFailed(err.to_string());
    let home = PrivateDir::create(work_dir).map_err(write_failed)?;
    let keyring = home.path.join("keyring.pgp");
    let sums_file = home.path.join("SHA256SUMS");
    let signature_file = home.path.join("SHA256SUMS.asc");
    std::fs::write(&keyring, anchor.key()).map_err(write_failed)?;
    std::fs::write(&sums_file, sums).map_err(write_failed)?;
    std::fs::write(&signature_file, signature).map_err(write_failed)?;

    let mut cmd = Command::new(gpgv);
    cmd.env_clear()
        .env("GNUPGHOME", &home.path)
        .env("LC_ALL", "C")
        .arg("--keyring")
        .arg(&keyring)
        .args(["--status-fd", "1"])
        .arg(&signature_file)
        .arg(&sums_file);
    let out = output_within(cmd, GPGV_TIMEOUT)
        .map_err(|err| InstallError::SignatureInvalid(format!("gpgv: {err}")))?;
    let status = String::from_utf8_lossy(&out.stdout);
    check_gpgv_status(&status, out.status.success(), anchor.fingerprint()).map_err(|reason| {
        let stderr = String::from_utf8_lossy(&out.stderr);
        InstallError::SignatureInvalid(format!("{reason}; gpgv: {}", clip(stderr.trim())))
    })
}

/// A new folder only this user can enter, removed with its content when dropped.
struct PrivateDir {
    path: PathBuf,
}

impl PrivateDir {
    fn create(parent: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let path = parent.join(format!("{}.gnupg", crate::restic_install::temp_name()));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self { path })
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Reads `gpgv --status-fd 1` output: valid only if `gpgv` succeeded, the file holds exactly
/// one signature, its `VALIDSIG` line names `fingerprint` as the primary key, and no line reports
/// a bad, expired-key, revoked-key or unverifiable signature. The error is the reason.
pub fn check_gpgv_status(status: &str, success: bool, fingerprint: &str) -> Result<(), String> {
    const REFUSING: [&str; 5] = ["BADSIG", "EXPSIG", "EXPKEYSIG", "REVKEYSIG", "ERRSIG"];
    let mut valid = false;
    let mut signatures = 0;
    let mut valid_lines = 0;
    for line in status.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("[GNUPG:]") {
            continue;
        }
        let keyword = fields.next().unwrap_or_default();
        let rest: Vec<&str> = fields.collect();
        if REFUSING.contains(&keyword) {
            return Err(format!("gpgv reports {keyword}"));
        }
        match keyword {
            "NEWSIG" => signatures += 1,
            "VALIDSIG" => valid_lines += 1,
            _ => {}
        }
        // VALIDSIG <fpr> <date> <timestamp> <expire> <version> <reserved> <pubkey-algo>
        // <hash-algo> <sig-class> <primary-key-fpr>
        if keyword == "VALIDSIG" && rest.len() >= 10 && rest.last() == Some(&fingerprint) {
            valid = true;
        }
    }
    if !success {
        return Err("gpgv did not accept the signature".to_string());
    }
    if signatures > 1 || valid_lines > 1 {
        return Err("SHA256SUMS.asc holds more than one signature".to_string());
    }
    if !valid {
        return Err(format!("no valid signature by the key {fingerprint}"));
    }
    Ok(())
}

/// The SHA-256 that `SHA256SUMS` lists for `name`.
pub fn expected_sha256(sums: &str, name: &str) -> Result<[u8; 32], InstallError> {
    let mut found = Vec::new();
    for line in sums.lines() {
        let line = line.trim_end_matches('\r');
        let Some((hash, file)) = line.split_once(' ') else {
            continue;
        };
        // `sha256sum` marks the file with a space (text mode) or `*` (binary mode).
        let file = file.strip_prefix(' ').or_else(|| file.strip_prefix('*'));
        if file != Some(name) {
            continue;
        }
        if let Some(digest) = parse_sha256(hash) {
            found.push(digest);
        }
    }
    match found.as_slice() {
        [digest] => Ok(*digest),
        [] => Err(InstallError::AssetMissing(format!(
            "SHA256SUMS line for {name}"
        ))),
        _ => Err(InstallError::ChecksumMismatch(name.to_string())),
    }
}

fn parse_sha256(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (byte, pair) in digest.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(digest)
}

/// Checks `data` against `expected`.
pub fn check_sha256(data: &[u8], expected: &[u8; 32], name: &str) -> Result<(), InstallError> {
    if Sha256::digest(data).as_slice() == expected.as_slice() {
        Ok(())
    } else {
        Err(InstallError::ChecksumMismatch(name.to_string()))
    }
}

/// `text` cut to at most [`DETAIL_LIMIT`] characters (an ellipsis marks a cut) and with control
/// characters (line breaks, escape sequences) and invisible format and direction characters as
/// spaces, for error details that come from the network or from `gpgv`.
pub fn clip(text: &str) -> String {
    let mut clipped: String = text
        .chars()
        .take(DETAIL_LIMIT)
        .map(|c| if is_hidden(c) { ' ' } else { c })
        .collect();
    if text.chars().nth(DETAIL_LIMIT).is_some() {
        clipped.push('…');
    }
    clipped
}

/// Control characters and the invisible characters that change how the text around them is
/// shown or hide text: zero-width and direction marks, direction embeddings, overrides and
/// isolates, line and paragraph separators, invisible operators, the byte order mark, the soft
/// hyphen, the grapheme joiner, variation selectors, interlinear annotation and tag
/// characters, the other format characters (Arabic, Syriac, Egyptian, shorthand, musical), and
/// the blank fillers (Hangul, Braille).
fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061C}'
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08E2}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{2800}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{110BD}'
                | '\u{110CD}'
                | '\u{13430}'..='\u{1343F}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0000}'..='\u{E007F}'
                | '\u{E0100}'..='\u{E01EF}'
        )
}

#[cfg(test)]
#[path = "restic_release_tests.rs"]
mod tests;
