// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M5-S01, how restic hands Kestic's ssh options to ssh (`architecture.md` «SFTP und
//! Host-Schlüssel»): `sftp.args` with `known_hosts` paths and an ssh configuration that contain
//! spaces, and an absolute repository path with a space (`sftp://u@h:p//abs%20path`), against a
//! test `sshd`. Runs with restic 0.18.0 and the latest restic.

mod common;

use kestic_core::model::DestinationKind;
use kestic_core::sftp::{KnownHostsPaths, restic_options_for, set_test_ssh_config};
use kestic_test_support::sftp_server::SftpServerGuard;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn restic(
    bin: &Path,
    repo: &str,
    options: &[String],
    password_file: &Path,
    sub: &[&str],
) -> Output {
    Command::new(bin)
        .arg("-r")
        .arg(repo)
        .args(options)
        .args(["--no-cache", "--password-file"])
        .arg(password_file)
        .args(sub)
        .env_remove("SSH_AUTH_SOCK")
        .output()
        .expect("run restic")
}

/// AC-M5-01-1 (d), AC-M5-01-2 (b): restic splits `sftp.args` like a shell, so the quoted
/// `UserKnownHostsFile` list and `-F` keep their spaces: with the server's key only in Kestic's
/// file below a folder with a space, `init` and `cat config` reach the repository at an absolute
/// path with a space; with that file empty, ssh refuses the server (strict checking, no prompt).
#[test]
fn m5_01_sftp_args_with_spaces_reach_ssh_unsplit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("with space");
    let server = SftpServerGuard::start(&root.join("ssh server"));
    let paths = KnownHostsPaths {
        kestic: root.join("data dir").join("kestic").join("known_hosts"),
        user: root.join("home dir").join(".ssh").join("known_hosts"),
        system: "/etc/ssh/ssh_known_hosts".into(),
    };
    fs::create_dir_all(paths.kestic.parent().expect("parent")).expect("mkdir kestic data");
    fs::write(&paths.kestic, format!("{}\n", server.known_hosts_line()))
        .expect("write known_hosts");
    set_test_ssh_config(Some(server.ssh_config.clone()));
    let options = restic_options_for(&paths).expect("quotable paths");
    let password_file = root.join("pw");
    fs::write(&password_file, "kestic-sftp-test").expect("write password file");

    for (label, bin) in common::restic_test_binaries() {
        let repo_dir = root.join(format!("repo {label}"));
        let repo = DestinationKind::Sftp {
            host: "127.0.0.1".to_string(),
            port: server.port,
            user: server.user.clone(),
            path: repo_dir.to_string_lossy().into_owned(),
        }
        .repository_uri();

        let init = restic(&bin, &repo, &options, &password_file, &["init"]);
        assert!(
            init.status.success(),
            "{label}: init over sftp failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        assert!(
            repo_dir.join("config").is_file(),
            "{label}: no repository at the absolute path with a space"
        );
        let cat = restic(&bin, &repo, &options, &password_file, &["cat", "config"]);
        assert!(
            cat.status.success(),
            "{label}: cat config failed: {}",
            String::from_utf8_lossy(&cat.stderr)
        );

        fs::write(&paths.kestic, "").expect("empty known_hosts");
        let refused = restic(&bin, &repo, &options, &password_file, &["cat", "config"]);
        assert!(
            !refused.status.success(),
            "{label}: unknown server accepted"
        );
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains("Host key verification failed"),
            "{label}: ssh did not refuse the host key: {}",
            String::from_utf8_lossy(&refused.stderr)
        );
        fs::write(&paths.kestic, format!("{}\n", server.known_hosts_line()))
            .expect("restore known_hosts");
    }
    set_test_ssh_config(None);
}
