// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! XDG Desktop Portal `Inhibit` client (F9, AC-M0-10-1).

use std::collections::HashMap;
use zbus::{Connection, OwnedObjectPath, Proxy, Variant, block_on};

/// Portal Inhibit flags per XDG Desktop Portal specification:
/// 4 = Suspend, 8 = Idle -> 12 inhibits both idle sleep and suspend.
pub const INHIBIT_SUSPEND: u32 = 4;
pub const INHIBIT_IDLE: u32 = 8;
pub const INHIBIT_IDLE_AND_SUSPEND: u32 = INHIBIT_SUSPEND | INHIBIT_IDLE;

/// Active portal inhibit handle; automatically calls `Request.Close()` on drop if not released.
pub struct PortalInhibitGuard {
    conn: Connection,
    dest: String,
    handle_path: Option<OwnedObjectPath>,
}

impl PortalInhibitGuard {
    /// Acquires an Idle + Suspend inhibit on `org.freedesktop.portal.Desktop`.
    pub fn acquire_default(reason: &str) -> zbus::Result<Self> {
        Self::acquire(
            "org.freedesktop.portal.Desktop",
            reason,
            INHIBIT_IDLE_AND_SUSPEND,
        )
    }

    /// Acquires an inhibit on the given D-Bus destination (allows pointing at a test fake).
    pub fn acquire(dest: &str, reason: &str, flags: u32) -> zbus::Result<Self> {
        let dest_owned = dest.to_string();
        let reason_owned = reason.to_string();
        let (conn, handle_path) = block_on(async move {
            let conn = Connection::session().await?;
            let proxy = Proxy::new(
                &conn,
                dest_owned.as_str(),
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Inhibit",
            )
            .await?;
            let mut options: HashMap<&str, Variant<'_>> = HashMap::new();
            options.insert("reason", Variant::from(reason_owned.as_str()));
            let handle: OwnedObjectPath = proxy.call("Inhibit", &("", flags, options)).await?;
            Ok::<_, zbus::Error>((conn, handle))
        })?;

        Ok(Self {
            conn,
            dest: dest.to_string(),
            handle_path: Some(handle_path),
        })
    }

    /// Returns the portal request object path.
    pub fn handle_path(&self) -> &str {
        self.handle_path
            .as_ref()
            .map(|p| p.as_str())
            .unwrap_or_default()
    }

    /// Releases the inhibit explicitly by calling `org.freedesktop.portal.Request.Close()`.
    pub fn release(mut self) -> zbus::Result<()> {
        self.close_inner()
    }

    fn close_inner(&mut self) -> zbus::Result<()> {
        let Some(handle) = self.handle_path.take() else {
            return Ok(());
        };
        let conn = self.conn.clone();
        let dest = self.dest.clone();
        block_on(async move {
            let proxy = Proxy::new(
                &conn,
                dest.as_str(),
                handle.as_str(),
                "org.freedesktop.portal.Request",
            )
            .await?;
            proxy.call_method("Close", &()).await?;
            Ok::<_, zbus::Error>(())
        })
    }
}

impl Drop for PortalInhibitGuard {
    fn drop(&mut self) {
        let _ = self.close_inner();
    }
}
