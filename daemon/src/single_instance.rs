// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! D-Bus single-instance lock for the Kestic daemon (F14, AC-M0-07-1).
//!
//! Claims the session bus name `net.daester.Kestic` with `DO_NOT_QUEUE` and
//! without `ALLOW_REPLACEMENT` or `REPLACE_EXISTING` so a second daemon
//! immediately observes `NameExists` / `AlreadyRunning` and exits cleanly.

use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::{Connection, block_on, interface};

/// Well-known session bus name used as the daemon single-instance lock.
pub const KESTIC_BUS_NAME: &str = "net.daester.Kestic";

/// Primary object path exported by the Kestic daemon.
pub const KESTIC_OBJECT_PATH: &str = "/net/daester/Kestic";

struct KesticDbusService;

#[interface(name = "net.daester.Kestic1")]
impl KesticDbusService {
    #[zbus(name = "DaemonVersion")]
    fn daemon_version(&self) -> String {
        crate::daemon_version().to_string()
    }
}

/// Result of attempting to acquire the daemon single-instance bus lock.
pub enum InstanceAcquire {
    /// This process owns the bus name and must keep [`SingleInstanceLock`] alive.
    Acquired(SingleInstanceLock),
    /// Another daemon instance already holds the bus name.
    AlreadyRunning,
}

/// Guard holding the D-Bus session connection and well-known bus name.
pub struct SingleInstanceLock {
    conn: Connection,
}

impl SingleInstanceLock {
    /// Attempts to acquire `bus_name` on the current D-Bus session bus.
    pub fn acquire(bus_name: &str) -> zbus::Result<InstanceAcquire> {
        let bus_name = bus_name.to_string();
        block_on(async move {
            let conn = zbus::connection::Builder::session()
                .serve_at(KESTIC_OBJECT_PATH, KesticDbusService)
                .build()
                .await?;

            match conn
                .request_name_with_flags(bus_name.as_str(), RequestNameFlags::DO_NOT_QUEUE)
                .await
            {
                Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
                    Ok(InstanceAcquire::Acquired(Self { conn }))
                }
                Ok(RequestNameReply::Exists | RequestNameReply::InQueue)
                | Err(zbus::Error::NameTaken) => Ok(InstanceAcquire::AlreadyRunning),
                Err(err) => Err(err),
            }
        })
    }

    /// Reference to the underlying D-Bus session connection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}
