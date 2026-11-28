// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M6-S33: opening the tray menu never waits for a slow destination check (`AC-M6-33-6`).

mod common;

use common::m3::{M3Rig, StdFakes, daemon_paths, latest_variant};
use common::{run_isolated_test, wait_for_sni_service};
use kestic_core::config::{AppConfig, save_config};
use kestic_core::model::{Destination, DestinationKind};
use kestic_test_support::wait::wait_until_within;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use time::macros::datetime;
use zbus::block_on;

/// Longest `AboutToShow` may take (`AC-M6-33-6`).
const MENU_WITHIN: Duration = Duration::from_millis(500);

/// A REST server that accepts connections and then either closes them at once (unreachable,
/// answered quickly) or holds them without a reply, so a reachability check waits until its
/// own timeout (more than 10 s).
struct SilentServer {
    url: String,
    hold: Arc<AtomicBool>,
    held: Arc<Mutex<Vec<TcpStream>>>,
    held_count: Arc<AtomicUsize>,
}

impl SilentServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind silent server");
        let url = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().expect("local addr").port()
        );
        let hold = Arc::new(AtomicBool::new(false));
        let held = Arc::new(Mutex::new(Vec::new()));
        let held_count = Arc::new(AtomicUsize::new(0));
        {
            let hold = Arc::clone(&hold);
            let held = Arc::clone(&held);
            let held_count = Arc::clone(&held_count);
            thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    if hold.load(Ordering::SeqCst) {
                        held.lock().expect("held streams").push(stream);
                        held_count.fetch_add(1, Ordering::SeqCst);
                    }
                }
            });
        }
        Self {
            url,
            hold,
            held,
            held_count,
        }
    }

    /// From now on connections are held without a reply.
    fn go_silent(&self) {
        self.hold.store(true, Ordering::SeqCst);
    }

    /// Closes the held connections and closes new ones at once again.
    fn release(&self) {
        self.hold.store(false, Ordering::SeqCst);
        self.held.lock().expect("held streams").clear();
    }
}

/// `com.canonical.dbusmenu.AboutToShow(0)` on the tray menu; returns how long the reply took.
fn time_about_to_show(conn: &zbus::Connection, sni_service: &str) -> Duration {
    let started = Instant::now();
    block_on(conn.call_method(
        Some(sni_service),
        "/MenuBar",
        Some("com.canonical.dbusmenu"),
        "AboutToShow",
        &(0i32,),
    ))
    .expect("AboutToShow");
    started.elapsed()
}

/// AC-M6-33-6: with a destination whose check takes more than 10 s, (a) the tray menu opens
/// (`AboutToShow` replies) within 500 ms instead of waiting for the check.
#[test]
fn ac_m6_33_6_tray_menu_does_not_wait_for_destination_check() {
    run_isolated_test(
        "ac_m6_33_6_tray_menu_does_not_wait_for_destination_check",
        "KESTIC_M6_S33_TRAY_MENU_INNER",
        |root| {
            let fakes = StdFakes::start();
            let server = SilentServer::start();
            let variant = latest_variant();
            let sub = variant.sub("TrayMenu");
            let (config_path, _) = daemon_paths(root, &sub);
            let dest = Destination::new(
                "Slow REST",
                DestinationKind::Rest {
                    url: server.url.clone(),
                    username: None,
                },
            )
            .expect("destination");
            save_config(
                &config_path,
                &AppConfig {
                    destinations: vec![dest],
                    ..Default::default()
                },
            )
            .expect("save config");

            let rig = M3Rig::start_on(
                root,
                &variant,
                &sub,
                &variant.bus_name("TrayMenu"),
                datetime!(2026-12-01 09:00:00 UTC),
                |opts| opts.enable_scheduler = false,
            );
            let sni_service = wait_for_sni_service(&fakes.sni_conn, Duration::from_secs(5));

            // The cached check is older than its 60 s lifetime, and the next one waits on the
            // silent server.
            server.go_silent();
            rig.clock.advance(Duration::from_secs(61));
            let elapsed = time_about_to_show(&fakes.sni_conn, &sni_service);

            wait_until_within(
                Duration::from_secs(5),
                "precondition: opening the menu starts the check, which waits on the server",
                || server.held_count.load(Ordering::SeqCst) > 0,
            );
            assert!(
                elapsed < MENU_WITHIN,
                "(a) AboutToShow replied after {elapsed:?}, not within {MENU_WITHIN:?}"
            );

            server.release();
            rig.daemon.stop();
        },
    );
}
