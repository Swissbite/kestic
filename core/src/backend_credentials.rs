// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backend credentials of a destination (REST password, S3 keys, secret variables of the generic
//! form): kept only in the password store and handed to restic only as environment variables,
//! never as arguments.

use crate::model::{Destination, DestinationKind};
use crate::secrets::{SecretError, SecretStore, SecretValue};
use std::collections::BTreeMap;
use std::process::Command;

/// Password store kind of the REST server password.
pub const SECRET_KIND_REST: &str = "rest";
/// Password store kind of the S3 access key ID.
pub const SECRET_KIND_S3_ACCESS_KEY: &str = "s3_access_key";
/// Password store kind of the S3 secret access key.
pub const SECRET_KIND_S3_SECRET_KEY: &str = "s3_secret_key";

/// A credential restic reads from the environment variable `env_name`, stored under `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSecretSlot {
    pub env_name: String,
    pub kind: String,
}

impl BackendSecretSlot {
    fn new(env_name: &str, kind: &str) -> Self {
        Self {
            env_name: env_name.to_string(),
            kind: kind.to_string(),
        }
    }
}

/// Prefixes of the variables in the caller's environment that would change which credentials,
/// profile, role or region restic uses for S3; Kestic removes every such variable so only its
/// own values count.
const INHERITED_AWS_PREFIXES: [&str; 2] = ["AWS_", "RESTIC_AWS_"];

/// The kinds of the fixed credentials, for removing a destination's credentials whatever its
/// kind was.
const FIXED_KINDS: [&str; 3] = [
    SECRET_KIND_REST,
    SECRET_KIND_S3_ACCESS_KEY,
    SECRET_KIND_S3_SECRET_KEY,
];

impl DestinationKind {
    /// Credentials restic needs for this backend: the REST password when a user name is set,
    /// both S3 keys for S3, the secret variables of a generic destination.
    pub fn backend_secret_slots(&self) -> Vec<BackendSecretSlot> {
        match self {
            Self::Rest { .. } if self.uses_rest_credentials() => {
                vec![BackendSecretSlot::new(
                    "RESTIC_REST_PASSWORD",
                    SECRET_KIND_REST,
                )]
            }
            Self::S3 { .. } => vec![
                BackendSecretSlot::new("AWS_ACCESS_KEY_ID", SECRET_KIND_S3_ACCESS_KEY),
                BackendSecretSlot::new("AWS_SECRET_ACCESS_KEY", SECRET_KIND_S3_SECRET_KEY),
            ],
            Self::Generic { env, .. } => crate::generic::allowed(env)
                .filter(|v| v.secret)
                .map(|v| BackendSecretSlot::new(&v.name, &crate::generic::secret_kind(&v.name)))
                .collect(),
            Self::Local { .. } | Self::Rest { .. } => Vec::new(),
        }
    }

    /// Name prefixes of the variables in the caller's environment that are removed before
    /// restic starts, so the backend gets credentials only from Kestic: `AWS_` and
    /// `RESTIC_AWS_` for S3.
    pub fn backend_env_cleared_prefixes(&self) -> Vec<&'static str> {
        match self {
            Self::S3 { .. } => INHERITED_AWS_PREFIXES.to_vec(),
            Self::Local { .. } | Self::Rest { .. } | Self::Generic { .. } => Vec::new(),
        }
    }

    /// Environment variables for restic that are not secret: the REST user name, the S3 region
    /// and the variables of a generic destination that are not secret.
    pub fn backend_plain_env(&self) -> Vec<(String, String)> {
        match self {
            Self::Rest {
                username: Some(user),
                ..
            } if !user.is_empty() => vec![("RESTIC_REST_USERNAME".to_string(), user.clone())],
            Self::S3 {
                region: Some(region),
                ..
            } if !region.trim().is_empty() => {
                vec![("AWS_DEFAULT_REGION".to_string(), region.trim().to_string())]
            }
            Self::Generic { env, .. } => crate::generic::allowed(env)
                .filter(|v| !v.secret)
                .map(|v| (v.name.clone(), v.value.clone().unwrap_or_default()))
                .collect(),
            Self::Local { .. } | Self::Rest { .. } | Self::S3 { .. } => Vec::new(),
        }
    }
}

/// Backend credential values for one restic process, by password store kind.
#[derive(Debug, Clone, Default)]
pub struct BackendCredentials {
    values: BTreeMap<String, SecretValue>,
}

impl BackendCredentials {
    /// No credentials.
    pub fn none() -> Self {
        Self::default()
    }

    /// The REST server password, if not empty.
    pub fn rest(password: Option<SecretValue>) -> Self {
        let mut creds = Self::default();
        creds.set(SECRET_KIND_REST, password);
        creds
    }

    /// Sets the value of `kind`; `None` or an empty value removes it.
    pub fn set(&mut self, kind: &str, value: Option<SecretValue>) {
        match value.filter(|v| !v.as_bytes().is_empty()) {
            Some(v) => {
                self.values.insert(kind.to_string(), v);
            }
            None => {
                self.values.remove(kind);
            }
        }
    }

    /// The value of `kind`, if set.
    pub fn get(&self, kind: &str) -> Option<&SecretValue> {
        self.values.get(kind)
    }

    /// The stored credentials `dest` needs; a missing one is left out.
    pub fn load_stored(dest: &Destination, store: &dyn SecretStore) -> Result<Self, SecretError> {
        let mut creds = Self::default();
        for slot in dest.kind.backend_secret_slots() {
            creds.set(&slot.kind, store.get_secret(dest.id.as_str(), &slot.kind)?);
        }
        Ok(creds)
    }

    /// These credentials, with the stored value of `dest` for every needed one that is empty
    /// (editing keeps what was saved).
    pub fn or_stored(
        mut self,
        dest: &Destination,
        store: &dyn SecretStore,
    ) -> Result<Self, SecretError> {
        for slot in dest.kind.backend_secret_slots() {
            if !self.values.contains_key(&slot.kind) {
                self.set(&slot.kind, store.get_secret(dest.id.as_str(), &slot.kind)?);
            }
        }
        Ok(self)
    }

    /// Kind of the first credential `kind` needs that is not set.
    pub fn first_missing(&self, kind: &DestinationKind) -> Option<String> {
        kind.backend_secret_slots()
            .into_iter()
            .map(|s| s.kind)
            .find(|k| !self.values.contains_key(k))
    }

    /// Saves the credentials `dest` needs in the password store; on failure removes what was
    /// saved by this call.
    pub fn persist(&self, dest: &Destination, store: &dyn SecretStore) -> Result<(), SecretError> {
        let mut saved: Vec<String> = Vec::new();
        for slot in dest.kind.backend_secret_slots() {
            let Some(value) = self.values.get(&slot.kind) else {
                continue;
            };
            if let Err(err) = store.set_secret(dest.id.as_str(), &slot.kind, value) {
                for kind in &saved {
                    let _ = store.delete_secret(dest.id.as_str(), kind);
                }
                return Err(err);
            }
            saved.push(slot.kind);
        }
        Ok(())
    }

    /// Removes every backend credential of `dest` from the password store.
    pub fn delete_all(dest: &Destination, store: &dyn SecretStore) -> Result<(), SecretError> {
        for kind in FIXED_KINDS {
            store.delete_secret(dest.id.as_str(), kind)?;
        }
        for slot in dest.kind.backend_secret_slots() {
            store.delete_secret(dest.id.as_str(), &slot.kind)?;
        }
        Ok(())
    }

    /// Removes the stored credentials `old` needed and `new` (the same destination after an
    /// edit) no longer needs, such as a secret variable that was removed or renamed.
    pub fn delete_unused(
        old: &Destination,
        new: &Destination,
        store: &dyn SecretStore,
    ) -> Result<(), SecretError> {
        let kept: Vec<String> = new
            .kind
            .backend_secret_slots()
            .into_iter()
            .map(|s| s.kind)
            .collect();
        for slot in old.kind.backend_secret_slots() {
            if !kept.contains(&slot.kind) {
                store.delete_secret(old.id.as_str(), &slot.kind)?;
            }
        }
        Ok(())
    }

    /// Sets the variables of `slots` that have a value on `cmd`; returns their names.
    pub fn apply_env(
        &self,
        cmd: &mut Command,
        slots: &[BackendSecretSlot],
    ) -> Result<Vec<String>, SecretError> {
        let mut names = Vec::new();
        for slot in slots {
            if let Some(value) = self.values.get(&slot.kind) {
                cmd.env(&slot.env_name, value.as_str()?);
                names.push(slot.env_name.clone());
            }
        }
        Ok(names)
    }

    /// The values as text, for redacting them from output.
    pub fn redaction_values(&self) -> Vec<&str> {
        self.values
            .values()
            .filter_map(|v| v.as_str().ok())
            .collect()
    }
}
