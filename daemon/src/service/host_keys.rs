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
use std::collections::BTreeMap;
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

/// After `removed` left the configuration: when no destination in `remaining` uses its SFTP
/// server (host and port) any more, removes the server's lines from Kestic's `known_hosts`.
/// `~/.ssh/known_hosts` is never written.
pub(super) fn forget_unused_host(removed: &Destination, remaining: &[Destination]) {
    let DestinationKind::Sftp {
        host, port, user, ..
    } = &removed.kind
    else {
        return;
    };
    let server = ssh_host(host).to_ascii_lowercase();
    let name = known_hosts_name(host, *port, user);
    // Another destination on the same host and port, or one ssh resolves to the same
    // `known_hosts` name (an alias of the server), still needs the line.
    let still_used = remaining.iter().any(|d| match &d.kind {
        DestinationKind::Sftp {
            host: h,
            port: p,
            user: u,
            ..
        } => {
            (*p == *port && ssh_host(h).to_ascii_lowercase() == server)
                || known_hosts_name(h, *p, u) == name
        }
        _ => false,
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
