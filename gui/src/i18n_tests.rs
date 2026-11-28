// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `i18n` (kept in their own file, NFR-12 file size).

use super::*;
use kestic_daemon::dbus_api::*;

/// #203, GUI side (German «25 %»: smoke scene `m3-overview-progress-percent`):
/// (a) `format_percent` puts the number from `QLocale` into the catalog's pattern, English «25%»
/// without a catalog, and (b) a message with a percentage is translated through it.
#[test]
fn m5_203_gui_percent_goes_through_format_percent() {
    let count = crate::qt_bridge::ffi::format_count(25);
    // (a)
    assert_eq!(format_percent(25), format!("{count}%"), "(a)");
    // (b)
    let message =
        kestic_i18n::messages::active_run_message(&kestic_i18n::messages::ActiveRunInfo {
            name: "Docs",
            kind: kestic_i18n::codes::RunKindCode::Backup,
            phase: RunProgressPhase::ScanFinished,
            percent_done: 25.0,
        });
    assert_eq!(tr(&message), format!("Docs: backing up · {count}%"), "(b)");
}

/// #203, GUI side: the number of a percentage comes from `QLocale`; in an Arabic locale it has
/// Arabic-Indic digits. `QLocale` reads the locale once from the environment, so the check runs
/// in a child process of this test binary with `LC_ALL` set.
#[test]
fn m5_203_gui_percent_number_comes_from_qlocale() {
    const CHILD: &str = "KESTIC_TEST_PERCENT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        assert_eq!(
            format_percent(25),
            "\u{662}\u{665}%",
            "(a) Arabic-Indic digits"
        );
        return;
    }
    let exe = std::env::current_exe().expect("test binary");
    let output = std::process::Command::new(exe)
        .args([
            "--exact",
            "state_cache::tests::m5_203_gui_percent_number_comes_from_qlocale",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env("LC_ALL", "ar_EG.UTF-8")
        .env("LANGUAGE", "en")
        .output()
        .expect("child test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "(a) child failed: {stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("1 passed"),
        "(a) child ran the check: {stdout}"
    );
}
