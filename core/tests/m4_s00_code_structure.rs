// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S00: the code layout of architecture.md §Code-Struktur, read from the source tree. File
//! and function sizes, sub-statement (b), are checked by `just verify` and clippy.

use kestic_test_support::workspace::workspace_root;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

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

fn camel_case(name: &str) -> String {
    name.split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + chars.as_str()
            })
        })
        .collect()
}

/// Source text without line comments, so that a commented-out item does not count.
fn without_comments(source: &str) -> String {
    source
        .lines()
        .map(|l| l.split("//").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn relative(path: &Path) -> String {
    path.strip_prefix(workspace_root())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
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

/// Members of the `net.daester.Kestic1` interface, by kind («method», «signal», «property»),
/// with their D-Bus names: the `name` of the `#[zbus(...)]` attribute, or zbus' default (the
/// function name in CamelCase).
fn interface_members(server: &str) -> BTreeMap<&'static str, BTreeSet<String>> {
    let source = without_comments(server);
    let body: Vec<&str> = source
        .lines()
        .skip_while(|l| !l.starts_with("#[interface(name = \"net.daester.Kestic1\""))
        .skip(1)
        .take_while(|l| *l != "}")
        .collect();
    assert!(
        body.len() > 10,
        "the `net.daester.Kestic1` interface impl is not found in daemon/src/dbus_server.rs"
    );
    let mut members: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    let mut attrs = String::new();
    for line in body {
        // Items of the impl block are indented by exactly four spaces.
        let Some(item) = line.strip_prefix("    ").filter(|l| !l.starts_with(' ')) else {
            continue;
        };
        if item.starts_with("#[") {
            attrs.push_str(item);
            continue;
        }
        let ident = ["pub async fn ", "pub fn ", "async fn ", "fn "]
            .iter()
            .find_map(|p| item.strip_prefix(p))
            .map(|rest| {
                rest.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or_default()
            });
        if let Some(ident) = ident {
            let kind = if attrs.contains("zbus(signal") {
                "signal"
            } else if attrs.contains("zbus(property") {
                "property"
            } else {
                "method"
            };
            let name = attrs
                .split("name = \"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .map_or_else(|| camel_case(ident), str::to_owned);
            members.entry(kind).or_default().insert(name);
        }
        if !item.trim().is_empty() {
            attrs.clear();
        }
    }
    members
}

/// AC-M4-00-1 (a), daemon: `daemon/src/service.rs` is only the thin entry that declares the
/// modules under `daemon/src/service/` and re-exports them; every such module has its file and
/// every file there is declared.
#[test]
fn ac_m4_00_1_a_daemon_service_is_a_thin_module_distributor() {
    let service = without_comments(&read("daemon/src/service.rs"));
    let code: String = service
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("#!") && !l.starts_with("#["))
        .collect::<Vec<_>>()
        .join(" ");
    let items: Vec<&str> = code
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let thin_item = |s: &&str| {
        let declared_module = ["mod ", "pub mod ", "pub(crate) mod "]
            .iter()
            .any(|p| s.starts_with(p));
        let re_export = ["use ", "pub use ", "pub(crate) use "]
            .iter()
            .any(|p| s.starts_with(p));
        (declared_module && !s.contains('{')) || re_export
    };
    let logic: Vec<&&str> = items.iter().filter(|s| !thin_item(s)).collect();
    assert!(
        logic.is_empty(),
        "AC-M4-00-1 (a): daemon/src/service.rs holds more than module declarations and \
         re-exports: {logic:?}"
    );

    let declared: BTreeSet<String> = items
        .iter()
        .filter_map(|s| s.rsplit_once("mod ").filter(|_| !s.contains("use ")))
        .map(|(_, m)| m.trim().to_owned())
        .collect();
    let root = workspace_root();
    let on_disk: BTreeSet<String> = fs::read_dir(root.join("daemon/src/service"))
        .expect("daemon/src/service/")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "rs"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    assert!(
        declared.len() >= 10,
        "AC-M4-00-1 (a): expected the service modules to be declared in service.rs: {declared:?}"
    );
    assert_eq!(
        declared, on_disk,
        "AC-M4-00-1 (a): the modules declared in service.rs are the files in daemon/src/service/"
    );
}

/// AC-M4-00-1 (a), GUI models: every model of the CXX-Qt bridge in `gui/src/models.rs` (the
/// bridges stay there, see the exception in §Code-Struktur) has its own file
/// `gui/src/models/<model>.rs`, which alone defines its Rust struct and implements it.
#[test]
fn ac_m4_00_1_a_one_file_per_gui_model() {
    let bridges = without_comments(&read("gui/src/models.rs"));
    let models: Vec<String> = bridges
        .lines()
        .filter_map(|l| l.trim().strip_prefix("type "))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|name| name.ends_with("Model"))
        .map(str::to_owned)
        .collect();
    assert!(
        models.len() >= 10,
        "AC-M4-00-1 (a): expected the models in the bridge of gui/src/models.rs: {models:?}"
    );

    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("gui/src"), &mut files);
    let sources: Vec<(String, String)> = files
        .iter()
        .map(|f| {
            let text = fs::read_to_string(f).expect("read source");
            (relative(f), without_comments(&text))
        })
        .collect();
    for model in &models {
        let expected = format!("gui/src/models/{}.rs", snake_case(model));
        let struct_decl = format!("pub struct {model}Rust");
        let defined_in: Vec<&str> = sources
            .iter()
            .filter(|(_, text)| {
                text.lines().any(|l| {
                    l.trim()
                        .strip_prefix(&struct_decl)
                        .is_some_and(|rest| rest.starts_with([' ', '{', ';', '(']))
                })
            })
            .map(|(path, _)| path.as_str())
            .collect();
        assert_eq!(
            defined_in,
            [expected.as_str()],
            "AC-M4-00-1 (a): `{model}Rust` is defined only in {expected}"
        );
        let impl_heads = [
            format!("impl qobject::{model} "),
            format!("impl {model}Rust "),
        ];
        let implemented_in: BTreeSet<&str> = sources
            .iter()
            .filter(|(_, text)| {
                text.lines()
                    .any(|l| impl_heads.iter().any(|h| l.trim().starts_with(h.as_str())))
            })
            .map(|(path, _)| path.as_str())
            .collect();
        assert!(
            implemented_in.iter().all(|path| *path == expected),
            "AC-M4-00-1 (a): `{model}` is implemented outside {expected}: {implemented_in:?}"
        );
    }
}

/// AC-M4-00-1 (a), QML: `Main.qml` defines no dialog, page, sheet or popup inline, and every
/// dialog, page, wizard and window it uses is its own file under `gui/src/qml/`.
#[test]
fn ac_m4_00_1_a_dialogs_and_pages_have_their_own_qml_file() {
    let main_qml = read("gui/src/qml/Main.qml");
    let mut objects = main_qml
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .filter_map(|l| l.trim().strip_suffix(" {"))
        .filter(|name| {
            name.starts_with(|c: char| c.is_ascii_uppercase())
                && name.chars().all(|c| c.is_alphanumeric() || c == '.')
        });
    assert_eq!(
        objects.next(),
        Some("Kirigami.ApplicationWindow"),
        "AC-M4-00-1 (a): Main.qml is the application window"
    );
    let mut own_types = BTreeSet::new();
    for name in objects {
        let ui_container = ["Dialog", "Page", "Sheet", "Popup", "Drawer", "Window"]
            .iter()
            .any(|suffix| name.ends_with(suffix));
        if name.contains('.') {
            assert!(
                !ui_container,
                "AC-M4-00-1 (a): Main.qml defines `{name}` inline instead of in its own QML file"
            );
        } else if ui_container || name.ends_with("Wizard") {
            own_types.insert(name.to_owned());
        }
    }
    assert!(
        own_types.len() >= 10,
        "AC-M4-00-1 (a): expected Main.qml to use its dialogs and pages, found {own_types:?}"
    );
    let root = workspace_root();
    for name in &own_types {
        assert!(
            root.join(format!("gui/src/qml/{name}.qml")).is_file(),
            "AC-M4-00-1 (a): `{name}` used in Main.qml has no file gui/src/qml/{name}.qml"
        );
    }
}

/// AC-M4-00-1 (c): the daemon's D-Bus interface is the one in dbus-api.md (the same methods and
/// signals, no property), and `core` still exports every module it exported before M4.
#[test]
fn ac_m4_00_1_c_dbus_interface_and_core_api_unchanged() {
    let spec = read("docs/spec/dbus-api.md");
    let members = interface_members(&read("daemon/src/dbus_server.rs"));
    let spec_methods = spec_table_names(&spec, "## Methoden auf `net.daester.Kestic1`");
    let spec_signals = spec_table_names(&spec, "## Signale auf `net.daester.Kestic1`");
    assert!(
        spec_methods.len() > 20 && spec_signals.len() > 5,
        "dbus-api.md tables not found"
    );
    assert_eq!(
        members.get("method").cloned().unwrap_or_default(),
        spec_methods,
        "AC-M4-00-1 (c): D-Bus methods differ from dbus-api.md"
    );
    assert_eq!(
        members.get("signal").cloned().unwrap_or_default(),
        spec_signals,
        "AC-M4-00-1 (c): D-Bus signals differ from dbus-api.md"
    );
    assert_eq!(
        members.get("property"),
        None,
        "AC-M4-00-1 (c): dbus-api.md lists no D-Bus property"
    );

    let lib = without_comments(&read("core/src/lib.rs"));
    let exported: BTreeSet<&str> = lib
        .lines()
        .filter_map(|l| l.trim().strip_prefix("pub mod "))
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
