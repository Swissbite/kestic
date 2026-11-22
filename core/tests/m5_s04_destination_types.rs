// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M5-S04: destination types «ready» or «needs installation», checked against a fake `PATH`
//! (a temp directory with stand-ins for `ssh` and `rclone`). The `Suggests` of the `.deb` are
//! checked in `m0_s05_deb_packaging.rs`, which runs where the `.deb` is built.

use kestic_core::destination_types::{
    DestinationTypeCard, DestinationTypeStatus, RequiredProgram, cards_ready_first,
    destination_types,
};
use kestic_i18n::codes::DestinationTypeCode;
use kestic_test_support::tools::write_executable;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A `PATH` of an empty directory plus an empty entry (current directory) and a missing
/// directory, which the check must both skip.
fn fake_path(dir: &Path) -> OsString {
    std::env::join_paths([dir.to_path_buf(), "".into(), dir.join("missing")]).expect("join PATH")
}

/// `dir` as a path relative to the working directory of the test (`../../tmp/...`).
fn relative_to_cwd(dir: &Path) -> PathBuf {
    let cwd = std::env::current_dir().expect("cwd");
    let mut rel = PathBuf::new();
    for _ in cwd.components().skip(1) {
        rel.push("..");
    }
    rel.push(dir.strip_prefix("/").expect("absolute temp dir"));
    rel
}

fn install(dir: &Path, program: &str) {
    write_executable(&dir.join(program), "#!/bin/sh\nexit 0\n");
}

fn status(types: &[DestinationTypeStatus], kind: DestinationTypeCode) -> &DestinationTypeStatus {
    types
        .iter()
        .find(|t| t.kind == kind)
        .unwrap_or_else(|| panic!("{} missing in {types:?}", kind.as_str()))
}

/// AC-M5-04-1: with `ssh` in `PATH` and no `rclone`, (a) drive, folder, SFTP, REST and the cloud
/// kinds (S3, B2, Azure, Google Cloud, Swift) are ready, (b) rclone needs an installation and
/// (c) every ready card comes before it.
#[test]
fn ac_m5_04_1_ssh_present_rclone_missing_ready_types_first() {
    let dir = TempDir::new().expect("tempdir");
    install(dir.path(), "ssh");
    let types = destination_types(Some(&fake_path(dir.path())));

    assert_eq!(
        types.iter().map(|t| t.kind).collect::<Vec<_>>(),
        DestinationTypeCode::ALL.to_vec(),
        "the daemon lists every type in card order"
    );
    for kind in [
        DestinationTypeCode::Drive,
        DestinationTypeCode::Folder,
        DestinationTypeCode::Sftp,
        DestinationTypeCode::Rest,
        DestinationTypeCode::S3,
        DestinationTypeCode::B2,
        DestinationTypeCode::Azure,
        DestinationTypeCode::Gs,
        DestinationTypeCode::Swift,
    ] {
        let t = status(&types, kind);
        // (a)
        assert!(t.ready, "(a) {} must be ready: {t:?}", kind.as_str());
        assert_eq!(t.missing_program, None, "(a) {}", kind.as_str());
    }

    // (b)
    let rclone = status(&types, DestinationTypeCode::Rclone);
    assert!(!rclone.ready, "(b) rclone must need installation");
    assert_eq!(rclone.missing_program, Some(RequiredProgram::Rclone));
    assert_eq!(rclone.package.as_deref(), Some("rclone"));
    assert_eq!(rclone.appstream_url.as_deref(), Some("appstream://rclone"));

    // (c)
    let cards = cards_ready_first(&types);
    assert_eq!(
        cards.iter().map(|c| c.card).collect::<Vec<_>>(),
        vec![
            DestinationTypeCard::Drive,
            DestinationTypeCard::Folder,
            DestinationTypeCard::Sftp,
            DestinationTypeCard::Rest,
            DestinationTypeCard::Cloud,
            DestinationTypeCard::Rclone,
        ],
        "(c) ready cards first, rclone last"
    );
    let first_not_ready = cards.iter().position(|c| !c.ready).expect("rclone card");
    assert!(
        cards[first_not_ready..].iter().all(|c| !c.ready),
        "(c) no ready card after the first card that needs installation: {cards:?}"
    );
    assert_eq!(
        cards[4].kinds,
        vec![
            DestinationTypeCode::S3,
            DestinationTypeCode::B2,
            DestinationTypeCode::Azure,
            DestinationTypeCode::Gs,
            DestinationTypeCode::Swift,
        ],
        "the cloud card groups the five cloud kinds"
    );
}

/// AC-M5-04-1 (c), the other way round: when `ssh` is missing, the ready cards that came after
/// SFTP in the daemon's list move before it.
#[test]
fn ac_m5_04_1_ready_cards_move_before_a_card_that_needs_installation() {
    let dir = TempDir::new().expect("tempdir");
    let types = destination_types(Some(&fake_path(dir.path())));
    let cards = cards_ready_first(&types);
    assert_eq!(
        cards.iter().map(|c| c.card).collect::<Vec<_>>(),
        vec![
            DestinationTypeCard::Drive,
            DestinationTypeCard::Folder,
            DestinationTypeCard::Rest,
            DestinationTypeCard::Cloud,
            DestinationTypeCard::Sftp,
            DestinationTypeCard::Rclone,
        ]
    );
}

/// AC-M5-04-2: without `ssh` in `PATH`, (a) «NAS / Server via SSH» needs an installation of
/// `openssh-client`.
#[test]
fn ac_m5_04_2_ssh_missing_sftp_needs_installation() {
    let dir = TempDir::new().expect("tempdir");
    install(dir.path(), "rclone");
    // A non-executable `ssh` does not count.
    fs::write(dir.path().join("ssh"), "#!/bin/sh\n").expect("write ssh");
    fs::set_permissions(dir.path().join("ssh"), fs::Permissions::from_mode(0o644)).expect("chmod");
    let types = destination_types(Some(&fake_path(dir.path())));

    let sftp = status(&types, DestinationTypeCode::Sftp);
    // (a)
    assert!(!sftp.ready, "(a) SFTP must need installation: {sftp:?}");
    assert_eq!(sftp.missing_program, Some(RequiredProgram::Ssh));
    assert_eq!(sftp.package.as_deref(), Some("openssh-client"));
    assert!(
        status(&types, DestinationTypeCode::Rclone).ready,
        "rclone is in PATH"
    );
    let card = cards_ready_first(&types)
        .into_iter()
        .find(|c| c.card == DestinationTypeCard::Sftp)
        .expect("SFTP card");
    assert_eq!(
        card.install_command.as_deref(),
        Some("sudo apt install openssh-client")
    );
}

/// AC-M5-04-2 with programs restic could not start: (a) an `ssh` only others may execute
/// (mode 0o011) and an `ssh` found through a relative `PATH` entry both leave SFTP needing
/// installation.
#[test]
fn ac_m5_04_2_ssh_not_executable_by_user_or_relative_is_missing() {
    let dir = TempDir::new().expect("tempdir");
    write_executable(&dir.path().join("ssh"), "#!/bin/sh\nexit 0\n");
    fs::set_permissions(dir.path().join("ssh"), fs::Permissions::from_mode(0o011)).expect("chmod");
    let types = destination_types(Some(&fake_path(dir.path())));
    // (a)
    assert!(
        !status(&types, DestinationTypeCode::Sftp).ready,
        "(a) an ssh this user may not execute must not count"
    );

    let rel_dir = TempDir::new().expect("tempdir");
    install(rel_dir.path(), "ssh");
    let relative = relative_to_cwd(rel_dir.path());
    assert!(
        relative.join("ssh").is_file(),
        "precondition: the relative entry reaches ssh"
    );
    let types = destination_types(Some(relative.as_os_str()));
    // (a)
    assert!(
        !status(&types, DestinationTypeCode::Sftp).ready,
        "(a) an ssh found through a relative PATH entry must not count"
    );
    let absolute = rel_dir.path().as_os_str();
    assert!(
        status(
            &destination_types(Some(absolute)),
            DestinationTypeCode::Sftp
        )
        .ready,
        "the same ssh counts through its absolute directory"
    );
}

/// AC-M5-04-2 without any `PATH`: nothing can be found, so SFTP and rclone need installation.
#[test]
fn ac_m5_04_2_no_path_means_programs_missing() {
    let types = destination_types(None);
    assert!(!status(&types, DestinationTypeCode::Sftp).ready);
    assert!(!status(&types, DestinationTypeCode::Rclone).ready);
}

/// AC-M5-04-3 (data of the entry): the rclone card names the package `rclone`, the Discover
/// link and the command `sudo apt install rclone`. The GUI smoke shows them.
#[test]
fn ac_m5_04_3_rclone_card_carries_package_discover_link_and_command() {
    let dir = TempDir::new().expect("tempdir");
    install(dir.path(), "ssh");
    let card = cards_ready_first(&destination_types(Some(&fake_path(dir.path()))))
        .into_iter()
        .find(|c| c.card == DestinationTypeCard::Rclone)
        .expect("rclone card");
    // (a)
    assert_eq!(card.package.as_deref(), Some("rclone"));
    // (b)
    assert_eq!(card.appstream_url.as_deref(), Some("appstream://rclone"));
    // (c)
    assert_eq!(
        card.install_command.as_deref(),
        Some("sudo apt install rclone")
    );
}

/// AC-M5-04-4: rclone put into `PATH` afterwards; (a) the next check reports it ready.
#[test]
fn ac_m5_04_4_check_again_finds_rclone_added_later() {
    let dir = TempDir::new().expect("tempdir");
    install(dir.path(), "ssh");
    let path = fake_path(dir.path());
    assert!(
        !status(&destination_types(Some(&path)), DestinationTypeCode::Rclone).ready,
        "precondition: rclone missing"
    );

    install(dir.path(), "rclone");
    let again = destination_types(Some(&path));
    let rclone = status(&again, DestinationTypeCode::Rclone);
    // (a)
    assert!(rclone.ready, "(a) rclone must be ready after it was added");
    assert_eq!(rclone.missing_program, None);
    assert_eq!(rclone.package, None);
    assert!(
        cards_ready_first(&again).iter().all(|c| c.ready),
        "every card is ready now"
    );
}
