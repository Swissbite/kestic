// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Direct `org.freedesktop.secrets` client over `zbus 6.3.0` (`plain` session),
//! avoiding `zbus 5` / `secret-service 5.x` / `keyring 4.x`.

use kestic_core::secrets::{SecretError, SecretStore, SecretValue};
use kestic_core::sync::LockUnpoisoned;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::export::ordered_stream::OrderedStreamExt;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Proxy, Variant, block_on};

const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_IFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_IFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_IFACE: &str = "org.freedesktop.Secret.Item";
const SESSION_IFACE: &str = "org.freedesktop.Secret.Session";
const PROMPT_IFACE: &str = "org.freedesktop.Secret.Prompt";
const INTERACTIVE_UNLOCK_TIMEOUT: Duration = Duration::from_secs(120);

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

    fn application_attributes() -> HashMap<String, String> {
        let mut attrs = HashMap::new();
        attrs.insert("application".to_string(), "net.daester.Kestic".to_string());
        attrs
    }

    fn attributes(target_id: &str, kind: &str) -> HashMap<String, String> {
        let mut attrs = Self::application_attributes();
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

    /// Runs `body` on a fresh connection with an open `plain` session, closing it afterwards.
    async fn with_plain_session<T>(
        &self,
        body: impl AsyncFnOnce(&Connection, &Proxy<'_>, &OwnedObjectPath) -> Result<T, SecretError>,
    ) -> Result<T, SecretError> {
        let conn = Connection::session()
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
        let (service, session) = Self::open_plain_session(&conn, &self.dest).await?;
        let res = body(&conn, &service, &session).await;
        Self::close_session(&conn, &self.dest, &session).await;
        res
    }

    async fn close_session(conn: &Connection, dest: &str, session: &OwnedObjectPath) {
        if let Ok(session_proxy) = Proxy::new(conn, dest, session.as_str(), SESSION_IFACE).await {
            let _: zbus::Result<()> = session_proxy.call("Close", &()).await;
        }
    }

    async fn resolve_default_collection(
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

        Ok(collection)
    }

    async fn is_collection_locked(
        conn: &Connection,
        dest: &str,
        collection: &OwnedObjectPath,
    ) -> Result<bool, SecretError> {
        let col_proxy = Proxy::new(conn, dest, collection.as_str(), COLLECTION_IFACE)
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
        col_proxy
            .get_property("Locked")
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))
    }

    async fn resolve_unlocked_collection(
        conn: &Connection,
        dest: &str,
        service: &Proxy<'_>,
        alias: &str,
    ) -> Result<OwnedObjectPath, SecretError> {
        let collection = Self::resolve_default_collection(service, alias).await?;
        let locked = Self::is_collection_locked(conn, dest, &collection).await?;

        if locked {
            let (unlocked, prompt) = unlock_items(service, vec![collection.clone()])
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

    async fn wait_prompt_completed(
        conn: &Connection,
        dest: &str,
        prompt_path: &OwnedObjectPath,
        timeout: Duration,
    ) -> Result<bool, SecretError> {
        let prompt_proxy = Proxy::new(conn, dest, prompt_path.as_str(), PROMPT_IFACE)
            .await
            .map_err(|e| SecretError::Io(format!("Prompt proxy failed: {e}")))?;

        // Subscribe to Completed before calling Prompt("") so fast responses are never missed.
        let mut completed_stream = prompt_proxy
            .receive_signal("Completed")
            .await
            .map_err(|e| SecretError::Io(format!("Prompt.Completed subscribe failed: {e}")))?;

        let _: () = prompt_proxy
            .call("Prompt", &("",))
            .await
            .map_err(|e| SecretError::Io(format!("Prompt.Prompt failed: {e}")))?;

        let timer_done: Arc<Mutex<(bool, Option<std::task::Waker>)>> =
            Arc::new(Mutex::new((false, None)));
        let timer_bg = Arc::clone(&timer_done);
        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            let waker = {
                let mut g = timer_bg.lock_unpoisoned();
                g.0 = true;
                g.1.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
        });

        let msg_opt = {
            let mut next_fut = std::pin::pin!(completed_stream.next());
            let start = Instant::now();
            std::future::poll_fn(|cx| {
                if let std::task::Poll::Ready(item) = next_fut.as_mut().poll(cx) {
                    return std::task::Poll::Ready(Some(item));
                }
                let mut g = timer_done.lock_unpoisoned();
                if g.0 || start.elapsed() >= timeout {
                    return std::task::Poll::Ready(None);
                }
                g.1 = Some(cx.waker().clone());
                std::task::Poll::Pending
            })
            .await
        };

        let Some(Some(msg)) = msg_opt else {
            return Err(SecretError::LockedOrPromptRequired(
                "unlock prompt timed out".to_string(),
            ));
        };

        let (dismissed, _result): (bool, OwnedVariant) = msg
            .body()
            .deserialize()
            .map_err(|e| SecretError::Io(format!("Prompt.Completed decode failed: {e}")))?;
        Ok(!dismissed)
    }

    async fn check_availability_async(&self) -> Result<(), SecretError> {
        self.with_plain_session(async |conn, service, _session| {
            Self::resolve_unlocked_collection(conn, &self.dest, service, &self.collection_alias)
                .await?;
            let app_attrs = Self::application_attributes();
            let (_unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
                search_items(service, app_attrs).await?;
            if !locked.is_empty() {
                let (now_unlocked, prompt) = unlock_items(service, locked.clone())
                    .await
                    .map_err(|e| SecretError::Io(format!("Unlock item failed: {e}")))?;
                if prompt.as_str() != "/" || now_unlocked.len() < locked.len() {
                    return Err(SecretError::LockedOrPromptRequired(format!(
                        "locked secret items require interactive prompt ({})",
                        prompt.as_str()
                    )));
                }
            }
            Ok(())
        })
        .await
    }

    async fn unlock_interactive_async(&self) -> Result<(), SecretError> {
        self.with_plain_session(async |conn, service, _session| {
            let collection =
                Self::resolve_default_collection(service, &self.collection_alias).await?;
            let mut to_unlock = Vec::new();
            if Self::is_collection_locked(conn, &self.dest, &collection).await? {
                to_unlock.push(collection.clone());
            }

            let app_attrs = Self::application_attributes();
            let (_unlocked, locked_items): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
                search_items(service, app_attrs).await?;
            for item in locked_items {
                if !to_unlock.contains(&item) {
                    to_unlock.push(item);
                }
            }

            if to_unlock.is_empty() {
                return Ok(());
            }

            let (unlocked, prompt) = unlock_items(service, to_unlock)
                .await
                .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;

            if prompt.as_str() == "/" {
                if Self::is_collection_locked(conn, &self.dest, &collection).await?
                    && !unlocked.contains(&collection)
                {
                    return Err(SecretError::LockedOrPromptRequired(format!(
                        "collection {} remained locked",
                        collection.as_str()
                    )));
                }
                return Ok(());
            }

            let accepted =
                Self::wait_prompt_completed(conn, &self.dest, &prompt, INTERACTIVE_UNLOCK_TIMEOUT)
                    .await?;
            if !accepted {
                return Err(SecretError::LockedOrPromptRequired(
                    "unlock prompt cancelled by user".to_string(),
                ));
            }

            if Self::is_collection_locked(conn, &self.dest, &collection).await? {
                return Err(SecretError::LockedOrPromptRequired(format!(
                    "collection {} remained locked after prompt",
                    collection.as_str()
                )));
            }
            Ok(())
        })
        .await
    }

    async fn set_secret_async(
        &self,
        target_id: &str,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError> {
        self.with_plain_session(async |conn, service, session| {
            let collection = Self::resolve_unlocked_collection(
                conn,
                &self.dest,
                service,
                &self.collection_alias,
            )
            .await?;

            let col_proxy = Proxy::new(
                conn,
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
                session,
                Vec::new(),
                value.as_bytes().to_vec(),
                "text/plain; charset=utf8",
            );

            let (_item, prompt): (OwnedObjectPath, OwnedObjectPath) = col_proxy
                .call("CreateItem", &(props, secret_struct, true))
                .await
                .map_err(|e| SecretError::Io(format!("CreateItem failed: {e}")))?;

            reject_prompt(&prompt, "CreateItem")?;

            Ok(())
        })
        .await
    }

    async fn get_secret_async(
        &self,
        target_id: &str,
        kind: &str,
    ) -> Result<Option<SecretValue>, SecretError> {
        self.with_plain_session(async |_conn, service, session| {
            let attrs = Self::attributes(target_id, kind);
            let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
                search_items(service, attrs).await?;

            let item_path = if let Some(path) = unlocked.into_iter().next() {
                path
            } else if !locked.is_empty() {
                let (now_unlocked, prompt) = unlock_items(service, locked)
                    .await
                    .map_err(|e| SecretError::Io(format!("Unlock item failed: {e}")))?;
                reject_prompt(&prompt, "Unlock")?;
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
        })
        .await
    }

    async fn delete_secret_async(&self, target_id: &str, kind: &str) -> Result<(), SecretError> {
        let conn = Connection::session()
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;
        let service = Proxy::new(&conn, self.dest.as_str(), SERVICE_PATH, SERVICE_IFACE)
            .await
            .map_err(|e| SecretError::BackendUnavailable(e.to_string()))?;

        let attrs = Self::attributes(target_id, kind);
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            search_items(&service, attrs).await?;
        for path in unlocked.into_iter().chain(locked) {
            let item_proxy = Proxy::new(&conn, self.dest.as_str(), path.as_str(), ITEM_IFACE)
                .await
                .map_err(|e| SecretError::Io(format!("Item proxy failed: {e}")))?;
            let prompt: OwnedObjectPath = item_proxy
                .call("Delete", &())
                .await
                .map_err(|e| SecretError::Io(format!("Item.Delete failed: {e}")))?;
            reject_prompt(&prompt, "Item.Delete")?;
        }

        Ok(())
    }
}

/// The secret service answers a call that needs the user with a prompt path; `"/"` means none.
/// Calls `Unlock` on `items`; returns the paths that are unlocked now and the prompt path
/// (`/` when no prompt is needed).
async fn unlock_items(
    service: &Proxy<'_>,
    items: Vec<OwnedObjectPath>,
) -> zbus::Result<(Vec<OwnedObjectPath>, OwnedObjectPath)> {
    service.call("Unlock", &(items,)).await
}

/// Items matching `attrs`, as (unlocked, locked) paths.
async fn search_items(
    service: &Proxy<'_>,
    attrs: HashMap<String, String>,
) -> Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>), SecretError> {
    service
        .call("SearchItems", &(attrs,))
        .await
        .map_err(|e| SecretError::Io(format!("SearchItems failed: {e}")))
}

fn reject_prompt(prompt: &OwnedObjectPath, call: &str) -> Result<(), SecretError> {
    if prompt.as_str() == "/" {
        return Ok(());
    }
    Err(SecretError::LockedOrPromptRequired(format!(
        "{call} returned prompt {}",
        prompt.as_str()
    )))
}

impl SecretStore for SecretServiceStore {
    fn set_secret(
        &self,
        target_id: &str,
        kind: &str,
        value: &SecretValue,
    ) -> Result<(), SecretError> {
        std::thread::scope(|s| {
            s.spawn(|| block_on(self.set_secret_async(target_id, kind, value)))
                .join()
                .map_err(|_| SecretError::Io("secret helper thread panicked".into()))?
        })
    }

    fn get_secret(&self, target_id: &str, kind: &str) -> Result<Option<SecretValue>, SecretError> {
        std::thread::scope(|s| {
            s.spawn(|| block_on(self.get_secret_async(target_id, kind)))
                .join()
                .map_err(|_| SecretError::Io("secret helper thread panicked".into()))?
        })
    }

    fn delete_secret(&self, target_id: &str, kind: &str) -> Result<(), SecretError> {
        std::thread::scope(|s| {
            s.spawn(|| block_on(self.delete_secret_async(target_id, kind)))
                .join()
                .map_err(|_| SecretError::Io("secret helper thread panicked".into()))?
        })
    }

    fn check_availability(&self) -> Result<(), SecretError> {
        std::thread::scope(|s| {
            s.spawn(|| block_on(self.check_availability_async()))
                .join()
                .map_err(|_| SecretError::Io("secret helper thread panicked".into()))?
        })
    }

    fn unlock_interactive(&self) -> Result<(), SecretError> {
        std::thread::scope(|s| {
            s.spawn(|| block_on(self.unlock_interactive_async()))
                .join()
                .map_err(|_| SecretError::Io("secret helper thread panicked".into()))?
        })
    }
}
