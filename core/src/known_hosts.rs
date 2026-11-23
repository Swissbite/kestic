// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Host keys of SFTP servers: which name ssh looks the server up by (`ssh -G`), what the three
//! `known_hosts` files say about it (`ssh-keygen -F`), which keys the server shows
//! (`ssh-keyscan`), and Kestic's own `known_hosts` file, the only one Kestic writes.

use crate::sftp::{DEFAULT_PORT, KnownHostsPaths, ssh_config_args, ssh_host};
use crate::sync::LockUnpoisoned;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
    #[error(transparent)]
    Paths(#[from] crate::sftp::KnownHostsPathError),
    #[error("{program} failed: {detail}")]
    Tool { program: String, detail: String },
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
    let tool_error = |detail: String| HostKeyError::Tool {
        program: program.to_string(),
        detail,
    };
    let mut child = Command::new(program)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| tool_error(err.to_string()))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes())?;
    }
    let read = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        })
    };
    let stdout = read(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = read(child.stderr.take().map(|p| Box::new(p) as _));
    // ssh reads the user's configuration (`Match exec`) and resolves names; none of that may hold
    // up a connection test or the check before a run.
    let deadline = std::time::Instant::now() + TOOL_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(tool_error(format!(
                "no answer within {} s",
                TOOL_TIMEOUT.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(std::process::Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Asks ssh how it would reach `host` (`ssh -G`, which only reads the configuration).
pub fn resolve_target(host: &str, port: u16, user: &str) -> Result<SshTarget, HostKeyError> {
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
    let out = run_tool("ssh", &args, None)?;
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
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("none"))
    };
    if value("proxyjump").is_some() || value("proxycommand").is_some() {
        return Err(HostKeyError::JumpHost(host));
    }
    Ok(SshTarget {
        hostname: value("hostname").unwrap_or(host),
        port: value("port").and_then(|p| p.parse().ok()).unwrap_or(port),
        host_key_alias: value("hostkeyalias"),
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
            Some(KnownEntry {
                revoked,
                key_type: fields.next()?.to_string(),
                key: fields.next()?.to_string(),
            })
        })
        .collect()
}

/// The keys of `types` that `target` shows (`ssh-keyscan`), in the order of [`SCAN_TYPES`], and
/// whether an SSH server answered at all (its banner); `timeout` is cut to whole seconds, at
/// least one.
fn scan(
    target: &SshTarget,
    types: &str,
    timeout: Duration,
) -> Result<(Vec<(String, String)>, bool), HostKeyError> {
    let secs = timeout.min(SCAN_TIMEOUT).as_secs().max(1);
    let out = run_tool(
        "ssh-keyscan",
        &[
            "-T".to_string(),
            secs.to_string(),
            "-p".to_string(),
            target.port.to_string(),
            "-t".to_string(),
            types.to_string(),
            "--".to_string(),
            target.hostname.clone(),
        ],
        None,
    )?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let answered = stdout
        .lines()
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
    let target = resolve_target(host, port, user)?;
    let name = target.known_hosts_name();
    let mut entries = Vec::new();
    for file in paths.all() {
        entries.extend(lookup(&name, file)?.into_iter().map(|e| (file, e)));
    }
    let types = scan_types(entries.iter().map(|(_, e)| e));
    let (mut scanned, answered) = scan(&target, &types, timeout)?;
    // The server answers but no longer offers the known types: its other keys are new to every
    // file.
    if scanned.is_empty() && answered && types != SCAN_TYPES {
        scanned = scan(&target, SCAN_TYPES, timeout)?.0;
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
    let _guard = WRITE_LOCK.lock_unpoisoned();
    let mut known = false;
    for file in paths.all() {
        for entry in lookup(&key.name, file)? {
            if entry.key_type != key.key_type {
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
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&format!("{} {} {}\n", key.name, key.key_type, key.key));
    write_atomically(&paths.kestic, &content)?;
    Ok(HostKeyCheck::Known)
}

/// Removes the lines of `name` from Kestic's `known_hosts` file; the other files stay as they
/// are. Returns whether a line was removed.
pub fn forget(paths: &KnownHostsPaths, name: &str) -> Result<bool, HostKeyError> {
    let _guard = WRITE_LOCK.lock_unpoisoned();
    let content = read_existing(&paths.kestic)?;
    let kept: Vec<&str> = content
        .lines()
        .filter(|line| !line_names(line).any(|n| n == name))
        .collect();
    if kept.len() == content.lines().count() {
        return Ok(false);
    }
    let mut next = kept.join("\n");
    if !next.is_empty() {
        next.push('\n');
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

fn read_existing(path: &Path) -> Result<String, HostKeyError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err.into()),
    }
}

/// Replaces `path` with `content`: a new file only the owner can read (`0600`) in the same
/// folder, synced, then renamed over the old one, and the folder synced.
fn write_atomically(path: &Path, content: &str) -> Result<(), HostKeyError> {
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
        file.write_all(content.as_bytes())?;
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
        let out = "# Host [h]:2222 found: line 1\n\
                   [h]:2222 ssh-ed25519 AAAA1\n\
                   |1|abc=|def= ecdsa-sha2-nistp256 AAAA2 comment\n\
                   @cert-authority * ssh-rsa AAAA3\n\
                   @revoked [h]:2222 ssh-rsa AAAA4\n";
        let entries = parse_entries(out);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].key_type, "ssh-ed25519");
        assert_eq!(entries[1].key, "AAAA2");
        assert!(entries[2].revoked);
        assert_eq!(entries[2].key, "AAAA4");
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
