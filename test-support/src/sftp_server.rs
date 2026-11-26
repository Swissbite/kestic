// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Local SFTP test server: an `sshd` of the current user on a random loopback port with its own
//! host key and one authorised user key, plus an ssh configuration for the tests that offers only
//! that key and never reads the real `~/.ssh`.

use crate::tools::free_loopback_port;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// The `sshd` binary (`KESTIC_SSHD` overrides `/usr/sbin/sshd`); sshd needs an absolute path.
fn sshd_binary() -> PathBuf {
    let bin = env::var_os("KESTIC_SSHD")
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from("/usr/sbin/sshd"), PathBuf::from);
    assert!(
        bin.is_file(),
        "sshd missing at {} (install openssh-server)",
        bin.display()
    );
    bin
}

/// Generates an ed25519 key pair at `path` (`path.pub`) without passphrase.
pub fn generate_key(path: &Path) {
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "kestic-test", "-f"])
        .arg(path)
        .stdin(Stdio::null())
        .status()
        .expect("run ssh-keygen");
    assert!(status.success(), "ssh-keygen failed for {}", path.display());
}

/// `<type> <key>` of the public key next to `private_key`.
pub fn public_key(private_key: &Path) -> String {
    let text = fs::read_to_string(private_key.with_extension("pub")).expect("read public key");
    text.split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

fn current_user() -> String {
    let out = Command::new("id").arg("-un").output().expect("run id -un");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Running `sshd`; stopped when dropped.
pub struct SftpServerGuard {
    pub port: u16,
    /// The only user the server lets in: the one running the tests.
    pub user: String,
    /// ssh configuration for Kestic's ssh calls (`KESTIC_TEST_SSH_CONFIG`): the authorised key.
    pub ssh_config: PathBuf,
    /// The same with a key the server does not know.
    pub ssh_config_unknown_key: PathBuf,
    /// `<type> <key>` of the server's host key.
    pub host_key: String,
    dir: PathBuf,
    child: Option<Child>,
}

impl SftpServerGuard {
    /// Starts the server on a free port; `dir` holds its keys, configuration and log.
    pub fn start(dir: &Path) -> Self {
        fs::create_dir_all(dir).expect("mkdir sftp server dir");
        let host_key = dir.join("host_key");
        let user_key = dir.join("user_key");
        let stranger_key = dir.join("stranger_key");
        for key in [&host_key, &user_key, &stranger_key] {
            generate_key(key);
        }
        fs::write(
            dir.join("authorized_keys"),
            format!("{}\n", public_key(&user_key)),
        )
        .expect("write authorized_keys");
        let ssh_config = dir.join("ssh_config");
        let ssh_config_unknown_key = dir.join("ssh_config_unknown_key");
        for (config, key) in [
            (&ssh_config, &user_key),
            (&ssh_config_unknown_key, &stranger_key),
        ] {
            fs::write(
                config,
                format!(
                    "Host *\n  IdentityFile \"{}\"\n  IdentitiesOnly yes\n  \
                     PasswordAuthentication no\n  KbdInteractiveAuthentication no\n",
                    key.display()
                ),
            )
            .expect("write ssh_config");
        }
        let mut guard = Self {
            port: free_loopback_port(),
            user: current_user(),
            ssh_config,
            ssh_config_unknown_key,
            host_key: public_key(&host_key),
            dir: dir.to_path_buf(),
            child: None,
        };
        guard.spawn();
        guard
    }

    /// Starts `sshd`. A test makes many connections from one address in a short time, some with
    /// a refused key on purpose; `PerSourcePenalties` (OpenSSH 9.8 and later) would then block
    /// the address for a while. An older sshd refuses the option, so it is set only where the
    /// configuration test accepts it.
    fn spawn(&mut self) {
        let config = self.dir.join("sshd_config");
        let base = format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey \"{dir}/host_key\"\n\
             AuthorizedKeysFile \"{dir}/authorized_keys\"\nPidFile \"{dir}/sshd.pid\"\n\
             UsePAM no\nStrictModes no\nPasswordAuthentication no\n\
             KbdInteractiveAuthentication no\nSubsystem sftp internal-sftp\n",
            port = self.port,
            dir = self.dir.display()
        );
        let with_penalties = format!("{base}PerSourcePenalties no\n");
        fs::write(&config, &with_penalties).expect("write sshd_config");
        let accepted = Command::new(sshd_binary())
            .args(["-t", "-f"])
            .arg(&config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !accepted {
            fs::write(&config, &base).expect("write sshd_config");
        }
        let log = fs::File::create(self.dir.join("sshd.log")).expect("create sshd log");
        let child = Command::new(sshd_binary())
            .args(["-D", "-e", "-f"])
            .arg(&config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("spawn sshd");
        self.child = Some(child);
        self.wait_ready();
    }

    fn wait_ready(&mut self) {
        let log = self.dir.join("sshd.log");
        crate::tools::wait_listening(
            self.child.as_mut(),
            self.port,
            Duration::from_secs(10),
            "sshd",
            || fs::read_to_string(&log).unwrap_or_default(),
        );
    }

    /// The name of the server in `known_hosts`: `[127.0.0.1]:<port>`.
    pub fn known_hosts_name(&self) -> String {
        format!("[127.0.0.1]:{}", self.port)
    }

    /// The `known_hosts` line of the server's host key.
    pub fn known_hosts_line(&self) -> String {
        format!("{} {}", self.known_hosts_name(), self.host_key)
    }

    /// A `known_hosts` line for this server with another key of the same type.
    pub fn other_key_line(&self) -> String {
        let other = self.dir.join("other_host_key");
        if !other.is_file() {
            generate_key(&other);
        }
        format!("{} {}", self.known_hosts_name(), public_key(&other))
    }

    /// Stops the server immediately.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for SftpServerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}
