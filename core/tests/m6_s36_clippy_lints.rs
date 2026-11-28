// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S36: the workspace switches on `redundant_clone` and `needless_pass_by_value` on top of the
//! defaults, every crate inherits the workspace lints, and nothing weakens them again: neither
//! the build scripts nor an attribute in the sources.

use kestic_test_support::workspace::workspace_root;
use std::fs;
use std::path::{Path, PathBuf};

/// Lints and groups that no attribute may switch off, as they appear in an `allow(…)` list.
const PROTECTED_LINTS: [&str; 6] = [
    "clippy::redundant_clone",
    "clippy::needless_pass_by_value",
    "clippy::all",
    "clippy::nursery",
    "clippy::pedantic",
    "warnings",
];

/// The one lint the exception for a signature fixed by a foreign trait or macro may cover.
const SIGNATURE_LINT: &str = "clippy::needless_pass_by_value";

/// Directories, relative to the workspace root, that hold no Kestic sources of this checkout.
const SKIPPED_DIRS: [&str; 4] = ["target", ".git", ".claude/worktrees", "site/node_modules"];

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

/// Every file below `root` that `keep` accepts, without following symlinks and without the
/// directories in `skipped` (relative to `root`).
fn files_below(root: &Path, skipped: &[&str], keep: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    fn walk(
        dir: &Path,
        root: &Path,
        skipped: &[&str],
        keep: &dyn Fn(&Path) -> bool,
        out: &mut Vec<PathBuf>,
    ) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.map_while(Result::ok) {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                if !skipped.iter().any(|s| relative == Path::new(s)) {
                    walk(&path, root, skipped, keep, out);
                }
            } else if kind.is_file() && keep(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, skipped, keep, &mut out);
    out
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    files_below(root, &SKIPPED_DIRS, &|p| {
        p.extension().and_then(|e| e.to_str()) == Some("rs")
    })
}

/// A command-line flag in `text` that switches a lint off or caps all lints. Quotes, brackets and
/// commas are ignored, so shell arrays and TOML arrays are read like a command line, and a
/// backslash at the end of a line continues it.
fn weakening_flag(text: &str) -> Option<String> {
    let joined = text.replace("\\\n", " ");
    for line in joined.lines() {
        let normalised: String = line
            .chars()
            .map(|c| if "\"'[](),".contains(c) { ' ' } else { c })
            .collect();
        let tokens: Vec<&str> = normalised.split_whitespace().collect();
        for (i, token) in tokens.iter().enumerate() {
            if token.starts_with("--cap-lints") {
                return Some(line.trim().to_string());
            }
            let lowered = |lint: &str| lint.starts_with("clippy") || lint == "warnings";
            let split_flag = ["-A", "--allow", "-W", "--warn"].contains(token)
                && tokens.get(i + 1).is_some_and(|next| lowered(next));
            let joined_flag = ["-A", "--allow=", "-W", "--warn="]
                .iter()
                .any(|flag| token.strip_prefix(flag).is_some_and(lowered));
            if split_flag || joined_flag {
                return Some(line.trim().to_string());
            }
        }
    }
    None
}

/// `source` with every comment and every string, byte-string and char literal replaced by
/// spaces (line breaks kept), so attributes can be read without being misled by their text.
/// The line comments are returned as well: line index and text after the `//`.
fn mask(source: &str) -> (Vec<char>, Vec<(usize, String)>) {
    let chars: Vec<char> = source.chars().collect();
    let mut out = chars.clone();
    let mut comments = Vec::new();
    let mut line = 0;
    let mut i = 0;
    let blank = |out: &mut Vec<char>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != '\n' {
                *c = ' ';
            }
        }
    };
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c == '/' && next == Some('/') {
            let end = chars[i..]
                .iter()
                .position(|&c| c == '\n')
                .map_or(chars.len(), |p| i + p);
            comments.push((line, chars[i + 2..end].iter().collect()));
            blank(&mut out, i, end);
            i = end;
        } else if c == '/' && next == Some('*') {
            let start = i;
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    line += usize::from(chars[i] == '\n');
                    i += 1;
                }
            }
            blank(&mut out, start, i.min(chars.len()));
        } else if c == 'r'
            && (i == 0
                || !ident(chars[i - 1])
                || (chars[i - 1] == 'b' && (i < 2 || !ident(chars[i - 2]))))
            && matches!(next, Some('"' | '#'))
        {
            let hashes = chars[i + 1..].iter().take_while(|&&c| c == '#').count();
            if chars.get(i + 1 + hashes) != Some(&'"') {
                i += 1;
                continue;
            }
            let start = i + 1 + hashes + 1;
            let mut end = start;
            while end < chars.len() {
                if chars[end] == '"'
                    && chars[end + 1..].iter().take_while(|&&c| c == '#').count() >= hashes
                {
                    break;
                }
                end += 1;
            }
            line += chars[start..end.min(chars.len())]
                .iter()
                .filter(|&&c| c == '\n')
                .count();
            blank(&mut out, start, end.min(chars.len()));
            i = (end + 1 + hashes).min(chars.len());
        } else if c == '"' {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && chars[end] != '"' {
                end += if chars[end] == '\\' { 2 } else { 1 };
            }
            let end = end.min(chars.len());
            line += chars[start..end].iter().filter(|&&c| c == '\n').count();
            blank(&mut out, start, end);
            i = end + 1;
        } else if c == '\'' {
            // A char literal (`'x'`, `'\n'`, `'\u{…}'`); otherwise a lifetime or label.
            let end = if next == Some('\\') {
                // The escaped character itself may be a quote: `'\''`.
                chars
                    .get(i + 3..)
                    .and_then(|rest| rest.iter().position(|&c| c == '\''))
                    .map(|p| i + 3 + p)
            } else if chars.get(i + 2) == Some(&'\'') {
                Some(i + 2)
            } else {
                None
            };
            if let Some(end) = end {
                blank(&mut out, i + 1, end);
                i = end + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    (out, comments)
}

/// One attribute: its masked text without whitespace, whether it is an inner attribute, the line
/// it starts on and the index just after its closing `]`.
struct Attribute {
    text: String,
    inner: bool,
    line: usize,
    end: usize,
}

/// The attribute that starts at `start` (a `#`) in the masked text, if one does.
fn attribute_at(code: &[char], start: usize, line: usize) -> Option<Attribute> {
    let mut i = start + 1;
    let skip_space = |i: &mut usize| {
        while *i < code.len() && code[*i].is_whitespace() {
            *i += 1;
        }
    };
    skip_space(&mut i);
    let inner = code.get(i) == Some(&'!');
    if inner {
        i += 1;
        skip_space(&mut i);
    }
    if code.get(i) != Some(&'[') {
        return None;
    }
    let mut depth = 0i32;
    let mut end = i;
    while end < code.len() {
        match code[end] {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth -= 1,
            _ => {}
        }
        end += 1;
        if depth == 0 {
            break;
        }
    }
    let text = code[start..end]
        .iter()
        .filter(|c| !c.is_whitespace())
        .collect();
    Some(Attribute {
        text,
        inner,
        line,
        end,
    })
}

fn attributes(code: &[char]) -> Vec<Attribute> {
    let mut out = Vec::new();
    let mut line = 0;
    for (i, &c) in code.iter().enumerate() {
        if c == '\n' {
            line += 1;
        } else if c == '#'
            && let Some(attribute) = attribute_at(code, i, line)
        {
            out.push(attribute);
        }
    }
    out
}

/// The protected lints an attribute switches off (`allow`, `expect`, also inside `cfg_attr`).
fn switched_off_lints(attribute: &Attribute) -> Vec<&'static str> {
    let tokens: Vec<&str> = attribute
        .text
        .split(['#', '!', '[', ']', '(', ')', ',', '='])
        .filter(|t| !t.is_empty())
        .collect();
    let lowers = |t: &&str| *t == "allow" || *t == "expect";
    let switches_off = match tokens.first() {
        Some(first) if lowers(first) => true,
        Some(&"cfg_attr") => tokens.iter().any(lowers),
        _ => false,
    };
    if !switches_off {
        return Vec::new();
    }
    PROTECTED_LINTS
        .iter()
        .copied()
        .filter(|lint| tokens.contains(lint))
        .collect()
}

/// After `from`, past whitespace and further attributes, the item is a function.
fn applies_to_fn(code: &[char], from: usize) -> bool {
    let mut i = from;
    loop {
        while i < code.len() && code[i].is_whitespace() {
            i += 1;
        }
        if code.get(i) == Some(&'#')
            && let Some(attribute) = attribute_at(code, i, 0)
        {
            i = attribute.end;
            continue;
        }
        break;
    }
    let rest: String = code[i..code.len().min(i + 200)].iter().collect();
    for word in rest.split_whitespace() {
        match word {
            "fn" => return true,
            "const" | "async" | "unsafe" | "extern" | "default" | "pub" => {}
            w if w.starts_with("pub(") && w.ends_with(')') => {}
            _ => return false,
        }
    }
    false
}

/// A plain `//` comment with text on the attribute's line, or directly above the attributes
/// stacked on its item.
fn has_reason_comment(
    source_lines: &[&str],
    comments: &[(usize, String)],
    attribute: &Attribute,
) -> bool {
    let is_reason = |line: usize| {
        comments.iter().any(|(l, text)| {
            *l == line
                && !text.starts_with('/')
                && !text.starts_with('!')
                && !text.trim().is_empty()
        })
    };
    if is_reason(attribute.line) {
        return true;
    }
    let mut above = attribute.line;
    while above > 0 {
        above -= 1;
        let trimmed = source_lines.get(above).map_or("", |l| l.trim_start());
        if trimmed.starts_with("#[") {
            continue;
        }
        return trimmed.starts_with("//") && is_reason(above);
    }
    false
}

/// Every attribute in `source` that switches off a protected lint without the allowed exception:
/// only `needless_pass_by_value`, directly on a function, with a comment giving the reason.
fn forbidden_attributes(source: &str) -> Vec<String> {
    let (code, comments) = mask(source);
    let source_lines: Vec<&str> = source.lines().collect();
    attributes(&code)
        .into_iter()
        .filter_map(|attribute| {
            let lints = switched_off_lints(&attribute);
            if lints.is_empty() {
                return None;
            }
            let exception = lints == [SIGNATURE_LINT]
                && !attribute.inner
                && applies_to_fn(&code, attribute.end)
                && has_reason_comment(&source_lines, &comments, &attribute);
            (!exception).then(|| {
                format!(
                    "line {}: {} in {}",
                    attribute.line + 1,
                    lints.join(", "),
                    attribute.text
                )
            })
        })
        .collect()
}

/// The `cargo clippy` lines of the `justfile` recipe `_clippy`.
fn clippy_recipe_lines(justfile: &str) -> Vec<String> {
    justfile
        .lines()
        .skip_while(|l| !l.starts_with("_clippy:"))
        .skip(1)
        .take_while(|l| l.starts_with(char::is_whitespace) || l.is_empty())
        .map(str::trim)
        .filter(|l| l.starts_with("cargo clippy"))
        .map(str::to_string)
        .collect()
}

#[test]
fn ac_m6_36_11_workspace_lints() {
    let manifest = workspace_manifest();
    let lints = clippy_lints(&manifest);

    // AC-M6-36-11 (a): both lints are switched on on top of the defaults, which stay on.
    for lint in ["redundant_clone", "needless_pass_by_value"] {
        assert_eq!(
            lints.get(lint).and_then(level),
            Some("warn"),
            "AC-M6-36-11 (a): [workspace.lints.clippy] sets {lint} = \"warn\""
        );
    }
    assert_eq!(
        lints.get("all").and_then(level),
        Some("warn"),
        "AC-M6-36-11 (a): the default lints (clippy::all) stay switched on"
    );

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
    for dir in [".cargo", ".github", ".devcontainer"] {
        scripts.extend(files_below(&root.join(dir), &[], &|_| true));
    }
    assert!(scripts.len() > 3, "precondition: the CI files are found");
    let weakened: Vec<String> = scripts
        .iter()
        .filter_map(|path| {
            let text = String::from_utf8(fs::read(path).ok()?).ok()?;
            weakening_flag(&text).map(|line| format!("{}: {line}", path.display()))
        })
        .collect();
    assert!(
        weakened.is_empty(),
        "AC-M6-36-11 (d): nothing weakens a clippy lint, found {weakened:#?}"
    );

    // AC-M6-36-11 (e): `just verify` runs clippy with -D warnings over all targets, without and
    // with the test harness.
    let clippy = clippy_recipe_lines(&read(&root.join("justfile")));
    let strict: Vec<&String> = clippy
        .iter()
        .filter(|l| {
            l.contains("--workspace")
                && l.contains("--all-targets")
                && l.ends_with("-- -D warnings")
        })
        .collect();
    assert_eq!(
        strict.len(),
        2,
        "AC-M6-36-11 (e): two strict clippy runs in _clippy, found {clippy:#?}"
    );
    assert_eq!(
        strict
            .iter()
            .filter(|l| l.contains("--features kestic-gui/test-harness"))
            .count(),
        1,
        "AC-M6-36-11 (e): one strict clippy run with the test harness, found {clippy:#?}"
    );
}

#[test]
fn ac_m6_36_11_d_weakening_flags_are_recognised() {
    // AC-M6-36-11 (d): the search finds each form that weakens a lint.
    for text in [
        "cargo clippy -- -D warnings -A clippy::redundant_clone",
        "cargo clippy -- -Aclippy::needless_pass_by_value",
        "cargo clippy -- --allow clippy::all",
        "cargo clippy -- --allow=clippy::all",
        "cargo clippy -- -A \"clippy::all\"",
        "cargo clippy -- -A 'clippy::all'",
        "cargo clippy -- -D warnings -A \\\n    clippy::all",
        "RUSTFLAGS=\"--cap-lints warn\" cargo clippy",
        "export RUSTFLAGS='-A warnings'",
        "cargo clippy -- -W clippy::redundant_clone",
        "rust_flags+=(\"-A\" \"clippy::redundant_clone\")",
        "rustflags = [\"-A\", \"clippy::all\"]",
        "rustflags = [\"--cap-lints\", \"allow\"]",
    ] {
        assert!(weakening_flag(text).is_some(), "found: {text}");
    }
    for text in [
        "cargo clippy --workspace --all-targets -- -D warnings",
        "read -ra rust_flags <<< \"${RUSTFLAGS:-}\"",
        "rust_flags+=(\"--remap-path-prefix=$map\")",
        "grep -A 3 warnings log.txt",
    ] {
        assert_eq!(weakening_flag(text), None, "not found: {text}");
    }
}

#[test]
fn ac_m6_36_12_no_lint_switched_off_in_source() {
    let sources = rust_sources(&workspace_root());
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
fn ac_m6_36_12_a_search_covers_every_source_directory() {
    // AC-M6-36-12 (a): only the build output at the root is skipped, not a source directory
    // that happens to be named `target`.
    let root = tempfile::tempdir().expect("tempdir");
    for dir in ["target", "core/src/target", ".claude/worktrees/x"] {
        fs::create_dir_all(root.path().join(dir)).expect("mkdir");
        fs::write(root.path().join(dir).join("a.rs"), "").expect("write");
    }
    let found: Vec<PathBuf> = rust_sources(root.path())
        .into_iter()
        .map(|p| {
            p.strip_prefix(root.path())
                .expect("below root")
                .to_path_buf()
        })
        .collect();
    assert_eq!(found, vec![PathBuf::from("core/src/target/a.rs")]);
}

#[test]
fn ac_m6_36_12_a_search_recognises_every_form() {
    // AC-M6-36-12 (a): each level (crate, module, impl, function) and each attribute form is
    // found exactly once.
    let forbidden = [
        "#![allow(clippy::redundant_clone)]\nfn main() {}",
        "#![expect(clippy::pedantic)]",
        "#![cfg_attr(test, allow(clippy::nursery))]",
        "#![allow(warnings)]",
        "#![cfg_attr(test, allow(warnings))]",
        "mod m {\n    #![allow(clippy::all)]\n}",
        "#[allow(clippy::needless_pass_by_value)]\nmod m {}",
        "#[allow(\n    clippy::too_many_lines,\n    clippy::redundant_clone,\n)]\nimpl A {}",
        "#[expect(clippy::needless_pass_by_value, reason = \"x\")]\nfn f(a: String) {}",
        "#[cfg_attr(feature = \"x\", expect(clippy::redundant_clone))]\nfn f() {}",
        "#[allow(clippy :: redundant_clone)]\nfn f() {}",
        "# [allow(clippy::redundant_clone)]\nfn f() {}",
        "#[allow(clippy::needless_pass_by_value)]\npub fn f(a: String) {}",
        "// reason\n#[allow(clippy::needless_pass_by_value)]\nimpl A {}",
        "/// Does x.\n#[allow(clippy::needless_pass_by_value)]\nfn f(a: String) {}",
        "//\n#[allow(clippy::needless_pass_by_value)]\nfn f(a: String) {}",
        "// hmm\n#[allow(clippy::redundant_clone)]\nfn f() {}",
        "// hmm\n#[allow(clippy::all)]\nfn f() {}",
        "// hmm\n#[allow(clippy::needless_pass_by_value, clippy::redundant_clone)]\nfn f() {}",
        "#[doc = \"see https://example.org\"]\n#[allow(clippy::redundant_clone)]\nfn f() {}",
        "#[cfg_attr(feature = \"http://x\", allow(clippy::redundant_clone))]\nfn f() {}",
        "#[allow(\n    clippy::too_many_lines,\n    reason = \"see x))\",\n    clippy::redundant_clone,\n)]\nfn f() {}",
        "#[derive(Debug)] #[allow(clippy::redundant_clone)]\nstruct A;",
        "fn f() { let c = '\"'; }\n#[allow(clippy::redundant_clone)]\nfn g() {}",
        "fn f<'a>(x: &'a str) {}\n#[allow(clippy::redundant_clone)]\nfn g() {}",
        "fn f() { let c = '\\''; }\n#[allow(clippy::redundant_clone)]\nfn g() {}",
    ];
    for source in forbidden {
        assert_eq!(
            forbidden_attributes(source).len(),
            1,
            "found once: {source}"
        );
    }

    // AC-M6-36-12 (a): the exception directly on a commented function, other lints, and
    // attribute text inside a string or comment pass.
    let allowed = [
        "// The signature is fixed by the zbus interface macro.\n#[allow(clippy::needless_pass_by_value)]\nfn notify(a: String) {}",
        "#[allow(clippy::needless_pass_by_value)] // fixed by the Notify protocol\n#[must_use]\npub async fn notify(a: String) {}",
        "    // zbus passes the header by value.\n    #[allow(clippy::needless_pass_by_value)]\n    #[zbus(name = \"X\")]\n    fn x(&self) {}",
        "// zbus\n#[allow(clippy::needless_pass_by_value)]\npub(super) fn f(a: String) {}",
        "// zbus\n#[allow(clippy::needless_pass_by_value)]\nconst fn f(a: String) {}",
        "// zbus\n#[allow(clippy::needless_pass_by_value)]\n#[cfg(\n    test\n)]\nfn f(a: String) {}",
        "#[allow(clippy::too_many_arguments)]\nfn f() {}",
        "#[allow(clippy::allow_attributes)]\nfn f() {}",
        "#![allow(dead_code)]",
        "#[cfg(test)]\nmod tests {}",
        "let s = r#\"\n#[allow(clippy::redundant_clone)]\nfn f() {}\n\"#;",
        "let s = \"#[allow(clippy::redundant_clone)]\";",
        "// #[allow(clippy::redundant_clone)]\nfn f() {}",
        "/* #![allow(warnings)] */",
    ];
    for source in allowed {
        assert_eq!(
            forbidden_attributes(source),
            Vec::<String>::new(),
            "allowed: {source}"
        );
    }
}
