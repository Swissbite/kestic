// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! AC-M4-01-7 (d) over every QML file, also the pages the GUI smoke does not open: every list
//! is a `KeyboardList` (one Tab stop), and no row of a list is a Tab stop of its own.

use kestic_test_support::workspace::workspace_root;
use std::fs;

/// The QML files of the GUI, also those in subdirectories, with their paths below `gui/src/qml`.
fn qml_files() -> Vec<(String, String)> {
    let root = workspace_root().join("gui/src/qml");
    let mut dirs = vec![root.clone()];
    let mut out: Vec<(String, String)> = Vec::new();
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).expect("read a QML directory") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|x| x == "qml") {
                let name = path
                    .strip_prefix(&root)
                    .expect("below gui/src/qml")
                    .to_string_lossy()
                    .into_owned();
                out.push((name, fs::read_to_string(&path).expect("read qml")));
            }
        }
    }
    out.sort();
    out
}

/// The code of a QML line: trimmed, its string literals emptied and its `//` comment removed,
/// so braces and property names in texts and comments count for nothing.
fn code_of(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match quote {
            // An escaped character inside a string is skipped with its backslash.
            Some(_) if c == '\\' => {
                chars.next();
            }
            Some(q) if c == q => {
                quote = None;
                out.push(c);
            }
            Some(_) => {}
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                out.push(c);
            }
            None if c == '/' && chars.peek() == Some(&'/') => break,
            None => out.push(c),
        }
    }
    out.trim().to_string()
}

/// The number of braces a line of code opens, minus those it closes.
fn depth_change(code: &str) -> i32 {
    let opened = i32::try_from(code.matches('{').count()).expect("count");
    let closed = i32::try_from(code.matches('}').count()).expect("count");
    opened - closed
}

/// The code of the lines directly inside the block that opens on the line `start` (one brace
/// deeper), without the lines of nested blocks.
fn direct_lines(lines: &[&str], start: usize) -> Vec<String> {
    let mut depth = 0i32;
    let mut out = Vec::new();
    for line in &lines[start..] {
        let code = code_of(line);
        let before = depth;
        depth += depth_change(&code);
        if before == 1 {
            out.push(code);
        }
        if depth <= 0 && before > 0 {
            break;
        }
    }
    out
}

/// The lines of the block that opens on the line `start`, nested blocks included.
fn block_lines<'a>(lines: &[&'a str], start: usize) -> Vec<&'a str> {
    let mut depth = 0i32;
    let mut out = Vec::new();
    for line in &lines[start..] {
        depth += depth_change(&code_of(line));
        out.push(*line);
        if depth <= 0 {
            break;
        }
    }
    out
}

/// Whether the code `code` opens an object of type `type_name` (also with a module prefix, as
/// the value of a property, or without a space before the brace).
fn opens(code: &str, type_name: &str) -> bool {
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let opening = format!("{type_name}{{");
    compact.starts_with(&opening)
        || compact.contains(&format!(".{opening}"))
        || compact.contains(&format!(":{opening}"))
}

/// Endings of the type names of controls that are Tab stops unless they say otherwise: every
/// button, delegate, list item, field and slider, and the controls named here.
const TAB_STOP_CONTROLS: [&str; 12] = [
    "Button{",
    "Delegate{",
    "ListItem{",
    "Field{",
    "Slider{",
    "CheckBox{",
    "Switch{",
    "TextArea{",
    "ComboBox{",
    "SpinBox{",
    "Card{",
    "Chip{",
];

/// Whether the code `code` opens a control of [`TAB_STOP_CONTROLS`], with or without space
/// before its brace.
fn opens_tab_stop_control(code: &str) -> bool {
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    TAB_STOP_CONTROLS.iter().any(|c| compact.contains(c))
}

/// The indexes of the lines of `block` that open a Tab stop: a control of [`TAB_STOP_CONTROLS`]
/// that does not take the focus by click only, or a line `activeFocusOnTab: true`. The first
/// line of the block (its own object) does not count. A control written on one line
/// (`Button { text: "x" }`) counts as well.
fn tab_stops_in(block: &[&str]) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, line) in block.iter().enumerate().skip(1) {
        let code = code_of(line);
        if code.starts_with("activeFocusOnTab: true") {
            out.push(i);
            continue;
        }
        if !opens_tab_stop_control(&code) {
            continue;
        }
        let own = if depth_change(&code) == 0 {
            vec![code]
        } else {
            direct_lines(block, i)
        };
        let click_only = own.iter().any(|l| {
            l.contains("focusPolicy: Qt.NoFocus")
                || l.contains("focusPolicy: Qt.ClickFocus")
                || l.contains("activeFocusOnTab: false")
        });
        if !click_only {
            out.push(i);
        }
    }
    out
}

/// The `Repeater`s with Tab-stop rows that AC-M4-01-7 exempts, as file and the reason in the
/// comment line right above them (`// Not a list: <reason>`). The cards of the destination
/// types are two `Repeater`s, the ready types and those that need a program.
const EXEMPTIONS: [(&str, &str); 12] = [
    (
        "BackupConditionsEditPage.qml",
        "form rows, one check box per known network.",
    ),
    (
        "BackupExcludesEditPage.qml",
        "form rows, one exclude rule each with its own controls.",
    ),
    (
        "BackupScheduleEditPage.qml",
        "a row of buttons, one per weekday.",
    ),
    (
        "DestinationDetailsForm.qml",
        "form rows of the environment variables",
    ),
    (
        "DestinationTypeChooser.qml",
        "radio choices of the cloud service",
    ),
    (
        "DestinationTypeChooser.qml",
        "the cards of the ready destination types",
    ),
    (
        "DestinationTypeChooser.qml",
        "the cards of the destination types that need a program",
    ),
    (
        "KindChips.qml",
        "the filter chips by kind, a row of buttons.",
    ),
    ("MainHeaderArea.qml", "the breadcrumb, a row of buttons."),
    (
        "OverviewPage.qml",
        "the backup cards, a card grid; each card is a Tab stop of its own.",
    ),
    ("RestoreBrowsePage.qml", "the path row, a row of buttons."),
    (
        "RunDetailPage.qml",
        "one card per command, each with its text to read and copy.",
    ),
];

/// Whether the block of a `Repeater` that opens on the line `start` holds a row that is a Tab
/// stop (see [`tab_stops_in`]).
fn repeats_tab_stops(lines: &[&str], start: usize) -> bool {
    !tab_stops_in(&block_lines(lines, start)).is_empty()
}

/// AC-M4-01-7 (d): a control on one line, and one with more code after its brace, is a
/// Tab-stop row; one that takes the focus by click only is not.
#[test]
fn ac_m4_01_7_one_line_controls_count_as_tab_stop_rows() {
    let one_line = [
        "Repeater {",
        "    delegate: Controls.Button { text: \"x\" }",
        "}",
    ];
    assert!(repeats_tab_stops(&one_line, 0), "a control on one line");
    let trailing = [
        "Repeater {",
        "    delegate: Controls.CheckBox { // a row",
        "        text: \"x\"",
        "    }",
        "}",
    ];
    assert!(
        repeats_tab_stops(&trailing, 0),
        "a control with a comment after its brace"
    );
    let click_only = [
        "Repeater {",
        "    delegate: Controls.Button { text: \"x\"; focusPolicy: Qt.ClickFocus }",
        "}",
    ];
    assert!(
        !repeats_tab_stops(&click_only, 0),
        "a control that takes the focus by click only"
    );
}

/// AC-M4-01-7 (d): every kind of control that is a Tab stop counts as a Tab-stop row, also
/// after a text or comment with braces; a commented-out focus policy does not hide it.
#[test]
fn ac_m4_01_7_every_tab_stop_control_counts_as_tab_stop_row() {
    for control in [
        "Controls.CheckDelegate",
        "Controls.RadioDelegate",
        "Controls.SwitchDelegate",
        "Controls.SwipeDelegate",
        "Kirigami.SwipeListItem",
        "Kirigami.BasicListItem",
        "Kirigami.SearchField",
        "Kirigami.PasswordField",
        "Controls.Slider",
        "Kirigami.NavigationTabButton",
    ] {
        let delegate = format!("    delegate: {control} {{");
        let rows = ["Repeater {", &delegate, "        text: \"x\"", "    }", "}"];
        assert!(repeats_tab_stops(&rows, 0), "{control}");
    }
    let brace_in_text = [
        "Repeater {",
        "    delegate: RowLayout {",
        "        Controls.Label { text: \"}}\" }",
        "        Controls.Button { text: \"x\" }",
        "    }",
        "}",
    ];
    assert!(repeats_tab_stops(&brace_in_text, 0), "a brace in a text");
    let commented_policy = [
        "Repeater {",
        "    delegate: Controls.Button {",
        "        // focusPolicy: Qt.NoFocus",
        "        text: \"x\"",
        "    }",
        "}",
    ];
    assert!(
        repeats_tab_stops(&commented_policy, 0),
        "a commented-out focus policy"
    );
    let no_space = [
        "Repeater {",
        "    delegate: Controls.CheckBox{",
        "        text: \"x\"",
        "    }",
        "}",
    ];
    assert!(
        repeats_tab_stops(&no_space, 0),
        "a control without a space before its brace"
    );
    assert!(
        opens(&code_of("Repeater{"), "Repeater"),
        "a Repeater without a space"
    );
    assert!(
        opens(&code_of("QtQuick.Repeater {"), "Repeater"),
        "a Repeater with its module"
    );
    assert!(
        !opens(&code_of("// Repeater {"), "Repeater"),
        "a Repeater in a comment"
    );
}

/// AC-M4-01-7 (d): a `Repeater` whose rows are Tab stops is no list of entries but one of the
/// exemptions the AC names: the comment line right above it says so (`// Not a list: <reason>`),
/// and the Repeaters found are exactly [`EXEMPTIONS`], so a new one fails. A list of entries is a `KeyboardList`.
#[test]
fn ac_m4_01_7_no_repeater_lists_tab_stop_rows() {
    let mut found: Vec<(String, String)> = Vec::new();
    for (name, text) in qml_files() {
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !opens(&code_of(line), "Repeater") || !repeats_tab_stops(&lines, i) {
                continue;
            }
            let above = i.checked_sub(1).map_or("", |j| lines[j].trim());
            let reason = above.strip_prefix("// Not a list: ");
            assert!(
                reason.is_some(),
                "(d) {name}:{}: a Repeater with Tab-stop rows; a list is a KeyboardList",
                i + 1
            );
            found.push((name.clone(), reason.unwrap_or_default().to_string()));
        }
    }
    found.sort();
    let mut expected: Vec<(String, String)> = EXEMPTIONS
        .iter()
        .map(|(f, r)| ((*f).to_string(), (*r).to_string()))
        .collect();
    expected.sort();
    assert_eq!(
        found, expected,
        "(d) the Repeaters with Tab-stop rows are exactly the exemptions of AC-M4-01-7"
    );
}

/// AC-M4-01-7 (d): a row of `Kirigami.Chip`s is a row of Tab stops.
#[test]
fn ac_m4_01_7_chips_count_as_tab_stop_rows() {
    let (_, text) = qml_files()
        .into_iter()
        .find(|(name, _)| name == "KindChips.qml")
        .expect("KindChips.qml");
    let lines: Vec<&str> = text.lines().collect();
    let repeater = lines
        .iter()
        .position(|l| opens(&code_of(l), "Repeater"))
        .expect("the Repeater of the chips");
    assert!(repeats_tab_stops(&lines, repeater));
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

/// AC-M4-01-7 (d): in every `KeyboardList`, of whatever type its rows are, (a) no row and (b) no
/// control inside a row is a Tab stop of its own; only the list is.
#[test]
fn ac_m4_01_7_no_row_of_a_keyboard_list_is_a_tab_stop() {
    let mut lists = 0;
    let mut rows = 0;
    for (name, text) in qml_files() {
        if name == "KeyboardList.qml" {
            continue;
        }
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !opens(&code_of(line), "KeyboardList") {
                continue;
            }
            lists += 1;
            let block = block_lines(&lines, i);
            rows += block
                .iter()
                .filter(|l| {
                    let code = code_of(l);
                    code.starts_with("delegate:") && opens_tab_stop_control(&code)
                })
                .count();
            for at in tab_stops_in(&block) {
                let code = code_of(block[at]);
                assert!(
                    !code.starts_with("delegate:"),
                    "(a) {name}:{}: a row of a list is a Tab stop",
                    i + at + 1
                );
                assert!(
                    code.starts_with("delegate:"),
                    "(b) {name}:{}: a control in a row of a list is a Tab stop",
                    i + at + 1
                );
            }
        }
    }
    assert!(lists >= 19, "lists checked: {lists}");
    assert!(rows >= 17, "rows of lists checked: {rows}");
}
