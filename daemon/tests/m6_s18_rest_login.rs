// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S18 over D-Bus against an in-process daemon on an isolated session bus and a local
//! `rest-server`: a REST address with a login in it is refused before restic starts or anything
//! is saved; an address without a login is added, and a config saved with a login still loads.

mod common;

use common::m3::{M3Rig, assert_dbus_error_name, connect_rest_repo, daemon_paths, latest_variant};
use common::{isolated_secret_store, run_isolated_test};
use kestic_core::backend_credentials::SECRET_KIND_REST;
use kestic_core::model::{Destination, DestinationId, DestinationKind};
use kestic_core::restic::SECRET_KIND_REPO;
use kestic_core::secrets::SecretStore;
use kestic_daemon::dbus_api::BackendSecrets;
use kestic_i18n::codes::ConnectionTestCode;
use kestic_i18n::messages::REST_ADDRESS_WITH_LOGIN;
use kestic_test_support::password::{s3_key, strong_password};
use kestic_test_support::tools::RestServerGuard;
use std::fs;
use std::path::Path;
use time::macros::datetime;

fn rest(url: &str) -> Destination {
    Destination::new(
        "NAS",
        DestinationKind::Rest {
            url: url.to_string(),
            username: None,
        },
    )
    .expect("REST destination")
}

/// A daemon without scheduler and system monitors behind a restic wrapper, in the sandbox `base`.
fn start_rig(root: &Path, base: &str) -> M3Rig {
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

/// The text of a refused D-Bus call.
fn error_text(err: &zbus::Error) -> String {
    match err {
        zbus::Error::MethodError(_, Some(text), _) => text.clone(),
        other => panic!("expected a D-Bus MethodError with a text, got {other:?}"),
    }
}

/// AC-M6-18-1: `AddDestination`, `UpdateDestination` and `TestDestinationConnection` with a REST
/// address that carries a login (a) refuse it with the message that points to the fields
/// «Username» and «Server password», (b) start no restic for it, and (c) save neither the config
/// nor a secret.
#[test]
fn ac_m6_18_1_login_in_rest_address_is_refused() {
    run_isolated_test(
        "ac_m6_18_1_login_in_rest_address_is_refused",
        "KESTIC_M6_18_1_INNER",
        |root| {
            let rig = start_rig(root, "M6R18A");
            let client = &rig.daemon.client;
            let server = RestServerGuard::start_no_auth(&root.join("rest-data"));
            let saved = rest(&server.url);
            connect_rest_repo(client, &rig.variant.bin, &server, &saved);

            let user = s3_key(12);
            let login = format!("{user}:{}", s3_key(24));
            let at_server = |path: &str| format!("http://{login}@127.0.0.1:{}{path}", server.port);
            let new_dest = rest(&at_server("/second"));
            let mut edited = saved.clone();
            edited.kind = DestinationKind::Rest {
                url: at_server("/"),
                username: None,
            };
            let repo_password = strong_password();
            let server_password = BackendSecrets::rest(&strong_password());
            let config_before = fs::read(&rig.daemon.config_path).expect("config");

            let added = client
                .add_destination(&new_dest, "init", &repo_password, &server_password)
                .expect_err("AddDestination refuses a login in the address");
            assert_dbus_error_name(&added, "net.daester.Kestic.Error.InvalidInput");
            assert_eq!(
                error_text(&added),
                REST_ADDRESS_WITH_LOGIN,
                "(a) AddDestination"
            );

            let updated = client
                .update_destination(&edited, "connect", "", &server_password, true)
                .expect_err("UpdateDestination refuses a login in the address");
            assert_dbus_error_name(&updated, "net.daester.Kestic.Error.InvalidInput");
            assert_eq!(
                error_text(&updated),
                REST_ADDRESS_WITH_LOGIN,
                "(a) UpdateDestination"
            );

            for (dest, mode) in [(&new_dest, "init"), (&edited, "connect")] {
                let tested = client
                    .test_destination_connection(dest, mode, &repo_password, &server_password)
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::InvalidInput),
                    "(a) TestDestinationConnection ({mode}): {tested:?}"
                );
                assert_eq!(
                    tested.detail, REST_ADDRESS_WITH_LOGIN,
                    "(a) TestDestinationConnection ({mode})"
                );
            }

            let restic_calls: Vec<_> = rig
                .wrapper
                .entries()
                .into_iter()
                .filter(|e| e.kind == 'S' && e.detail.contains(&user))
                .collect();
            assert!(
                restic_calls.is_empty(),
                "(b) restic started for the address with a login: {} calls",
                restic_calls.len()
            );

            assert_eq!(
                fs::read(&rig.daemon.config_path).expect("config"),
                config_before,
                "(c) config unchanged"
            );
            let store = isolated_secret_store();
            for (id, kind) in [
                (&new_dest.id, SECRET_KIND_REPO),
                (&new_dest.id, SECRET_KIND_REST),
                (&saved.id, SECRET_KIND_REST),
            ] {
                assert!(
                    store
                        .get_secret(id.as_str(), kind)
                        .expect("read secret")
                        .is_none(),
                    "(c) no secret `{kind}` stored for {id}"
                );
            }
            rig.daemon.stop();
        },
    );
}

/// AC-M6-18-3 over D-Bus: (b) the daemon starts with a config whose REST address carries a login
/// and lists that destination, and (a) `AddDestination` adds a REST address without a login.
#[test]
fn ac_m6_18_3_dbus_address_without_login_and_old_config() {
    run_isolated_test(
        "ac_m6_18_3_dbus_address_without_login_and_old_config",
        "KESTIC_M6_18_3_INNER",
        |root| {
            let base = "M6R18B";
            let (config_path, _) = daemon_paths(root, &latest_variant().sub(base));
            let login = format!("{}:{}", s3_key(12), s3_key(24));
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../core/tests/fixtures/m6/config_rest_login.toml");
            let raw = fs::read_to_string(fixture).expect("read fixture");
            fs::create_dir_all(config_path.parent().expect("config dir")).expect("mkdir config");
            fs::write(&config_path, raw.replace("@LOGIN@", &login)).expect("write config");

            let rig = start_rig(root, base);
            let client = &rig.daemon.client;
            let old_id = DestinationId::parse("rstlogin").expect("id");
            let state = client.get_state().expect("GetState");
            let old = state
                .destinations
                .iter()
                .find(|d| d.destination.id == old_id)
                .expect("(b) the destination of the loaded config is listed");
            assert_eq!(
                old.destination.kind,
                DestinationKind::Rest {
                    url: format!("http://{login}@127.0.0.1:9/kestic"),
                    username: None,
                },
                "(b) its address as saved"
            );

            let server = RestServerGuard::start_no_auth(&root.join("rest-data"));
            let new_dest = rest(&format!("{}/second", server.url));
            let new_id = client
                .add_destination(
                    &new_dest,
                    "init",
                    &strong_password(),
                    &BackendSecrets::default(),
                )
                .expect("(a) AddDestination accepts an address without a login");
            let state = client.get_state().expect("GetState");
            assert!(
                state
                    .destinations
                    .iter()
                    .any(|d| d.destination.id.as_str() == new_id
                        && d.destination.kind == new_dest.kind),
                "(a) the destination is added"
            );
            rig.daemon.stop();
        },
    );
}
