// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! AC-M4-01-7 (d) over every QML file, also the pages the GUI smoke does not open: every list
//! is a `KeyboardList` (one Tab stop), and no row of a list is a Tab stop of its own.

use kestic_test_support::workspace::workspace_root;
use std::fs;

/// The QML files of the GUI with their names.
fn qml_files() -> Vec<(String, String)> {
    let dir = workspace_root().join("gui/src/qml");
    let mut out: Vec<(String, String)> = fs::read_dir(&dir)
        .expect("read gui/src/qml")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "qml"))
        .map(|p| {
            let name = p.file_name().expect("name").to_string_lossy().into_owned();
            (name, fs::read_to_string(&p).expect("read qml"))
        })
        .collect();
    out.sort();
    out
}

/// The lines directly inside the block that opens on the line `start` (one brace deeper),
/// without the lines of nested blocks.
fn direct_lines(lines: &[&str], start: usize) -> Vec<String> {
    let mut depth = 0i32;
    let mut out = Vec::new();
    for line in &lines[start..] {
        let before = depth;
        depth += i32::try_from(line.matches('{').count()).expect("count");
        depth -= i32::try_from(line.matches('}').count()).expect("count");
        if before == 1 {
            out.push(line.trim().to_string());
        }
        if depth <= 0 && before > 0 {
            break;
        }
    }
    out
}

/// AC-M4-01-7 (d): (a) no QML file declares a plain `ListView` (lists are `KeyboardList`),
/// (b) every row delegate takes the focus by click only, and (c) none is a Tab stop.
#[test]
fn ac_m4_01_7_every_list_is_one_tab_stop_in_every_qml_file() {
    let files = qml_files();
    assert!(files.len() > 40, "QML files found: {}", files.len());
    let mut rows = 0;
    for (name, text) in &files {
        if name == "KeyboardList.qml" {
            continue;
        }
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let code = line.trim();
            assert!(
                !(code.starts_with("ListView {") || code.contains(": ListView {")),
                "(a) {name}:{}: a plain ListView",
                i + 1
            );
            if code.starts_with("delegate: Controls.ItemDelegate {") {
                rows += 1;
                let own = direct_lines(&lines, i);
                assert!(
                    own.iter().any(|l| l == "focusPolicy: Qt.ClickFocus"),
                    "(b) {name}:{}: a row that takes the focus by Tab",
                    i + 1
                );
                assert!(
                    !own.iter().any(|l| l.starts_with("activeFocusOnTab: true")),
                    "(c) {name}:{}: a row that is a Tab stop",
                    i + 1
                );
            }
        }
    }
    assert!(rows >= 15, "(b) rows of lists checked: {rows}");
}
