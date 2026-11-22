// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Local S3 test server: `rclone serve s3` with one access key pair on a random loopback port,
//! serving a temp directory whose subdirectories are the buckets.

use crate::random::{hex, random_bytes};
use crate::tools::{dev_tools_bin_dir, free_loopback_port};
use std::env;
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Resolves the `rclone` binary installed by `scripts/bootstrap.sh` (`KESTIC_RCLONE` overrides).
pub fn rclone_binary() -> PathBuf {
    let bin = env::var_os("KESTIC_RCLONE")
        .filter(|v| !v.is_empty())
        .map_or_else(|| dev_tools_bin_dir().join("rclone"), PathBuf::from);
    assert!(
        bin.is_file(),
        "rclone missing at {} (run scripts/bootstrap.sh)",
        bin.display()
    );
    bin
}

/// Running `rclone serve s3`; stopped when dropped.
pub struct S3ServerGuard {
    /// `http://127.0.0.1:<port>`.
    pub endpoint: String,
    pub port: u16,
    pub access_key: String,
    pub secret_key: String,
    data_dir: PathBuf,
    child: Option<Child>,
}

impl S3ServerGuard {
    /// Starts the server on a free port with generated keys; `bucket` exists and is empty.
    pub fn start(data_dir: &Path, bucket: &str) -> Self {
        fs::create_dir_all(data_dir.join(bucket)).expect("mkdir bucket");
        let access_key = format!("KESTIC{}", hex(&random_bytes(8)).to_uppercase());
        let secret_key = hex(&random_bytes(20));
        Self::start_on_port(data_dir, free_loopback_port(), &access_key, &secret_key)
    }

    fn start_on_port(data_dir: &Path, port: u16, access_key: &str, secret_key: &str) -> Self {
        let addr = format!("127.0.0.1:{port}");
        // The key pair goes through the environment, not the command line; the quotes keep it
        // one value of rclone's comma-separated list.
        let child = Command::new(rclone_binary())
            .args(["serve", "s3", "--addr", &addr])
            .arg(data_dir)
            .env("RCLONE_CONFIG", "/dev/null")
            .env("RCLONE_AUTH_KEY", format!("\"{access_key},{secret_key}\""))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rclone serve s3");
        let mut guard = Self {
            endpoint: format!("http://{addr}"),
            port,
            access_key: access_key.to_string(),
            secret_key: secret_key.to_string(),
            data_dir: data_dir.to_path_buf(),
            child: Some(child),
        };
        guard.wait_ready();
        guard
    }

    fn wait_ready(&mut self) {
        let addr = format!("127.0.0.1:{}", self.port);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(ref mut child) = self.child
                && let Some(status) = child.try_wait().expect("try_wait rclone")
            {
                panic!("rclone serve s3 exited prematurely: {status}");
            }
            if TcpStream::connect_timeout(
                &addr.parse().expect("socket addr"),
                Duration::from_millis(100),
            )
            .is_ok()
            {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("rclone serve s3 did not start listening on {addr} in time");
    }

    /// Directory behind bucket `bucket`.
    pub fn bucket_dir(&self, bucket: &str) -> PathBuf {
        self.data_dir.join(bucket)
    }

    /// Starts the server again on the same port, directory and keys.
    pub fn restart(&self) -> Self {
        Self::start_on_port(
            &self.data_dir,
            self.port,
            &self.access_key,
            &self.secret_key,
        )
    }

    /// Stops the server immediately; the data stays.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for S3ServerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}
