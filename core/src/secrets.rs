// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Secret storage abstraction and in-memory / file fallback backends (FR-08, NFR-02, M0-S08).

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Wrapper for secret UTF-8/byte values whose `Debug` output is always redacted
/// and whose buffer is overwritten with zeros on drop (NFR-02).
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(Vec<u8>);

impl SecretValue {
    /// Creates a [`SecretValue`] from a UTF-8 string.
    pub fn from_str_secret(secret: impl Into<String>) -> Self {
        Self(secret.into().into_bytes())
    }

    /// Creates a [`SecretValue`] from raw bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Returns the raw secret bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the secret as UTF-8 if valid.
    pub fn as_str(&self) -> Result<&str, SecretError> {
        std::str::from_utf8(&self.0).map_err(|_| SecretError::InvalidUtf8)
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}

/// Errors produced by [`SecretStore`] backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// Collection or item requires interactive unlocking (e.g. KWallet without PAM).
    LockedOrPromptRequired(String),
    /// Secret Service D-Bus daemon or file path unavailable.
    BackendUnavailable(String),
    /// Stored bytes were not valid UTF-8.
    InvalidUtf8,
    /// Underlying I/O or serialization failure.
    Io(String),
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LockedOrPromptRequired(msg) => write!(f, "secret store locked: {msg}"),
            Self::BackendUnavailable(msg) => write!(f, "secret backend unavailable: {msg}"),
            Self::InvalidUtf8 => write!(f, "secret is not valid UTF-8"),
            Self::Io(msg) => write!(f, "secret store I/O error: {msg}"),
        }
    }
}

impl std::error::Error for SecretError {}

/// Password store abstraction implemented by Secret Service (`org.freedesktop.secrets`),
/// [`InMemorySecretStore`], and [`FileSecretStore`] (ADR-0004, AC-M0-08-1, AC-M0-08-2).
pub trait SecretStore {
    fn set_secret(
        &self,
        target_id: &str,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError>;
    fn get_secret(&self, target_id: &str, kind: &str) -> Result<Option<SecretValue>, SecretError>;
    fn delete_secret(&self, target_id: &str, kind: &str) -> Result<(), SecretError>;

    /// Non-interactive probe checking whether the secret store is unlocked and usable
    /// without blocking on UI prompts.
    fn check_availability(&self) -> Result<(), SecretError> {
        self.get_secret("kestic_health_probe", "probe").map(|_| ())
    }

    /// Attempts an interactive unlock (e.g. via `org.freedesktop.Secret.Prompt`) when the
    /// secret store is locked.
    fn unlock_interactive(&self) -> Result<(), SecretError> {
        self.check_availability()
    }
}

/// In-memory [`SecretStore`] for fast unit tests.
#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    entries: Mutex<BTreeMap<(String, String), Vec<u8>>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for InMemorySecretStore {
    fn set_secret(
        &self,
        target_id: &str,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError> {
        let mut guard = self
            .entries
            .lock()
            .map_err(|_| SecretError::Io("mutex poisoned".to_string()))?;
        guard.insert(
            (target_id.to_string(), kind.to_string()),
            value.as_bytes().to_vec(),
        );
        Ok(())
    }

    fn get_secret(&self, target_id: &str, kind: &str) -> Result<Option<SecretValue>, SecretError> {
        let guard = self
            .entries
            .lock()
            .map_err(|_| SecretError::Io("mutex poisoned".to_string()))?;
        Ok(guard
            .get(&(target_id.to_string(), kind.to_string()))
            .cloned()
            .map(SecretValue::from_bytes))
    }

    fn delete_secret(&self, target_id: &str, kind: &str) -> Result<(), SecretError> {
        let mut guard = self
            .entries
            .lock()
            .map_err(|_| SecretError::Io("mutex poisoned".to_string()))?;
        guard.remove(&(target_id.to_string(), kind.to_string()));
        Ok(())
    }
}

/// File-backed fallback [`SecretStore`] for headless test environments without a D-Bus keyring.
/// Never intended for production persistence (ADR-0004 mandates Secret Service in production).
#[derive(Debug, Clone)]
pub struct FileSecretStore {
    path: PathBuf,
}

impl FileSecretStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load_map(&self) -> Result<BTreeMap<String, Vec<u8>>, SecretError> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        let raw = fs::read_to_string(&self.path)
            .map_err(|err| SecretError::Io(format!("read {}: {err}", self.path.display())))?;
        let mut map = BTreeMap::new();
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some((key, hex_val)) = trimmed.split_once('=') else {
                return Err(SecretError::Io("corrupt fallback secret entry".to_string()));
            };
            let bytes = decode_hex(hex_val)?;
            map.insert(key.to_string(), bytes);
        }
        Ok(map)
    }

    fn save_map(&self, map: &BTreeMap<String, Vec<u8>>) -> Result<(), SecretError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| SecretError::Io(format!("mkdir {}: {err}", parent.display())))?;
        }
        let mut content = String::new();
        for (key, val) in map {
            content.push_str(key);
            content.push('=');
            content.push_str(&encode_hex(val));
            content.push('\n');
        }
        fs::write(&self.path, content)
            .map_err(|err| SecretError::Io(format!("write {}: {err}", self.path.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))
                .map_err(|err| SecretError::Io(format!("chmod {}: {err}", self.path.display())))?;
        }
        Ok(())
    }

    fn make_key(target_id: &str, kind: &str) -> Result<String, SecretError> {
        if target_id.is_empty()
            || kind.is_empty()
            || target_id.contains([':', '=', '\n', '\r'])
            || kind.contains([':', '=', '\n', '\r'])
        {
            return Err(SecretError::Io("invalid target_id or kind".to_string()));
        }
        Ok(format!("{target_id}:{kind}"))
    }
}

impl SecretStore for FileSecretStore {
    fn set_secret(
        &self,
        target_id: &str,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError> {
        let key = Self::make_key(target_id, kind)?;
        let mut map = self.load_map()?;
        map.insert(key, value.as_bytes().to_vec());
        self.save_map(&map)
    }

    fn get_secret(&self, target_id: &str, kind: &str) -> Result<Option<SecretValue>, SecretError> {
        let key = Self::make_key(target_id, kind)?;
        let map = self.load_map()?;
        Ok(map.get(&key).cloned().map(SecretValue::from_bytes))
    }

    fn delete_secret(&self, target_id: &str, kind: &str) -> Result<(), SecretError> {
        let key = Self::make_key(target_id, kind)?;
        let mut map = self.load_map()?;
        map.remove(&key);
        self.save_map(&map)
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, SecretError> {
    if hex.len() % 2 != 0 {
        return Err(SecretError::Io("odd hex length".to_string()));
    }
    let bytes = hex.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let hi = from_hex_nibble(chunk[0])?;
        let lo = from_hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn from_hex_nibble(b: u8) -> Result<u8, SecretError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(SecretError::Io("invalid hex character".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn secret_value_debug_is_redacted() {
        let secret = SecretValue::from_str_secret("super-sensitive-repo-passphrase");
        let dbg = format!("{secret:?}");
        assert!(!dbg.contains("super-sensitive"));
        assert!(dbg.contains("REDACTED"));
    }

    #[test]
    fn in_memory_and_file_secret_store_roundtrip() {
        let mem = InMemorySecretStore::new();
        let val = SecretValue::from_str_secret("p@ssw0rd-äöü");
        mem.set_secret("nas", "repo-password", &val).expect("set");
        assert_eq!(
            mem.get_secret("nas", "repo-password").expect("get"),
            Some(val.clone())
        );
        mem.delete_secret("nas", "repo-password").expect("delete");
        assert_eq!(mem.get_secret("nas", "repo-password").expect("get"), None);

        let tmp = TempDir::new().expect("tempdir");
        let file_store = FileSecretStore::new(tmp.path().join("secrets.hex"));
        file_store
            .set_secret("nas", "repo-password", &val)
            .expect("set");
        assert_eq!(
            file_store.get_secret("nas", "repo-password").expect("get"),
            Some(val)
        );
        file_store
            .delete_secret("nas", "repo-password")
            .expect("delete");
        assert_eq!(
            file_store.get_secret("nas", "repo-password").expect("get"),
            None
        );
    }
}
