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
    let mut in_comment = false;
    let mut prev = '\0';
    for (i, c) in args.char_indices() {
        if in_comment {
            in_comment = c != '\n';
            prev = c;
            continue;
        }
        if in_str.is_none() && prev == '/' && c == '/' {
            in_comment = true;
            prev = c;
            continue;
        }
        prev = c;
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

/// The check macros and the argument that carries their failure code.
const CODE_MACROS: [(&str, usize); 20] = [
    ("CHECK", 0),
    ("CHECK2", 0),
    ("CHECK3", 0),
    ("CHECK4", 0),
    ("CHECK_EQ", 0),
    ("CHECK_STARTS", 0),
    ("RECORD_PAGE", 0),
    ("SMOKE_CHECK", 1),
    ("SMOKE_CHECK2", 1),
    ("SMOKE_CHECK3", 1),
    ("SMOKE_CHECK4", 1),
    ("SMOKE_CHECK5", 1),
    ("SMOKE_CHECK_VISIBLE", 1),
    ("SMOKE_CHECK_ENABLED", 1),
    ("SMOKE_CHECK_DISABLED", 1),
    ("SMOKE_CHECK_OPENED", 1),
    ("SMOKE_CHECK_CLOSED", 1),
    ("SMOKE_FAIL", 1),
    ("SMOKE_CLICK", 1),
    ("SMOKE_ENTER_TEXT", 1),
];

/// Names of the parameters that carry a failure code (or the first of a run of codes).
const CODE_PARAMS: [&str; 5] = ["code", "base", "error_code", "timeout_code", "unknown_code"];

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whole-word occurrences of `word` in `text`.
fn word_at<'a>(text: &'a str, word: &'a str) -> impl Iterator<Item = usize> + 'a {
    text.match_indices(word)
        .map(|(at, _)| at)
        .filter(move |&at| {
            !text[..at].chars().next_back().is_some_and(is_ident)
                && !text[at + word.len()..].chars().next().is_some_and(is_ident)
        })
}

/// The text from the `{` at the start of `rest` (after whitespace) to its matching `}`.
fn body_at(rest: &str) -> Option<&str> {
    let trimmed = rest.trim_start();
    let trimmed = trimmed
        .strip_prefix("const")
        .map_or(trimmed, str::trim_start);
    if !trimmed.starts_with('{') {
        return None;
    }
    let mut depth = 0usize;
    for (i, c) in trimmed.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&trimmed[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// A function or lambda of the smoke that takes a failure code: its name, the index of the code
/// parameter, the parameter's name and the body of the definition.
struct CodeTaker<'a> {
    file: &'a str,
    name: String,
    index: usize,
    param: &'static str,
    body: &'a str,
}

/// Finds the code-taking functions and lambdas from their parameter lists (`name(…, int code,
/// …) { … }`, `auto name = [&](int code, …) { … }`); declarations without a body are skipped.
fn code_takers(sources: &[Source]) -> Vec<CodeTaker<'_>> {
    let mut out = Vec::new();
    for src in sources {
        let text = src.text.as_str();
        for param in CODE_PARAMS {
            let needle = format!("int {param}");
            for (at, _) in text.match_indices(&needle) {
                let after = text[at + needle.len()..].chars().next();
                if !matches!(after, Some(',' | ')' | ' ' | '\n')) {
                    continue;
                }
                // The unmatched `(` before the parameter opens its parameter list.
                let mut depth = 0i32;
                let mut index = 0usize;
                let mut open = None;
                for (i, c) in text[..at].char_indices().rev() {
                    match c {
                        ')' | ']' | '}' => depth += 1,
                        '(' if depth == 0 => {
                            open = Some(i);
                            break;
                        }
                        '(' | '[' | '{' => {
                            depth -= 1;
                            if depth < 0 {
                                break;
                            }
                        }
                        ',' if depth == 0 => index += 1,
                        ';' if depth == 0 => break,
                        _ => {}
                    }
                }
                let Some(open) = open else { continue };
                let Some(close) = split_args(&text[open + 1..])
                    .last()
                    .map(|last| last.as_ptr() as usize - text.as_ptr() as usize + last.len())
                else {
                    continue;
                };
                let Some(body) = body_at(&text[close + 1..]) else {
                    continue;
                };
                let mut head = text[..open].trim_end();
                if let Some(capture) = head.strip_suffix(']') {
                    let Some(lb) = capture.rfind('[') else {
                        continue;
                    };
                    head = capture[..lb].trim_end();
                    let Some(h) = head.strip_suffix('=') else {
                        continue;
                    };
                    head = h.trim_end();
                }
                let name: String = head
                    .chars()
                    .rev()
                    .take_while(|c| is_ident(*c))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                if !name.is_empty() {
                    out.push(CodeTaker {
                        file: &src.name,
                        name,
                        index,
                        param,
                        body,
                    });
                }
            }
        }
    }
    out
}

/// `k` when `arg` is `param` (0) or `param + k`.
fn offset_of(arg: &str, param: &str) -> Option<u32> {
    let arg = arg.trim();
    if arg == param {
        return Some(0);
    }
    let rest = arg.strip_prefix(param)?.trim_start().strip_prefix('+')?;
    rest.trim().parse().ok()
}

/// A code taker: the file it is defined in, its name and the index of its code parameter.
type TakerKey = (String, String, usize);

/// The code takers a call of `name` in `file` can reach: the one of that file, else those of
/// every file (helpers of the shared headers and of the helper file).
fn resolve<'s>(
    spans: &'s BTreeMap<TakerKey, u32>,
    file: &str,
    name: &str,
) -> Vec<(&'s TakerKey, u32)> {
    let local: Vec<_> = spans
        .iter()
        .filter(|(k, _)| k.0 == file && k.1 == name)
        .map(|(k, v)| (k, *v))
        .collect();
    if !local.is_empty() {
        return local;
    }
    spans
        .iter()
        .filter(|(k, _)| k.1 == name)
        .map(|(k, v)| (k, *v))
        .collect()
}

/// How many codes each code-taking function uses from its code on: 1 + the largest `k` of
/// `param + k` in its body, and for a call `g(…, param + k)` of another code taker `k` plus that
/// one's span (to a fixed point).
fn spans(takers: &[CodeTaker<'_>]) -> BTreeMap<TakerKey, u32> {
    let mut span: BTreeMap<TakerKey, u32> = BTreeMap::new();
    for t in takers {
        let mut max = 0u32;
        for at in word_at(t.body, t.param) {
            let rest = t.body[at + t.param.len()..].trim_start();
            if let Some(k) = rest
                .strip_prefix('+')
                .and_then(|r| r.trim_start().split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|d| d.parse::<u32>().ok())
            {
                max = max.max(k);
            }
        }
        let e = span
            .entry((t.file.to_string(), t.name.clone(), t.index))
            .or_insert(1);
        *e = (*e).max(max + 1);
    }
    let names: Vec<String> = span.keys().map(|k| k.1.clone()).collect();
    loop {
        let mut changed = false;
        for t in takers {
            let mut need = 1u32;
            for name in &names {
                for (_, args) in calls(t.body, name) {
                    let args = split_args_lenient(args);
                    for (key, g_span) in resolve(&span, t.file, name) {
                        if let Some(k) = args.get(key.2).and_then(|a| offset_of(a, t.param)) {
                            need = need.max(k + g_span);
                        }
                    }
                }
            }
            let e = span
                .entry((t.file.to_string(), t.name.clone(), t.index))
                .or_insert(1);
            if need > *e {
                *e = need;
                changed = true;
            }
        }
        if !changed {
            return span;
        }
    }
}

/// Like [`split_args`], but an unterminated call gives no arguments.
fn split_args_lenient(args: &str) -> Vec<&str> {
    std::panic::catch_unwind(|| split_args(args)).unwrap_or_default()
}

/// Every failure code range `[from, to)` of the smoke with where it stands (`file:line`): the
/// literal code of each check macro (one code) and the literal code given to each code-taking
/// function (as many codes as that function uses).
fn code_ranges(sources: &[Source]) -> Vec<(u32, u32, String)> {
    let takers = code_takers(sources);
    let spans = spans(&takers);
    let mut out = Vec::new();
    for src in sources {
        let mut note = |at: usize, from: u32, span: u32| {
            let line = src.text[..at].lines().count();
            out.push((from, from + span, format!("{}:{line}", src.name)));
        };
        for (name, pos) in CODE_MACROS {
            for (at, args) in calls(&src.text, name) {
                let args = split_args_lenient(args);
                if let Some(code) = args.get(pos).and_then(|a| a.trim().parse::<u32>().ok()) {
                    note(at, code, 1);
                }
            }
        }
        let names: std::collections::BTreeSet<&String> = spans.keys().map(|k| &k.1).collect();
        for name in names {
            for (at, args) in calls(&src.text, name) {
                let args = split_args_lenient(args);
                for (key, span) in resolve(&spans, &src.name, name) {
                    if let Some(code) = args.get(key.2).and_then(|a| a.trim().parse::<u32>().ok()) {
                        note(at, code, span);
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Failure codes changed at run time (`code++`, `code += 2`, `int code = 5110;` …), which make
/// the codes of a scenario depend on how often a loop ran.
fn code_arithmetic(sources: &[Source]) -> Vec<String> {
    let mut out = Vec::new();
    for src in sources {
        for param in CODE_PARAMS {
            for at in word_at(&src.text, param) {
                let before = src.text[..at].trim_end();
                let after = src.text[at + param.len()..].trim_start();
                let assigned_number = after
                    .strip_prefix('=')
                    .filter(|r| !r.starts_with('='))
                    .is_some_and(|r| r.trim_start().starts_with(|c: char| c.is_ascii_digit()));
                if before.ends_with("++")
                    || after.starts_with("++")
                    || after.starts_with("+=")
                    || after.starts_with("-=")
                    || assigned_number
                {
                    let line = src.text[..at].lines().count();
                    out.push(format!("{}:{line}", src.name));
                }
            }
        }
        // A code argument of a check macro is a number or a code parameter (plus a number), not
        // a number plus a loop index.
        for (name, pos) in CODE_MACROS {
            for (at, args) in calls(&src.text, name) {
                let args = split_args_lenient(args);
                let Some(arg) = args.get(pos).map(|a| a.trim()) else {
                    continue;
                };
                let fixed = arg.parse::<u32>().is_ok()
                    || CODE_PARAMS.iter().any(|p| offset_of(arg, p).is_some());
                let line_end = src.text[at..].find('\n').map_or(src.text.len(), |e| at + e);
                let in_macro_definition = src.text[..line_end].trim_end().ends_with('\\');
                if !fixed && !in_macro_definition {
                    let line = src.text[..at].lines().count();
                    out.push(format!("{}:{line}: {name}({arg}, …)", src.name));
                }
            }
        }
    }
    out
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
        // The poll interval inside the definition of `wait_until` is no fixed wait.
        let poll_loop = src
            .text
            .find("bool wait_until(const std::function<bool()>& pred, int timeout_ms) {")
            .and_then(|at| {
                let body = body_at(&src.text[at + src.text[at..].find('{')?..])?;
                let first = src.text[..at].lines().count();
                Some(first..first + body.lines().count())
            });
        for (i, line) in lines.iter().enumerate() {
            if poll_loop.as_ref().is_some_and(|r| r.contains(&i)) {
                continue;
            }
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

    // (c) unique failure codes: no code is computed at run time, and the codes of the checks
    // and the runs of codes the helpers use do not overlap.
    let arithmetic = code_arithmetic(&sources);
    assert!(
        arithmetic.is_empty(),
        "(c) failure codes are fixed in the source, not counted at run time: {arithmetic:?}"
    );
    let ranges = code_ranges(&sources);
    assert!(
        ranges.len() > 1000,
        "(c) the parser finds the failure codes of the smoke, found {}",
        ranges.len()
    );
    let mut clashes = Vec::new();
    let mut reach: Option<&(u32, u32, String)> = None;
    for range in &ranges {
        if let Some(prev) = reach
            && range.0 < prev.1
        {
            clashes.push(format!(
                "{}..{} at {} overlaps {}..{} at {}",
                range.0, range.1, range.2, prev.0, prev.1, prev.2
            ));
        }
        if reach.is_none_or(|prev| range.1 > prev.1) {
            reach = Some(range);
        }
    }
    assert!(
        clashes.is_empty(),
        "(c) every failure code of the smoke is used once: {clashes:#?}"
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
