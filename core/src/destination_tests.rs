// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `destination`: connection test time limits, early stops and probe output.

use super::*;
use kestic_test_support::password::strong_password;
use std::os::unix::fs::PermissionsExt;

#[test]
fn retry_duration_is_not_read_as_http_status() {
    let missing = "Stat(<config/>) returned error, retrying after 1.403627s: Stat: The specified bucket does not exist";
    let refused = "Stat(<config/>) returned error, retrying after 401.5ms: dial tcp 127.0.0.1:9: connect: connection refused";
    let signature = "Stat(<config/>) returned error, retrying after 1.2s: Stat: The request signature we calculated does not match the signature you provided.";
    assert!(conclusive_retry_error(missing).is_some());
    assert_eq!(
        classify_probe(
            crate::restic::Transport::Other,
            1,
            Some("Stat: The specified bucket does not exist"),
            ""
        )
        .kind,
        ResticFailureKind::RepositoryMissing
    );
    // A bucket called `nosuchbucket` in the repository address is no missing bucket.
    assert_eq!(
        classify_probe(
            crate::restic::Transport::Other,
            10,
            Some("Fatal: repository does not exist: unable to open config file: Stat: The specified key does not exist.\nIs there a repository at the following location?\ns3:http://127.0.0.1:9000/nosuchbucket"),
            ""
        )
        .kind,
        ResticFailureKind::RepositoryMissing
    );
    assert!(!is_missing_bucket(
        "Is there a repository at the following location?\ns3:http://127.0.0.1:9000/nosuchbucket"
    ));
    assert!(is_missing_bucket(
        "Stat: The specified bucket does not exist"
    ));
    // A run keeps its own reading: a missing bucket is no permanent «repository missing».
    let run = crate::restic::classify_exit(
        1,
        Some("Fatal: unable to open config file: Stat: The specified bucket does not exist"),
        Vec::new(),
        "",
    );
    assert_ne!(run.kind, ResticFailureKind::RepositoryMissing);
    assert!(!run.kind.is_permanent());
    assert!(conclusive_retry_error(refused).is_some());
    assert!(conclusive_retry_error(signature).is_some());
    // The probe reports the error without the duration.
    let reported = |line: &str| {
        classify_probe(
            crate::restic::Transport::Other,
            1,
            conclusive_retry_error(line),
            "",
        )
        .kind
    };
    assert_eq!(reported(refused), ResticFailureKind::DestinationUnreachable);
    assert_eq!(
        reported(
            "Stat(<config/>) returned error, retrying after 403.2ms: Stat: The specified bucket does not exist"
        ),
        ResticFailureKind::RepositoryMissing
    );
    assert_eq!(reported(signature), ResticFailureKind::BackendAuthFailed);
    assert!(
        conclusive_retry_error(
            "Load(<lock/1>) returned error, retrying after 1.4s: unexpected HTTP 500"
        )
        .is_none()
    );
    assert!(conclusive_retry_error("Stat: The specified bucket does not exist").is_none());
    // M5-S01: a server's banner, passed on as a line of ssh, stops no probe.
    assert!(
        conclusive_retry_error("subprocess ssh: Maintenance, retrying after 1s: 401 Unauthorized")
            .is_none()
    );
    let refused_on_port = "Stat(<config/>) returned error, retrying after 508.8ms: Stat: Get \"http://127.0.0.1:40312/bucket/?location=\": dial tcp 127.0.0.1:40312: connect: connection refused";
    assert_eq!(
        crate::restic::classify_exit(1, Some(refused_on_port), Vec::new(), "").kind,
        ResticFailureKind::DestinationUnreachable
    );
    assert!(
    conclusive_retry_error(
        "Stat(<config/>) returned error, retrying after 1.403s: Stat: Please reduce your request rate."
    )
    .is_none()
);
}

/// Whether the process `pid` ends within 3 s; a zombie that waits to be reaped by its new
/// parent counts as ended.
fn process_ends(pid: &str) -> bool {
    for _ in 0..60 {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(_) => return true,
            Ok(stat)
                if stat
                    .rsplit(") ")
                    .next()
                    .is_some_and(|rest| rest.starts_with('Z')) =>
            {
                return true;
            }
            Ok(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    false
}

/// A restic that ignores SIGTERM (a backend program trapping it) is killed after the grace
/// period: the check still ends with «the server does not respond» and leaves no process.
#[test]
fn connection_test_kills_restic_that_ignores_sigterm() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let script = dir.path().join("restic");
    let pid_file = dir.path().join("restic.pid");
    kestic_test_support::tools::write_executable(
        &script,
        &format!(
            "#!/bin/sh\ntrap '' TERM\necho $$ > '{}'\nsleep 60\n",
            pid_file.display()
        ),
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let dest = Destination::new(
        "Stubborn",
        DestinationKind::Local {
            path: dir.path().to_path_buf(),
        },
    )
    .expect("destination");
    let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
    let helper = dir.path().join("kestic");
    let started = std::time::Instant::now();

    let res = run_with_temp_repo_password_file(
        &dest,
        mode_subcommand(DestinationSetupMode::ConnectExisting),
        &SecretValue::from_str_secret(strong_password()),
        &BackendCredentials::none(),
        ProbeRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            clock: &clock,
            cache_dir: None,
            timeout: Some(Duration::from_secs(1)),
            stop_on_unreachable: true,
            host_key_checked: false,
        },
    );

    assert!(
        matches!(res, Err(DestinationError::NotResponding(_))),
        "a restic that ignores SIGTERM ends as not responding"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "killed after the grace period, took {:?}",
        started.elapsed()
    );
    let pid = fs::read_to_string(&pid_file).expect("restic started");
    assert!(
        !Path::new(&format!("/proc/{}", pid.trim())).exists(),
        "restic still runs"
    );
}

/// A process the backend leaves below restic (here a subshell) that keeps restic's output
/// open does not stretch the time limit: the whole process group is stopped.
#[test]
fn connection_test_stops_processes_below_restic() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let script = dir.path().join("restic");
    let pid_file = dir.path().join("grandchild.pid");
    kestic_test_support::tools::write_executable(
        &script,
        &format!(
            "#!/bin/sh\n(echo $$ > /dev/null; sh -c 'echo $$ > \"{}\"; exec sleep 25'; true) &\nwait\n",
            pid_file.display()
        ),
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let dest = Destination::new(
        "Wrapped",
        DestinationKind::Local {
            path: dir.path().to_path_buf(),
        },
    )
    .expect("destination");
    let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
    let helper = dir.path().join("kestic");
    let started = std::time::Instant::now();

    let res = run_with_temp_repo_password_file(
        &dest,
        mode_subcommand(DestinationSetupMode::ConnectExisting),
        &SecretValue::from_str_secret(strong_password()),
        &BackendCredentials::none(),
        ProbeRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            clock: &clock,
            cache_dir: None,
            timeout: Some(Duration::from_secs(1)),
            stop_on_unreachable: true,
            host_key_checked: false,
        },
    );

    assert!(
        matches!(res, Err(DestinationError::NotResponding(_))),
        "the check ends as not responding"
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the process below restic kept the check waiting {:?}",
        started.elapsed()
    );
    let pid = fs::read_to_string(&pid_file).expect("process below restic started");
    assert!(
        process_ends(pid.trim()),
        "the process below restic still runs"
    );
}

/// restic that ends by itself while a process it started keeps its output open: the check
/// gives up waiting for the output after the time limit and reports restic's own result.
#[test]
fn connection_test_does_not_wait_for_output_held_after_restic_ended() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let script = dir.path().join("restic");
    let pid_file = dir.path().join("left-behind.pid");
    kestic_test_support::tools::write_executable(
        &script,
        &format!(
            "#!/bin/sh\nsh -c 'echo $$ > \"{}\"; exec sleep 30' &\nexit 0\n",
            pid_file.display()
        ),
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let dest = Destination::new(
        "Lingering",
        DestinationKind::Local {
            path: dir.path().to_path_buf(),
        },
    )
    .expect("destination");
    let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
    let helper = dir.path().join("kestic");
    let started = std::time::Instant::now();

    let res = run_with_temp_repo_password_file(
        &dest,
        mode_subcommand(DestinationSetupMode::ConnectExisting),
        &SecretValue::from_str_secret(strong_password()),
        &BackendCredentials::none(),
        ProbeRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            clock: &clock,
            cache_dir: None,
            timeout: Some(Duration::from_secs(1)),
            stop_on_unreachable: true,
            host_key_checked: false,
        },
    );

    assert!(
        res.is_ok_and(|r| r.exit_code == 0),
        "restic's own result counts"
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "waited {:?} for output held by a left-behind process",
        started.elapsed()
    );
    // The test started the left-behind process, so it ends it by its PID.
    let pid = fs::read_to_string(&pid_file).expect("left-behind process started");
    Command::new("kill")
        .arg(pid.trim())
        .status()
        .expect("stop the left-behind process");
}

/// The check before a run does not give up on a connection error restic retries: a backend
/// that answers within the time limit after one reset connection passes.
#[test]
fn restic_preflight_waits_out_a_passing_connection_error() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let script = dir.path().join("restic");
    kestic_test_support::tools::write_executable(
        &script,
        "#!/bin/sh\n\
         echo 'Load(<config/0000000000>, 0, 0) returned error, retrying after 552ms: read tcp 10.0.0.1:5000->192.0.2.4:443: read: connection reset by peer' >&2\n\
         sleep 2\n\
         echo '{}'\n\
         exit 0\n",
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let dest = Destination::new(
        "Cloud",
        DestinationKind::Generic {
            url: "b2:bucket:laptop".to_string(),
            env: Vec::new(),
        },
    )
    .expect("destination");
    let store = crate::secrets::InMemorySecretStore::new();
    let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
    let helper = dir.path().join("kestic");
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
        cache_dir: None,
    };

    assert!(
        preflight_check_with_restic(&dest, &runtime).is_ok(),
        "a backend that answers after one reset connection is reachable"
    );
}

/// A password store that cannot be read says nothing about the destination: the pre-check of
/// a generic destination passes and leaves the error to the run.
#[test]
fn restic_preflight_passes_when_the_password_store_fails() {
    struct LockedStore;
    impl SecretStore for LockedStore {
        fn set_secret(&self, _: &str, _: &str, _: &SecretValue) -> Result<(), SecretError> {
            Err(SecretError::LockedOrPromptRequired("locked".to_string()))
        }
        fn get_secret(&self, _: &str, _: &str) -> Result<Option<SecretValue>, SecretError> {
            Err(SecretError::LockedOrPromptRequired("locked".to_string()))
        }
        fn delete_secret(&self, _: &str, _: &str) -> Result<(), SecretError> {
            Err(SecretError::LockedOrPromptRequired("locked".to_string()))
        }
    }
    let dir = tempfile::TempDir::new().expect("tempdir");
    let marker = dir.path().join("started");
    let script = dir.path().join("restic");
    kestic_test_support::tools::write_executable(
        &script,
        &format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let dest = Destination::new(
        "Cloud",
        DestinationKind::Generic {
            url: "b2:bucket:laptop".to_string(),
            env: vec![crate::generic::EnvVar {
                name: "B2_ACCOUNT_KEY".to_string(),
                secret: true,
                value: None,
            }],
        },
    )
    .expect("destination");
    let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
    let helper = dir.path().join("kestic");
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &LockedStore,
        clock: &clock,
        cache_dir: None,
    };

    assert!(
        preflight_check_with_restic(&dest, &runtime).is_ok(),
        "a locked password store is no unreachable destination"
    );
    assert!(
        !marker.exists(),
        "restic started without the secret variable"
    );
}

/// A connection test whose restic does not answer ends after the time limit with «the server
/// does not respond» and stops restic.
#[test]
fn connection_test_stops_restic_that_does_not_answer() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let script = dir.path().join("restic");
    let pid_file = dir.path().join("restic.pid");
    kestic_test_support::tools::write_executable(
        &script,
        &format!(
            "#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n",
            pid_file.display()
        ),
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let dest = Destination::new(
        "Slow",
        DestinationKind::Local {
            path: dir.path().to_path_buf(),
        },
    )
    .expect("destination");
    let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
    let helper = dir.path().join("kestic");
    let started = std::time::Instant::now();

    let res = run_with_temp_repo_password_file(
        &dest,
        mode_subcommand(DestinationSetupMode::ConnectExisting),
        &SecretValue::from_str_secret(strong_password()),
        &BackendCredentials::none(),
        ProbeRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            clock: &clock,
            cache_dir: None,
            timeout: Some(Duration::from_secs(1)),
            stop_on_unreachable: true,
            host_key_checked: false,
        },
    );

    assert!(
        matches!(res, Err(DestinationError::NotResponding(_))),
        "{res:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "waited {:?}",
        started.elapsed()
    );
    let pid = fs::read_to_string(&pid_file).expect("restic started");
    assert!(
        !Path::new(&format!("/proc/{}", pid.trim())).exists(),
        "restic still runs"
    );
}

/// The connection test waits at most 15 s for restic; setup has no limit.
#[test]
fn connection_test_limit_is_15_seconds() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let resolved = ResolvedRestic {
        binary: dir.path().join("restic"),
        version: semver::Version::new(0, 18, 0),
    };
    let helper = dir.path().join("kestic");
    let store = crate::secrets::InMemorySecretStore::new();
    let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
        cache_dir: None,
    };
    assert_eq!(
        ProbeRuntime::connection_test(&runtime).timeout,
        Some(Duration::from_secs(15))
    );
    assert_eq!(ProbeRuntime::without_timeout(&runtime).timeout, None);
    assert_eq!(
        ProbeRuntime::setup(DestinationSetupMode::ConnectExisting, &runtime).timeout,
        Some(Duration::from_secs(15)),
        "connecting an existing repository has the same limit"
    );
    assert_eq!(
        ProbeRuntime::setup(DestinationSetupMode::InitNew, &runtime).timeout,
        None,
        "init has no limit"
    );
}

/// Setting up in mode `connect` stops restic at a conclusive retry error, as the connection
/// test does; `init` lets restic run to its end.
#[test]
fn setup_connect_stops_early_and_init_runs_to_end() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let script = dir.path().join("restic");
    kestic_test_support::tools::write_executable(
        &script,
        "#!/bin/sh\n\
         echo 'Stat(<config/>) returned error, retrying after 1.2s: Stat: The request signature we calculated does not match the signature you provided.' >&2\n\
         sleep 3\n\
         exit 1\n",
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let repo = dir.path().join("repo");
    fs::create_dir(&repo).expect("repo dir");
    let dest =
        Destination::new("Revoked", DestinationKind::Local { path: repo }).expect("destination");
    let helper = dir.path().join("kestic");
    let store = crate::secrets::InMemorySecretStore::new();
    let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &helper,
        secret_store: &store,
        clock: &clock,
        cache_dir: None,
    };
    let password = SecretValue::from_str_secret(strong_password());
    let setup = |mode| {
        let started = std::time::Instant::now();
        let res = setup_destination(
            &dest,
            mode,
            &password,
            &BackendCredentials::none(),
            &runtime,
        );
        (res, started.elapsed())
    };

    let (connect, connect_took) = setup(DestinationSetupMode::ConnectExisting);
    assert!(
        matches!(connect, Err(DestinationError::AuthFailed(_))),
        "connect: a rejected login stops restic with AuthFailed"
    );
    assert!(
        connect_took < Duration::from_secs(2),
        "connect stopped early, took {connect_took:?}"
    );

    let (init, init_took) = setup(DestinationSetupMode::InitNew);
    assert!(init.is_err(), "init: the stand-in fails");
    assert!(
        init_took >= Duration::from_secs(3),
        "init ran to its end, took {init_took:?}"
    );
}

/// Updating the stored repository password reads the repository like connecting does: a
/// conclusive retry error stops restic at once, and the stored password stays.
#[test]
fn password_update_stops_early_and_keeps_the_stored_password() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let script = dir.path().join("restic");
    kestic_test_support::tools::write_executable(
        &script,
        "#!/bin/sh\n\
         echo 'Stat(<config/>) returned error, retrying after 1.2s: Stat: The request signature we calculated does not match the signature you provided.' >&2\n\
         sleep 30\n\
         exit 1\n",
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let repo = dir.path().join("repo");
    fs::create_dir(&repo).expect("repo dir");
    let dest =
        Destination::new("Revoked", DestinationKind::Local { path: repo }).expect("destination");
    let store = crate::secrets::InMemorySecretStore::new();
    let old_password = strong_password();
    store
        .set_secret(
            dest.id.as_str(),
            SECRET_KIND_REPO,
            &SecretValue::from_str_secret(&old_password),
        )
        .expect("store password");
    let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
    let started = std::time::Instant::now();

    let res = verify_and_update_stored_password(
        &dest,
        &SecretValue::from_str_secret(strong_password()),
        &store,
        &resolved,
        &dir.path().join("kestic"),
        &clock,
    );

    assert!(
        matches!(res, Err(DestinationError::AuthFailed(_))),
        "a rejected login ends the check with AuthFailed"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "stopped early, took {:?}",
        started.elapsed()
    );
    assert!(
        store
            .get_secret(dest.id.as_str(), SECRET_KIND_REPO)
            .expect("read store")
            .is_some_and(|v| v.as_str().expect("utf-8") == old_password),
        "stored password unchanged"
    );
}

/// A line that is no UTF-8 does not end the reading: the time limit still stops restic.
#[test]
fn connection_test_reads_on_after_a_line_that_is_no_utf8() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let script = dir.path().join("restic");
    kestic_test_support::tools::write_executable(
        &script,
        "#!/bin/sh\nprintf '\\377\\n' >&2\nexec sleep 60\n",
    );
    let resolved = ResolvedRestic {
        binary: script,
        version: semver::Version::new(0, 18, 0),
    };
    let dest = Destination::new(
        "Slow",
        DestinationKind::Local {
            path: dir.path().to_path_buf(),
        },
    )
    .expect("destination");
    let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
    let helper = dir.path().join("kestic");
    let started = std::time::Instant::now();

    let res = run_with_temp_repo_password_file(
        &dest,
        mode_subcommand(DestinationSetupMode::ConnectExisting),
        &SecretValue::from_str_secret(strong_password()),
        &BackendCredentials::none(),
        ProbeRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            clock: &clock,
            cache_dir: None,
            timeout: Some(Duration::from_secs(1)),
            stop_on_unreachable: true,
            host_key_checked: false,
        },
    );

    assert!(
        matches!(res, Err(DestinationError::NotResponding(_))),
        "{res:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "waited {:?}",
        started.elapsed()
    );
}

#[test]
fn probe_password_file_is_private() {
    let dir = create_private_temp_dir().expect("create probe dir");
    let password = strong_password();
    let file = write_private_password_file(&dir, &SecretValue::from_str_secret(&password))
        .expect("write password file");

    let dir_mode = fs::metadata(&dir)
        .expect("dir metadata")
        .permissions()
        .mode();
    let file_mode = fs::metadata(&file)
        .expect("file metadata")
        .permissions()
        .mode();
    let content = fs::read(&file).expect("read password file");
    fs::remove_dir_all(&dir).expect("remove probe dir");

    // (a) the directory is owner-only
    assert_eq!(dir_mode & 0o777, 0o700);
    // (b) the password file is owner read/write only
    assert_eq!(file_mode & 0o777, 0o600);
    // (c) the file holds exactly the password
    assert!(
        content == password.as_bytes(),
        "password file content differs"
    );
}
