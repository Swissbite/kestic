// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Headless `gnome-keyring-daemon` as the Secret Service (`org.freedesktop.secrets`) of a private
//! session bus. It speaks the same D-Bus protocol as KWallet, so the real client code runs
//! end to end without touching the user's wallet.

use crate::bus::wait_for_name_owner;
use crate::process::KillOnDrop;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Proxy, Variant};

/// Bus name of the Secret Service.
pub const SECRET_SERVICE_NAME: &str = "org.freedesktop.secrets";

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
/// File in the control directory that receives the keyring's stderr.
const STDERR_LOG: &str = "stderr.log";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_IFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_IFACE: &str = "org.freedesktop.Secret.Collection";
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";

/// Where the keyring keeps its state; every path lies inside the test's isolated tree.
pub struct KeyringDirs<'a> {
    pub home: &'a Path,
    pub data_home: &'a Path,
    pub runtime_dir: &'a Path,
}

/// A running headless keyring with an unlocked default collection (empty password), killed on
/// `Drop`. If the keyring exited before, its exit status and stderr are printed to stderr: every
/// later password access of the test fails without it.
pub struct HeadlessKeyring {
    child: KillOnDrop,
    /// Short private directory (sun_path limit) for the control socket and the keyring's stderr;
    /// removed on drop.
    control_dir: tempfile::TempDir,
    bus_address: String,
}

impl HeadlessKeyring {
    /// Starts the keyring on the bus at `bus_address` and waits until it owns
    /// `org.freedesktop.secrets`.
    pub fn start(bus_address: &str, dirs: &KeyringDirs<'_>) -> Self {
        let control_dir = crate::tools::short_tempdir("kk");
        // gnome-keyring ignores a control directory that others may access.
        fs::set_permissions(control_dir.path(), fs::Permissions::from_mode(0o700))
            .expect("chmod 0700 keyring control dir");
        let stderr_log = fs::File::create(control_dir.path().join(STDERR_LOG))
            .expect("create keyring stderr log");
        let mut child = KillOnDrop::spawn(
            Command::new("gnome-keyring-daemon")
                .args([
                    "--foreground",
                    "--replace",
                    "--unlock",
                    "--components=secrets",
                    "--control-directory",
                ])
                .arg(control_dir.path())
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("DBUS_SESSION_BUS_ADDRESS", bus_address)
                .env("HOME", dirs.home)
                .env("XDG_DATA_HOME", dirs.data_home)
                .env("XDG_RUNTIME_DIR", dirs.runtime_dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(stderr_log),
            "headless gnome-keyring-daemon",
        );
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(b"\n")
                .expect("write empty keyring password");
        }
        let mut keyring = Self {
            child,
            control_dir,
            bus_address: bus_address.to_string(),
        };
        wait_for_name_owner(
            bus_address,
            SECRET_SERVICE_NAME,
            &mut keyring.child,
            STARTUP_TIMEOUT,
        );
        keyring
    }

    /// Stores `secret` for `target_id` and `kind` in the default collection, as an item with
    /// the attributes Kestic looks up (`application`, `target_id`, `kind`). Written
    /// independently of `SecretServiceStore`, so a change of that item layout fails the tests
    /// that read seeded secrets.
    pub fn store_secret(&self, target_id: &str, kind: &str, secret: &str) {
        zbus::block_on(create_item(&self.bus_address, target_id, kind, secret))
            // The D-Bus error is not shown: an error reply may echo the call's arguments.
            .unwrap_or_else(|_| panic!("storing an item in the headless keyring failed"));
    }
}

async fn create_item(
    bus_address: &str,
    target_id: &str,
    kind: &str,
    secret: &str,
) -> zbus::Result<()> {
    let conn = zbus::connection::Builder::address(bus_address)
        .build()
        .await?;
    // A first call answered from gnome-keyring's main loop, so that its registration of this
    // connection has run before `OpenSession` (gnome-keyring issue #206).
    let properties = Proxy::new(&conn, SECRET_SERVICE_NAME, SERVICE_PATH, PROPERTIES_IFACE).await?;
    let _: OwnedVariant = properties
        .call("Get", &(SERVICE_IFACE, "Collections"))
        .await?;
    let service = Proxy::new(&conn, SECRET_SERVICE_NAME, SERVICE_PATH, SERVICE_IFACE).await?;
    let (_output, session): (OwnedVariant, OwnedObjectPath) = service
        .call("OpenSession", &("plain", Variant::from("")))
        .await?;
    let collection = default_collection(&conn, &service).await?;

    let attributes = HashMap::from([
        ("application", "net.daester.Kestic"),
        ("target_id", target_id),
        ("kind", kind),
    ]);
    let mut props: HashMap<&str, Variant<'_>> = HashMap::new();
    props.insert(
        "org.freedesktop.Secret.Item.Label",
        Variant::from(format!("Kestic ({target_id} / {kind})")),
    );
    props.insert(
        "org.freedesktop.Secret.Item.Attributes",
        Variant::from(attributes),
    );
    let value: (&OwnedObjectPath, Vec<u8>, Vec<u8>, &str) = (
        &session,
        Vec::new(),
        secret.as_bytes().to_vec(),
        "text/plain; charset=utf8",
    );
    let (_item, prompt): (OwnedObjectPath, OwnedObjectPath) =
        collection.call("CreateItem", &(props, value, true)).await?;
    assert_eq!(
        prompt.as_str(),
        "/",
        "the unlocked headless keyring must not prompt"
    );
    Ok(())
}

/// The collection behind the `default` alias, or the first one when no alias is set.
async fn default_collection<'a>(
    conn: &'a Connection,
    service: &Proxy<'_>,
) -> zbus::Result<Proxy<'a>> {
    let mut path: OwnedObjectPath = service.call("ReadAlias", &("default",)).await?;
    if path.as_str() == "/" {
        let collections: Vec<OwnedObjectPath> = service.get_property("Collections").await?;
        path = collections
            .into_iter()
            .next()
            .expect("headless keyring has a collection");
    }
    Proxy::new(conn, SECRET_SERVICE_NAME, path, COLLECTION_IFACE).await
}

impl Drop for HeadlessKeyring {
    fn drop(&mut self) {
        // Printed on drop, so it follows the failing assertion; the test harness shows captured
        // output only for failed tests.
        if let Ok(Some(status)) = self.child.try_wait() {
            let log =
                fs::read_to_string(self.control_dir.path().join(STDERR_LOG)).unwrap_or_default();
            eprintln!(
                "headless keyring on {} ended before the test did ({status}); its stderr:\n{log}",
                self.bus_address
            );
        }
    }
}
