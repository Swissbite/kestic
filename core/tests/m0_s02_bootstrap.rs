// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

mod common;

use common::tools::{dev_tools_bin_dir, short_tempdir};
use common::workspace::workspace_root;

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn id(&self) -> u32 {
        self.child.as_ref().expect("child alive").id()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn allocate_random_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

fn wait_for_tcp(port: u16, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for TCP port {port}");
}

fn isolated_env(cmd: &mut Command, root: &Path) {
    let xdg_config = root.join("xdg/config");
    let xdg_data = root.join("xdg/data");
    let xdg_cache = root.join("xdg/cache");
    let xdg_state = root.join("xdg/state");
    let xdg_runtime = root.join("xdg/runtime");
    let home = root.join("home");
    for d in [
        &xdg_config,
        &xdg_data,
        &xdg_cache,
        &xdg_state,
        &xdg_runtime,
        &home,
    ] {
        fs::create_dir_all(d).expect("create isolated XDG dir");
    }
    fs::set_permissions(&xdg_runtime, fs::Permissions::from_mode(0o700))
        .expect("chmod 700 runtime dir");

    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", xdg_config)
        .env("XDG_DATA_HOME", xdg_data)
        .env("XDG_CACHE_HOME", xdg_cache)
        .env("XDG_STATE_HOME", xdg_state)
        .env("XDG_RUNTIME_DIR", xdg_runtime);
}

fn go_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => panic!("unsupported arch in test: {other}"),
    }
}

/// AC-M0-02-2: Second run of scripts/bootstrap.sh changes/downloads nothing and exits 0.
#[test]
fn ac_m0_02_2_bootstrap_is_idempotent() {
    let tools = dev_tools_bin_dir();
    let tracked_files = [
        tools.join("restic-0.18.0"),
        tools.join("restic-0.19.1"),
        tools.join("rest-server"),
        tools.join("rclone"),
    ];
    for path in &tracked_files {
        assert!(
            path.exists(),
            "expected bootstrap binary to exist before idempotency test: {}",
            path.display()
        );
    }

    let before_mtimes: Vec<_> = tracked_files
        .iter()
        .map(|p| {
            fs::metadata(p)
                .and_then(|m| m.modified())
                .expect("read mtime before")
        })
        .collect();

    let output = Command::new(workspace_root().join("scripts/bootstrap.sh"))
        .env("KESTIC_BOOTSTRAP_SKIP_APT", "1")
        .output()
        .expect("execute scripts/bootstrap.sh");

    assert!(
        output.status.success(),
        "second bootstrap.sh run failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Downloading"),
        "AC-M0-02-2 expected no downloads on second run, got stdout:\n{stdout}"
    );

    for (path, before) in tracked_files.iter().zip(before_mtimes.iter()) {
        let after = fs::metadata(path)
            .and_then(|m| m.modified())
            .expect("read mtime after");
        assert_eq!(
            *before,
            after,
            "AC-M0-02-2 expected {} not to be modified on second run",
            path.display()
        );
    }
}

/// AC-M0-02-3: Corrupted binary with wrong SHA256 aborts bootstrap.sh with Exit != 0
/// and installs nothing.
#[test]
fn ac_m0_02_3_bootstrap_aborts_on_checksum_mismatch() {
    let temp = short_tempdir("kestic-ac-m0-02-3");
    let gnupg_dir = temp.path().join("gnupg-gen");
    fs::create_dir_all(&gnupg_dir).expect("create gnupg dir");
    fs::set_permissions(&gnupg_dir, fs::Permissions::from_mode(0o700)).expect("chmod 700 gnupg");

    // Generate ephemeral test GPG key without passphrase
    let key_params = temp.path().join("gpg-batch");
    fs::write(
        &key_params,
        "%no-protection\n\
         Key-Type: eddsa\n\
         Key-Curve: ed25519\n\
         Name-Real: Kestic Test\n\
         Name-Email: test@example.invalid\n\
         Expire-Date: 0\n\
         %commit\n",
    )
    .expect("write gpg batch params");

    let gen_out = Command::new("gpg")
        .env("GNUPGHOME", &gnupg_dir)
        .args(["--batch", "--gen-key"])
        .arg(&key_params)
        .output()
        .expect("generate test gpg key");
    assert!(
        gen_out.status.success(),
        "gpg --gen-key failed: {}",
        String::from_utf8_lossy(&gen_out.stderr)
    );

    let fpr_out = Command::new("gpg")
        .env("GNUPGHOME", &gnupg_dir)
        .args(["--batch", "--with-colons", "--list-keys"])
        .output()
        .expect("list gpg keys");
    let fpr_stdout = String::from_utf8_lossy(&fpr_out.stdout);
    let fpr = fpr_stdout
        .lines()
        .find_map(|line| {
            let parts: Vec<_> = line.split(':').collect();
            if parts.first() == Some(&"fpr") {
                parts.get(9).map(|s| (*s).to_string())
            } else {
                None
            }
        })
        .expect("extract test gpg fingerprint");

    let pubkey_file = temp.path().join("pubkey.asc");
    let export_out = Command::new("gpg")
        .env("GNUPGHOME", &gnupg_dir)
        .args(["--batch", "--armor", "--export", &fpr])
        .output()
        .expect("export test gpg key");
    assert!(export_out.status.success());
    fs::write(&pubkey_file, &export_out.stdout).expect("write pubkey.asc");

    let archive_name = format!("restic_0.18.0_linux_{}.bz2", go_arch());
    let sums_content = format!(
        "0000000000000000000000000000000000000000000000000000000000000000  {archive_name}\n"
    );
    let sums_file = temp.path().join("SHA256SUMS");
    let asc_file = temp.path().join("SHA256SUMS.asc");
    fs::write(&sums_file, sums_content.as_bytes()).expect("write SHA256SUMS");

    let sign_out = Command::new("gpg")
        .env("GNUPGHOME", &gnupg_dir)
        .args([
            "--batch",
            "--yes",
            "--armor",
            "--detach-sign",
            "--local-user",
            &fpr,
            "--output",
        ])
        .arg(&asc_file)
        .arg(&sums_file)
        .output()
        .expect("sign SHA256SUMS");
    assert!(
        sign_out.status.success(),
        "gpg detach-sign failed: {}",
        String::from_utf8_lossy(&sign_out.stderr)
    );
    let asc_bytes = fs::read(&asc_file).expect("read SHA256SUMS.asc");

    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    routes.insert(
        format!("/{archive_name}"),
        b"tampered-binary-bytes".to_vec(),
    );
    routes.insert("/SHA256SUMS".to_string(), sums_content.into_bytes());
    routes.insert("/SHA256SUMS.asc".to_string(), asc_bytes);
    let routes = Arc::new(routes);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock http");
    let port = listener.local_addr().expect("mock addr").port();
    listener
        .set_nonblocking(true)
        .expect("set mock nonblocking");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_srv = Arc::clone(&stop);
    let routes_srv = Arc::clone(&routes);

    let srv_handle = thread::spawn(move || {
        while !stop_srv.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 2048];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let path = req
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("/");
                    if let Some(body) = routes_srv.get(path) {
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(body);
                    } else {
                        let _ = stream
                            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });

    let install_target = temp.path().join("tools");
    let mut cmd = Command::new(workspace_root().join("scripts/bootstrap.sh"));
    isolated_env(&mut cmd, temp.path());
    cmd.env("KESTIC_BOOTSTRAP_SKIP_APT", "1")
        .env("KESTIC_BOOTSTRAP_COMPONENTS", "restic-0.18.0")
        .env("KESTIC_TOOLS_DIR", &install_target)
        .env("KESTIC_TEST_MOCK_URLS", "1")
        .env("KESTIC_RESTIC_BASE_URL", format!("http://127.0.0.1:{port}"))
        .env("KESTIC_GPG_PUBKEY_FILE", &pubkey_file)
        .env("KESTIC_GPG_TRUSTED_FPR", &fpr);

    let result = cmd.output().expect("run bootstrap.sh against mock");
    stop.store(true, Ordering::Relaxed);
    let _ = srv_handle.join();

    assert!(
        !result.status.success(),
        "AC-M0-02-3 expected bootstrap.sh to fail on checksum mismatch; stdout={}, stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        !install_target.join("restic-0.18.0").exists(),
        "AC-M0-02-3 expected nothing installed on checksum mismatch"
    );
}

fn verify_s3_server_unprivileged(rclone: &Path, restic_versions: &[PathBuf]) {
    let s3_temp = short_tempdir("kestic-ac-m0-02-4-s3");
    let s3_root = s3_temp.path().join("s3-root");
    fs::create_dir_all(s3_root.join("testbucket")).expect("create s3 bucket dir");
    let s3_port = allocate_random_port();

    let mut rclone_cmd = Command::new(rclone);
    isolated_env(&mut rclone_cmd, s3_temp.path());
    rclone_cmd
        .args(["serve", "s3"])
        .arg(&s3_root)
        .args([
            "--addr",
            &format!("127.0.0.1:{s3_port}"),
            "--auth-key",
            "kesticaccess,kesticsecret",
            "--config",
            "/dev/null",
            "--no-console",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let rclone_guard = ChildGuard::new(rclone_cmd.spawn().expect("spawn rclone serve s3"));
    let rclone_pid = rclone_guard.id();
    wait_for_tcp(s3_port, Duration::from_secs(5));

    for (idx, restic_bin) in restic_versions.iter().enumerate() {
        let bucket_subdir = format!("repo-{idx}");
        fs::create_dir_all(s3_root.join("testbucket").join(&bucket_subdir))
            .expect("create repo prefix dir");
        let mut restic_cmd = Command::new(restic_bin);
        isolated_env(&mut restic_cmd, s3_temp.path());
        let out = restic_cmd
            .env("AWS_ACCESS_KEY_ID", "kesticaccess")
            .env("AWS_SECRET_ACCESS_KEY", "kesticsecret")
            .env("RESTIC_PASSWORD", "test-pass")
            .args([
                "-r",
                &format!("s3:http://127.0.0.1:{s3_port}/testbucket/{bucket_subdir}"),
                "init",
            ])
            .output()
            .expect("run restic init against rclone serve s3");
        assert!(
            out.status.success(),
            "restic ({}) init against rclone serve s3 failed: {}",
            restic_bin.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    drop(rclone_guard);
    assert!(
        !Path::new(&format!("/proc/{rclone_pid}")).exists(),
        "rclone serve s3 process {rclone_pid} must terminate after test"
    );
}

fn verify_sftp_server_unprivileged(restic_versions: &[PathBuf]) {
    let sftp_temp = short_tempdir("kestic-ac-m0-02-4-sftp");
    let host_key = sftp_temp.path().join("ssh_host_ed25519_key");
    let client_key = sftp_temp.path().join("client_ed25519_key");
    for key in [&host_key, &client_key] {
        let status = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(key)
            .status()
            .expect("run ssh-keygen");
        assert!(status.success(), "ssh-keygen failed");
    }

    let client_pub = fs::read_to_string(sftp_temp.path().join("client_ed25519_key.pub"))
        .expect("read client pubkey");
    let auth_keys = sftp_temp.path().join("authorized_keys");
    fs::write(&auth_keys, client_pub).expect("write authorized_keys");
    fs::set_permissions(&auth_keys, fs::Permissions::from_mode(0o600)).expect("chmod 600 auth");

    let host_pub = fs::read_to_string(sftp_temp.path().join("ssh_host_ed25519_key.pub"))
        .expect("read host pubkey");
    let sftp_port = allocate_random_port();
    let known_hosts = sftp_temp.path().join("known_hosts");
    fs::write(&known_hosts, format!("[127.0.0.1]:{sftp_port} {host_pub}"))
        .expect("write known_hosts");

    let current_user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .expect("USER set");
    let sshd_config = sftp_temp.path().join("sshd_config");
    let pid_file = sftp_temp.path().join("sshd.pid");
    fs::write(
        &sshd_config,
        format!(
            "Port {sftp_port}\n\
             ListenAddress 127.0.0.1\n\
             HostKey {}\n\
             AuthorizedKeysFile {}\n\
             PidFile {}\n\
             Subsystem sftp internal-sftp\n\
             PasswordAuthentication no\n\
             KbdInteractiveAuthentication no\n\
             PubkeyAuthentication yes\n\
             UsePAM no\n\
             StrictModes no\n\
             AllowUsers {current_user}\n",
            host_key.display(),
            auth_keys.display(),
            pid_file.display()
        ),
    )
    .expect("write sshd_config");

    let ssh_client_cfg = sftp_temp.path().join("ssh_client_config");
    fs::write(
        &ssh_client_cfg,
        format!(
            "Host 127.0.0.1\n\
             User {current_user}\n\
             Port {sftp_port}\n\
             IdentityFile {}\n\
             IdentitiesOnly yes\n\
             StrictHostKeyChecking yes\n\
             BatchMode yes\n",
            client_key.display()
        ),
    )
    .expect("write ssh_client_config");

    let sshd_guard = ChildGuard::new(
        Command::new("/usr/sbin/sshd")
            .args(["-D", "-e", "-f"])
            .arg(&sshd_config)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn unprivileged sshd"),
    );
    let sshd_pid = sshd_guard.id();
    wait_for_tcp(sftp_port, Duration::from_secs(5));

    let sftp_args = format!(
        "-p {sftp_port} -F {} -o UserKnownHostsFile={} -o IdentitiesOnly=yes",
        ssh_client_cfg.display(),
        known_hosts.display()
    );

    for (idx, restic_bin) in restic_versions.iter().enumerate() {
        let repo_dir = sftp_temp.path().join(format!("sftp-repo-{idx}"));
        fs::create_dir_all(&repo_dir).expect("create sftp repo dir");
        let mut restic_cmd = Command::new(restic_bin);
        isolated_env(&mut restic_cmd, sftp_temp.path());
        let out = restic_cmd
            .env("RESTIC_PASSWORD", "test-pass")
            .args([
                "-r",
                &format!("sftp:{current_user}@127.0.0.1:{}", repo_dir.display()),
                "-o",
                &format!("sftp.args={sftp_args}"),
                "init",
            ])
            .output()
            .expect("run restic init against unprivileged sshd");
        assert!(
            out.status.success(),
            "restic ({}) init against sshd failed: {}",
            restic_bin.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    drop(sshd_guard);
    assert!(
        !Path::new(&format!("/proc/{sshd_pid}")).exists(),
        "sshd process {sshd_pid} must terminate after test"
    );
}

/// AC-M0-02-4: `rclone serve s3` and `sshd` run as normal user on random port
/// with temporary keys/config, work with restic 0.18.0 and current, and terminate on drop.
#[test]
fn ac_m0_02_4_s3_and_sftp_test_servers_run_unprivileged_and_stop() {
    let tools = dev_tools_bin_dir();
    let restic_versions = [tools.join("restic-0.18.0"), tools.join("restic-0.19.1")];
    let rclone = tools.join("rclone");
    assert!(rclone.exists(), "rclone missing in {}", tools.display());

    verify_s3_server_unprivileged(&rclone, &restic_versions);
    verify_sftp_server_unprivileged(&restic_versions);
}

/// AC-M0-02-5: Pre-0.17 snapshot fixture exists without `summary` field,
/// parses cleanly, and bootstrap.sh does not download restic 0.16.4.
#[test]
fn ac_m0_02_5_fixture_present_and_no_0_16_4_binary_loaded() {
    let fixture = workspace_root().join("core/tests/fixtures/snapshot_pre_0_17_no_summary.json");
    assert!(fixture.exists(), "snapshot fixture must exist");
    let raw = fs::read_to_string(&fixture).expect("read snapshot fixture");
    assert!(
        !raw.contains("\"summary\""),
        "pre-0.17 fixture must not have a summary field"
    );
    let parsed: Vec<kestic_core::snapshot::ResticSnapshot> =
        serde_json::from_str(&raw).expect("parse fixture");
    assert_eq!(parsed.len(), 1);
    assert!(parsed[0].summary.is_none());

    let bootstrap_src =
        fs::read_to_string(workspace_root().join("scripts/bootstrap.sh")).expect("read bootstrap");
    assert!(
        !bootstrap_src.contains("0.16.4"),
        "bootstrap.sh must not download restic 0.16.4"
    );
}
