// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S36: the smoke harness sends its input through Qt Test, not through private Qt symbols, and
//! only the test-harness build adds Qt Test.

use kestic_test_support::workspace::workspace_root;
use std::fs;
use std::path::{Path, PathBuf};

const SOURCE_EXTENSIONS: [&str; 7] = ["cpp", "cc", "cxx", "h", "hh", "hpp", "rs"];

/// Every source file below `gui/src`, with its text.
fn gui_sources() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    collect(&workspace_root().join("gui/src"), &mut out);
    assert!(out.len() > 10, "precondition: the sources are found");
    out
}

fn collect(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(dir).expect("read dir").map_while(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SOURCE_EXTENSIONS.contains(&e))
        {
            let text = fs::read_to_string(&path).expect("read source");
            out.push((path, text));
        }
    }
}

/// `line` without a `//` comment and without `/* … */` comments that start and end on it.
fn without_comments(line: &str) -> String {
    let mut text = line.split("//").next().unwrap_or("").to_string();
    while let (Some(start), Some(end)) = (text.find("/*"), text.find("*/")) {
        if end < start {
            break;
        }
        text.replace_range(start..end + 2, " ");
    }
    text
}

/// The header a C++ `#include` (also `# include`) or a cxx `include!("…")` names.
fn included_path(line: &str) -> Option<String> {
    let line = without_comments(line);
    let trimmed = line.trim();
    let rest = if let Some(after_hash) = trimmed.strip_prefix('#') {
        after_hash.trim_start().strip_prefix("include")?
    } else {
        let at = trimmed.find("include!(")?;
        &trimmed[at + "include!(".len()..]
    };
    let rest = rest.trim();
    let open = rest.find(['<', '"'])?;
    let close = rest[open + 1..].find(['>', '"'])? + open + 1;
    Some(rest[open + 1..close].to_string())
}

fn is_private_include(line: &str) -> bool {
    included_path(line).is_some_and(|path| {
        path.contains("/private/")
            || path.contains("/qpa/")
            || path.starts_with("private/")
            || path.starts_with("qpa/")
            || path.ends_with("_p.h")
    })
}

fn name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// The include check finds private headers also behind a spaced `#`, a comment or `include!`.
#[test]
fn ac_m6_36_5_a_private_include_forms() {
    for (line, private) in [
        ("#include <QtGui/private/qguiapplication_p.h>", true),
        ("# include <QtGui/private/qguiapplication_p.h>", true),
        ("#  include <qguiapplication_p.h> // x", true),
        ("#include \"qwindow_p.h\" /* c */", true),
        ("#include <qpa/qwindowsysteminterface.h>", true),
        ("    include!(\"QtGui/private/qguiapplication_p.h\");", true),
        ("#include <QQuickItem>", false),
        ("#include \"src/smoke_common.h\"", false),
        ("// #include <QtGui/private/x_p.h> was here", false),
    ] {
        assert_eq!(is_private_include(line), private, "{line}");
    }
}

/// AC-M6-36-5 (a), (b): no private Qt symbol or header in the smoke sources; clicks, drags and
/// keys go through QTest.
#[test]
fn ac_m6_36_5_smoke_input_through_qtest_only() {
    let sources = gui_sources();
    for (path, text) in &sources {
        let smoke = name(path).starts_with("smoke");
        if smoke {
            for line in text.lines().map(without_comments) {
                for bypass in ["sendEvent(", "postEvent("] {
                    assert!(
                        !line.contains(bypass),
                        "AC-M6-36-5 (b): {} sends input past QTest: {line}",
                        name(path)
                    );
                }
                for event in ["QKeyEvent", "QMouseEvent", "QWheelEvent"] {
                    assert!(
                        line.trim_start().starts_with("#include") || !line.contains(event),
                        "AC-M6-36-5 (b): {} builds a {event} itself: {line}",
                        name(path)
                    );
                }
            }
        }
        for banned in [
            "qt_handleMouseEvent",
            "qt_handleKeyEvent",
            "QWindowSystemInterface",
            "Q_GUI_EXPORT",
        ] {
            assert!(
                !text.contains(banned),
                "AC-M6-36-5 (a): {} names `{banned}`",
                name(path)
            );
        }
        for line in text.lines() {
            assert!(
                !is_private_include(line),
                "AC-M6-36-5 (a): {} includes a private Qt header: {line}",
                name(path)
            );
        }
    }
    let helper = sources
        .iter()
        .find(|(p, _)| name(p) == "smoke_helper.cpp")
        .map(|(_, t)| t.as_str())
        .expect("smoke_helper.cpp");
    for call in [
        "QTest::mouseMove(",
        "QTest::mousePress(",
        "QTest::mouseRelease(",
        "QTest::mouseClick(",
        "QTest::keyClick(",
        "QTest::sendKeyEvent(",
    ] {
        assert!(
            helper.contains(call),
            "AC-M6-36-5 (b): smoke_helper.cpp uses `{call}`"
        );
    }
}

/// AC-M6-36-6 (a): `gui/build.rs` adds Qt Test only for the `test-harness` feature.
#[test]
fn ac_m6_36_6_a_qt_test_only_in_the_test_harness_build() {
    let build = fs::read_to_string(workspace_root().join("gui/build.rs")).expect("read build.rs");
    let feature = build
        .find("CARGO_FEATURE_TEST_HARNESS")
        .expect("precondition: build.rs has the test-harness branch");
    let module = build
        .find(".qt_module(\"Test\")")
        .expect("AC-M6-36-6 (a): build.rs adds the Test module");
    assert_eq!(
        build.matches(".qt_module(\"Test\")").count(),
        1,
        "AC-M6-36-6 (a): the Test module is added once"
    );
    let branch_end = build[feature..]
        .find("\n    }\n")
        .map(|i| feature + i)
        .expect("end of the test-harness branch");
    assert!(
        module > feature && module < branch_end,
        "AC-M6-36-6 (a): the Test module sits inside the test-harness branch"
    );
}
