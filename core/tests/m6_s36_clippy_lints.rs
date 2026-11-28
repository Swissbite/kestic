// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S36: the workspace switches on `redundant_clone` and `needless_pass_by_value` on top of the
//! defaults, every crate inherits the workspace lints, and nothing weakens them again: neither
//! the build scripts nor an attribute in the sources.

use kestic_test_support::workspace::workspace_root;
use std::fs;
use std::path::{Path, PathBuf};

/// Lints and groups that no attribute may switch off.
const PROTECTED_LINTS: [&str; 5] = [
    "redundant_clone",
    "needless_pass_by_value",
    "clippy::all",
    "clippy::nursery",
    "clippy::pedantic",
];

/// Directories that never hold Kestic sources.
const SKIPPED_DIRS: [&str; 3] = ["target", ".git", "node_modules"];

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn workspace_manifest() -> toml::Table {
    read(&workspace_root().join("Cargo.toml"))
        .parse()
        .expect("Cargo.toml is TOML")
}

fn clippy_lints(manifest: &toml::Table) -> toml::Table {
    manifest["workspace"]["lints"]["clippy"]
        .as_table()
        .expect("[workspace.lints.clippy] is a table")
        .clone()
}

/// The level of a lint entry, whether written as `"warn"` or as `{ level = "warn", … }`.
fn level(entry: &toml::Value) -> Option<&str> {
    entry
        .as_str()
        .or_else(|| entry.get("level").and_then(toml::Value::as_str))
}

/// Every file below `dir` (recursive), skipping build output and VCS data.
fn files_below(dir: &Path, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.map_while(Result::ok) {
        let path = entry.path();
        let skipped = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| SKIPPED_DIRS.contains(&n));
        if skipped {
            continue;
        }
        if path.is_dir() {
            files_below(&path, keep, out);
        } else if keep(&path) {
            out.push(path);
        }
    }
}

/// A command-line flag in `line` that switches a lint off or caps all lints.
fn weakening_flag(line: &str) -> Option<String> {
    let compact: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.contains("--cap-lints") {
        return Some("--cap-lints".to_string());
    }
    for flag in [
        "-A ", "-A", "--allow ", "--allow=", "-W ", "--warn ", "--warn=",
    ] {
        for target in ["clippy", "warnings"] {
            let needle = format!("{flag}{target}");
            if compact.contains(&needle) {
                return Some(needle);
            }
        }
    }
    None
}

/// The text of every outer or inner attribute in `source`, across lines, with the line it starts on.
fn attributes(source: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
            let start = index;
            let mut text = String::new();
            let mut depth = 0i32;
            loop {
                let line = lines[index].split("//").next().unwrap_or("");
                for c in line.chars() {
                    match c {
                        '[' | '(' => depth += 1,
                        ']' | ')' => depth -= 1,
                        _ => {}
                    }
                }
                text.push_str(line.trim());
                text.push(' ');
                if depth <= 0 || index + 1 == lines.len() {
                    break;
                }
                index += 1;
            }
            out.push((start, text));
        }
        index += 1;
    }
    out
}

/// The protected lint that `attribute` switches off, if any.
fn switched_off_lint(attribute: &str) -> Option<&'static str> {
    let compact: String = attribute.chars().filter(|c| !c.is_whitespace()).collect();
    let body = compact
        .strip_prefix("#![")
        .or_else(|| compact.strip_prefix("#["))?;
    let lowers = ["allow(", "expect("].iter().any(|kind| {
        body.starts_with(kind)
            || (body.starts_with("cfg_attr(") && body.contains(&format!(",{kind}")))
    });
    if !lowers {
        return None;
    }
    PROTECTED_LINTS.iter().copied().find(|lint| {
        let lint: String = lint.chars().filter(|c| !c.is_whitespace()).collect();
        if lint.contains("::") {
            body.contains(&lint)
        } else {
            body.contains(&format!("clippy::{lint}"))
        }
    })
}

/// An attribute directly on a `fn` (other attributes in between are fine) with a `//` comment
/// right above it or on its line: the one allowed form, for a signature that a foreign trait or
/// macro fixes.
fn is_commented_fn_exception(lines: &[&str], start: usize) -> bool {
    let commented = lines[start].contains("//")
        || start
            .checked_sub(1)
            .is_some_and(|above| lines[above].trim_start().starts_with("//"));
    let applies_to_fn = lines[start + 1..]
        .iter()
        .map(|l| l.trim_start())
        .find(|l| !l.is_empty() && !l.starts_with("#[") && !l.starts_with("//"))
        .is_some_and(|l| {
            l.starts_with("fn ")
                || l.starts_with("pub fn ")
                || l.starts_with("pub(crate) fn ")
                || l.starts_with("async fn ")
                || l.starts_with("pub async fn ")
        });
    commented && applies_to_fn && !lines[start].trim_start().starts_with("#![")
}

/// Every attribute in `source` that switches off a protected lint without the allowed exception.
fn forbidden_attributes(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    attributes(source)
        .into_iter()
        .filter_map(|(start, text)| {
            let lint = switched_off_lint(&text)?;
            (!is_commented_fn_exception(&lines, start))
                .then(|| format!("line {}: {lint} in {}", start + 1, text.trim()))
        })
        .collect()
}

#[test]
fn ac_m6_36_11_workspace_lints() {
    let manifest = workspace_manifest();
    let lints = clippy_lints(&manifest);

    // AC-M6-36-11 (a): both lints are switched on on top of the defaults.
    for lint in ["redundant_clone", "needless_pass_by_value"] {
        assert_eq!(
            lints.get(lint).and_then(level),
            Some("warn"),
            "AC-M6-36-11 (a): [workspace.lints.clippy] sets {lint} = \"warn\""
        );
    }

    // AC-M6-36-11 (b): pedantic and nursery stay off.
    for group in ["pedantic", "nursery"] {
        let on = lints.get(group).and_then(level).filter(|l| *l != "allow");
        assert_eq!(
            on, None,
            "AC-M6-36-11 (b): the group {group} is not switched on"
        );
    }

    // AC-M6-36-11 (c): every member inherits the workspace lints.
    let members = manifest["workspace"]["members"]
        .as_array()
        .expect("workspace.members is a list");
    assert!(members.len() >= 6, "precondition: the members are found");
    for member in members {
        let member = member.as_str().expect("member is a path");
        let crate_manifest: toml::Table = read(&workspace_root().join(member).join("Cargo.toml"))
            .parse()
            .expect("member Cargo.toml is TOML");
        let inherits = crate_manifest
            .get("lints")
            .and_then(|l| l.get("workspace"))
            .and_then(toml::Value::as_bool);
        assert_eq!(
            inherits,
            Some(true),
            "AC-M6-36-11 (c): {member}/Cargo.toml has [lints] workspace = true"
        );
    }

    // AC-M6-36-11 (d): no build script or CI configuration weakens a lint.
    let root = workspace_root();
    let mut scripts = vec![root.join("justfile")];
    files_below(&root.join(".cargo"), &|_| true, &mut scripts);
    files_below(&root.join(".github/workflows"), &|_| true, &mut scripts);
    files_below(&root.join(".github/scripts"), &|_| true, &mut scripts);
    assert!(scripts.len() > 3, "precondition: the CI files are found");
    let weakened: Vec<String> = scripts
        .iter()
        .flat_map(|path| {
            read(path)
                .lines()
                .enumerate()
                .filter_map(|(n, line)| {
                    weakening_flag(line).map(|flag| format!("{}:{}: {flag}", path.display(), n + 1))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        weakened.is_empty(),
        "AC-M6-36-11 (d): nothing weakens a clippy lint, found {weakened:#?}"
    );
}

#[test]
fn ac_m6_36_11_d_weakening_flags_are_recognised() {
    // AC-M6-36-11 (d): the search would find each form that weakens a lint.
    for line in [
        "cargo clippy -- -D warnings -A clippy::redundant_clone",
        "cargo clippy -- -Aclippy::needless_pass_by_value",
        "cargo clippy -- --allow clippy::all",
        "cargo clippy -- --allow=clippy::all",
        "RUSTFLAGS=\"--cap-lints warn\" cargo clippy",
        "export RUSTFLAGS='-A warnings'",
        "cargo clippy -- -W clippy::redundant_clone",
    ] {
        assert!(weakening_flag(line).is_some(), "found: {line}");
    }
    for line in [
        "cargo clippy --workspace --all-targets -- -D warnings",
        "read -ra rust_flags <<< \"${RUSTFLAGS:-}\"",
        "rust_flags+=(\"--remap-path-prefix=$map\")",
    ] {
        assert_eq!(weakening_flag(line), None, "not found: {line}");
    }
}

#[test]
fn ac_m6_36_12_no_lint_switched_off_in_source() {
    let mut sources = Vec::new();
    files_below(
        &workspace_root(),
        &|p| p.extension().and_then(|e| e.to_str()) == Some("rs"),
        &mut sources,
    );
    assert!(sources.len() > 200, "precondition: the sources are found");

    // AC-M6-36-12 (a): no attribute switches off a protected lint outside the allowed exception.
    let found: Vec<String> = sources
        .iter()
        .flat_map(|path| {
            forbidden_attributes(&read(path))
                .into_iter()
                .map(move |f| format!("{}: {f}", path.display()))
        })
        .collect();
    assert!(
        found.is_empty(),
        "AC-M6-36-12 (a): no allow/expect switches off a protected lint, found {found:#?}"
    );
}

#[test]
fn ac_m6_36_12_a_search_recognises_every_form() {
    // AC-M6-36-12 (a): each level (crate, module, impl, function) and each attribute form is found.
    let forbidden = [
        "#![allow(clippy::redundant_clone)]\nfn main() {}",
        "#![expect(clippy::pedantic)]",
        "#![cfg_attr(test, allow(clippy::nursery))]",
        "mod m {\n    #![allow(clippy::all)]\n}",
        "#[allow(clippy::needless_pass_by_value)]\nmod m {}",
        "#[allow(\n    clippy::too_many_lines,\n    clippy::redundant_clone,\n)]\nimpl A {}",
        "#[expect(clippy::needless_pass_by_value, reason = \"x\")]\nfn f(a: String) {}",
        "#[cfg_attr(feature = \"x\", expect(clippy::redundant_clone))]\nfn f() {}",
        "#[allow(clippy :: redundant_clone)]\nfn f() {}",
        "#[allow(clippy::needless_pass_by_value)]\npub fn f(a: String) {}",
        "// reason\n#[allow(clippy::needless_pass_by_value)]\nimpl A {}",
    ];
    for source in forbidden {
        assert_eq!(forbidden_attributes(source).len(), 1, "found: {source}");
    }

    // AC-M6-36-12 (a): the exception directly on a commented function, and other lints, pass.
    let allowed = [
        "// The signature is fixed by the zbus interface macro.\n#[allow(clippy::needless_pass_by_value)]\nfn notify(a: String) {}",
        "#[allow(clippy::needless_pass_by_value)] // fixed by the Notify protocol\n#[must_use]\npub async fn notify(a: String) {}",
        "#[allow(clippy::too_many_arguments)]\nfn f() {}",
        "#![allow(dead_code)]",
        "#[cfg(test)]\nmod tests {}",
    ];
    for source in allowed {
        assert_eq!(
            forbidden_attributes(source),
            Vec::<String>::new(),
            "allowed: {source}"
        );
    }
}
