// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! SFTP destinations: field checks, the restic repository address
//! `sftp://<user>@<host>:<port>/<path>`, the `known_hosts` files ssh reads and the ssh options
//! Kestic hands to restic in `sftp.args`.

use crate::model::ModelError;
use std::net::Ipv6Addr;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Port of an SFTP destination when none is given.
pub const DEFAULT_PORT: u16 = 22;

/// Seconds ssh waits for the TCP connection (`ConnectTimeout`).
pub const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;

/// Checks the fields of an SFTP destination: a host name or IP address, a port, a user name and
/// a path. A host or user starting with `-` would reach ssh as an option.
pub fn validate(host: &str, port: u16, user: &str, path: &str) -> Result<(), ModelError> {
    let invalid = |msg: String| Err(ModelError::InvalidDestination(msg));
    if !is_valid_host(host) {
        return invalid(format!(
            "invalid SFTP host '{}': a host name (letters, digits, dots, hyphens) or an IP address",
            host.escape_default()
        ));
    }
    if port == 0 {
        return invalid("SFTP port must be between 1 and 65535".to_string());
    }
    let user = user.trim();
    if user.is_empty()
        || user.starts_with('-')
        || user.chars().any(|c| {
            c.is_control() || c.is_whitespace() || matches!(c, '@' | ':' | '/' | '\\' | '\'' | '"')
        })
    {
        return invalid(format!(
            "invalid SFTP user '{}': not empty, no spaces, '@', ':', '/', quotes or a leading '-'",
            user.escape_default()
        ));
    }
    let path = path.trim();
    if path.is_empty() {
        return invalid("SFTP path must not be empty".to_string());
    }
    if path.chars().any(char::is_control) {
        return invalid("SFTP path must not contain control characters".to_string());
    }
    // restic refuses a path starting with `~`: it would not be expanded on the server.
    if path.starts_with('~') {
        return invalid(
            "SFTP path must not start with '~'; enter it relative to the home folder instead"
                .to_string(),
        );
    }
    Ok(())
}

fn is_valid_host(host: &str) -> bool {
    let host = host.trim();
    if bare_ipv6(host).is_some() {
        return true;
    }
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['-', '.'])
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// The IPv6 address of `host` (`::1` or `[::1]`), if it is one.
fn bare_ipv6(host: &str) -> Option<Ipv6Addr> {
    let inner = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    inner.parse().ok()
}

/// The host as ssh gets it: trimmed, an IPv6 address without brackets.
pub fn ssh_host(host: &str) -> String {
    let host = host.trim();
    bare_ipv6(host).map_or_else(|| host.to_string(), |ip| ip.to_string())
}

/// The host inside a URL: an IPv6 address in brackets.
fn url_host(host: &str) -> String {
    let host = host.trim();
    bare_ipv6(host).map_or_else(|| host.to_string(), |ip| format!("[{ip}]"))
}

/// restic repository address `sftp://<user>@<host>:<port>/<path>`. A path relative to the home
/// folder follows one slash after the port, an absolute path two (`sftp://u@h:22//srv/repo`).
/// restic decodes the address as URL, so characters with a meaning there are percent-encoded.
pub fn repository_uri(host: &str, port: u16, user: &str, path: &str) -> String {
    format!(
        "sftp://{}@{}:{port}/{}",
        percent_encode(user.trim()),
        url_host(host),
        percent_encode(path.trim())
    )
}

/// Comparison key of the repository location: user, host in lowercase, port and path without a
/// trailing slash.
pub fn location_key(host: &str, port: u16, user: &str, path: &str) -> String {
    let path = path.trim();
    let path = if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    };
    format!(
        "sftp:{}@{}:{port}/{path}",
        user.trim(),
        ssh_host(host).to_ascii_lowercase()
    )
}

/// `<user>@<host>:<path>` as shown to the user, with the port when it is not 22.
pub fn display_location(host: &str, port: u16, user: &str, path: &str) -> String {
    let host = url_host(host);
    let host = if port == DEFAULT_PORT {
        host
    } else {
        format!("{host}:{port}")
    };
    format!("{}@{host}:{}", user.trim(), path.trim())
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/!$&'()*+,;=:@".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The `known_hosts` files ssh reads for Kestic: Kestic's own file (the only one Kestic writes),
/// the user's file and the system file. Built from `$XDG_DATA_HOME` and `$HOME`, never from `~`,
/// which OpenSSH resolves through the passwd entry instead of `$HOME`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownHostsPaths {
    pub kestic: PathBuf,
    pub user: PathBuf,
    pub system: PathBuf,
}

/// Why the `known_hosts` paths cannot be handed to ssh.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KnownHostsPathError {
    #[error("HOME is not set")]
    NoHome,
    /// ssh reads the path list as one quoted option line; a quote, backslash or control
    /// character in a path would change it.
    #[error(
        "the path '{0}' contains a quote, backslash or control character, which ssh options cannot carry"
    )]
    Unquotable(String),
}

impl KnownHostsPaths {
    /// Paths from the environment of this process.
    pub fn from_env() -> Result<Self, KnownHostsPathError> {
        let home = std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .ok_or(KnownHostsPathError::NoHome)?;
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map_or_else(|| home.join(".local").join("share"), PathBuf::from);
        let paths = Self {
            kestic: data.join("kestic").join("known_hosts"),
            user: home.join(".ssh").join("known_hosts"),
            system: PathBuf::from("/etc/ssh/ssh_known_hosts"),
        };
        for path in [&paths.kestic, &paths.user] {
            quotable(path)?;
        }
        Ok(paths)
    }

    /// The three files in the order Kestic looks a server up.
    pub fn all(&self) -> [&Path; 3] {
        [&self.kestic, &self.user, &self.system]
    }
}

fn quotable(path: &Path) -> Result<&str, KnownHostsPathError> {
    let text = path.to_str().unwrap_or_default();
    if text.is_empty()
        || text
            .chars()
            .any(|c| c.is_control() || matches!(c, '\'' | '"' | '\\'))
    {
        return Err(KnownHostsPathError::Unquotable(
            path.to_string_lossy().into_owned(),
        ));
    }
    Ok(text)
}

static TEST_SSH_CONFIG: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Makes every ssh call of Kestic read `config` (`-F`) and offer only its identity files
/// (`IdentitiesOnly=yes`) instead of the user's `~/.ssh/config`. Only test builds call this.
pub fn set_test_ssh_config(config: Option<PathBuf>) {
    if let Ok(mut slot) = TEST_SSH_CONFIG.write() {
        *slot = config;
    }
}

/// The ssh configuration file set by [`set_test_ssh_config`].
pub fn test_ssh_config() -> Option<PathBuf> {
    TEST_SSH_CONFIG.read().ok().and_then(|slot| slot.clone())
}

/// ssh options Kestic adds in front of the destination's own: the test configuration, if any.
pub fn ssh_config_args() -> Vec<String> {
    match test_ssh_config() {
        Some(config) => vec![
            "-F".to_string(),
            config.to_string_lossy().into_owned(),
            "-o".to_string(),
            "IdentitiesOnly=yes".to_string(),
        ],
        None => Vec::new(),
    }
}

/// The value of restic's `sftp.args`: both `known_hosts` paths explicitly, strict host key
/// checking without any write by ssh, no prompts, and a connect time limit. restic splits it like
/// a shell (single quotes keep the spaces), ssh reads the double-quoted paths of
/// `UserKnownHostsFile` as a list.
pub fn sftp_args(paths: &KnownHostsPaths) -> Result<String, KnownHostsPathError> {
    let kestic = quotable(&paths.kestic)?;
    let user = quotable(&paths.user)?;
    let mut parts = Vec::new();
    if let Some(config) = test_ssh_config() {
        let config = quotable(&config)?;
        parts.push(format!("-F '{config}' -o IdentitiesOnly=yes"));
    }
    parts.push(format!(
        "-o 'UserKnownHostsFile=\"{kestic}\" \"{user}\"' -o StrictHostKeyChecking=yes \
         -o UpdateHostKeys=no -o BatchMode=yes -o ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}"
    ));
    Ok(parts.join(" "))
}

/// restic arguments that set `sftp.args` for the `known_hosts` files of this process (see
/// [`restic_options_for`]). Without usable paths ssh gets `/dev/null` as its only `known_hosts`
/// file and refuses every server, never asks and never writes; the check before each access
/// reports the actual cause.
pub fn restic_options() -> Vec<String> {
    KnownHostsPaths::from_env()
        .and_then(|paths| restic_options_for(&paths))
        .unwrap_or_else(|_| {
            option_args(
                "-o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=yes -o BatchMode=yes",
            )
        })
}

/// restic arguments that set `sftp.args` to [`sftp_args`] of `paths`.
pub fn restic_options_for(paths: &KnownHostsPaths) -> Result<Vec<String>, KnownHostsPathError> {
    Ok(option_args(&sftp_args(paths)?))
}

/// `-o sftp.args=<args>`: restic reads the value of `-o` as one CSV field, so it is quoted with
/// doubled inner quotes.
fn option_args(args: &str) -> Vec<String> {
    let option = format!("sftp.args={args}");
    vec![
        "-o".to_string(),
        format!("\"{}\"", option.replace('"', "\"\"")),
    ]
}

/// What ssh reported on its way to the server, from restic's `subprocess ssh:` lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshFailure {
    /// The server rejected every key (`Permission denied (publickey…)`).
    KeyRejected(String),
    /// ssh refused the server's host key (unknown or changed).
    HostKeyVerificationFailed(String),
}

impl SshFailure {
    /// The line of ssh.
    pub fn line(&self) -> &str {
        match self {
            Self::KeyRejected(line) | Self::HostKeyVerificationFailed(line) => line,
        }
    }

    /// Whether the server rejected the SSH key.
    pub fn is_key_rejected(&self) -> bool {
        matches!(self, Self::KeyRejected(_))
    }
}

/// The ssh failure in restic's error output, if any: the line of ssh that names it.
pub fn ssh_failure(stderr: &str) -> Option<SshFailure> {
    let lines = || {
        stderr
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("subprocess ssh:"))
    };
    if let Some(line) = lines().find(|l| l.contains("Host key verification failed")) {
        return Some(SshFailure::HostKeyVerificationFailed(line.to_string()));
    }
    lines()
        .find(|l| l.contains("Permission denied ("))
        .map(|line| SshFailure::KeyRejected(line.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_and_relative_paths_become_the_sftp_url_restic_reads() {
        assert_eq!(
            repository_uri("nas.local", 22, "anna", "/srv/backup"),
            "sftp://anna@nas.local:22//srv/backup"
        );
        assert_eq!(
            repository_uri("nas.local", 2222, "anna", "backup/laptop"),
            "sftp://anna@nas.local:2222/backup/laptop"
        );
        assert_eq!(
            repository_uri("::1", 22, "anna", "/srv/Backup Daten/#1"),
            "sftp://anna@[::1]:22//srv/Backup%20Daten/%231"
        );
    }

    #[test]
    fn options_and_odd_fields_are_refused() {
        assert!(validate("nas.local", 22, "anna", "/srv").is_ok());
        assert!(validate("[fe80::1]", 22, "anna", "backup").is_ok());
        for (host, port, user, path) in [
            ("-oProxyCommand=x", 22, "anna", "/srv"),
            ("nas local", 22, "anna", "/srv"),
            ("nas.local", 0, "anna", "/srv"),
            ("nas.local", 22, "-oProxyCommand=x", "/srv"),
            ("nas.local", 22, "an na", "/srv"),
            ("nas.local", 22, "anna@x", "/srv"),
            ("nas.local", 22, "anna", ""),
            ("nas.local", 22, "anna", "~/backup"),
            ("nas.local", 22, "anna", "/srv/\nx"),
        ] {
            assert!(
                validate(host, port, user, path).is_err(),
                "accepted {host:?} {port} {user:?} {path:?}"
            );
        }
    }

    #[test]
    fn sftp_args_quote_paths_with_spaces_for_restic_and_ssh() {
        let paths = KnownHostsPaths {
            kestic: PathBuf::from("/home/a b/.local/share/kestic/known_hosts"),
            user: PathBuf::from("/home/a b/.ssh/known_hosts"),
            system: PathBuf::from("/etc/ssh/ssh_known_hosts"),
        };
        let args = sftp_args(&paths).expect("quotable");
        assert!(args.contains(
            "-o 'UserKnownHostsFile=\"/home/a b/.local/share/kestic/known_hosts\" \"/home/a b/.ssh/known_hosts\"'"
        ));
        assert!(args.contains("-o StrictHostKeyChecking=yes"));
        assert!(args.contains("-o BatchMode=yes"));
        assert!(args.contains("-o ConnectTimeout=10"));
        let quoted = KnownHostsPaths {
            user: PathBuf::from("/home/o'brien/.ssh/known_hosts"),
            ..paths
        };
        assert!(sftp_args(&quoted).is_err());
    }

    #[test]
    fn ssh_lines_in_restic_output_name_the_failure() {
        let rejected = "subprocess ssh: anna@127.0.0.1: Permission denied (publickey).\n\
                        Fatal: unable to open repository: unexpected EOF";
        assert!(matches!(
            ssh_failure(rejected),
            Some(SshFailure::KeyRejected(_))
        ));
        let host_key = "subprocess ssh: No ED25519 host key is known for [127.0.0.1]:2222\n\
                        subprocess ssh: Host key verification failed.";
        assert!(matches!(
            ssh_failure(host_key),
            Some(SshFailure::HostKeyVerificationFailed(_))
        ));
        assert_eq!(
            ssh_failure("Fatal: Permission denied (publickey) in a path"),
            None
        );
    }
}
