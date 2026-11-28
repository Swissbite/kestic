// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M6-S35: the C++ GUI smoke harness waits for conditions instead of fixed times, has one
//! check macro family, `shot()`/`lookup()` and scenario tables, unique failure codes and no
//! file over the size limit.

use kestic_test_support::workspace::workspace_root;
use std::collections::BTreeMap;
use std::fs;

/// Line limit of a source file below `src/` (architecture.md §Code-Struktur).
const MAX_LINES: usize = 1500;

/// A smoke source (`gui/src/smoke_*.cpp` / `.h`) with its file name.
struct Source {
    name: String,
    text: String,
}

fn smoke_sources() -> Vec<Source> {
    let dir = workspace_root().join("gui").join("src");
    let mut out: Vec<Source> = fs::read_dir(&dir)
        .expect("read gui/src")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            name.starts_with("smoke_") && (name.ends_with(".cpp") || name.ends_with(".h"))
        })
        .map(|p| Source {
            name: p
                .file_name()
                .and_then(|n| n.to_str())
                .expect("utf-8 file name")
                .to_string(),
            text: fs::read_to_string(&p).expect("read smoke source"),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(
        out.len() >= 10,
        "smoke sources not found in {}",
        dir.display()
    );
    out
}

fn source<'a>(sources: &'a [Source], name: &str) -> &'a Source {
    sources
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("{name} missing"))
}

/// Calls of `name(` in `text`, not counting `#define`s and function definitions/declarations.
fn calls<'a>(text: &'a str, name: &str) -> Vec<(usize, &'a str)> {
    let needle = format!("{name}(");
    let mut out = Vec::new();
    for (at, _) in text.match_indices(&needle) {
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let prefix = text[line_start..at].trim();
        if prefix.starts_with("#define")
            || prefix.ends_with("void")
            || prefix.ends_with("int")
            || prefix.ends_with("bool")
        {
            continue;
        }
        out.push((at, &text[at + needle.len()..]));
    }
    out
}

/// The top-level arguments of the call whose argument list starts at `args` (after the `(`).
fn split_args(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 1usize;
    let mut start = 0usize;
    let mut in_str: Option<char> = None;
    let mut escaped = false;
    for (i, c) in args.char_indices() {
        if let Some(q) = in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => in_str = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    out.push(&args[start..i]);
                    return out;
                }
            }
            ',' if depth == 1 => {
                out.push(&args[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    panic!("unterminated call: {}", &args[..args.len().min(80)]);
}

/// Which arguments of a call carry a failure code: the macros of the check families, the smoke
/// failure and the helpers that take codes.
fn code_positions(name: &str) -> Option<&'static [usize]> {
    match name {
        "CHECK" | "CHECK2" | "CHECK3" | "CHECK4" | "CHECK_EQ" | "CHECK_STARTS" | "RECORD_PAGE" => {
            Some(&[0])
        }
        "SMOKE_CHECK"
        | "SMOKE_CHECK2"
        | "SMOKE_CHECK3"
        | "SMOKE_CHECK4"
        | "SMOKE_CHECK5"
        | "SMOKE_CHECK_VISIBLE"
        | "SMOKE_CHECK_ENABLED"
        | "SMOKE_CHECK_DISABLED"
        | "SMOKE_CHECK_OPENED"
        | "SMOKE_CHECK_CLOSED"
        | "SMOKE_FAIL"
        | "SMOKE_CLICK"
        | "SMOKE_ENTER_TEXT"
        | "smoke_fail" => Some(&[1]),
        "poll_backup_run" => Some(&[2, 3]),
        "exit_scenario" => Some(&[3]),
        _ => None,
    }
}

const CODE_CALLS: [&str; 22] = [
    "CHECK",
    "CHECK2",
    "CHECK3",
    "CHECK4",
    "CHECK_EQ",
    "CHECK_STARTS",
    "RECORD_PAGE",
    "SMOKE_CHECK",
    "SMOKE_CHECK2",
    "SMOKE_CHECK3",
    "SMOKE_CHECK4",
    "SMOKE_CHECK5",
    "SMOKE_CHECK_VISIBLE",
    "SMOKE_CHECK_ENABLED",
    "SMOKE_CHECK_DISABLED",
    "SMOKE_CHECK_OPENED",
    "SMOKE_CHECK_CLOSED",
    "SMOKE_FAIL",
    "SMOKE_CLICK",
    "SMOKE_ENTER_TEXT",
    "smoke_fail",
    "poll_backup_run",
];

/// Every literal failure code with where it stands (`file:line`).
fn literal_codes(sources: &[Source]) -> BTreeMap<u32, Vec<String>> {
    let mut codes: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for src in sources {
        for name in CODE_CALLS.iter().copied().chain(["exit_scenario"]) {
            let positions = code_positions(name).expect("known code call");
            for (at, args) in calls(&src.text, name) {
                let args = split_args(args);
                for &pos in positions {
                    let Some(arg) = args.get(pos) else { continue };
                    if let Ok(code) = arg.trim().parse::<u32>() {
                        let line = src.text[..at].lines().count();
                        codes
                            .entry(code)
                            .or_default()
                            .push(format!("{}:{line}", src.name));
                    }
                }
            }
        }
    }
    codes
}

/// AC-M6-35-4 (a): the smoke waits for a condition (`wait_until`, `press_until`); a fixed
/// `wait_ms(...)` is left only where nothing can be observed, marked `// quiet window` on its
/// line or the line above.
#[test]
fn ac_m6_35_4_smoke_waits_are_predicates() {
    let sources = smoke_sources();
    let common = source(&sources, "smoke_common.h");
    assert!(
        common.text.contains("inline bool press_until("),
        "(a) smoke_common.h offers press_until (press a key, then wait for a condition)"
    );

    let fixed_key_waits: Vec<String> = sources
        .iter()
        .flat_map(|s| {
            s.text
                .lines()
                .enumerate()
                .filter(|(_, l)| l.contains("press_and_wait("))
                .map(move |(i, _)| format!("{}:{}", s.name, i + 1))
        })
        .collect();
    assert!(
        fixed_key_waits.is_empty(),
        "(a) no key press with a fixed wait (press_and_wait) is left: {fixed_key_waits:?}"
    );

    let mut unmarked = Vec::new();
    let mut quiet = 0usize;
    let mut predicate_waits = 0usize;
    for src in &sources {
        let lines: Vec<&str> = src.text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            predicate_waits +=
                line.matches("wait_until(").count() + line.matches("press_until(").count();
            if !line.contains("wait_ms(") || line.contains("void wait_ms(int ms)") {
                continue;
            }
            let marked = line.contains("// quiet window")
                || i.checked_sub(1)
                    .is_some_and(|p| lines[p].contains("// quiet window"));
            if marked {
                quiet += 1;
            } else {
                unmarked.push(format!("{}:{}: {}", src.name, i + 1, line.trim()));
            }
        }
    }
    assert!(
        unmarked.is_empty(),
        "(a) every fixed wait_ms(...) is a marked quiet window: {unmarked:#?}"
    );
    assert!(
        predicate_waits > quiet,
        "(a) the smoke waits for conditions more often than it waits fixed times \
         ({predicate_waits} conditions, {quiet} quiet windows)"
    );
}

/// AC-M6-35-5 (b) and (c): `smoke_m4.cpp` is split like the M3 smoke (a shared header and files
/// per area, none over the limit), and every literal failure code of the smoke is used once.
#[test]
fn ac_m6_35_5_smoke_codes_unique() {
    let sources = smoke_sources();

    // (b) split like the M3 smoke: a shared helper header and several area files.
    assert!(
        sources.iter().any(|s| s.name == "smoke_m4_util.h"),
        "(b) the M4 smoke has a shared helper header smoke_m4_util.h"
    );
    let m4_parts = sources
        .iter()
        .filter(|s| s.name.starts_with("smoke_m4_") && s.name.ends_with(".cpp"))
        .count();
    assert!(
        m4_parts >= 3,
        "(b) the M4 scenarios live in smoke_m4_<area>.cpp files, found {m4_parts}"
    );
    let oversized: Vec<String> = sources
        .iter()
        .filter(|s| s.text.lines().count() > MAX_LINES)
        .map(|s| format!("{} ({} lines)", s.name, s.text.lines().count()))
        .collect();
    assert!(
        oversized.is_empty(),
        "(b) no smoke file has more than {MAX_LINES} lines: {oversized:?}"
    );

    // (c) unique failure codes.
    let codes = literal_codes(&sources);
    assert!(
        codes.len() > 1000,
        "(c) the parser finds the failure codes of the smoke, found {}",
        codes.len()
    );
    let duplicates: Vec<String> = codes
        .iter()
        .filter(|(_, at)| at.len() > 1)
        .map(|(code, at)| format!("{code}: {}", at.join(", ")))
        .collect();
    assert!(
        duplicates.is_empty(),
        "(c) every failure code of the smoke is used once: {duplicates:#?}"
    );
}

/// AC-M6-35-6 (a): one shared check macro family, `shot()` and `lookup()`, and scenario tables
/// instead of chains of `if (scen == …)`.
#[test]
fn ac_m6_35_6_smoke_structure() {
    let sources = smoke_sources();
    let common = source(&sources, "smoke_common.h");

    // Shared CHECK macros: defined once, in smoke_common.h; no file keeps its own family.
    assert!(
        common.text.contains("#define CHECK(code, cond, msg)"),
        "(a) smoke_common.h defines the CHECK macro family"
    );
    let own_families: Vec<String> = sources
        .iter()
        .flat_map(|s| {
            s.text
                .lines()
                .filter(|l| {
                    let l = l.trim_start();
                    l.starts_with("#define ")
                        && l.contains("_CHECK")
                        && !l.starts_with("#define SMOKE_")
                })
                .map(move |l| format!("{}: {}", s.name, l.trim()))
        })
        .collect();
    assert!(
        own_families.is_empty(),
        "(a) no smoke file defines its own check macros: {own_families:?}"
    );
    let check_users = sources
        .iter()
        .filter(|s| s.name.ends_with(".cpp") && !calls(&s.text, "CHECK").is_empty())
        .count();
    assert!(
        check_users >= 5,
        "(a) the scenario files use the shared CHECK macros, {check_users} do"
    );

    // shot() and lookup(): one definition in the shared header, used by the scenario files.
    let util = source(&sources, "smoke_m3_util.h");
    assert!(
        util.text
            .contains("inline int shot(const Ctx& c, int code)"),
        "(a) the shared scenario header defines shot()"
    );
    assert!(
        util.text
            .contains("T* lookup(QQuickWindow* window, const char* name)"),
        "(a) the shared scenario header defines lookup()"
    );
    let shot_users = sources
        .iter()
        .filter(|s| s.name.ends_with(".cpp") && s.text.contains("return shot(c, "))
        .count();
    assert!(
        shot_users >= 5,
        "(a) shot() is used by {shot_users} scenario files"
    );
    let lookup_users = sources
        .iter()
        .filter(|s| s.name.ends_with(".cpp") && s.text.contains("lookup(w, "))
        .count();
    assert!(
        lookup_users >= 5,
        "(a) lookup() is used by {lookup_users} scenario files"
    );
    let old_lookups: Vec<&str> = sources
        .iter()
        .filter(|s| s.text.contains("item_named(") || s.text.contains("object_named("))
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        old_lookups.is_empty(),
        "(a) lookup() replaces item_named()/object_named(): {old_lookups:?}"
    );

    // Scenario tables instead of if chains: no scenario name is compared in code; only the
    // table lookup of smoke_common.h compares names.
    let chains: Vec<String> = sources
        .iter()
        .flat_map(|s| {
            s.text
                .lines()
                .enumerate()
                .filter(|(_, l)| {
                    l.contains("scen == QLatin1String(\"")
                        || l.contains("scen.startsWith(QLatin1String(\"")
                })
                .map(move |(i, _)| format!("{}:{}", s.name, i + 1))
        })
        .collect();
    assert!(
        chains.is_empty(),
        "(a) no scenario is chosen by an if chain on its name: {chains:?}"
    );
    let dispatchers: Vec<&str> = sources
        .iter()
        .filter(|s| s.text.contains("_smoke_scenario(") && s.name.ends_with(".cpp"))
        .filter(|s| !s.text.contains("find_scenario") && !s.text.contains("find_m4_lists_scenario"))
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        dispatchers.is_empty(),
        "(a) every dispatcher looks its scenario up in a table: {dispatchers:?}"
    );
    let tables = sources
        .iter()
        .filter(|s| {
            s.text.contains("NamedScenario<") || s.text.contains("const Scenario kScenarios[]")
        })
        .count();
    assert!(
        tables >= 8,
        "(a) the smoke files keep scenario tables, {tables} do"
    );
}
