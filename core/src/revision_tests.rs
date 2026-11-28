// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::{Backup, Destination, DestinationKind, Source};
use std::path::PathBuf;

fn destination() -> Destination {
    Destination::new(
        "NAS",
        DestinationKind::Local {
            path: PathBuf::from("/mnt/nas"),
        },
    )
    .expect("destination")
}

/// AC-M6-36-7 (b), (c): the revision is the same for the same content, also for a copy read back
/// from its stored form, and changes with every change.
#[test]
fn ac_m6_36_7_revision_follows_the_content() {
    let dest = destination();
    let backup = Backup::new(
        "Home",
        Source::folder(PathBuf::from("/home/u")),
        dest.id.clone(),
    )
    .expect("backup");
    let rev = revision_of(&backup);
    assert_eq!(rev.len(), 64, "SHA-256 as hex");
    assert!(
        rev.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );

    let reread: Backup =
        toml::from_str(&toml::to_string(&backup).expect("to toml")).expect("from toml");
    assert_eq!(revision_of(&reread), rev, "(c) same content, same revision");

    let mut renamed = backup.clone();
    renamed.name = "Home 2".to_string();
    assert_ne!(
        revision_of(&renamed),
        rev,
        "(b) a change gives a new revision"
    );
    renamed.name = backup.name;
    assert_eq!(
        revision_of(&renamed),
        rev,
        "(c) the change undone gives the old revision"
    );

    let mut moved = dest.clone();
    moved.kind = DestinationKind::Local {
        path: PathBuf::from("/mnt/other"),
    };
    assert_ne!(
        revision_of(&moved),
        revision_of(&dest),
        "(b) destinations too"
    );
}
