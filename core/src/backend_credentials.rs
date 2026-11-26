// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backend credentials of a destination (REST password, S3 keys, secret variables of the generic
//! form): kept only in the password store and handed to restic only as environment variables,
//! never as arguments.

use crate::generic::EnteredSecret;
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
/// profile, role or region restic uses for S3 (restic also reads MinIO's own variables); Kestic
/// removes every such variable so only its own values count.
const INHERITED_S3_PREFIXES: [&str; 3] = ["AWS_", "RESTIC_AWS_", "MINIO_"];

/// Prefix of the user name and password restic reads for a REST server.
const INHERITED_REST_PREFIXES: [&str; 1] = ["RESTIC_REST_"];

/// Prefixes of the variables in the caller's environment that the backends of the generic form
/// read: B2, Azure (and its managed identity: `IDENTITY_`, `MSI_`, `IMDS_`), Google Cloud
/// Storage (and the gcloud configuration and metadata server: `CLOUDSDK_`, `GCE_`), Swift
/// (`OS_`, and `ST_` for its v1 login), rclone, and the AWS variables rclone reads for an S3
/// remote; a destination gets only the variables configured for it.
const INHERITED_GENERIC_PREFIXES: [&str; 12] = [
    "B2_",
    "AZURE_",
    "IDENTITY_",
    "MSI_",
    "IMDS_",
    "GOOGLE_",
    "CLOUDSDK_",
    "GCE_",
    "OS_",
    "ST_",
    "RCLONE_",
    "AWS_",
];

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
            Self::Local { .. } | Self::Rest { .. } | Self::Sftp { .. } => Vec::new(),
        }
    }

    /// restic options after the repository: the ssh options of an SFTP destination
    /// ([`crate::sftp::restic_options`]).
    pub fn restic_backend_options(&self) -> Vec<String> {
        match self {
            Self::Sftp { .. } => crate::sftp::restic_options(),
            Self::Local { .. } | Self::Rest { .. } | Self::S3 { .. } | Self::Generic { .. } => {
                Vec::new()
            }
        }
    }

    /// Name prefixes of the variables in the caller's environment that are removed before
    /// restic starts, so the backend gets credentials only from Kestic: `AWS_`, `RESTIC_AWS_`
    /// and `MINIO_` for S3; `RESTIC_REST_` for REST; the backend prefixes of the generic form
    /// (`B2_`, `AZURE_`, `GOOGLE_`, …), whose own variables are set afterwards.
    pub fn backend_env_cleared_prefixes(&self) -> Vec<&'static str> {
        match self {
            Self::S3 { .. } => INHERITED_S3_PREFIXES.to_vec(),
            Self::Rest { .. } => INHERITED_REST_PREFIXES.to_vec(),
            Self::Generic { .. } => INHERITED_GENERIC_PREFIXES.to_vec(),
            Self::Local { .. } | Self::Sftp { .. } => Vec::new(),
        }
    }

    /// The credential variables a generic destination lacks (see
    /// [`crate::generic::missing_credentials`]), with the secret values from `creds`; `None`
    /// for every other kind.
    pub fn missing_credential_variables(&self, creds: &BackendCredentials) -> Option<String> {
        let Self::Generic { url, env } = self else {
            return None;
        };
        crate::generic::missing_credentials(url, env, |name| {
            match creds.get(&crate::generic::secret_kind(name)) {
                None => EnteredSecret::Missing,
                Some(value) => value
                    .as_str()
                    .map_or(EnteredSecret::Saved, EnteredSecret::Value),
            }
        })
    }

    /// Whether restic, missing a credential Kestic would set, falls back to credential files or
    /// logins of the user (`~/.aws/credentials`, a gcloud or Azure CLI login) and would send
    /// those to the endpoint; a REST server without password only gets the user name.
    pub(crate) fn falls_back_to_user_credentials(&self) -> bool {
        matches!(self, Self::S3 { .. } | Self::Generic { .. })
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
            Self::Local { .. } | Self::Rest { .. } | Self::Sftp { .. } | Self::S3 { .. } => {
                Vec::new()
            }
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

    /// Whether no credential is set.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
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

    /// Saves the credentials `dest` needs in the password store and returns the values they
    /// replaced; on failure writes back what this call replaced.
    pub fn persist(
        &self,
        dest: &Destination,
        store: &dyn SecretStore,
    ) -> Result<PreviousSecrets, SecretError> {
        let mut previous = PreviousSecrets::new(dest);
        for slot in dest.kind.backend_secret_slots() {
            let Some(value) = self.values.get(&slot.kind) else {
                continue;
            };
            if let Err(err) = previous.replace(store, &slot.kind, value) {
                previous.restore(store);
                return Err(err);
            }
        }
        Ok(previous)
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

/// The password store entries of one destination as they were before a save, to write them back
/// when a later step fails: a value is restored, an entry that did not exist is removed.
#[derive(Debug)]
pub struct PreviousSecrets {
    target: String,
    entries: Vec<(String, Option<SecretValue>)>,
}

impl PreviousSecrets {
    /// Nothing replaced yet for `dest`.
    pub fn new(dest: &Destination) -> Self {
        Self {
            target: dest.id.as_str().to_string(),
            entries: Vec::new(),
        }
    }

    /// Writes `value` under `kind` and remembers the value it replaces.
    pub fn replace(
        &mut self,
        store: &dyn SecretStore,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError> {
        let before = store.get_secret(&self.target, kind)?;
        store.set_secret(&self.target, kind, value)?;
        self.entries.push((kind.to_string(), before));
        Ok(())
    }

    /// Adds the entries replaced by `other` (a later save of the same destination).
    pub fn extend(&mut self, other: Self) {
        self.entries.extend(other.entries);
    }

    /// Writes the remembered values back, the last replaced first; errors are ignored because
    /// the caller is already reporting the failure that made it undo the save.
    pub fn restore(self, store: &dyn SecretStore) {
        for (kind, before) in self.entries.into_iter().rev() {
            let _ = match before {
                Some(value) => store.set_secret(&self.target, &kind, &value),
                None => store.delete_secret(&self.target, &kind),
            };
        }
    }
}
