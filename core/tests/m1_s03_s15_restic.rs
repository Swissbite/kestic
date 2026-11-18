// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use kestic_core::clock::FakeClock;
use kestic_core::model::{Destination, DestinationKind, RunStatus};
use kestic_core::restic::{
    RecoveryAction, ResticCommandSpec, ResticFailureKind, ResticStreamEvent, classify_exit,
    parse_json_line, redact_secrets, run_restic,
};
use kestic_core::secrets::{InMemorySecretStore, SecretStore, SecretValue};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use tempfile::TempDir;
use time::macros::datetime;

#[test]
fn ac_m1_03_1_tolerant_json_parser_ignores_unknown_fields_and_message_types() {
    let unknown_msg = r#"{"message_type":"future_metric_v2","foo":"bar","nested":[1,2,3]}"#;
    let parsed_unknown = parse_json_line(unknown_msg).expect("unknown message type must parse");
    assert_eq!(
        parsed_unknown,
        ResticStreamEvent::Unknown {
            message_type: Some("future_metric_v2".to_string())
        }
    );

    let summary_with_extra = r#"{
        "message_type":"summary",
        "files_new":12,
        "files_changed":3,
        "files_unmodified":100,
        "dirs_new":2,
        "dirs_changed":1,
        "dirs_unmodified":10,
        "data_added":4096,
        "total_files_processed":115,
        "total_bytes_processed":987654,
        "snapshot_id":"abc123def456",
        "unexpected_future_field":{"ok":true}
    }"#;
    match parse_json_line(summary_with_extra).expect("summary with unknown field must parse") {
        ResticStreamEvent::Summary(stats) => {
            assert_eq!(stats.snapshot_id.as_deref(), Some("abc123def456"));
            assert_eq!(stats.files_new, 12);
            assert_eq!(stats.total_files_processed, 115);
            assert_eq!(stats.total_bytes_processed, 987_654);
        }
        other => panic!("expected Summary, got {other:?}"),
    }
}

#[test]
fn ac_m1_03_2_and_s15_exit_code_classification_and_exit_error() {
    assert_eq!(classify_exit(0, None, vec![], "").status, RunStatus::Ok);

    let w = classify_exit(
        3,
        None,
        vec!["/home/alice/secret.key".to_string()],
        "incomplete",
    );
    assert_eq!(w.status, RunStatus::Warning);
    assert_eq!(w.action, Some(RecoveryAction::AddToExcludes));

    let e10 = classify_exit(10, Some("repository does not exist"), vec![], "");
    assert_eq!(e10.status, RunStatus::Error);
    assert_eq!(e10.kind, ResticFailureKind::RepositoryMissing);

    let e11 = classify_exit(11, Some("unable to create lock in backend"), vec![], "");
    assert_eq!(e11.status, RunStatus::Error);
    assert_eq!(e11.kind, ResticFailureKind::RepositoryLocked);

    let e12 = classify_exit(12, Some("wrong password or no key found"), vec![], "");
    assert_eq!(e12.status, RunStatus::Error);
    assert_eq!(e12.kind, ResticFailureKind::WrongPassword);
    assert_eq!(e12.action, Some(RecoveryAction::UpdateStoredPassword));

    let e1_net = classify_exit(1, Some("dial tcp: connection refused"), vec![], "");
    assert_eq!(e1_net.kind, ResticFailureKind::DestinationUnreachable);

    let e99 = classify_exit(99, None, vec![], "weird backend boom");
    assert_eq!(e99.status, RunStatus::Error);
    assert_eq!(e99.kind, ResticFailureKind::Unknown { exit_code: 99 });
    assert_eq!(e99.raw_detail.as_deref(), Some("weird backend boom"));
    assert!(e99.kind.default_summary().contains("99"));
}

#[test]
fn ac_m1_15_3_exit_error_json_from_real_restic_stderr_is_captured() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let dir = TempDir::new().expect("tempdir");
        let missing_repo = dir.path().join("nonexistent-repo");
        let dest = Destination::new(
            "Missing",
            DestinationKind::Local {
                path: missing_repo.clone(),
            },
        )
        .expect("dest");

        let clock = FakeClock::new(datetime!(2026-12-01 12:00:00 UTC));
        let mut spec = ResticCommandSpec::for_destination(
            &restic_bin,
            "kestic",
            &dest,
            vec!["snapshots".to_string()],
        );
        spec.low_priority = false;
        spec.password_command_override = Some("printf 'test-pw\\n'".to_string());

        let res = run_restic(&spec, &dest, None, &clock, None, &[], |_| {})
            .unwrap_or_else(|e| panic!("{label}: runner failed: {e}"));

        assert_eq!(
            res.exit_code, 10,
            "{label} expected exit 10 on missing repo"
        );
        assert_eq!(res.presentation.kind, ResticFailureKind::RepositoryMissing);
        let detail = res
            .presentation
            .raw_detail
            .as_deref()
            .expect("exit_error message captured");
        assert!(
            detail.contains("does not exist") || detail.contains("config"),
            "{label} unexpected exit_error detail: {detail}"
        );
    }
}

#[test]
fn ac_m1_03_3_and_03_4_nice_ionice_and_no_secrets_in_proc_cmdline() {
    let dir = TempDir::new().expect("tempdir");
    let repo_dir = dir.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("mkdir");

    let dest = Destination::new(
        "REST Test",
        DestinationKind::Rest {
            url: "https://backup.example.invalid/repo".to_string(),
            username: Some("alice".to_string()),
        },
    )
    .expect("dest");

    let store = InMemorySecretStore::new();
    let repo_secret = "SecretRepoPass_NeverInCmdline!";
    let rest_secret = "SecretRestPass_NeverInCmdline!";
    store
        .set_secret(
            dest.id.as_str(),
            "repo",
            &SecretValue::from_str_secret(repo_secret),
        )
        .expect("set repo secret");
    store
        .set_secret(
            dest.id.as_str(),
            "rest",
            &SecretValue::from_str_secret(rest_secret),
        )
        .expect("set rest secret");

    // Fake executable simulating `restic`: inspects /proc/self/stat (field 19 = nice),
    // `ionice -p $$`, `/proc/self/cmdline`, and env vars.
    let probe = dir.path().join("fake-restic");
    let out_file = dir.path().join("probe.out");
    let script = format!(
        r#"#!/usr/bin/env sh
nice_val=$(awk '{{print $19}}' /proc/$$/stat)
io_val=$(ionice -p $$)
cmdline=$(tr '\0' ' ' < /proc/$$/cmdline)
printf 'nice=%s\nio=%s\ncmdline=%s\nhas_pw_env=%s\nhas_pw_cmd=%s\nhas_rest_pw=%s\n' \
    "$nice_val" "$io_val" "$cmdline" \
    "${{RESTIC_PASSWORD:+YES}}" "${{RESTIC_PASSWORD_COMMAND:-NONE}}" "${{RESTIC_REST_PASSWORD:-NONE}}" \
    > {}
printf '{{"message_type":"summary","files_new":1,"snapshot_id":"deadbeef"}}\n'
"#,
        shlex::try_quote(&out_file.to_string_lossy()).unwrap()
    );
    fs::write(&probe, script).expect("write probe");
    let mut perms = fs::metadata(&probe).expect("meta").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&probe, perms).expect("chmod");

    let clock = FakeClock::new(datetime!(2026-12-01 12:00:00 UTC));
    let mut spec = ResticCommandSpec::for_destination(
        &probe,
        "/usr/bin/kestic",
        &dest,
        vec!["backup".to_string(), "/tmp".to_string()],
    );
    spec.low_priority = true;

    let res = run_restic(&spec, &dest, Some(&store), &clock, None, &[], |_| {}).expect("run probe");
    assert_eq!(res.exit_code, 0);

    let captured = fs::read_to_string(&out_file).expect("read probe output");
    assert!(captured.contains("nice=10\n"), "captured:\n{captured}");
    assert!(captured.contains("io=idle\n"), "captured:\n{captured}");
    assert!(
        captured.contains("has_pw_env=\n"),
        "RESTIC_PASSWORD must not be set:\n{captured}"
    );
    assert!(
        captured.contains(&format!(
            "has_pw_cmd=/usr/bin/kestic secret get {}",
            dest.id
        )),
        "captured:\n{captured}"
    );
    assert!(
        !captured.contains(repo_secret),
        "repo secret leaked to cmdline"
    );
    assert!(
        !captured
            .lines()
            .find(|l| l.starts_with("cmdline="))
            .unwrap()
            .contains(rest_secret),
        "rest secret leaked to cmdline"
    );

    // Also verify real `ionice`/`nice` values via command transparency string (AC-M1-14-1, AC-M1-14-2).
    let shell_cmd = spec.to_transparent_shell_command();
    assert!(shell_cmd.contains("ionice -c 3 nice -n 10"));
    assert!(shell_cmd.contains(&format!(
        "RESTIC_PASSWORD_COMMAND='kestic secret get {}'",
        dest.id
    )));
    assert!(shell_cmd.contains(&format!(
        "RESTIC_REST_PASSWORD=\"$(kestic secret get {} rest)\"",
        dest.id
    )));
    assert!(!shell_cmd.contains(repo_secret));
    assert!(!shell_cmd.contains(rest_secret));

    // Verify redaction helper never leaves secrets in logs.
    let redacted = redact_secrets(
        &format!("error with {repo_secret} and {rest_secret}"),
        &[repo_secret, rest_secret],
    );
    assert!(!redacted.contains(repo_secret));
    assert!(!redacted.contains(rest_secret));

    // Sanity-check `Command::new("ionice")` exists on host.
    assert!(Command::new("ionice").arg("-V").status().is_ok());
}
