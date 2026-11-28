// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S19: an SFTP path with a `..` folder in it is refused where it is entered, and a config
//! saved with such a path still loads.

use kestic_core::config::parse_config_str;
use kestic_core::model::{Destination, DestinationId, DestinationKind, ModelError};
use kestic_core::sftp;
use kestic_i18n::messages::SFTP_PATH_WITH_PARENT;
use std::path::Path;

fn sftp_dest(path: &str) -> Destination {
    Destination::new(
        "Server",
        DestinationKind::Sftp {
            host: "backup.example.org".to_string(),
            port: 22,
            user: "anna".to_string(),
            path: path.to_string(),
        },
    )
    .expect("an SFTP destination passes the load-time rules")
}

/// AC-M6-19-1 (a), in `core`: a path with a whole `..` folder fails the input rules with the
/// message.
#[test]
fn ac_m6_19_1_parent_segment_fails_the_input_rules() {
    for path in [
        "/srv/x/../repo",
        "../repo",
        "backups/..",
        " /srv/../repo ",
        "/srv//../repo",
    ] {
        let err = sftp_dest(path)
            .validate_input()
            .expect_err("`..` folder refused");
        assert_eq!(
            err,
            ModelError::ParentSegmentInSftpPath,
            "(a) refusal: {path}"
        );
        assert_eq!(
            err.to_string(),
            SFTP_PATH_WITH_PARENT,
            "(a) message: {path}"
        );
    }
}

/// AC-M6-19-2, the rule behind the form: only a whole `..` folder is refused; a name with two dots
/// in it, or a `.` folder, stays allowed.
#[test]
fn ac_m6_19_2_only_a_whole_parent_segment_is_refused() {
    assert_eq!(
        sftp::refuse_parent_segments("/srv/x/../repo"),
        Err(ModelError::ParentSegmentInSftpPath),
        "whole `..` folder"
    );
    for path in [
        "/srv/a..b/repo",
        "/srv/..repo",
        "backups../x",
        "/srv/./repo",
        "/srv/.../repo",
    ] {
        assert_eq!(
            sftp::refuse_parent_segments(path),
            Ok(()),
            "allowed: {path}"
        );
        assert_eq!(
            sftp_dest(path).validate_input(),
            Ok(()),
            "input rules allow: {path}"
        );
    }
}

/// AC-M6-19-3 (a): a config file whose SFTP path has a `..` folder loads without error, with the
/// path as it was saved.
#[test]
fn ac_m6_19_3_config_with_parent_segment_loads() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m6/config_sftp_parent.toml");
    let raw = std::fs::read_to_string(fixture).expect("read fixture");
    let config = parse_config_str(&raw).expect("(a) the config loads");
    let id = DestinationId::parse("sftpdots").expect("id");
    let dest = config
        .destination(&id)
        .expect("(a) the destination is there");
    assert_eq!(
        dest.kind,
        DestinationKind::Sftp {
            host: "backup.invalid".to_string(),
            port: 22,
            user: "anna".to_string(),
            path: "/srv/x/../repo".to_string(),
        },
        "(a) the path as saved"
    );
}
