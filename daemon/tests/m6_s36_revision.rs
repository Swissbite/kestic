// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M6-S36 revision against lost updates: `GetState` reports a revision per backup and
//! destination; `UpdateBackup` and `UpdateDestination` save only with the current one and answer
//! `Conflict` otherwise (AC-M6-36-7 to AC-M6-36-9).

mod common;

use common::m3::{M3DaemonOpts, M3Rig, assert_dbus_error_name, latest_variant, spawn_m3_daemon};
use common::revision::{backup_revision, destination_revision};
use common::{isolated_secret_store, load_config};
use kestic_core::model::{Backup, Destination, MaintenanceEntrySchedule, MaintenanceSchedule};
use kestic_core::secrets::SecretStore;
use kestic_daemon::dbus_api::{BackendSecrets, KESTIC_OBJECT_PATH, KesticClient};
use kestic_i18n::codes::DestinationUpdateOutcome;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;
use time::macros::datetime;

const CONFLICT: &str = "net.daester.Kestic.Error.Conflict";

fn rig(root: &Path, base: &str) -> M3Rig {
    M3Rig::start(
        root,
        &latest_variant(),
        base,
        datetime!(2026-12-01 10:00:00 UTC),
        |o| {
            o.enable_monitors = false;
            o.enable_scheduler = false;
        },
    )
}

fn second_client(rig: &M3Rig, base: &str) -> KesticClient {
    KesticClient::connect(&rig.variant.bus_name(base), KESTIC_OBJECT_PATH).expect("second client")
}

fn saved_backup(rig: &M3Rig, backup: &Backup) -> Backup {
    load_config(&rig.daemon.config_path)
        .expect("load config")
        .backups
        .into_iter()
        .find(|b| b.id == backup.id)
        .expect("saved backup")
}

fn saved_destination(rig: &M3Rig, dest: &Destination) -> Destination {
    load_config(&rig.daemon.config_path)
        .expect("load config")
        .destinations
        .into_iter()
        .find(|d| d.id == dest.id)
        .expect("saved destination")
}

fn renamed<T: Clone>(value: &T, rename: impl FnOnce(&mut T)) -> T {
    let mut copy = value.clone();
    rename(&mut copy);
    copy
}

/// Restic processes the daemon started that touch a repository, so far.
fn restic_starts(rig: &M3Rig) -> usize {
    rig.wrapper.started_repo_count()
}

/// AC-M6-36-7 (a)–(e): every backup and destination has a revision; it follows each saved change,
/// whatever method made it, stays the same for the same content also over a restart, is not in
/// `config.toml`, and secrets do not change it.
#[test]
fn ac_m6_36_7_revision_in_get_state() {
    common::run_isolated_test(
        "ac_m6_36_7_revision_in_get_state",
        "KESTIC_M6_36_7_INNER",
        |root| {
            let rig = rig(root, "S36R7");
            let client = &rig.daemon.client;
            let fx = rig.add_backup("Docs", |_| {});
            let other = rig.add_destination("Other", |_| {});
            let rev = || backup_revision(client, &fx.backup);
            let dest_rev = || destination_revision(client, &fx.dest);

            // (a)
            for value in [rev(), dest_rev()] {
                assert_eq!(
                    value.len(),
                    64,
                    "AC-M6-36-7 (a): a SHA-256 revision: {value:?}"
                );
                assert!(
                    value.chars().all(|c| c.is_ascii_hexdigit()),
                    "AC-M6-36-7 (a)"
                );
            }
            // (b) each method that saves a change moves the revision on.
            let before = rev();
            client
                .rename_backup(fx.backup_id_str.as_str(), "Docs 2")
                .expect("rename");
            let after_rename = rev();
            assert_ne!(after_rename, before, "AC-M6-36-7 (b): RenameBackup");
            client
                .add_exclude_rule_for_item(
                    fx.backup_id_str.as_str(),
                    &format!("{}/data.txt", fx.src_dir.display()),
                )
                .expect("add exclude rule");
            let after_exclude = rev();
            assert_ne!(
                after_exclude, after_rename,
                "AC-M6-36-7 (b): AddExcludeRuleForItem"
            );
            client
                .reassign_backup(fx.backup_id_str.as_str(), other.id.as_str())
                .expect("reassign");
            let after_reassign = rev();
            assert_ne!(
                after_reassign, after_exclude,
                "AC-M6-36-7 (b): ReassignBackup"
            );
            let dest_before = dest_rev();
            let maintenance = MaintenanceSchedule {
                prune: MaintenanceEntrySchedule::Daily { hour: 3, minute: 0 },
                ..MaintenanceSchedule::default()
            };
            client
                .update_destination_maintenance(fx.dest.id.as_str(), &maintenance)
                .expect("maintenance");
            let dest_after = dest_rev();
            assert_ne!(
                dest_after, dest_before,
                "AC-M6-36-7 (b): UpdateDestinationMaintenance"
            );
            // (e) a password alone does not change the destination's revision.
            client
                .update_destination_password(fx.dest.id.as_str(), &fx.password)
                .expect("update stored password");
            assert_eq!(
                dest_rev(),
                dest_after,
                "AC-M6-36-7 (e): secrets are not part of it"
            );
            // (d) not in config.toml.
            let config = fs::read_to_string(&rig.daemon.config_path).expect("read config");
            assert!(
                !config.contains(&after_reassign) && !config.contains(&dest_after),
                "AC-M6-36-7 (d): the revision is not stored"
            );
            // (c) the same after a restart.
            let M3Rig {
                daemon, wrapper, ..
            } = rig;
            daemon.stop();
            let restarted = spawn_m3_daemon(
                root,
                M3DaemonOpts::on_request(
                    &latest_variant().sub("S36R7"),
                    &latest_variant().bus_name("S36R7"),
                    &wrapper.path,
                ),
            );
            assert_eq!(
                backup_revision(&restarted.client, &fx.backup),
                after_reassign,
                "AC-M6-36-7 (c): same content, same revision after a restart"
            );
            assert_eq!(
                destination_revision(&restarted.client, &fx.dest),
                dest_after,
                "AC-M6-36-7 (c): also for the destination"
            );
            restarted.stop();
        },
    );
}

/// AC-M6-36-8 (a)–(f): `UpdateBackup` saves only with the current revision.
#[test]
fn ac_m6_36_8_update_backup_needs_the_current_revision() {
    common::run_isolated_test(
        "ac_m6_36_8_update_backup_needs_the_current_revision",
        "KESTIC_M6_36_8_INNER",
        |root| {
            let rig = rig(root, "S36R8");
            let first = &rig.daemon.client;
            let second = second_client(&rig, "S36R8");
            let fx = rig.add_backup("Docs", |_| {});
            let read = backup_revision(first, &fx.backup);

            // (a)
            let mine = renamed(&fx.backup, |b| b.name = "First".to_string());
            let new_rev = first.update_backup(&mine, &read).expect("first save");
            assert_ne!(new_rev, read, "AC-M6-36-8 (a): a new revision comes back");
            assert_eq!(
                new_rev,
                backup_revision(first, &fx.backup),
                "AC-M6-36-8 (a)"
            );
            // (b)
            let theirs = renamed(&fx.backup, |b| b.name = "Second".to_string());
            let err = second
                .update_backup(&theirs, &read)
                .expect_err("stale save");
            assert_dbus_error_name(&err, CONFLICT);
            // (c)
            assert_eq!(
                saved_backup(&rig, &fx.backup).name,
                "First",
                "AC-M6-36-8 (c)"
            );
            // (d)
            second
                .update_backup(&theirs, &new_rev)
                .expect("AC-M6-36-8 (d): saves with the new revision");
            assert_eq!(
                saved_backup(&rig, &fx.backup).name,
                "Second",
                "AC-M6-36-8 (d)"
            );
            // (e) eight at once with one revision: exactly one saves.
            let current = backup_revision(first, &fx.backup);
            let results: Vec<bool> = thread::scope(|scope| {
                let handles: Vec<_> = (0..8)
                    .map(|i| {
                        let edit = renamed(&fx.backup, |b| b.name = format!("Racer {i}"));
                        let current = current.clone();
                        let bus = rig.variant.bus_name("S36R8");
                        scope.spawn(move || {
                            let client =
                                KesticClient::connect(&bus, KESTIC_OBJECT_PATH).expect("client");
                            client.update_backup(&edit, &current).is_ok()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().expect("racer"))
                    .collect()
            });
            assert_eq!(
                results.iter().filter(|ok| **ok).count(),
                1,
                "AC-M6-36-8 (e): exactly one of eight saves: {results:?}"
            );
            // (f) an empty or unknown revision saves nothing.
            let before = saved_backup(&rig, &fx.backup);
            for revision in ["", "zz", &current] {
                let attempt = renamed(&fx.backup, |b| b.name = "Never".to_string());
                let err = first
                    .update_backup(&attempt, revision)
                    .expect_err("AC-M6-36-8 (f): refused");
                assert_dbus_error_name(&err, CONFLICT);
            }
            assert_eq!(
                saved_backup(&rig, &fx.backup),
                before,
                "AC-M6-36-8 (f): nothing saved"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M6-36-9 (a), (b), (e), (f): `UpdateDestination` saves only with the current revision and
/// refuses a stale one before any restic call.
#[test]
fn ac_m6_36_9_update_destination_needs_the_current_revision() {
    common::run_isolated_test(
        "ac_m6_36_9_update_destination_needs_the_current_revision",
        "KESTIC_M6_36_9_INNER",
        |root| {
            let rig = rig(root, "S36R9");
            let first = &rig.daemon.client;
            let second = second_client(&rig, "S36R9");
            let dest = rig.add_destination("Vault", |_| {});
            let none = BackendSecrets::default();
            let read = destination_revision(first, &dest);

            // (a)
            let mine = renamed(&dest, |d| d.name = "Vault A".to_string());
            let saved = first
                .update_destination(&mine, "connect", "", &none, false, &read)
                .expect("first save");
            assert_eq!(
                saved.outcome,
                DestinationUpdateOutcome::Saved,
                "AC-M6-36-9 (a)"
            );
            assert_ne!(
                saved.revision, read,
                "AC-M6-36-9 (a): a new revision comes back"
            );
            assert_eq!(
                saved.revision,
                destination_revision(first, &dest),
                "AC-M6-36-9 (a)"
            );
            // (b) refused before restic: also a save that would check the connection.
            let starts = restic_starts(&rig);
            let theirs = renamed(&dest, |d| d.name = "Vault B".to_string());
            let err = second
                .update_destination(&theirs, "connect", "", &none, true, &read)
                .expect_err("stale save");
            assert_dbus_error_name(&err, CONFLICT);
            assert_eq!(
                restic_starts(&rig),
                starts,
                "AC-M6-36-9 (b): no restic call"
            );
            assert_eq!(
                saved_destination(&rig, &dest).name,
                "Vault A",
                "AC-M6-36-9 (b)"
            );
            // (e) eight at once: exactly one saves.
            let current = destination_revision(first, &dest);
            let results: Vec<bool> = thread::scope(|scope| {
                let handles: Vec<_> = (0..8)
                    .map(|i| {
                        let edit = renamed(&dest, |d| d.name = format!("Racer {i}"));
                        let current = current.clone();
                        let bus = rig.variant.bus_name("S36R9");
                        scope.spawn(move || {
                            let client =
                                KesticClient::connect(&bus, KESTIC_OBJECT_PATH).expect("client");
                            client
                                .update_destination(
                                    &edit,
                                    "connect",
                                    "",
                                    &BackendSecrets::default(),
                                    false,
                                    &current,
                                )
                                .is_ok()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().expect("racer"))
                    .collect()
            });
            assert_eq!(
                results.iter().filter(|ok| **ok).count(),
                1,
                "AC-M6-36-9 (e): exactly one of eight saves: {results:?}"
            );
            // (f) an empty or unknown revision: refused, no restic, nothing saved.
            let before = saved_destination(&rig, &dest);
            let starts = restic_starts(&rig);
            for revision in ["", "zz"] {
                let attempt = renamed(&dest, |d| d.name = "Never".to_string());
                let err = first
                    .update_destination(&attempt, "connect", "", &none, true, revision)
                    .expect_err("AC-M6-36-9 (f): refused");
                assert_dbus_error_name(&err, CONFLICT);
            }
            assert_eq!(
                restic_starts(&rig),
                starts,
                "AC-M6-36-9 (f): no restic call"
            );
            assert_eq!(saved_destination(&rig, &dest), before, "AC-M6-36-9 (f)");
            rig.daemon.stop();
        },
    );
}

/// AC-M6-36-9 (c), (d): a change of the destination while its connection check runs ends the save
/// with `Conflict`, a removal with `NotFound`; neither writes `config.toml` or a secret.
#[test]
fn ac_m6_36_9_c_d_change_during_the_connection_check() {
    common::run_isolated_test(
        "ac_m6_36_9_c_d_change_during_the_connection_check",
        "KESTIC_M6_36_9C_INNER",
        |root| {
            let rig = rig(root, "S36R9C");
            let client = &rig.daemon.client;
            let dest = rig.add_destination("Vault", |_| {});
            let spare = rig.add_destination("Spare", |_| {});
            let store = isolated_secret_store();
            let password_before = store
                .get_secret(dest.id.as_str(), "repo")
                .expect("read secret")
                .map(|s| s.as_str().expect("utf-8").to_string());

            for (round, (target, change)) in [(&dest, "maintenance"), (&spare, "remove")]
                .into_iter()
                .enumerate()
            {
                let read = destination_revision(client, target);
                let before = load_config(&rig.daemon.config_path).expect("config");
                rig.wrapper.hold("cat");
                let checked = renamed(target, |d| d.name = format!("{} renamed", d.name));
                let bus = rig.variant.bus_name("S36R9C");
                let saving = thread::spawn(move || {
                    let other = KesticClient::connect(&bus, KESTIC_OBJECT_PATH).expect("client");
                    other.update_destination(
                        &checked,
                        "connect",
                        "",
                        &BackendSecrets::default(),
                        true,
                        &read,
                    )
                });
                rig.wrapper
                    .wait_for('S', "cat", round + 1, Duration::from_secs(10));
                if change == "maintenance" {
                    let maintenance = MaintenanceSchedule {
                        prune: MaintenanceEntrySchedule::Daily { hour: 4, minute: 0 },
                        ..MaintenanceSchedule::default()
                    };
                    client
                        .update_destination_maintenance(target.id.as_str(), &maintenance)
                        .expect("change during the check");
                } else {
                    client
                        .remove_destination(target.id.as_str())
                        .expect("remove during the check");
                }
                rig.wrapper.release("cat");
                let err = saving.join().expect("saving thread").expect_err("refused");
                let expected = if change == "maintenance" {
                    CONFLICT
                } else {
                    "net.daester.Kestic.Error.NotFound"
                };
                assert_dbus_error_name(&err, expected);
                let after = load_config(&rig.daemon.config_path).expect("config");
                let names = |c: &kestic_core::config::AppConfig| {
                    c.destinations
                        .iter()
                        .map(|d| d.name.clone())
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    names(&after),
                    names(&before)
                        .into_iter()
                        .filter(|n| change != "remove" || n != &target.name)
                        .collect::<Vec<_>>(),
                    "AC-M6-36-9 (d): the refused save wrote no name ({change})"
                );
                rig.wrapper
                    .wait_for('E', "cat", round + 1, Duration::from_secs(10));
            }
            let password_after = store
                .get_secret(dest.id.as_str(), "repo")
                .expect("read secret")
                .map(|s| s.as_str().expect("utf-8").to_string());
            assert_eq!(
                password_after, password_before,
                "AC-M6-36-9 (d): no secret written"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M6-36-9 (g): `init` with a stale revision runs no `restic init` and leaves no repository.
#[test]
fn ac_m6_36_9_g_init_with_a_stale_revision_creates_nothing() {
    common::run_isolated_test(
        "ac_m6_36_9_g_init_with_a_stale_revision_creates_nothing",
        "KESTIC_M6_36_9G_INNER",
        |root| {
            let rig = rig(root, "S36R9G");
            let client = &rig.daemon.client;
            let dest = rig.add_destination("Gone", |_| {});
            let repo_dir = root.join(&rig.sub).join("repo-Gone");
            let stale = destination_revision(client, &dest);
            client
                .update_destination_maintenance(
                    dest.id.as_str(),
                    &MaintenanceSchedule {
                        prune: MaintenanceEntrySchedule::Daily { hour: 5, minute: 0 },
                        ..MaintenanceSchedule::default()
                    },
                )
                .expect("change elsewhere");
            // The repository there was deleted: `init` would create it anew.
            fs::remove_dir_all(&repo_dir).expect("delete repository");
            fs::create_dir_all(&repo_dir).expect("empty folder");
            let err = client
                .update_destination(
                    &dest,
                    "init",
                    "new-password",
                    &BackendSecrets::default(),
                    true,
                    &stale,
                )
                .expect_err("refused");
            assert_dbus_error_name(&err, CONFLICT);
            assert!(
                rig.wrapper.of('S', "init").is_empty(),
                "AC-M6-36-9 (g): no restic init: {:?}",
                rig.wrapper.entries()
            );
            assert!(
                !repo_dir.join("config").exists(),
                "AC-M6-36-9 (g): no repository at the location"
            );
            rig.daemon.stop();
        },
    );
}
