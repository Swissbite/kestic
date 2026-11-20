// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared helpers for the M3 automation acceptance tests (`M3-S03`–`M3-S14`): daemon spawning with
//! injectable clock, restic wrapper with call log, state polling, and notification assertions.

use super::{CapturedNotification, NotificationCapture, find_kestic_binary, wait_for_file};
use kestic_core::clock::Clock;
use kestic_core::history::{BackupRuntimeState, RunHistory};
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, DestinationKind, ResticProvider, Schedule, Source,
};
use kestic_core::mount::MountInspector;
use kestic_core::scheduler::TimezoneSpec;
use kestic_core::secrets::FileSecretStore;
use kestic_daemon::dbus_api::{
    BackupSummaryDto, DaemonStateDto, DestinationSummaryDto, KESTIC_OBJECT_PATH, KesticClient,
};
use kestic_daemon::monitors::MonitorBusConfig;
use kestic_daemon::service::{DaemonOutcome, DaemonRunConfig, run_daemon};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// One restic binary under test (`AC-M1-Q-2`: 0.18.0 and latest).
#[derive(Debug, Clone)]
pub struct ResticVariant {
    pub index: usize,
    pub label: &'static str,
    pub bin: PathBuf,
}

impl ResticVariant {
    /// Unique session-bus name per variant so sequential daemons never collide.
    pub fn bus_name(&self, base: &str) -> String {
        format!("net.daester.Kestic.{base}V{}", self.index)
    }

    /// Unique sub-directory below the isolated test root per variant.
    pub fn sub(&self, base: &str) -> String {
        format!("{base}-v{}", self.index)
    }
}

/// Both restic versions (`restic 0.18.0` and latest) for tests that execute restic.
pub fn restic_variants() -> Vec<ResticVariant> {
    super::restic_test_binaries()
        .into_iter()
        .enumerate()
        .map(|(index, (label, bin))| ResticVariant { index, label, bin })
        .collect()
}

/// Latest restic only, for tests that never run a restic data command.
pub fn latest_variant() -> ResticVariant {
    restic_variants()
        .pop()
        .expect("latest restic variant must exist")
}

/// Returns `(config.toml path, state dir)` of the daemon sandbox `sub` below `root`.
pub fn daemon_paths(root: &Path, sub: &str) -> (PathBuf, PathBuf) {
    let base = root.join(sub);
    (
        base.join("cfg").join("kestic").join("config.toml"),
        base.join("state").join("kestic"),
    )
}

/// Options for [`spawn_m3_daemon`].
pub struct M3DaemonOpts<'a> {
    pub sub_name: &'a str,
    pub bus_name: &'a str,
    pub restic_bin: &'a Path,
    pub clock: Arc<dyn Clock>,
    pub mount_inspector: Option<Arc<dyn MountInspector>>,
    pub timezone: TimezoneSpec,
    pub enable_monitors: bool,
    pub enable_scheduler: bool,
    pub enable_job_view: bool,
    pub enable_portal_inhibit: bool,
}

impl<'a> M3DaemonOpts<'a> {
    /// Scheduler and system monitors on, JobView and portal off, Europe/Zurich time zone.
    pub fn new(
        sub_name: &'a str,
        bus_name: &'a str,
        restic_bin: &'a Path,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            sub_name,
            bus_name,
            restic_bin,
            clock,
            mount_inspector: None,
            timezone: TimezoneSpec::named("Europe/Zurich"),
            enable_monitors: true,
            enable_scheduler: true,
            enable_job_view: false,
            enable_portal_inhibit: false,
        }
    }
}

/// Running in-process daemon thread plus its D-Bus client.
pub struct M3Daemon {
    pub client: KesticClient,
    pub state_dir: PathBuf,
    pub config_path: PathBuf,
    bus_name: String,
    handle: Option<thread::JoinHandle<Result<DaemonOutcome, String>>>,
}

impl M3Daemon {
    /// Confirms quit over D-Bus and asserts a clean daemon shutdown with the bus name released.
    pub fn stop(mut self) {
        self.client.confirm_quit().expect("ConfirmQuit");
        let handle = self.handle.take().expect("daemon handle");
        assert_eq!(
            handle.join().expect("join daemon thread"),
            Ok(DaemonOutcome::RanAndStopped)
        );
        super::wait_for_bus_name_released(
            self.client.connection(),
            &self.bus_name,
            Duration::from_secs(5),
        );
    }
}

/// Starts the daemon in-process (existing `config.toml` and history in the sandbox are honoured).
pub fn spawn_m3_daemon(root: &Path, opts: M3DaemonOpts<'_>) -> M3Daemon {
    let (config_path, state_dir) = daemon_paths(root, opts.sub_name);
    let ready_file = root.join(opts.sub_name).join("daemon.ready");
    fs::create_dir_all(root.join(opts.sub_name)).expect("mkdir daemon sandbox");
    let _ = fs::remove_file(&ready_file);

    let run_cfg = DaemonRunConfig {
        bus_name: opts.bus_name.to_string(),
        assume_sni_available: true,
        ready_file: Some(ready_file.clone()),
        config_path: Some(config_path.clone()),
        state_dir: Some(state_dir.clone()),
        kestic_binary: Some(find_kestic_binary()),
        clock: opts.clock,
        secret_store: Some(Arc::new(FileSecretStore::new(root.join("secrets.hex")))),
        mount_inspector: opts.mount_inspector,
        suppress_ui_spawn: true,
        enable_job_view: opts.enable_job_view,
        enable_portal_inhibit: opts.enable_portal_inhibit,
        timezone: opts.timezone,
        monitor_bus: MonitorBusConfig {
            use_session_bus: true,
            enable_watcher: opts.enable_monitors,
            ..Default::default()
        },
        enable_system_monitors: opts.enable_monitors,
        enable_scheduler: opts.enable_scheduler,
        ..Default::default()
    };

    let handle = thread::spawn(move || run_daemon(run_cfg));
    wait_for_file(&ready_file, super::DAEMON_READY_TIMEOUT);

    let client =
        KesticClient::connect(opts.bus_name, KESTIC_OBJECT_PATH).expect("connect KesticClient");
    let prov = client
        .set_restic_provider(&ResticProvider::Custom {
            path: opts.restic_bin.to_path_buf(),
        })
        .expect("set restic provider");
    assert!(prov.valid, "restic provider must be valid: {prov:?}");
    M3Daemon {
        client,
        state_dir,
        config_path,
        bus_name: opts.bus_name.to_string(),
        handle: Some(handle),
    }
}

/// Polls `GetState` until `pred` holds; fails with a state dump after `timeout`.
pub fn wait_for_state<F>(
    client: &KesticClient,
    timeout: Duration,
    label: &str,
    mut pred: F,
) -> DaemonStateDto
where
    F: FnMut(&DaemonStateDto) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let state = client.get_state().expect("GetState");
        if pred(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for condition ({label}); last overall={:?}, backups={:?}",
            state.overall,
            state
                .backups
                .iter()
                .map(|b| (
                    &b.backup.name,
                    &b.pending_primary_code,
                    &b.pending_reasons,
                    b.is_paused,
                    b.is_overdue,
                    b.has_error,
                    b.active_run.as_ref().map(|r| &r.status),
                    b.last_run.as_ref().map(|r| (&r.status, &r.summary_message)),
                ))
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn backup_summary<'a>(state: &'a DaemonStateDto, id: &BackupId) -> &'a BackupSummaryDto {
    state
        .backups
        .iter()
        .find(|b| &b.backup.id == id)
        .expect("backup summary present")
}

pub fn dest_summary<'a>(
    state: &'a DaemonStateDto,
    dest_id: &DestinationId,
) -> &'a DestinationSummaryDto {
    state
        .destinations
        .iter()
        .find(|d| &d.destination.id == dest_id)
        .expect("destination summary present")
}

/// Asserts that a D-Bus call failed with the given `net.daester.Kestic.Error.*` name.
pub fn assert_dbus_error_name(err: &zbus::Error, expected_name: &str) {
    match err {
        zbus::Error::MethodError(name, _, _) => assert_eq!(
            name.as_str(),
            expected_name,
            "unexpected D-Bus error: {err:?}"
        ),
        other => panic!("expected D-Bus MethodError `{expected_name}`, got {other:?}"),
    }
}

/// Number of scheduler ticks (1 s each) to wait so an expected absence is meaningful.
pub const SETTLE: Duration = Duration::from_millis(2300);

/// Sleeps long enough for at least two scheduler ticks plus asynchronous worker completion.
pub fn settle() {
    thread::sleep(SETTLE);
}

/// All captured notifications carrying the KNotification event id `event`.
pub fn notifications_for(capture: &NotificationCapture, event: &str) -> Vec<CapturedNotification> {
    capture
        .snapshot()
        .into_iter()
        .filter(|n| n.event_id.as_deref() == Some(event))
        .collect()
}

/// Waits until at least `min` notifications with `event` were captured and returns them all.
pub fn wait_for_notifications(
    capture: &NotificationCapture,
    event: &str,
    min: usize,
    timeout: Duration,
) -> Vec<CapturedNotification> {
    let deadline = Instant::now() + timeout;
    loop {
        let items = notifications_for(capture, event);
        if items.len() >= min {
            return items;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {min} `{event}` notification(s); captured: {:?}",
            capture.snapshot()
        );
        thread::sleep(Duration::from_millis(40));
    }
}

/// Writes `num_files` incompressible blobs of `bytes_per_file` bytes below `dir`.
pub fn write_incompressible_dataset(dir: &Path, num_files: usize, bytes_per_file: usize) {
    fs::create_dir_all(dir).expect("mkdir dataset");
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut buf = vec![0u8; bytes_per_file];
    for idx in 0..num_files {
        for chunk in buf.chunks_exact_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            chunk.copy_from_slice(&state.to_le_bytes());
        }
        let path = dir.join(format!("blob_{idx:03}.bin"));
        let mut f = fs::File::create(&path).expect("create dataset file");
        f.write_all(&buf).expect("write dataset file");
    }
}

/// Finds a non-empty restic lock file (64 hex chars) below `locks_dir`.
pub fn find_lock_file(locks_dir: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(locks_dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()))
            && fs::metadata(&path).is_ok_and(|m| m.len() > 0)
        {
            return Some(path);
        }
    }
    None
}

/// Creates a folder backup `name` (daily 12:00) with a one-file source and a local destination.
pub struct LocalBackupFixture {
    pub dest: Destination,
    pub backup: Backup,
    pub backup_id_str: String,
    pub repo_dir: PathBuf,
    pub src_dir: PathBuf,
}

/// Adds a local destination (`restic init`) and a daily 12:00 backup through the daemon API.
pub fn add_local_backup(
    client: &KesticClient,
    root: &Path,
    sub: &str,
    name: &str,
    configure: impl FnOnce(&mut Backup),
) -> LocalBackupFixture {
    let repo_dir = root.join(sub).join(format!("repo-{name}"));
    let src_dir = root.join(sub).join(format!("src-{name}"));
    fs::create_dir_all(&repo_dir).expect("mkdir repo");
    fs::create_dir_all(&src_dir).expect("mkdir src");
    fs::write(src_dir.join("data.txt"), format!("payload {name}")).expect("write src");
    let dest = Destination::new(
        format!("Dest {name}"),
        DestinationKind::Local {
            path: repo_dir.clone(),
        },
    )
    .expect("dest");
    client
        .add_destination(&dest, "init", &format!("pw-{name}"), "")
        .expect("add destination");
    let mut backup = Backup::new(name, Source::folder(&src_dir), dest.id.clone()).expect("backup");
    backup.schedule = Schedule::Daily {
        hour: 12,
        minute: 0,
    };
    configure(&mut backup);
    let backup_id_str = client.add_backup(&backup).expect("add backup");
    backup.id = BackupId::parse(&backup_id_str).expect("backup id");
    LocalBackupFixture {
        dest,
        backup,
        backup_id_str,
        repo_dir,
        src_dir,
    }
}

/// One line of the restic wrapper call log.
#[derive(Debug, Clone)]
pub struct WrapperEntry {
    /// `S` = process start, `E` = process end (with exit code in `detail`), `T` = SIGTERM received.
    pub kind: char,
    pub at_ms: u128,
    pub sub: String,
    pub detail: String,
}

/// Shell wrapper around the real restic binary that records start, end, and SIGTERM per call
/// and can hold `backup` runs until released (progress `25 %` is emitted while held).
pub struct ResticWrapper {
    pub path: PathBuf,
    pub log: PathBuf,
    /// Prefix of the per-subcommand hold markers (`<hold>.<subcommand>`).
    pub hold: PathBuf,
}

impl ResticWrapper {
    pub fn create(dir: &Path, real_restic: &Path) -> Self {
        fs::create_dir_all(dir).expect("mkdir wrapper dir");
        let path = dir.join("restic-wrapper.sh");
        let log = dir.join("restic-wrapper.log");
        let hold = dir.join("restic-wrapper.hold");
        let script = format!(
            r#"#!/bin/sh
REAL='{real}'
LOG='{log}'
HOLD='{hold}'
now() {{ date +%s%3N; }}
sub=""
for a in "$@"; do
  case "$a" in
    backup|forget|prune|check|unlock|init|snapshots|stats|list|key|version|cat|ls|find|restore|mount|diff|copy|rewrite|tag|cache)
      sub="$a"; break;;
  esac
done
echo "S $(now) $sub $*" >> "$LOG"
if [ -n "$sub" ] && [ -f "$HOLD.$sub" ]; then
  if [ "$sub" = "backup" ]; then
    printf '{{"message_type":"status","percent_done":0.25,"bytes_done":1048576,"total_bytes":4194304,"seconds_elapsed":1}}\n'
  fi
  trap 'echo "T $(now) $sub SIGTERM" >> "$LOG"; echo "E $(now) $sub 143" >> "$LOG"; exit 143' TERM INT
  while [ -f "$HOLD.$sub" ]; do sleep 0.1; done
  trap - TERM INT
fi
"$REAL" "$@" &
pid=$!
trap 'echo "T $(now) $sub SIGTERM" >> "$LOG"; kill -TERM $pid 2>/dev/null' TERM INT
wait $pid
code=$?
if kill -0 $pid 2>/dev/null; then wait $pid; code=$?; fi
echo "E $(now) $sub $code" >> "$LOG"
exit $code
"#,
            real = real_restic.display(),
            log = log.display(),
            hold = hold.display(),
        );
        fs::write(&path, script).expect("write restic wrapper");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod wrapper");
        fs::write(&log, "").expect("create wrapper log");
        Self { path, log, hold }
    }

    fn hold_marker(&self, sub: &str) -> PathBuf {
        PathBuf::from(format!("{}.{sub}", self.hold.display()))
    }

    /// Makes every subsequent call of restic subcommand `sub` block until [`Self::release`]
    /// (a held `backup` additionally reports 25 % progress).
    pub fn hold(&self, sub: &str) {
        fs::write(self.hold_marker(sub), b"1").expect("create hold marker");
    }

    /// Lets held (and future) calls of restic subcommand `sub` run on.
    pub fn release(&self, sub: &str) {
        let _ = fs::remove_file(self.hold_marker(sub));
    }

    pub fn entries(&self) -> Vec<WrapperEntry> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut it = line.splitn(4, ' ');
                let kind = it.next()?.chars().next()?;
                let at_ms: u128 = it.next()?.parse().ok()?;
                let sub = it.next()?.to_string();
                let detail = it.next().unwrap_or("").to_string();
                Some(WrapperEntry {
                    kind,
                    at_ms,
                    sub,
                    detail,
                })
            })
            .collect()
    }

    /// Entries of kind `kind` for restic subcommand `sub` in log order.
    pub fn of(&self, kind: char, sub: &str) -> Vec<WrapperEntry> {
        self.entries()
            .into_iter()
            .filter(|e| e.kind == kind && e.sub == sub)
            .collect()
    }

    /// Number of restic processes started so far (all subcommands).
    pub fn started_count(&self) -> usize {
        self.entries().iter().filter(|e| e.kind == 'S').count()
    }

    /// Number of started restic processes that touch the repository (everything except the
    /// repository-independent `restic version` probe).
    pub fn started_repo_count(&self) -> usize {
        self.entries()
            .iter()
            .filter(|e| e.kind == 'S' && e.sub != "version")
            .count()
    }

    /// Waits until at least `min` calls of `kind`/`sub` were logged.
    pub fn wait_for(&self, kind: char, sub: &str, min: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.of(kind, sub).len() < min {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {min} wrapper `{kind} {sub}` entries; log: {:?}",
                self.entries()
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
}

/// Kills and reaps the child on drop (stops stray restic helper processes on test failure).
pub struct DropKillChild(pub std::process::Child);

impl Drop for DropKillChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Reads the persisted scheduling/backoff state of `backup_id` from the daemon's `history.db`.
pub fn runtime_state(state_dir: &Path, backup_id: &BackupId) -> BackupRuntimeState {
    RunHistory::open(state_dir)
        .expect("open history")
        .get_backup_runtime_state(backup_id)
        .expect("backup runtime state")
}

/// Waits until the persisted backoff `attempt` counter of the backup equals `attempt`.
pub fn wait_for_backoff_attempt(
    state_dir: &Path,
    backup_id: &BackupId,
    attempt: u32,
    timeout: Duration,
) -> BackupRuntimeState {
    let deadline = Instant::now() + timeout;
    loop {
        let rs = runtime_state(state_dir, backup_id);
        if rs.backoff_attempt == attempt {
            return rs;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for backoff attempt {attempt}; state: {rs:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// `restic -r <repo>` command with password and cache dir from the isolated test environment.
pub fn restic_repo_cmd(
    bin: &Path,
    repo: &Path,
    password: &str,
    cache_dir: &Path,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(["-r", &repo.to_string_lossy()])
        .env("RESTIC_PASSWORD", password)
        .env("RESTIC_CACHE_DIR", cache_dir);
    cmd
}

/// Spawns a foreign restic process (stdin piped) and waits until it created a lock in `repo_dir`.
pub fn hold_repo_lock(mut cmd: std::process::Command, repo_dir: &Path) -> DropKillChild {
    use std::process::Stdio;
    let child = DropKillChild(
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn lock holder"),
    );
    let locks = repo_dir.join("locks");
    let deadline = Instant::now() + Duration::from_secs(10);
    while find_lock_file(&locks).is_none() {
        assert!(Instant::now() < deadline, "lock never appeared");
        thread::sleep(Duration::from_millis(1));
    }
    child
}

/// Sends `signal` (e.g. `STOP`, `CONT`, `TERM`) to `pid`.
pub fn send_signal(pid: u32, signal: &str) {
    let status = std::process::Command::new("kill")
        .args([&format!("-{signal}"), &pid.to_string()])
        .status()
        .expect("kill");
    assert!(status.success(), "kill -{signal} {pid}");
}
