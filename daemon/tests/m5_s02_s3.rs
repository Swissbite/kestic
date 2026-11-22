// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M5-S02 destination «S3-compatible» against `rclone serve s3` with an access key pair:
//! creating it and the first snapshot (`AC-M5-02-1`), keys only in the Secret Service and the
//! environment of restic (`AC-M5-02-2`), a wrong secret key (`AC-M5-02-3`) and a stopped server
//! before «Back Up Now» (`AC-M5-02-4`), over D-Bus against an in-process daemon on an isolated
//! session bus.

mod common;

use common::m3::{
    M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, assert_dbus_error_name, backup_summary,
    dest_summary, restic_variants, spawn_m3_daemon, wait_for_state,
};
use common::{
    CapturedKesticSignal, KesticSignalCollector, isolated_secret_store, run_isolated_test,
};
use kestic_core::backend_credentials::{SECRET_KIND_S3_ACCESS_KEY, SECRET_KIND_S3_SECRET_KEY};
use kestic_core::model::{Backup, BackupId, Destination, DestinationKind, RunStatus, Source};
use kestic_core::secrets::SecretStore;
use kestic_daemon::dbus_api::{BackendSecrets, KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH};
use kestic_i18n::codes::{BlockedReason, ConnectionNoteCode, ConnectionTestCode};
use kestic_i18n::messages::{UiText, connection_test_text};
use kestic_test_support::password::{s3_key, strong_password};
use kestic_test_support::s3_server::S3ServerGuard;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BUCKET: &str = "kestic-backup";

/// Daemon (real clock, runs on request) behind a restic wrapper that logs every call's
/// arguments, plus an S3 server with an empty bucket.
struct S3Rig {
    daemon: M3Daemon,
    wrapper: ResticWrapper,
    server: S3ServerGuard,
    sub: String,
    bus: String,
}

fn start_s3_rig(root: &Path, variant: &ResticVariant, base: &str) -> S3Rig {
    let sub = variant.sub(base);
    let bus = variant.bus_name(base);
    let wrapper = ResticWrapper::create(&root.join(&sub).join("wrapper"), &variant.bin);
    let daemon = spawn_m3_daemon(root, M3DaemonOpts::on_request(&sub, &bus, &wrapper.path));
    let server = S3ServerGuard::start(&root.join(&sub).join("s3-data"), BUCKET);
    S3Rig {
        daemon,
        wrapper,
        server,
        sub,
        bus,
    }
}

impl S3Rig {
    fn destination(&self, name: &str) -> Destination {
        Destination::new(
            name,
            DestinationKind::S3 {
                endpoint: self.server.endpoint.clone(),
                bucket: BUCKET.to_string(),
                path: None,
                region: None,
            },
        )
        .expect("S3 destination")
    }

    fn keys(&self) -> BackendSecrets {
        BackendSecrets::s3(&self.server.access_key, &self.server.secret_key)
    }

    fn bucket_dir(&self) -> PathBuf {
        self.server.bucket_dir(BUCKET)
    }

    /// Adds the S3 destination with a new repository and a one-file folder backup on it.
    fn add_destination_and_backup(&self, root: &Path) -> (Destination, BackupId) {
        let dest = self.destination("Cloud");
        self.daemon
            .client
            .add_destination(&dest, "init", &strong_password(), &self.keys())
            .expect("AddDestination");
        let src = root.join(&self.sub).join("src");
        fs::create_dir_all(&src).expect("mkdir src");
        fs::write(src.join("file.txt"), b"hello s3").expect("write source");
        let backup =
            Backup::new("Documents", Source::folder(&src), dest.id.clone()).expect("valid backup");
        let backup_id = self.daemon.client.add_backup(&backup).expect("AddBackup");
        (dest, BackupId::parse(&backup_id).expect("backup id"))
    }
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir).map_or_else(
        |_| Vec::new(),
        |entries| entries.flatten().map(|e| e.path()).collect(),
    )
}

/// AC-M5-02-1 (FR-01): with `rclone serve s3` (bucket and key pair), entering endpoint, bucket,
/// access key and secret key and adding the destination (a) runs `init` successfully (the
/// repository config lies in the bucket), and (b) a following backup creates a snapshot in the
/// bucket. Runs with restic 0.18.0 and the latest restic.
#[test]
fn ac_m5_02_1_s3_destination_is_initialised_and_backup_creates_snapshot_in_bucket() {
    run_isolated_test(
        "ac_m5_02_1_s3_destination_is_initialised_and_backup_creates_snapshot_in_bucket",
        "KESTIC_M5_02_1_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_s3_rig(root, &variant, "M5S3A");
                let dest = rig.destination("Cloud");
                let client = &rig.daemon.client;

                let tested = client
                    .test_destination_connection(&dest, "init", &strong_password(), &rig.keys())
                    .expect("TestDestinationConnection");
                assert!(
                    tested.ok,
                    "{}: connection test failed: {tested:?}",
                    variant.label
                );
                client
                    .add_destination(&dest, "init", &strong_password(), &rig.keys())
                    .expect("AddDestination");
                // (a)
                assert!(
                    rig.bucket_dir().join("config").is_file(),
                    "{}: (a) no repository config in the bucket: {:?}",
                    variant.label,
                    files_in(&rig.bucket_dir())
                );

                let src = root.join(&rig.sub).join("src");
                fs::create_dir_all(&src).expect("mkdir src");
                fs::write(src.join("file.txt"), b"hello s3").expect("write source");
                let backup = Backup::new("Documents", Source::folder(&src), dest.id.clone())
                    .expect("valid backup");
                let backup_id =
                    BackupId::parse(client.add_backup(&backup).expect("AddBackup")).expect("id");
                client
                    .start_backup(backup_id.as_str())
                    .expect("StartBackup");
                let state = wait_for_state(client, Duration::from_secs(60), "backup done", |s| {
                    let b = backup_summary(s, &backup_id);
                    b.active_run.is_none() && b.last_run.is_some()
                });
                let last = backup_summary(&state, &backup_id)
                    .last_run
                    .clone()
                    .expect("last run");
                assert_eq!(
                    last.status,
                    RunStatus::Ok,
                    "{}: run {last:?}",
                    variant.label
                );
                // (b)
                assert_eq!(
                    files_in(&rig.bucket_dir().join("snapshots")).len(),
                    1,
                    "{}: (b) exactly one snapshot in the bucket",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-02-2 (FR-01, FR-08): for the S3 destination, `config.toml` (a) contains endpoint and
/// bucket, (b) contains neither the access key nor the secret key, (c) both keys are in the
/// Secret Service, and (d) no restic call of the setup and of a backup has a key in its
/// arguments (the backup still reaches the bucket, so restic got them from the environment).
#[test]
fn ac_m5_02_2_keys_only_in_secret_service_and_never_in_restic_arguments() {
    run_isolated_test(
        "ac_m5_02_2_keys_only_in_secret_service_and_never_in_restic_arguments",
        "KESTIC_M5_02_2_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_s3_rig(root, &variant, "M5S3B");
                let (dest, backup_id) = rig.add_destination_and_backup(root);
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
                let config = fs::read_to_string(&rig.daemon.config_path).expect("read config.toml");

                // (a)
                assert!(
                    config.contains(&format!("endpoint = \"{}\"", rig.server.endpoint)),
                    "(a) endpoint missing in config"
                );
                assert!(
                    config.contains(&format!("bucket = \"{BUCKET}\"")),
                    "(a) bucket missing in config"
                );
                // (b)
                assert!(
                    !config.contains(&rig.server.access_key),
                    "(b) access key in config.toml"
                );
                assert!(
                    !config.contains(&rig.server.secret_key),
                    "(b) secret key in config.toml"
                );
                // (c)
                let store = isolated_secret_store();
                let stored = |kind: &str| {
                    store
                        .get_secret(dest.id.as_str(), kind)
                        .expect("read Secret Service")
                        .map(|v| v.as_str().expect("utf-8").to_string())
                };
                assert!(
                    stored(SECRET_KIND_S3_ACCESS_KEY).as_deref()
                        == Some(rig.server.access_key.as_str()),
                    "(c) access key in the Secret Service"
                );
                assert!(
                    stored(SECRET_KIND_S3_SECRET_KEY).as_deref()
                        == Some(rig.server.secret_key.as_str()),
                    "(c) secret key in the Secret Service"
                );
                // (d)
                assert_eq!(
                    backup_summary(&state, &backup_id)
                        .last_run
                        .as_ref()
                        .map(|r| r.status),
                    Some(RunStatus::Ok),
                    "(d) the backup reached the bucket"
                );
                let calls = fs::read_to_string(&rig.wrapper.log).expect("read wrapper log");
                assert!(calls.contains(" backup "), "(d) the wrapper saw the backup");
                assert!(
                    !calls.contains(&rig.server.access_key),
                    "(d) access key in restic arguments"
                );
                assert!(
                    !calls.contains(&rig.server.secret_key),
                    "(d) secret key in restic arguments"
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-02-3 (FR-01): with a wrong secret key, the connection test (a) fails with a message
/// that tells what to do (check the access data) and does not wait for restic's retries, and
/// (b) no destination is created: neither the test nor `AddDestination` saves anything.
#[test]
fn ac_m5_02_3_wrong_secret_key_fails_with_action_and_creates_nothing() {
    run_isolated_test(
        "ac_m5_02_3_wrong_secret_key_fails_with_action_and_creates_nothing",
        "KESTIC_M5_02_3_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_s3_rig(root, &variant, "M5S3C");
                let dest = rig.destination("Cloud");
                let wrong = BackendSecrets::s3(&rig.server.access_key, &s3_key(40));
                let client = &rig.daemon.client;
                let config_before = fs::read(&rig.daemon.config_path).ok();

                // Connecting without a repository password still checks the login.
                let password = strong_password();
                for (mode, repo_password) in [
                    ("init", password.as_str()),
                    ("connect", password.as_str()),
                    ("connect", ""),
                ] {
                    let started = Instant::now();
                    let tested = client
                        .test_destination_connection(&dest, mode, repo_password, &wrong)
                        .expect("TestDestinationConnection");
                    // (a)
                    assert_eq!(
                        tested.error_kind,
                        Some(ConnectionTestCode::AuthFailed),
                        "(a) {mode}: {tested:?}"
                    );
                    assert!(
                        started.elapsed() < Duration::from_secs(15),
                        "(a) {mode}: the test waited {:?} for restic's retries",
                        started.elapsed()
                    );
                    let text =
                        match connection_test_text(tested.ok, tested.error_kind, &tested.detail) {
                            UiText::Message(m) => m.english(),
                            UiText::Raw(raw) => raw,
                        };
                    assert_eq!(
                        text,
                        "The destination rejected the login. Check the access data and test again.",
                        "(a) {mode}: message with what to do"
                    );
                }

                let err = client
                    .add_destination(&dest, "init", &strong_password(), &wrong)
                    .expect_err("AddDestination with a wrong secret key");
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.AuthFailed");
                // (b)
                assert_eq!(
                    fs::read(&rig.daemon.config_path).ok(),
                    config_before,
                    "(b) config.toml unchanged"
                );
                assert!(
                    client
                        .get_state()
                        .expect("GetState")
                        .destinations
                        .is_empty(),
                    "(b) no destination"
                );
                assert!(
                    !rig.bucket_dir().join("config").exists(),
                    "(b) no repository in the bucket"
                );
                assert_eq!(
                    isolated_secret_store()
                        .get_secret(dest.id.as_str(), SECRET_KIND_S3_SECRET_KEY)
                        .expect("read Secret Service"),
                    None,
                    "(b) no key in the Secret Service"
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-02-4 (FR-30, P32): an S3 destination whose server was stopped while its status still
/// says reachable; «Back Up Now» (a) fails the pre-check (TCP/HTTP, connect timeout 10 s), (b)
/// starts no restic, (c) reports `ManualRunBlocked` «destination unreachable» within 15 s, (d)
/// sets the status to unreachable, and (e) disables «Back Up Now» afterwards.
#[test]
fn ac_m5_02_4_stopped_s3_server_blocks_manual_run_within_15_seconds() {
    run_isolated_test(
        "ac_m5_02_4_stopped_s3_server_blocks_manual_run_within_15_seconds",
        "KESTIC_M5_02_4_INNER",
        |root| {
            for variant in restic_variants() {
                let mut rig = start_s3_rig(root, &variant, "M5S3D");
                let (dest, backup_id) = rig.add_destination_and_backup(root);
                let client = rig.daemon.client.clone();
                let before = client.get_state().expect("GetState");
                assert!(
                    dest_summary(&before, &dest.id).availability.reachable,
                    "the status still says reachable"
                );
                let signals =
                    KesticSignalCollector::start(&rig.bus, KESTIC_OBJECT_PATH, KESTIC_INTERFACE_V1);
                rig.server.stop();
                let restic_before = rig.wrapper.started_repo_count();

                let started = Instant::now();
                let err = client
                    .start_backup(backup_id.as_str())
                    .expect_err("StartBackup on a stopped server");
                // (a)
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.ManualRunBlocked");
                // (b)
                assert_eq!(
                    rig.wrapper.started_repo_count(),
                    restic_before,
                    "(b) no restic call: {:?}",
                    rig.wrapper.entries()
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

/// M5-S02, missing bucket (restic-integration.md §Credentials): the connection test of a new
/// repository in a bucket that does not exist passes with the note `bucket_missing`, whose
/// text names the bucket, and creates nothing; adding the destination then creates the bucket
/// with the repository. Runs with restic 0.18.0 and the latest restic.
#[test]
fn m5_02_missing_bucket_is_named_before_adding_creates_it() {
    run_isolated_test(
        "m5_02_missing_bucket_is_named_before_adding_creates_it",
        "KESTIC_M5_02_BUCKET_INNER",
        |root| {
            for variant in restic_variants() {
                let rig = start_s3_rig(root, &variant, "M5S3E");
                let new_bucket = "kestic-new-bucket";
                let dest = Destination::new(
                    "Cloud",
                    DestinationKind::S3 {
                        endpoint: rig.server.endpoint.clone(),
                        bucket: new_bucket.to_string(),
                        path: None,
                        region: None,
                    },
                )
                .expect("S3 destination");
                let client = &rig.daemon.client;

                let tested = client
                    .test_destination_connection(&dest, "init", &strong_password(), &rig.keys())
                    .expect("TestDestinationConnection");
                assert!(tested.ok, "{}: {tested:?}", variant.label);
                assert_eq!(
                    tested.notes,
                    vec![ConnectionNoteCode::BucketMissing],
                    "{}: the result says the bucket is missing",
                    variant.label
                );
                assert_eq!(
                    ConnectionNoteCode::BucketMissing
                        .message(new_bucket)
                        .english(),
                    "Bucket 'kestic-new-bucket' does not exist yet. Kestic creates it when the destination is added.",
                    "the text names the bucket"
                );
                assert!(
                    !rig.server.bucket_dir(new_bucket).exists(),
                    "{}: the test created the bucket",
                    variant.label
                );

                client
                    .add_destination(&dest, "init", &strong_password(), &rig.keys())
                    .expect("AddDestination");
                assert!(
                    rig.server.bucket_dir(new_bucket).join("config").is_file(),
                    "{}: adding the destination created bucket and repository",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}
