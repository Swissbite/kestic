// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S19 over D-Bus against an in-process daemon on an isolated session bus: an SFTP path with a
//! `..` folder in it is refused before restic starts or anything is saved, and a config saved with
//! such a path still loads.

mod common;

use common::m3::{M3Rig, assert_dbus_error_name, daemon_paths, latest_variant};
use common::revision::destination_revision;
use common::run_isolated_test;
use kestic_core::model::{Destination, DestinationId, DestinationKind};
use kestic_daemon::dbus_api::BackendSecrets;
use kestic_i18n::codes::ConnectionTestCode;
use kestic_i18n::messages::SFTP_PATH_WITH_PARENT;
use kestic_test_support::password::strong_password;
use std::fs;
use std::path::Path;
use time::macros::datetime;

const PARENT_PATH: &str = "/srv/x/../repo";

/// The fixture config: one SFTP destination `sftpdots` on `backup.invalid` with the path
/// `/srv/x/../repo`.
fn fixture() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/tests/fixtures/m6/config_sftp_parent.toml");
    fs::read_to_string(path).expect("read fixture")
}

/// A daemon without scheduler and system monitors behind a restic wrapper, in the sandbox `base`,
/// started on the config `config`.
fn start_rig(root: &Path, base: &str, config: &str) -> M3Rig {
    let (config_path, _) = daemon_paths(root, &latest_variant().sub(base));
    fs::create_dir_all(config_path.parent().expect("config dir")).expect("mkdir config");
    fs::write(&config_path, config).expect("write config");
    M3Rig::start(
        root,
        &latest_variant(),
        base,
        datetime!(2026-12-01 10:00:00 UTC),
        |opts| {
            opts.enable_scheduler = false;
            opts.enable_monitors = false;
        },
    )
}

fn sftp_kind(path: &str) -> DestinationKind {
    DestinationKind::Sftp {
        host: "backup.invalid".to_string(),
        port: 22,
        user: "anna".to_string(),
        path: path.to_string(),
    }
}

/// The text of a refused D-Bus call.
fn error_text(err: &zbus::Error) -> String {
    match err {
        zbus::Error::MethodError(_, Some(text), _) => text.clone(),
        other => panic!("expected a D-Bus MethodError with a text, got {other:?}"),
    }
}

/// AC-M6-19-1: `AddDestination` and `UpdateDestination` with the SFTP path `/srv/x/../repo` (a)
/// refuse it with the message, and (b) start no restic for it; nothing is saved.
/// `TestDestinationConnection` refuses it the same way.
#[test]
fn ac_m6_19_1_parent_segment_in_sftp_path_is_refused() {
    run_isolated_test(
        "ac_m6_19_1_parent_segment_in_sftp_path_is_refused",
        "KESTIC_M6_19_1_INNER",
        |root| {
            let rig = start_rig(root, "M6S19A", &fixture().replace(PARENT_PATH, "/srv/repo"));
            let client = &rig.daemon.client;
            let saved_id = DestinationId::parse("sftpdots").expect("id");
            let saved = client
                .get_state()
                .expect("GetState")
                .destinations
                .into_iter()
                .find(|d| d.destination.id == saved_id)
                .expect("the saved SFTP destination")
                .destination;
            let new_dest = Destination::new("Server 2", sftp_kind(PARENT_PATH)).expect("SFTP");
            let mut edited = saved.clone();
            edited.kind = sftp_kind(PARENT_PATH);
            let repo_password = strong_password();
            let no_secrets = BackendSecrets::default();
            let config_before = fs::read(&rig.daemon.config_path).expect("config");

            let added = client
                .add_destination(&new_dest, "init", &repo_password, &no_secrets)
                .expect_err("AddDestination refuses a `..` folder");
            assert_dbus_error_name(&added, "net.daester.Kestic.Error.InvalidInput");
            assert_eq!(
                error_text(&added),
                SFTP_PATH_WITH_PARENT,
                "(a) AddDestination"
            );

            let revision = destination_revision(client, &saved);
            let updated = client
                .update_destination(&edited, "connect", "", &no_secrets, true, &revision)
                .expect_err("UpdateDestination refuses a `..` folder");
            assert_dbus_error_name(&updated, "net.daester.Kestic.Error.InvalidInput");
            assert_eq!(
                error_text(&updated),
                SFTP_PATH_WITH_PARENT,
                "(a) UpdateDestination"
            );

            for (dest, mode) in [(&new_dest, "init"), (&edited, "connect")] {
                let tested = client
                    .test_destination_connection(dest, mode, &repo_password, &no_secrets)
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::InvalidInput),
                    "TestDestinationConnection ({mode}): {tested:?}"
                );
                assert_eq!(
                    tested.detail, SFTP_PATH_WITH_PARENT,
                    "TestDestinationConnection ({mode})"
                );
            }

            let restic_calls: Vec<_> = rig
                .wrapper
                .entries()
                .into_iter()
                .filter(|e| e.kind == 'S' && e.detail.contains("backup.invalid"))
                .collect();
            assert!(
                restic_calls.is_empty(),
                "(b) restic started for the SFTP destination: {restic_calls:?}"
            );
            assert_eq!(
                fs::read(&rig.daemon.config_path).expect("config"),
                config_before,
                "(b) config unchanged"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M6-19-3 over D-Bus: (a) the daemon starts with a config whose SFTP path has a `..` folder
/// and lists that destination with its path as saved.
#[test]
fn ac_m6_19_3_dbus_config_with_parent_segment_loads() {
    run_isolated_test(
        "ac_m6_19_3_dbus_config_with_parent_segment_loads",
        "KESTIC_M6_19_3_INNER",
        |root| {
            let rig = start_rig(root, "M6S19B", &fixture());
            let id = DestinationId::parse("sftpdots").expect("id");
            let state = rig.daemon.client.get_state().expect("GetState");
            let dest = state
                .destinations
                .iter()
                .find(|d| d.destination.id == id)
                .expect("(a) the destination of the loaded config is listed");
            assert_eq!(
                dest.destination.kind,
                sftp_kind(PARENT_PATH),
                "(a) its path as saved"
            );
            rig.daemon.stop();
        },
    );
}
