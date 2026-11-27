// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S23: the AppStream metainfo as it lies in the repository and how `just verify` builds and
//! checks it. The shipped file is checked in `m0_s05_deb_packaging` (its tests need the `.deb`).

use kestic_test_support::metainfo::TEMPLATE;
use kestic_test_support::workspace::workspace_root;
use std::fs;

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Body of the recipe `name` in the justfile: the indented lines after `name:`.
fn recipe(justfile: &str, name: &str) -> String {
    let header = format!("{name}:");
    justfile
        .lines()
        .skip_while(|line| *line != header)
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with(' '))
        .collect::<Vec<_>>()
        .join("\n")
}

/// AC-M6-23-2 (b): `just verify` builds the `.deb` with `just deb`, and that recipe validates the
/// metainfo with `appstreamcli` (warnings fail it) before packaging it.
#[test]
fn ac_m6_23_2_just_verify_validates_metainfo() {
    let justfile = read("justfile");
    let verify = recipe(&justfile, "verify");
    let deb = recipe(&justfile, "deb");
    assert!(
        verify.lines().any(|l| l.trim() == "just deb"),
        "(b) `just verify` does not run `just deb`:\n{verify}"
    );
    let validate = deb
        .find("appstreamcli validate --no-net")
        .unwrap_or_else(|| panic!("(b) `just deb` does not run appstreamcli validate:\n{deb}"));
    let package = deb.find("cargo deb ").expect("`just deb` runs cargo deb");
    assert!(
        validate < package,
        "(b) the metainfo must be validated before it is packaged"
    );
}

/// AC-M6-23-3 (a): no metainfo file in the repository has a `<release>` entry written by hand;
/// the entry comes from the build version (`build-stamp/src/metainfo.rs`).
#[test]
fn ac_m6_23_3_repository_has_no_hand_written_release() {
    let packaging = workspace_root().join("packaging");
    let metainfo_files: Vec<_> = fs::read_dir(&packaging)
        .expect("read packaging/")
        .map(|e| e.expect("entry").path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".metainfo.xml") || n.ends_with(".appdata.xml"))
        })
        .collect();
    assert_eq!(
        metainfo_files,
        vec![workspace_root().join(TEMPLATE)],
        "exactly one metainfo template"
    );
    let template = read(TEMPLATE);
    assert!(
        !template.contains("<release"),
        "(a) {TEMPLATE} must not contain a <release> or <releases> entry"
    );
}

/// The lines `name`, `source`, `caption` of `packaging/screenshots.tsv`.
fn screenshot_list() -> Vec<(String, String, String)> {
    read("packaging/screenshots.tsv")
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let fields: Vec<&str> = l.split('\t').collect();
            assert_eq!(fields.len(), 3, "malformed line {l:?}");
            (
                fields[0].to_owned(),
                fields[1].to_owned(),
                fields[2].to_owned(),
            )
        })
        .collect()
}

/// AC-M6-23-4 (b): the screenshots come from one list in the repository: the template has none of
/// its own, every name is a fixed `screenshot-….png` independent of the test names, every source
/// is a screenshot the GUI smoke tests write (`smoke_artifact("<name>", …)` in
/// `daemon/tests/m<n>_gui_smoke.rs`), and every caption has a German translation.
#[test]
fn ac_m6_23_4_screenshots_come_from_one_list() {
    assert!(
        !read(TEMPLATE).contains("<screenshot"),
        "(b) {TEMPLATE} must not list screenshots of its own"
    );
    let list = screenshot_list();
    assert!(!list.is_empty(), "(b) the list names screenshots");
    let catalog: std::collections::HashMap<String, String> =
        kestic_test_support::po::catalog(&workspace_root())
            .into_iter()
            .filter(|e| !e.plural)
            .map(|e| (e.msgid().to_owned(), e.german()[0].clone()))
            .collect();
    let mut names = std::collections::BTreeSet::new();
    for (name, source, caption) in &list {
        let stem = name
            .strip_prefix("screenshot-")
            .and_then(|n| n.strip_suffix(".png"));
        assert!(
            stem.is_some_and(|s| !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')),
            "(b) {name} is no fixed screenshot-….png name"
        );
        assert!(names.insert(name.clone()), "(b) {name} listed twice");
        assert_ne!(name, source, "(b) the name must not be the test file name");

        let (milestone, rest) = source.split_once('-').expect("source m<n>-…");
        let rest = rest.strip_suffix(".png").expect("source is a PNG");
        let mut parts = rest.rsplitn(3, '-');
        let (lang, layout, screen) = (parts.next(), parts.next(), parts.next());
        let smoke = read(&format!("daemon/tests/{milestone}_gui_smoke.rs"));
        assert!(
            screen.is_some_and(|s| smoke.contains(&format!("smoke_artifact(\"{s}\"")))
                && layout.is_some_and(|l| l == "wide" || l == "narrow")
                && lang.is_some_and(|l| l == "en" || l == "de"),
            "(b) {source} is no screenshot of daemon/tests/{milestone}_gui_smoke.rs"
        );
        assert!(
            catalog.get(caption).is_some_and(|de| !de.is_empty()),
            "(b) caption {caption:?} has no German translation in po/de/kestic.po"
        );
    }
}
