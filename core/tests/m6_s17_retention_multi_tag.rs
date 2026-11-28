// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S17: the retention of a backup never removes a snapshot that carries the `kestic:` tags of
//! more than one backup. Real restic (0.18.0 and latest) with temp repositories; snapshots with
//! two backup tags are written by hand, Kestic never writes them.

mod common;

use kestic_core::backup::{
    RetentionRun, execute_forget_dry_run_with, execute_retention, list_snapshots_for_backup,
};
use kestic_core::model::{
    Backup, Destination, DestinationKind, KeepCount, RetentionPolicy, Source,
};
use kestic_core::provider::ResolvedRestic;
use kestic_core::restic::{CancelHandle, SECRET_KIND_REPO};
use kestic_core::secrets::{SecretStore, SecretValue};
use kestic_test_support::password::strong_password;
use kestic_test_support::tools::{assert_has_flag_pair, install_args_spy, write_executable};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use time::macros::datetime;

/// A temp repository with two configured backups `b1` (keep the last snapshot) and `b2`.
struct Scenario {
    dir: TempDir,
    rig: common::ResticRig,
    dest: Destination,
    password: String,
    repo_dir: PathBuf,
    src_dir: PathBuf,
    b1: Backup,
    b2: Backup,
    label: &'static str,
}

impl Scenario {
    fn new(label: &'static str, restic_bin: &Path) -> Self {
        let dir = TempDir::new().expect("tempdir");
        let repo_dir = dir.path().join("repo");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("mkdir src");
        fs::write(src_dir.join("data.txt"), b"multi tag retention").expect("write data");
        let rig =
            common::ResticRig::new(dir.path(), restic_bin, datetime!(2026-11-27 09:00:00 UTC));
        let (dest, password) = rig.local_repo("Multi Tag", &repo_dir);
        let mut b1 = Backup::new("b1", Source::folder(&src_dir), dest.id.clone()).expect("b1");
        b1.retention = RetentionPolicy {
            keep_last: Some(KeepCount::Count(1)),
            ..RetentionPolicy::empty()
        };
        let b2 = Backup::new("b2", Source::folder(&src_dir), dest.id.clone()).expect("b2");
        Self {
            dir,
            rig,
            dest,
            password,
            repo_dir,
            src_dir,
            b1,
            b2,
            label,
        }
    }

    fn restic(&self) -> Command {
        let mut cmd = Command::new(&self.rig.resolved.binary);
        cmd.arg("-r")
            .arg(&self.repo_dir)
            .env("RESTIC_PASSWORD", &self.password)
            .env("RESTIC_CACHE_DIR", &self.rig.cache_dir)
            .env("TZ", "UTC");
        cmd
    }

    /// Writes a snapshot at `time` (UTC) with `tags` and returns its full ID.
    fn snapshot(&self, tags: &[&str], time: &str) -> String {
        let mut cmd = self.restic();
        cmd.args(["backup", "--json", "--time", time]);
        for tag in tags {
            cmd.args(["--tag", tag]);
        }
        let out = cmd
            .arg(&self.src_dir)
            .output()
            .expect("spawn restic backup");
        assert!(
            out.status.success(),
            "{}: restic backup failed: {}",
            self.label,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find_map(|v| v["snapshot_id"].as_str().map(str::to_string))
            .expect("snapshot_id in backup summary")
    }

    /// Both backup tags (`kestic:b1`, `kestic:b2`) on one snapshot.
    fn both_tags(&self) -> [String; 2] {
        [self.b1.tag(), self.b2.tag()]
    }

    fn multi_tag_snapshot(&self, time: &str) -> String {
        let [t1, t2] = self.both_tags();
        self.snapshot(&[&t1, &t2], time)
    }

    /// IDs of all snapshots in the repository.
    fn existing_ids(&self) -> BTreeSet<String> {
        let out = self
            .restic()
            .args(["snapshots", "--json", "--no-lock"])
            .output()
            .expect("spawn restic snapshots");
        assert!(out.status.success(), "{}: restic snapshots", self.label);
        let list: Vec<serde_json::Value> =
            serde_json::from_slice(&out.stdout).expect("parse snapshots");
        list.iter()
            .filter_map(|s| s["id"].as_str().map(str::to_string))
            .collect()
    }

    /// The IDs plain restic would remove with b1's rules (no Kestic filtering).
    fn plain_restic_removes(&self) -> BTreeSet<String> {
        let out = self
            .restic()
            .args([
                "forget",
                "--json",
                "--dry-run",
                "--no-lock",
                "--group-by",
                "tags",
                "--tag",
                &self.b1.tag(),
                "--keep-last",
                "1",
            ])
            .output()
            .expect("spawn restic forget --dry-run");
        assert!(
            out.status.success(),
            "{}: plain forget --dry-run",
            self.label
        );
        let line = String::from_utf8_lossy(&out.stdout)
            .lines()
            .find(|l| l.starts_with('['))
            .expect("forget JSON line")
            .to_string();
        let groups: Vec<serde_json::Value> = serde_json::from_str(&line).expect("parse groups");
        groups
            .iter()
            .filter_map(|g| g["remove"].as_array())
            .flatten()
            .filter_map(|s| s["id"].as_str().map(str::to_string))
            .collect()
    }

    /// A restic wrapper that logs every call's arguments; returns its binary and log path.
    fn spy(&self) -> (ResolvedRestic, PathBuf) {
        let (script, log) =
            install_args_spy(self.dir.path(), "restic-spy", &self.rig.resolved.binary);
        // `version` is not logged by the spy.
        wait_until_executable(&script, "version");
        (
            ResolvedRestic {
                binary: script,
                version: self.rig.resolved.version.clone(),
            },
            log,
        )
    }

    fn run_retention(&self, resolved: &ResolvedRestic) -> RetentionRun {
        let runtime = self.rig.runtime_with(resolved);
        execute_retention(&self.b1, &self.dest, &runtime, None, &[("TZ", "UTC")])
            .unwrap_or_else(|e| panic!("{}: retention failed: {e}", self.label))
    }
}

/// Logged calls of the spy, one argument list per call.
fn spy_calls(log: &Path) -> Vec<Vec<String>> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|l| l.split_whitespace().map(str::to_string).collect())
        .collect()
}

/// The arguments after the subcommand `forget` of one call.
fn after_forget(call: &[String]) -> Vec<String> {
    let pos = call
        .iter()
        .position(|a| a == "forget")
        .unwrap_or_else(|| panic!("no forget in {call:?}"));
    call[pos + 1..].to_vec()
}

/// The positional snapshot IDs of a `forget <id…>` call.
fn forget_call_ids(call: &[String]) -> BTreeSet<String> {
    after_forget(call)
        .into_iter()
        .filter(|a| a.len() == 64 && a.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect()
}

/// AC-M6-17-1: two snapshots tagged by hand with `kestic:b1` and `kestic:b2` (plain restic would
/// remove the older one with `--keep-last 1`) and two snapshots of `b1` alone; the retention of
/// `b1` (a) keeps both snapshots with two tags, and (b) neither is on the remove list.
#[test]
fn ac_m6_17_1_multi_tag_snapshots_survive_retention() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let sc = Scenario::new(label, &restic_bin);
        let own_old = sc.snapshot(&[&sc.b1.tag()], "2026-11-08 12:00:00");
        let own_new = sc.snapshot(&[&sc.b1.tag()], "2026-11-09 12:00:00");
        let multi_old = sc.multi_tag_snapshot("2026-11-10 12:00:00");
        let multi_new = sc.multi_tag_snapshot("2026-11-11 12:00:00");
        assert!(
            sc.plain_restic_removes().contains(&multi_old),
            "{label}: scenario: plain restic would remove the older snapshot with two tags"
        );

        let run = sc.run_retention(&sc.rig.resolved);
        assert!(run.succeeded(), "{label}: retention succeeded: {run:?}");

        let existing = sc.existing_ids();
        assert!(
            existing.contains(&multi_old) && existing.contains(&multi_new),
            "{label}: (a) both snapshots with two backup tags remain: {existing:?}"
        );
        assert!(
            !run.forget_ids.contains(&multi_old) && !run.forget_ids.contains(&multi_new),
            "{label}: (b) no snapshot with two backup tags on the remove list: {:?}",
            run.forget_ids
        );
        assert_eq!(
            run.forget_ids,
            vec![own_old.clone()],
            "{label}: the retention of b1 still removes its own old snapshot"
        );
        assert!(
            !existing.contains(&own_old) && existing.contains(&own_new),
            "{label}: b1's own old snapshot is gone, its newest stays: {existing:?}"
        );
    }
}

/// AC-M6-17-2: the call log of a retention run: (a) first `forget --dry-run --json` with the
/// rules, `--tag kestic:<id>` and `--group-by tags`; (b) the snapshots with more than one
/// `kestic:` tag are struck from the remove list; (c) exactly one `restic forget <id…>` with the
/// remaining IDs; (d) with an empty list no `forget` runs.
#[test]
fn ac_m6_17_2_plan_then_one_forget_with_ids() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let sc = Scenario::new(label, &restic_bin);
        let own_old = sc.snapshot(&[&sc.b1.tag()], "2026-11-07 12:00:00");
        let own_mid = sc.snapshot(&[&sc.b1.tag()], "2026-11-08 12:00:00");
        sc.snapshot(&[&sc.b1.tag()], "2026-11-09 12:00:00");
        let multi_old = sc.multi_tag_snapshot("2026-11-10 12:00:00");
        let multi_new = sc.multi_tag_snapshot("2026-11-11 12:00:00");
        let (spy, log) = sc.spy();

        sc.run_retention(&spy);
        let calls = spy_calls(&log);
        assert!(!calls.is_empty(), "{label}: restic was called");

        let plan = &calls[0];
        let plan_args = after_forget(plan);
        assert!(
            plan_args.iter().any(|a| a == "--dry-run") && plan.iter().any(|a| a == "--json"),
            "{label}: (a) the first call is `forget --dry-run --json`: {plan:?}"
        );
        assert_has_flag_pair(&plan_args, "--keep-last", "1");
        assert_has_flag_pair(&plan_args, "--tag", &sc.b1.tag());
        assert_has_flag_pair(&plan_args, "--group-by", "tags");

        let forgets: Vec<&Vec<String>> = calls[1..]
            .iter()
            .filter(|c| c.iter().any(|a| a == "forget"))
            .collect();
        assert_eq!(
            forgets.len(),
            1,
            "{label}: (c) exactly one `forget <id…>` after the plan: {calls:?}"
        );
        let ids = forget_call_ids(forgets[0]);
        assert!(
            !ids.contains(&multi_old) && !ids.contains(&multi_new),
            "{label}: (b) snapshots with two backup tags are struck: {ids:?}"
        );
        assert_eq!(
            ids,
            BTreeSet::from([own_old, own_mid]),
            "{label}: (c) `forget` gets exactly the remaining IDs"
        );
        let extra: Vec<String> = after_forget(forgets[0])
            .into_iter()
            .filter(|a| !ids.contains(a))
            .collect();
        assert!(
            extra.iter().all(|a| !a.starts_with("--keep")
                && a != "--tag"
                && a != "--group-by"
                && a != "--dry-run"),
            "{label}: (c) `forget <id…>` carries no rules and no filter: {extra:?}"
        );

        // Only snapshots with two tags would be removed: the list is empty after striking them.
        let sc = Scenario::new(label, &restic_bin);
        sc.snapshot(&[&sc.b1.tag()], "2026-11-09 12:00:00");
        let multi_old = sc.multi_tag_snapshot("2026-11-10 12:00:00");
        sc.multi_tag_snapshot("2026-11-11 12:00:00");
        assert_eq!(
            sc.plain_restic_removes(),
            BTreeSet::from([multi_old]),
            "{label}: scenario: plain restic would remove only a snapshot with two tags"
        );
        let (spy, log) = sc.spy();
        let run = sc.run_retention(&spy);
        let calls = spy_calls(&log);
        assert_eq!(
            calls.len(),
            1,
            "{label}: (d) with an empty remove list only the plan runs: {calls:?}"
        );
        assert!(
            run.forget.is_none() && run.forget_ids.is_empty(),
            "{label}: (d) no `forget` recorded: {run:?}"
        );
        assert!(run.succeeded(), "{label}: (d) the retention is done");
    }
}

/// AC-M6-17-3: (a) the retention preview shows snapshots with more than one backup tag as
/// «keep», and (b) its remove list is the one the retention run passes to `forget`.
#[test]
fn ac_m6_17_3_preview_and_run_share_the_remove_list() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let sc = Scenario::new(label, &restic_bin);
        sc.snapshot(&[&sc.b1.tag()], "2026-11-08 12:00:00");
        sc.snapshot(&[&sc.b1.tag()], "2026-11-09 12:00:00");
        let multi_old = sc.multi_tag_snapshot("2026-11-10 12:00:00");
        let multi_new = sc.multi_tag_snapshot("2026-11-11 12:00:00");

        let preview = execute_forget_dry_run_with(
            &sc.b1,
            &sc.dest,
            &sc.rig.runtime(),
            None,
            &[("TZ", "UTC")],
        )
        .unwrap_or_else(|e| panic!("{label}: preview failed: {e}"));
        for id in [&multi_old, &multi_new] {
            let entry = preview
                .snapshots
                .iter()
                .find(|s| &s.snapshot_id == id)
                .unwrap_or_else(|| panic!("{label}: {id} in the preview: {preview:?}"));
            assert_eq!(
                entry.action, "keep",
                "{label}: (a) a snapshot with two backup tags is kept"
            );
        }
        let previewed = preview.removal_ids();
        assert!(!previewed.is_empty(), "{label}: scenario removes something");

        let (spy, log) = sc.spy();
        let run = sc.run_retention(&spy);
        assert_eq!(
            run.forget_ids, previewed,
            "{label}: (b) preview and run remove the same snapshots"
        );
        let calls = spy_calls(&log);
        assert_eq!(
            forget_call_ids(calls.last().expect("forget call")),
            previewed.into_iter().collect::<BTreeSet<_>>(),
            "{label}: (b) `forget` got the previewed IDs"
        );
        let remaining = list_snapshots_for_backup(
            &sc.b1.id,
            &sc.dest,
            &sc.rig.resolved,
            &sc.rig.helper,
            &sc.rig.store,
            &sc.rig.clock,
            &[("RESTIC_CACHE_DIR", &sc.rig.cache_dir.to_string_lossy())],
        )
        .expect("list b1 snapshots");
        assert_eq!(
            remaining.len(),
            3,
            "{label}: b1's newest and both snapshots with two tags remain"
        );
    }
}

/// AC-M6-17-1: a snapshot whose second `kestic:` tag is (a) the same backup's tag again or
/// (b) one without an ID, and (c) one that also carries the backup's metadata tag, is not
/// removed either; the backup's own old snapshot still is.
#[test]
fn ac_m6_17_1_duplicate_empty_and_meta_tags_are_kept() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let sc = Scenario::new(label, &restic_bin);
        let own = sc.b1.tag();
        let meta = sc.b1.meta_tag();
        let own_old = sc.snapshot(&[&own], "2026-11-04 12:00:00");
        sc.snapshot(&[&own], "2026-11-05 12:00:00");
        let twice: Vec<String> = ["2026-11-06 12:00:00", "2026-11-07 12:00:00"]
            .iter()
            .map(|t| sc.snapshot(&[&own, &own], t))
            .collect();
        let empty: Vec<String> = ["2026-11-08 12:00:00", "2026-11-09 12:00:00"]
            .iter()
            .map(|t| sc.snapshot(&[&own, "kestic:"], t))
            .collect();
        let with_meta: Vec<String> = ["2026-11-10 12:00:00", "2026-11-11 12:00:00"]
            .iter()
            .map(|t| sc.snapshot(&[&own, &meta], t))
            .collect();
        let plain = sc.plain_restic_removes();
        for group in [&twice, &empty, &with_meta] {
            assert!(
                plain.contains(&group[0]),
                "{label}: scenario: plain restic would remove {}",
                group[0]
            );
        }

        let run = sc.run_retention(&sc.rig.resolved);
        let existing = sc.existing_ids();
        for id in &twice {
            assert!(
                existing.contains(id) && !run.forget_ids.contains(id),
                "{label}: (a) the same backup tag twice is kept: {id}"
            );
        }
        for id in &empty {
            assert!(
                existing.contains(id) && !run.forget_ids.contains(id),
                "{label}: (b) a second tag without an ID is kept: {id}"
            );
        }
        for id in &with_meta {
            assert!(
                existing.contains(id) && !run.forget_ids.contains(id),
                "{label}: (c) a snapshot with the metadata tag is kept: {id}"
            );
        }
        assert_eq!(
            run.forget_ids,
            vec![own_old],
            "{label}: only the backup's own old snapshot is removed"
        );
    }
}

/// AC-M6-17-2 (d): a backup without snapshots: restic prints no plan, Kestic runs only the plan
/// and the retention is done.
#[test]
fn ac_m6_17_2_no_snapshots_runs_only_the_plan() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let sc = Scenario::new(label, &restic_bin);
        let (spy, log) = sc.spy();
        let run = sc.run_retention(&spy);
        assert_eq!(spy_calls(&log).len(), 1, "{label}: only the plan ran");
        assert!(
            run.succeeded() && run.forget.is_none(),
            "{label}: done without `forget`: {run:?}"
        );
    }
}

/// AC-M6-17-2 (c): a cancel before the retention starts stops at the plan: no `forget` runs, the
/// retention is not done, and every snapshot remains.
#[test]
fn ac_m6_17_2_cancelled_retention_runs_no_forget() {
    for (label, restic_bin) in common::restic_test_binaries() {
        let sc = Scenario::new(label, &restic_bin);
        sc.snapshot(&[&sc.b1.tag()], "2026-11-08 12:00:00");
        sc.snapshot(&[&sc.b1.tag()], "2026-11-09 12:00:00");
        let before = sc.existing_ids();
        let (spy, log) = sc.spy();
        let cancel = CancelHandle::new();
        cancel.cancel();
        let runtime = sc.rig.runtime_with(&spy);
        let run = execute_retention(&sc.b1, &sc.dest, &runtime, Some(&cancel), &[])
            .unwrap_or_else(|e| panic!("{label}: retention failed: {e}"));
        assert!(run.cancelled, "{label}: cancelled: {run:?}");
        assert!(!run.succeeded(), "{label}: not done");
        assert!(run.forget.is_none(), "{label}: no `forget` result");
        assert!(
            spy_calls(&log)
                .iter()
                .all(|c| c.iter().any(|a| a == "--dry-run")),
            "{label}: no `forget <id…>` call"
        );
        assert_eq!(sc.existing_ids(), before, "{label}: every snapshot remains");
    }
}

/// A destination without repository and a restic stand-in: `forget … --dry-run` prints the
/// plan from `plan.json` and exits 0, every other call is logged to `calls.log` and exits
/// `forget_exit`. The repository password in the secret store is `password`.
struct FakePlan {
    dir: TempDir,
    rig: common::ResticRig,
    resolved: ResolvedRestic,
    dest: Destination,
    b1: Backup,
}

/// Waits until the freshly written `script` can be started: while a test on another thread
/// forks, its child still holds the file open for writing until it execs, and starting the
/// script fails with «text file busy» meanwhile.
fn wait_until_executable(script: &Path, probe_arg: &str) {
    for _ in 0..100 {
        match Command::new(script).arg(probe_arg).output() {
            Ok(out) => {
                assert!(out.status.success(), "probe of {}", script.display());
                return;
            }
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(err) => panic!("start {}: {err}", script.display()),
        }
    }
    panic!("{} stayed busy", script.display());
}

impl FakePlan {
    fn new(password: &str, forget_exit: i32) -> Self {
        let dir = TempDir::new().expect("tempdir");
        let repo_dir = dir.path().join("repo");
        fs::create_dir_all(&repo_dir).expect("mkdir repo");
        let rig = common::ResticRig::new(
            dir.path(),
            &common::latest_restic(),
            datetime!(2026-11-27 09:00:00 UTC),
        );
        let dest = Destination::new(
            "Fake",
            DestinationKind::Local {
                path: repo_dir.clone(),
            },
        )
        .expect("destination");
        rig.store
            .set_secret(
                dest.id.as_str(),
                SECRET_KIND_REPO,
                &SecretValue::from_str_secret(password),
            )
            .expect("store password");
        let mut b1 = Backup::new("b1", Source::folder(&repo_dir), dest.id.clone()).expect("b1");
        b1.retention = RetentionPolicy {
            keep_last: Some(KeepCount::Count(1)),
            ..RetentionPolicy::empty()
        };
        let script = dir.path().join("fake-restic.sh");
        let plan = dir.path().join("plan.json");
        let log = dir.path().join("calls.log");
        write_executable(
            &script,
            &format!(
                "#!/bin/sh\n[ \"$1\" = --probe ] && exit 0\necho \"$*\" >> \"{log}\"\ncase \" $* \" in\n  *\" --dry-run \"*) cat \"{plan}\"; exit 0 ;;\nesac\nexit {forget_exit}\n",
                log = log.display(),
                plan = plan.display(),
            ),
        );
        wait_until_executable(&script, "--probe");
        let resolved = ResolvedRestic {
            binary: script,
            version: rig.resolved.version.clone(),
        };
        Self {
            dir,
            rig,
            resolved,
            dest,
            b1,
        }
    }

    /// Writes a plan whose one group removes `ids`, each tagged with b1's tag only.
    fn plan_removing(&self, ids: &[&str]) {
        let remove: Vec<serde_json::Value> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                serde_json::json!({
                    "id": id,
                    "short_id": &id[..8],
                    "time": format!("2026-11-0{}T12:00:00Z", i + 1),
                    "hostname": "laptop",
                    "paths": ["/home/user"],
                    "tags": [self.b1.tag()],
                })
            })
            .collect();
        let plan = serde_json::json!([{
            "tags": [self.b1.tag()],
            "keep": [],
            "remove": remove,
            "reasons": [],
        }]);
        self.plan_raw(&plan.to_string());
    }

    fn plan_raw(&self, line: &str) {
        fs::write(self.dir.path().join("plan.json"), format!("{line}\n")).expect("write plan");
    }

    fn calls(&self) -> Vec<Vec<String>> {
        spy_calls(&self.dir.path().join("calls.log"))
    }

    fn retention(&self) -> Result<RetentionRun, kestic_core::backup::BackupRunError> {
        execute_retention(
            &self.b1,
            &self.dest,
            &self.rig.runtime_with(&self.resolved),
            None,
            &[],
        )
    }
}

/// AC-M6-17-2 (c): a plan restic cut off (exit 0, unreadable JSON) is an error kept with the
/// plan's call, and no `forget` runs; a plan whose groups remove nothing (`remove: null`) runs
/// none either.
#[test]
fn ac_m6_17_2_unreadable_plan_runs_no_forget() {
    let fake = FakePlan::new(&strong_password(), 0);
    fake.plan_raw(r#"[{"tags":["kestic:x"],"remove":[{"id":"#);
    let run = fake.retention().expect("the plan ran");
    assert!(run.error.is_some(), "an unreadable plan is an error");
    assert!(!run.succeeded(), "an unreadable plan is not done");
    assert_eq!(run.plan.exit_code, 0, "the plan's call stays on record");
    assert_eq!(
        fake.calls().len(),
        1,
        "only the plan ran: {:?}",
        fake.calls()
    );

    let fake = FakePlan::new(&strong_password(), 0);
    fake.plan_raw(&format!(
        r#"[{{"tags":["{}"],"keep":null,"remove":null,"reasons":null}}]"#,
        fake.b1.tag()
    ));
    let run = fake.retention().expect("empty plan");
    assert!(run.succeeded(), "an empty plan is done: {run:?}");
    assert_eq!(
        fake.calls().len(),
        1,
        "only the plan ran: {:?}",
        fake.calls()
    );
}

/// AC-M6-17-2 (c): a `forget <id…>` that fails (exit 1, e.g. a snapshot another host removed
/// after the plan) leaves the retention not done and counts no removed snapshots.
#[test]
fn ac_m6_17_2_failed_forget_is_not_done() {
    let fake = FakePlan::new(&strong_password(), 1);
    let id = "ab".repeat(32);
    fake.plan_removing(&[&id]);
    let run = fake.retention().expect("retention ran");
    assert_eq!(run.forget_ids, vec![id], "forget got the planned ID");
    assert!(!run.succeeded(), "a failed `forget` is not done");
    assert_eq!(run.removed_count(), None, "no removed snapshots counted");
    assert_eq!(fake.calls().len(), 2, "plan and one `forget`");
}

/// AC-M6-17-3 (b): the plan is read as restic wrote it, before secrets are redacted from the
/// log: a repository password that is part of a snapshot ID leaves the ID intact in the preview
/// and in `forget`.
#[test]
fn ac_m6_17_3_plan_is_read_before_redaction() {
    // A generated password made of hex digits, so it can be part of a snapshot ID.
    let password: String = strong_password()
        .bytes()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect();
    let fake = FakePlan::new(&password, 0);
    let id = format!("{password}{}", "0".repeat(56));
    fake.plan_removing(&[&id]);

    let preview = execute_forget_dry_run_with(
        &fake.b1,
        &fake.dest,
        &fake.rig.runtime_with(&fake.resolved),
        None,
        &[],
    )
    .expect("preview");
    assert_eq!(
        preview.removal_ids(),
        vec![id.clone()],
        "preview keeps the full ID"
    );

    let run = fake.retention().expect("retention ran");
    assert_eq!(run.forget_ids, vec![id.clone()], "run removes the full ID");
    let calls = fake.calls();
    assert_eq!(
        forget_call_ids(calls.last().expect("forget call")),
        BTreeSet::from([id]),
        "`forget` got the full ID"
    );
}
