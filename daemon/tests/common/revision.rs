// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `UpdateBackup` and `UpdateDestination` with the revision the daemon reports right now, for
//! tests that are not about revisions.

use kestic_core::model::{Backup, Destination};
use kestic_daemon::dbus_api::{BackendSecrets, DestinationUpdateResultDto, KesticClient};

/// The revisions of a backup or destination as `GetState` reports them now.
pub fn backup_revision(client: &KesticClient, backup: &Backup) -> String {
    client
        .get_state()
        .ok()
        .and_then(|s| {
            s.backups
                .into_iter()
                .find(|b| b.backup.id == backup.id)
                .map(|b| b.revision)
        })
        .unwrap_or_default()
}

pub fn destination_revision(client: &KesticClient, dest: &Destination) -> String {
    client
        .get_state()
        .ok()
        .and_then(|s| {
            s.destinations
                .into_iter()
                .find(|d| d.destination.id == dest.id)
                .map(|d| d.revision)
        })
        .unwrap_or_default()
}

/// The save calls with the current revision.
pub trait UpdateNow {
    fn update_backup_now(&self, backup: &Backup) -> zbus::Result<String>;

    fn update_destination_now(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        backend: &BackendSecrets,
        verify_connection: bool,
    ) -> zbus::Result<DestinationUpdateResultDto>;
}

impl UpdateNow for KesticClient {
    fn update_backup_now(&self, backup: &Backup) -> zbus::Result<String> {
        self.update_backup(backup, &backup_revision(self, backup))
    }

    fn update_destination_now(
        &self,
        dest: &Destination,
        mode: &str,
        repo_password: &str,
        backend: &BackendSecrets,
        verify_connection: bool,
    ) -> zbus::Result<DestinationUpdateResultDto> {
        let revision = destination_revision(self, dest);
        self.update_destination(
            dest,
            mode,
            repo_password,
            backend,
            verify_connection,
            &revision,
        )
    }
}
