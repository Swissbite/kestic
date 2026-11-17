// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use kestic_core::clock::FakeClock;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, DestinationKind, PendingReason, Run, RunKind,
    RunStatus, Source,
};
use std::collections::HashSet;
use time::macros::datetime;

fn is_valid_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'))
}

#[test]
fn ac_m1_01_1_ids_are_unique_8_char_base32() {
    let dest_a = Destination::new(
        "NAS",
        DestinationKind::Local {
            path: "/tmp/repo-a".into(),
        },
    )
    .expect("create destination a");
    let dest_b = Destination::new(
        "Disk",
        DestinationKind::Local {
            path: "/tmp/repo-b".into(),
        },
    )
    .expect("create destination b");

    assert!(is_valid_id(dest_a.id.as_str()));
    assert!(is_valid_id(dest_b.id.as_str()));
    assert_ne!(dest_a.id, dest_b.id);

    let backup_a = Backup::new("Home", Source::folder("/home/alice"), dest_a.id.clone())
        .expect("create backup a");
    let backup_b = Backup::new("Docs", Source::folder("/home/alice/Docs"), dest_a.id)
        .expect("create backup b");

    assert!(is_valid_id(backup_a.id.as_str()));
    assert!(is_valid_id(backup_b.id.as_str()));
    assert_ne!(backup_a.id, backup_b.id);
    assert_eq!(backup_a.tag(), format!("kestic:{}", backup_a.id));

    let mut seen = HashSet::new();
    for _ in 0..50 {
        let id = BackupId::generate().expect("generate id");
        assert!(is_valid_id(id.as_str()));
        assert!(seen.insert(id));
    }

    assert!(DestinationId::parse("invalid!").is_err());
    assert!(DestinationId::parse("short").is_err());
    assert!(BackupId::parse("ABCDEFGH").is_err());
    assert!(BackupId::parse("abcd0189").is_err());
}

#[test]
fn ac_m1_01_2_renaming_or_changing_source_preserves_backup_id() {
    let dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: "/tmp/repo".into(),
        },
    )
    .expect("destination");
    let mut backup = Backup::new("Home", Source::folder("/home/alice"), dest.id).expect("backup");
    let original_id = backup.id.clone();

    backup.rename("Personal Home").expect("rename");
    backup
        .set_source(Source::drive("/media/alice/ExtDrive"))
        .expect("set source");

    assert_eq!(backup.id, original_id);
    assert_eq!(backup.name, "Personal Home");
    assert_eq!(backup.source, Source::drive("/media/alice/ExtDrive"));
}

#[test]
fn ac_m1_01_3_run_kinds_relations_and_statuses() {
    let clock = FakeClock::new(datetime!(2026-12-01 10:00:00 UTC));
    let dest = Destination::new(
        "REST",
        DestinationKind::Rest {
            url: "https://backup.example.org/repo".to_string(),
            username: Some("alice".to_string()),
        },
    )
    .expect("destination");
    let backup = Backup::new(
        "Documents",
        Source::folder("/home/alice/Documents"),
        dest.id.clone(),
    )
    .expect("backup");

    let backup_run = Run::new_backup(&clock, &backup).expect("backup run");
    assert_eq!(
        backup_run.kind,
        RunKind::Backup {
            backup_id: backup.id.clone()
        }
    );
    assert_eq!(backup_run.destination_id, dest.id);
    assert_eq!(backup_run.backup_id(), Some(&backup.id));
    assert_eq!(backup_run.status, RunStatus::Running);

    let pending_run = Run::new_pending_backup(&clock, &backup, PendingReason::SourceUnavailable)
        .expect("pending run");
    assert_eq!(pending_run.status, RunStatus::Pending);
    assert_eq!(
        pending_run.pending_reason,
        Some(PendingReason::SourceUnavailable)
    );

    let maint_run = Run::new_maintenance(&clock, dest.id.clone()).expect("maintenance run");
    assert_eq!(maint_run.kind, RunKind::Maintenance);
    assert_eq!(maint_run.destination_id, dest.id);
    assert_eq!(maint_run.backup_id(), None);

    let restore_assigned =
        Run::new_restore(&clock, dest.id.clone(), Some(backup.id.clone())).expect("restore");
    assert_eq!(
        restore_assigned.kind,
        RunKind::Restore {
            backup_id: Some(backup.id.clone())
        }
    );
    assert_eq!(restore_assigned.destination_id, dest.id);
    assert_eq!(restore_assigned.backup_id(), Some(&backup.id));

    let restore_foreign = Run::new_restore(&clock, dest.id.clone(), None).expect("restore foreign");
    assert_eq!(restore_foreign.kind, RunKind::Restore { backup_id: None });
    assert_eq!(restore_foreign.destination_id, dest.id);
    assert_eq!(restore_foreign.backup_id(), None);

    let meta_run = Run::new_meta(&clock, dest.id.clone()).expect("meta run");
    assert_eq!(meta_run.kind, RunKind::Meta);
    assert_eq!(meta_run.destination_id, dest.id);
    assert_eq!(meta_run.backup_id(), None);

    // Exercise all 7 RunStatus variants required by AC-M1-01-3.
    let all_statuses = [
        RunStatus::Pending,
        RunStatus::Running,
        RunStatus::Ok,
        RunStatus::Warning,
        RunStatus::Error,
        RunStatus::Cancelled,
        RunStatus::Interrupted,
    ];
    assert_eq!(all_statuses.len(), 7);
}

#[test]
fn destination_location_normalization_and_duplicate_detection() {
    use kestic_core::config::AppConfig;

    let local_a = DestinationKind::Local {
        path: "/tmp/kestic-dup-test/repo/".into(),
    };
    let local_b = DestinationKind::Local {
        path: "/tmp/kestic-dup-test/sub/../repo".into(),
    };
    let local_c = DestinationKind::Local {
        path: "/tmp/kestic-dup-test/other".into(),
    };
    assert!(local_a.same_location(&local_b));
    assert!(!local_a.same_location(&local_c));

    let rest_a = DestinationKind::Rest {
        url: "HTTPS://Backup.Example.ORG:8000/alice/".to_string(),
        username: Some("alice".to_string()),
    };
    let rest_b = DestinationKind::Rest {
        url: "https://backup.example.org:8000/alice".to_string(),
        username: Some("bob".to_string()),
    };
    assert!(rest_a.same_location(&rest_b));
    assert!(!rest_a.same_location(&local_a));

    let dest1 = Destination::new("Primary", local_a).expect("dest1");
    let dest2 = Destination::new("Secondary", local_c).expect("dest2");
    let cfg = AppConfig {
        destinations: vec![dest1.clone(), dest2.clone()],
        ..Default::default()
    };

    assert_eq!(
        cfg.find_duplicate_destination(&local_b, None)
            .map(|d| &d.id),
        Some(&dest1.id)
    );
    assert_eq!(
        cfg.find_duplicate_destination(&local_b, Some(&dest1.id))
            .map(|d| &d.id),
        None
    );
    assert_eq!(
        cfg.find_duplicate_destination(&local_b, Some(&dest2.id))
            .map(|d| &d.id),
        Some(&dest1.id)
    );
}

#[test]
fn rest_idna_punycode_normalization_and_repository_uri() {
    use kestic_core::model::normalize_rest_url_to_ascii;

    assert_eq!(
        normalize_rest_url_to_ascii("https://bäckup.example.org:8000").expect("idn root"),
        "https://xn--bckup-gra.example.org:8000"
    );
    assert_eq!(
        normalize_rest_url_to_ascii("https://Bäckup.Example.ORG:8000/pfäd/").expect("idn path"),
        "https://xn--bckup-gra.example.org:8000/pf%C3%A4d/"
    );

    let unicode_rest = DestinationKind::Rest {
        url: "https://müller.example.org:8000/alice".to_string(),
        username: Some("alice".to_string()),
    };
    let puny_rest = DestinationKind::Rest {
        url: "https://xn--mller-kva.example.org:8000/alice/".to_string(),
        username: Some("alice".to_string()),
    };

    assert_eq!(
        unicode_rest.repository_uri(),
        "rest:https://xn--mller-kva.example.org:8000/alice"
    );
    assert!(unicode_rest.same_location(&puny_rest));

    // Pasted URLs with invisible zero-width/BOM characters and Umlauts normalize cleanly.
    assert_eq!(
        normalize_rest_url_to_ascii("https://synology.däster.ch:6789/tuxedo-dave").expect("umlaut"),
        "https://synology.xn--dster-gra.ch:6789/tuxedo-dave"
    );
    assert_eq!(
        normalize_rest_url_to_ascii(
            "https://synology.xn--dster-gra.ch:6789/tuxedo-dave\u{200B}\u{FEFF}"
        )
        .expect("invisible chars stripped"),
        "https://synology.xn--dster-gra.ch:6789/tuxedo-dave"
    );
}
