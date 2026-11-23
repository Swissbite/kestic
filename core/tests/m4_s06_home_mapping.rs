// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S06 without restic: the original location of a snapshot of another user's home, mapped
//! to the home of the current user.

use kestic_core::restore::RestoreTarget;
use kestic_core::restore_target::{HomeMapping, check_target, map_to_home, original_location};
use kestic_core::snapshot::ResticSnapshot;
use kestic_i18n::codes::RestoreTargetCode;
use std::path::{Path, PathBuf};

fn snapshot(paths: &[&str], username: Option<&str>) -> ResticSnapshot {
    serde_json::from_value(serde_json::json!({
        "id": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "time": "2026-11-15T12:00:00Z",
        "paths": paths,
        "username": username,
    }))
    .expect("snapshot json")
}

/// A temporary home folder named `neu`, holding the folders `dirs`.
fn home_neu(dirs: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("neu");
    std::fs::create_dir_all(&home).expect("mkdir home");
    for dir in dirs {
        std::fs::create_dir_all(home.join(dir)).expect("mkdir below home");
    }
    (tmp, home)
}

fn mapped(dir: &str, from: &str, to: &str) -> HomeMapping {
    HomeMapping::Mapped {
        dir: dir.to_string(),
        from: from.to_string(),
        to: to.to_string(),
    }
}

/// AC-M4-06-1 (unit part): for `/home/alt/Dokumente/a.txt` and the user `neu`, the original
/// location (a) is `Dokumente` below the user's home (`/home/neu/Dokumente`, where `a.txt` lands
/// as `/home/neu/Dokumente/a.txt`), and (b) carries the mapping `alt → neu`.
#[test]
fn ac_m4_06_1_other_users_home_is_mapped_to_the_users_home() {
    let HomeMapping::Mapped { dir, from, to } = map_to_home(
        "/home/alt/Dokumente",
        Some("alt"),
        Some(Path::new("/home/neu")),
    ) else {
        panic!("/home/alt/Dokumente is mapped");
    };
    assert_eq!(dir, "/home/neu/Dokumente", "(a) below the user's home");
    assert_eq!(
        Path::new(&dir).join("a.txt"),
        Path::new("/home/neu/Dokumente/a.txt"),
        "(a) where a.txt lands"
    );
    assert_eq!(
        (from.as_str(), to.as_str()),
        ("alt", "neu"),
        "(b) mapping alt → neu"
    );

    let (_tmp, home) = home_neu(&["Dokumente"]);
    let snap = snapshot(&["/home/alt/Dokumente/a.txt"], Some("alt"));
    let original = original_location("/home/alt/Dokumente", &snap, Some(&home));
    assert_eq!(
        original.mapped_dir.as_deref(),
        Some(home.join("Dokumente").to_string_lossy().as_ref()),
        "(a) the suggestion is the folder in the user's home"
    );
    assert!(!original.needs_user_choice, "(a) the folder exists: usable");
    assert_eq!(
        original.original_dir, "/home/alt/Dokumente",
        "(b) the original folder is named"
    );
    assert_eq!(
        original.mapping,
        Some(("alt".to_string(), "neu".to_string())),
        "(b) the suggestion carries alt → neu"
    );
}

/// AC-M4-06-3: a folder outside `/home/<name>/` (`/media/daten/Fotos`) (a) stays as it is.
#[test]
fn ac_m4_06_3_folder_outside_home_stays() {
    let home = Path::new("/home/neu");
    assert_eq!(
        map_to_home("/media/daten/Fotos", Some("alt"), Some(home)),
        HomeMapping::Unchanged,
        "(a) not mapped"
    );
    let snap = snapshot(&["/media/daten/Fotos"], Some("alt"));
    let original = original_location("/media/daten/Fotos", &snap, Some(home));
    assert_eq!(
        original.mapped_dir.as_deref(),
        Some("/media/daten/Fotos"),
        "(a) the original location is the folder itself"
    );
    assert_eq!(original.mapping, None, "(a) no mapping is shown");
}

/// AC-M4-06-4 (unit part): `/home/alt/Projekte` of user `alt`, user `neu` without `Projekte`:
/// (a) the original location needs a chosen folder, (b) checking it never creates
/// `/home/neu/Projekte`, and (d) the check of that target reports `missing_mapped_dir`.
#[test]
fn ac_m4_06_4_missing_mapped_folder_needs_a_chosen_folder() {
    let (_tmp, home) = home_neu(&[]);
    let snap = snapshot(&["/home/alt/Projekte"], Some("alt"));
    let original = original_location("/home/alt/Projekte", &snap, Some(&home));
    let projekte = home.join("Projekte");
    assert!(original.needs_user_choice, "(a) a folder has to be chosen");
    assert_eq!(
        original.mapped_dir.as_deref(),
        Some(projekte.to_string_lossy().as_ref()),
        "(a) the missing folder is named"
    );
    let check = check_target(
        &RestoreTarget::Original {
            dir: projekte.to_string_lossy().into_owned(),
        },
        &[],
    );
    assert_eq!(
        check.code,
        RestoreTargetCode::MissingMappedDir,
        "(d) the check reports the missing folder"
    );
    assert!(
        std::fs::symlink_metadata(&projekte).is_err(),
        "(b) Projekte is not created"
    );
}

/// AC-M4-06-5: with the home `/var/home/neu`, `/home/alt/Dokumente` (a) becomes
/// `/var/home/neu/Dokumente`, not `/home/neu/…`.
#[test]
fn ac_m4_06_5_mapping_uses_the_real_home_folder() {
    assert_eq!(
        map_to_home(
            "/home/alt/Dokumente",
            Some("alt"),
            Some(Path::new("/var/home/neu"))
        ),
        mapped("/var/home/neu/Dokumente", "alt", "neu"),
        "(a)"
    );
}

/// AC-M4-06-6: paths below `/home/alt/` in a snapshot whose `username` is not `alt`: the
/// suggestion (a) has `needs_user_choice` and (b) no `mapped_dir`. The old name comes from the
/// selected paths and is checked against `snapshot.username`.
#[test]
fn ac_m4_06_6_other_snapshot_user_needs_a_chosen_folder() {
    let (_tmp, home) = home_neu(&["Dokumente"]);
    for user in [Some("root"), Some("altx"), None] {
        let snap = snapshot(&["/home/alt/Dokumente"], user);
        let original = original_location("/home/alt/Dokumente/sub", &snap, Some(&home));
        assert!(original.needs_user_choice, "(a) username {user:?}");
        assert_eq!(original.mapped_dir, None, "(b) username {user:?}");
        assert_eq!(original.mapping, None, "(b) username {user:?}: no mapping");
    }
}

/// AC-M4-06-7: `/home/alt/../neu/x`, `/home/alt/..`, `/home/altx/a.txt` and `/home/alt` (a) are
/// never mapped outside the user's home, and (b) `/home/altx/a.txt` is not mapped as a path of
/// `alt`.
#[test]
fn ac_m4_06_7_mapping_never_leaves_the_users_home() {
    let home = Path::new("/home/neu");
    let map = |path: &str| map_to_home(path, Some("alt"), Some(home));
    assert_eq!(
        map("/home/alt/../neu/x"),
        HomeMapping::Unmappable,
        "(a) `..` below the old home"
    );
    assert_eq!(
        map("/home/alt/.."),
        HomeMapping::Unmappable,
        "(a) `..` as the last segment"
    );
    assert_eq!(
        map("/home/alt"),
        mapped("/home/neu", "alt", "neu"),
        "(a) the old home itself becomes the user's home"
    );
    assert_eq!(
        map("/home/altx/a.txt"),
        HomeMapping::Unmappable,
        "(b) altx is not alt"
    );
    for path in ["/home/alt/../neu/x", "/home/alt/..", "/home/altx/a.txt"] {
        let snap = snapshot(&[path], Some("alt"));
        let original = original_location(path, &snap, Some(home));
        assert_eq!(original.mapped_dir, None, "(a) {path}: no target");
    }
}

/// M4-S06: a folder in the user's own home stays, also when the snapshot was made by another
/// user (e.g. by root); there is nothing to map.
#[test]
fn m4_s06_own_home_is_not_mapped() {
    let home = Path::new("/home/neu");
    for user in [Some("neu"), Some("root"), None] {
        assert_eq!(
            map_to_home("/home/neu/Dokumente", user, Some(home)),
            HomeMapping::Unchanged,
            "{user:?}"
        );
    }
}
