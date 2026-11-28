// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S18 offscreen GUI smoke test against an in-process daemon on an isolated session bus: the
//! REST form refuses an address with a login in it.

mod common;

use common::m3::{FakeSetup, M3Rig, StdFakes, connect_rest_repo, latest_variant};
use common::run_isolated_test;
use kestic_core::model::{Destination, DestinationKind};
use kestic_daemon::dbus_api::KESTIC_BUS_NAME;
use kestic_test_support::password::s3_key;
use kestic_test_support::tools::RestServerGuard;
use time::macros::datetime;

/// AC-M6-18-2: in «Add Destination» and on the edit page of a REST destination, an address with
/// a login (a) shows the message that points to the fields «Username» and «Server password»
/// below the address field, and (b) keeps «Test Connection» and «Next» or «Save» disabled; the
/// same address without the login enables «Test Connection». The C++ scene checks each part;
/// screenshots of both forms in English and German, wide and narrow. Nothing is saved.
#[test]
fn ac_m6_18_2_gui_login_in_rest_address() {
    run_isolated_test(
        "ac_m6_18_2_gui_login_in_rest_address",
        "KESTIC_M6_18_2_GUI_INNER",
        |root| {
            let _fakes = StdFakes::start_with(FakeSetup::default());
            let rig = M3Rig::start_on(
                root,
                &latest_variant(),
                "",
                KESTIC_BUS_NAME,
                datetime!(2026-12-01 10:00:00 UTC),
                |_| {},
            );
            let server = RestServerGuard::start_no_auth(&root.join("rest-data"));
            let dest = Destination::new(
                "NAS",
                DestinationKind::Rest {
                    url: server.url.clone(),
                    username: None,
                },
            )
            .expect("REST destination");
            connect_rest_repo(&rig.daemon.client, &rig.variant.bin, &server, &dest);
            let config_before = std::fs::read(&rig.daemon.config_path).expect("config");
            let login_url = format!(
                "http://{}:{}@127.0.0.1:{}/",
                s3_key(12),
                s3_key(24),
                server.port
            );

            let iso = kestic_test_support::gui::IsolatedEnv::adopt(root);
            let dbus_addr =
                std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("isolated session bus");
            let smoke = kestic_test_support::gui::SmokeContext {
                kestic_bin: &common::find_kestic_binary(),
                iso: &iso,
                dbus_addr: &dbus_addr,
            };
            for (layout, lang) in [
                ("wide", "en"),
                ("wide", "de"),
                ("narrow", "en"),
                ("narrow", "de"),
            ] {
                smoke.run_with_env(
                    "m6-rest-login",
                    layout,
                    lang,
                    &format!("target/smoke-artifacts/m6-rest-login-wizard-{layout}-{lang}.png"),
                    &[],
                    &[
                        ("KESTIC_SMOKE_REST_URL", &format!("{}/", server.url)),
                        ("KESTIC_SMOKE_REST_LOGIN_URL", &login_url),
                        ("KESTIC_SMOKE_DEST_ID", dest.id.as_str()),
                    ],
                );
            }
            assert_eq!(
                std::fs::read(&rig.daemon.config_path).expect("config"),
                config_before,
                "nothing saved"
            );
            rig.daemon.stop();
        },
    );
}
