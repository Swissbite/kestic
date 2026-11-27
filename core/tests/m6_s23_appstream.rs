// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S23: the AppStream metainfo as it lies in the repository and how `just verify` builds and
//! checks it. The shipped file is checked in `m0_s05_deb_packaging` (its tests need the `.deb`).

use kestic_test_support::metainfo::{TEMPLATE, elements};
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

/// Value of `key: "…"` in the site's Astro configuration.
fn astro_setting(config: &str, key: &str) -> String {
    let start = config
        .find(&format!("{key}: \""))
        .unwrap_or_else(|| panic!("no {key} in site/astro.config.ts"))
        + key.len()
        + 3;
    let len = config[start..].find('"').expect("closing quote");
    config[start..start + len].to_owned()
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

/// AC-M6-23-4: the screenshots of the metainfo (a) are images of the website (its address and
/// base from `site/astro.config.ts`, the folder the site copies them to) and (b) are listed in
/// `site/src/screenshots.ts`, so the site keeps publishing them under the same address.
#[test]
fn ac_m6_23_4_screenshots_are_site_images() {
    let config = read("site/astro.config.ts");
    let prefix = format!(
        "{}{}/screenshots/",
        astro_setting(&config, "site"),
        astro_setting(&config, "base")
    );
    assert!(
        read("site/src/integrations/screenshots.ts")
            .contains("join(fileURLToPath(config.publicDir), \"screenshots\")"),
        "the site no longer publishes screenshots under /screenshots/"
    );
    let site_shots = read("site/src/screenshots.ts");
    let images = elements(&read(TEMPLATE), "image");
    assert!(images.len() >= 3, "expected several screenshots");
    for image in images {
        let Some(file) = image.text.strip_prefix(&prefix) else {
            panic!(
                "(a) {} is not an image of the website ({prefix}…)",
                image.text
            );
        };
        assert!(
            site_shots.contains(&format!("file: \"{file}\"")),
            "(b) {file} is not listed in site/src/screenshots.ts"
        );
    }
}
