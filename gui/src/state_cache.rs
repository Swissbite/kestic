// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared D-Bus client state cache, signal listener, daemon auto-start (`M1-S21`),
//! and localized formatting helpers for the Qt/CXX-Qt GUI (`NFR-06`).

pub struct PendingI18n<'a>(pub &'a str);

impl PendingI18n<'_> {
    pub fn subs(self, args: &[&str]) -> String {
        crate::smoke::ffi::i18n_subs(self.0, args)
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

/// Plural-aware translation (`i18np`): `%1` is `n`, further `%N` come from `args`.
pub fn i18np(singular: &str, plural: &str, n: usize, args: &[&str]) -> String {
    crate::smoke::ffi::i18np_subs(singular, plural, i64::try_from(n).unwrap_or(i64::MAX), args)
}

/// Translates a `core` [`Message`] (code → msgid mapping in `kestic_i18n::messages`) in the
/// GUI language with KI18n.
pub fn tr(message: &Message) -> String {
    if message.args.is_empty() {
        i18n(message.msgid).to_string()
    } else {
        let args: Vec<&str> = message.args.iter().map(String::as_str).collect();
        i18n(message.msgid).subs(&args)
    }
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

use crate::overall_text::localized_availability_reason;
use kestic_core::backup::{default_state_dir, exclude_file_path};
use kestic_core::clock::{Clock, SystemClock};
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, DestinationKind, ExcludeRules, KeepCount,
    MaintenanceEntrySchedule, MaintenanceSchedule, NthWeekday, ResticProvider, RetentionPolicy,
    RetentionPreset, Run, RunConditions, RunKind, RunStatus, Schedule, Source, SourceKind, Weekday,
};
use kestic_core::scheduler::{TimezoneSpec, format_local_slot_hint, next_backup_slots};
use kestic_daemon::dbus_api::{
    ActiveRunProgressDto, BackupAvailabilityDto, BackupSnapshotsDto, BackupSummaryDto,
    ConnectionTestResultDto, DaemonStateDto, DestinationSummaryDto, KESTIC_BUS_NAME,
    KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH, KesticClient, OverallStatusKind, RunDetailDto,
    StartAllResultDto,
};
use kestic_i18n::codes::{BlockedReason, ConnectionTestCode, PendingReasonCode};
use kestic_i18n::codes::{ErrorKind, ServiceErrorCode};
use kestic_i18n::messages::{
    Message, UiText, completed_message, interrupted_by_sleep_message, not_backed_up_message,
    pause_message, unmet_conditions_message,
};

/// «Background service unavailable.» (text of `ServiceErrorCode::DaemonUnreachable`).
pub fn daemon_unavailable_text() -> String {
    tr(&ServiceErrorCode::DaemonUnreachable.message())
}

/// The requested item no longer exists (text of `ServiceErrorCode::NotFound`).
pub fn not_found_text() -> String {
    tr(&ServiceErrorCode::NotFound.message())
}
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use zbus::export::ordered_stream::OrderedStreamExt;

/// Runtime options passed from `main.rs` CLI parsing / environment variables.
#[derive(Debug, Clone)]
pub struct GuiLaunchConfig {
    pub bus_name: String,
    pub object_path: String,
    pub daemon_bin: Option<PathBuf>,
    pub no_autostart_daemon: bool,
    pub show_quit_dialog_on_start: bool,
}

impl Default for GuiLaunchConfig {
    fn default() -> Self {
        Self {
            bus_name: env::var("KESTIC_BUS_NAME")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| KESTIC_BUS_NAME.to_string()),
            object_path: KESTIC_OBJECT_PATH.to_string(),
            daemon_bin: env::var_os("KESTIC_DAEMON_BIN")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            no_autostart_daemon: env::var("KESTIC_NO_AUTOSTART_DAEMON").as_deref() == Ok("1"),
            show_quit_dialog_on_start: false,
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
    pub snapshots_error: Option<String>,
    pub run_detail: Option<RunDetailDto>,
    pub state_dirty: bool,
    pub progress_dirty: bool,
    pub listener_started: bool,
    pub pending_blocked: Option<(String, BlockedReason)>,
    pub pending_show_window: bool,
    pub pending_quit: Option<(bool, String)>,
    pub maintenance_preview: Option<(String, Result<String, String>)>,
    pub signal_conn: Option<zbus::Connection>,
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
            snapshots_error: None,
            run_detail: None,
            state_dirty: false,
            progress_dirty: false,
            listener_started: false,
            pending_blocked: None,
            pending_show_window: false,
            pending_quit: None,
            maintenance_preview: None,
            signal_conn: None,
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

fn connect_client(shared: &SharedGuiState) -> Result<KesticClient, String> {
    KesticClient::connect(&shared.launch.bus_name, &shared.launch.object_path)
        .map_err(|e| e.to_string())
}

pub fn make_client() -> Result<KesticClient, String> {
    let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
    connect_client(&guard)
}

/// Ensures the daemon is running (spawning `kestic daemon` without `--autostart` when needed, `M1-S21`)
/// and starts the background D-Bus signal listener.
pub fn initialize_and_sync() {
    let launch = {
        let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let start_listener = !guard.listener_started;
        guard.listener_started = true;
        (guard.launch.clone(), start_listener)
    };

    if launch.1 {
        spawn_signal_listener(launch.0.bus_name.clone(), launch.0.object_path.clone());
    }

    ensure_daemon_and_refresh();
}

pub fn ensure_daemon_and_refresh() {
    let launch = {
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        guard.launch.clone()
    };

    let client = match KesticClient::connect(&launch.bus_name, &launch.object_path) {
        Ok(c) => c,
        Err(err) => {
            let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard.daemon_reachable = false;
            guard.daemon_error = Some(err.to_string());
            guard.state_dirty = true;
            return;
        }
    };

    let mut owned = client.is_bus_owned().unwrap_or(false);
    if !owned && !launch.no_autostart_daemon {
        let exe = launch
            .daemon_bin
            .clone()
            .unwrap_or_else(|| env::current_exe().unwrap_or_else(|_| PathBuf::from("kestic")));
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
            Ok(_child) => {
                let deadline = Instant::now() + Duration::from_millis(4500);
                while Instant::now() < deadline {
                    if client.is_bus_owned().unwrap_or(false) {
                        owned = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }
            Err(err) => {
                let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                guard.daemon_reachable = false;
                guard.daemon_error = Some(err.to_string());
                guard.state_dirty = true;
                return;
            }
        }
    }

    if !owned {
        let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        guard.daemon_reachable = false;
        guard.daemon_error = Some(i18n("The background service did not start.").to_string());
        guard.state_dirty = true;
        return;
    }

    refresh_state_now();
}

pub fn refresh_state_now() {
    let client = match make_client() {
        Ok(c) => c,
        Err(err) => {
            let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard.daemon_reachable = false;
            guard.daemon_error = Some(err);
            guard.state_dirty = true;
            return;
        }
    };

    match client.get_state() {
        Ok(state) => {
            let (sig_conn_opt, bus_name, object_path, should_register) = {
                let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                let was_reachable = guard.daemon_reachable;
                guard.daemon_reachable = true;
                guard.daemon_error = None;
                let active_ids: Vec<String> = state
                    .backups
                    .iter()
                    .filter_map(|b| b.active_run.as_ref().map(|r| r.id.as_str().to_string()))
                    .collect();
                guard.active_progress.retain(|k, _| active_ids.contains(k));
                if let Some(p) = &state.overall.active_run_progress {
                    guard
                        .active_progress
                        .entry(p.run_id.clone())
                        .or_insert_with(|| p.clone());
                }
                guard.state = Some(state);
                guard.state_dirty = true;
                (
                    guard.signal_conn.clone(),
                    guard.launch.bus_name.clone(),
                    guard.launch.object_path.clone(),
                    !was_reachable,
                )
            };
            if should_register && let Some(sig_conn) = sig_conn_opt {
                let _ =
                    KesticClient::from_connection(sig_conn, &bus_name, &object_path).register_gui();
            }
        }
        Err(err) => {
            let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
            guard.daemon_reachable = false;
            guard.daemon_error = Some(err.to_string());
            guard.state_dirty = true;
        }
    }
}

async fn register_gui_async(conn: &zbus::Connection, bus_name: &str, object_path: &str) {
    if let Ok(proxy) = zbus::Proxy::new(conn, bus_name, object_path, KESTIC_INTERFACE_V1).await {
        let _ = proxy.call::<_, _, ()>("RegisterGui", &()).await;
    }
}

fn spawn_signal_listener(bus_name: String, object_path: String) {
    thread::spawn(move || {
        zbus::block_on(async move {
            let Ok(conn) = zbus::Connection::session().await else {
                return;
            };
            let Ok(rule) = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface(KESTIC_INTERFACE_V1)
                .path(object_path.as_str())
                .build()
            else {
                return;
            };
            let Ok(mut stream) = zbus::MessageStream::for_match_rule(rule, &conn, None).await
            else {
                return;
            };
            {
                let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                guard.signal_conn = Some(conn.clone());
            }
            register_gui_async(&conn, &bus_name, &object_path).await;

            while let Some(msg_res) = stream.next().await {
                let Ok(msg) = msg_res else {
                    continue;
                };
                let header = msg.header();
                let member = header.member().map(|m| m.as_str().to_string());
                match member.as_deref() {
                    Some("StatusChanged") | Some("RunUpdated") => {
                        let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                        guard.state_dirty = true;
                    }
                    Some("RunProgress") => {
                        if let Ok((run_id, pct, bytes_done, total_bytes, speed_bps, secs_rem)) = msg
                            .body()
                            .deserialize::<(String, f64, u64, u64, u64, u64)>()
                        {
                            let mut guard =
                                shared_state().lock().unwrap_or_else(|p| p.into_inner());
                            let backup_name = guard
                                .state
                                .as_ref()
                                .and_then(|st| {
                                    st.backups.iter().find_map(|b| {
                                        b.active_run
                                            .as_ref()
                                            .filter(|r| r.id.as_str() == run_id)
                                            .map(|_| b.backup.name.clone())
                                    })
                                })
                                .unwrap_or_default();
                            let backup_id = guard.state.as_ref().and_then(|st| {
                                st.backups.iter().find_map(|b| {
                                    b.active_run
                                        .as_ref()
                                        .filter(|r| r.id.as_str() == run_id)
                                        .map(|_| b.backup.id.as_str().to_string())
                                })
                            });
                            guard.active_progress.insert(
                                run_id.clone(),
                                ActiveRunProgressDto {
                                    run_id,
                                    backup_id,
                                    backup_name,
                                    percent_done: pct,
                                    bytes_done,
                                    total_bytes,
                                    speed_bps,
                                    seconds_remaining: secs_rem,
                                },
                            );
                            guard.progress_dirty = true;
                        }
                    }
                    Some("ManualRunBlocked") => {
                        // A reason code this GUI does not know is ignored, never guessed.
                        if let Ok((backup_id, code)) = msg.body().deserialize::<(String, String)>()
                            && let Some(reason) = BlockedReason::parse(&code)
                        {
                            let mut guard =
                                shared_state().lock().unwrap_or_else(|p| p.into_inner());
                            guard.pending_blocked = Some((backup_id, reason));
                            guard.state_dirty = true;
                        }
                    }
                    Some("ShowWindowRequested") => {
                        let mut guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
                        guard.pending_show_window = true;
                    }
                    Some("QuitConfirmationRequested") => {
                        if let Ok((has_running, name)) = msg.body().deserialize::<(bool, String)>()
                        {
                            let mut guard =
                                shared_state().lock().unwrap_or_else(|p| p.into_inner());
                            guard.pending_quit = Some((has_running, name));
                        }
                    }
                    _ => {}
                }
            }
        });
    });
}

#[derive(serde::Deserialize)]
struct DestinationFieldsInput {
    #[serde(default)]
    id: Option<String>,
    kind: String,
    name: String,
    location: String,
    #[serde(default)]
    username: String,
}

pub fn test_destination_connection(
    fields_json: &str,
    mode: &str,
    repo_pw: &str,
    rest_pw: &str,
) -> ConnectionTestResultDto {
    let dest = match build_destination(None, fields_json) {
        Ok(d) => d,
        Err(err) => {
            return ConnectionTestResultDto {
                ok: false,
                error_kind: Some(ConnectionTestCode::InvalidInput),
                detail: if err.is_empty() {
                    i18n("Please fill in all required fields.").to_string()
                } else {
                    err
                },
            };
        }
    };
    let client = match make_client() {
        Ok(c) => c,
        Err(err) => {
            return ConnectionTestResultDto {
                ok: false,
                error_kind: Some(ConnectionTestCode::DaemonUnreachable),
                detail: err,
            };
        }
    };
    match client.test_destination_connection(&dest, mode, repo_pw, rest_pw) {
        Ok(res) => res,
        // Already translated: the text of the D-Bus error name.
        Err(err) => ConnectionTestResultDto {
            ok: false,
            error_kind: Some(ConnectionTestCode::Internal),
            detail: dbus_error_text(&err),
        },
    }
}

pub fn build_destination(
    existing_id: Option<&str>,
    fields_json: &str,
) -> Result<Destination, String> {
    let fields: DestinationFieldsInput =
        serde_json::from_str(fields_json).map_err(|e| e.to_string())?;
    let trimmed_loc = fields.location.trim();
    let trimmed_name = fields.name.trim();
    let trimmed_user = fields.username.trim();
    let dest_kind = match fields.kind.as_str() {
        "rest" => DestinationKind::Rest {
            url: trimmed_loc.to_string(),
            username: (!trimmed_user.is_empty()).then(|| trimmed_user.to_string()),
        },
        _ => DestinationKind::Local {
            path: PathBuf::from(trimmed_loc),
        },
    };
    let id_opt = existing_id
        .filter(|s| !s.is_empty())
        .or_else(|| fields.id.as_deref().filter(|s| !s.is_empty()));
    let dest = if let Some(raw_id) = id_opt {
        let id = DestinationId::parse(raw_id).map_err(|e| e.to_string())?;
        Destination::with_id(id, trimmed_name, dest_kind).map_err(|e| e.to_string())?
    } else {
        Destination::new(trimmed_name, dest_kind).map_err(|e| e.to_string())?
    };
    Ok(dest)
}

/// Kind of the cached destination `dest_id`, if the daemon state knows it.
pub fn cached_destination_kind(dest_id: &str) -> Option<DestinationKind> {
    let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
    guard.state.as_ref().and_then(|s| {
        s.destinations
            .iter()
            .find(|d| d.destination.id.as_str() == dest_id.trim())
            .map(|d| d.destination.kind.clone())
    })
}

/// New backup as the wizard creates it; on a local destination it has no network run
/// conditions (decision 2026-11-20).
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

pub fn clone_updated_backup(
    existing: &Backup,
    name: &str,
    source_path: &str,
    dest_id: &str,
    excludes_text: &str,
    exclude_caches: bool,
    detected_source: Option<Source>,
) -> Result<Backup, String> {
    let mut updated = existing.clone();
    updated.rename(name.trim()).map_err(|e| e.to_string())?;
    let trimmed_source = source_path.trim();
    if trimmed_source != existing.source.path.to_string_lossy().as_ref() {
        let new_src =
            detected_source.unwrap_or_else(|| Source::folder(PathBuf::from(trimmed_source)));
        updated.set_source(new_src).map_err(|e| e.to_string())?;
    } else if let Some(det) = detected_source {
        updated.set_source(det).map_err(|e| e.to_string())?;
    }
    if !dest_id.trim().is_empty() {
        let d_id = DestinationId::parse(dest_id.trim()).map_err(|e| e.to_string())?;
        updated.reassign_destination(d_id);
    }
    let mut excludes = existing.excludes.clone();
    excludes.lines = excludes_text.lines().map(|l| l.to_string()).collect();
    excludes.exclude_caches = exclude_caches;
    updated.excludes = excludes;
    updated.validate().map_err(|e| e.to_string())?;
    Ok(updated)
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
/// destination (decision 2026-11-20).
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
            return i18n("Standard (7 daily, 4 weekly, 12 monthly)").to_string();
        }
        RetentionPreset::Economical => {
            return i18n("Economical (7 daily, 4 weekly)").to_string();
        }
        RetentionPreset::Long => {
            return i18n("Long (7 daily, 4 weekly, 12 monthly, all yearly)").to_string();
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
    if let Some(w) = &policy.keep_within {
        parts.push(i18n("all within %1").subs(&[w]));
    }
    if let Some(w) = &policy.keep_within_hourly {
        parts.push(i18n("hourly within %1").subs(&[w]));
    }
    if let Some(w) = &policy.keep_within_daily {
        parts.push(i18n("daily within %1").subs(&[w]));
    }
    if let Some(w) = &policy.keep_within_weekly {
        parts.push(i18n("weekly within %1").subs(&[w]));
    }
    if let Some(w) = &policy.keep_within_monthly {
        parts.push(i18n("monthly within %1").subs(&[w]));
    }
    if let Some(w) = &policy.keep_within_yearly {
        parts.push(i18n("yearly within %1").subs(&[w]));
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
    if folder_count == 1 {
        parts.push(i18n("1 folder").to_string());
    } else if folder_count > 1 {
        parts.push(i18n("%1 folders").subs(&[&folder_count.to_string()]));
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
    if expert_count == 1 {
        parts.push(i18n("1 expert rule").to_string());
    } else if expert_count > 1 {
        parts.push(i18n("%1 expert rules").subs(&[&expert_count.to_string()]));
    }
    if !excludes.exclude_if_present.is_empty() {
        parts.push(i18n("marker: %1").subs(&[&excludes.exclude_if_present.join(", ")]));
    }
    if parts.is_empty() {
        i18n("None").to_string()
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
                i18n("Drive (%1)").subs(&[lbl.trim()])
            } else {
                i18n("Drive").to_string()
            }
        }
        SourceKind::Folder => i18n("Folder").to_string(),
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
        _ => Ok(ResticProvider::System),
    }
}

pub fn format_relative_time(ts: OffsetDateTime, now: OffsetDateTime) -> String {
    let diff = (now - ts).whole_seconds();
    if diff < 60 {
        return i18n("just now").to_string();
    }
    let mins = diff / 60;
    if mins == 1 {
        return i18n("1 minute ago").to_string();
    }
    if mins < 60 {
        return i18n("%1 minutes ago").subs(&[&mins.to_string()]);
    }
    let hours = mins / 60;
    if hours == 1 {
        return i18n("1 hour ago").to_string();
    }
    if hours < 24 {
        return i18n("%1 hours ago").subs(&[&hours.to_string()]);
    }
    let days = hours / 24;
    if days == 1 {
        return i18n("1 day ago").to_string();
    }
    i18n("%1 days ago").subs(&[&days.to_string()])
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

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn weekday_short_label(w: Weekday) -> String {
    match w {
        Weekday::Monday => i18n("Mon").to_string(),
        Weekday::Tuesday => i18n("Tue").to_string(),
        Weekday::Wednesday => i18n("Wed").to_string(),
        Weekday::Thursday => i18n("Thu").to_string(),
        Weekday::Friday => i18n("Fri").to_string(),
        Weekday::Saturday => i18n("Sat").to_string(),
        Weekday::Sunday => i18n("Sun").to_string(),
    }
}

fn weekday_full_label(w: Weekday) -> String {
    match w {
        Weekday::Monday => i18n("Monday").to_string(),
        Weekday::Tuesday => i18n("Tuesday").to_string(),
        Weekday::Wednesday => i18n("Wednesday").to_string(),
        Weekday::Thursday => i18n("Thursday").to_string(),
        Weekday::Friday => i18n("Friday").to_string(),
        Weekday::Saturday => i18n("Saturday").to_string(),
        Weekday::Sunday => i18n("Sunday").to_string(),
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
        Schedule::ManualOnly => i18n("Manual only").to_string(),
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
                .map(weekday_full_label)
                .unwrap_or_else(|_| weekday.to_string());
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
            i18n("Every %1 days from %2 at %3").subs(&[
                &interval_days.to_string(),
                start_date,
                &hhmm,
            ])
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
        MaintenanceEntrySchedule::ManualOnly => i18n("Manual only").to_string(),
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
        .map(format_time_short)
        .unwrap_or_else(|| "–".to_string());
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
/// availability only, e.g. «Manual only · Destination unreachable» (decision 2026-11-19).
fn unavailable_status(
    schedule: &Schedule,
    availability: &BackupAvailabilityDto,
) -> (String, String, String) {
    if matches!(schedule, Schedule::ManualOnly) {
        let text = i18n("%1 · %2").subs(&[
            &i18n("Manual only").to_string(),
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
        let pct = active_progress
            .map(|p| p.percent_done.round().clamp(0.0, 100.0) as u32)
            .unwrap_or(0);
        let text = i18n("Backing up · %1%").subs(&[&pct.to_string()]);
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
                    i18n("Backed up, %1 files unreadable").subs(&[&count.to_string()])
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
                    i18n("Backing up · %1%").subs(&["0"]),
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
        RunStatus::Interrupted => (i18n("Interrupted").to_string(), "state-offline", "text"),
    }
}

fn to_local(ts: OffsetDateTime) -> OffsetDateTime {
    let offset_secs = crate::smoke::ffi::local_offset_seconds(ts.unix_timestamp());
    let offset = time::UtcOffset::from_whole_seconds(offset_secs).unwrap_or(ts.offset());
    ts.to_offset(offset)
}

pub fn format_day_group(ts: OffsetDateTime, now: OffsetDateTime) -> String {
    let date = to_local(ts).date();
    let today = to_local(now).date();
    if date == today {
        return i18n("Today").to_string();
    }
    if let Some(yesterday) = today.previous_day()
        && date == yesterday
    {
        return i18n("Yesterday").to_string();
    }
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month() as u8,
        date.day()
    )
}

pub fn format_time_short(ts: OffsetDateTime) -> String {
    let local = to_local(ts);
    format!("{:02}:{:02}", local.hour(), local.minute())
}

pub fn format_datetime_display(ts: OffsetDateTime) -> String {
    let local = to_local(ts);
    let date = local.date();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        date.year(),
        date.month() as u8,
        date.day(),
        local.hour(),
        local.minute()
    )
}

pub fn current_now() -> OffsetDateTime {
    SystemClock.now_utc()
}

pub fn daemon_now(state: &DaemonStateDto) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(state.overall.now_unix).unwrap_or_else(|_| current_now())
}

pub fn cached_daemon_now() -> OffsetDateTime {
    let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
    guard
        .state
        .as_ref()
        .map(daemon_now)
        .unwrap_or_else(current_now)
}

pub fn cached_today_iso_date() -> String {
    let date = to_local(cached_daemon_now()).date();
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month() as u8,
        date.day()
    )
}

/// Title of a run in the history; a backup that no longer exists is named as deleted, never
/// by its internal ID.
pub fn format_run_kind_name(run: &Run, backup_name: Option<&str>, dest_name: &str) -> String {
    match &run.kind {
        RunKind::Backup { .. } => backup_name
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| i18n("Deleted backup").to_string()),
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

/// Localized reason for a skipped backup of «Back Up All Now» (FR-13b).
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
    if queued == 0 {
        return Some(i18np(
            "%1 backup skipped: %2",
            "%1 backups skipped: %2",
            skipped,
            &[&details],
        ));
    }
    let skipped_text = i18np(
        "%1 backup skipped: %2",
        "%1 backups skipped: %2",
        skipped,
        &[&details],
    );
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
