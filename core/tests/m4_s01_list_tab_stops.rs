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

/// The lines of the block that opens on the line `start`, nested blocks included.
fn block_lines<'a>(lines: &[&'a str], start: usize) -> Vec<&'a str> {
    let mut depth = 0i32;
    let mut out = Vec::new();
    for line in &lines[start..] {
        depth += i32::try_from(line.matches('{').count()).expect("count");
        depth -= i32::try_from(line.matches('}').count()).expect("count");
        out.push(*line);
        if depth <= 0 {
            break;
        }
    }
    out
}

/// Controls that are Tab stops unless they say otherwise.
const TAB_STOP_CONTROLS: [&str; 12] = [
    "Button {",
    "ToolButton {",
    "CheckBox {",
    "RadioButton {",
    "Switch {",
    "TextField {",
    "TextArea {",
    "ComboBox {",
    "SpinBox {",
    "ItemDelegate {",
    "Card {",
    "Chip {",
];

/// The `Repeater`s with Tab-stop rows that AC-M4-01-7 exempts, as file and the reason in the
/// comment line right above them (`// Not a list: <reason>`).
const EXEMPTIONS: [(&str, &str); 11] = [
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
        "the cards to choose the type of the destination from",
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

/// Whether the block of a `Repeater` holds a row that is a Tab stop: a control of
/// [`TAB_STOP_CONTROLS`] that does not take the focus by click only, or `activeFocusOnTab: true`.
fn repeats_tab_stops(lines: &[&str], start: usize) -> bool {
    let block = block_lines(lines, start);
    block.iter().enumerate().any(|(i, line)| {
        let code = line.trim();
        if code.starts_with("activeFocusOnTab: true") {
            return true;
        }
        let is_control = TAB_STOP_CONTROLS
            .iter()
            .any(|c| code.ends_with(c) && !code.starts_with("//"));
        is_control
            && !direct_lines(&block, i).iter().any(|l| {
                l == "focusPolicy: Qt.NoFocus"
                    || l == "focusPolicy: Qt.ClickFocus"
                    || l.starts_with("activeFocusOnTab: false")
            })
    })
}

/// AC-M4-01-7 (d): a `Repeater` whose rows are Tab stops is no list of entries but one of the
/// exemptions the AC names: (a) the comment line right above it says so
/// (`// Not a list: <reason>`), and (b) the Repeaters found are exactly [`EXEMPTIONS`], so a new
/// one fails. A list of entries is a `KeyboardList`.
#[test]
fn ac_m4_01_7_no_repeater_lists_tab_stop_rows() {
    let mut found: Vec<(String, String)> = Vec::new();
    for (name, text) in qml_files() {
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.trim().starts_with("Repeater {") || !repeats_tab_stops(&lines, i) {
                continue;
            }
            let above = i.checked_sub(1).map_or("", |j| lines[j].trim());
            let reason = above.strip_prefix("// Not a list: ");
            assert!(
                reason.is_some(),
                "(a) {name}:{}: a Repeater with Tab-stop rows; a list is a KeyboardList",
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
        "(b) the Repeaters with Tab-stop rows are the exemptions of AC-M4-01-7"
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
        .position(|l| l.trim().starts_with("Repeater {"))
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
