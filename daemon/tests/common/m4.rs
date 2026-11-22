// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared helpers for the M4 restore tests: snapshots written directly with restic.

use std::fs;
use std::path::Path;
use std::process::Command;

/// A snapshot to write with restic: tag, time (restic format «2026-11-15 12:00:00») and host.
pub struct RawSnapshot<'a> {
    pub tag: &'a str,
    pub time: &'a str,
    pub host: &'a str,
}

/// Writes `snap` into the local repository `repo` with `restic`; the source below `work` gets a
/// file whose content differs per snapshot, so each snapshot adds new data.
pub fn write_snapshot(
    restic: &Path,
    repo: &Path,
    password: &str,
    work: &Path,
    snap: &RawSnapshot<'_>,
) {
    let src = work.join(format!("raw-{}", snap.tag.replace(':', "-")));
    fs::create_dir_all(&src).expect("mkdir raw source");
    fs::write(
        src.join("data.txt"),
        format!("{} {} {}", snap.tag, snap.time, snap.host).repeat(200),
    )
    .expect("write raw source");
    let out = Command::new(restic)
        .arg("-r")
        .arg(repo)
        .args(["backup", "--no-cache", "--json", "--tag", snap.tag])
        .args(["--time", snap.time, "--host", snap.host])
        .arg(&src)
        .env("RESTIC_PASSWORD", password)
        .output()
        .expect("run restic backup");
    assert!(
        out.status.success(),
        "restic backup {}: {}",
        snap.tag,
        String::from_utf8_lossy(&out.stderr)
    );
}
