// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Fields of the destination form: the destination they make and its connection test.

use crate::state_cache::{dbus_error_text, i18n, make_client, tr_text};
use crate::ui_texts::GuiText;
use kestic_core::generic::{EnvVar, GenericInputError};
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
    /// Variables of the generic form; the value of a secret one goes in `backendSecretsJson()`.
    #[serde(default)]
    env: Vec<EnvFieldInput>,
}

#[derive(serde::Deserialize)]
struct EnvFieldInput {
    name: String,
    #[serde(default)]
    secret: bool,
    #[serde(default)]
    value: String,
}

impl EnvFieldInput {
    fn into_env_var(self) -> EnvVar {
        EnvVar {
            name: self.name.trim().to_string(),
            secret: self.secret,
            value: (!self.secret).then_some(self.value),
        }
    }
}

/// Failed connection test with `code` and its detail.
fn failed_test(code: ConnectionTestCode, detail: String) -> ConnectionTestResultDto {
    ConnectionTestResultDto {
        ok: false,
        error_kind: Some(code),
        detail,
        notes: Vec::new(),
        program: None,
    }
}

/// Why the fields of the destination form do not make a [`Destination`].
pub enum DestinationInputError {
    /// A refusal with its own text: a network address of the file dialog as path
    /// (`not_a_local_path`), an unknown scheme or a refused variable of the generic form.
    Coded(ConnectionTestCode, String),
    Invalid(String),
}

impl DestinationInputError {
    /// Message in the GUI language.
    pub fn text(&self) -> String {
        match self {
            Self::Coded(code, detail) => tr_text(&code.text(detail)),
            Self::Invalid(err) => err.clone(),
        }
    }
}

fn input_error(err: ModelError) -> DestinationInputError {
    match err {
        ModelError::NotALocalPath(NotALocalPath(address)) => {
            DestinationInputError::Coded(ConnectionTestCode::NotALocalPath, address)
        }
        ModelError::Generic(generic) => generic_error(generic),
        other => DestinationInputError::Invalid(other.to_string()),
    }
}

fn generic_error(err: GenericInputError) -> DestinationInputError {
    match err {
        GenericInputError::UnsupportedScheme(url) => {
            DestinationInputError::Coded(ConnectionTestCode::UnsupportedScheme, url)
        }
        GenericInputError::ReservedVariable(name) => {
            DestinationInputError::Coded(ConnectionTestCode::ReservedVariable, name)
        }
        GenericInputError::MissingAddress(_) => DestinationInputError::Invalid(
            i18n("Enter the address of the storage after the scheme.").to_string(),
        ),
        GenericInputError::OptionLikeAddress(_) => DestinationInputError::Invalid(
            i18n("The address after the scheme must not start with '-'.").to_string(),
        ),
        GenericInputError::DuplicateVariable(name) => {
            DestinationInputError::Invalid(i18n("The variable '%1' is given twice.").subs(&[&name]))
        }
        GenericInputError::NulInValue(name) => DestinationInputError::Invalid(
            i18n("The value of '%1' contains a character that cannot be passed on.").subs(&[&name]),
        ),
        other @ GenericInputError::SecretValueStored(_) => {
            DestinationInputError::Invalid(other.to_string())
        }
    }
}

/// Why `core` refuses the repository URL of the generic form, as on saving; `None` when it is
/// accepted. The form keeps «Test Connection», and so adding the destination, locked while there
/// is one.
pub fn generic_url_error(url: &str) -> Option<DestinationInputError> {
    kestic_core::generic::validate_url(url)
        .err()
        .map(generic_error)
}

/// Why `core` refuses the variables of the generic form (`env_json`: the `env` list of
/// `fieldsJson()`); `None` when they are accepted.
pub fn generic_env_error(env_json: &str) -> Option<DestinationInputError> {
    let fields: Vec<EnvFieldInput> = match serde_json::from_str(env_json) {
        Ok(f) => f,
        Err(e) => return Some(DestinationInputError::Invalid(e.to_string())),
    };
    let env: Vec<EnvVar> = fields
        .into_iter()
        .map(EnvFieldInput::into_env_var)
        .collect();
    kestic_core::generic::validate_env(&env)
        .err()
        .map(generic_error)
}

pub fn test_destination_connection(
    fields_json: &str,
    mode: &str,
    repo_pw: &str,
    backend_secrets_json: &str,
) -> ConnectionTestResultDto {
    let dest = match destination_from_fields(None, fields_json) {
        Ok(d) => d,
        Err(DestinationInputError::Coded(code, detail)) => {
            return failed_test(code, detail);
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
        "generic" => DestinationKind::Generic {
            url: trimmed_loc.to_string(),
            env: fields
                .env
                .into_iter()
                .map(EnvFieldInput::into_env_var)
                .collect(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(err: Option<DestinationInputError>) -> Option<(ConnectionTestCode, String)> {
        match err {
            Some(DestinationInputError::Coded(code, detail)) => Some((code, detail)),
            _ => None,
        }
    }

    /// AC-M5-03-1 (c): the form check behind «Test Connection» (and so «Anlegen») refuses an
    /// unknown scheme with `core`'s code and accepts the known ones.
    #[test]
    fn ac_m5_03_1_unknown_scheme_keeps_adding_locked() {
        assert_eq!(
            code_of(generic_url_error("ftp:host:/x")),
            Some((
                ConnectionTestCode::UnsupportedScheme,
                "ftp:host:/x".to_string()
            )),
            "(c) unknown scheme"
        );
        assert!(
            generic_url_error("gs:bucket:/laptop").is_none(),
            "(c) known scheme"
        );
    }

    /// AC-M5-03-7 (b): a reserved or invalid variable name keeps «Anlegen» locked, with the name
    /// in the message.
    #[test]
    fn ac_m5_03_7_reserved_variable_keeps_adding_locked() {
        for name in ["LD_PRELOAD", "HOME", "lower_case"] {
            let env = format!(r#"[{{"name":"{name}","secret":false,"value":"x"}}]"#);
            assert_eq!(
                code_of(generic_env_error(&env)),
                Some((ConnectionTestCode::ReservedVariable, name.to_string())),
                "(b) {name}"
            );
        }
        let ok = r#"[{"name":"B2_ACCOUNT_KEY","secret":true,"value":""}]"#;
        assert!(
            generic_env_error(ok).is_none(),
            "(b) a valid secret variable"
        );
    }
}
