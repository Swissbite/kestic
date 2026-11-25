// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M5-S03 generic form (repository URL and environment variables) over D-Bus against an
//! in-process daemon on an isolated session bus. The repository is an rclone remote on the local
//! disk (`rclone::local:<dir>`), so restic runs its real rclone backend: a secret variable only
//! in the Secret Service and restic's environment (`AC-M5-03-2`), rclone missing
//! (`AC-M5-03-3`), the pre-check with `restic cat config` and its 15 s limit (`AC-M5-03-5`) and
//! refused variables over D-Bus (`AC-M5-03-7`).

mod common;

use common::m3::{
    M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, assert_dbus_error_name, backup_summary,
    dest_summary, latest_variant, restic_variants, spawn_m3_daemon, wait_for_state,
};
use common::{
    CapturedKesticSignal, KesticSignalCollector, isolated_secret_store, run_isolated_test,
};
use kestic_core::generic::{EnvVar, secret_kind};
use kestic_core::model::{Backup, BackupId, Destination, DestinationKind, RunStatus, Source};
use kestic_core::secrets::SecretStore;
use kestic_daemon::dbus_api::{BackendSecrets, KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH};
use kestic_i18n::codes::{BlockedReason, ConnectionTestCode};
use kestic_test_support::password::strong_password;
use kestic_test_support::s3_server::rclone_binary;
use kestic_test_support::tools::write_executable;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const TOKEN_VAR: &str = "KESTIC_TEST_TOKEN";

/// Daemon (real clock, runs on request) whose restic is a script that logs each call's
/// environment, puts rclone on the `PATH` and calls a [`ResticWrapper`] around the real restic.
struct GenericRig {
    daemon: M3Daemon,
    wrapper: ResticWrapper,
    env_log: PathBuf,
    repo: PathBuf,
    sub: String,
    bus: String,
}

fn start_rig(root: &Path, variant: &ResticVariant, base: &str, with_rclone: bool) -> GenericRig {
    let sub = variant.sub(base);
    let bus = variant.bus_name(base);
    let dir = root.join(&sub);
    let wrapper = ResticWrapper::create(&dir.join("wrapper"), &variant.bin);
    let rclone = rclone_binary();
    let rclone_dir = rclone.parent().expect("rclone directory").to_path_buf();
    let env_log = dir.join("restic-env.log");
    let script = dir.join("restic-generic.sh");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\n{{ echo \"== $*\"; /usr/bin/env; }} >> '{log}'\nPATH='{rclone}':\"$PATH\"\nexport PATH\nexec '{wrapper}' \"$@\"\n",
            log = env_log.display(),
            rclone = rclone_dir.display(),
            wrapper = wrapper.path.display()
        ),
    );
    let programs = dir.join("programs");
    fs::create_dir_all(&programs).expect("mkdir programs");
    let search_path = if with_rclone { rclone_dir } else { programs };
    let mut opts = M3DaemonOpts::on_request(&sub, &bus, &script);
    opts.program_search_path = Some(search_path.into_os_string());
    let daemon = spawn_m3_daemon(root, opts);
    GenericRig {
        daemon,
        wrapper,
        env_log,
        repo: dir.join("repo"),
        sub,
        bus,
    }
}

impl GenericRig {
    /// rclone remote on the local disk with a secret token and a plain variable.
    fn destination(&self) -> Destination {
        Destination::new(
            "Cloud",
            DestinationKind::Generic {
                url: format!("rclone::local:{}", self.repo.display()),
                env: vec![
                    EnvVar {
                        name: TOKEN_VAR.to_string(),
                        secret: true,
                        value: None,
                    },
                    EnvVar {
                        name: "KESTIC_TEST_PLAIN".to_string(),
                        secret: false,
                        value: Some("visible".to_string()),
                    },
                ],
            },
        )
        .expect("generic destination")
    }

    /// Adds the destination with a new repository and a one-file folder backup on it.
    fn add_destination_and_backup(&self, root: &Path, token: &str) -> (Destination, BackupId) {
        let dest = self.destination();
        self.daemon
            .client
            .add_destination(
                &dest,
                "init",
                &strong_password(),
                &BackendSecrets::env(&[(TOKEN_VAR, token)]),
            )
            .expect("AddDestination");
        let src = root.join(&self.sub).join("src");
        fs::create_dir_all(&src).expect("mkdir src");
        fs::write(src.join("file.txt"), b"hello generic").expect("write source");
        let backup =
            Backup::new("Documents", Source::folder(&src), dest.id.clone()).expect("valid backup");
        let backup_id = self.daemon.client.add_backup(&backup).expect("AddBackup");
        (dest, BackupId::parse(&backup_id).expect("backup id"))
    }

    /// The environment logged for the first restic call whose arguments contain `sub`.
    fn env_of(&self, sub: &str) -> Vec<String> {
        let log = fs::read_to_string(&self.env_log).expect("read env log");
        log.split("== ")
            .find(|call| call.lines().next().is_some_and(|args| args.contains(sub)))
            .map_or_else(Vec::new, |call| {
                call.lines().skip(1).map(ToOwned::to_owned).collect()
            })
    }
}

/// AC-M5-03-2 (FR-01, FR-08): a destination of the generic form with a variable marked secret,
/// saved over D-Bus and backed up: (a) the secret value is in the Secret Service, (b) not in
/// `config.toml`, and (c) restic gets it as environment variable (and never as argument), so the
/// backup creates a snapshot. Runs with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_03_2_secret_variable_in_secret_service_and_restic_environment() {
    run_isolated_test(
        "ac_m5_03_2_secret_variable_in_secret_service_and_restic_environment",
        "KESTIC_M5_03_2_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_rig(root, &variant, "M5G2", true);
                let token = strong_password();
                let (dest, backup_id) = rig.add_destination_and_backup(root, &token);
                rig.daemon
                    .client
                    .start_backup(backup_id.as_str())
                    .expect("StartBackup");
                let state = wait_for_state(
                    &rig.daemon.client,
                    Duration::from_secs(60),
                    "backup done",
                    |s| {
                        let b = backup_summary(s, &backup_id);
                        b.active_run.is_none() && b.last_run.is_some()
                    },
                );
                assert_eq!(
                    backup_summary(&state, &backup_id)
                        .last_run
                        .as_ref()
                        .map(|r| r.status),
                    Some(RunStatus::Ok),
                    "{}: the backup ran",
                    variant.label
                );

                // (a)
                let stored = isolated_secret_store()
                    .get_secret(dest.id.as_str(), &secret_kind(TOKEN_VAR))
                    .expect("read Secret Service");
                assert!(
                    stored.is_some_and(|v| v.as_str().expect("utf-8") == token),
                    "(a) secret value in the Secret Service"
                );
                // (b)
                let config = fs::read_to_string(&rig.daemon.config_path).expect("read config");
                assert!(!config.contains(&token), "(b) secret value in config.toml");
                assert!(config.contains(TOKEN_VAR), "(b) the variable is named");
                // (c)
                let backup_env = rig.env_of(" backup ");
                assert!(
                    backup_env.contains(&format!("{TOKEN_VAR}={token}")),
                    "(c) restic got the secret variable"
                );
                assert!(
                    backup_env.contains(&"KESTIC_TEST_PLAIN=visible".to_string()),
                    "(c) and the plain variable"
                );
                let calls = fs::read_to_string(&rig.wrapper.log).expect("read wrapper log");
                assert!(
                    !calls.contains(&token),
                    "(c) secret value in restic arguments"
                );
                assert_eq!(
                    fs::read_dir(rig.repo.join("snapshots")).map_or(0, Iterator::count),
                    1,
                    "(c) the backup reached the rclone remote"
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-03-3 (FR-01), daemon part: without `rclone` in the daemon's `PATH`, (a) the connection
/// test of an rclone destination reports `program_missing` with the program (the M5-S04 hint),
/// adding it is refused, and (b) restic is not started at all.
#[test]
fn ac_m5_03_3_rclone_missing_is_reported_and_restic_not_started() {
    run_isolated_test(
        "ac_m5_03_3_rclone_missing_is_reported_and_restic_not_started",
        "KESTIC_M5_03_3_INNER",
        |root| {
            let rig = start_rig(root, &latest_variant(), "M5G3", false);
            let dest = rig.destination();
            let secrets = BackendSecrets::env(&[(TOKEN_VAR, "x")]);
            let client = &rig.daemon.client;
            let restic_before = rig.wrapper.started_repo_count();

            // (a)
            let tested = client
                .test_destination_connection(&dest, "init", &strong_password(), &secrets)
                .expect("TestDestinationConnection");
            assert_eq!(
                tested.error_kind,
                Some(ConnectionTestCode::ProgramMissing),
                "(a) {tested:?}"
            );
            assert_eq!(
                tested.program.as_deref(),
                Some("rclone"),
                "(a) program named"
            );
            let err = client
                .add_destination(&dest, "init", &strong_password(), &secrets)
                .expect_err("AddDestination without rclone");
            assert_dbus_error_name(&err, "net.daester.Kestic.Error.InvalidInput");
            // (b)
            assert_eq!(
                rig.wrapper.started_repo_count(),
                restic_before,
                "(b) restic started: {:?}",
                rig.wrapper.entries()
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M5-03-5 (FR-30, P32): before a manual run of a generic destination whose restic call does
/// not answer (the `cat` of the wrapper is held), Kestic (a) runs `restic cat config`, (b) stops
/// the process after 15 s, (c) starts no `restic backup`, (d) sets the status to «nicht
/// erreichbar», and (e) reports «Ziel nicht erreichbar» (`ManualRunBlocked`); (f) when restic
/// answers within 15 s the run starts normally. Runs with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_03_5_unanswered_preflight_blocks_run_after_15_seconds() {
    run_isolated_test(
        "ac_m5_03_5_unanswered_preflight_blocks_run_after_15_seconds",
        "KESTIC_M5_03_5_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_rig(root, &variant, "M5G5", true);
                let (dest, backup_id) = rig.add_destination_and_backup(root, &strong_password());
                let client = rig.daemon.client.clone();
                let signals =
                    KesticSignalCollector::start(&rig.bus, KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1);
                rig.wrapper.hold("cat");
                let cat_before = rig.wrapper.of('S', "cat").len();

                let started = Instant::now();
                let err = client
                    .start_backup(backup_id.as_str())
                    .expect_err("StartBackup on a destination that does not answer");
                let blocked_after = started.elapsed();
                // (a)
                let cats = rig.wrapper.of('S', "cat");
                assert!(
                    cats.iter().skip(cat_before).any(|call| {
                        // The first argument after `cat` that is no flag names what restic reads.
                        let mut args = call.detail.split_whitespace();
                        args.any(|arg| arg == "cat")
                            && args.find(|arg| !arg.starts_with('-')) == Some("config")
                    }),
                    "(a) restic cat config ran: {:?}",
                    rig.wrapper.entries()
                );
                // (b)
                assert!(
                    blocked_after >= Duration::from_secs(15),
                    "(b) stopped after {blocked_after:?}, before the 15 s limit"
                );
                assert!(
                    blocked_after < Duration::from_secs(22),
                    "(b) stopped only after {blocked_after:?}"
                );
                assert!(
                    !rig.wrapper.of('T', "cat").is_empty(),
                    "(b) the held restic got SIGTERM: {:?}",
                    rig.wrapper.entries()
                );
                // (c)
                assert!(
                    rig.wrapper.of('S', "backup").is_empty(),
                    "(c) restic backup started"
                );
                let state = client.get_state().expect("GetState");
                // (d)
                assert!(
                    !dest_summary(&state, &dest.id).availability.reachable,
                    "(d) status unreachable"
                );
                // (e)
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.ManualRunBlocked");
                let sigs = signals.wait_until(Duration::from_secs(5), |s| {
                    s.iter()
                        .any(|sig| matches!(sig, CapturedKesticSignal::ManualRunBlocked { .. }))
                });
                assert!(
                    sigs.iter().any(|s| matches!(
                        s,
                        CapturedKesticSignal::ManualRunBlocked { reason, .. }
                            if reason == BlockedReason::DestinationUnreachable.as_str()
                    )),
                    "(e) ManualRunBlocked «destination unreachable»: {sigs:?}"
                );

                // (f)
                rig.wrapper.release("cat");
                client
                    .start_backup(backup_id.as_str())
                    .expect("StartBackup once restic answers");
                let state = wait_for_state(&client, Duration::from_secs(60), "backup done", |s| {
                    let b = backup_summary(s, &backup_id);
                    b.active_run.is_none() && b.last_run.is_some()
                });
                assert_eq!(
                    backup_summary(&state, &backup_id)
                        .last_run
                        .as_ref()
                        .map(|r| r.status),
                    Some(RunStatus::Ok),
                    "(f) the run started normally"
                );
                assert_eq!(
                    rig.wrapper.of('S', "backup").len(),
                    1,
                    "(f) one restic backup"
                );
                drop(signals);
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-03-7 (FR-01, NFR-02), D-Bus part: a variable named `RESTIC_PASSWORD_COMMAND`,
/// `RESTIC_REPOSITORY_FILE`, `PATH`, `HOME`, `LD_PRELOAD` or outside `[A-Z_][A-Z0-9_]*` (a) is
/// refused by `core` over D-Bus too: the connection test reports
/// `reserved_variable` with the name, `AddDestination` fails, nothing is saved and (c) restic is
/// not started.
#[test]
fn ac_m5_03_7_reserved_variable_is_refused_over_dbus() {
    run_isolated_test(
        "ac_m5_03_7_reserved_variable_is_refused_over_dbus",
        "KESTIC_M5_03_7_INNER",
        |root| {
            let rig = start_rig(root, &latest_variant(), "M5G7", true);
            let client = &rig.daemon.client;
            let config_before = fs::read(&rig.daemon.config_path).ok();
            let restic_before = rig.wrapper.started_repo_count();
            for name in [
                "RESTIC_PASSWORD_COMMAND",
                "RESTIC_REPOSITORY_FILE",
                "PATH",
                "HOME",
                "LD_PRELOAD",
                "my_var",
                "1TOKEN",
                "B2-KEY",
                "",
            ] {
                let mut dest = rig.destination();
                dest.kind = DestinationKind::Generic {
                    url: format!("rclone::local:{}", rig.repo.display()),
                    env: vec![EnvVar {
                        name: name.to_string(),
                        secret: false,
                        value: Some("/tmp/x".to_string()),
                    }],
                };
                // (a)
                let tested = client
                    .test_destination_connection(
                        &dest,
                        "init",
                        &strong_password(),
                        &BackendSecrets::default(),
                    )
                    .expect("TestDestinationConnection");
                assert_eq!(
                    tested.error_kind,
                    Some(ConnectionTestCode::ReservedVariable),
                    "(a) {name}: {tested:?}"
                );
                assert_eq!(tested.detail, name, "(a) the name is the detail");
                let err = client
                    .add_destination(
                        &dest,
                        "init",
                        &strong_password(),
                        &BackendSecrets::default(),
                    )
                    .expect_err("AddDestination with a reserved variable");
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.InvalidInput");
            }
            assert_eq!(
                fs::read(&rig.daemon.config_path).ok(),
                config_before,
                "(a) nothing saved"
            );
            // (c)
            assert_eq!(
                rig.wrapper.started_repo_count(),
                restic_before,
                "(c) restic started: {:?}",
                rig.wrapper.entries()
            );
            rig.daemon.stop();
        },
    );
}
