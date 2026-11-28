// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! A local stand-in for restic's GitHub releases: a signing key made at test time with `gpg` in
//! a temporary `GNUPGHOME`, release files signed with it, and a plain HTTP server on a random
//! loopback port that serves them and records every request path.

use crate::tools::short_tempdir;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// An OpenPGP signing key that exists only in its own temporary `GNUPGHOME`.
pub struct TestSigningKey {
    home: tempfile::TempDir,
    fingerprint: String,
    export: PathBuf,
}

impl TestSigningKey {
    /// Creates a certify-only ed25519 primary key with an ed25519 signing subkey, without
    /// passphrase or expiry; `name` goes into its user ID. Signatures come from the subkey, as
    /// with keys that keep their primary key offline.
    pub fn generate(name: &str) -> Self {
        let home = short_tempdir("kestic-gpg");
        let uid = format!("{name} <{name}@kestic.invalid>");
        run_gpg(
            home.path(),
            "generate key",
            &[
                "--passphrase",
                "",
                "--quick-gen-key",
                &uid,
                "ed25519",
                "cert",
                "never",
            ],
        );
        let listing = run_gpg(
            home.path(),
            "list fingerprint",
            &["--with-colons", "--fingerprint", &uid],
        );
        let fingerprint = String::from_utf8_lossy(&listing)
            .lines()
            .find_map(|line| {
                line.strip_prefix("fpr:")
                    .and_then(|rest| rest.split(':').nth(8))
                    .map(str::to_string)
            })
            .expect("fingerprint of the generated key");
        run_gpg(
            home.path(),
            "add signing subkey",
            &[
                "--passphrase",
                "",
                "--quick-add-key",
                &fingerprint,
                "ed25519",
                "sign",
                "never",
            ],
        );
        let export = home.path().join("public.pgp");
        let export_arg = export.to_string_lossy().into_owned();
        run_gpg(
            home.path(),
            "export public key",
            &["--export", "--output", &export_arg, &fingerprint],
        );
        Self {
            home,
            fingerprint,
            export,
        }
    }

    /// Fingerprint of the primary key, 40 upper-case hex digits.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Binary export of the public key (a file `gpgv --keyring` reads).
    pub fn public_key_file(&self) -> &Path {
        &self.export
    }

    /// ASCII-armoured detached signature of `data`, as in `SHA256SUMS.asc`.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let input = self.home.path().join("to-sign");
        std::fs::write(&input, data).expect("write data to sign");
        let input = input.to_string_lossy().into_owned();
        run_gpg(
            self.home.path(),
            "sign",
            &[
                "--armor",
                "--detach-sign",
                "--local-user",
                &self.fingerprint,
                "--output",
                "-",
                &input,
            ],
        )
    }
}

/// One keyring file holding the public keys of `keys`, in `dir`.
pub fn keyring_of(dir: &Path, keys: &[&TestSigningKey]) -> PathBuf {
    let path = dir.join("keyring.pgp");
    let mut bytes = Vec::new();
    for key in keys {
        bytes.extend(std::fs::read(key.public_key_file()).expect("read key export"));
    }
    std::fs::write(&path, bytes).expect("write keyring");
    path
}

impl Drop for TestSigningKey {
    fn drop(&mut self) {
        let _ = Command::new("gpgconf")
            .args(["--kill", "all"])
            .env("GNUPGHOME", self.home.path())
            .status();
    }
}

/// Runs `gpg` in batch mode with `home` as `GNUPGHOME`; returns its stdout. A failure names only
/// `event`, never the arguments or gpg's output (they carry the key's user ID).
fn run_gpg(home: &Path, event: &'static str, args: &[&str]) -> Vec<u8> {
    let out = Command::new("gpg")
        .env("GNUPGHOME", home)
        .args(["--batch", "--quiet", "--pinentry-mode", "loopback"])
        .args(args)
        .output()
        .expect("run gpg");
    assert!(out.status.success(), "gpg {event} failed: {}", out.status);
    out.stdout
}

/// A restic stand-in: a shell script that answers `version --json` (and `version`) with
/// `version`. It ends with `exit 0`, so bytes appended to it are never run.
pub fn restic_stand_in_script(version: &str) -> Vec<u8> {
    format!(
        "#!/bin/sh\n\
         if [ \"$1\" = version ] && [ \"$2\" = --json ]; then\n\
         \x20 echo '{{\"message_type\":\"version\",\"version\":\"{version}\",\"go_version\":\"go1.25\",\"go_os\":\"linux\",\"go_arch\":\"amd64\"}}'\n\
         \x20 exit 0\n\
         fi\n\
         echo 'restic {version} compiled with go1.25 on linux/amd64'\n\
         exit 0\n"
    )
    .into_bytes()
}

/// [`restic_stand_in_script`] compressed with bzip2, like restic's release archives.
pub fn restic_stand_in_archive(version: &str) -> Vec<u8> {
    bzip2_compress(&restic_stand_in_script(version))
}

/// bzip2 compression of `data`.
pub fn bzip2_compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    encoder.write_all(data).expect("compress");
    encoder.finish().expect("finish bzip2 stream")
}

/// Lower-case hex SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The files of one restic release as the mock serves them.
pub struct ReleaseFiles {
    pub version: String,
    /// Asset name → content (archives, `SHA256SUMS`, `SHA256SUMS.asc`).
    pub assets: HashMap<String, Vec<u8>>,
    /// Body of `releases/latest`.
    pub latest_json: String,
}

impl ReleaseFiles {
    /// Release `version` with a stand-in archive for amd64 and arm64, `SHA256SUMS` over both and
    /// its signature by `key`.
    pub fn signed(version: &str, key: &TestSigningKey) -> Self {
        let mut assets = HashMap::new();
        for arch in ["amd64", "arm64"] {
            assets.insert(
                format!("restic_{version}_linux_{arch}.bz2"),
                restic_stand_in_archive(version),
            );
        }
        let mut files = Self {
            version: version.to_string(),
            assets,
            latest_json: latest_json(version),
        };
        files.resign(key);
        files
    }

    /// Rewrites `SHA256SUMS` from the archives and signs it with `key`.
    pub fn resign(&mut self, key: &TestSigningKey) {
        let mut names: Vec<&String> = self.assets.keys().filter(|n| n.ends_with(".bz2")).collect();
        names.sort();
        let sums: String = names
            .iter()
            .map(|name| format!("{}  {name}\n", sha256_hex(&self.assets[*name])))
            .collect();
        let signature = key.sign(sums.as_bytes());
        self.assets
            .insert("SHA256SUMS".to_string(), sums.into_bytes());
        self.assets.insert("SHA256SUMS.asc".to_string(), signature);
    }

    /// Name of the archive for `arch` (`amd64`, `arm64`).
    pub fn archive_name(&self, arch: &str) -> String {
        format!("restic_{}_linux_{arch}.bz2", self.version)
    }
}

/// A `releases/latest` answer for `version`, shaped like GitHub's (with a download URL that
/// points elsewhere, which Kestic must ignore).
pub fn latest_json(version: &str) -> String {
    format!(
        "{{\"tag_name\":\"v{version}\",\"name\":\"restic {version}\",\"draft\":false,\
         \"prerelease\":false,\"assets\":[{{\"name\":\"restic_{version}_linux_amd64.bz2\",\
         \"browser_download_url\":\"https://downloads.example.invalid/restic.bz2\"}}]}}"
    )
}

/// Paths the mock serves, relative to its base URL.
pub const LATEST_PATH: &str = "/repos/restic/restic/releases/latest";

/// Path of a release asset on the mock.
pub fn asset_path(version: &str, name: &str) -> String {
    format!("/restic/restic/releases/download/v{version}/{name}")
}

/// How the mock answers one path.
#[derive(Debug, Clone)]
pub enum Route {
    /// `200` with `Content-Length`.
    Body(Vec<u8>),
    /// `200` without `Content-Length`; the body ends when the connection closes.
    BodyUntilClose(Vec<u8>),
    /// `302` to the given `Location`.
    Redirect(String),
    /// `200` headers, then nothing until the client gives up.
    StallAfterHeaders,
}

/// HTTP server on `127.0.0.1:<random port>`; records each request path.
pub struct ReleaseMock {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ReleaseMock {
    /// Serves `files` at the paths of GitHub's API and download host.
    pub fn start(files: &ReleaseFiles) -> Self {
        Self::start_with(files, Vec::new())
    }

    /// Serves `files`, with `overrides` replacing or adding the answers for their paths.
    pub fn start_with(files: &ReleaseFiles, overrides: Vec<(String, Route)>) -> Self {
        let mut routes: HashMap<String, Route> = HashMap::new();
        routes.insert(
            LATEST_PATH.to_string(),
            Route::Body(files.latest_json.clone().into_bytes()),
        );
        for (name, body) in &files.assets {
            routes.insert(asset_path(&files.version, name), Route::Body(body.clone()));
        }
        routes.extend(overrides);
        Self::serve_routes(routes)
    }

    /// A server that answers every path with `404`.
    pub fn empty() -> Self {
        Self::serve_routes(HashMap::new())
    }

    fn serve_routes(routes: HashMap<String, Route>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind release mock");
        let port = listener.local_addr().expect("local addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let requests = Arc::clone(&requests);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        let routes = routes.clone();
                        let requests = Arc::clone(&requests);
                        std::thread::spawn(move || serve(stream, &routes, &requests));
                    }
                }
            })
        };
        Self {
            port,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// `http://127.0.0.1:<port>`, the value for `KESTIC_TEST_RELEASE_URL`.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Request paths in the order they arrived.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("request log")
            .iter()
            .map(|logged| {
                logged
                    .trim_end_matches(JSON_ACCEPTED)
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Paths of the requests that asked for GitHub's JSON media type (`Accept`).
    pub fn json_requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("request log")
            .iter()
            .filter_map(|logged| logged.strip_suffix(JSON_ACCEPTED))
            .map(|path| path.trim_end().to_string())
            .collect()
    }
}

impl Drop for ReleaseMock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Mark in the request log of a request that asked for GitHub's JSON media type.
const JSON_ACCEPTED: &str = "[json]";

fn serve(stream: TcpStream, routes: &HashMap<String, Route>, requests: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut json_accepted = false;
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) | Err(_) => break,
            Ok(_) if header == "\r\n" || header == "\n" => break,
            Ok(_) => {
                json_accepted |= header
                    .to_ascii_lowercase()
                    .starts_with("accept: application/vnd.github+json");
            }
        }
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    let logged = if json_accepted {
        format!("{path} {JSON_ACCEPTED}")
    } else {
        path.clone()
    };
    requests.lock().expect("request log").push(logged);
    let mut stream = reader.into_inner();
    let route = routes
        .get(&path)
        .cloned()
        .unwrap_or_else(|| Route::Body(b"not found".to_vec()));
    let found = routes.contains_key(&path);
    let head = |status: &str, extra: &str| {
        format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Type: application/octet-stream\r\nConnection: close\r\n\r\n"
        )
    };
    let (head, body) = match route {
        Route::Body(body) => {
            let status = if found { "200 OK" } else { "404 Not Found" };
            (
                head(status, &format!("Content-Length: {}\r\n", body.len())),
                body,
            )
        }
        Route::BodyUntilClose(body) => (head("200 OK", ""), body),
        Route::Redirect(location) => (
            head(
                "302 Found",
                &format!("Location: {location}\r\nContent-Length: 0\r\n"),
            ),
            Vec::new(),
        ),
        Route::StallAfterHeaders => {
            let _ = stream.write_all(head("200 OK", "Content-Length: 1000\r\n").as_bytes());
            let _ = stream.flush();
            // quiet window: the body never comes; the client under test gives up first
            std::thread::sleep(std::time::Duration::from_secs(60));
            return;
        }
    };
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
}
