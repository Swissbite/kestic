// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M5-S07: which restic the source «Automatic», «System» and «Custom path» resolve to, how the
//! version is read, and the migration of `config.toml` from version 3 to 4.

mod common;

use kestic_core::config::{AppConfig, load_optional_config, parse_config_str, save_config};
use kestic_core::model::ResticProvider;
use kestic_core::provider::{
    ResticLocations, ResticProviderError, ResticSurvey, check_binary_version, resolve_with,
};
use kestic_i18n::codes::{ResticPathProblem, ResticStatusCode};
use kestic_i18n::messages::ResticStatusParams;
use kestic_test_support::tools::write_executable;
use semver::Version;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A restic stand-in that answers `version --json` with `version` (restic 0.17.0 and newer).
fn json_stand_in(path: &Path, version: &str) -> PathBuf {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    write_executable(
        path,
        &format!(
            "#!/bin/sh\n\
             if [ \"${{1:-}}\" = version ] && [ \"${{2:-}}\" = --json ]; then\n\
             echo '{{\"message_type\":\"version\",\"version\":\"{version}\"}}'\n\
             exit 0\n\
             fi\n\
             exit 3\n"
        ),
    );
    path.to_path_buf()
}

/// A stand-in that behaves like the real restic 0.16.4: no `--json`, a text line instead.
fn stand_in_0_16_4(path: &Path) -> PathBuf {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    write_executable(
        path,
        "#!/bin/sh\n\
         if [ \"${1:-}\" = version ] && [ \"${2:-}\" = --json ]; then\n\
         echo 'unknown flag: --json' >&2\n\
         exit 1\n\
         fi\n\
         if [ \"${1:-}\" = version ]; then\n\
         echo 'restic 0.16.4 compiled with go1.22.2 on linux/amd64'\n\
         exit 0\n\
         fi\n\
         exit 2\n",
    );
    path.to_path_buf()
}

fn locations(system: Option<PathBuf>, app_managed: PathBuf) -> ResticLocations {
    ResticLocations {
        system,
        app_managed: Some(app_managed),
    }
}

fn version(v: &str) -> Version {
    Version::parse(v).expect("version")
}

/// AC-M5-07-1: «Automatic» with a system restic 0.18.1 and an app-managed 0.19.1 uses the
/// system one.
#[test]
fn ac_m5_07_1_automatic_uses_system_restic() {
    let dir = TempDir::new().expect("tempdir");
    let system = json_stand_in(&dir.path().join("bin/restic"), "0.18.1");
    let app = json_stand_in(&dir.path().join("data/kestic/restic"), "0.19.1");
    let locations = locations(Some(system.clone()), app);

    let resolved = resolve_with(&ResticProvider::Automatic, &locations).expect("resolved");
    // (a) the system restic is used
    assert_eq!(resolved.binary, system);
    assert_eq!(resolved.version, version("0.18.1"));
    // the survey the daemon reports resolves the same way
    let survey = ResticSurvey::probe(&locations);
    assert_eq!(
        survey.resolve(&ResticProvider::Automatic).expect("survey"),
        resolved
    );
}

/// AC-M5-07-2: «Automatic» with a system restic 0.16.4 and an app-managed 0.19.1 uses the
/// app-managed one; so it does without any system restic.
#[test]
fn ac_m5_07_2_automatic_falls_back_to_app_managed() {
    let dir = TempDir::new().expect("tempdir");
    let system = stand_in_0_16_4(&dir.path().join("bin/restic"));
    let app = json_stand_in(&dir.path().join("data/kestic/restic"), "0.19.1");

    let too_old_system = locations(Some(system), app.clone());
    let resolved = resolve_with(&ResticProvider::Automatic, &too_old_system).expect("resolved");
    // (a) the app-managed restic is used
    assert_eq!(resolved.binary, app);
    assert_eq!(resolved.version, version("0.19.1"));
    let survey = ResticSurvey::probe(&too_old_system);
    assert_eq!(
        survey.resolve(&ResticProvider::Automatic).expect("survey"),
        resolved
    );

    let no_system = locations(None, app.clone());
    let resolved = resolve_with(&ResticProvider::Automatic, &no_system).expect("resolved");
    assert_eq!(resolved.binary, app);
}

/// AC-M5-07-6 (core): «System» chosen explicitly with a system restic 0.16.4 resolves to no
/// binary, even with a usable app-managed restic.
#[test]
fn ac_m5_07_6_explicit_system_too_old_blocks_runs() {
    let dir = TempDir::new().expect("tempdir");
    let system = stand_in_0_16_4(&dir.path().join("bin/restic"));
    let app = json_stand_in(&dir.path().join("data/kestic/restic"), "0.19.1");
    let locations = locations(Some(system.clone()), app);

    let err = resolve_with(&ResticProvider::System, &locations).expect_err("too old");
    // (a) no binary: a run cannot start
    assert_eq!(
        err,
        ResticProviderError::TooOld {
            binary: system,
            found: version("0.16.4"),
            min_version: "0.18.0".to_string(),
        }
    );
    let survey = ResticSurvey::probe(&locations);
    assert!(survey.resolve(&ResticProvider::System).is_err());
}

/// AC-M5-07-7: a custom path that is no executable file, or restic older than 0.18.0, is
/// rejected with the reason; restic 0.18.0 is used.
#[test]
fn ac_m5_07_7_custom_path_rejected_or_used() {
    let dir = TempDir::new().expect("tempdir");
    let no_restic = ResticLocations::default();
    let custom = |path: &Path| {
        resolve_with(
            &ResticProvider::Custom {
                path: path.to_path_buf(),
            },
            &no_restic,
        )
    };
    let problem = |path: &Path| match custom(path) {
        Err(ResticProviderError::InvalidCustomPath { problem, .. }) => problem,
        other => panic!(
            "{}: expected an invalid path, got {other:?}",
            path.display()
        ),
    };

    // (a) missing file
    assert_eq!(
        problem(&dir.path().join("missing")),
        ResticPathProblem::NotFound
    );
    // (a) not executable
    let plain = dir.path().join("plain");
    fs::write(&plain, "#!/bin/sh\n").expect("write");
    fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).expect("chmod");
    assert_eq!(problem(&plain), ResticPathProblem::NotExecutable);
    // (a) a folder
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).expect("mkdir");
    assert_eq!(problem(&folder), ResticPathProblem::NotAFile);
    // (a) a parent folder without access
    let locked = dir.path().join("locked");
    let hidden = json_stand_in(&locked.join("restic"), "0.18.0");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
    let denied = problem(&hidden);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod back");
    assert_eq!(denied, ResticPathProblem::PermissionDenied);
    // (a) restic 0.17.1
    let old = json_stand_in(&dir.path().join("old/restic"), "0.17.1");
    match custom(&old) {
        Err(ResticProviderError::TooOld { found, .. }) => assert_eq!(found, version("0.17.1")),
        other => panic!("expected TooOld, got {other:?}"),
    }
    // the reason reaches the user as a text naming the path
    let text = ResticStatusCode::InvalidCustomPath
        .message(&ResticStatusParams {
            binary_path: Some("/opt/restic"),
            path_problem: Some(ResticPathProblem::NotExecutable),
            detail: Some("raw"),
            min_version: "0.18.0",
            ..ResticStatusParams::default()
        })
        .english();
    assert_eq!(
        text,
        "Custom restic path is invalid: /opt/restic cannot be started"
    );

    // (a) another program whose `version` prints a version is not restic
    let other = dir.path().join("other");
    write_executable(
        &other,
        "#!/bin/sh
if [ \"${2:-}\" = --json ]; then exit 129; fi
echo 'git version 2.53.0'
",
    );
    assert!(
        matches!(
            custom(&other),
            Err(ResticProviderError::UnreadableVersion { .. })
        ),
        "{:?}",
        custom(&other)
    );
    // (a) a relative path, which restic would look up in `PATH` instead
    assert_eq!(problem(Path::new("restic")), ResticPathProblem::Other);

    // (b) restic 0.18.0 is used
    let good = json_stand_in(&dir.path().join("good/restic"), "0.18.0");
    let resolved = custom(&good).expect("0.18.0 is accepted");
    assert_eq!(resolved.binary, good);
    assert_eq!(resolved.version, version("0.18.0"));
}

/// AC-M5-07-8: a stand-in like the real 0.16.4 (no `version --json`) is recognised as 0.16.4
/// from its text and reported as too old, not as an unreadable version.
#[test]
fn ac_m5_07_8_text_fallback_detects_016() {
    let dir = TempDir::new().expect("tempdir");
    let old = stand_in_0_16_4(&dir.path().join("restic"));
    let provider = ResticProvider::Custom { path: old.clone() };

    let err = resolve_with(&provider, &ResticLocations::default()).expect_err("too old");
    match &err {
        ResticProviderError::TooOld {
            found, min_version, ..
        } => {
            // (a) 0.16.4 from the text
            assert_eq!(found, &version("0.16.4"));
            // (b) «too old, at least 0.18.0»
            assert_eq!(min_version, "0.18.0");
        }
        other => panic!("expected TooOld, got {other:?}"),
    }
    assert!(
        !matches!(
            check_binary_version(&old),
            Err(ResticProviderError::UnreadableVersion { .. })
        ),
        "no unreadable version"
    );
}

/// AC-M5-07-8 (a): a `version --json` answer that is no version message (another
/// `message_type`, or none) is not taken as the version; the text fallback reads it.
#[test]
fn ac_m5_07_8_text_fallback_without_version_message() {
    let dir = TempDir::new().expect("tempdir");
    for (label, json) in [
        (
            "other type",
            r#"{"message_type":"status","version":"0.99.0"}"#,
        ),
        ("no type", r#"{"version":"0.99.0"}"#),
    ] {
        let binary = dir.path().join(label.replace(' ', "-"));
        write_executable(
            &binary,
            &format!(
                "#!/bin/sh\n\
                 if [ \"${{1:-}}\" = version ] && [ \"${{2:-}}\" = --json ]; then\n\
                 echo '{json}'\n\
                 exit 0\n\
                 fi\n\
                 if [ \"${{1:-}}\" = version ]; then\n\
                 echo 'restic 0.18.1 compiled with go1.24.1 on linux/amd64'\n\
                 exit 0\n\
                 fi\n\
                 exit 2\n"
            ),
        );

        let resolved = check_binary_version(&binary).unwrap_or_else(|e| panic!("{label}: {e}"));
        // (a) the version from the text, not from the JSON answer
        assert_eq!(resolved.version, version("0.18.1"), "{label}");
    }
}

/// AC-M5-07-9: for real restic 0.18.0 and the latest, the version comes from `version --json`
/// alone.
#[test]
fn ac_m5_07_9_real_restic_uses_version_json() {
    let dir = TempDir::new().expect("tempdir");
    for (label, real) in common::restic_test_binaries() {
        let log = dir.path().join(format!("{label}.log"));
        let wrapper = dir.path().join(format!("{label}-restic"));
        write_executable(
            &wrapper,
            &format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
                log.display(),
                real.display()
            ),
        );
        let resolved = resolve_with(
            &ResticProvider::Custom {
                path: wrapper.clone(),
            },
            &ResticLocations::default(),
        )
        .unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(resolved.binary, wrapper);
        let calls = fs::read_to_string(&log).expect("log");
        let calls: Vec<&str> = calls.lines().collect();
        // (a) `version --json`
        assert_eq!(calls.first(), Some(&"version --json"), "{label}: {calls:?}");
        // (b) no plain `version`
        assert!(!calls.contains(&"version"), "{label}: {calls:?}");
    }
}

const CONFIG_V3: &str = r#"config_version = 3

[restic_provider]
kind = "system"
"#;

/// AC-M5-07-10: a version-3 config turns «System» into «Automatic», keeps a custom path and is
/// written as version 4 only by the next save.
#[test]
fn ac_m5_07_10_migrates_config_3_to_4() {
    // (a) system becomes automatic
    let migrated = parse_config_str(CONFIG_V3).expect("v3");
    assert_eq!(migrated.restic_provider, ResticProvider::Automatic);
    assert_eq!(migrated.config_version, 4);

    // (b) a custom path stays as it was
    let custom = parse_config_str(
        "config_version = 3\n\n[restic_provider]\nkind = \"custom\"\npath = \"/opt/restic/restic\"\n",
    )
    .expect("v3 custom");
    assert_eq!(
        custom.restic_provider,
        ResticProvider::Custom {
            path: PathBuf::from("/opt/restic/restic")
        }
    );

    // (c) the file stays as it is until the next save, which writes version 4
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, CONFIG_V3).expect("write");
    let loaded = load_optional_config(&path).expect("load").expect("config");
    assert_eq!(fs::read_to_string(&path).expect("read"), CONFIG_V3);
    save_config(&path, &loaded).expect("save");
    let saved = fs::read_to_string(&path).expect("read");
    assert!(saved.contains("config_version = 4"), "{saved}");
    assert!(saved.contains("kind = \"automatic\""), "{saved}");

    // a version-4 «System» was chosen on purpose and stays
    let v4 = parse_config_str("config_version = 4\n\n[restic_provider]\nkind = \"system\"\n")
        .expect("v4");
    assert_eq!(v4.restic_provider, ResticProvider::System);
    // an app-managed source of version 3 still carried its path
    let app = parse_config_str(
        "config_version = 3\n\n[restic_provider]\nkind = \"app_managed\"\npath = \"/home/u/.local/share/kestic/restic\"\n",
    )
    .expect("v3 app-managed");
    assert_eq!(app.restic_provider, ResticProvider::AppManaged);
    // a new config starts with «Automatic» and checks for releases
    let fresh = AppConfig::default();
    assert_eq!(fresh.restic_provider, ResticProvider::Automatic);
    assert!(fresh.restic_release_check);
}
