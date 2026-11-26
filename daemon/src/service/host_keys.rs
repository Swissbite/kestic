// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Host keys of SFTP servers that wait for the user's confirmation (`TrustHostKey`), and the
//! lines of Kestic's own `known_hosts` file that go when the last SFTP destination on a server is
//! removed. Only the daemon writes that file.

use super::error::KesticServiceErr;
use super::state::DaemonCore;
use crate::dbus_api::HostKeyDto;
use kestic_core::clock::Clock;
use kestic_core::known_hosts::{HostKeyCheck, ScannedHostKey, forget, known_hosts_name, trust};
use kestic_core::model::{Destination, DestinationKind, RunId};
use kestic_core::sftp::{KnownHostsPaths, ssh_host};
use kestic_core::sync::LockUnpoisoned;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;

/// How long a shown host key can be trusted.
const OFFER_LIFETIME: time::Duration = time::Duration::minutes(10);

struct Offer {
    key: ScannedHostKey,
    expires_at: OffsetDateTime,
}

/// Shown host keys by `host_key_id`; they live only in the daemon's memory.
static OFFERS: Mutex<BTreeMap<String, Offer>> = Mutex::new(BTreeMap::new());

/// Keeps `key` for [`OFFER_LIFETIME`] and returns what the GUI shows of it.
pub(super) fn offer_host_key(key: &ScannedHostKey, clock: &dyn Clock) -> HostKeyDto {
    let now = clock.now_utc();
    let id = RunId::generate().map_or_else(|_| String::new(), |id| id.as_str().to_string());
    let mut offers = OFFERS.lock_unpoisoned();
    offers.retain(|_, offer| offer.expires_at > now);
    if !id.is_empty() {
        offers.insert(
            id.clone(),
            Offer {
                key: key.clone(),
                expires_at: now + OFFER_LIFETIME,
            },
        );
    }
    HostKeyDto {
        host_key_id: id,
        host: key.host.clone(),
        port: key.port,
        key_type: key.key_type.clone(),
        fingerprint_sha256: key.fingerprint_sha256.clone(),
    }
}

/// `TrustHostKey`: writes exactly the key shown under `host_key_id` into Kestic's `known_hosts`.
/// An unknown or expired id is `NotFound`; a key of the same type that some file got meanwhile
/// is a changed key.
pub(crate) fn trust_host_key_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    host_key_id: &str,
) -> Result<(), KesticServiceErr> {
    let clock = Arc::clone(&shared.lock_unpoisoned().clock);
    let now = clock.now_utc();
    let key = {
        let mut offers = OFFERS.lock_unpoisoned();
        offers.retain(|_, offer| offer.expires_at > now);
        offers
            .get(host_key_id)
            .map(|offer| offer.key.clone())
            .ok_or_else(|| KesticServiceErr::NotFound(host_key_id.to_string()))?
    };
    let paths =
        KnownHostsPaths::from_env().map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
    match trust(&paths, &key).map_err(|e| KesticServiceErr::Internal(e.to_string()))? {
        HostKeyCheck::Changed(file) => Err(KesticServiceErr::HostKeyChanged(
            file.to_string_lossy().into_owned(),
        )),
        _ => {
            OFFERS.lock_unpoisoned().remove(host_key_id);
            Ok(())
        }
    }
}

/// SFTP destinations being added or moved to another server right now: their server's lines in
/// Kestic's `known_hosts` were checked and must stay until the destination is saved or given up.
#[derive(Default)]
pub(crate) struct SftpSetups {
    next: u64,
    running: HashMap<u64, Destination>,
}

impl SftpSetups {
    /// Registers `dest` while it is set up; `None` for a destination that is no SFTP one.
    fn begin(&mut self, dest: &Destination) -> Option<u64> {
        if !matches!(dest.kind, DestinationKind::Sftp { .. }) {
            return None;
        }
        self.next += 1;
        self.running.insert(self.next, dest.clone());
        Some(self.next)
    }

    /// Ends the setup that [`SftpSetups::begin`] returned `token` for.
    fn end(&mut self, token: Option<u64>) {
        if let Some(token) = token {
            self.running.remove(&token);
        }
    }
}

/// Keeps a destination registered in [`SftpSetups`] until dropped. Dropping it locks the daemon
/// state, so a function creates it before any other guard of that lock it holds at the same time
/// (locals are dropped in reverse order).
pub(super) struct SftpSetupGuard<'a> {
    shared: &'a Arc<Mutex<DaemonCore>>,
    token: Option<u64>,
}

impl<'a> SftpSetupGuard<'a> {
    /// Registers `dest` in `core`, the locked state of `shared`.
    pub(super) fn begin(
        shared: &'a Arc<Mutex<DaemonCore>>,
        core: &mut DaemonCore,
        dest: &Destination,
    ) -> Self {
        Self {
            shared,
            token: core.sftp_setups.begin(dest),
        }
    }
}

impl Drop for SftpSetupGuard<'_> {
    fn drop(&mut self) {
        if self.token.is_some() {
            self.shared.lock_unpoisoned().sftp_setups.end(self.token);
        }
    }
}

/// The destinations that may need a line of Kestic's `known_hosts`: the saved ones and those
/// being set up.
fn host_key_users(core: &DaemonCore) -> Vec<Destination> {
    core.config
        .destinations
        .iter()
        .chain(core.sftp_setups.running.values())
        .cloned()
        .collect()
}

/// Host, port and user of an SFTP destination.
fn sftp_login(dest: &Destination) -> Option<(&str, u16, &str)> {
    match &dest.kind {
        DestinationKind::Sftp {
            host, port, user, ..
        } => Some((host.as_str(), *port, user.as_str())),
        _ => None,
    }
}

/// After `removed` left the configuration: when no destination uses its SFTP server (host and
/// port) any more, removes the server's lines from Kestic's `known_hosts`. Asking ssh how it
/// resolves the servers may take a while, so it runs without the daemon state; the decision and
/// the removal then hold it, so a destination saved or set up meanwhile keeps its line.
/// `~/.ssh/known_hosts` is never written.
pub(super) fn forget_unused_host(shared: &Arc<Mutex<DaemonCore>>, removed: &Destination) {
    let DestinationKind::Sftp {
        host, port, user, ..
    } = &removed.kind
    else {
        return;
    };
    let server = ssh_host(host).to_ascii_lowercase();
    let same_server = |d: &Destination| {
        sftp_login(d)
            .is_some_and(|(h, p, _)| p == *port && ssh_host(h).to_ascii_lowercase() == server)
    };
    let users = host_key_users(&shared.lock_unpoisoned());
    let name = known_hosts_name(host, *port, user);
    // The `known_hosts` name of every other server: an alias ssh resolves to the same name
    // still needs the line.
    let names: HashMap<(&str, u16, &str), String> = users
        .iter()
        .filter(|d| !same_server(d))
        .filter_map(|d| sftp_login(d).map(|(h, p, u)| ((h, p, u), known_hosts_name(h, p, u))))
        .collect();
    let core = shared.lock_unpoisoned();
    // A server saved or set up since the names were asked counts as the same one.
    let still_used = host_key_users(&core).iter().any(|d| {
        same_server(d)
            || sftp_login(d).is_some_and(|login| names.get(&login).is_none_or(|n| *n == name))
    });
    if still_used {
        return;
    }
    let result = KnownHostsPaths::from_env()
        .map_err(|e| e.to_string())
        .and_then(|paths| forget(&paths, &name).map_err(|e| e.to_string()));
    if let Err(err) = result {
        eprintln!("kestic-daemon: could not remove the host key of {server}: {err}");
    }
}
