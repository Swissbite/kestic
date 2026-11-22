// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Fields of the destination form: the destination they make and its connection test.

use crate::state_cache::{dbus_error_text, make_client, tr_text};
use crate::ui_texts::GuiText;
use kestic_core::local_path::NotALocalPath;
use kestic_core::model::{Destination, DestinationId, DestinationKind, ModelError};
use kestic_daemon::dbus_api::{BackendSecrets, ConnectionTestResultDto};
use kestic_i18n::codes::ConnectionTestCode;
use std::path::PathBuf;

#[derive(serde::Deserialize)]
struct DestinationFieldsInput {
    #[serde(default)]
    id: Option<String>,
    kind: String,
    name: String,
    location: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    endpoint: String,
    #[serde(default)]
    bucket: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    region: String,
}

/// Failed connection test with `code` and its detail.
fn failed_test(code: ConnectionTestCode, detail: String) -> ConnectionTestResultDto {
    ConnectionTestResultDto {
        ok: false,
        error_kind: Some(code),
        detail,
        notes: Vec::new(),
    }
}

/// Why the fields of the destination form do not make a [`Destination`].
pub enum DestinationInputError {
    /// The path is a network address of the file dialog (e.g. `smb://…`).
    NotALocalPath(String),
    Invalid(String),
}

impl DestinationInputError {
    /// Message in the GUI language.
    pub fn text(&self) -> String {
        match self {
            Self::NotALocalPath(address) => {
                tr_text(&ConnectionTestCode::NotALocalPath.text(address))
            }
            Self::Invalid(err) => err.clone(),
        }
    }
}

fn input_error(err: ModelError) -> DestinationInputError {
    match err {
        ModelError::NotALocalPath(NotALocalPath(address)) => {
            DestinationInputError::NotALocalPath(address)
        }
        other => DestinationInputError::Invalid(other.to_string()),
    }
}

pub fn test_destination_connection(
    fields_json: &str,
    mode: &str,
    repo_pw: &str,
    backend_secrets_json: &str,
) -> ConnectionTestResultDto {
    let dest = match destination_from_fields(None, fields_json) {
        Ok(d) => d,
        Err(DestinationInputError::NotALocalPath(address)) => {
            return failed_test(ConnectionTestCode::NotALocalPath, address);
        }
        Err(DestinationInputError::Invalid(err)) => {
            let detail = if err.is_empty() {
                GuiText::FillRequiredFields.text()
            } else {
                err
            };
            return failed_test(ConnectionTestCode::InvalidInput, detail);
        }
    };
    let backend = match BackendSecrets::parse(backend_secrets_json) {
        Ok(b) => b,
        Err(err) => return failed_test(ConnectionTestCode::InvalidInput, err),
    };
    let client = match make_client() {
        Ok(c) => c,
        Err(err) => return failed_test(ConnectionTestCode::DaemonUnreachable, err),
    };
    match client.test_destination_connection(&dest, mode, repo_pw, &backend) {
        Ok(res) => res,
        // Already translated: the text of the D-Bus error name.
        Err(err) => failed_test(ConnectionTestCode::Internal, dbus_error_text(&err)),
    }
}

fn non_empty(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// [`destination_from_fields`] with the error as text in the GUI language.
pub fn build_destination(
    existing_id: Option<&str>,
    fields_json: &str,
) -> Result<Destination, String> {
    destination_from_fields(existing_id, fields_json).map_err(|e| e.text())
}

/// Destination from the fields of the destination form (`fieldsJson()` in QML).
pub fn destination_from_fields(
    existing_id: Option<&str>,
    fields_json: &str,
) -> Result<Destination, DestinationInputError> {
    let fields: DestinationFieldsInput = serde_json::from_str(fields_json)
        .map_err(|e| DestinationInputError::Invalid(e.to_string()))?;
    let trimmed_loc = fields.location.trim();
    let trimmed_name = fields.name.trim();
    let trimmed_user = fields.username.trim();
    let dest_kind = match fields.kind.as_str() {
        "rest" => DestinationKind::Rest {
            url: trimmed_loc.to_string(),
            username: (!trimmed_user.is_empty()).then(|| trimmed_user.to_string()),
        },
        "s3" => DestinationKind::S3 {
            endpoint: fields.endpoint.trim().to_string(),
            bucket: fields.bucket.trim().to_string(),
            path: non_empty(&fields.path),
            region: non_empty(&fields.region),
        },
        _ => DestinationKind::Local {
            path: PathBuf::from(trimmed_loc),
        },
    };
    let id_opt = existing_id
        .filter(|s| !s.is_empty())
        .or_else(|| fields.id.as_deref().filter(|s| !s.is_empty()));
    let dest = if let Some(raw_id) = id_opt {
        let id = DestinationId::parse(raw_id)
            .map_err(|e| DestinationInputError::Invalid(e.to_string()))?;
        Destination::with_id(id, trimmed_name, dest_kind).map_err(input_error)?
    } else {
        Destination::new(trimmed_name, dest_kind).map_err(input_error)?
    };
    Ok(dest)
}
