// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Private session bus for tests: a `dbus-daemon` without service activation and a wait for a
//! bus name to get an owner.

use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Configuration of the private session bus (no service activation, so nothing on the host is
/// started on demand).
pub const PRIVATE_SESSION_BUS_CONFIG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/private-session-bus.xml"
);

/// Private `dbus-daemon --session --fork` instance (no service activation) killed on `Drop`.
pub struct PrivateDbusGuard {
    pub address: String,
    pub pid: u32,
}

impl PrivateDbusGuard {
    pub fn start() -> Self {
        let out = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", PRIVATE_SESSION_BUS_CONFIG))
            .args(["--fork", "--print-address=1", "--print-pid=1"])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn private dbus-daemon");
        assert!(
            out.status.success(),
            "dbus-daemon failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).expect("utf8 dbus-daemon output");
        let mut lines = stdout.lines();
        let address = lines
            .next()
            .expect("dbus-daemon address line")
            .trim()
            .to_string();
        let pid: u32 = lines
            .next()
            .expect("dbus-daemon pid line")
            .trim()
            .parse()
            .expect("parse dbus-daemon pid");
        Self { address, pid }
    }
}

impl Drop for PrivateDbusGuard {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Whether `name` has an owner on the bus at `address`.
fn name_has_owner(address: &str, name: &str) -> zbus::Result<bool> {
    zbus::block_on(async {
        let conn = zbus::connection::Builder::address(address).build().await?;
        let proxy = zbus::fdo::DBusProxy::new(&conn).await?;
        let bus_name = zbus::names::BusName::try_from(name)?;
        proxy
            .name_has_owner(bus_name)
            .await
            .map_err(zbus::Error::from)
    })
}

/// Waits until `name` has an owner on the bus at `address`; panics when `owner` exits first or
/// `timeout` passes.
pub fn wait_for_name_owner(address: &str, name: &str, owner: &mut Child, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = owner.try_wait().expect("try_wait bus name owner") {
            panic!("process expected to own `{name}` exited early with {status}");
        }
        if matches!(name_has_owner(address, name), Ok(true)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for D-Bus owner `{name}`"
        );
        thread::sleep(Duration::from_millis(20));
    }
}
