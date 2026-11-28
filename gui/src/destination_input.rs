// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Fields of the destination form: the destination they make and its connection test.

use crate::state_cache::{dbus_error_text, i18n, tr_text};
use crate::ui_texts::GuiText;
use kestic_core::generic::{EnteredSecret, EnvVar, GenericInputError};
use kestic_core::local_path::NotALocalPath;
use kestic_core::model::{Destination, DestinationId, DestinationKind, ModelError};
use kestic_daemon::dbus_api::{BackendSecrets, ConnectionTestResultDto, KesticDbusError};
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
    /// SFTP server; `path` and `username` hold the path and the user.
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: Option<u16>,
    /// Variables of the generic form; the value of a secret one goes in `backendSecretsJson()`.
    #[serde(default)]
    env: Vec<EnvFieldInput>,
}

#[derive(serde::Deserialize)]
struct EnvFieldInput {
    name: String,
    #[serde(default)]
    secret: bool,
    /// For a secret variable only in the check of the credential variables
    /// ([`generic_credentials_error`]); it never goes into the destination.
    #[serde(default)]
    value: String,
    /// A secret variable left empty while editing keeps its saved value.
    #[serde(default)]
    saved: bool,
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
        host_key: None,
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
        GenericInputError::SecretInUrl(param) => {
            DestinationInputError::Coded(ConnectionTestCode::SecretInUrl, param)
        }
        GenericInputError::ReservedVariable(name) if name.is_empty() => {
            DestinationInputError::Invalid(i18n("Enter a name for every variable.").to_string())
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

/// Why the generic form lacks a credential variable its backend needs (an `azure:` or `gs:`
/// destination, see [`kestic_core::generic::missing_credentials`]), naming the variables;
/// `env_json` holds the variables with the values of the secret ones as typed. `None` when one
/// is entered or the backend needs none.
pub fn generic_credentials_error(url: &str, env_json: &str) -> Option<DestinationInputError> {
    let fields: Vec<EnvFieldInput> = serde_json::from_str(env_json).ok()?;
    let secrets: Vec<(String, EnvFieldInput)> = fields
        .into_iter()
        .map(|f| (f.name.trim().to_string(), f))
        .collect();
    let env: Vec<EnvVar> = secrets
        .iter()
        .map(|(name, f)| EnvVar {
            name: name.clone(),
            secret: f.secret,
            value: (!f.secret).then(|| f.value.clone()),
        })
        .collect();
    kestic_core::generic::missing_credentials(url, &env, |name| {
        match secrets.iter().find(|(n, _)| n == name) {
            Some((_, f)) if !f.value.is_empty() => EnteredSecret::Value(&f.value),
            Some((_, f)) if f.saved => EnteredSecret::Saved,
            _ => EnteredSecret::Missing,
        }
    })
    .map(|variables| {
        DestinationInputError::Coded(ConnectionTestCode::CredentialsNotEntered, variables)
    })
}

/// The destination and its backend credentials from the form, or the result of a test that
/// the form's input already fails.
pub fn connection_test_input(
    fields_json: &str,
    backend_secrets_json: &str,
) -> Result<(Destination, BackendSecrets), ConnectionTestResultDto> {
    let dest = match destination_from_fields(None, fields_json) {
        Ok(d) => d,
        Err(DestinationInputError::Coded(code, detail)) => {
            return Err(failed_test(code, detail));
        }
        Err(DestinationInputError::Invalid(err)) => {
            let detail = if err.is_empty() {
                GuiText::FillRequiredFields.text()
            } else {
                err
            };
            return Err(failed_test(ConnectionTestCode::InvalidInput, detail));
        }
    };
    let backend = BackendSecrets::parse(backend_secrets_json)
        .map_err(|err| failed_test(ConnectionTestCode::InvalidInput, err))?;
    Ok((dest, backend))
}

/// The result of the daemon's connection test as the form shows it: `Err` from the bus (no
/// connection) or from the daemon becomes a failed test.
pub fn connection_test_outcome(
    answer: Result<Result<ConnectionTestResultDto, KesticDbusError>, String>,
) -> ConnectionTestResultDto {
    match answer {
        Ok(Ok(res)) => res,
        // Already translated: the text of the D-Bus error name.
        Ok(Err(err)) => failed_test(ConnectionTestCode::Internal, dbus_error_text(&err)),
        Err(err) => failed_test(ConnectionTestCode::DaemonUnreachable, err),
    }
}

fn non_empty(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The S3 and SFTP fields of the destination form as QML has them (`fieldChecks` in
/// `DestinationDraft.qml`).
#[derive(serde::Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct FieldsToCheck {
    endpoint: String,
    bucket: String,
    s3_path: String,
    region: String,
    host: String,
    port: String,
    username: String,
    sftp_path: String,
}

/// Which S3 and SFTP fields `core` accepts, each as a boolean of the same name, for the form's
/// messages next to the fields; unreadable input accepts nothing.
pub fn field_checks(fields_json: &str) -> serde_json::Value {
    use kestic_core::{s3, sftp};
    let f: FieldsToCheck = serde_json::from_str(fields_json).unwrap_or_default();
    let s3_path = f.s3_path.trim();
    serde_json::json!({
        "endpoint": s3::normalized_endpoint(&f.endpoint).is_ok(),
        "bucket": s3::is_valid_bucket_name(f.bucket.trim()),
        "s3Path": s3_path.is_empty() || s3::check_path(s3_path).is_ok(),
        "region": s3::validate_region(Some(&f.region)).is_ok(),
        "host": sftp::is_valid_host(&f.host),
        "port": sftp::parse_port(&f.port).is_some(),
        "username": sftp::is_valid_user(&f.username),
        "sftpPath": sftp::check_path(&f.sftp_path).is_ok(),
    })
}

/// The `revision` an edit page adds to the form fields (empty without one).
pub fn revision_of_fields(fields_json: &str) -> String {
    #[derive(serde::Deserialize)]
    struct WithRevision {
        #[serde(default)]
        revision: String,
    }
    serde_json::from_str::<WithRevision>(fields_json).map_or_else(|_| String::new(), |f| f.revision)
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
        "sftp" => DestinationKind::Sftp {
            host: fields.host.trim().to_string(),
            port: fields.port.unwrap_or(kestic_core::sftp::DEFAULT_PORT),
            user: trimmed_user.to_string(),
            path: fields.path.trim().to_string(),
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

    /// The form checks the S3 and SFTP fields with `core`'s rules.
    #[test]
    fn field_checks_follow_core() {
        let good = r#"{"endpoint":"https://s3.example.com","bucket":"kestic-backups","s3Path":"laptop/home","region":"eu-west-1","host":"[::1]","port":"22","username":"anna","sftpPath":"backups"}"#;
        let checks = field_checks(good);
        for field in [
            "endpoint", "bucket", "s3Path", "region", "host", "port", "username", "sftpPath",
        ] {
            assert_eq!(checks[field], true, "{field} accepted");
        }
        let bad = r#"{"endpoint":"https://h/x?y","bucket":"192.168.1.1","s3Path":"a/../b","region":"eu west","host":"dead:beef","port":"+22","username":"-o","sftpPath":"~/backups"}"#;
        let checks = field_checks(bad);
        for field in [
            "endpoint", "bucket", "s3Path", "region", "host", "port", "username", "sftpPath",
        ] {
            assert_eq!(checks[field], false, "{field} refused");
        }
        assert_eq!(field_checks(r#"{"s3Path":""}"#)["s3Path"], true, "no path");
        assert_eq!(field_checks("not json")["port"], false, "unreadable");
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

    /// AC-M5-03-8 (a): the form check behind «Test Connection» (and so «Anlegen») refuses an
    /// `azure:` or `gs:` destination without a credential variable with the code that names the
    /// variables, and accepts one entered as plain value, as typed secret or as saved secret.
    #[test]
    fn ac_m5_03_8_missing_credential_variable_keeps_adding_locked() {
        let azure = "azure:kestic-container:/laptop";
        let azure_needs = Some((
            ConnectionTestCode::CredentialsNotEntered,
            "AZURE_ACCOUNT_KEY, AZURE_ACCOUNT_SAS, AZURE_FORCE_CLI_CREDENTIAL=true".to_string(),
        ));
        for env in [
            r#"[]"#,
            r#"[{"name":"AZURE_ACCOUNT_NAME","secret":false,"value":"kestic"}]"#,
            r#"[{"name":"AZURE_ACCOUNT_KEY","secret":true,"value":""}]"#,
            r#"[{"name":"AZURE_FORCE_CLI_CREDENTIAL","secret":true,"value":"false"}]"#,
        ] {
            assert_eq!(
                code_of(generic_credentials_error(azure, env)),
                azure_needs,
                "(a) refused: {env}"
            );
        }
        assert_eq!(
            code_of(generic_credentials_error("gs:bucket:/laptop", "[]")),
            Some((
                ConnectionTestCode::CredentialsNotEntered,
                "GOOGLE_ACCESS_TOKEN, GOOGLE_APPLICATION_CREDENTIALS".to_string()
            )),
            "(a) gs: names its variables"
        );
        for env in [
            r#"[{"name":"AZURE_FORCE_CLI_CREDENTIAL","secret":false,"value":"true"}]"#,
            r#"[{"name":"AZURE_FORCE_CLI_CREDENTIAL","secret":true,"value":"true"}]"#,
            r#"[{"name":"AZURE_ACCOUNT_SAS","secret":true,"value":"sv=2024"}]"#,
            r#"[{"name":"AZURE_ACCOUNT_KEY","secret":true,"value":"","saved":true}]"#,
        ] {
            assert!(
                generic_credentials_error(azure, env).is_none(),
                "(a) accepted: {env}"
            );
        }
        assert!(
            generic_credentials_error("b2:bucket:laptop", "[]").is_none(),
            "(a) other schemes need no such variable"
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
