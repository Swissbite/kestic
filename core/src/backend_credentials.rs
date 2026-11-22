// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backend credentials of a destination (REST password, S3 keys): kept only in the password
//! store and handed to restic only as environment variables, never as arguments.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendSecretSlot {
    pub env_name: &'static str,
    pub kind: &'static str,
}

const REST_PASSWORD: BackendSecretSlot = BackendSecretSlot {
    env_name: "RESTIC_REST_PASSWORD",
    kind: SECRET_KIND_REST,
};
const S3_ACCESS_KEY: BackendSecretSlot = BackendSecretSlot {
    env_name: "AWS_ACCESS_KEY_ID",
    kind: SECRET_KIND_S3_ACCESS_KEY,
};
const S3_SECRET_KEY: BackendSecretSlot = BackendSecretSlot {
    env_name: "AWS_SECRET_ACCESS_KEY",
    kind: SECRET_KIND_S3_SECRET_KEY,
};

/// AWS variables of the caller's environment that would change which credentials, profile or
/// region restic uses for S3; Kestic removes them so only its own values count.
const INHERITED_AWS_VARIABLES: [&str; 11] = [
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_SECURITY_TOKEN",
    "AWS_PROFILE",
    "AWS_SHARED_CREDENTIALS_FILE",
    "AWS_CONFIG_FILE",
    "AWS_ROLE_ARN",
    "AWS_WEB_IDENTITY_TOKEN_FILE",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
];

/// Every slot any destination kind uses, for removing a destination's credentials.
const ALL_SLOTS: [BackendSecretSlot; 3] = [REST_PASSWORD, S3_ACCESS_KEY, S3_SECRET_KEY];

impl DestinationKind {
    /// Credentials restic needs for this backend: the REST password when a user name is set,
    /// both S3 keys for S3.
    pub fn backend_secret_slots(&self) -> Vec<BackendSecretSlot> {
        match self {
            Self::Rest { .. } if self.uses_rest_credentials() => vec![REST_PASSWORD],
            Self::S3 { .. } => vec![S3_ACCESS_KEY, S3_SECRET_KEY],
            Self::Local { .. } | Self::Rest { .. } => Vec::new(),
        }
    }

    /// Variables of the caller's environment that are removed before restic starts, so the
    /// backend gets credentials only from Kestic: the AWS variables for S3.
    pub fn backend_env_cleared(&self) -> Vec<&'static str> {
        match self {
            Self::S3 { .. } => INHERITED_AWS_VARIABLES.to_vec(),
            Self::Local { .. } | Self::Rest { .. } => Vec::new(),
        }
    }

    /// Environment variables for restic that are not secret: the REST user name and the S3
    /// region.
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
            Self::Local { .. } | Self::Rest { .. } | Self::S3 { .. } => Vec::new(),
        }
    }
}

/// Backend credential values for one restic process, by password store kind.
#[derive(Debug, Clone, Default)]
pub struct BackendCredentials {
    values: BTreeMap<&'static str, SecretValue>,
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
        let Some(slot) = ALL_SLOTS.iter().find(|s| s.kind == kind) else {
            return;
        };
        match value.filter(|v| !v.as_bytes().is_empty()) {
            Some(v) => {
                self.values.insert(slot.kind, v);
            }
            None => {
                self.values.remove(slot.kind);
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
            creds.set(slot.kind, store.get_secret(dest.id.as_str(), slot.kind)?);
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
            if !self.values.contains_key(slot.kind) {
                self.set(slot.kind, store.get_secret(dest.id.as_str(), slot.kind)?);
            }
        }
        Ok(self)
    }

    /// Kind of the first credential `kind` needs that is not set.
    pub fn first_missing(&self, kind: &DestinationKind) -> Option<&'static str> {
        kind.backend_secret_slots()
            .into_iter()
            .map(|s| s.kind)
            .find(|k| !self.values.contains_key(k))
    }

    /// Saves the credentials `dest` needs in the password store; on failure removes what was
    /// saved by this call.
    pub fn persist(&self, dest: &Destination, store: &dyn SecretStore) -> Result<(), SecretError> {
        let mut saved = Vec::new();
        for slot in dest.kind.backend_secret_slots() {
            let Some(value) = self.values.get(slot.kind) else {
                continue;
            };
            if let Err(err) = store.set_secret(dest.id.as_str(), slot.kind, value) {
                for kind in saved {
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
        for slot in ALL_SLOTS {
            store.delete_secret(dest.id.as_str(), slot.kind)?;
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
            if let Some(value) = self.values.get(slot.kind) {
                cmd.env(slot.env_name, value.as_str()?);
                names.push(slot.env_name.to_string());
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
