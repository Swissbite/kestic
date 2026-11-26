// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M5-S09 editing the connection of a destination, over D-Bus against an in-process daemon on an
//! isolated session bus: the connection test and `UpdateDestination` read the repository ID at
//! the edited connection and save only when it is the destination's own repository. Local
//! folders, a REST server on a new port, SFTP (new host, new path), S3 (`rclone serve s3`) and the
//! generic form (an rclone remote on the local disk).

mod common;

use common::m3::{
    M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, add_folder_backup, add_local_destination,
    assert_dbus_error_name, latest_variant, restic_variants, seed_local_destination,
    spawn_m3_daemon, wait_for_state,
};
use common::{isolated_secret_store, load_config, run_isolated_test};
use kestic_core::backend_credentials::{SECRET_KIND_S3_ACCESS_KEY, SECRET_KIND_S3_SECRET_KEY};
use kestic_core::generic::{EnvVar, secret_kind};
use kestic_core::model::{BackupId, Destination, DestinationKind, RunStatus};
use kestic_core::restic::SECRET_KIND_REPO;
use kestic_core::secrets::SecretStore;
use kestic_core::sftp::{KnownHostsPaths, set_test_ssh_config};
use kestic_daemon::dbus_api::{BackendSecrets, ConnectionTestResultDto};
use kestic_i18n::codes::{ConnectionTestCode, DestinationUpdateOutcome};
use kestic_i18n::messages::{UiText, connection_test_text};
use kestic_test_support::password::{s3_key, strong_password};
use kestic_test_support::repo_template::copy_template_repo;
use kestic_test_support::s3_server::{S3ServerGuard, rclone_binary};
use kestic_test_support::sftp_server::SftpServerGuard;
use kestic_test_support::tools::{RestServerGuard, free_loopback_port, write_executable};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// Runs restic directly on a repository (password from the environment, no cache).
fn restic(bin: &Path, repo: &str, password: &str, args: &[&str]) -> Output {
    Command::new(bin)
        .args(["-r", repo, "--no-cache"])
        .args(args)
        .env("RESTIC_PASSWORD", password)
        .env_remove("RESTIC_PASSWORD_COMMAND")
        .output()
        .expect("run restic")
}

fn restic_ok(bin: &Path, repo: &str, password: &str, args: &[&str]) -> String {
    let out = restic(bin, repo, password, args);
    assert!(
        out.status.success(),
        "restic {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `(id, parent)` of the newest snapshot in `repo`.
fn newest_snapshot(bin: &Path, repo: &str, password: &str) -> (String, Option<String>) {
    let json = restic_ok(bin, repo, password, &["snapshots", "--json"]);
    let snaps: Vec<serde_json::Value> = serde_json::from_str(&json).expect("snapshots JSON");
    let newest = snaps
        .iter()
        .max_by_key(|s| s["time"].as_str().unwrap_or_default().to_string())
        .expect("a snapshot");
    (
        newest["id"].as_str().expect("id").to_string(),
        newest["parent"].as_str().map(ToOwned::to_owned),
    )
}

/// A new repository at `dir` with `password` (its own `init`, so its own ID).
fn init_repo(bin: &Path, dir: &Path, password: &str) {
    restic_ok(bin, &dir.to_string_lossy(), password, &["init"]);
}

fn copy_dir(from: &Path, to: &Path) {
    let status = Command::new("cp")
        .arg("-r")
        .arg(from)
        .arg(to)
        .status()
        .expect("cp -r");
    assert!(
        status.success(),
        "cp -r {} {}",
        from.display(),
        to.display()
    );
}

fn local(dest: &Destination, path: &Path) -> Destination {
    let mut edited = dest.clone();
    edited.kind = DestinationKind::Local {
        path: path.to_path_buf(),
    };
    edited
}

fn none() -> BackendSecrets {
    BackendSecrets::default()
}

fn config_bytes(daemon: &M3Daemon) -> Option<Vec<u8>> {
    fs::read(&daemon.config_path).ok()
}

/// The saved form of the destination with `id`.
fn saved(daemon: &M3Daemon, id: &Destination) -> Destination {
    load_config(&daemon.config_path)
        .expect("config")
        .destination(&id.id)
        .cloned()
        .expect("destination in config")
}

/// `edited` leads to a repository of its own `init`: the test reports another backup
/// destination, `UpdateDestination` saves nothing and `config.toml` stays as `before`.
fn assert_other_repository(
    daemon: &M3Daemon,
    edited: &Destination,
    before: &Option<Vec<u8>>,
    ac: &str,
) {
    let tested = daemon
        .client
        .test_destination_connection(edited, "connect", "", &none())
        .expect("TestDestinationConnection");
    assert_eq!(
        tested.error_kind,
        Some(ConnectionTestCode::OtherRepository),
        "{ac} the repository ID is compared: {tested:?}"
    );
    let result = daemon
        .client
        .update_destination(edited, "connect", "", &none(), true)
        .expect("UpdateDestination");
    assert_eq!(
        result.outcome,
        DestinationUpdateOutcome::OtherRepository,
        "{ac} another repository is not saved"
    );
    assert_eq!(&config_bytes(daemon), before, "{ac} config unchanged");
}

fn english(result: &ConnectionTestResultDto) -> String {
    match connection_test_text(result.ok, result.error_kind, &result.detail) {
        UiText::Message(m) => m.english(),
        UiText::Raw(raw) => raw,
    }
}

fn spawn(root: &Path, variant: &ResticVariant, base: &str) -> (M3Daemon, ResticWrapper, String) {
    let sub = variant.sub(base);
    let bus = variant.bus_name(base);
    let wrapper = ResticWrapper::create(&root.join(&sub).join("wrapper"), &variant.bin);
    let daemon = spawn_m3_daemon(root, M3DaemonOpts::on_request(&sub, &bus, &wrapper.path));
    (daemon, wrapper, sub)
}

/// Starts a backup and waits until it ended with `Ok`.
fn back_up(daemon: &M3Daemon, backup_id: &BackupId) {
    let run_id = daemon
        .client
        .start_backup(backup_id.as_str())
        .expect("StartBackup");
    wait_for_state(
        &daemon.client,
        Duration::from_secs(60),
        "backup ok",
        |state| {
            state
                .recent_runs
                .iter()
                .any(|r| r.id.as_str() == run_id && r.status == RunStatus::Ok)
        },
    );
}

/// AC-M5-05-4 (FR-56, U18): a local destination whose path is changed and saved. (a) Kestic opens
/// the repository at the new path with the saved password, (b) compares the repository ID, (c)
/// `UpdateDestination` saves only when both pass, and (d) otherwise the configuration stays
/// unchanged. The new folder must also be writable. Runs with restic 0.18.0 and the latest
/// restic.
#[test]
fn ac_m5_05_4_local_path_change_checks_password_and_repository_id() {
    run_isolated_test(
        "ac_m5_05_4_local_path_change_checks_password_and_repository_id",
        "KESTIC_M5_09_054_INNER",
        |root| {
            for variant in restic_variants() {
                let (daemon, _wrapper, sub) = spawn(root, &variant, "M5E054");
                let dir = root.join(&sub);
                let client = &daemon.client;
                let (dest, password) =
                    add_local_destination(client, &variant.bin, &dir.join("repo"), "Disk", |_| {});

                // (a) a repository the saved password does not open.
                copy_template_repo(&variant.bin, &dir.join("foreign"));
                let before = config_bytes(&daemon);
                let foreign = local(&dest, &dir.join("foreign"));
                let tested = client
                    .test_destination_connection(&foreign, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::WrongPassword),
                    "(a) {tested:?}"
                );
                let err = client
                    .update_destination(&foreign, "connect", "", &none(), true)
                    .expect_err("(a) UpdateDestination with a password that does not fit");
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.WrongPassword");
                // (d)
                assert_eq!(config_bytes(&daemon), before, "(d) after a wrong password");

                // (b) the saved password opens another repository (its own `init`).
                init_repo(&variant.bin, &dir.join("other"), &password);
                let other = local(&dest, &dir.join("other"));
                let tested = client
                    .test_destination_connection(&other, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::OtherRepository),
                    "(b) {tested:?}"
                );
                let result = client
                    .update_destination(&other, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(
                    result.outcome,
                    DestinationUpdateOutcome::OtherRepository,
                    "(b) another repository is not saved"
                );
                // (d)
                assert_eq!(
                    config_bytes(&daemon),
                    before,
                    "(d) after another repository"
                );

                // A copy of the repository in a folder Kestic cannot write to.
                let read_only = dir.join("read-only");
                copy_dir(&dir.join("repo"), &read_only);
                fs::set_permissions(&read_only, fs::Permissions::from_mode(0o555)).expect("chmod");
                let tested = client
                    .test_destination_connection(&local(&dest, &read_only), "connect", "", &none())
                    .expect("TestDestinationConnection");
                fs::set_permissions(&read_only, fs::Permissions::from_mode(0o755))
                    .expect("chmod back");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::NotWritable),
                    "a folder that is not writable: {tested:?}"
                );

                // (c) the same repository at the new path is saved.
                copy_dir(&dir.join("repo"), &dir.join("moved"));
                let moved = local(&dest, &dir.join("moved"));
                let tested = client
                    .test_destination_connection(&moved, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert!(tested.ok, "(c) the test passes: {tested:?}");
                let result = client
                    .update_destination(&moved, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(result.outcome, DestinationUpdateOutcome::Saved, "(c)");
                assert_eq!(saved(&daemon, &dest).kind, moved.kind, "(c) new path saved");
                daemon.stop();
            }
        },
    );
}

/// AC-M5-05-5 (FR-56, P42): the repository folder of a local destination copied with `cp -r` to
/// a new path; changing the connection to it (a) finds the same repository ID, (b) saves without
/// another question, and (c) the next backup finds its parent. Runs with restic 0.18.0 and the
/// latest restic.
#[test]
fn ac_m5_05_5_copied_repository_keeps_its_id_and_parent() {
    run_isolated_test(
        "ac_m5_05_5_copied_repository_keeps_its_id_and_parent",
        "KESTIC_M5_09_055_INNER",
        |root| {
            for variant in restic_variants() {
                let (daemon, wrapper, sub) = spawn(root, &variant, "M5E055");
                let dir = root.join(&sub);
                let client = &daemon.client;
                let (dest, password) =
                    add_local_destination(client, &variant.bin, &dir.join("repo"), "Disk", |_| {});
                let backup = add_folder_backup(client, &dir.join("src"), &dest.id, "Docs", |_| {});
                back_up(&daemon, &backup.id);
                let (before_move, _) =
                    newest_snapshot(&variant.bin, &dir.join("repo").to_string_lossy(), &password);

                let copy = dir.join("copy");
                copy_dir(&dir.join("repo"), &copy);
                let moved = local(&dest, &copy);
                // (a)
                let calls_before_test = wrapper.entries().len();
                let tested = client
                    .test_destination_connection(&moved, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert!(tested.ok, "(a) the same repository: {tested:?}");
                assert!(
                    wrapper.entries()[calls_before_test..]
                        .iter()
                        .any(|e| e.kind == 'S'
                            && e.sub == "cat"
                            && e.detail.split_whitespace().any(|arg| arg == "config")),
                    "(a) the repository ID comes from `restic cat config`"
                );
                // (b)
                let calls_before = wrapper.entries().len();
                let result = client
                    .update_destination(&moved, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(result.outcome, DestinationUpdateOutcome::Saved, "(b)");
                assert_eq!(saved(&daemon, &dest).kind, moved.kind, "(b) saved");
                assert!(
                    wrapper.entries().len() > calls_before,
                    "(b) UpdateDestination checked the repository itself"
                );
                // (c)
                back_up(&daemon, &backup.id);
                let (_, parent) = newest_snapshot(&variant.bin, &copy.to_string_lossy(), &password);
                assert_eq!(
                    parent,
                    Some(before_move),
                    "(c) the parent is the snapshot before"
                );
                daemon.stop();
            }
        },
    );
}

/// AC-M5-05-7 (FR-56, P42): at the new path lies a repository of its own `init`, filled with
/// `restic copy` from the destination's repository. Changing the connection to it (a) is not
/// saved, (b) is reported as another backup destination; (c) the GUI offers «Als neues Ziel
/// hinzufügen» with the entered values (GUI smoke). Runs with restic 0.18.0 and the latest
/// restic.
#[test]
fn ac_m5_05_7_other_repository_is_refused() {
    run_isolated_test(
        "ac_m5_05_7_other_repository_is_refused",
        "KESTIC_M5_09_057_INNER",
        |root| {
            for variant in restic_variants() {
                let (daemon, _wrapper, sub) = spawn(root, &variant, "M5E057");
                let dir = root.join(&sub);
                let client = &daemon.client;
                let repo = dir.join("repo");
                let (dest, password) =
                    add_local_destination(client, &variant.bin, &repo, "Disk", |_| {});
                let backup = add_folder_backup(client, &dir.join("src"), &dest.id, "Docs", |_| {});
                back_up(&daemon, &backup.id);

                let copied = dir.join("copied");
                init_repo(&variant.bin, &copied, &password);
                let out = Command::new(&variant.bin)
                    .args(["-r", &copied.to_string_lossy(), "--no-cache", "copy"])
                    .args(["--from-repo", &repo.to_string_lossy()])
                    .env("RESTIC_PASSWORD", &password)
                    .env("RESTIC_FROM_PASSWORD", &password)
                    .output()
                    .expect("restic copy");
                assert!(out.status.success(), "restic copy: {out:?}");

                let before = config_bytes(&daemon);
                let edited = local(&dest, &copied);
                let tested = client
                    .test_destination_connection(&edited, "connect", "", &none())
                    .expect("TestDestinationConnection");
                // (b)
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::OtherRepository),
                    "(b) {tested:?}"
                );
                assert_eq!(
                    english(&tested),
                    "There is another backup destination at this address.",
                    "(b) message"
                );
                // (a)
                let result = client
                    .update_destination(&edited, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(
                    result.outcome,
                    DestinationUpdateOutcome::OtherRepository,
                    "(a) not saved"
                );
                assert_eq!(config_bytes(&daemon), before, "(a) config.toml unchanged");
                daemon.stop();
            }
        },
    );
}

/// AC-M5-09-3 (FR-56, P42): (a) with the old address gone the ID is compared with the stored one
/// and (b) saved without a question; (c) a destination without a stored ID (from before M5) is
/// compared with the ID read at its old address when that answers, else every new ID counts as
/// another backup destination. Runs with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_09_3_stored_id_decides_when_old_address_is_gone() {
    run_isolated_test(
        "ac_m5_09_3_stored_id_decides_when_old_address_is_gone",
        "KESTIC_M5_09_093_INNER",
        |root| {
            for variant in restic_variants() {
                let sub = variant.sub("M5E093");
                let dir = root.join(&sub);
                fs::create_dir_all(&dir).expect("mkdir");
                // Two destinations from before M5: saved in the configuration and the Secret
                // Service, without a stored repository ID.
                let (old_answers, _) =
                    seed_local_destination(root, &variant.bin, "Answers", &dir.join("a"));
                let (old_gone, _) =
                    seed_local_destination(root, &variant.bin, "Gone", &dir.join("g"));
                let config = kestic_core::config::AppConfig {
                    destinations: vec![old_answers.clone(), old_gone.clone()],
                    ..Default::default()
                };
                let (config_path, _) = common::m3::daemon_paths(root, &sub);
                fs::create_dir_all(config_path.parent().expect("config dir")).expect("mkdir");
                kestic_core::config::save_config(&config_path, &config).expect("seed config");
                let wrapper = ResticWrapper::create(&dir.join("wrapper"), &variant.bin);
                let bus = variant.bus_name("M5E093");
                let daemon =
                    spawn_m3_daemon(root, M3DaemonOpts::on_request(&sub, &bus, &wrapper.path));
                let client = &daemon.client;

                // (c) the old address answers: its ID decides, and is stored.
                copy_dir(&dir.join("a"), &dir.join("a2"));
                let result = client
                    .update_destination(
                        &local(&old_answers, &dir.join("a2")),
                        "connect",
                        "",
                        &none(),
                        true,
                    )
                    .expect("UpdateDestination");
                assert_eq!(
                    result.outcome,
                    DestinationUpdateOutcome::Saved,
                    "(c) the ID read at the old address decides"
                );

                // (a), (b): the old address is gone now; the stored ID decides.
                copy_dir(&dir.join("a2"), &dir.join("a3"));
                fs::remove_dir_all(dir.join("a2")).expect("remove old address");
                let edited = local(&old_answers, &dir.join("a3"));
                let tested = client
                    .test_destination_connection(&edited, "connect", "", &none())
                    .expect("TestDestinationConnection");
                // (a)
                assert!(tested.ok, "(a) compared with the stored ID: {tested:?}");
                // (b)
                let result = client
                    .update_destination(&edited, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(result.outcome, DestinationUpdateOutcome::Saved, "(b)");

                // (c) no stored ID and the old address is gone: another backup destination.
                copy_dir(&dir.join("g"), &dir.join("g2"));
                fs::remove_dir_all(dir.join("g")).expect("remove old address");
                let started = Instant::now();
                let result = client
                    .update_destination(
                        &local(&old_gone, &dir.join("g2")),
                        "connect",
                        "",
                        &none(),
                        true,
                    )
                    .expect("UpdateDestination");
                assert_eq!(
                    result.outcome,
                    DestinationUpdateOutcome::OtherRepository,
                    "(c) without a known ID every repository is another one"
                );
                assert!(
                    started.elapsed() < Duration::from_secs(25),
                    "(c) took {:?}",
                    started.elapsed()
                );
                daemon.stop();
            }
        },
    );
}

/// M5-S09: saving a changed connection keeps the destination's maintenance schedule, which
/// `UpdateDestinationMaintenance` set before, and, for the same repository, its recovery kit
/// key, which the GUI's destination does not carry; a repository created anew in place drops
/// the key of the old one.
#[test]
fn m5_09_saving_a_connection_keeps_the_maintenance_schedule() {
    run_isolated_test(
        "m5_09_saving_a_connection_keeps_the_maintenance_schedule",
        "KESTIC_M5_09_MAINT_INNER",
        |root| {
            let variant = latest_variant();
            let (daemon, _wrapper, sub) = spawn(root, &variant, "M5EMNT");
            let dir = root.join(&sub);
            let client = &daemon.client;
            let (dest, _) =
                add_local_destination(client, &variant.bin, &dir.join("repo"), "Disk", |_| {});
            let mut schedule = saved(&daemon, &dest).maintenance;
            schedule.prune_interval_days = Some(60);
            schedule.check_interval_days = Some(90);
            client
                .update_destination_maintenance(dest.id.as_str(), &schedule)
                .expect("UpdateDestinationMaintenance");
            // No call sets the recovery kit key yet: it is written into the config of a stopped
            // daemon.
            let config_path = daemon.config_path.clone();
            daemon.stop();
            let mut config = load_config(&config_path).expect("config");
            config
                .destinations
                .iter_mut()
                .find(|d| d.id == dest.id)
                .expect("destination")
                .recovery_kit_key_id = Some("KITKEY01".to_string());
            kestic_core::config::save_config(&config_path, &config).expect("save config");
            let (daemon, _wrapper, _) = spawn(root, &variant, "M5EMNT");
            let client = &daemon.client;

            let copy = dir.join("copy");
            copy_dir(&dir.join("repo"), &copy);
            let mut moved = local(&dest, &copy);
            moved.name = "Disk copy".to_string();
            let result = client
                .update_destination(&moved, "connect", "", &none(), true)
                .expect("UpdateDestination");
            assert_eq!(result.outcome, DestinationUpdateOutcome::Saved);
            let after = saved(&daemon, &dest);
            assert_eq!(after.name, "Disk copy", "the name is saved");
            assert_eq!(after.maintenance, schedule, "the schedule is kept");
            assert_eq!(
                after.recovery_kit_key_id.as_deref(),
                Some("KITKEY01"),
                "the recovery kit key is kept for the same repository"
            );

            fs::remove_dir_all(&copy).expect("remove repository");
            fs::create_dir_all(&copy).expect("mkdir");
            let result = client
                .update_destination(&moved, "init", "", &none(), true)
                .expect("UpdateDestination init in place");
            assert_eq!(result.outcome, DestinationUpdateOutcome::Saved);
            let renewed = saved(&daemon, &dest);
            assert_eq!(
                renewed.recovery_kit_key_id, None,
                "the key of the old repository is dropped"
            );
            assert_eq!(renewed.maintenance, schedule, "the schedule is kept");
            daemon.stop();
        },
    );
}

/// AC-M5-09-1 (FR-56, P42): the `rest-server` of a REST destination serves the same repository
/// on a new port; changing the URL (a) finds the same repository ID and (b) saves without a
/// question, with the saved server password (same host). Runs with restic 0.18.0 and the latest
/// restic.
#[test]
fn ac_m5_09_1_rest_server_on_new_port_is_the_same_repository() {
    run_isolated_test(
        "ac_m5_09_1_rest_server_on_new_port_is_the_same_repository",
        "KESTIC_M5_09_091_INNER",
        |root| {
            for variant in restic_variants() {
                let (daemon, _wrapper, sub) = spawn(root, &variant, "M5E091");
                let dir = root.join(&sub);
                let client = &daemon.client;
                let data = dir.join("rest-data");
                let mut server = RestServerGuard::start_with_auth(&data);
                let (user, server_password) = server
                    .auth()
                    .map(|(u, p)| (u.to_string(), p.to_string()))
                    .expect("auth");
                let dest = Destination::new(
                    "NAS",
                    DestinationKind::Rest {
                        url: server.url.clone(),
                        username: Some(user.clone()),
                    },
                )
                .expect("REST destination");
                common::m3::connect_rest_repo(client, &variant.bin, &server, &dest);
                server.stop();
                let moved_server = RestServerGuard::start_on_port(
                    &data,
                    free_loopback_port(),
                    Some((&user, &server_password)),
                );
                let mut edited = dest.clone();
                edited.kind = DestinationKind::Rest {
                    url: moved_server.url.clone(),
                    username: Some(user.clone()),
                };
                // (a)
                let tested = client
                    .test_destination_connection(&edited, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert!(tested.ok, "(a) same repository: {tested:?}");
                // (b)
                let result = client
                    .update_destination(&edited, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(result.outcome, DestinationUpdateOutcome::Saved, "(b)");
                assert_eq!(saved(&daemon, &dest).kind, edited.kind, "(b) new URL saved");
                daemon.stop();
            }
        },
    );
}

/// SFTP rig: test server, the daemon behind a restic wrapper, a working directory on the server.
struct SftpRig {
    daemon: M3Daemon,
    wrapper: ResticWrapper,
    server: SftpServerGuard,
    dir: PathBuf,
}

fn start_sftp(root: &Path, variant: &ResticVariant, base: &str) -> SftpRig {
    let sub = variant.sub(base);
    let dir = root.join(&sub);
    let server = SftpServerGuard::start(&dir.join("sshd"));
    set_test_ssh_config(Some(server.ssh_config.clone()));
    let wrapper = ResticWrapper::create(&dir.join("wrapper"), &variant.bin);
    let bus = variant.bus_name(base);
    let daemon = spawn_m3_daemon(root, M3DaemonOpts::on_request(&sub, &bus, &wrapper.path));
    SftpRig {
        daemon,
        wrapper,
        server,
        dir,
    }
}

impl SftpRig {
    fn kind(&self, host: &str, repo: &str) -> DestinationKind {
        DestinationKind::Sftp {
            host: host.to_string(),
            port: self.server.port,
            user: self.server.user.clone(),
            path: self.dir.join(repo).to_string_lossy().into_owned(),
        }
    }

    /// Trusts the key of `dest`'s server as the test shows it.
    fn trust(&self, dest: &Destination) {
        let tested = self
            .daemon
            .client
            .test_destination_connection(dest, "connect", "", &none())
            .expect("TestDestinationConnection");
        assert_eq!(
            tested.error_kind,
            Some(ConnectionTestCode::UnknownHostKey),
            "unknown server: {tested:?}"
        );
        self.daemon
            .client
            .trust_host_key(&tested.host_key.expect("host key").host_key_id)
            .expect("TrustHostKey");
    }

    /// An SFTP destination on 127.0.0.1 with a copied repository at `repo`, and its password.
    fn add(&self, variant: &ResticVariant, repo: &str) -> (Destination, String) {
        self.add_at(variant, "127.0.0.1", repo)
    }

    /// [`SftpRig::add`] with the server under the name `host`.
    fn add_at(&self, variant: &ResticVariant, host: &str, repo: &str) -> (Destination, String) {
        let created = copy_template_repo(&variant.bin, &self.dir.join(repo));
        let dest = Destination::new(format!("Server {repo}"), self.kind(host, repo)).expect("SFTP");
        self.trust(&dest);
        self.daemon
            .client
            .add_destination(&dest, "connect", &created.password, &none())
            .expect("AddDestination");
        (dest, created.password)
    }
}

/// AC-M5-09-2 (FR-56, P42, P24): the repository of an SFTP destination is reachable under a new
/// host name; changing the host (a) shows the host key first (nothing saved before it is
/// trusted), (b) after trusting finds the same repository ID, and (c) saves without another
/// question. Runs with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_09_2_sftp_new_host_asks_for_host_key_then_saves() {
    run_isolated_test(
        "ac_m5_09_2_sftp_new_host_asks_for_host_key_then_saves",
        "KESTIC_M5_09_092_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_sftp(root, &variant, "M5E092");
                let client = &rig.daemon.client;
                let (dest, _) = rig.add(&variant, "repo");
                let mut edited = dest.clone();
                edited.kind = rig.kind("localhost", "repo");
                let before = config_bytes(&rig.daemon);

                // (a)
                let tested = client
                    .test_destination_connection(&edited, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::UnknownHostKey),
                    "(a) the host key comes first: {tested:?}"
                );
                let err = client
                    .update_destination(&edited, "connect", "", &none(), true)
                    .expect_err("(a) UpdateDestination before the key is trusted");
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.HostKeyUnknown");
                assert_eq!(config_bytes(&rig.daemon), before, "(a) nothing saved");
                client
                    .trust_host_key(&tested.host_key.expect("host key").host_key_id)
                    .expect("TrustHostKey");
                // (b)
                let tested = client
                    .test_destination_connection(&edited, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert!(tested.ok, "(b) same repository: {tested:?}");
                // (c)
                let result = client
                    .update_destination(&edited, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(result.outcome, DestinationUpdateOutcome::Saved, "(c)");
                assert_eq!(saved(&rig.daemon, &dest).kind, edited.kind, "(c) new host");
                // The old server's line goes when no destination uses it.
                let known = fs::read_to_string(
                    KnownHostsPaths::from_env()
                        .expect("known_hosts paths")
                        .kestic,
                )
                .expect("Kestic's known_hosts");
                assert!(
                    !known.contains(&rig.server.known_hosts_name()),
                    "the line of the old host is removed: {known}"
                );
                assert!(
                    known.contains(&format!("[localhost]:{}", rig.server.port)),
                    "the line of the new host stays: {known}"
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-01-8 (a), with an `UpdateDestination` that moves a destination to the server still
/// running: removing the only saved destination on that server keeps the line, which the moved
/// destination checked and still needs once it is saved.
#[test]
fn ac_m5_01_8_line_stays_for_a_destination_moved_meanwhile() {
    run_isolated_test(
        "ac_m5_01_8_line_stays_for_a_destination_moved_meanwhile",
        "KESTIC_M5_09_018_MOVE_INNER",
        |root| {
            let variant = latest_variant();
            let rig = start_sftp(root, &variant, "M5E018");
            // One server under two names, each with its own line in Kestic's `known_hosts`.
            let (moving, password) = rig.add_at(&variant, "127.0.0.1", "a");
            let (other, _) = rig.add_at(&variant, "localhost", "b");
            let kestic = KnownHostsPaths::from_env().expect("paths").kestic;
            let localhost_line = || {
                fs::read_to_string(&kestic)
                    .unwrap_or_default()
                    .lines()
                    .any(|l| l.starts_with(&format!("[localhost]:{} ", rig.server.port)))
            };
            assert!(localhost_line(), "the trusted line of localhost");
            let mut moved = moving.clone();
            moved.kind = rig.kind("localhost", "a");
            let cats_before = rig.wrapper.of('S', "cat").len();
            rig.wrapper.hold("cat");
            let client = rig.daemon.client.clone();
            let updating = std::thread::spawn({
                let moved = moved.clone();
                move || client.update_destination(&moved, "connect", &password, &none(), true)
            });
            // The update checks the repository at the new address with restic.
            rig.wrapper
                .wait_for('S', "cat", cats_before + 1, Duration::from_secs(30));

            rig.daemon
                .client
                .remove_destination(other.id.as_str())
                .expect("RemoveDestination");
            rig.wrapper.release("cat");
            let result = updating
                .join()
                .expect("UpdateDestination thread")
                .expect("UpdateDestination");
            assert_eq!(result.outcome, DestinationUpdateOutcome::Saved, "moved");
            // (a)
            assert!(
                localhost_line(),
                "(a) line removed while a destination was being moved to the server"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M5-01-6 (FR-56, U18): an SFTP destination whose path is changed. (a) The connection test
/// and the repository-ID check run first, (b) only then `UpdateDestination` saves and the
/// configuration has the new path, and (c) on a failure the configuration stays unchanged and
/// the message names the reason. Runs with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_01_6_sftp_path_change_is_checked_before_saving() {
    run_isolated_test(
        "ac_m5_01_6_sftp_path_change_is_checked_before_saving",
        "KESTIC_M5_09_016_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_sftp(root, &variant, "M5E016");
                let client = &rig.daemon.client;
                let (dest, password) = rig.add(&variant, "repo");
                let before = config_bytes(&rig.daemon);

                // (c) no repository at the new path.
                fs::create_dir_all(rig.dir.join("empty")).expect("mkdir");
                let mut empty = dest.clone();
                empty.kind = rig.kind("127.0.0.1", "empty");
                let tested = client
                    .test_destination_connection(&empty, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::RepositoryMissing),
                    "(c) reason: {tested:?}"
                );
                assert!(
                    client
                        .update_destination(&empty, "connect", "", &none(), true)
                        .is_err(),
                    "(c) UpdateDestination fails"
                );
                assert_eq!(config_bytes(&rig.daemon), before, "(c) config unchanged");

                // (c) another repository at the new path.
                let other_password = {
                    let created = copy_template_repo(&variant.bin, &rig.dir.join("other"));
                    created.password
                };
                assert!(!other_password.is_empty());
                let mut other = dest.clone();
                other.kind = rig.kind("127.0.0.1", "other");
                let tested = client
                    .test_destination_connection(&other, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::WrongPassword),
                    "(c) reason: {tested:?}"
                );
                assert_eq!(config_bytes(&rig.daemon), before, "(c) config unchanged");

                // (a) a repository of its own `init` that the saved password opens: the ID
                // decides.
                init_repo(&variant.bin, &rig.dir.join("own-init"), &password);
                let mut own_init = dest.clone();
                own_init.kind = rig.kind("127.0.0.1", "own-init");
                assert_other_repository(&rig.daemon, &own_init, &before, "(a)");

                // (a), (b) the repository copied to a new path.
                copy_dir(&rig.dir.join("repo"), &rig.dir.join("moved"));
                let mut moved = dest.clone();
                moved.kind = rig.kind("127.0.0.1", "moved");
                let tested = client
                    .test_destination_connection(&moved, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert!(tested.ok, "(a) {tested:?}");
                let result = client
                    .update_destination(&moved, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(result.outcome, DestinationUpdateOutcome::Saved, "(b)");
                assert_eq!(saved(&rig.daemon, &dest).kind, moved.kind, "(b) new path");
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-02-5 (FR-56, U18): an S3 destination whose bucket, keys or endpoint change. (a) The test
/// against `rclone serve s3` and the ID check run first, (b) `UpdateDestination` saves only
/// after them, (c) keys entered are only in the Secret Service, and (d) a wrong key saves
/// nothing. Saved keys never go to another endpoint. Runs with restic 0.18.0 and the latest
/// restic.
#[test]
fn ac_m5_02_5_s3_change_is_checked_and_keys_stay_in_secret_service() {
    run_isolated_test(
        "ac_m5_02_5_s3_change_is_checked_and_keys_stay_in_secret_service",
        "KESTIC_M5_09_025_INNER",
        |root| {
            for variant in restic_variants() {
                let (daemon, wrapper, sub) = spawn(root, &variant, "M5E025");
                let client = &daemon.client;
                let mut server = S3ServerGuard::start(&root.join(&sub).join("s3-data"), "one");
                let keys = BackendSecrets::s3(&server.access_key, &server.secret_key);
                let s3 = |endpoint: &str, bucket: &str| DestinationKind::S3 {
                    endpoint: endpoint.to_string(),
                    bucket: bucket.to_string(),
                    path: None,
                    region: None,
                };
                let dest =
                    Destination::new("Cloud", s3(&server.endpoint, "one")).expect("S3 destination");
                let password = strong_password();
                client
                    .add_destination(&dest, "init", &password, &keys)
                    .expect("AddDestination");
                // The same repository in a second bucket, and a repository of its own `init` with
                // the same password in a third; the server lists them after a restart.
                copy_dir(&server.bucket_dir("one"), &server.bucket_dir("two"));
                init_repo(&variant.bin, &server.bucket_dir("three"), &password);
                server.stop();
                let server = server.restart();
                let store = isolated_secret_store();
                let stored = |kind: &str| {
                    store
                        .get_secret(dest.id.as_str(), kind)
                        .expect("read Secret Service")
                        .map(|v| v.as_str().expect("utf-8").to_string())
                };

                // (d) a wrong key.
                let before = config_bytes(&daemon);
                let mut other_bucket = dest.clone();
                other_bucket.kind = s3(&server.endpoint, "two");
                let wrong = BackendSecrets::s3(&server.access_key, &s3_key(40));
                let tested = client
                    .test_destination_connection(&other_bucket, "connect", "", &wrong)
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::AuthFailed),
                    "(d) {tested:?}"
                );
                let err = client
                    .update_destination(&other_bucket, "connect", "", &wrong, true)
                    .expect_err("(d) UpdateDestination with a wrong key");
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.AuthFailed");
                assert_eq!(config_bytes(&daemon), before, "(d) config unchanged");
                assert_eq!(
                    stored(SECRET_KIND_S3_SECRET_KEY).as_deref(),
                    Some(server.secret_key.as_str()),
                    "(d) the saved key stays"
                );

                // Another endpoint (the same server under another name) gets no saved key.
                let other_endpoint = server.endpoint.replace("127.0.0.1", "localhost");
                assert_ne!(other_endpoint, server.endpoint);
                let mut moved = dest.clone();
                moved.kind = s3(&other_endpoint, "one");
                let calls_before = wrapper.entries().len();
                let tested = client
                    .test_destination_connection(&moved, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::CredentialsRequired),
                    "a new endpoint needs the keys: {tested:?}"
                );
                assert!(
                    client
                        .update_destination(&moved, "connect", "", &none(), true)
                        .is_err(),
                    "UpdateDestination without keys for a new endpoint"
                );
                assert_eq!(
                    wrapper.entries().len(),
                    calls_before,
                    "restic never ran with the saved keys for the new endpoint"
                );

                // (a) the repository ID is compared.
                let mut own_init = dest.clone();
                own_init.kind = s3(&server.endpoint, "three");
                assert_other_repository(&daemon, &own_init, &before, "(a)");

                // (a), (b) the same repository in another bucket, keys reused.
                let tested = client
                    .test_destination_connection(&other_bucket, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert!(tested.ok, "(a) {tested:?}");
                let result = client
                    .update_destination(&other_bucket, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(result.outcome, DestinationUpdateOutcome::Saved, "(b)");
                assert_eq!(
                    saved(&daemon, &dest).kind,
                    other_bucket.kind,
                    "(b) bucket two"
                );

                // (c) keys entered with the new endpoint are saved, only in the Secret Service.
                let result = client
                    .update_destination(&moved, "connect", "", &keys, true)
                    .expect("UpdateDestination with keys");
                assert_eq!(result.outcome, DestinationUpdateOutcome::Saved, "(c)");
                let config_text = fs::read_to_string(&daemon.config_path).expect("config");
                assert!(
                    !config_text.contains(&server.secret_key)
                        && !config_text.contains(&server.access_key),
                    "(c) no key in config.toml"
                );
                assert_eq!(
                    stored(SECRET_KIND_S3_ACCESS_KEY).as_deref(),
                    Some(server.access_key.as_str()),
                    "(c) access key in the Secret Service"
                );

                // Keys entered with `verify_connection = false` are checked and saved too.
                store
                    .set_secret(
                        dest.id.as_str(),
                        SECRET_KIND_S3_SECRET_KEY,
                        &kestic_core::secrets::SecretValue::from_str_secret("stale-key"),
                    )
                    .expect("stale key");
                let err = client
                    .update_destination(&moved, "connect", "", &wrong, false)
                    .expect_err("wrong keys without verify_connection");
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.AuthFailed");
                assert_eq!(
                    stored(SECRET_KIND_S3_SECRET_KEY).as_deref(),
                    Some("stale-key"),
                    "wrong keys are not saved"
                );
                let result = client
                    .update_destination(&moved, "connect", "", &keys, false)
                    .expect("keys without verify_connection");
                assert_eq!(result.outcome, DestinationUpdateOutcome::Saved);
                assert_eq!(
                    stored(SECRET_KIND_S3_SECRET_KEY).as_deref(),
                    Some(server.secret_key.as_str()),
                    "the entered keys are saved"
                );
                daemon.stop();
            }
        },
    );
}

/// Daemon whose restic script puts rclone on the `PATH` and calls a wrapper around restic.
fn spawn_generic(
    root: &Path,
    variant: &ResticVariant,
    base: &str,
) -> (M3Daemon, ResticWrapper, PathBuf) {
    let sub = variant.sub(base);
    let dir = root.join(&sub);
    let wrapper = ResticWrapper::create(&dir.join("wrapper"), &variant.bin);
    let rclone_dir = rclone_binary()
        .parent()
        .expect("rclone directory")
        .to_path_buf();
    let script = dir.join("restic-generic.sh");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\nPATH='{rclone}':\"$PATH\"\nexport PATH\nexec '{wrapper}' \"$@\"\n",
            rclone = rclone_dir.display(),
            wrapper = wrapper.path.display()
        ),
    );
    let bus = variant.bus_name(base);
    let mut opts = M3DaemonOpts::on_request(&sub, &bus, &script);
    opts.program_search_path = Some(rclone_dir.into_os_string());
    (spawn_m3_daemon(root, opts), wrapper, dir)
}

fn generic_kind(repo: &Path, plain: &str) -> DestinationKind {
    DestinationKind::Generic {
        url: format!("rclone::local:{}", repo.display()),
        env: vec![
            EnvVar {
                name: "KESTIC_TEST_TOKEN".to_string(),
                secret: true,
                value: None,
            },
            EnvVar {
                name: "KESTIC_TEST_PLAIN".to_string(),
                secret: false,
                value: Some(plain.to_string()),
            },
        ],
    }
}

/// AC-M5-03-6 (FR-56, U18): a destination of the generic form whose URL or variables change. (a)
/// The check runs `restic cat config` (15 s) and compares the repository ID, (b)
/// `UpdateDestination` saves only after it, and (c) otherwise nothing changes. A changed variable
/// that is not secret can name another server, so the secret values must be entered again. Runs
/// with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_03_6_generic_change_is_checked_before_saving() {
    run_isolated_test(
        "ac_m5_03_6_generic_change_is_checked_before_saving",
        "KESTIC_M5_09_036_INNER",
        |root| {
            for variant in restic_variants() {
                let (daemon, wrapper, dir) = spawn_generic(root, &variant, "M5E036");
                let client = &daemon.client;
                let token = strong_password();
                let secrets = BackendSecrets::env(&[("KESTIC_TEST_TOKEN", token.as_str())]);
                let dest = Destination::new("Cloud", generic_kind(&dir.join("repo"), "one"))
                    .expect("generic destination");
                let password = strong_password();
                client
                    .add_destination(&dest, "init", &password, &secrets)
                    .expect("AddDestination");
                let before = config_bytes(&daemon);

                // (a), (c) restic does not answer: stopped after 15 s, nothing saved.
                copy_dir(&dir.join("repo"), &dir.join("moved"));
                let mut moved = dest.clone();
                moved.kind = generic_kind(&dir.join("moved"), "one");
                wrapper.hold("cat");
                let started = Instant::now();
                let tested = client
                    .test_destination_connection(&moved, "connect", "", &none())
                    .expect("TestDestinationConnection");
                wrapper.release("cat");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::NotResponding),
                    "(a) {tested:?}"
                );
                assert!(
                    started.elapsed() < Duration::from_secs(22),
                    "(a) took {:?}",
                    started.elapsed()
                );
                assert_eq!(config_bytes(&daemon), before, "(c) config unchanged");

                // A changed plain variable needs the secret value again.
                let mut plain_changed = dest.clone();
                plain_changed.kind = generic_kind(&dir.join("repo"), "two");
                let tested = client
                    .test_destination_connection(&plain_changed, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::CredentialsRequired),
                    "{tested:?}"
                );

                // A secret inside an rclone URL is refused.
                let mut secret_url = dest.clone();
                secret_url.kind = DestinationKind::Generic {
                    url: "rclone::sftp,host=nas,pass=hunter2:backup".to_string(),
                    env: Vec::new(),
                };
                let tested = client
                    .test_destination_connection(&secret_url, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::SecretInUrl),
                    "{tested:?}"
                );

                // (a) a repository of its own `init` that the saved password opens.
                init_repo(&variant.bin, &dir.join("own-init"), &password);
                let mut own_init = dest.clone();
                own_init.kind = generic_kind(&dir.join("own-init"), "one");
                let tested = client
                    .test_destination_connection(&own_init, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::OtherRepository),
                    "(a) the repository ID is compared: {tested:?}"
                );
                let result = client
                    .update_destination(&own_init, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(
                    result.outcome,
                    DestinationUpdateOutcome::OtherRepository,
                    "(a) another repository is not saved"
                );
                assert_eq!(config_bytes(&daemon), before, "(c) config unchanged");

                // (a), (b) the repository at a new path: compared and saved, token reused.
                let tested = client
                    .test_destination_connection(&moved, "connect", "", &none())
                    .expect("TestDestinationConnection");
                assert!(tested.ok, "(a) {tested:?}");
                let result = client
                    .update_destination(&moved, "connect", "", &none(), true)
                    .expect("UpdateDestination");
                assert_eq!(result.outcome, DestinationUpdateOutcome::Saved, "(b)");
                assert_eq!(saved(&daemon, &dest).kind, moved.kind, "(b) saved");
                assert_eq!(
                    isolated_secret_store()
                        .get_secret(dest.id.as_str(), &secret_kind("KESTIC_TEST_TOKEN"))
                        .expect("read")
                        .map(|v| v.as_str().expect("utf-8").to_string()),
                    Some(token),
                    "the token stays"
                );
                daemon.stop();
            }
        },
    );
}

/// `UpdateDestination` with `verify_connection = false` still checks a changed
/// connection or new credentials, and never saves the configuration without the entered keys.
#[test]
fn m5_09_verify_connection_false_still_checks_changes() {
    run_isolated_test(
        "m5_09_verify_connection_false_still_checks_changes",
        "KESTIC_M5_09_VC_INNER",
        |root| {
            let variant = latest_variant();
            let (daemon, wrapper, sub) = spawn(root, &variant, "M5EVC");
            let dir = root.join(&sub);
            let client = &daemon.client;
            let (dest, password) =
                add_local_destination(client, &variant.bin, &dir.join("repo"), "Disk", |_| {});
            init_repo(&variant.bin, &dir.join("other"), &password);
            let before = config_bytes(&daemon);
            let calls_before = wrapper.entries().len();
            let result = client
                .update_destination(
                    &local(&dest, &dir.join("other")),
                    "connect",
                    "",
                    &none(),
                    false,
                )
                .expect("UpdateDestination");
            assert_eq!(
                result.outcome,
                DestinationUpdateOutcome::OtherRepository,
                "the change is checked anyway"
            );
            assert!(wrapper.entries().len() > calls_before, "restic ran");
            assert_eq!(config_bytes(&daemon), before, "nothing saved");

            // A rename alone needs no check.
            let mut renamed = dest.clone();
            renamed.name = "Renamed".to_string();
            let calls_before = wrapper.entries().len();
            let result = client
                .update_destination(&renamed, "connect", "", &none(), false)
                .expect("UpdateDestination");
            assert_eq!(result.outcome, DestinationUpdateOutcome::Saved);
            assert_eq!(
                wrapper.entries().len(),
                calls_before,
                "no restic for a rename"
            );
            assert_eq!(saved(&daemon, &dest).name, "Renamed");
            daemon.stop();
        },
    );
}

/// When `config.toml` cannot be written after the secrets were, the secrets get
/// their previous values back and an entry that had none is removed.
#[test]
fn m5_09_failed_config_write_restores_secrets() {
    run_isolated_test(
        "m5_09_failed_config_write_restores_secrets",
        "KESTIC_M5_09_CW_INNER",
        |root| {
            let variant = latest_variant();
            let (daemon, _wrapper, sub) = spawn(root, &variant, "M5ECW");
            let client = &daemon.client;
            let server = S3ServerGuard::start(&root.join(&sub).join("s3-data"), "one");
            let keys = BackendSecrets::s3(&server.access_key, &server.secret_key);
            let dest = Destination::new(
                "Cloud",
                DestinationKind::S3 {
                    endpoint: server.endpoint.clone(),
                    bucket: "one".to_string(),
                    path: None,
                    region: None,
                },
            )
            .expect("S3 destination");
            client
                .add_destination(&dest, "init", &strong_password(), &keys)
                .expect("AddDestination");
            let store = isolated_secret_store();
            // An entry without a value, and one with an old value: both as before after the
            // failed save.
            store
                .delete_secret(dest.id.as_str(), SECRET_KIND_S3_ACCESS_KEY)
                .expect("delete");
            store
                .set_secret(
                    dest.id.as_str(),
                    SECRET_KIND_S3_SECRET_KEY,
                    &kestic_core::secrets::SecretValue::from_str_secret("old-secret-key"),
                )
                .expect("old value");
            let config_dir = daemon
                .config_path
                .parent()
                .expect("config dir")
                .to_path_buf();
            fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o555)).expect("chmod");
            let result = client.update_destination(&dest, "connect", "", &keys, true);
            fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o755)).expect("chmod");
            assert!(result.is_err(), "the config write failed: {result:?}");
            assert_eq!(
                store
                    .get_secret(dest.id.as_str(), SECRET_KIND_S3_ACCESS_KEY)
                    .expect("read"),
                None,
                "an entry that had no value is removed again"
            );
            assert_eq!(
                store
                    .get_secret(dest.id.as_str(), SECRET_KIND_S3_SECRET_KEY)
                    .expect("read")
                    .map(|v| v.as_str().expect("utf-8").to_string()),
                Some("old-secret-key".to_string()),
                "the previous value is back"
            );
            daemon.stop();
        },
    );
}

/// `UpdateDestination` in mode `init`: re-creates a missing repository at the unchanged place and
/// saves; with a changed connection it never runs restic and reports another repository.
#[test]
fn m5_09_init_only_in_place() {
    run_isolated_test(
        "m5_09_init_only_in_place",
        "KESTIC_M5_09_INIT_INNER",
        |root| {
            let variant = latest_variant();
            let (daemon, wrapper, sub) = spawn(root, &variant, "M5EINIT");
            let dir = root.join(&sub);
            let client = &daemon.client;
            let (dest, _) =
                add_local_destination(client, &variant.bin, &dir.join("repo"), "Disk", |_| {});
            fs::create_dir_all(dir.join("elsewhere")).expect("mkdir");
            let calls_before = wrapper.entries().len();
            let tested = client
                .test_destination_connection(
                    &local(&dest, &dir.join("elsewhere")),
                    "init",
                    "",
                    &none(),
                )
                .expect("TestDestinationConnection");
            assert_eq!(
                tested.error_kind,
                Some(ConnectionTestCode::OtherRepository),
                "the test of init elsewhere: {tested:?}"
            );
            let result = client
                .update_destination(
                    &local(&dest, &dir.join("elsewhere")),
                    "init",
                    "",
                    &none(),
                    true,
                )
                .expect("UpdateDestination");
            assert_eq!(result.outcome, DestinationUpdateOutcome::OtherRepository);
            assert_eq!(
                wrapper.entries().len(),
                calls_before,
                "no restic for init elsewhere"
            );
            assert!(
                !dir.join("elsewhere").join("config").exists(),
                "no repository created"
            );

            fs::remove_dir_all(dir.join("repo")).expect("remove repository");
            fs::create_dir_all(dir.join("repo")).expect("mkdir");
            let result = client
                .update_destination(&dest, "init", "", &none(), true)
                .expect("UpdateDestination init in place");
            assert_eq!(result.outcome, DestinationUpdateOutcome::Saved);
            assert!(
                dir.join("repo").join("config").is_file(),
                "re-created in place"
            );
            // The new repository's ID is the one known now: a copy of it is the same repository.
            copy_dir(&dir.join("repo"), &dir.join("copy"));
            let result = client
                .update_destination(
                    &local(&dest, &dir.join("copy")),
                    "connect",
                    "",
                    &none(),
                    true,
                )
                .expect("UpdateDestination");
            assert_eq!(result.outcome, DestinationUpdateOutcome::Saved);
            assert!(
                isolated_secret_store()
                    .get_secret(dest.id.as_str(), SECRET_KIND_REPO)
                    .expect("read")
                    .is_some(),
                "the repository password stays"
            );
            daemon.stop();
        },
    );
}

/// `AddDestination` whose `config.toml` cannot be written afterwards removes the secrets it has
/// just written for the new destination.
#[test]
fn m5_09_failed_config_write_after_add_removes_new_secrets() {
    run_isolated_test(
        "m5_09_failed_config_write_after_add_removes_new_secrets",
        "KESTIC_M5_09_ADD_INNER",
        |root| {
            let variant = latest_variant();
            let (daemon, _wrapper, sub) = spawn(root, &variant, "M5EADD");
            let dir = root.join(&sub);
            let repo = copy_template_repo(&variant.bin, &dir.join("repo"));
            let dest = Destination::new(
                "Disk",
                DestinationKind::Local {
                    path: dir.join("repo"),
                },
            )
            .expect("destination");
            let config_dir = daemon
                .config_path
                .parent()
                .expect("config dir")
                .to_path_buf();
            fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o555)).expect("chmod");
            let result = daemon
                .client
                .add_destination(&dest, "connect", &repo.password, &none());
            fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o755)).expect("chmod");
            assert!(result.is_err(), "the config write failed: {result:?}");
            assert_eq!(
                isolated_secret_store()
                    .get_secret(dest.id.as_str(), SECRET_KIND_REPO)
                    .expect("read"),
                None,
                "no repository password left behind"
            );
            daemon.stop();
        },
    );
}

/// A repository swapped at the saved address (another `init` with the same password) is not the
/// destination's own: the test and `UpdateDestination` report another backup destination and
/// keep the known ID, so a copy of the original repository is still accepted afterwards.
#[test]
fn m5_09_swapped_repository_at_the_saved_address_is_refused() {
    run_isolated_test(
        "m5_09_swapped_repository_at_the_saved_address_is_refused",
        "KESTIC_M5_09_SWAP_INNER",
        |root| {
            let variant = latest_variant();
            let (daemon, _wrapper, sub) = spawn(root, &variant, "M5ESWAP");
            let dir = root.join(&sub);
            let client = &daemon.client;
            let (dest, password) =
                add_local_destination(client, &variant.bin, &dir.join("repo"), "Disk", |_| {});
            copy_dir(&dir.join("repo"), &dir.join("original"));
            fs::remove_dir_all(dir.join("repo")).expect("remove repository");
            init_repo(&variant.bin, &dir.join("repo"), &password);
            let before = config_bytes(&daemon);

            let tested = client
                .test_destination_connection(&dest, "connect", "", &none())
                .expect("TestDestinationConnection");
            assert_eq!(
                tested.error_kind,
                Some(ConnectionTestCode::OtherRepository),
                "the swapped repository: {tested:?}"
            );
            let result = client
                .update_destination(&dest, "connect", "", &none(), true)
                .expect("UpdateDestination");
            assert_eq!(result.outcome, DestinationUpdateOutcome::OtherRepository);
            assert_eq!(config_bytes(&daemon), before, "nothing saved");

            let result = client
                .update_destination(
                    &local(&dest, &dir.join("original")),
                    "connect",
                    "",
                    &none(),
                    true,
                )
                .expect("UpdateDestination");
            assert_eq!(
                result.outcome,
                DestinationUpdateOutcome::Saved,
                "the known ID stayed: the original repository is accepted"
            );
            daemon.stop();
        },
    );
}
