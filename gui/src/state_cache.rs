// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared D-Bus client state cache, signal listener, daemon auto-start,
//! and localized formatting helpers for the Qt/CXX-Qt GUI.

pub struct PendingI18n<'a>(pub &'a str);

impl PendingI18n<'_> {
    pub fn subs(self, args: &[&str]) -> String {
        crate::qt_bridge::ffi::i18n_subs(self.0, args)
    }
}

impl std::fmt::Display for PendingI18n<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&cxx_kde_frameworks::ki18n::i18n(self.0).to_string())
    }
}

pub fn i18n(text: &str) -> PendingI18n<'_> {
    PendingI18n(text)
}

/// `value` as JSON text for QML; `fallback` when it cannot be serialised.
pub fn json_or<T: serde::Serialize + ?Sized>(value: &T, fallback: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| fallback.to_string())
}

/// Plural-aware translation (`i18np`): `%1` is `n`, further `%N` come from `args`. A count
/// beyond `i64` is shown as `i64::MAX`.
pub fn i18np(singular: &str, plural: &str, n: impl TryInto<i64>, args: &[&str]) -> String {
    crate::qt_bridge::ffi::i18np_subs(singular, plural, n.try_into().unwrap_or(i64::MAX), args)
}

/// Translates a `kestic-i18n` [`Message`] (code → msgid mapping in `kestic_i18n::messages`)
/// in the GUI language with KI18n.
pub fn tr(message: &Message) -> String {
    if message.args.is_empty() {
        i18n(message.msgid).to_string()
    } else {
        let args = message.rendered_args(format_percent);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        i18n(message.msgid).subs(&args)
    }
}

/// `percent` in the GUI language: the number from `QLocale`, the place of the percent sign and
/// the space before it from the catalog («25%», German «25 %»); the one formatting path for
/// percentages in the GUI.
pub fn format_percent(percent: u8) -> String {
    i18n(PERCENT_PATTERN).subs(&[&crate::qt_bridge::ffi::format_count(i64::from(percent))])
}

/// Translates a [`UiText`]: messages via [`tr`], raw detail unchanged.
pub fn tr_text(text: &UiText) -> String {
    match text {
        UiText::Message(m) => tr(m),
        UiText::Raw(raw) => raw.clone(),
    }
}

/// Text of a failed D-Bus call in the GUI language: the error name is the code
/// (`kestic_i18n::messages`), the error message is never shown as main text.
pub fn dbus_error_text<E>(err: &E) -> String
where
    E: Clone + Into<kestic_daemon::dbus_api::KesticDbusError>,
{
    let err: kestic_daemon::dbus_api::KesticDbusError = err.clone().into();
    tr(&err.code().message())
}

use crate::daemon_events;
use crate::overall_text::localized_availability_reason;
use crate::retention_texts::within_text;
use crate::time_texts::{format_datetime_display, format_time_short};
use crate::ui_texts::GuiText;
use kestic_core::backup::{default_state_dir, exclude_file_path};
use kestic_core::model::{
    Backup, BackupId, DestinationId, DestinationKind, ExcludeRules, KeepCount,
    MaintenanceEntrySchedule, MaintenanceSchedule, NthWeekday, ResticProvider, RetentionPeriod,
    RetentionPolicy, RetentionPreset, Run, RunConditions, RunKind, RunStatus, Schedule, Source,
    SourceKind, Weekday,
};
use kestic_core::scheduler::{TimezoneSpec, format_local_slot_hint, next_backup_slots};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{
    ActiveRunProgressDto, BackupAvailabilityDto, BackupSnapshotsDto, BackupSummaryDto,
    DaemonStateDto, DestinationSummaryDto, KESTIC_BUS_NAME, KESTIC_OBJECT_PATH, KesticClient,
    OverallStatusKind, RunDetailDto, StartAllResultDto,
};
use kestic_i18n::codes::{BlockedReason, PendingReasonCode, RunProgressPhase};
use kestic_i18n::codes::{ErrorKind, ServiceErrorCode};
use kestic_i18n::messages::{
    Message, PERCENT_PATTERN, UiText, completed_message, interrupted_by_sleep_message,
    not_backed_up_message, pause_message, unmet_conditions_message,
};

/// «Background service unavailable.» (text of `ServiceErrorCode::DaemonUnreachable`).
pub fn daemon_unavailable_text() -> String {
    tr(&ServiceErrorCode::DaemonUnreachable.message())
}

/// The requested item no longer exists (text of `ServiceErrorCode::NotFound`).
pub fn not_found_text() -> String {
    tr(&ServiceErrorCode::NotFound.message())
}
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

/// Runtime options passed from `main.rs` CLI parsing.
#[derive(Debug, Clone)]
pub struct GuiLaunchConfig {
    pub bus_name: String,
    pub object_path: String,
    pub no_autostart_daemon: bool,
    pub show_quit_dialog_on_start: bool,
    pub show_about_on_start: bool,
    /// Open the window on the tab «Restore» (`--show-restore`, tray «Restore Files…»).
    pub show_restore_on_start: bool,
}

impl Default for GuiLaunchConfig {
    fn default() -> Self {
        Self {
            bus_name: KESTIC_BUS_NAME.to_string(),
            object_path: KESTIC_OBJECT_PATH.to_string(),
            no_autostart_daemon: false,
            show_quit_dialog_on_start: false,
            show_about_on_start: false,
            show_restore_on_start: false,
        }
    }
}

pub struct SharedGuiState {
    pub launch: GuiLaunchConfig,
    pub daemon_reachable: bool,
    pub daemon_error: Option<String>,
    pub state: Option<DaemonStateDto>,
    pub active_progress: HashMap<String, ActiveRunProgressDto>,
    pub snapshots: Option<BackupSnapshotsDto>,
    pub run_detail: Option<RunDetailDto>,
    /// The cached state or the reachability changed: show it in the window.
    pub state_fetched: bool,
    pub progress_dirty: bool,
    /// The GUI started the daemon and waits for it to take its bus name.
    pub daemon_starting: bool,
    /// The first state of the daemon is still on its way.
    pub first_fetch_pending: bool,
    pub pending_blocked: Option<(String, BlockedReason)>,
    /// A blocked manual run whose message waits for the state fetched after it, so the message
    /// names the backup and its unmet conditions as they are now.
    pub blocked_after_refresh: Option<(String, BlockedReason)>,
    pub pending_show_window: bool,
    pub pending_show_about: bool,
    pub pending_show_restore: bool,
    pub pending_quit: Option<(bool, String)>,
    pub maintenance_preview: Option<(String, Result<String, String>)>,
    /// The signal listener receives the daemon's signals (and has registered the GUI).
    pub listening: bool,
}

impl SharedGuiState {
    fn new(launch: GuiLaunchConfig) -> Self {
        Self {
            launch,
            daemon_reachable: false,
            daemon_error: None,
            state: None,
            active_progress: HashMap::new(),
            snapshots: None,
            run_detail: None,
            state_fetched: false,
            progress_dirty: false,
            daemon_starting: false,
            first_fetch_pending: true,
            pending_blocked: None,
            blocked_after_refresh: None,
            pending_show_window: false,
            pending_show_about: false,
            pending_show_restore: false,
            pending_quit: None,
            maintenance_preview: None,
            listening: false,
        }
    }
}

static SHARED_STATE: OnceLock<Arc<Mutex<SharedGuiState>>> = OnceLock::new();
static LAUNCH_CONFIG: OnceLock<Mutex<GuiLaunchConfig>> = OnceLock::new();

pub fn set_launch_config(config: GuiLaunchConfig) {
    let lock = LAUNCH_CONFIG.get_or_init(|| Mutex::new(GuiLaunchConfig::default()));
    if let Ok(mut guard) = lock.lock() {
        *guard = config.clone();
    }
    if let Some(shared) = SHARED_STATE.get()
        && let Ok(mut guard) = shared.lock()
    {
        guard.launch = config;
    }
}

pub fn shared_state() -> &'static Arc<Mutex<SharedGuiState>> {
    SHARED_STATE.get_or_init(|| {
        let initial = LAUNCH_CONFIG
            .get()
            .and_then(|l| l.lock().ok().map(|g| g.clone()))
            .unwrap_or_default();
        Arc::new(Mutex::new(SharedGuiState::new(initial)))
    })
}

/// The GUI's one session-bus connection: every daemon call and the signal listener use it.
static GUI_CONNECTION: Mutex<Option<zbus::Connection>> = Mutex::new(None);

/// The GUI's session-bus connection; opened on first use and kept for the whole session. A
/// failed attempt is not cached, so the next call tries again.
pub fn gui_connection() -> zbus::Result<zbus::Connection> {
    let mut slot = GUI_CONNECTION.lock_unpoisoned();
    if let Some(conn) = slot.as_ref() {
        return Ok(conn.clone());
    }
    let conn = zbus::block_on(zbus::Connection::session())?;
    *slot = Some(conn.clone());
    Ok(conn)
}

/// Forgets the GUI's connection after it was lost; the next call opens a new one.
pub fn drop_gui_connection() {
    *GUI_CONNECTION.lock_unpoisoned() = None;
}

/// A daemon client on the GUI's connection.
pub fn make_client() -> Result<KesticClient, String> {
    let conn = gui_connection().map_err(|e| e.to_string())?;
    let guard = shared_state().lock_unpoisoned();
    Ok(KesticClient::from_connection(
        conn,
        &guard.launch.bus_name,
        &guard.launch.object_path,
    ))
}

/// Starts the background threads that follow the daemon and has its state loaded in the
/// background. A daemon that is not running yet is started in the background too, so the window
/// shows at once.
pub fn initialize_and_sync() {
    daemon_events::start();
    let no_autostart = shared_state().lock_unpoisoned().launch.no_autostart_daemon;
    let owned = make_client().is_ok_and(|c| c.is_bus_owned().unwrap_or(false));
    if owned {
        // The refresher fetches the first state; meanwhile the window shows that it loads.
        daemon_events::request_refresh();
    } else if no_autostart {
        // Nothing to start: this only reports that the daemon is missing.
        ensure_daemon_and_refresh();
    } else {
        daemon_events::start_daemon_in_background();
    }
}

/// Ensures the daemon is running (spawning `kestic daemon` without `--autostart` when needed,
/// and waiting for it) and loads its state. It waits, so the Qt thread calls it only when the
/// daemon already runs.
pub fn ensure_daemon_and_refresh() {
    let launch = {
        let guard = shared_state().lock_unpoisoned();
        guard.launch.clone()
    };

    let client = match make_client() {
        Ok(c) => c,
        Err(err) => {
            let mut guard = shared_state().lock_unpoisoned();
            guard.daemon_reachable = false;
            guard.daemon_error = Some(err);
            guard.first_fetch_pending = false;
            guard.state_fetched = true;
            return;
        }
    };

    let mut owned = client.is_bus_owned().unwrap_or(false);
    if !owned && !launch.no_autostart_daemon {
        let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("kestic"));
        let mut cmd = Command::new(&exe);
        cmd.arg("daemon");
        if launch.bus_name != KESTIC_BUS_NAME {
            cmd.arg("--bus-name").arg(&launch.bus_name);
        }
        // stderr is inherited so the daemon's lifecycle messages reach the session journal.
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());

        match cmd.spawn() {
            Ok(mut child) => owned = wait_for_daemon(&client, &mut child),
            Err(err) => {
                let mut guard = shared_state().lock_unpoisoned();
                guard.daemon_reachable = false;
                guard.daemon_error = Some(err.to_string());
                guard.first_fetch_pending = false;
                guard.state_fetched = true;
                return;
            }
        }
    }

    if !owned {
        let mut guard = shared_state().lock_unpoisoned();
        guard.daemon_reachable = false;
        guard.daemon_error = Some(i18n("The background service did not start.").to_string());
        guard.first_fetch_pending = false;
        guard.state_fetched = true;
        return;
    }

    refresh_state_now();
}

/// Longest wait for a started daemon to take its bus name; a slow disk or a busy machine may
/// need seconds.
const DAEMON_START_LIMIT: Duration = Duration::from_secs(60);

/// Waits until the started daemon owns its bus name (`true`), or until it exited or the limit
/// passed (`false`). A daemon that ended because another one took the name in the meantime
/// counts as started.
fn wait_for_daemon(client: &KesticClient, child: &mut std::process::Child) -> bool {
    let deadline = Instant::now() + DAEMON_START_LIMIT;
    loop {
        if client.is_bus_owned().unwrap_or(false) {
            return true;
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            return client.is_bus_owned().unwrap_or(false);
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Drops the progress of runs that ended. Still active are the run of each backup and every run
/// the state lists as running: a restore or a maintenance also runs beside a backup, and only
/// one run is the overall progress. That one is added when its progress has not arrived yet.
fn retain_active_progress(
    progress: &mut HashMap<String, ActiveRunProgressDto>,
    backups: &[BackupSummaryDto],
    recent_runs: &[Run],
    overall: Option<&ActiveRunProgressDto>,
) {
    let active_ids: HashSet<&str> = backups
        .iter()
        .filter_map(|b| b.active_run.as_ref())
        .chain(
            recent_runs
                .iter()
                .filter(|r| r.status == RunStatus::Running),
        )
        .map(|r| r.id.as_str())
        .collect();
    progress.retain(|id, _| active_ids.contains(id.as_str()));
    if let Some(p) = overall {
        progress
            .entry(p.run_id.clone())
            .or_insert_with(|| p.clone());
    }
}

pub fn refresh_state_now() {
    daemon_events::start();
    let client = match make_client() {
        Ok(c) => c,
        Err(err) => {
            let mut guard = shared_state().lock_unpoisoned();
            guard.daemon_reachable = false;
            guard.daemon_error = Some(err);
            guard.first_fetch_pending = false;
            guard.state_fetched = true;
            return;
        }
    };

    match client.get_state() {
        Ok(state) => {
            let (became_reachable, listening) = {
                let mut guard = shared_state().lock_unpoisoned();
                let was_reachable = guard.daemon_reachable;
                guard.daemon_reachable = true;
                guard.daemon_error = None;
                retain_active_progress(
                    &mut guard.active_progress,
                    &state.backups,
                    &state.recent_runs,
                    state.overall.active_run_progress.as_ref(),
                );
                guard.state = Some(state);
                guard.first_fetch_pending = false;
                guard.state_fetched = true;
                (!was_reachable, guard.listening)
            };
            // A daemon that (re)appeared learns that this GUI is open; before the listener
            // receives signals, the listener registers the GUI itself.
            if became_reachable && listening {
                let _ = client.register_gui();
            }
        }
        Err(err) => {
            {
                let mut guard = shared_state().lock_unpoisoned();
                guard.daemon_reachable = false;
                guard.daemon_error = Some(err.to_string());
                guard.first_fetch_pending = false;
                guard.state_fetched = true;
            }
            daemon_events::retry_while_unreachable();
        }
    }
}

/// Kind of the cached destination `dest_id`, if the daemon state knows it.
pub fn cached_destination_kind(dest_id: &str) -> Option<DestinationKind> {
    let guard = shared_state().lock_unpoisoned();
    guard.state.as_ref().and_then(|s| {
        s.destinations
            .iter()
            .find(|d| d.destination.id.as_str() == dest_id.trim())
            .map(|d| d.destination.kind.clone())
    })
}

/// New backup as the wizard creates it; on a local destination it has no network run
/// conditions.
pub fn build_new_backup(
    name: &str,
    source_path: &str,
    dest_id: &str,
    excludes_text: &str,
    detected_source: Option<Source>,
    dest_kind: Option<&DestinationKind>,
) -> Result<Backup, String> {
    let d_id = DestinationId::parse(dest_id.trim()).map_err(|e| e.to_string())?;
    let source =
        detected_source.unwrap_or_else(|| Source::folder(PathBuf::from(source_path.trim())));
    let mut backup = Backup::new(name.trim(), source, d_id).map_err(|e| e.to_string())?;
    if let Some(kind) = dest_kind {
        backup.run_conditions = backup.run_conditions.clone().for_destination(kind);
    }
    backup.schedule = Schedule::default_scheduled();
    backup.excludes = ExcludeRules::from_text(excludes_text);
    Ok(backup)
}

pub fn clone_updated_backup_excludes(
    existing: &Backup,
    excludes: ExcludeRules,
) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.excludes = excludes;
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
}

pub fn clone_updated_backup_retention(
    existing: &Backup,
    retention: RetentionPolicy,
) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.retention = retention;
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
}

pub fn clone_updated_backup_source(existing: &Backup, source: Source) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.set_source(source).map_err(|e| e.to_string())?;
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
}

pub fn clone_updated_backup_schedule(
    existing: &Backup,
    schedule: Schedule,
    overdue_threshold: Option<u32>,
) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.schedule = schedule;
    updated.overdue_threshold = overdue_threshold;
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
}

/// Backup with the edited run conditions; network conditions are dropped for a local
/// destination.
pub fn clone_updated_backup_conditions(
    existing: &Backup,
    run_conditions: RunConditions,
    dest_kind: Option<&DestinationKind>,
) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.run_conditions = match dest_kind {
        Some(kind) => run_conditions.for_destination(kind),
        None => run_conditions,
    };
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
}

fn format_keep_count_part(kc: KeepCount, unit_singular_plural: &str) -> String {
    match kc {
        KeepCount::Unlimited => i18n("all %1").subs(&[unit_singular_plural]),
        KeepCount::Count(n) => format!("{n} {unit_singular_plural}"),
    }
}

pub fn format_retention_summary(policy: &RetentionPolicy) -> String {
    match policy.preset() {
        RetentionPreset::Standard => {
            return GuiText::PresetStandard.text();
        }
        RetentionPreset::Economical => {
            return GuiText::PresetEconomical.text();
        }
        RetentionPreset::Long => {
            return GuiText::PresetLong.text();
        }
        RetentionPreset::Custom => {}
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(kc) = policy.keep_last {
        parts.push(match kc {
            KeepCount::Unlimited => i18n("all snapshots").to_string(),
            KeepCount::Count(n) => i18n("Last %1").subs(&[&n.to_string()]),
        });
    }
    if let Some(kc) = policy.keep_hourly {
        parts.push(format_keep_count_part(kc, &i18n("hourly").to_string()));
    }
    if let Some(kc) = policy.keep_daily {
        parts.push(format_keep_count_part(kc, &i18n("daily").to_string()));
    }
    if let Some(kc) = policy.keep_weekly {
        parts.push(format_keep_count_part(kc, &i18n("weekly").to_string()));
    }
    if let Some(kc) = policy.keep_monthly {
        parts.push(format_keep_count_part(kc, &i18n("monthly").to_string()));
    }
    if let Some(kc) = policy.keep_yearly {
        parts.push(format_keep_count_part(kc, &i18n("yearly").to_string()));
    }
    let within_rules = [
        (None, &policy.keep_within),
        (Some(RetentionPeriod::Hourly), &policy.keep_within_hourly),
        (Some(RetentionPeriod::Daily), &policy.keep_within_daily),
        (Some(RetentionPeriod::Weekly), &policy.keep_within_weekly),
        (Some(RetentionPeriod::Monthly), &policy.keep_within_monthly),
        (Some(RetentionPeriod::Yearly), &policy.keep_within_yearly),
    ];
    for (period, duration) in within_rules {
        if let Some(duration) = duration {
            parts.push(within_text(period, duration));
        }
    }
    if parts.is_empty() {
        i18n("No rules configured").to_string()
    } else {
        i18n("Custom (%1)").subs(&[&parts.join(", ")])
    }
}

pub fn format_exclude_summary(excludes: &ExcludeRules) -> String {
    let mut parts: Vec<String> = Vec::new();
    let folder_count = excludes
        .folders
        .iter()
        .filter(|f| !f.trim().is_empty())
        .count();
    if folder_count > 0 {
        parts.push(i18np("%1 folder", "%1 folders", folder_count, &[]));
    }
    if excludes.exclude_caches {
        parts.push(i18n("caches").to_string());
    }
    if excludes.exclude_trash {
        parts.push(i18n("trash").to_string());
    }
    if let Some(ref limit) = excludes.exclude_larger_than
        && !limit.trim().is_empty()
    {
        parts.push(i18n("files over %1").subs(&[limit.trim()]));
    }
    let expert_count = excludes.expert_rule_count();
    if expert_count > 0 {
        parts.push(i18np(
            "%1 expert rule",
            "%1 expert rules",
            expert_count,
            &[],
        ));
    }
    if !excludes.exclude_if_present.is_empty() {
        parts.push(i18n("marker: %1").subs(&[&excludes.exclude_if_present.join(", ")]));
    }
    if parts.is_empty() {
        GuiText::None.text()
    } else {
        parts.join(" · ")
    }
}

pub fn format_source_kind_label(source: &Source) -> String {
    match source.kind {
        SourceKind::Drive => {
            if let Some(lbl) = source
                .drive_label
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                GuiText::DriveWithLabel.with_args(&[lbl.trim()])
            } else {
                GuiText::Drive.text()
            }
        }
        SourceKind::Folder => GuiText::Folder.text(),
    }
}

pub fn backup_exclude_file_path_str(backup_id: &BackupId) -> String {
    let state_dir = default_state_dir().unwrap_or_else(|_| PathBuf::from("~/.local/state/kestic"));
    exclude_file_path(&state_dir, backup_id)
        .to_string_lossy()
        .into_owned()
}

pub fn build_provider(kind: &str, custom_path: &str) -> Result<ResticProvider, String> {
    match kind {
        "custom" => {
            let trimmed = custom_path.trim();
            if trimmed.is_empty() {
                return Err(i18n("Custom restic path must not be empty.").to_string());
            }
            Ok(ResticProvider::Custom {
                path: PathBuf::from(trimmed),
            })
        }
        "automatic" => Ok(ResticProvider::Automatic),
        "app_managed" => Ok(ResticProvider::AppManaged),
        "system" => Ok(ResticProvider::System),
        other => Err(format!("unknown restic source '{other}'")),
    }
}

pub fn format_relative_time(ts: OffsetDateTime, now: OffsetDateTime) -> String {
    let diff = (now - ts).whole_seconds();
    if diff < 60 {
        return i18n("just now").to_string();
    }
    let mins = diff / 60;
    if mins < 60 {
        return i18np("%1 minute ago", "%1 minutes ago", mins, &[]);
    }
    let hours = mins / 60;
    if hours < 24 {
        return i18np("%1 hour ago", "%1 hours ago", hours, &[]);
    }
    i18np("%1 day ago", "%1 days ago", hours / 24, &[])
}

pub fn format_duration_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    if total_secs < 60 {
        return i18n("%1 s").subs(&[&total_secs.to_string()]);
    }
    let mins = total_secs / 60;
    let rem_secs = total_secs % 60;
    if mins < 60 {
        if rem_secs == 0 {
            return i18n("%1 min").subs(&[&mins.to_string()]);
        }
        return i18n("%1 min %2 s").subs(&[&mins.to_string(), &rem_secs.to_string()]);
    }
    let hours = mins / 60;
    let rem_mins = mins % 60;
    i18n("%1 h %2 min").subs(&[&hours.to_string(), &rem_mins.to_string()])
}

/// A size in decimal units (B, KB, MB, GB) with the decimal separator of the locale; the one
/// formatting path for sizes in models and QML.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let b = bytes as f64;
    let decimal = crate::qt_bridge::ffi::format_decimal;
    if b >= GB {
        format!("{} GB", decimal(b / GB, 1))
    } else if b >= MB {
        format!("{} MB", decimal(b / MB, 1))
    } else if b >= KB {
        format!("{} KB", decimal(b / KB, 0))
    } else {
        format!("{bytes} B")
    }
}

fn weekday_short_label(w: Weekday) -> String {
    match w {
        Weekday::Monday => GuiText::Mon.text(),
        Weekday::Tuesday => GuiText::Tue.text(),
        Weekday::Wednesday => GuiText::Wed.text(),
        Weekday::Thursday => GuiText::Thu.text(),
        Weekday::Friday => GuiText::Fri.text(),
        Weekday::Saturday => GuiText::Sat.text(),
        Weekday::Sunday => GuiText::Sun.text(),
    }
}

fn weekday_full_label(w: Weekday) -> String {
    match w {
        Weekday::Monday => GuiText::Monday.text(),
        Weekday::Tuesday => GuiText::Tuesday.text(),
        Weekday::Wednesday => GuiText::Wednesday.text(),
        Weekday::Thursday => GuiText::Thursday.text(),
        Weekday::Friday => GuiText::Friday.text(),
        Weekday::Saturday => GuiText::Saturday.text(),
        Weekday::Sunday => GuiText::Sunday.text(),
    }
}

fn nth_weekday_label(nth: NthWeekday) -> String {
    match nth {
        NthWeekday::First => i18n("first").to_string(),
        NthWeekday::Second => i18n("second").to_string(),
        NthWeekday::Third => i18n("third").to_string(),
        NthWeekday::Fourth => i18n("fourth").to_string(),
        NthWeekday::Last => i18n("last").to_string(),
    }
}

fn weekday_plural_label(weekday: Weekday) -> String {
    match weekday {
        Weekday::Monday => i18n("Mondays").to_string(),
        Weekday::Tuesday => i18n("Tuesdays").to_string(),
        Weekday::Wednesday => i18n("Wednesdays").to_string(),
        Weekday::Thursday => i18n("Thursdays").to_string(),
        Weekday::Friday => i18n("Fridays").to_string(),
        Weekday::Saturday => i18n("Saturdays").to_string(),
        Weekday::Sunday => i18n("Sundays").to_string(),
    }
}

pub fn format_schedule_summary(schedule: &Schedule) -> String {
    match schedule {
        Schedule::ManualOnly => GuiText::ManualOnly.text(),
        Schedule::Daily { hour, minute } => {
            let hhmm = format!("{hour:02}:{minute:02}");
            i18n("Daily at %1").subs(&[&hhmm])
        }
        Schedule::SpecificWeekdays {
            weekdays,
            hour,
            minute,
        } => {
            let mut sorted = weekdays.clone();
            sorted.sort_by_key(|w| w.iso_number());
            sorted.dedup();
            let days: Vec<String> = sorted.into_iter().map(weekday_short_label).collect();
            let hhmm = format!("{hour:02}:{minute:02}");
            if days.is_empty() {
                i18n("On weekdays at %1").subs(&[&hhmm])
            } else {
                i18n("On %1 at %2").subs(&[&days.join(", "), &hhmm])
            }
        }
        Schedule::Weekly {
            weekday,
            hour,
            minute,
        } => {
            let day = Weekday::from_iso_number(*weekday)
                .map_or_else(|_| weekday.to_string(), weekday_full_label);
            let hhmm = format!("{hour:02}:{minute:02}");
            i18n("Weekly on %1 at %2").subs(&[&day, &hhmm])
        }
        Schedule::EveryNDays {
            interval_days,
            start_date,
            hour,
            minute,
        } => {
            let hhmm = format!("{hour:02}:{minute:02}");
            i18np(
                "Every %1 day from %2 at %3",
                "Every %1 days from %2 at %3",
                *interval_days,
                &[start_date, &hhmm],
            )
        }
    }
}

pub fn format_conditions_summary(conditions: &RunConditions) -> String {
    let mut parts: Vec<String> = Vec::new();
    if conditions.require_ac_power {
        parts.push(i18n("AC power only").to_string());
    }
    if conditions.require_network {
        parts.push(i18n("Network required").to_string());
    }
    if conditions.skip_on_metered_network {
        parts.push(i18n("Not on metered connection").to_string());
    }
    if !conditions.allowed_networks.is_empty() {
        parts.push(i18n("Networks: %1").subs(&[&conditions.allowed_networks.join(", ")]));
    }
    if parts.is_empty() {
        i18n("No restrictions").to_string()
    } else {
        parts.join(" · ")
    }
}

pub fn format_maintenance_entry(entry: &MaintenanceEntrySchedule) -> String {
    match entry {
        MaintenanceEntrySchedule::ManualOnly => GuiText::ManualOnly.text(),
        MaintenanceEntrySchedule::Daily { hour, minute } => {
            let hhmm = format!("{hour:02}:{minute:02}");
            i18n("Daily at %1").subs(&[&hhmm])
        }
        MaintenanceEntrySchedule::Weekly {
            weekday,
            hour,
            minute,
        } => {
            let hhmm = format!("{hour:02}:{minute:02}");
            i18n("%1 at %2").subs(&[&weekday_plural_label(*weekday), &hhmm])
        }
        MaintenanceEntrySchedule::Monthly {
            nth,
            weekday,
            hour,
            minute,
        } => {
            let hhmm = format!("{hour:02}:{minute:02}");
            i18n("%1 %2 of the month at %3").subs(&[
                &nth_weekday_label(*nth),
                &weekday_full_label(*weekday),
                &hhmm,
            ])
        }
    }
}

pub fn format_maintenance_summary(maint: &MaintenanceSchedule) -> String {
    i18n("Clean up: %1 · Verify: %2").subs(&[
        &format_maintenance_entry(&maint.prune),
        &format_maintenance_entry(&maint.check),
    ])
}

pub fn format_slot_timestamp(unix_opt: Option<i64>, now: OffsetDateTime) -> String {
    let Some(ts) = unix_opt else {
        return String::new();
    };
    let Ok(dt) = OffsetDateTime::from_unix_timestamp(ts) else {
        return String::new();
    };
    format_local_slot_hint(dt, now, &TimezoneSpec::System)
}

pub fn format_next_scheduled_line(
    next_unix: Option<i64>,
    schedule: &Schedule,
    now: OffsetDateTime,
) -> String {
    let hint = format_slot_timestamp(next_unix, now);
    if !hint.is_empty() {
        return i18n("Next: %1").subs(&[&hint]);
    }
    if matches!(schedule, Schedule::ManualOnly) {
        return i18n("Schedule: Manual only").to_string();
    }
    String::new()
}

pub fn preview_next_schedule_slots(schedule: &Schedule, now: OffsetDateTime) -> Vec<String> {
    next_backup_slots(schedule, now, 3, &TimezoneSpec::System)
        .into_iter()
        .map(format_datetime_display)
        .collect()
}

pub fn format_pause_line(
    since_unix: Option<i64>,
    until_unix: Option<i64>,
    now: OffsetDateTime,
) -> String {
    let since_str = since_unix
        .and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok())
        .map_or_else(|| "–".to_string(), format_time_short);
    let until_str = format_slot_timestamp(until_unix, now);
    let until = (!until_str.is_empty()).then_some(until_str.as_str());
    tr(&pause_message(&since_str, until))
}

/// Waiting reason in the GUI locale from the daemon's reason code; the backoff reason uses
/// `retry_at_unix` (`pending_retry_at_unix`).
pub fn pending_reason_line(
    code: PendingReasonCode,
    retry_at_unix: Option<i64>,
    now: OffsetDateTime,
) -> String {
    tr(&code.waiting_message(&format_slot_timestamp(retry_at_unix, now)))
}

/// Cause of the last run's error/warning of `summary` in the GUI locale (`error_kind`).
pub fn localized_last_error(summary: &BackupSummaryDto) -> String {
    let is_warning = summary
        .last_run
        .as_ref()
        .is_some_and(|r| r.status == RunStatus::Warning);
    match summary.error_kind {
        Some(kind) => tr(&kind.message(summary.error_exit_code, is_warning)),
        None if is_warning => String::new(),
        None => i18n("Backup failed").to_string(),
    }
}

/// Unmet run conditions of `summary` as «Condition not met: …» in the GUI locale.
pub fn localized_unmet_conditions(summary: &BackupSummaryDto) -> Option<String> {
    if summary.unmet_run_conditions.is_empty() {
        return None;
    }
    let allowed = &summary.backup.run_conditions.allowed_networks;
    let parts: Vec<String> = summary
        .unmet_run_conditions
        .iter()
        .map(|code| tr(&code.condition_message(summary.battery_percent, allowed)))
        .collect();
    Some(tr(&unmet_conditions_message(&parts.join(", "))))
}

/// Status line of a backup whose source or destination is unavailable.
///
/// A manual-only backup is never due, so it never waits (glossary «wartet»): it shows the
/// availability only, e.g. «Manual only · Destination unreachable».
fn unavailable_status(
    schedule: &Schedule,
    availability: &BackupAvailabilityDto,
) -> (String, String, String) {
    if matches!(schedule, Schedule::ManualOnly) {
        let text = GuiText::TwoParts.with_args(&[
            &GuiText::ManualOnly.text(),
            &localized_availability_reason(availability),
        ]);
        return (text, "state-information".to_string(), "text".to_string());
    }
    let reason_line = tr(&BlockedReason::waiting_message(
        availability.blocked_reason,
        availability.source_status,
    ));
    (reason_line, "chronometer".to_string(), "text".to_string())
}

pub fn format_backup_status(
    summary: &BackupSummaryDto,
    active_progress: Option<&ActiveRunProgressDto>,
    now: OffsetDateTime,
) -> (String, String, String) {
    if summary.active_run.is_some() {
        let text = match active_progress {
            Some(p) if p.phase == RunProgressPhase::ScanFinished => {
                let pct = kestic_i18n::messages::rounded_percent(p.percent_done);
                i18n("Backing up · %1").subs(&[&format_percent(pct)])
            }
            _ => i18n("Backing up · looking through files").to_string(),
        };
        return (text, "state-sync".to_string(), "highlight".to_string());
    }

    if summary.has_error
        && let Some(run) = &summary.last_run
        && run.status == RunStatus::Error
    {
        let cause = localized_last_error(summary);
        let text = i18n("Failed: %1").subs(&[&cause]);
        return (text, "state-error".to_string(), "negative".to_string());
    }

    if summary.is_overdue {
        let text = if summary.missed_slots_count > 0 {
            i18n("Overdue (%1 missed)").subs(&[&summary.missed_slots_count.to_string()])
        } else {
            i18n("Overdue").to_string()
        };
        return (text, "alarm-symbolic".to_string(), "negative".to_string());
    }

    if summary.is_paused {
        return (
            format_pause_line(summary.paused_since_unix, summary.paused_until_unix, now),
            "state-pause".to_string(),
            "text".to_string(),
        );
    }

    if let Some(code) = summary.pending_primary_code {
        return (
            pending_reason_line(code, summary.pending_retry_at_unix, now),
            "chronometer".to_string(),
            "text".to_string(),
        );
    }

    if !summary.availability.available {
        return unavailable_status(&summary.backup.schedule, &summary.availability);
    }

    if let Some(run) = &summary.last_run {
        match run.status {
            RunStatus::Error => {
                let cause = localized_last_error(summary);
                let text = i18n("Failed: %1").subs(&[&cause]);
                return (text, "state-error".to_string(), "negative".to_string());
            }
            RunStatus::Warning => {
                let count = summary.unreadable_files.len();
                let text = if count > 0 {
                    i18np(
                        "Backed up, %1 file unreadable",
                        "Backed up, %1 files unreadable",
                        count,
                        &[],
                    )
                } else if summary.pending_forget {
                    tr(&ErrorKind::RetentionPending.message(None, false))
                } else {
                    tr(&ErrorKind::Unknown.message(None, true))
                };
                return (text, "state-warning".to_string(), "neutral".to_string());
            }
            RunStatus::Ok => {
                let rel = format_relative_time(run.finished_at.unwrap_or(run.started_at), now);
                let text = i18n("Backed up · %1").subs(&[&rel]);
                return (text, "state-ok".to_string(), "positive".to_string());
            }
            RunStatus::Cancelled => {
                return (
                    i18n("Cancelled").to_string(),
                    "process-stop".to_string(),
                    "text".to_string(),
                );
            }
            RunStatus::Interrupted => {
                return (
                    tr(&interrupted_by_sleep_message()),
                    "state-offline".to_string(),
                    "text".to_string(),
                );
            }
            RunStatus::Pending => {
                return (
                    i18n("Waiting").to_string(),
                    "chronometer".to_string(),
                    "text".to_string(),
                );
            }
            RunStatus::Running => {
                return (
                    i18n("Backing up · %1").subs(&[&format_percent(0)]),
                    "state-sync".to_string(),
                    "highlight".to_string(),
                );
            }
        }
    }

    (
        tr(&not_backed_up_message()),
        "state-information".to_string(),
        "text".to_string(),
    )
}

pub fn format_destination_status(
    summary: &DestinationSummaryDto,
    now: OffsetDateTime,
) -> (String, String) {
    if summary.password_rejected {
        return (
            tr(&ErrorKind::WrongPassword.message(None, false)),
            "state-error".to_string(),
        );
    }
    if summary.has_stale_lock {
        return (
            i18n("Lock from an interrupted run").to_string(),
            "object-locked".to_string(),
        );
    }
    if !summary.availability.reachable {
        return (
            tr(&ErrorKind::DestinationUnreachable.message(None, false)),
            "network-disconnect".to_string(),
        );
    }
    let checked =
        OffsetDateTime::from_unix_timestamp(summary.availability.checked_at_unix).unwrap_or(now);
    let rel = format_relative_time(checked, now);
    let text = i18n("Reachable · last checked %1").subs(&[&rel]);
    (text, "network-connect".to_string())
}

pub fn format_overall_visual(kind: OverallStatusKind) -> (&'static str, &'static str) {
    match kind {
        OverallStatusKind::Blocked | OverallStatusKind::Error => ("state-error", "negative"),
        OverallStatusKind::Overdue => ("alarm-symbolic", "negative"),
        OverallStatusKind::Warning => ("state-warning", "neutral"),
        OverallStatusKind::Running => ("state-sync", "highlight"),
        OverallStatusKind::Paused => ("state-pause", "text"),
        OverallStatusKind::Pending => ("chronometer", "text"),
        OverallStatusKind::Interrupted => ("state-offline", "text"),
        OverallStatusKind::Cancelled => ("process-stop", "text"),
        OverallStatusKind::NeverRun | OverallStatusKind::ManualOnly => {
            ("state-information", "text")
        }
        OverallStatusKind::Ok => ("state-ok", "positive"),
    }
}

pub fn format_run_status(
    kind: &RunKind,
    status: RunStatus,
) -> (String, &'static str, &'static str) {
    match status {
        // A maintenance run (`forget`/`prune`/`check`) backs nothing up.
        RunStatus::Ok if matches!(kind, RunKind::Maintenance) => {
            (tr(&completed_message()), "state-ok", "positive")
        }
        RunStatus::Ok => (i18n("Backed up").to_string(), "state-ok", "positive"),
        RunStatus::Warning => (i18n("Warning").to_string(), "state-warning", "neutral"),
        RunStatus::Error => (i18n("Failed").to_string(), "state-error", "negative"),
        RunStatus::Running => (i18n("Running").to_string(), "state-sync", "highlight"),
        RunStatus::Pending => (i18n("Waiting").to_string(), "chronometer", "text"),
        RunStatus::Cancelled => (i18n("Cancelled").to_string(), "process-stop", "text"),
        RunStatus::Interrupted => (GuiText::Interrupted.text(), "state-offline", "text"),
    }
}

/// Title of a run in the history; a backup that no longer exists is named as deleted, never
/// by its internal ID.
pub fn format_run_kind_name(run: &Run, backup_name: Option<&str>, dest_name: &str) -> String {
    match &run.kind {
        RunKind::Backup { .. } => {
            backup_name.map_or_else(|| GuiText::DeletedBackup.text(), ToOwned::to_owned)
        }
        RunKind::Maintenance => i18n("Maintenance %1").subs(&[dest_name]),
        RunKind::Restore { .. } => i18n("Restore %1").subs(&[backup_name.unwrap_or(dest_name)]),
        RunKind::Meta => i18n("Metadata %1").subs(&[dest_name]),
    }
}

pub fn localized_manual_block_message(backup_name: &str, reason: BlockedReason) -> String {
    tr(&reason.manual_block_message(backup_name))
}

pub fn localized_condition_override_message(backup_name: &str, unmet_text: Option<&str>) -> String {
    if let Some(detail) = unmet_text.filter(|s| !s.trim().is_empty()) {
        i18n("Run conditions for '%1' are currently not met (%2). Start backup anyway?")
            .subs(&[backup_name, detail])
    } else {
        i18n("Run conditions for '%1' are currently not met. Start backup anyway?")
            .subs(&[backup_name])
    }
}

/// Localized reason for a skipped backup of «Back Up All Now».
fn localized_skip_reason(skipped: &kestic_daemon::dbus_api::SkippedBackupDto) -> String {
    tr_text(
        &skipped
            .reason
            .text(skipped.error_kind, skipped.error_detail.as_deref()),
    )
}

pub fn localized_start_all_summary(res: &StartAllResultDto) -> Option<String> {
    let queued = res.queued_backup_ids.len();
    let skipped = res.skipped.len();
    if queued == 0 && skipped == 0 {
        return None;
    }
    let details: Vec<String> = res
        .skipped
        .iter()
        .map(|s| format!("{} ({})", s.backup_name, localized_skip_reason(s)))
        .collect();
    let details = details.join(", ");
    if skipped == 0 {
        return Some(i18np(
            "%1 backup waits for the current run on its destination",
            "%1 backups wait for the current run on their destination",
            queued,
            &[],
        ));
    }
    let skipped_text = i18np(
        "%1 backup skipped: %2",
        "%1 backups skipped: %2",
        skipped,
        &[&details],
    );
    if queued == 0 {
        return Some(skipped_text);
    }
    Some(i18np(
        "%1 backup queued, %2",
        "%1 backups queued, %2",
        queued,
        &[&skipped_text],
    ))
}

pub fn lookup_backup_summary<'a>(
    state: &'a DaemonStateDto,
    backup_id: &str,
) -> Option<&'a BackupSummaryDto> {
    BackupId::parse(backup_id)
        .ok()
        .and_then(|id| state.backups.iter().find(|b| b.backup.id == id))
}

#[cfg(test)]
#[path = "state_cache_tests.rs"]
mod tests;
