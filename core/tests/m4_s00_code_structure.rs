// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S00: the code layout of architecture.md §Code-Struktur, read from the source tree.

use kestic_test_support::workspace::workspace_root;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const MAX_FILE_LINES: usize = 1500;

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Names in backticks from the first column of the first table below the given heading.
fn spec_table_names(spec: &str, heading_prefix: &str) -> BTreeSet<String> {
    let mut in_section = false;
    let mut names = BTreeSet::new();
    for line in spec.lines() {
        if line.starts_with('#') {
            in_section = line.starts_with(heading_prefix);
            continue;
        }
        if in_section && !names.is_empty() && !line.starts_with('|') {
            break;
        }
        if in_section && let Some(rest) = line.strip_prefix("| `") {
            let name = rest.split('`').next().unwrap_or_default();
            names.insert(name.to_owned());
        }
    }
    names
}

/// D-Bus names of the methods (`signal == false`) or signals of the daemon's interface.
fn interface_names(source: &str, signal: bool) -> BTreeSet<String> {
    let prefix = if signal {
        "#[zbus(signal, name = \""
    } else {
        "#[zbus(name = \""
    };
    source
        .lines()
        .filter_map(|line| line.trim().strip_prefix(prefix))
        .map(|rest| rest.split('"').next().unwrap_or_default().to_owned())
        .collect()
}

fn source_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext == "rs" || ext == "qml")
        {
            out.push(path);
        }
    }
}

/// AC-M4-00-1 (a): the daemon's `service.rs` only distributes to `service/<bereich>.rs`, every
/// GUI list model has its own file under `gui/src/models/`, and every dialog, page and window
/// that `Main.qml` uses is its own QML file.
#[test]
fn ac_m4_00_1_a_modules_models_and_dialogs_in_place() {
    let service = read("daemon/src/service.rs");
    let logic_lines: Vec<&str> = service
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .filter(|l| {
            !(l.starts_with("mod ")
                || l.starts_with("pub mod ")
                || l.starts_with("pub use ")
                || l.starts_with("pub(crate) use ")
                || l.starts_with("use ")
                || l.starts_with("#[")
                || l.ends_with(',')
                || l.ends_with("};")
                || l.ends_with('{'))
        })
        .collect();
    assert!(
        logic_lines.is_empty(),
        "AC-M4-00-1 (a): daemon/src/service.rs holds more than module declarations and re-exports: {logic_lines:?}"
    );
    let root = workspace_root();
    for line in service.lines() {
        if let Some(module) = line.strip_prefix("mod ").and_then(|m| m.strip_suffix(';')) {
            assert!(
                root.join(format!("daemon/src/service/{module}.rs"))
                    .is_file(),
                "AC-M4-00-1 (a): service module `{module}` is not in daemon/src/service/"
            );
        }
    }

    let models = read("gui/src/models.rs");
    let model_types: Vec<String> = models
        .lines()
        .filter_map(|l| l.trim().strip_prefix("type "))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|name| name.ends_with("Model"))
        .map(str::to_owned)
        .collect();
    assert!(
        !model_types.is_empty(),
        "AC-M4-00-1 (a): no list model found in gui/src/models.rs"
    );
    for model in &model_types {
        let file = format!("gui/src/models/{}.rs", snake_case(model));
        assert!(
            root.join(&file).is_file(),
            "AC-M4-00-1 (a): list model `{model}` has no file {file}"
        );
    }

    let main_qml = read("gui/src/qml/Main.qml");
    let mut own_types = BTreeSet::new();
    for line in main_qml.lines() {
        let Some(name) = line.trim().strip_suffix(" {") else {
            continue;
        };
        if name.contains('.') || !name.starts_with(|c: char| c.is_ascii_uppercase()) {
            assert!(
                !(name.ends_with("Dialog")
                    || name.ends_with("Page")
                    || name.ends_with("Sheet")
                    || name.ends_with("Popup")),
                "AC-M4-00-1 (a): Main.qml defines `{name}` inline instead of in its own QML file"
            );
            continue;
        }
        if ["Dialog", "Page", "Wizard", "Window"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            own_types.insert(name.to_owned());
        }
    }
    assert!(
        own_types.len() >= 10,
        "AC-M4-00-1 (a): expected Main.qml to use its dialogs and pages, found {own_types:?}"
    );
    for name in &own_types {
        assert!(
            root.join(format!("gui/src/qml/{name}.qml")).is_file(),
            "AC-M4-00-1 (a): `{name}` used in Main.qml has no file gui/src/qml/{name}.qml"
        );
    }
}

/// AC-M4-00-1 (b): no `.rs` or `.qml` file under a crate's `src/` exceeds 1500 lines. The
/// function length (150 lines) is clippy's `too_many_lines` in `clippy.toml`.
#[test]
fn ac_m4_00_1_b_no_source_file_over_limit() {
    let root = workspace_root();
    let mut files = Vec::new();
    for dir in [
        "core/src",
        "daemon/src",
        "gui/src",
        "i18n/src",
        "test-support/src",
    ] {
        source_files(&root.join(dir), &mut files);
    }
    assert!(
        files.len() > 50,
        "AC-M4-00-1 (b): too few source files found ({})",
        files.len()
    );
    let oversized: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let lines = fs::read_to_string(f).expect("read source").lines().count();
            (lines > MAX_FILE_LINES).then(|| format!("{} ({lines})", f.display()))
        })
        .collect();
    assert!(
        oversized.is_empty(),
        "AC-M4-00-1 (b): files over {MAX_FILE_LINES} lines: {oversized:?}"
    );
    let clippy = read("clippy.toml");
    assert!(
        clippy
            .lines()
            .any(|l| l.replace(' ', "") == "too-many-lines-threshold=150"),
        "AC-M4-00-1 (b): clippy.toml must set too-many-lines-threshold = 150"
    );
}

/// AC-M4-00-1 (c): the daemon's D-Bus interface is the one in dbus-api.md (same methods and
/// signals), and `core` still exports every module it exported before M4.
#[test]
fn ac_m4_00_1_c_dbus_interface_and_core_api_unchanged() {
    let spec = read("docs/spec/dbus-api.md");
    let server = read("daemon/src/dbus_server.rs");
    assert_eq!(
        interface_names(&server, false),
        spec_table_names(&spec, "## Methoden auf"),
        "AC-M4-00-1 (c): D-Bus methods differ from dbus-api.md"
    );
    assert_eq!(
        interface_names(&server, true),
        spec_table_names(&spec, "## Signale auf"),
        "AC-M4-00-1 (c): D-Bus signals differ from dbus-api.md"
    );

    let lib = read("core/src/lib.rs");
    let exported: BTreeSet<&str> = lib
        .lines()
        .filter_map(|l| l.strip_prefix("pub mod "))
        .filter_map(|l| l.strip_suffix(';'))
        .collect();
    let before_m4 = [
        "backup",
        "backup_progress",
        "build_info",
        "clock",
        "config",
        "destination",
        "excludes",
        "history",
        "maintenance",
        "migrations",
        "model",
        "mount",
        "provider",
        "restic",
        "retention",
        "scheduler",
        "secrets",
        "snapshot",
        "sync",
    ];
    let missing: Vec<&str> = before_m4
        .iter()
        .copied()
        .filter(|m| !exported.contains(m))
        .collect();
    assert!(
        missing.is_empty(),
        "AC-M4-00-1 (c): core no longer exports {missing:?}"
    );
}

/// AC-M4-00-2 (c): `just verify` is the whole suite: static checks, clippy with and without the
/// test harness, the .deb, the offscreen smoke and all tests. (a) and (b) hold because M4-S00
/// moved no code; the tripwire on each pull request into `m4` checks changed assertions.
#[test]
fn ac_m4_00_2_c_verify_runs_the_whole_suite() {
    let justfile = read("justfile");
    let recipe: Vec<&str> = justfile
        .split("\nverify:\n")
        .nth(1)
        .expect("AC-M4-00-2 (c): justfile has a `verify` recipe")
        .lines()
        .take_while(|l| l.starts_with("    "))
        .map(str::trim)
        .collect();
    for step in [
        "just _static-checks",
        "just _clippy",
        "just deb",
        "just _build-test-harness",
        "just _smoke-run",
        "just _nextest",
    ] {
        assert!(
            recipe.contains(&step),
            "AC-M4-00-2 (c): `just verify` no longer runs `{step}`: {recipe:?}"
        );
    }
}
