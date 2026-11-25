// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Host keys of SFTP servers: which name ssh looks the server up by (`ssh -G`), what the three
//! `known_hosts` files say about it (`ssh-keygen -F`), which keys the server shows
//! (`ssh-keyscan`), and Kestic's own `known_hosts` file, the only one Kestic writes.

use crate::bounded_command::{Bounds, output_bounded};
use crate::sftp::{DEFAULT_PORT, KnownHostsPaths, ssh_config_args, ssh_host};
use crate::sync::LockUnpoisoned;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

/// Key types Kestic asks a server for, in the order it prefers them.
const SCAN_TYPES: &str = "ed25519,ecdsa,rsa";

/// Longest `ssh-keyscan` waits for a server.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest Kestic waits for `ssh -G`, `ssh-keygen` or `ssh-keyscan` (whose own limit is
/// [`SCAN_TIMEOUT`]).
const TOOL_TIMEOUT: Duration = Duration::from_secs(12);

/// Every write of Kestic's `known_hosts` file in this process, one after the other.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// How ssh reaches a server after `~/.ssh/config`: the host name, the port and the name of its
/// `known_hosts` entries (`HostKeyAlias`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub hostname: String,
    pub port: u16,
    pub host_key_alias: Option<String>,
}

impl SshTarget {
    /// The name ssh looks up in `known_hosts`: the alias, else the host on port 22 and
    /// `[host]:port` on any other port.
    pub fn known_hosts_name(&self) -> String {
        match &self.host_key_alias {
            Some(alias) => alias.clone(),
            None if self.port == DEFAULT_PORT => self.hostname.clone(),
            None => format!("[{}]:{}", self.hostname, self.port),
        }
    }
}

/// A host key shown by a server that no `known_hosts` file knows yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedHostKey {
    /// Name of the `known_hosts` line (see [`SshTarget::known_hosts_name`]).
    pub name: String,
    pub host: String,
    pub port: u16,
    pub key_type: String,
    /// The key in base64, as in `known_hosts`.
    pub key: String,
    /// `SHA256:…` as `ssh-keygen -l` shows it.
    pub fingerprint_sha256: String,
}

/// What the `known_hosts` files say about the key a server shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyCheck {
    /// A file has the key the server shows.
    Known,
    /// No file knows the server; the key to ask the user about.
    Unknown(ScannedHostKey),
    /// A file has another key of the same type for this server: that file.
    Changed(PathBuf),
    /// The server showed no key (not reachable, no ssh server); `String` is the reason.
    NoKey(String),
}

/// Why the host key could not be checked or written.
#[derive(Debug, thiserror::Error)]
pub enum HostKeyError {
    /// ssh reaches the server through another one (`ProxyJump`, `ProxyCommand`).
    #[error("the server '{0}' is reached through a jump host, which Kestic does not support yet")]
    JumpHost(String),
    /// The ssh configuration gives the server a name (`HostName`, `HostKeyAlias`) that would be
    /// a pattern or a broken line in `known_hosts`.
    #[error(
        "the ssh configuration names the server '{}', which cannot stand in a known_hosts line",
        .0.escape_default()
    )]
    UnusableName(String),
    #[error(transparent)]
    Paths(#[from] crate::sftp::KnownHostsPathError),
    #[error("{program} failed: {detail}")]
    Tool { program: String, detail: String },
    /// The program got no answer in time (a server that stops answering).
    #[error("{program} got no answer within {secs} s")]
    NoAnswer { program: String, secs: u64 },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// One `known_hosts` entry for a server: its marker (`@revoked`, if any), type and key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KnownEntry {
    revoked: bool,
    key_type: String,
    key: String,
}

fn run_tool(
    program: &str,
    args: &[String],
    stdin: Option<&str>,
) -> Result<std::process::Output, HostKeyError> {
    run_tool_within(program, args, stdin, TOOL_TIMEOUT)
}

/// [`run_tool`] that stops the program after `limit`.
fn run_tool_within(
    program: &str,
    args: &[String],
    stdin: Option<&str>,
    limit: Duration,
) -> Result<std::process::Output, HostKeyError> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    // ssh reads the user's configuration (`Match exec`) and resolves names; none of that may hold
    // up a connection test or the check before a run. Its own process group: at the time limit
    // the commands ssh started for the configuration end with it.
    let bounds = Bounds {
        stdin: stdin.map(str::as_bytes),
        own_group: true,
    };
    output_bounded(cmd, limit, bounds).map_err(|err| match err.kind() {
        std::io::ErrorKind::TimedOut => HostKeyError::NoAnswer {
            program: program.to_string(),
            secs: limit.as_secs(),
        },
        _ => HostKeyError::Tool {
            program: program.to_string(),
            detail: err.to_string(),
        },
    })
}

/// Asks ssh how it would reach `host` (`ssh -G`, which only reads the configuration).
pub fn resolve_target(host: &str, port: u16, user: &str) -> Result<SshTarget, HostKeyError> {
    resolve_target_within(host, port, user, TOOL_TIMEOUT)
}

/// [`resolve_target`] that stops ssh after `limit` (the user's configuration may run commands).
fn resolve_target_within(
    host: &str,
    port: u16,
    user: &str,
    limit: Duration,
) -> Result<SshTarget, HostKeyError> {
    let host = ssh_host(host);
    let mut args = ssh_config_args();
    args.extend([
        "-G".to_string(),
        "-p".to_string(),
        port.to_string(),
        "-l".to_string(),
        user.trim().to_string(),
        "--".to_string(),
        host.clone(),
    ]);
    let out = run_tool_within("ssh", &args, None, limit)?;
    if !out.status.success() {
        return Err(HostKeyError::Tool {
            program: "ssh -G".to_string(),
            detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let value = |key: &str| {
        text.lines()
            .filter_map(|line| line.split_once(' '))
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.to_string())
            .filter(|v| !v.is_empty())
    };
    // `none` turns a jump host off; as a name or alias it is a name.
    let jump = |key: &str| value(key).filter(|v| !v.eq_ignore_ascii_case("none"));
    if jump("proxyjump").is_some() || jump("proxycommand").is_some() {
        return Err(HostKeyError::JumpHost(host));
    }
    let target = SshTarget {
        hostname: value("hostname").unwrap_or(host),
        port: value("port").and_then(|p| p.parse().ok()).unwrap_or(port),
        host_key_alias: value("hostkeyalias"),
    };
    for name in std::iter::once(&target.hostname).chain(&target.host_key_alias) {
        if !is_plain_name(name) {
            return Err(HostKeyError::UnusableName(name.clone()));
        }
    }
    Ok(target)
}

/// Whether `name` stands for exactly one host in a `known_hosts` line: no space that splits
/// the line, no pattern (`*`, `?`, `!`), list (`,`) or hash (`|`) character, and no start that
/// makes the line a marker (`@`), a comment (`#`) or an option (`-`).
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['@', '#', '-'])
        && !name.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '*' | '?' | '!' | ',' | '|')
        })
}

/// The entries for `name` in `file` (`ssh-keygen -F`, which also finds hashed names). A missing
/// file has none.
fn lookup(name: &str, file: &Path) -> Result<Vec<KnownEntry>, HostKeyError> {
    if !file.is_file() {
        return Ok(Vec::new());
    }
    let out = run_tool(
        "ssh-keygen",
        &[
            "-F".to_string(),
            name.to_string(),
            "-f".to_string(),
            file.to_string_lossy().into_owned(),
        ],
        None,
    )?;
    match out.status.code() {
        Some(0) => Ok(parse_entries(&String::from_utf8_lossy(&out.stdout))),
        Some(1) => Ok(Vec::new()),
        _ => Err(HostKeyError::Tool {
            program: "ssh-keygen -F".to_string(),
            detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        }),
    }
}

fn parse_entries(text: &str) -> Vec<KnownEntry> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let first = fields.next()?;
            let revoked = match first {
                "@revoked" => true,
                // A certificate authority signs host keys; it is no key of this host.
                m if m.starts_with('@') => return None,
                _ => false,
            };
            if revoked {
                fields.next()?;
            }
            let key_type = fields.next()?;
            let key = fields.next()?;
            // ssh ignores a line whose key it cannot read, so it is no key of the host.
            is_key_blob(key_type, key).then(|| KnownEntry {
                revoked,
                key_type: key_type.to_string(),
                key: key.to_string(),
            })
        })
        .collect()
}

/// Whether `key` is a public key of `key_type` as ssh writes it: base64 of length-prefixed
/// fields that fill it exactly, at least the type and one more, the first naming the type.
fn is_key_blob(key_type: &str, key: &str) -> bool {
    let Some(blob) = decode_base64(key) else {
        return false;
    };
    let mut rest = blob.as_slice();
    let mut fields: Vec<&[u8]> = Vec::new();
    while let Some((len, tail)) = rest.split_first_chunk::<4>() {
        let Ok(len) = usize::try_from(u32::from_be_bytes(*len)) else {
            return false;
        };
        if tail.len() < len {
            return false;
        }
        let (field, tail) = tail.split_at(len);
        fields.push(field);
        rest = tail;
    }
    rest.is_empty() && fields.len() >= 2 && fields[0] == key_type.as_bytes()
}

/// Standard base64 with padding; `None` for anything else.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let body = text.trim_end_matches('=');
    if !text.len().is_multiple_of(4) || text.len() - body.len() > 2 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0_u32, 0_u32);
    for c in body.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from(acc >> bits).ok()?);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The keys of `types` that `target` shows (`ssh-keyscan`), in the order of [`SCAN_TYPES`], and
/// whether an SSH server answered at all (its banner). Like ssh, it tries the addresses of the
/// host name one after the other until one answers, within `timeout`.
fn scan(
    target: &SshTarget,
    types: &str,
    timeout: Duration,
) -> Result<(Vec<(String, String)>, bool), HostKeyError> {
    let started = std::time::Instant::now();
    let mut result = (Vec::new(), false);
    for address in scan_addresses(target, timeout, resolve) {
        let left = timeout.saturating_sub(started.elapsed());
        if left < Duration::from_secs(1) {
            break;
        }
        result = scan_address(&address, target.port, types, left)?;
        if result.1 || !result.0.is_empty() {
            break;
        }
    }
    Ok(result)
}

/// The addresses of a host name and port, as the system resolves them.
fn resolve(host: &str, port: u16) -> Vec<std::net::IpAddr> {
    use std::net::ToSocketAddrs;
    (host, port)
        .to_socket_addrs()
        .map_or_default(|addrs| addrs.map(|addr| addr.ip()).collect())
}

/// The addresses ssh would try for `target`: the IP address itself, else those the host name
/// resolves to within `limit` (the name itself when it does not resolve in time, so
/// `ssh-keyscan` reports that). A resolver that hangs keeps only its own thread.
fn scan_addresses(
    target: &SshTarget,
    limit: Duration,
    resolver: fn(&str, u16) -> Vec<std::net::IpAddr>,
) -> Vec<String> {
    if target.hostname.parse::<std::net::IpAddr>().is_ok() {
        return vec![target.hostname.clone()];
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let (host, port) = (target.hostname.clone(), target.port);
    std::thread::spawn(move || tx.send(resolver(&host, port)));
    let mut addresses: Vec<String> = Vec::new();
    for ip in rx.recv_timeout(limit).unwrap_or_default() {
        let ip = ip.to_string();
        if !addresses.contains(&ip) {
            addresses.push(ip);
        }
    }
    if addresses.is_empty() {
        addresses.push(target.hostname.clone());
    }
    addresses
}

/// [`scan`] of one address; `timeout` is cut to whole seconds, at least one.
fn scan_address(
    address: &str,
    port: u16,
    types: &str,
    timeout: Duration,
) -> Result<(Vec<(String, String)>, bool), HostKeyError> {
    let secs = timeout.min(SCAN_TIMEOUT).as_secs().max(1);
    // `-T` does not limit a server that stops after its banner.
    let out = run_tool_within(
        "ssh-keyscan",
        &[
            "-T".to_string(),
            secs.to_string(),
            "-p".to_string(),
            port.to_string(),
            "-t".to_string(),
            types.to_string(),
            "--".to_string(),
            address.to_string(),
        ],
        None,
        Duration::from_secs(secs + 1),
    )?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    // OpenSSH prints the banner comment on stdout, older versions on stderr.
    let answered = stdout
        .lines()
        .chain(String::from_utf8_lossy(&out.stderr).lines())
        .any(|line| line.starts_with('#') && line.contains(" SSH-"));
    let mut keys: Vec<(String, String)> = stdout
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split_whitespace().skip(1);
            Some((fields.next()?.to_string(), fields.next()?.to_string()))
        })
        .collect();
    keys.sort_by_key(|(key_type, _)| type_rank(key_type));
    Ok((keys, answered))
}

/// The `ssh-keyscan -t` list for a server with `entries`: only the types the files know, so a
/// known server gets one connection (servers count connections without login against the
/// address, `PerSourcePenalties`); all of [`SCAN_TYPES`] for an unknown one.
fn scan_types<'a>(entries: impl Iterator<Item = &'a KnownEntry>) -> String {
    let mut types: Vec<&str> = Vec::new();
    for entry in entries {
        let scan_type = match entry.key_type.as_str() {
            "ssh-ed25519" => "ed25519",
            t if t.starts_with("ecdsa-sha2-") => "ecdsa",
            "ssh-rsa" => "rsa",
            _ => return SCAN_TYPES.to_string(),
        };
        if !types.contains(&scan_type) {
            types.push(scan_type);
        }
    }
    if types.is_empty() {
        SCAN_TYPES.to_string()
    } else {
        types.join(",")
    }
}

fn type_rank(key_type: &str) -> u8 {
    if key_type == "ssh-ed25519" {
        0
    } else if key_type.starts_with("ecdsa-") {
        1
    } else {
        2
    }
}

/// `SHA256:…` of a key (`ssh-keygen -l -E sha256`).
fn fingerprint(key_type: &str, key: &str) -> Result<String, HostKeyError> {
    let out = run_tool(
        "ssh-keygen",
        &[
            "-l".to_string(),
            "-E".to_string(),
            "sha256".to_string(),
            "-f".to_string(),
            "-".to_string(),
        ],
        Some(&format!("{key_type} {key}\n")),
    )?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .find(|word| word.starts_with("SHA256:"))
        .map(ToOwned::to_owned)
        .ok_or_else(|| HostKeyError::Tool {
            program: "ssh-keygen -l".to_string(),
            detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
}

/// Compares the keys the server shows with the three `known_hosts` files: a file with another
/// key of the same type (or the shown key revoked) is a changed key, whatever the other files
/// say; else a file with a shown key makes it known.
pub fn check_host_key(
    host: &str,
    port: u16,
    user: &str,
    paths: &KnownHostsPaths,
    timeout: Duration,
) -> Result<HostKeyCheck, HostKeyError> {
    // Asking ssh and the files counts against the time of the scan.
    let started = std::time::Instant::now();
    let target = resolve_target_within(host, port, user, timeout)?;
    let name = target.known_hosts_name();
    let mut entries = Vec::new();
    for file in paths.all() {
        entries.extend(lookup(&name, file)?.into_iter().map(|e| (file, e)));
    }
    let types = scan_types(entries.iter().map(|(_, e)| e));
    let (mut scanned, answered) = scan(&target, &types, timeout.saturating_sub(started.elapsed()))?;
    // The server answered without a key of the known types: ask for all types in the time left,
    // to tell a server that now shows another type from one that did not finish its answer.
    let left = timeout.saturating_sub(started.elapsed());
    if scanned.is_empty() && answered && types != SCAN_TYPES && left >= Duration::from_secs(1) {
        scanned = scan(&target, SCAN_TYPES, left)?.0;
    }
    // A known server that shows keys, but none of a known type, has changed: a new type is not
    // offered for trust, as ssh with strict checking would not accept it either.
    let known_entry = entries.iter().find(|(_, e)| !e.revoked);
    if let Some((file, _)) = known_entry
        && !scanned.is_empty()
        && !scanned
            .iter()
            .any(|(t, _)| entries.iter().any(|(_, e)| !e.revoked && e.key_type == *t))
    {
        return Ok(HostKeyCheck::Changed(file.to_path_buf()));
    }
    let Some((first_type, first_key)) = scanned.first().cloned() else {
        return Ok(HostKeyCheck::NoKey(format!(
            "no SSH host key received from {name}"
        )));
    };
    let mut known = false;
    for (file, entry) in &entries {
        let shown = scanned.iter().find(|(t, _)| *t == entry.key_type);
        match shown {
            Some((_, key)) if entry.revoked == (*key == entry.key) => {
                return Ok(HostKeyCheck::Changed(file.to_path_buf()));
            }
            Some(_) if !entry.revoked => known = true,
            _ => {}
        }
    }
    if known {
        return Ok(HostKeyCheck::Known);
    }
    Ok(HostKeyCheck::Unknown(ScannedHostKey {
        fingerprint_sha256: fingerprint(&first_type, &first_key)?,
        name,
        host: target.hostname,
        port: target.port,
        key_type: first_type,
        key: first_key,
    }))
}

/// Adds `key` to Kestic's `known_hosts` file, as one unhashed line. Looks the name up in all
/// three files again first: a key of the same type written meanwhile is a changed key, and the
/// key itself already there needs no line.
pub fn trust(paths: &KnownHostsPaths, key: &ScannedHostKey) -> Result<HostKeyCheck, HostKeyError> {
    if !is_plain_name(&key.name) {
        return Err(HostKeyError::UnusableName(key.name.clone()));
    }
    let _guard = WRITE_LOCK.lock_unpoisoned();
    let mut known = false;
    for file in paths.all() {
        for entry in lookup(&key.name, file)? {
            if entry.key_type != key.key_type {
                continue;
            }
            // A revoked other key says nothing about this one; [`check_host_key`] reads it so too.
            if entry.revoked && entry.key != key.key {
                continue;
            }
            if entry.revoked || entry.key != key.key {
                return Ok(HostKeyCheck::Changed(file.to_path_buf()));
            }
            known = true;
        }
    }
    if known {
        return Ok(HostKeyCheck::Known);
    }
    let mut content = read_existing(&paths.kestic)?;
    if !content.is_empty() && !content.ends_with(b"\n") {
        content.push(b'\n');
    }
    content.extend_from_slice(format!("{} {} {}\n", key.name, key.key_type, key.key).as_bytes());
    write_atomically(&paths.kestic, &content)?;
    Ok(HostKeyCheck::Known)
}

/// Removes the lines of `name` from Kestic's `known_hosts` file; the other files stay as they
/// are. Returns whether a line was removed.
pub fn forget(paths: &KnownHostsPaths, name: &str) -> Result<bool, HostKeyError> {
    let _guard = WRITE_LOCK.lock_unpoisoned();
    // Lines are kept as bytes: a line that is no UTF-8 stays as it is.
    let content = read_existing(&paths.kestic)?;
    let lines: Vec<&[u8]> = content
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    let kept: Vec<&[u8]> = lines
        .iter()
        .copied()
        .filter(|line| !line_names(&String::from_utf8_lossy(line)).any(|n| n == name))
        .collect();
    if kept.len() == lines.len() {
        return Ok(false);
    }
    let mut next = Vec::new();
    for line in kept {
        next.extend_from_slice(line);
        next.push(b'\n');
    }
    write_atomically(&paths.kestic, &next)?;
    Ok(true)
}

/// The host names of a `known_hosts` line (none for comments and marker lines).
fn line_names(line: &str) -> impl Iterator<Item = &str> {
    let first = line.split_whitespace().next().unwrap_or_default();
    let names = if first.starts_with('#') || first.starts_with('@') {
        ""
    } else {
        first
    };
    names.split(',').filter(|n| !n.is_empty())
}

/// Why ssh cannot log in to the server, asked from ssh itself with Kestic's options: restic ends
/// as soon as the SFTP session fails, often before it has passed on the line of ssh that says
/// why. `None` when ssh names no host key or login failure.
pub fn login_failure(
    host: &str,
    port: u16,
    user: &str,
    paths: &KnownHostsPaths,
) -> Option<crate::sftp::SshFailure> {
    let mut args = ssh_config_args();
    args.extend([
        "-o".to_string(),
        format!(
            "UserKnownHostsFile=\"{}\" \"{}\"",
            paths.kestic.display(),
            paths.user.display()
        ),
    ]);
    for option in [
        "StrictHostKeyChecking=yes",
        "UpdateHostKeys=no",
        "BatchMode=yes",
        "ConnectTimeout=10",
    ] {
        args.extend(["-o".to_string(), option.to_string()]);
    }
    args.extend([
        "-p".to_string(),
        port.to_string(),
        "-l".to_string(),
        user.trim().to_string(),
        "-s".to_string(),
        "--".to_string(),
        ssh_host(host),
        "sftp".to_string(),
    ]);
    let out = run_tool("ssh", &args, None).ok()?;
    let stderr = String::from_utf8_lossy(&out.stderr)
        .lines()
        .map(|line| format!("subprocess ssh: {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    crate::sftp::ssh_failure(&stderr)
}

/// The name of the `known_hosts` lines of an SFTP destination, as ssh resolves its host; the
/// plain host and port when ssh cannot tell.
pub fn known_hosts_name(host: &str, port: u16, user: &str) -> String {
    resolve_target(host, port, user)
        .unwrap_or_else(|_| SshTarget {
            hostname: ssh_host(host),
            port,
            host_key_alias: None,
        })
        .known_hosts_name()
}

fn read_existing(path: &Path) -> Result<Vec<u8>, HostKeyError> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err.into()),
    }
}

/// Replaces `path` with `content`: a new file only the owner can read (`0600`) in the same
/// folder, synced, then renamed over the old one, and the folder synced.
fn write_atomically(path: &Path, content: &[u8]) -> Result<(), HostKeyError> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("known_hosts path has no folder"))?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let nonce =
        crate::model::RunId::generate().map_err(|e| std::io::Error::other(e.to_string()))?;
    let temp = dir.join(format!(".known_hosts.{nonce}.tmp"));
    let written = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(content)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        fs::File::open(dir)?.sync_all()
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    Ok(written?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hosts_names_follow_ssh() {
        let target = |port, alias: Option<&str>| SshTarget {
            hostname: "nas.local".to_string(),
            port,
            host_key_alias: alias.map(ToOwned::to_owned),
        };
        assert_eq!(target(22, None).known_hosts_name(), "nas.local");
        assert_eq!(target(2222, None).known_hosts_name(), "[nas.local]:2222");
        assert_eq!(target(2222, Some("mynas")).known_hosts_name(), "mynas");
    }

    #[test]
    fn entries_of_ssh_keygen_find_output_are_parsed() {
        let out = format!(
            "# Host [h]:2222 found: line 1\n\
             [h]:2222 ssh-ed25519 {KEY_A}\n\
             |1|abc=|def= ecdsa-sha2-nistp256 {KEY_ECDSA} comment\n\
             @cert-authority * ssh-ed25519 {KEY_A}\n\
             @revoked [h]:2222 ssh-ed25519 {KEY_B}\n"
        );
        let entries = parse_entries(&out);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].key_type, "ssh-ed25519");
        assert_eq!(entries[1].key, KEY_ECDSA);
        assert!(entries[2].revoked);
        assert_eq!(entries[2].key, KEY_B);
    }

    /// M5-S01 (AC-M5-01-7): a line whose key ssh cannot read (cut off, no base64, the blob of
    /// another type, nothing after the type) is no entry, as for ssh.
    #[test]
    fn m5_01_lines_with_an_unreadable_key_are_no_entries() {
        let cut = &KEY_A[..KEY_A.len() - 8];
        let type_only = "AAAAC3NzaC1lZDI1NTE5";
        for (case, line) in [
            ("cut off", format!("h ssh-ed25519 {cut}")),
            ("no base64", "h ssh-ed25519 garbage!".to_string()),
            ("other type", format!("h ssh-rsa {KEY_A}")),
            ("type only", format!("h ssh-ed25519 {type_only}")),
            ("revoked, cut off", format!("@revoked h ssh-ed25519 {cut}")),
        ] {
            assert!(parse_entries(&line).is_empty(), "{case}: {line}");
        }
    }

    /// M5-S01 (AC-M5-01-7): a line for the host whose key ssh cannot read does not count as a
    /// changed key; the key the server shows is trusted.
    #[test]
    fn m5_01_an_unreadable_key_line_is_no_changed_key() {
        let cut = &KEY_A[..KEY_A.len() - 8];
        for line in [
            format!("h.example ssh-ed25519 {cut}\n"),
            "h.example ssh-ed25519 garbage!\n".to_string(),
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let paths = temp_paths(dir.path());
            fs::write(&paths.user, &line).expect("write user known_hosts");
            assert_eq!(
                trust(&paths, &scanned("h.example", KEY_B)).expect("trust"),
                HostKeyCheck::Known,
                "{line}"
            );
            assert_eq!(
                fs::read_to_string(&paths.kestic).expect("read"),
                format!("h.example ssh-ed25519 {KEY_B}\n"),
                "the shown key is written"
            );
        }
    }

    #[test]
    fn a_line_that_is_no_utf8_neither_blocks_nor_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = KnownHostsPaths {
            kestic: dir.path().join("kestic").join("known_hosts"),
            user: dir.path().join("known_hosts.user"),
            system: dir.path().join("known_hosts.system"),
        };
        fs::create_dir_all(dir.path().join("kestic")).expect("mkdir");
        let keep = b"keep.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKeepKeepKeepKeepKeepKeepKeepKeepKeepKeepKe\n# caf\xe9\n";
        let gone = b"gone.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGoneGoneGoneGoneGoneGoneGoneGoneGoneGoneGo\n";
        fs::write(&paths.kestic, [&keep[..], &gone[..]].concat()).expect("write");
        assert!(
            forget(&paths, "gone.example").expect("forget"),
            "line removed"
        );
        assert_eq!(fs::read(&paths.kestic).expect("read"), keep.to_vec());
        let key = ScannedHostKey {
            name: "new.example".to_string(),
            host: "new.example".to_string(),
            port: 22,
            key_type: "ssh-ed25519".to_string(),
            key: "AAAAC3NzaC1lZDI1NTE5AAAAINewNewNewNewNewNewNewNewNewNewNewNewNewNewNe"
                .to_string(),
            fingerprint_sha256: String::new(),
        };
        assert_eq!(trust(&paths, &key).expect("trust"), HostKeyCheck::Known);
        let written = fs::read(&paths.kestic).expect("read");
        assert!(
            written.starts_with(keep),
            "the other lines keep their bytes"
        );
        assert!(written.ends_with(b"new.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINewNewNewNewNewNewNewNewNewNewNewNewNewNewNe\n"));
    }

    const KEY_A: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIDs3azM0A+L/eWx8w8fjCWEg9C28yO26W7PfAf94XvHq";
    const KEY_B: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIKJIQrwuBBjZZ5pj4R9SRKB6ie3rmpih689TdvvKAeGQ";
    const KEY_ECDSA: &str = "AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBHhEg4IZO4e7gCyzonrFu4UUPaxndtB0O5MABz3p9IwkTKD/kEBm7NH52lzDAh36YJlhaUXUPigG6NGlr619FKQ=";

    fn temp_paths(dir: &Path) -> KnownHostsPaths {
        fs::create_dir_all(dir.join("kestic")).expect("mkdir");
        KnownHostsPaths {
            kestic: dir.join("kestic").join("known_hosts"),
            user: dir.join("known_hosts.user"),
            system: dir.join("known_hosts.system"),
        }
    }

    fn scanned(name: &str, key: &str) -> ScannedHostKey {
        ScannedHostKey {
            name: name.to_string(),
            host: "h.example".to_string(),
            port: 22,
            key_type: "ssh-ed25519".to_string(),
            key: key.to_string(),
            fingerprint_sha256: String::new(),
        }
    }

    /// M5-S01 (AC-M5-01-4): a name from `ssh -G` that would be a pattern, a marker or a broken
    /// line in `known_hosts` is refused before any key is scanned or written; a plain alias and
    /// an IPv6 address pass.
    #[test]
    fn m5_01_names_from_the_ssh_configuration_must_be_plain() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("ssh_config");
        fs::write(
            &config,
            "Host star\n  HostKeyAlias *\n\
             Host question\n  HostKeyAlias nas?\n\
             Host negated\n  HostKeyAlias !nas\n\
             Host list\n  HostKeyAlias a,b\n\
             Host hashed\n  HostKeyAlias |1|abc\n\
             Host spaced\n  HostKeyAlias \"a b\"\n\
             Host marker\n  HostKeyAlias @revoked\n\
             Host dash\n  HostName -oProxyCommand=x\n\
             Host pipe\n  HostName a|b\n\
             Host alias\n  HostName 127.0.0.1\n  HostKeyAlias mynas\n\
             Host six\n  HostName ::1\n\
             Host literal\n  HostName 127.0.0.1\n  HostKeyAlias none\n",
        )
        .expect("write ssh_config");
        crate::sftp::set_test_ssh_config(Some(config));
        for host in [
            "star", "question", "negated", "list", "hashed", "spaced", "marker", "dash", "pipe",
        ] {
            let resolved = resolve_target(host, 22, "anna");
            assert!(
                matches!(resolved, Err(HostKeyError::UnusableName(_))),
                "{host}: {resolved:?}"
            );
        }
        let alias = resolve_target("alias", 22, "anna").expect("plain alias");
        assert_eq!(alias.known_hosts_name(), "mynas");
        let six = resolve_target("six", 2222, "anna").expect("IPv6 address");
        assert_eq!(six.known_hosts_name(), "[::1]:2222");
        // ssh takes the alias `none` literally; only a jump host is turned off by `none`.
        let literal = resolve_target("literal", 22, "anna").expect("alias none");
        assert_eq!(literal.known_hosts_name(), "none");
        crate::sftp::set_test_ssh_config(None);

        // Trust writes nothing for such a name either.
        let paths = temp_paths(dir.path());
        let refused = trust(&paths, &scanned("*", KEY_A));
        assert!(
            matches!(refused, Err(HostKeyError::UnusableName(_))),
            "{refused:?}"
        );
        assert!(!paths.kestic.exists(), "nothing written");
    }

    /// M5-S01 (AC-M5-01-4, AC-M5-01-7): a revoked entry for another key of the server does not
    /// stop trusting the key it shows, as in [`check_host_key`]; the revoked key itself stays a
    /// changed key.
    #[test]
    fn m5_01_trust_reads_a_revoked_entry_like_the_check() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = temp_paths(dir.path());
        let revoked = format!("@revoked h.example ssh-ed25519 {KEY_A}\n");
        fs::write(&paths.user, &revoked).expect("write user known_hosts");

        assert_eq!(
            trust(&paths, &scanned("h.example", KEY_A)).expect("trust revoked"),
            HostKeyCheck::Changed(paths.user.clone())
        );
        assert!(!paths.kestic.exists(), "the revoked key is not written");
        assert_eq!(
            trust(&paths, &scanned("h.example", KEY_B)).expect("trust"),
            HostKeyCheck::Known
        );
        assert_eq!(
            fs::read_to_string(&paths.kestic).expect("read"),
            format!("h.example ssh-ed25519 {KEY_B}\n"),
            "the shown key is written"
        );
        assert_eq!(
            fs::read_to_string(&paths.user).expect("read user"),
            revoked,
            "the user's file stays as it is"
        );
    }

    /// M5-S01 (AC-M5-01-5 (c)): resolving the host name counts against the time of the key scan;
    /// a resolver that does not answer leaves the name itself, at once after the limit.
    #[test]
    fn m5_01_name_resolution_stays_within_the_time_limit() {
        fn hanging(_: &str, _: u16) -> Vec<std::net::IpAddr> {
            std::thread::sleep(Duration::from_secs(30));
            Vec::new()
        }
        fn twice(_: &str, _: u16) -> Vec<std::net::IpAddr> {
            let ip = std::net::IpAddr::from([192, 0, 2, 7]);
            vec![ip, ip, std::net::IpAddr::from([192, 0, 2, 8])]
        }
        let target = SshTarget {
            hostname: "nas.example".to_string(),
            port: 22,
            host_key_alias: None,
        };
        let started = std::time::Instant::now();
        let addresses = scan_addresses(&target, Duration::from_millis(300), hanging);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "waited {:?}",
            started.elapsed()
        );
        assert_eq!(addresses, vec!["nas.example".to_string()]);
        assert_eq!(
            scan_addresses(&target, Duration::from_secs(5), twice),
            vec!["192.0.2.7".to_string(), "192.0.2.8".to_string()]
        );
    }

    /// M5-S01 (AC-M5-01-5 (c)): a command of the user's configuration that ssh started in the
    /// background and that holds ssh's output open does not hold up the check either.
    #[test]
    fn m5_01_a_background_command_of_the_configuration_does_not_hold_up_the_check() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("ssh_config");
        let marker = format!("kestic-match-bg-{}", std::process::id());
        fs::write(
            &config,
            format!("Match exec \"(sleep 30; : {marker}) >&2 & true\"\n  HostKeyAlias slow\n"),
        )
        .expect("write");
        crate::sftp::set_test_ssh_config(Some(config));
        let paths = temp_paths(dir.path());
        let started = std::time::Instant::now();
        let checked = check_host_key("nas.example", 22, "anna", &paths, Duration::from_secs(1));
        let elapsed = started.elapsed();
        crate::sftp::set_test_ssh_config(None);
        assert!(
            matches!(checked, Err(HostKeyError::NoAnswer { .. })),
            "{checked:?}"
        );
        assert!(elapsed < Duration::from_secs(3), "waited {elapsed:?}");
    }

    /// M5-S01 (AC-M5-01-5 (c)): `ssh -G`, which may run commands of the user's configuration
    /// (`Match exec`), counts against the time of the host key check.
    #[test]
    fn m5_01_asking_ssh_counts_against_the_time_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("ssh_config");
        let marker = format!("kestic-match-exec-{}", std::process::id());
        fs::write(
            &config,
            format!("Match exec \"sleep 5; : {marker}\"\n  HostKeyAlias slow\n"),
        )
        .expect("write");
        crate::sftp::set_test_ssh_config(Some(config));
        let paths = temp_paths(dir.path());
        let started = std::time::Instant::now();
        let checked = check_host_key("nas.example", 22, "anna", &paths, Duration::from_secs(1));
        let elapsed = started.elapsed();
        crate::sftp::set_test_ssh_config(None);
        assert!(
            matches!(checked, Err(HostKeyError::NoAnswer { .. })),
            "{checked:?}"
        );
        assert!(elapsed < Duration::from_secs(3), "waited {elapsed:?}");
        // The command of the configuration ended with ssh.
        std::thread::sleep(Duration::from_millis(300));
        let left: Vec<String> = fs::read_dir("/proc")
            .expect("/proc")
            .filter_map(|entry| fs::read(entry.ok()?.path().join("cmdline")).ok())
            .map(|cmdline| String::from_utf8_lossy(&cmdline).replace('\0', " "))
            .filter(|cmdline| cmdline.contains(&marker))
            .collect();
        assert!(left.is_empty(), "left running: {left:?}");
    }

    #[test]
    fn line_names_split_patterns_and_skip_markers() {
        assert_eq!(
            line_names("a,[b]:22 ssh-ed25519 K").collect::<Vec<_>>(),
            vec!["a", "[b]:22"]
        );
        assert_eq!(line_names("@revoked a ssh-rsa K").count(), 0);
        assert_eq!(line_names("# a").count(), 0);
    }
}
