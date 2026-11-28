// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S19 offscreen GUI smoke test against an in-process daemon on an isolated session bus: the
//! SFTP form refuses a path with a `..` folder in it.

mod common;

use common::m3::{FakeSetup, M3Rig, StdFakes, daemon_paths, latest_variant};
use common::run_isolated_test;
use kestic_daemon::dbus_api::KESTIC_BUS_NAME;
use std::fs;
use std::path::Path;
use time::macros::datetime;

/// AC-M6-19-2: in «Add Destination» and on the edit page of an SFTP destination, a path with a
/// whole `..` folder (a) shows the message below the path field, and (b) keeps «Test Connection»
/// and «Next» or «Save» disabled; a path with `..` inside a folder name (`a..b`) enables «Test
/// Connection». The C++ scene checks each part; screenshots of both forms in English and German,
/// wide and narrow. Nothing is saved.
#[test]
fn ac_m6_19_2_gui_parent_segment_in_sftp_path() {
    run_isolated_test(
        "ac_m6_19_2_gui_parent_segment_in_sftp_path",
        "KESTIC_M6_19_2_GUI_INNER",
        |root| {
            let _fakes = StdFakes::start_with(FakeSetup::default());
            // A saved SFTP destination on a closed local port with a plain path, for the edit page.
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../core/tests/fixtures/m6/config_sftp_parent.toml");
            let config = fs::read_to_string(fixture)
                .expect("read fixture")
                .replace("backup.invalid", "127.0.0.1")
                .replace("port = 22", "port = 9")
                .replace("/srv/x/../repo", "/srv/repo");
            let (config_path, _) = daemon_paths(root, "");
            fs::create_dir_all(config_path.parent().expect("config dir")).expect("mkdir config");
            fs::write(&config_path, config).expect("write config");
            let rig = M3Rig::start_on(
                root,
                &latest_variant(),
                "",
                KESTIC_BUS_NAME,
                datetime!(2026-12-01 10:00:00 UTC),
                |_| {},
            );
            let config_before = fs::read(&rig.daemon.config_path).expect("config");

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
                    "m6-sftp-parent",
                    layout,
                    lang,
                    &format!("target/smoke-artifacts/m6-sftp-parent-wizard-{layout}-{lang}.png"),
                    &[],
                    &[("KESTIC_SMOKE_DEST_ID", "sftpdots")],
                );
            }
            assert_eq!(
                fs::read(&rig.daemon.config_path).expect("config"),
                config_before,
                "nothing saved"
            );
            rig.daemon.stop();
        },
    );
}
