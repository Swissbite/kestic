// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S19 over D-Bus against an in-process daemon on an isolated session bus and a test `sshd`:
//! an SFTP path with a `..` folder in it is refused before restic starts or anything is saved, and
//! a config saved with such a path still loads.

mod common;

use common::m3::{
    M3DaemonOpts, M3Rig, ResticWrapper, assert_dbus_error_name, daemon_paths, latest_variant,
    spawn_m3_daemon,
};
use common::revision::destination_revision;
use common::run_isolated_test;
use kestic_core::model::{Destination, DestinationId, DestinationKind};
use kestic_core::sftp::set_test_ssh_config;
use kestic_daemon::dbus_api::BackendSecrets;
use kestic_i18n::codes::ConnectionTestCode;
use kestic_i18n::messages::SFTP_PATH_WITH_PARENT;
use kestic_test_support::password::strong_password;
use kestic_test_support::sftp_server::SftpServerGuard;
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

/// The restic calls the wrapper logged so far.
fn restic_starts(wrapper: &ResticWrapper) -> usize {
    wrapper.entries().iter().filter(|e| e.kind == 'S').count()
}

/// AC-M6-19-1: against a test `sshd` whose host key is trusted, `AddDestination` and
/// `UpdateDestination` with an SFTP path that has a `..` folder (a) refuse it with the message,
/// and (b) start no restic for it, while the same server with a plain path does start restic;
/// nothing is saved. `TestDestinationConnection` refuses it the same way.
#[test]
fn ac_m6_19_1_parent_segment_in_sftp_path_is_refused() {
    run_isolated_test(
        "ac_m6_19_1_parent_segment_in_sftp_path_is_refused",
        "KESTIC_M6_19_1_INNER",
        |root| {
            let variant = latest_variant();
            let sub = variant.sub("M6S19A");
            let dir = root.join(&sub);
            let server = SftpServerGuard::start(&dir.join("sshd"));
            set_test_ssh_config(Some(server.ssh_config.clone()));
            let wrapper = ResticWrapper::create(&dir.join("wrapper"), &variant.bin);
            let daemon = spawn_m3_daemon(
                root,
                M3DaemonOpts::on_request(&sub, &variant.bus_name("M6S19A"), &wrapper.path),
            );
            let client = &daemon.client;
            let at = |path: &Path| {
                Destination::new(
                    "Server",
                    DestinationKind::Sftp {
                        host: "127.0.0.1".to_string(),
                        port: server.port,
                        user: server.user.clone(),
                        path: path.to_string_lossy().into_owned(),
                    },
                )
                .expect("SFTP destination")
            };
            let no_secrets = BackendSecrets::default();
            let repo_password = strong_password();

            let saved = at(&dir.join("repo"));
            let tested = client
                .test_destination_connection(&saved, "init", "", &no_secrets)
                .expect("TestDestinationConnection");
            let key = tested.host_key.expect("host key of the unknown server");
            client
                .trust_host_key(&key.host_key_id)
                .expect("TrustHostKey");
            let starts = restic_starts(&wrapper);
            client
                .add_destination(&saved, "init", &repo_password, &no_secrets)
                .expect("AddDestination with a plain path");
            assert!(
                restic_starts(&wrapper) > starts,
                "the plain path starts restic on this server"
            );

            let parent = dir.join("x").join("..").join("second");
            let new_dest = at(&parent);
            let mut edited = saved.clone();
            edited.kind = new_dest.kind.clone();
            let config_before = fs::read(&daemon.config_path).expect("config");
            let starts = restic_starts(&wrapper);

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

            assert_eq!(
                restic_starts(&wrapper),
                starts,
                "(b) restic started for the path with a `..` folder: {:?}",
                wrapper.entries()
            );
            assert_eq!(
                fs::read(&daemon.config_path).expect("config"),
                config_before,
                "(b) config unchanged"
            );
            daemon.stop();
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
