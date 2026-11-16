// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Direct `org.freedesktop.secrets` client over `zbus 6.3.0` (`plain` session),
//! avoiding `zbus 5` / `secret-service 5.x` / `keyring 4.x` (M0-S08 Option B).

use kestic_core::secrets::{SecretError, SecretStore, SecretValue};
use std::collections::HashMap;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Proxy, Variant, block_on};

const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_IFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_IFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_IFACE: &str = "org.freedesktop.Secret.Item";
const SESSION_IFACE: &str = "org.freedesktop.Secret.Session";

/// `org.freedesktop.secrets` implementation of [`SecretStore`] backed by `zbus 6.3.0`.
#[derive(Debug, Clone)]
pub struct SecretServiceStore {
    dest: String,
    collection_alias: String,
}

impl Default for SecretServiceStore {
    fn default() -> Self {
        Self::new("org.freedesktop.secrets", "default")
    }
}

impl SecretServiceStore {
    /// Creates a store targeting `dest` (normally `"org.freedesktop.secrets"`)
    /// and collection alias (normally `"default"`).
    pub fn new(dest: impl Into<String>, collection_alias: impl Into<String>) -> Self {
        Self {
            dest: dest.into(),
            collection_alias: collection_alias.into(),
        }
    }

    fn attributes(target_id: &str, kind: &str) -> HashMap<String, String> {
        let mut attrs = HashMap::new();
        attrs.insert("application".to_string(), "net.daester.Kestic".to_string());
        attrs.insert("target_id".to_string(), target_id.to_string());
        attrs.insert("kind".to_string(), kind.to_string());
        attrs
    }

    async fn open_plain_session<'a>(
        conn: &'a Connection,
        dest: &'a str,
    ) -> Result<(Proxy<'a>, OwnedObjectPath), SecretError> {
        let service = Proxy::new(conn, dest, SERVICE_PATH, SERVICE_IFACE)
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
        let (_output, session_path): (OwnedVariant, OwnedObjectPath) = service
            .call("OpenSession", &("plain", Variant::from("")))
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
        Ok((service, session_path))
    }

    async fn close_session(conn: &Connection, dest: &str, session: &OwnedObjectPath) {
        if let Ok(session_proxy) = Proxy::new(conn, dest, session.as_str(), SESSION_IFACE).await {
            let _: zbus::Result<()> = session_proxy.call("Close", &()).await;
        }
    }

    async fn resolve_unlocked_collection(
        conn: &Connection,
        dest: &str,
        service: &Proxy<'_>,
        alias: &str,
    ) -> Result<OwnedObjectPath, SecretError> {
        let mut collection: OwnedObjectPath = service
            .call("ReadAlias", &(alias,))
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;

        if collection.as_str() == "/" {
            let collections: Vec<OwnedObjectPath> = service
                .get_property("Collections")
                .await
                .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
            collection = collections.into_iter().next().ok_or_else(|| {
                SecretError::BackendUnavailable("no Secret Service collection found".to_string())
            })?;
        }

        let col_proxy = Proxy::new(conn, dest, collection.as_str(), COLLECTION_IFACE)
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
        let locked: bool = col_proxy
            .get_property("Locked")
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;

        if locked {
            let (unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) = service
                .call("Unlock", &(vec![collection.clone()],))
                .await
                .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
            if prompt.as_str() != "/" || !unlocked.contains(&collection) {
                return Err(SecretError::LockedOrPromptRequired(format!(
                    "collection {} is locked and requires interactive prompt ({})",
                    collection.as_str(),
                    prompt.as_str()
                )));
            }
        }

        Ok(collection)
    }

    async fn set_secret_async(
        &self,
        target_id: &str,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError> {
        let conn = Connection::session()
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
        let (service, session) = Self::open_plain_session(&conn, &self.dest).await?;

        let res = async {
            let collection = Self::resolve_unlocked_collection(
                &conn,
                &self.dest,
                &service,
                &self.collection_alias,
            )
            .await?;

            let col_proxy = Proxy::new(
                &conn,
                self.dest.as_str(),
                collection.as_str(),
                COLLECTION_IFACE,
            )
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;

            let label = format!("Kestic ({target_id} / {kind})");
            let attrs = Self::attributes(target_id, kind);
            let mut props: HashMap<&str, Variant<'_>> = HashMap::new();
            props.insert("org.freedesktop.Secret.Item.Label", Variant::from(label));
            props.insert(
                "org.freedesktop.Secret.Item.Attributes",
                Variant::from(attrs),
            );

            let secret_struct: (&OwnedObjectPath, Vec<u8>, Vec<u8>, &str) = (
                &session,
                Vec::new(),
                value.as_bytes().to_vec(),
                "text/plain; charset=utf8",
            );

            let (_item, prompt): (OwnedObjectPath, OwnedObjectPath) = col_proxy
                .call("CreateItem", &(props, secret_struct, true))
                .await
                .map_err(|e| SecretError::Io(format!("CreateItem failed: {e}")))?;

            if prompt.as_str() != "/" {
                return Err(SecretError::LockedOrPromptRequired(format!(
                    "CreateItem returned prompt {}",
                    prompt.as_str()
                )));
            }

            Ok(())
        }
        .await;

        Self::close_session(&conn, &self.dest, &session).await;
        res
    }

    async fn get_secret_async(
        &self,
        target_id: &str,
        kind: &str,
    ) -> Result<Option<SecretValue>, SecretError> {
        let conn = Connection::session()
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
        let (service, session) = Self::open_plain_session(&conn, &self.dest).await?;

        let res = async {
            let attrs = Self::attributes(target_id, kind);
            let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = service
                .call("SearchItems", &(attrs,))
                .await
                .map_err(|e| SecretError::Io(format!("SearchItems failed: {e}")))?;

            let item_path = if let Some(path) = unlocked.into_iter().next() {
                path
            } else if !locked.is_empty() {
                let (now_unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) = service
                    .call("Unlock", &(locked,))
                    .await
                    .map_err(|e| SecretError::Io(format!("Unlock item failed: {e}")))?;
                if prompt.as_str() != "/" {
                    return Err(SecretError::LockedOrPromptRequired(format!(
                        "Unlock returned prompt {}",
                        prompt.as_str()
                    )));
                }
                match now_unlocked.into_iter().next() {
                    Some(path) => path,
                    None => return Ok(None),
                }
            } else {
                return Ok(None);
            };

            let secrets: HashMap<OwnedObjectPath, (OwnedObjectPath, Vec<u8>, Vec<u8>, String)> =
                service
                    .call("GetSecrets", &(vec![item_path.clone()], session.clone()))
                    .await
                    .map_err(|e| SecretError::Io(format!("GetSecrets failed: {e}")))?;

            Ok(secrets
                .into_iter()
                .find(|(p, _)| *p == item_path)
                .map(|(_, (_sess, _params, bytes, _ct))| SecretValue::from_bytes(bytes)))
        }
        .await;

        Self::close_session(&conn, &self.dest, &session).await;
        res
    }

    async fn delete_secret_async(&self, target_id: &str, kind: &str) -> Result<(), SecretError> {
        let conn = Connection::session()
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
        let service = Proxy::new(&conn, self.dest.as_str(), SERVICE_PATH, SERVICE_IFACE)
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;

        let attrs = Self::attributes(target_id, kind);
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = service
            .call("SearchItems", &(attrs,))
            .await
            .map_err(|e| SecretError::Io(format!("SearchItems failed: {e}")))?;

        for path in unlocked.into_iter().chain(locked) {
            let item_proxy = Proxy::new(&conn, self.dest.as_str(), path.as_str(), ITEM_IFACE)
                .await
                .map_err(|e| SecretError::Io(format!("Item proxy failed: {e}")))?;
            let prompt: OwnedObjectPath = item_proxy
                .call("Delete", &())
                .await
                .map_err(|e| SecretError::Io(format!("Item.Delete failed: {e}")))?;
            if prompt.as_str() != "/" {
                return Err(SecretError::LockedOrPromptRequired(format!(
                    "Item.Delete returned prompt {}",
                    prompt.as_str()
                )));
            }
        }

        Ok(())
    }
}

impl SecretStore for SecretServiceStore {
    fn set_secret(
        &self,
        target_id: &str,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError> {
        block_on(self.set_secret_async(target_id, kind, value))
    }

    fn get_secret(&self, target_id: &str, kind: &str) -> Result<Option<SecretValue>, SecretError> {
        block_on(self.get_secret_async(target_id, kind))
    }

    fn delete_secret(&self, target_id: &str, kind: &str) -> Result<(), SecretError> {
        block_on(self.delete_secret_async(target_id, kind))
    }
}
