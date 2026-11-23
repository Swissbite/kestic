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

/// Backs up the folder `src` into `repo` as one snapshot with the tag, time and host of
/// `snap`; returns the full snapshot ID.
pub fn backup_dir(
    restic: &Path,
    repo: &Path,
    password: &str,
    src: &Path,
    snap: &RawSnapshot<'_>,
) -> String {
    let out = Command::new(restic)
        .arg("-r")
        .arg(repo)
        .args(["backup", "--no-cache", "--json", "--tag", snap.tag])
        .args(["--time", snap.time, "--host", snap.host])
        .arg(src)
        .env("RESTIC_PASSWORD", password)
        .output()
        .expect("run restic backup");
    assert!(
        out.status.success(),
        "restic backup {}: {}",
        src.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["message_type"] == "summary")
        .and_then(|v| v["snapshot_id"].as_str().map(str::to_string))
        .expect("snapshot_id in the backup summary")
}

/// The nodes `restic ls --json <snapshot_id> <dir>` prints for `dir`, without the first node
/// (the folder itself).
pub fn restic_ls_nodes(
    restic: &Path,
    repo: &Path,
    password: &str,
    snapshot_id: &str,
    dir: &str,
) -> Vec<serde_json::Value> {
    let out = Command::new(restic)
        .arg("-r")
        .arg(repo)
        .args(["ls", "--no-cache", "--json", snapshot_id, dir])
        .env("RESTIC_PASSWORD", password)
        .output()
        .expect("run restic ls");
    assert!(out.status.success(), "restic ls {dir}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["message_type"] == "node" && v["path"] != dir)
        .collect()
}

/// A folder `src` below `work` with files, sub-folders and a nested folder:
/// `a.txt` (5 bytes), `b.bin` (1000 bytes), `docs/x.txt`, `docs/deep/y.txt` and `empty/`.
pub fn small_tree(work: &Path) -> std::path::PathBuf {
    let src = work.join("src");
    fs::create_dir_all(src.join("docs/deep")).expect("mkdir docs/deep");
    fs::create_dir_all(src.join("empty")).expect("mkdir empty");
    fs::write(src.join("a.txt"), "hello").expect("write a.txt");
    fs::write(src.join("b.bin"), vec![7u8; 1000]).expect("write b.bin");
    fs::write(src.join("docs/x.txt"), "x").expect("write x.txt");
    fs::write(src.join("docs/deep/y.txt"), "yy").expect("write y.txt");
    src
}

/// Number of root folders of [`big_tree`] whose name contains the filter text «bericht».
pub const BIG_TREE_MATCHES: usize = 37;
/// Of these, the names written in another case («BeRiCHT»).
pub const BIG_TREE_OTHER_CASE: usize = 5;

/// A folder `big` below `work` with 1000 folders of 100 empty files each (100 000 files):
/// every 27th folder name contains «bericht» (37 in total, the first 5 as «BeRiCHT»).
pub fn big_tree(work: &Path) -> std::path::PathBuf {
    let root = work.join("big");
    for i in 0..1000usize {
        let name = match (i % 27, i / 27) {
            (0, k) if k < BIG_TREE_OTHER_CASE => format!("d{i:04}-BeRiCHT"),
            (0, k) if k < BIG_TREE_MATCHES => format!("d{i:04}-bericht"),
            _ => format!("d{i:04}"),
        };
        let dir = root.join(name);
        fs::create_dir_all(&dir).expect("mkdir big folder");
        for f in 0..100 {
            fs::write(dir.join(format!("f{f:03}")), b"").expect("write big file");
        }
    }
    root
}

/// Paths `restic find --json --snapshot <snapshot_id> <pattern>` reports, to check what a
/// snapshot holds independently of Kestic.
pub fn restic_find_paths(
    restic: &Path,
    repo: &Path,
    password: &str,
    snapshot_id: &str,
    pattern: &str,
) -> Vec<String> {
    let out = Command::new(restic)
        .arg("-r")
        .arg(repo)
        .args([
            "find",
            "--no-cache",
            "--json",
            "--snapshot",
            snapshot_id,
            pattern,
        ])
        .env("RESTIC_PASSWORD", password)
        .output()
        .expect("run restic find");
    assert!(out.status.success(), "restic find {pattern}");
    let groups: Vec<serde_json::Value> =
        serde_json::from_slice(&out.stdout).expect("restic find --json output");
    groups
        .iter()
        .flat_map(|g| g["matches"].as_array().cloned().unwrap_or_default())
        .filter_map(|m| m["path"].as_str().map(str::to_string))
        .collect()
}

/// Writes `name` (with `content`) into the folder `dir`, creating it.
pub fn write_file(dir: &Path, name: &str, content: &str) {
    fs::create_dir_all(dir).expect("mkdir");
    fs::write(dir.join(name), content).expect("write file");
}

/// Backs up `content` as the file `path` (absolute, without the leading `/`) with
/// `restic backup --stdin --stdin-filename`, so the snapshot holds a path that does not exist
/// here (e.g. `home/alt/Dokumente/a.txt`); returns the full snapshot ID. Folders in the file
/// name need restic 0.19 or newer.
pub fn backup_stdin(
    restic: &Path,
    repo: &Path,
    password: &str,
    path: &str,
    content: &str,
    snap: &RawSnapshot<'_>,
) -> String {
    use std::io::Write;
    let mut child = Command::new(restic)
        .arg("-r")
        .arg(repo)
        .args(["backup", "--no-cache", "--json", "--tag", snap.tag])
        .args(["--time", snap.time, "--host", snap.host])
        .args(["--stdin", "--stdin-filename", path])
        .env("RESTIC_PASSWORD", password)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run restic backup --stdin");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(content.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("restic backup --stdin");
    assert!(
        out.status.success(),
        "restic backup --stdin {path}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["message_type"] == "summary")
        .and_then(|v| v["snapshot_id"].as_str().map(str::to_string))
        .expect("snapshot_id in the backup summary")
}

/// The `username` restic recorded in snapshot `snapshot_id` (the user running the tests).
pub fn snapshot_username(restic: &Path, repo: &Path, password: &str, snapshot_id: &str) -> String {
    let out = Command::new(restic)
        .arg("-r")
        .arg(repo)
        .args(["snapshots", "--no-cache", "--json", snapshot_id])
        .env("RESTIC_PASSWORD", password)
        .output()
        .expect("run restic snapshots");
    assert!(out.status.success(), "restic snapshots {snapshot_id}");
    let list: serde_json::Value = serde_json::from_slice(&out.stdout).expect("snapshots JSON");
    list[0]["username"]
        .as_str()
        .expect("username in the snapshot")
        .to_string()
}
