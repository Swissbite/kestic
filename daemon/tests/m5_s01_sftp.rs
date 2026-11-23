// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M5-S01 destination «SFTP» against a test `sshd` of the current user with key login, over
//! D-Bus against an in-process daemon on an isolated session bus: connection test and adding
//! (`AC-M5-01-1`, `-2`), a rejected SSH key (`-3`), an unknown host key and Kestic's own
//! `known_hosts` (`-4`), a stopped server before «Back Up Now» (`-5`), a changed host key (`-7`)
//! and removing destinations (`-8`). A stand-in `ssh` in restic's `PATH` records every ssh call of
//! restic; `~/.ssh` and every `known_hosts` file live in the test's temp `HOME`/`XDG_DATA_HOME`.

mod common;

use common::m3::{
    M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, assert_dbus_error_name, backup_summary,
    dest_summary, restic_variants, spawn_m3_daemon, wait_for_state,
};
use common::{CapturedKesticSignal, KesticSignalCollector, run_isolated_test};
use kestic_core::model::{Backup, BackupId, Destination, DestinationKind, Source};
use kestic_core::sftp::{KnownHostsPaths, set_test_ssh_config};
use kestic_daemon::dbus_api::{BackendSecrets, KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH};
use kestic_i18n::codes::{BlockedReason, ConnectionTestCode, ErrorKind};
use kestic_i18n::messages::{UiText, connection_test_text};
use kestic_test_support::password::strong_password;
use kestic_test_support::repo_template::copy_template_repo;
use kestic_test_support::sftp_server::SftpServerGuard;
use kestic_test_support::tools::write_executable;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Daemon behind a restic wrapper that logs every call, with restic's `ssh` replaced by a
/// stand-in that logs its arguments and runs the real ssh; plus the test `sshd`.
struct SftpRig {
    daemon: M3Daemon,
    wrapper: ResticWrapper,
    server: SftpServerGuard,
    ssh_log: PathBuf,
    dir: PathBuf,
    bus: String,
}

fn real_ssh() -> PathBuf {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join("ssh"))
        .find(|p| p.is_file())
        .expect("ssh in PATH")
}

/// A restic that finds the logging `ssh` first in its `PATH`; returns `(restic, ssh log)`.
fn install_ssh_spy(dir: &Path, real_restic: &Path) -> (PathBuf, PathBuf) {
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).expect("mkdir spy bin");
    let log = dir.join("ssh-calls.log");
    write_executable(
        &bin.join("ssh"),
        &format!(
            "#!/bin/sh\n{{ for a in \"$@\"; do printf '%s\\037' \"$a\"; done; printf '\\n'; }} >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real_ssh().display()
        ),
    );
    let restic = dir.join("restic-with-ssh-spy.sh");
    write_executable(
        &restic,
        &format!(
            "#!/bin/sh\nPATH='{}':\"$PATH\" exec '{}' \"$@\"\n",
            bin.display(),
            real_restic.display()
        ),
    );
    (restic, log)
}

fn start_sftp_rig(root: &Path, variant: &ResticVariant, base: &str) -> SftpRig {
    let sub = variant.sub(base);
    let bus = variant.bus_name(base);
    let dir = root.join(&sub);
    let server = SftpServerGuard::start(&dir.join("sshd"));
    set_test_ssh_config(Some(server.ssh_config.clone()));
    let (spy, ssh_log) = install_ssh_spy(&dir.join("spy"), &variant.bin);
    let wrapper = ResticWrapper::create(&dir.join("wrapper"), &spy);
    let daemon = spawn_m3_daemon(root, M3DaemonOpts::on_request(&sub, &bus, &wrapper.path));
    SftpRig {
        daemon,
        wrapper,
        server,
        ssh_log,
        dir,
        bus,
    }
}

fn paths() -> KnownHostsPaths {
    KnownHostsPaths::from_env().expect("known_hosts paths of the test")
}

fn read(path: &Path) -> Option<Vec<u8>> {
    fs::read(path).ok()
}

fn no_secrets() -> BackendSecrets {
    BackendSecrets::default()
}

impl SftpRig {
    fn destination(&self, name: &str, repo: &str) -> Destination {
        Destination::new(
            name,
            DestinationKind::Sftp {
                host: "127.0.0.1".to_string(),
                port: self.server.port,
                user: self.server.user.clone(),
                path: self.repo_dir(repo).to_string_lossy().into_owned(),
            },
        )
        .expect("SFTP destination")
    }

    fn repo_dir(&self, repo: &str) -> PathBuf {
        self.dir.join(repo)
    }

    /// The unknown server's key as the connection test shows it, then trusted.
    fn trust_server(&self, dest: &Destination) {
        let tested = self
            .daemon
            .client
            .test_destination_connection(dest, "init", "", &no_secrets())
            .expect("TestDestinationConnection");
        assert_eq!(
            tested.error_kind,
            Some(ConnectionTestCode::UnknownHostKey),
            "unknown server expected: {tested:?}"
        );
        let key = tested.host_key.expect("host key of the unknown server");
        self.daemon
            .client
            .trust_host_key(&key.host_key_id)
            .expect("TrustHostKey");
    }

    /// Trusts the server and adds `dest` with a new repository.
    fn add_new(&self, dest: &Destination) {
        self.trust_server(dest);
        self.daemon
            .client
            .add_destination(dest, "init", &strong_password(), &no_secrets())
            .expect("AddDestination");
    }

    /// Arguments of every ssh call restic made so far.
    fn ssh_calls(&self) -> Vec<Vec<String>> {
        fs::read_to_string(&self.ssh_log)
            .unwrap_or_default()
            .lines()
            .map(|line| {
                line.split('\u{1f}')
                    .filter(|a| !a.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .collect()
    }

    fn configured_destinations(&self) -> usize {
        common::load_config(&self.daemon.config_path).map_or(0, |config| config.destinations.len())
    }
}

/// The `UserKnownHostsFile` option Kestic gives ssh: both files, quoted.
fn expected_known_hosts_option(paths: &KnownHostsPaths) -> String {
    format!(
        "UserKnownHostsFile=\"{}\" \"{}\"",
        paths.kestic.display(),
        paths.user.display()
    )
}

fn has_option(call: &[String], value: &str) -> bool {
    call.windows(2)
        .any(|pair| pair[0] == "-o" && pair[1] == value)
}

/// `userknownhostsfile` as ssh applies it to `call` (`ssh -G`, which only reads options).
fn effective_known_hosts(call: &[String]) -> String {
    let out = Command::new(real_ssh())
        .arg("-G")
        .args(call)
        .env_remove("SSH_AUTH_SOCK")
        .output()
        .expect("ssh -G");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            line.strip_prefix("userknownhostsfile ")
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default()
}

/// AC-M5-01-1 (FR-01, P24): an SSH server with key login; the connection test with host, user and
/// path (a) succeeds, (b) finds no repository there, (c) creates nothing (only `AddDestination`
/// runs `init`), and (d) every ssh call of restic carries `-o UserKnownHostsFile=<Kestic's file>
/// <~/.ssh/known_hosts>` through `sftp.args`. Runs with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_01_1_connection_test_succeeds_and_only_adding_runs_init() {
    run_isolated_test(
        "ac_m5_01_1_connection_test_succeeds_and_only_adding_runs_init",
        "KESTIC_M5_01_1_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_sftp_rig(root, &variant, "M5SftpA");
                let dest = rig.destination("NAS", "repo");
                let client = &rig.daemon.client;
                rig.trust_server(&dest);

                let tested = client
                    .test_destination_connection(&dest, "init", "", &no_secrets())
                    .expect("TestDestinationConnection");
                // (a)
                assert!(tested.ok, "{}: (a) test failed: {tested:?}", variant.label);
                // (b)
                let connect = client
                    .test_destination_connection(
                        &dest,
                        "connect",
                        &strong_password(),
                        &no_secrets(),
                    )
                    .expect("TestDestinationConnection connect");
                assert_eq!(
                    connect.error_kind,
                    Some(ConnectionTestCode::RepositoryMissing),
                    "{}: (b) no repository found: {connect:?}",
                    variant.label
                );
                // (c)
                assert!(
                    !rig.repo_dir("repo").join("config").exists(),
                    "{}: (c) the test created a repository",
                    variant.label
                );
                assert!(
                    rig.wrapper.of('S', "init").is_empty(),
                    "(c) init during the test"
                );
                assert_eq!(
                    rig.configured_destinations(),
                    0,
                    "(c) the test saved a destination"
                );
                client
                    .add_destination(&dest, "init", &strong_password(), &no_secrets())
                    .expect("AddDestination");
                assert!(
                    rig.repo_dir("repo").join("config").is_file(),
                    "{}: (c) AddDestination created no repository",
                    variant.label
                );
                assert_eq!(rig.wrapper.of('S', "init").len(), 1, "(c) one init");
                // (d)
                let expected = expected_known_hosts_option(&paths());
                let calls = rig.ssh_calls();
                assert!(!calls.is_empty(), "(d) restic made no ssh call");
                for call in &calls {
                    assert!(
                        has_option(call, &expected),
                        "{}: (d) ssh call without both known_hosts files: {call:?}",
                        variant.label
                    );
                }
                assert!(
                    rig.wrapper
                        .entries()
                        .iter()
                        .filter(|e| e.kind == 'S' && e.sub != "version")
                        .all(|e| e.detail.contains("sftp.args=")),
                    "(d) a restic call without sftp.args: {:?}",
                    rig.wrapper.entries()
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-01-2 (FR-01): the same server with an existing repository and the right password: (a)
/// the repository is connected, not initialised, and (b) ssh uses only the `known_hosts` paths
/// Kestic passes. Runs with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_01_2_existing_repository_is_connected_with_only_the_given_known_hosts() {
    run_isolated_test(
        "ac_m5_01_2_existing_repository_is_connected_with_only_the_given_known_hosts",
        "KESTIC_M5_01_2_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_sftp_rig(root, &variant, "M5SftpB");
                let repo = copy_template_repo(&variant.bin, &rig.repo_dir("existing"));
                let dest = rig.destination("NAS", "existing");
                let client = &rig.daemon.client;
                rig.trust_server(&dest);
                let config_before = read(&repo.path.join("config"));

                let tested = client
                    .test_destination_connection(&dest, "connect", &repo.password, &no_secrets())
                    .expect("TestDestinationConnection");
                assert!(
                    tested.ok,
                    "{}: connect test failed: {tested:?}",
                    variant.label
                );
                client
                    .add_destination(&dest, "connect", &repo.password, &no_secrets())
                    .expect("AddDestination connect");
                // (a)
                assert!(rig.wrapper.of('S', "init").is_empty(), "(a) init was run");
                assert_eq!(
                    read(&repo.path.join("config")),
                    config_before,
                    "(a) the repository config changed"
                );
                assert_eq!(rig.configured_destinations(), 1, "(a) destination saved");
                // (b)
                let p = paths();
                let expected = format!("{} {}", p.kestic.display(), p.user.display());
                let calls = rig.ssh_calls();
                assert!(!calls.is_empty(), "(b) restic made no ssh call");
                for call in &calls {
                    assert_eq!(
                        effective_known_hosts(call),
                        expected,
                        "{}: (b) ssh reads other known_hosts files: {call:?}",
                        variant.label
                    );
                }
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-01-3 (FR-01): no matching key: the test (a) explains the refused login with a hint on
/// the SSH key and the ssh-agent, and (b) no destination is created. Runs with restic 0.18.0 and
/// the latest restic.
#[test]
fn ac_m5_01_3_rejected_ssh_key_is_explained_and_nothing_is_added() {
    run_isolated_test(
        "ac_m5_01_3_rejected_ssh_key_is_explained_and_nothing_is_added",
        "KESTIC_M5_01_3_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_sftp_rig(root, &variant, "M5SftpC");
                let dest = rig.destination("NAS", "repo");
                let client = &rig.daemon.client;
                rig.trust_server(&dest);
                set_test_ssh_config(Some(rig.server.ssh_config_unknown_key.clone()));

                let tested = client
                    .test_destination_connection(&dest, "init", "", &no_secrets())
                    .expect("TestDestinationConnection");
                // (a)
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::SshKeyRejected),
                    "{}: (a) {tested:?}",
                    variant.label
                );
                let UiText::Message(message) =
                    connection_test_text(tested.ok, tested.error_kind, &tested.detail)
                else {
                    panic!("(a) a raw text instead of the message");
                };
                let text = message.english();
                assert!(
                    text.contains("SSH key"),
                    "(a) no hint on the SSH key: {text}"
                );
                assert!(
                    text.contains("ssh-agent"),
                    "(a) no hint on the ssh-agent: {text}"
                );
                // (b)
                let err = client
                    .add_destination(&dest, "init", &strong_password(), &no_secrets())
                    .expect_err("AddDestination with a rejected key");
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.AuthFailed");
                assert_eq!(
                    rig.configured_destinations(),
                    0,
                    "(b) a destination was created"
                );
                assert!(
                    !rig.repo_dir("repo").join("config").exists(),
                    "(b) a repository was created"
                );
                set_test_ssh_config(Some(rig.server.ssh_config.clone()));
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-01-4 (FR-01, P24): an unknown host key: the test (a) shows key type and SHA256
/// fingerprint and asks, (b) only the confirmation puts exactly this key into Kestic's
/// `known_hosts` under `XDG_DATA_HOME`, (c) without it the file stays unchanged and no destination
/// is created, and (d) `~/.ssh/known_hosts` never changes. Runs with restic 0.18.0 and the latest
/// restic.
#[test]
fn ac_m5_01_4_unknown_host_key_is_trusted_only_into_kestics_file() {
    run_isolated_test(
        "ac_m5_01_4_unknown_host_key_is_trusted_only_into_kestics_file",
        "KESTIC_M5_01_4_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_sftp_rig(root, &variant, "M5SftpD");
                let p = paths();
                fs::create_dir_all(p.user.parent().expect("~/.ssh")).expect("mkdir ~/.ssh");
                fs::write(&p.user, "other.example.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOtherHostKeyOnlyForTheTest\n")
                    .expect("write ~/.ssh/known_hosts");
                let user_before = read(&p.user);
                let _ = fs::remove_file(&p.kestic);
                let dest = rig.destination("NAS", "repo");
                let client = &rig.daemon.client;

                let tested = client
                    .test_destination_connection(&dest, "init", "", &no_secrets())
                    .expect("TestDestinationConnection");
                // (a)
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::UnknownHostKey),
                    "(a) {tested:?}"
                );
                let key = tested.host_key.clone().expect("(a) host key to confirm");
                assert_eq!(key.key_type, "ssh-ed25519", "(a) key type");
                let fingerprint = Command::new("ssh-keygen")
                    .args(["-l", "-E", "sha256", "-f"])
                    .arg(rig.dir.join("sshd").join("host_key.pub"))
                    .output()
                    .expect("ssh-keygen -l");
                let expected_fp = String::from_utf8_lossy(&fingerprint.stdout)
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                assert!(
                    expected_fp.starts_with("SHA256:"),
                    "fingerprint of the test key"
                );
                assert_eq!(
                    key.fingerprint_sha256, expected_fp,
                    "(a) SHA256 fingerprint"
                );
                // (c)
                let err = client
                    .add_destination(&dest, "init", &strong_password(), &no_secrets())
                    .expect_err("AddDestination without trust");
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.HostKeyUnknown");
                assert_eq!(
                    read(&p.kestic),
                    None,
                    "(c) Kestic's file changed without trust"
                );
                assert_eq!(
                    rig.configured_destinations(),
                    0,
                    "(c) a destination was created"
                );
                assert!(
                    rig.wrapper.of('S', "init").is_empty(),
                    "(c) restic ran init"
                );
                // (b)
                client
                    .trust_host_key(&key.host_key_id)
                    .expect("TrustHostKey");
                let written = fs::read_to_string(&p.kestic).expect("(b) Kestic's known_hosts");
                assert_eq!(
                    written.lines().collect::<Vec<_>>(),
                    vec![rig.server.known_hosts_line().as_str()],
                    "(b) exactly the shown key"
                );
                let again = client
                    .test_destination_connection(&dest, "init", "", &no_secrets())
                    .expect("TestDestinationConnection after trust");
                assert!(again.ok, "(b) the trusted server is known: {again:?}");
                client
                    .add_destination(&dest, "init", &strong_password(), &no_secrets())
                    .expect("AddDestination after trust");
                // (d)
                assert_eq!(read(&p.user), user_before, "(d) ~/.ssh/known_hosts changed");
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-01-5 (FR-30, P32): an SFTP destination whose `sshd` was stopped while its status still
/// says reachable; «Back Up Now» (a) fails the pre-check (`restic cat config` through
/// `sftp.args`), (b) starts no `restic backup`, (c) reports `ManualRunBlocked` «destination
/// unreachable» within 15 s, (d) sets the status to unreachable, and (e) disables «Back Up Now»
/// with that reason. Runs with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_01_5_stopped_sshd_blocks_manual_run_within_15_seconds() {
    run_isolated_test(
        "ac_m5_01_5_stopped_sshd_blocks_manual_run_within_15_seconds",
        "KESTIC_M5_01_5_INNER",
        |root| {
            for variant in restic_variants() {
                let mut rig = start_sftp_rig(root, &variant, "M5SftpE");
                let dest = rig.destination("NAS", "repo");
                rig.add_new(&dest);
                let src = rig.dir.join("src");
                fs::create_dir_all(&src).expect("mkdir src");
                fs::write(src.join("file.txt"), b"hello sftp").expect("write source");
                let backup = Backup::new("Documents", Source::folder(&src), dest.id.clone())
                    .expect("valid backup");
                let backup_id =
                    BackupId::parse(rig.daemon.client.add_backup(&backup).expect("AddBackup"))
                        .expect("backup id");
                let client = rig.daemon.client.clone();
                wait_for_state(&client, Duration::from_secs(20), "reachable", |s| {
                    dest_summary(s, &dest.id).availability.reachable
                });
                let signals =
                    KesticSignalCollector::start(&rig.bus, KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1);
                rig.server.stop();
                let cat_before = rig.wrapper.of('S', "cat").len();

                let started = Instant::now();
                let err = match client.start_backup(backup_id.as_str()) {
                    Err(err) => err,
                    Ok(run) => panic!(
                        "StartBackup on a stopped server started run {run}; restic calls: {:?}; \
                         sshd log: {}",
                        rig.wrapper.entries(),
                        fs::read_to_string(rig.dir.join("sshd").join("sshd.log"))
                            .unwrap_or_default()
                    ),
                };
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.ManualRunBlocked");
                // (a)
                let cats = rig.wrapper.of('E', "cat");
                assert!(
                    rig.wrapper.of('S', "cat").len() > cat_before,
                    "(a) no restic cat config: {:?}",
                    rig.wrapper.entries()
                );
                assert!(
                    cats.last().is_some_and(|e| e.detail.trim() != "0"),
                    "(a) the pre-check passed: {cats:?}"
                );
                let precheck = rig.wrapper.of('S', "cat");
                assert!(
                    precheck
                        .last()
                        .is_some_and(|e| e.detail.contains("sftp.args=")
                            && e.detail.contains("ConnectTimeout=10")),
                    "(a) the pre-check without ConnectTimeout=10 in sftp.args: {precheck:?}"
                );
                // (b)
                assert!(
                    rig.wrapper.of('S', "backup").is_empty(),
                    "(b) restic backup started"
                );
                // (c)
                let sigs = signals.wait_until(Duration::from_secs(15), |s| {
                    s.iter()
                        .any(|sig| matches!(sig, CapturedKesticSignal::ManualRunBlocked { .. }))
                });
                assert!(
                    started.elapsed() <= Duration::from_secs(15),
                    "(c) blocked after {:?}",
                    started.elapsed()
                );
                assert!(
                    sigs.iter().any(|s| matches!(
                        s,
                        CapturedKesticSignal::ManualRunBlocked { reason, .. }
                            if reason == BlockedReason::DestinationUnreachable.as_str()
                    )),
                    "(c) ManualRunBlocked «destination unreachable»: {sigs:?}"
                );
                let state = client.get_state().expect("GetState");
                // (d)
                assert!(
                    !dest_summary(&state, &dest.id).availability.reachable,
                    "(d) status unreachable"
                );
                // (e)
                let availability = &backup_summary(&state, &backup_id).availability;
                assert!(!availability.available, "(e) «Back Up Now» disabled");
                assert_eq!(
                    availability.blocked_reason,
                    Some(BlockedReason::DestinationUnreachable),
                    "(e) with the reason"
                );
                drop(signals);
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-01-7 (FR-01, P24): Kestic's file or `~/.ssh/known_hosts` holds another key of the same
/// type for the server; the connection test and a run (a) stop with «the server identifies itself
/// differently» naming the file with the old entry, (b) offer no trust, (c) leave both files
/// unchanged, and (d) start no restic. Runs with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_01_7_changed_host_key_stops_test_and_run_without_restic() {
    run_isolated_test(
        "ac_m5_01_7_changed_host_key_stops_test_and_run_without_restic",
        "KESTIC_M5_01_7_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_sftp_rig(root, &variant, "M5SftpF");
                let p = paths();
                let dest = rig.destination("NAS", "repo");
                rig.add_new(&dest);
                let src = rig.dir.join("src");
                fs::create_dir_all(&src).expect("mkdir src");
                let backup = Backup::new("Documents", Source::folder(&src), dest.id.clone())
                    .expect("valid backup");
                let backup_id =
                    BackupId::parse(rig.daemon.client.add_backup(&backup).expect("AddBackup"))
                        .expect("backup id");
                fs::create_dir_all(p.user.parent().expect("~/.ssh")).expect("mkdir ~/.ssh");
                let other = format!("{}\n", rig.server.other_key_line());
                let probe = rig.destination("Other", "other");
                let client = &rig.daemon.client;

                for (changed_file, other_file) in [(&p.kestic, &p.user), (&p.user, &p.kestic)] {
                    fs::write(changed_file, &other).expect("write the other key");
                    let _ = fs::remove_file(other_file);
                    let before = (read(&p.kestic), read(&p.user));
                    let restic_before = rig.wrapper.started_repo_count();

                    let tested = client
                        .test_destination_connection(&probe, "init", "", &no_secrets())
                        .expect("TestDestinationConnection");
                    // (a)
                    assert_eq!(
                        tested.error_kind,
                        Some(ConnectionTestCode::HostKeyChanged),
                        "{}: (a) {tested:?}",
                        variant.label
                    );
                    assert_eq!(
                        tested.detail,
                        changed_file.to_string_lossy(),
                        "(a) the file with the old entry"
                    );
                    let UiText::Message(message) =
                        connection_test_text(tested.ok, tested.error_kind, &tested.detail)
                    else {
                        panic!("(a) a raw text instead of the message");
                    };
                    assert!(
                        message
                            .english()
                            .starts_with("The server identifies itself differently than before."),
                        "(a) message: {}",
                        message.english()
                    );
                    // (b)
                    assert_eq!(tested.host_key, None, "(b) a key to trust was offered");

                    let run_id = client
                        .start_backup(backup_id.as_str())
                        .expect("StartBackup");
                    let state = wait_for_state(client, Duration::from_secs(30), "run ended", |s| {
                        let b = backup_summary(s, &backup_id);
                        b.active_run.is_none()
                            && b.last_run.as_ref().is_some_and(|r| r.id.as_str() == run_id)
                    });
                    let summary = backup_summary(&state, &backup_id);
                    // (a)
                    assert_eq!(
                        summary.error_kind,
                        Some(ErrorKind::HostKeyChanged),
                        "(a) run error: {:?}",
                        summary.error_detail
                    );
                    assert!(
                        summary
                            .error_detail
                            .as_deref()
                            .is_some_and(|d| d.contains(&*changed_file.to_string_lossy())),
                        "(a) the run names the file: {:?}",
                        summary.error_detail
                    );
                    // (c)
                    assert_eq!(
                        (read(&p.kestic), read(&p.user)),
                        before,
                        "(c) a known_hosts file changed"
                    );
                    // (d)
                    assert_eq!(
                        rig.wrapper.started_repo_count(),
                        restic_before,
                        "(d) restic started: {:?}",
                        rig.wrapper.entries()
                    );
                }
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-01-8 (FR-01, FR-56): two SFTP destinations on the same host and port; removing the
/// first (a) keeps the server's line in Kestic's `known_hosts`, and (b) removing the second too
/// removes it; the line in `~/.ssh/known_hosts` always stays.
#[test]
fn ac_m5_01_8_host_line_goes_with_the_last_destination_on_the_server() {
    run_isolated_test(
        "ac_m5_01_8_host_line_goes_with_the_last_destination_on_the_server",
        "KESTIC_M5_01_8_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let rig = start_sftp_rig(root, &variant, "M5SftpG");
            let p = paths();
            let first = rig.destination("NAS", "first");
            let second = rig.destination("NAS 2", "second");
            rig.add_new(&first);
            rig.daemon
                .client
                .add_destination(&second, "init", &strong_password(), &no_secrets())
                .expect("AddDestination second");
            fs::create_dir_all(p.user.parent().expect("~/.ssh")).expect("mkdir ~/.ssh");
            fs::write(&p.user, format!("{}\n", rig.server.known_hosts_line()))
                .expect("write ~/.ssh/known_hosts");
            let user_before = read(&p.user);
            let line = rig.server.known_hosts_line();
            let kestic_lines = || {
                fs::read_to_string(&p.kestic)
                    .unwrap_or_default()
                    .lines()
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            };
            assert_eq!(kestic_lines(), vec![line.clone()], "the trusted line");

            rig.daemon
                .client
                .remove_destination(first.id.as_str())
                .expect("RemoveDestination first");
            // (a)
            assert_eq!(
                kestic_lines(),
                vec![line.clone()],
                "(a) line removed too early"
            );
            assert_eq!(read(&p.user), user_before, "(a) ~/.ssh/known_hosts changed");

            rig.daemon
                .client
                .remove_destination(second.id.as_str())
                .expect("RemoveDestination second");
            // (b)
            assert!(
                kestic_lines().is_empty(),
                "(b) line still there: {:?}",
                kestic_lines()
            );
            assert_eq!(read(&p.user), user_before, "(b) ~/.ssh/known_hosts changed");
            rig.daemon.stop();
        },
    );
}

/// M5-S01 (AC-M5-01-4 (a)): a server that no longer offers the key type Kestic's file knows for
/// it (an ecdsa line, the server shows only ed25519) is unknown, not changed: the test offers its
/// ed25519 key, and once trusted the server is reachable.
#[test]
fn m5_01_server_with_a_new_key_type_is_offered_for_trust() {
    run_isolated_test(
        "m5_01_server_with_a_new_key_type_is_offered_for_trust",
        "KESTIC_M5_01_NEWTYPE_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let rig = start_sftp_rig(root, &variant, "M5SftpH");
            let p = paths();
            let ecdsa = rig.dir.join("old_ecdsa_key");
            let generated = Command::new("ssh-keygen")
                .args(["-q", "-t", "ecdsa", "-N", "", "-f"])
                .arg(&ecdsa)
                .status()
                .expect("ssh-keygen ecdsa");
            assert!(generated.success(), "ecdsa key generated");
            let old_line = format!(
                "{} {}",
                rig.server.known_hosts_name(),
                kestic_test_support::sftp_server::public_key(&ecdsa)
            );
            fs::create_dir_all(p.kestic.parent().expect("kestic data")).expect("mkdir");
            fs::write(&p.kestic, format!("{old_line}\n")).expect("write old ecdsa line");
            let dest = rig.destination("NAS", "repo");
            let client = &rig.daemon.client;

            let tested = client
                .test_destination_connection(&dest, "init", "", &no_secrets())
                .expect("TestDestinationConnection");
            assert_eq!(
                tested.error_kind,
                Some(ConnectionTestCode::UnknownHostKey),
                "the new key type is offered: {tested:?}"
            );
            let key = tested.host_key.expect("key to trust");
            assert_eq!(key.key_type, "ssh-ed25519", "the key the server shows");
            client
                .trust_host_key(&key.host_key_id)
                .expect("TrustHostKey");
            assert_eq!(
                fs::read_to_string(&p.kestic)
                    .expect("known_hosts")
                    .lines()
                    .collect::<Vec<_>>(),
                vec![old_line.as_str(), rig.server.known_hosts_line().as_str()],
                "the old line stays, the new key is added"
            );
            let again = client
                .test_destination_connection(&dest, "init", "", &no_secrets())
                .expect("TestDestinationConnection after trust");
            assert!(again.ok, "reachable after trust: {again:?}");
            rig.daemon.stop();
        },
    );
}

/// M5-S01, hosts from the ssh configuration (maintainer decision on #72, Q2): Kestic asks ssh how
/// it reaches a host (`ssh -G`). An alias with `HostName` and `HostKeyAlias` is scanned at the
/// real address and trusted under the alias, and the destination works; a host behind a jump
/// host (`ProxyJump`) gets `jump_host_unsupported` and no destination.
#[test]
fn m5_01_hosts_resolve_through_the_ssh_configuration() {
    run_isolated_test(
        "m5_01_hosts_resolve_through_the_ssh_configuration",
        "KESTIC_M5_01_SSHCONFIG_INNER",
        |root| {
            let variant = common::m3::latest_variant();
            let rig = start_sftp_rig(root, &variant, "M5SftpI");
            let p = paths();
            let config = rig.dir.join("ssh_config_aliases");
            fs::write(
                &config,
                format!(
                    "Host nas-alias\n  HostName 127.0.0.1\n  HostKeyAlias mynas\n\
                     Host behind-gateway\n  ProxyJump gateway.invalid\n{}",
                    fs::read_to_string(&rig.server.ssh_config).expect("read test ssh_config")
                ),
            )
            .expect("write ssh_config");
            set_test_ssh_config(Some(config));
            let client = &rig.daemon.client;
            let with_host = |name: &str, host: &str| {
                let mut dest = rig.destination(name, "repo");
                if let DestinationKind::Sftp { host: h, .. } = &mut dest.kind {
                    *h = host.to_string();
                }
                dest
            };

            let alias = with_host("NAS", "nas-alias");
            rig.add_new(&alias);
            let written = fs::read_to_string(&p.kestic).expect("known_hosts");
            assert_eq!(
                written.lines().collect::<Vec<_>>(),
                vec![format!("mynas {}", rig.server.host_key).as_str()],
                "trusted under the HostKeyAlias"
            );
            assert!(
                rig.repo_dir("repo").join("config").is_file(),
                "the destination behind the alias was created"
            );

            let jumped = with_host("Behind", "behind-gateway");
            let tested = client
                .test_destination_connection(&jumped, "init", "", &no_secrets())
                .expect("TestDestinationConnection");
            assert_eq!(
                tested.error_kind,
                Some(ConnectionTestCode::JumpHostUnsupported),
                "jump host: {tested:?}"
            );
            client
                .add_destination(&jumped, "init", &strong_password(), &no_secrets())
                .expect_err("AddDestination behind a jump host");
            assert_eq!(
                rig.configured_destinations(),
                1,
                "no destination behind the jump host"
            );
            set_test_ssh_config(Some(rig.server.ssh_config.clone()));
            rig.daemon.stop();
        },
    );
}
