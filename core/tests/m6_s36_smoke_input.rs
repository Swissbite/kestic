// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S36: the smoke harness sends its input through Qt Test, not through private Qt symbols, and
//! only the test-harness build adds Qt Test.

use kestic_test_support::workspace::workspace_root;
use std::fs;
use std::path::{Path, PathBuf};

fn gui_sources() -> Vec<(PathBuf, String)> {
    let dir = workspace_root().join("gui/src");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir)
        .expect("read gui/src")
        .map_while(Result::ok)
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "cpp" || e == "h") {
            let text = fs::read_to_string(&path).expect("read source");
            out.push((path, text));
        }
    }
    assert!(out.len() > 5, "precondition: the C++ sources are found");
    out
}

fn is_private_include(line: &str) -> bool {
    let line = line.trim();
    let Some(rest) = line.strip_prefix("#include") else {
        return false;
    };
    let path = rest
        .trim()
        .trim_matches(|c| c == '<' || c == '>' || c == '"');
    path.contains("/private/")
        || path.contains("/qpa/")
        || path.starts_with("private/")
        || path.starts_with("qpa/")
        || path.ends_with("_p.h")
}

fn name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// AC-M6-36-5 (a), (b): no private Qt symbol or header in the smoke sources; clicks, drags and
/// keys go through QTest.
#[test]
fn ac_m6_36_5_smoke_input_through_qtest_only() {
    let sources = gui_sources();
    for (path, text) in &sources {
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
