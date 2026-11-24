// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where the app-managed restic comes from and how its release files are checked: restic's
//! GitHub releases only, redirects only to GitHub's download hosts, `SHA256SUMS` signed by
//! restic's key (checked with `gpgv` against the key embedded here) and the archive's SHA-256.

use crate::provider::MIN_RESTIC_VERSION;
use kestic_i18n::codes::ResticInstallCode;
use semver::Version;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
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
/// Most bytes kept of a helper's stdout or stderr (`gpgv`, `restic version --json`).
const OUTPUT_LIMIT: u64 = 1 << 20;

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
    #[error("could not write the new restic: {0}")]
    WriteFailed(String),
}

impl InstallError {
    /// Language-independent code of this error.
    pub fn code(&self) -> ResticInstallCode {
        match self {
            Self::Network(_) => ResticInstallCode::Network,
            Self::HostNotAllowed(_) => ResticInstallCode::HostNotAllowed,
            Self::AssetMissing(_) => ResticInstallCode::AssetMissing,
            Self::UnsupportedArchitecture(_) => ResticInstallCode::UnsupportedArchitecture,
            Self::GpgvMissing => ResticInstallCode::GpgvMissing,
            Self::SignatureInvalid(_) => ResticInstallCode::SignatureInvalid,
            Self::ChecksumMismatch(_) => ResticInstallCode::ChecksumMismatch,
            Self::VersionMismatch { .. } => ResticInstallCode::VersionMismatch,
            Self::TooOld { .. } => ResticInstallCode::TooOld,
            Self::WriteFailed(_) => ResticInstallCode::WriteFailed,
        }
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
            Self::from_env()
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
            anchor.fingerprint = fingerprint;
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
        let network = |err: ureq::Error| InstallError::Network(format!("{url}: {err}"));
        let mut response = self
            .agent
            .get(url.as_str())
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(network)?;
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
        let body = response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(network)?;
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
    let version = release_version(tag).ok_or_else(|| missing(&format!("tag {tag:?}")))?;
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
                let next = url
                    .join(&location)
                    .map_err(|err| InstallError::HostNotAllowed(format!("{location}: {err}")))?;
                if !anchor.redirect_allowed(&next) {
                    return Err(InstallError::HostNotAllowed(next.to_string()));
                }
                url = next;
            }
            404 | 410 => return Err(InstallError::AssetMissing(what.to_string())),
            status => {
                return Err(InstallError::Network(format!(
                    "{url}: HTTP status {status}"
                )));
            }
        }
    }
    Err(InstallError::Network(format!(
        "{what}: more than {MAX_REDIRECTS} redirects"
    )))
}

/// `gpgv` from `search_path` (absolute entries only), if there is one.
pub fn find_gpgv(search_path: Option<&OsStr>) -> Option<PathBuf> {
    let absolute: Vec<PathBuf> = std::env::split_paths(search_path?)
        .filter(|dir| dir.is_absolute())
        .collect();
    let absolute = std::env::join_paths(absolute).ok()?;
    which::which_in("gpgv", Some(absolute), "/").ok()
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
        InstallError::SignatureInvalid(format!("{reason}; gpgv: {}", stderr.trim()))
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

/// Reads `gpgv --status-fd 1` output: valid only if `gpgv` succeeded, a `VALIDSIG` line names
/// `fingerprint` as the primary key, and no line reports a bad, expired-key, revoked-key or
/// unverifiable signature. The error is the reason.
pub fn check_gpgv_status(status: &str, success: bool, fingerprint: &str) -> Result<(), String> {
    const REFUSING: [&str; 5] = ["BADSIG", "EXPSIG", "EXPKEYSIG", "REVKEYSIG", "ERRSIG"];
    let mut valid = false;
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
        // VALIDSIG <fpr> <date> <timestamp> <expire> <version> <reserved> <pubkey-algo>
        // <hash-algo> <sig-class> <primary-key-fpr>
        if keyword == "VALIDSIG" && rest.len() >= 10 && rest.last() == Some(&fingerprint) {
            valid = true;
        }
    }
    if !success {
        return Err("gpgv did not accept the signature".to_string());
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

/// Runs `cmd` with stdout and stderr captured and kills it after `limit`.
pub(crate) fn output_within(mut cmd: Command, limit: Duration) -> std::io::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Each pipe is read on its own thread, at most OUTPUT_LIMIT bytes. A process the child left
    // in the background may keep a pipe open; the reader then never finishes, so the result is
    // awaited only until the deadline.
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = pipe {
                // Whatever arrived before an error is all there is.
                let _ = pipe.take(OUTPUT_LIMIT).read_to_end(&mut buf);
            }
            // The receiver is gone once the deadline has passed.
            let _ = sender.send(buf);
        });
        receiver
    };
    let stdout = read(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = read(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("no answer within {} s", limit.as_secs_f32()),
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let collected = |receiver: std::sync::mpsc::Receiver<Vec<u8>>| {
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("output not closed within {} s", limit.as_secs_f32()),
                )
            })
    };
    Ok(Output {
        status,
        stdout: collected(stdout)?,
        stderr: collected(stderr)?,
    })
}

#[cfg(test)]
#[path = "restic_release_tests.rs"]
mod tests;
