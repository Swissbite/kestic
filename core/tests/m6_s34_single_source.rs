// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S34, read from the source tree: the editor pages leave their domain logic to `core`, the
//! GUI layer has no API nothing uses, `state_cache.rs` holds only the cache, and the daemon
//! shares its progress tail and state push and waits for JobView without polling.

use kestic_test_support::workspace::workspace_root;
use std::fs;
use std::path::Path;

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// The files below `rel` with extension `ext`, as (path, text).
fn files(rel: &str, ext: &str) -> Vec<(String, String)> {
    fn walk(dir: &Path, ext: &str, out: &mut Vec<(String, String)>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, ext, out);
            } else if path.extension().is_some_and(|x| x == ext) {
                let text = fs::read_to_string(&path).expect("read source");
                out.push((path.to_string_lossy().into_owned(), text));
            }
        }
    }
    let mut out = Vec::new();
    walk(&workspace_root().join(rel), ext, &mut out);
    out.sort();
    out
}

/// Whether `text` contains `word` with no identifier character on either side.
fn has_word(text: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// Whether QML `text` reads `name`: an occurrence that is not the target of an assignment.
fn read_in_qml(text: &str, name: &str) -> bool {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(name).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let rest = &text[i + name.len()..];
        let assigned = rest.trim_start().starts_with('=') && !rest.trim_start().starts_with("==");
        !before.is_some_and(is_ident) && !rest.starts_with(is_ident) && !assigned
    })
}

/// The names of the functions a QML file declares.
fn qml_functions(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix("function "))
        .filter_map(|rest| rest.split('(').next())
        .map(|name| name.trim().to_string())
        .collect()
}

/// AC-M6-34-1 (b): the retention page reads, normalises and matches no policy itself; it asks
/// `core` through the GUI layer.
#[test]
fn ac_m6_34_1_retention_page_has_no_domain_logic() {
    let page = read("gui/src/qml/BackupRetentionPage.qml");
    let functions = qml_functions(&page);
    assert!(
        !functions.iter().any(|f| f == "parseDuration"),
        "(b) no parseDuration: {functions:?}"
    );
    assert!(
        !page.to_lowercase().contains("normaliz"),
        "(b) no normalisation in the retention page"
    );
    for gone in [
        "applyKeepCountField",
        "encodeKeepCount",
        "detectPresetFromFields",
    ] {
        assert!(!has_word(&page, gone), "(b) {gone} is still in the page");
    }
    for call in ["retentionFormJson", "retentionFormState"] {
        assert!(
            page.contains(&format!("appBackend.{call}(")),
            "(b) the page calls {call}"
        );
    }
}

/// AC-M6-34-2 (b): schedule, exclude rules and maintenance keep no domain functions of their
/// own (reading, encoding, normalising, comparing, parsing restic output) and call `core`
/// through the GUI layer.
#[test]
fn ac_m6_34_2_editor_pages_have_no_domain_logic() {
    let pages = [
        (
            "gui/src/qml/BackupScheduleEditPage.qml",
            &[
                "isoWeekdayToKey",
                "parseIsoDateOrToday",
                "currentScheduleObj",
                "normalizedScheduleJson",
            ][..],
            &["scheduleFormJson", "scheduleFormState"][..],
        ),
        (
            "gui/src/qml/BackupExcludesEditPage.qml",
            &[
                "parseSizeLimit",
                "splitNonEmptyLines",
                "splitMarkers",
                "currentExcludesObj",
                "canonicalExcludes",
            ][..],
            &["excludeFormJson", "excludeFormState", "excludeLintFix"][..],
        ),
        (
            "gui/src/qml/DestinationMaintenancePage.qml",
            &[
                "buildEntryObj",
                "normalizeEntry",
                "normalizedSavedObj",
                "currentMaintObj",
                "formatPreviewSummary",
            ][..],
            &[
                "maintenanceFormJson",
                "maintenanceFormState",
                "maintenancePreviewText",
            ][..],
        ),
    ];
    for (path, gone, calls) in pages {
        let page = read(path);
        for name in gone {
            assert!(!has_word(&page, name), "(b) {path} still has {name}");
        }
        assert!(!page.contains(".exec("), "(b) {path} parses with a regex");
        let normalising: Vec<String> = qml_functions(&page)
            .into_iter()
            .filter(|f| f.to_lowercase().contains("normaliz"))
            .collect();
        assert!(
            normalising.is_empty(),
            "(b) {path} normalises: {normalising:?}"
        );
        for call in calls {
            assert!(
                page.contains(&format!("appBackend.{call}(")),
                "(b) {path} calls {call}"
            );
        }
    }
}

/// AC-M6-34-3 (a): the GUI takes the interval range and the default of «every n days» from
/// `core` instead of spelling them itself.
#[test]
fn ac_m6_34_3_interval_values_come_from_core() {
    let spelled: Vec<String> = files("gui/src", "rs")
        .into_iter()
        .filter(|(_, text)| text.contains("2..=365") || text.contains("365 days"))
        .map(|(path, _)| path)
        .collect();
    assert!(
        spelled.is_empty(),
        "(a) the GUI spells the range in {spelled:?}"
    );
    assert!(
        read("gui/src/backend/schedules.rs").contains("EVERY_N_DAYS_RANGE"),
        "(a) the field message takes the range from core"
    );
    let page = read("gui/src/qml/BackupScheduleEditPage.qml");
    assert!(
        page.contains("to: page.formState.interval_days_max"),
        "(a) the interval field ends where core's range ends"
    );
    assert!(
        !page.contains("to: 365"),
        "(a) the page spells the range end"
    );
    assert!(
        !page.contains("intervalDays: 3"),
        "(a) the page spells the default interval"
    );
}

/// AC-M6-34-4 (a): the invokables nobody calls stay gone.
#[test]
fn ac_m6_34_4_dead_invokables_are_gone() {
    let sources: Vec<(String, String)> = files("gui/src", "rs")
        .into_iter()
        .chain(files("gui/src", "qml"))
        .chain(files("gui/src", "cpp"))
        .collect();
    for gone in [
        "updateBackup",
        "clone_updated_backup",
        "buildExcludeFilePreview",
        "build_exclude_file_preview",
        "formatScheduleSummaryJson",
        "formatConditionsSummaryJson",
        "formatRetentionSummaryJson",
    ] {
        let hits: Vec<&str> = sources
            .iter()
            .filter(|(_, text)| has_word(text, gone))
            .map(|(path, _)| path.as_str())
            .collect();
        assert!(hits.is_empty(), "(a) {gone} is still in {hits:?}");
    }
}

/// AC-M6-34-4 (a): every property of `KesticApp` is read by QML, by the smoke tests or by the
/// GUI's Rust code; a property only written is gone.
#[test]
fn ac_m6_34_4_every_qproperty_is_read() {
    let backend = read("gui/src/backend.rs");
    let properties: Vec<&str> = backend
        .lines()
        .filter_map(|l| l.trim().strip_prefix("#[qproperty("))
        .filter_map(|rest| rest.split(',').nth(1))
        .map(|name| name.trim().trim_end_matches(")]").trim())
        .collect();
    assert!(
        properties.len() > 50,
        "found the properties: {properties:?}"
    );
    let qml = files("gui/src/qml", "qml");
    let cpp: Vec<(String, String)> = files("gui/src", "cpp")
        .into_iter()
        .chain(files("gui/src", "h"))
        .collect();
    let rust: Vec<(String, String)> = files("gui/src", "rs")
        .into_iter()
        .filter(|(path, _)| !path.ends_with("gui/src/backend.rs"))
        .collect();
    let unread: Vec<&str> = properties
        .iter()
        .copied()
        .filter(|name| {
            let in_qml = qml.iter().any(|(_, text)| read_in_qml(text, name));
            let in_cpp = cpp
                .iter()
                .any(|(_, text)| text.contains(&format!("\"{name}\"")));
            let in_rust = rust
                .iter()
                .any(|(_, text)| text.contains(&format!(".{name}()")));
            !(in_qml || in_cpp || in_rust)
        })
        .collect();
    assert!(
        unread.is_empty(),
        "(a) properties nothing reads: {unread:?}"
    );
}

/// AC-M6-34-5 (a) and (b): `state_cache.rs` holds only the cache; the KI18n wrapper, the daemon
/// client, the DTO builders and the `format_*` texts have their own modules.
#[test]
fn ac_m6_34_5_state_cache_holds_only_the_cache() {
    let cache = read("gui/src/state_cache.rs");
    for gone in [
        "fn i18n(",
        "struct PendingI18n",
        "fn tr(",
        "fn gui_connection(",
        "fn make_client(",
        "fn ensure_daemon_and_refresh(",
        "fn refresh_state_now(",
        "fn build_new_backup(",
        "fn clone_updated_backup_",
        "fn build_provider(",
        "fn format_",
        "fn localized_",
        "fn json_or(",
    ] {
        assert!(
            !cache.contains(gone),
            "(a) state_cache.rs still has `{gone}`"
        );
    }
    for kept in ["pub struct SharedGuiState", "pub fn shared_state("] {
        assert!(cache.contains(kept), "(a) the cache keeps `{kept}`");
    }
    let lines = cache.lines().count();
    assert!(
        lines <= 300,
        "(a) state_cache.rs has {lines} lines; the cache alone is small"
    );

    for (module, item) in [
        ("gui/src/i18n.rs", "pub struct PendingI18n"),
        ("gui/src/i18n.rs", "pub fn tr("),
        ("gui/src/daemon_client.rs", "pub fn make_client("),
        (
            "gui/src/daemon_client.rs",
            "pub fn ensure_daemon_and_refresh(",
        ),
        ("gui/src/dto_builders.rs", "pub fn build_new_backup("),
        ("gui/src/dto_builders.rs", "pub fn build_provider("),
        (
            "gui/src/summary_texts.rs",
            "pub fn format_retention_summary(",
        ),
        (
            "gui/src/summary_texts.rs",
            "pub fn format_schedule_summary(",
        ),
        ("gui/src/status_texts.rs", "pub fn format_backup_status("),
        ("gui/src/status_texts.rs", "pub fn format_run_status("),
    ] {
        assert!(read(module).contains(item), "(b) {module} has `{item}`");
    }
}

/// AC-M6-34-6 (b): the JobView cancel watcher waits for its message or its stop, with no loop
/// that sleeps; (a) backup and restore share one progress tail, and only the state helpers push
/// the tray state.
#[test]
fn ac_m6_34_6_job_view_has_no_sleep_loop() {
    let job_view = read("daemon/src/job_view.rs");
    assert!(!job_view.contains("sleep"), "(b) JobView sleeps");
    assert!(
        job_view.contains("until_stopped("),
        "(b) the cancel watcher waits with StopSignal::until_stopped"
    );

    for runner in [
        "daemon/src/service/backup_runner.rs",
        "daemon/src/service/restore_runner.rs",
    ] {
        let text = read(runner);
        assert!(
            text.contains("publish_run_progress("),
            "(a) {runner} uses the shared progress tail"
        );
        assert!(
            !text.contains("emit_run_progress"),
            "(a) {runner} sends RunProgress itself"
        );
    }

    let by_hand: Vec<String> = files("daemon/src", "rs")
        .into_iter()
        .filter(|(path, text)| {
            !path.ends_with("service/state_summary.rs")
                && !path.ends_with("daemon/src/tray.rs")
                && text.contains(".set_full_state(")
        })
        .map(|(path, _)| path)
        .collect();
    assert!(
        by_hand.is_empty(),
        "(a) the tray state is pushed by hand in {by_hand:?}"
    );
}
