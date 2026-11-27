// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M6-S23: the AppStream metainfo in the `.deb`, so that software centres such as Discover show
//! name, developer and licence.

use super::{deb_control, find_deb_package};
use kestic_test_support::metainfo::{SHIPPED, elements, text};
use kestic_test_support::po::catalog;
use kestic_test_support::tools::short_tempdir;
use kestic_test_support::workspace::workspace_root;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Unpacks the `.deb` into a fresh temp directory; returns it and the unpacked root.
fn unpack_deb(tag: &str) -> (tempfile::TempDir, PathBuf) {
    let unpacked = short_tempdir(tag);
    let root = unpacked.path().join("root");
    let status = Command::new("dpkg-deb")
        .arg("-x")
        .arg(find_deb_package())
        .arg(&root)
        .status()
        .expect("run dpkg-deb -x");
    assert!(status.success(), "dpkg-deb -x failed");
    (unpacked, root)
}

fn shipped_metainfo(root: &std::path::Path) -> String {
    fs::read_to_string(root.join(SHIPPED))
        .unwrap_or_else(|e| panic!("(a) the .deb has no /{SHIPPED}: {e}"))
}

/// The German catalog: English msgid → German translation.
fn german() -> HashMap<String, String> {
    catalog(&workspace_root())
        .into_iter()
        .filter(|e| !e.plural)
        .map(|e| (e.msgid().to_owned(), e.german()[0].clone()))
        .collect()
}

/// Every untranslated `<tag>` in `xml` is followed by its German translation from the catalog.
fn assert_translated(xml: &str, tag: &str, catalog: &HashMap<String, String>) {
    let all = elements(xml, tag);
    let originals: Vec<_> = all.iter().filter(|e| e.lang().is_none()).collect();
    assert!(!originals.is_empty(), "(b) no <{tag}> in the metainfo");
    for original in originals {
        let expected = catalog.get(&original.text).unwrap_or_else(|| {
            panic!("(b) <{tag}> {:?} missing in po/de/kestic.po", original.text)
        });
        assert!(
            all.iter()
                .any(|e| e.lang() == Some("de") && e.text == *expected),
            "(b) <{tag}> {:?} has no German translation {expected:?}",
            original.text
        );
    }
}

/// `xml` without the `<developer>` block, whose `<name>` is not the name of the component.
fn without_developer(xml: &str) -> String {
    let start = xml.find("<developer").expect("(b) <developer> block");
    let end = xml[start..].find("</developer>").expect("</developer>") + start;
    format!("{}{}", &xml[..start], &xml[end + "</developer>".len()..])
}

/// AC-M6-23-1: the `.deb` contains (a) the metainfo with the component ID of the `.desktop` file
/// it ships, and (b) name, summary and description in English with their German translations
/// from `po/de/kestic.po`, the developer, both licences, icon, launchable, the URLs for homepage,
/// bug tracker and help (the handbook) and the content rating.
#[test]
fn ac_m6_23_1_deb_ships_appstream_metainfo() {
    let (_unpacked, root) = unpack_deb("kestic-m6-s23-content");
    let xml = shipped_metainfo(&root);
    let desktop = "net.daester.Kestic.desktop";
    assert!(
        root.join("usr/share/applications").join(desktop).is_file(),
        "the .deb ships {desktop}"
    );
    assert_eq!(
        text(&xml, "id").as_deref(),
        desktop.strip_suffix(".desktop"),
        "(a) component ID = ID of the .desktop file"
    );

    let catalog = german();
    let component_name = without_developer(&xml);
    assert_eq!(
        text(&component_name, "name").as_deref(),
        Some("Kestic"),
        "(b) name"
    );
    assert_translated(&component_name, "name", &catalog);
    assert!(
        text(&xml, "summary").is_some_and(|s| !s.is_empty()),
        "(b) summary"
    );
    assert_translated(&xml, "summary", &catalog);
    let description = text(&xml, "description").expect("(b) description");
    assert!(
        elements(&description, "p").len() >= 2,
        "(b) description has paragraphs"
    );
    assert_translated(&description, "p", &catalog);
    assert_translated(&description, "li", &catalog);
    let developer = text(&xml, "developer").expect("(b) developer");
    assert_eq!(
        text(&developer, "name").as_deref(),
        Some("David Däster"),
        "(b) developer"
    );
    assert_eq!(
        text(&xml, "project_license").as_deref(),
        Some("GPL-3.0-or-later"),
        "(b) project_license"
    );
    assert_eq!(
        text(&xml, "metadata_license").as_deref(),
        Some("CC0-1.0"),
        "(b) metadata_license"
    );
    let icon = elements(&xml, "icon");
    assert_eq!(icon.len(), 1, "(b) one icon");
    assert_eq!(icon[0].attr("type"), Some("stock"), "(b) icon type");
    assert!(
        root.join("usr/share/icons/hicolor/scalable/apps")
            .join(format!("{}.svg", icon[0].text))
            .is_file(),
        "(b) the icon {:?} is shipped",
        icon[0].text
    );
    let launchable = elements(&xml, "launchable");
    assert_eq!(launchable.len(), 1, "(b) one launchable");
    assert_eq!(
        launchable[0].attr("type"),
        Some("desktop-id"),
        "(b) launchable type"
    );
    assert_eq!(launchable[0].text, desktop, "(b) launchable");
    let urls: HashMap<String, String> = elements(&xml, "url")
        .into_iter()
        .map(|u| (u.attr("type").unwrap_or_default().to_owned(), u.text))
        .collect();
    assert_eq!(
        urls.get("homepage").map(String::as_str),
        Some("https://swissbite.github.io/kestic/"),
        "(b) homepage"
    );
    assert_eq!(
        urls.get("bugtracker").map(String::as_str),
        Some("https://github.com/Swissbite/kestic/issues"),
        "(b) bug tracker"
    );
    assert_eq!(
        urls.get("help").map(String::as_str),
        Some("https://swissbite.github.io/kestic/docs/"),
        "(b) help: the handbook"
    );
    let rating = elements(&xml, "content_rating");
    assert_eq!(rating.len(), 1, "(b) content_rating");
    assert_eq!(
        rating[0].attr("type"),
        Some("oars-1.1"),
        "(b) content_rating type"
    );
}

/// AC-M6-23-2 (a): `appstreamcli validate` accepts the shipped metainfo with exit 0 and no
/// warning. Without network, so the result does not depend on the website being reachable.
#[test]
fn ac_m6_23_2_shipped_metainfo_validates_without_warnings() {
    let (_unpacked, root) = unpack_deb("kestic-m6-s23-validate");
    let out = Command::new("appstreamcli")
        .args(["validate", "--no-net", "--no-color"])
        .arg(root.join(SHIPPED))
        .output()
        .expect("run appstreamcli (package appstream, scripts/bootstrap.sh)");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "(a) exit {:?}:\n{report}",
        out.status.code()
    );
    let warnings: Vec<&str> = report.lines().filter(|l| l.starts_with("W:")).collect();
    assert!(warnings.is_empty(), "(a) warnings: {warnings:#?}");
}

/// AC-M6-23-3 (b): the `.deb` has one release entry, for exactly its own version (the package
/// version without the Debian revision), generated by `just deb` from the Git tags or the
/// `KESTIC_VERSION` the release workflow sets from the tag.
#[test]
fn ac_m6_23_3_deb_has_release_of_its_version() {
    let control = deb_control();
    let version = control
        .lines()
        .find_map(|l| l.strip_prefix("Version: "))
        .and_then(|v| v.trim().strip_suffix("-1"))
        .expect("package version with revision -1")
        .to_owned();
    let (_unpacked, root) = unpack_deb("kestic-m6-s23-release");
    let releases = elements(&shipped_metainfo(&root), "release");
    assert_eq!(releases.len(), 1, "(b) exactly one release entry");
    assert_eq!(
        releases[0].attr("version"),
        Some(version.as_str()),
        "(b) release entry for the package version"
    );
    assert!(
        releases[0].attr("date").is_some_and(|d| d.len() == 10),
        "(b) release entry has a date"
    );
}

/// AC-M6-23-4 (a): the screenshots of the shipped metainfo are assets of the package's own
/// release (`v<version>` with `~` as `-`); a development build (`+` in the version, as in the CI
/// runs of `just verify`) and a pre-release of an integration branch (`~m<n>.<k>`, published
/// without screenshots) ship none.
#[test]
fn ac_m6_23_4_deb_screenshots_are_assets_of_its_own_release() {
    let control = deb_control();
    let version = control
        .lines()
        .find_map(|l| l.strip_prefix("Version: "))
        .and_then(|v| v.trim().strip_suffix("-1"))
        .expect("package version with revision -1")
        .to_owned();
    let (_unpacked, root) = unpack_deb("kestic-m6-s23-screenshots");
    let images = elements(&shipped_metainfo(&root), "image");
    let integration_pre_release = version.split_once("~m").is_some_and(|(_, rest)| {
        rest.split('.')
            .all(|p| p.bytes().all(|b| b.is_ascii_digit()))
    });
    if version.contains('+') || version.starts_with("0.0.0~dev") || integration_pre_release {
        assert!(
            images.is_empty(),
            "(a) a build without release ships no screenshots"
        );
    } else {
        let prefix = format!(
            "https://github.com/Swissbite/kestic/releases/download/v{}/screenshot-",
            version.replace('~', "-")
        );
        assert!(!images.is_empty(), "(a) a release build ships screenshots");
        for image in images {
            assert!(
                image.text.starts_with(&prefix),
                "(a) {} is no asset of the release of {version}",
                image.text
            );
        }
    }
}
