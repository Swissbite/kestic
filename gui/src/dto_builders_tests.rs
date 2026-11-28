// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `dto_builders` (kept in their own file, NFR-12 file size).

use super::*;
use kestic_core::model::*;

fn sample_backup() -> Backup {
    let dest = Destination::new(
        "Vault",
        DestinationKind::Local {
            path: "/srv/vault".into(),
        },
    )
    .expect("dest");
    Backup::new("Documents", Source::folder("/home/u/docs"), dest.id).expect("backup")
}

/// A new backup on a local destination gets no network run conditions; on a network
/// destination it keeps the defaults (AC-M3-03-1).
#[test]
fn m3_acceptance_new_backup_has_network_conditions_only_on_network_destinations() {
    let local = DestinationKind::Local {
        path: "/srv/vault".into(),
    };
    let rest = DestinationKind::Rest {
        url: "https://nas.example.invalid/kestic".to_string(),
        username: None,
    };
    let dest_id = sample_backup().destination_id.as_str().to_string();

    let on_local =
        build_new_backup("Docs", "/home/u/docs", &dest_id, "", None, Some(&local)).expect("local");
    let on_rest =
        build_new_backup("Docs", "/home/u/docs", &dest_id, "", None, Some(&rest)).expect("rest");

    // (a) local: power yes, network no.
    assert!(on_local.run_conditions.require_ac_power);
    assert!(!on_local.run_conditions.require_network);
    // (b) network destination: power and network (AC-M3-03-1).
    assert!(on_rest.run_conditions.require_ac_power);
    assert!(on_rest.run_conditions.require_network);
}

/// Saving the run conditions of a backup on a local destination drops network conditions.
#[test]
fn m3_acceptance_saved_conditions_drop_network_for_local_destination() {
    let local = DestinationKind::Local {
        path: "/srv/vault".into(),
    };
    let edited = RunConditions {
        require_ac_power: true,
        require_network: true,
        skip_on_metered_network: true,
        allowed_networks: vec!["Heim".to_string()],
    };

    let updated =
        clone_updated_backup_conditions(&sample_backup(), edited, Some(&local)).expect("updated");

    assert!(!updated.run_conditions.require_network);
    assert!(!updated.run_conditions.skip_on_metered_network);
    assert!(updated.run_conditions.allowed_networks.is_empty());
}

/// The restic source from the settings: every known kind, and an unknown one is refused instead
/// of silently becoming «System».
#[test]
fn restic_source_kinds_are_parsed_and_unknown_ones_refused() {
    assert_eq!(
        build_provider("automatic", ""),
        Ok(ResticProvider::Automatic)
    );
    assert_eq!(build_provider("system", ""), Ok(ResticProvider::System));
    assert_eq!(
        build_provider("app_managed", ""),
        Ok(ResticProvider::AppManaged)
    );
    assert_eq!(
        build_provider("custom", " /opt/restic "),
        Ok(ResticProvider::Custom {
            path: PathBuf::from("/opt/restic")
        })
    );
    assert!(build_provider("apm_managed", "").is_err());
}
