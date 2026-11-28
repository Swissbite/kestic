// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn rules(json: &str) -> ExcludeRules {
    serde_json::from_str(json).expect("rules json")
}

/// AC-M6-34-2 (a) and (c): loading saved rules gives the fields the QML showed
/// (`loadFromExcludesJson`, `parseSizeLimit`).
#[test]
fn ac_m6_34_2_exclude_form_loads_like_former_qml() {
    let form = ExcludeForm::from_rules(&rules(
        r#"{"folders":["$HOME/.cache"],"lines":["*.tmp","!keep"],"exclude_caches":true,
            "exclude_trash":true,"exclude_larger_than":"2G","ignore_case":true,
            "exclude_if_present":[".nobackup","CACHEDIR.TAG"]}"#,
    ));
    assert_eq!(
        form.folders,
        vec!["$HOME/.cache".to_string()],
        "(c) folders"
    );
    assert_eq!(form.lines_text, "*.tmp\n!keep", "(c) lines");
    assert!(form.exclude_caches, "(c) caches");
    assert!(form.exclude_trash, "(c) trash");
    assert!(form.ignore_case, "(c) ignore case");
    assert_eq!(form.markers_text, ".nobackup, CACHEDIR.TAG", "(c) markers");
    assert!(form.size_limit_enabled, "(c) size limit on");
    assert_eq!(
        (form.size_limit_num, form.size_limit_unit),
        (2, SizeUnit::G),
        "(c) size limit"
    );

    let table: [(&str, Option<(u64, SizeUnit)>); 10] = [
        ("500M", Some((500, SizeUnit::M))),
        (" 500 mb ", Some((500, SizeUnit::M))),
        ("1kB", Some((1, SizeUnit::K))),
        ("0t", Some((1, SizeUnit::T))),
        ("3 G", Some((3, SizeUnit::G))),
        ("500", None),
        ("500B", None),
        ("500M B", None),
        ("M", None),
        ("1.5G", None),
    ];
    for (raw, expected) in table {
        let form = ExcludeForm::from_rules(&ExcludeRules {
            exclude_larger_than: Some(raw.to_string()),
            ..ExcludeRules::default()
        });
        let shown = form
            .size_limit_enabled
            .then_some((form.size_limit_num, form.size_limit_unit));
        assert_eq!(shown, expected, "(c) size limit {raw:?}");
        if expected.is_none() {
            assert_eq!(
                (form.size_limit_num, form.size_limit_unit),
                (100, SizeUnit::M),
                "(c) {raw:?} offers 100 MB"
            );
        }
    }
}

/// AC-M6-34-2 (a) and (c): the fields give the rules the QML built (`currentExcludesObj`):
/// lines split at line ends (also `\r\n`), markers split at commas without empty ones, and the
/// size limit only when it is on.
#[test]
fn ac_m6_34_2_exclude_form_writes_like_former_qml() {
    let form = ExcludeForm {
        folders: vec!["/tmp".to_string()],
        lines_text: "*.tmp\r\n\n# note".to_string(),
        exclude_caches: false,
        exclude_trash: true,
        size_limit_enabled: true,
        size_limit_num: 0,
        size_limit_unit: SizeUnit::K,
        ignore_case: false,
        markers_text: " .nobackup ,, x ".to_string(),
    };
    let written = form.to_rules();
    assert_eq!(
        written.lines,
        vec!["*.tmp".to_string(), String::new(), "# note".to_string()],
        "(c) lines, blank ones kept"
    );
    assert_eq!(
        written.exclude_if_present,
        vec![".nobackup".to_string(), "x".to_string()],
        "(c) markers"
    );
    assert_eq!(
        written.exclude_larger_than.as_deref(),
        Some("1K"),
        "(c) 0 counts as 1"
    );
    assert_eq!(written.folders, vec!["/tmp".to_string()], "(c) folders");
    assert!(written.exclude_trash, "(c) trash");

    let empty = ExcludeForm::from_rules(&ExcludeRules::default());
    assert_eq!(empty.to_rules().lines, Vec::<String>::new(), "(c) no lines");
    assert_eq!(
        empty.to_rules().exclude_larger_than,
        None,
        "(c) no size limit"
    );
}

/// AC-M6-34-2 (a) and (c): unchanged rules are not dirty, a blank saved size limit equals none,
/// and the expert rule count skips blank lines and comments.
#[test]
fn ac_m6_34_2_exclude_form_state_detects_changes() {
    let saved = rules(r##"{"lines":["*.tmp","","# c","dir"],"exclude_larger_than":""}"##);
    let form = ExcludeForm::from_rules(&saved);
    let state = form.state(&saved);
    assert!(!state.dirty, "(c) blank size limit = none");
    assert_eq!(state.expert_rule_count, 2, "(c) expert rules");

    let mut changed = form;
    changed.markers_text = ".nobackup".to_string();
    assert!(changed.state(&saved).dirty, "(c) a marker more");
}

/// AC-M6-34-2 (a): the lint's suggestion replaces the first line that is the parent rule.
#[test]
fn ac_m6_34_2_exclude_form_applies_lint_fix() {
    assert_eq!(
        apply_lint_fix("dir\n !dir/inner\n dir ", "dir", "dir/*"),
        "dir/*\n !dir/inner\n dir ",
        "(a) first match only"
    );
    assert_eq!(
        apply_lint_fix("a\r\n dir \r\nb", "dir", "dir/*"),
        "a\ndir/*\nb",
        "(a) trimmed match, lines joined with \\n"
    );
    assert_eq!(apply_lint_fix("a", "dir", "dir/*"), "a", "(a) no match");
}
