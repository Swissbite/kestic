// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! D-Bus single-instance lock for the Kestic daemon (F14, AC-M0-07-1, AC-M1-09-1).
//!
//! Claims the session bus name `net.daester.Kestic` with `DO_NOT_QUEUE` and
//! without `ALLOW_REPLACEMENT` or `REPLACE_EXISTING` so a second daemon
//! immediately observes `NameExists` / `AlreadyRunning` and exits cleanly.

pub use crate::dbus_api::{KESTIC_BUS_NAME, KESTIC_OBJECT_PATH};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::Interface;
use zbus::{Connection, block_on, interface};

struct MinimalKesticDbusService;

#[interface(name = "net.daester.Kestic1")]
impl MinimalKesticDbusService {
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
    /// Attempts to acquire `bus_name` on the current D-Bus session bus with minimal v1 interface.
    pub fn acquire(bus_name: &str) -> zbus::Result<InstanceAcquire> {
        Self::acquire_with_interface(bus_name, MinimalKesticDbusService)
    }

    /// Attempts to acquire `bus_name` serving a caller-provided D-Bus interface object at [`KESTIC_OBJECT_PATH`].
    pub fn acquire_with_interface<I>(bus_name: &str, service: I) -> zbus::Result<InstanceAcquire>
    where
        I: Interface,
    {
        let bus_name = bus_name.to_string();
        block_on(async move {
            let conn = zbus::connection::Builder::session()
                .serve_at(KESTIC_OBJECT_PATH, service)
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
