// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pure deterministic scheduler, DST/timezone resolution, overdue tracking, pause windows,
//! backoff state machine, and per-destination run planner.

use crate::history::{BackupRuntimeState, DestinationMaintenanceState, PauseState};
use crate::model::{
    Backup, BackupId, Destination, DestinationId, MaintenanceEntrySchedule, NthWeekday,
    PendingReason, RunConditions, Schedule, Weekday,
};
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::{AmbiguousOffset, TimeZone};
use kestic_i18n::codes::PendingReasonCode;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use time::OffsetDateTime;

/// Backoff delays (in seconds) after consecutive transient failures.
pub const BACKOFF_DELAYS_SECS: [i64; 3] = [5 * 60, 15 * 60, 60 * 60];

/// Injectable timezone specification so unit tests can evaluate IANA DST rules
/// (e.g. `Europe/Zurich`) deterministically without mutating process environment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TimezoneSpec {
    #[default]
    System,
    Named(String),
}

impl TimezoneSpec {
    pub fn resolve(&self) -> TimeZone {
        match self {
            Self::System => TimeZone::system(),
            Self::Named(name) => TimeZone::get(name).unwrap_or(TimeZone::UTC),
        }
    }
}

fn to_jiff_ts(dt: OffsetDateTime) -> Timestamp {
    Timestamp::from_second(dt.unix_timestamp()).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn to_offset_dt(ts: Timestamp) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(ts.as_second()).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// Resolves a local `(date, hour, minute)` into a UTC [`OffsetDateTime`]:
/// - Unambiguous: exact instant.
/// - DST fold (autumn fallback): first occurrence only.
/// - DST gap (spring forward): next valid minute at the transition (e.g. `02:30` -> `03:00`).
pub fn resolve_local_slot(date: Date, hour: u8, minute: u8, tz: &TimeZone) -> OffsetDateTime {
    let civil = date.at(hour as i8, minute as i8, 0, 0);
    let amb = tz.to_ambiguous_timestamp(civil);
    let ts = match amb.offset() {
        AmbiguousOffset::Unambiguous { .. } => amb.compatible().unwrap_or(Timestamp::UNIX_EPOCH),
        AmbiguousOffset::Fold { .. } => amb.earlier().unwrap_or(Timestamp::UNIX_EPOCH),
        AmbiguousOffset::Gap { .. } => {
            if let Ok(before) = amb.earlier()
                && let Some(transition) = tz.following(before).next()
            {
                transition.timestamp()
            } else {
                amb.compatible().unwrap_or(Timestamp::UNIX_EPOCH)
            }
        }
    };
    to_offset_dt(ts)
}

/// Resolves `00:00` local time on `date` (handling rare midnight DST gaps).
pub fn resolve_local_midnight(date: Date, tz: &TimeZone) -> OffsetDateTime {
    resolve_local_slot(date, 0, 0, tz)
}

/// Formats `dt` as `HH:MM` in the resolved `tz_spec`.
pub fn format_local_hhmm(dt: OffsetDateTime, tz_spec: &TimezoneSpec) -> String {
    let local = tz_spec.resolve().to_datetime(to_jiff_ts(dt));
    format!("{:02}:{:02}", local.hour(), local.minute())
}

/// Formats `dt` in `tz_spec`, omitting the date when on the same local calendar day as `now`.
pub fn format_local_slot_hint(
    dt: OffsetDateTime,
    now: OffsetDateTime,
    tz_spec: &TimezoneSpec,
) -> String {
    let tz = tz_spec.resolve();
    let local_dt = tz.to_datetime(to_jiff_ts(dt));
    let local_now = tz.to_datetime(to_jiff_ts(now));
    if local_dt.date() == local_now.date() {
        format!("{:02}:{:02}", local_dt.hour(), local_dt.minute())
    } else {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            local_dt.year(),
            local_dt.month(),
            local_dt.day(),
            local_dt.hour(),
            local_dt.minute()
        )
    }
}

fn schedule_matches_date(schedule: &Schedule, date: Date) -> Option<(u8, u8)> {
    match schedule {
        Schedule::ManualOnly => None,
        Schedule::Daily { hour, minute } => Some((*hour, *minute)),
        Schedule::SpecificWeekdays {
            weekdays,
            hour,
            minute,
        } => {
            let wd = Weekday::from_jiff(date.weekday());
            weekdays.contains(&wd).then_some((*hour, *minute))
        }
        Schedule::Weekly {
            weekday,
            hour,
            minute,
        } => {
            let target = Weekday::from_iso_number(*weekday).ok()?;
            (Weekday::from_jiff(date.weekday()) == target).then_some((*hour, *minute))
        }
        Schedule::EveryNDays {
            interval_days,
            start_date,
            hour,
            minute,
        } => {
            let start = start_date.parse::<Date>().ok()?;
            if date < start || *interval_days == 0 {
                return None;
            }
            let delta_days = date.since(start).ok()?.get_days() as i64;
            let step = i64::from(*interval_days);
            (delta_days >= 0 && delta_days % step == 0).then_some((*hour, *minute))
        }
    }
}

fn nth_weekday_of_month(year: i16, month: i8, nth: NthWeekday, target: Weekday) -> Option<Date> {
    let first = Date::new(year, month, 1).ok()?;
    let days_in_month = first.days_in_month();
    let mut matches = Vec::new();
    for day in 1..=days_in_month {
        if let Ok(d) = Date::new(year, month, day)
            && Weekday::from_jiff(d.weekday()) == target
        {
            matches.push(d);
        }
    }
    match nth {
        NthWeekday::First => matches.first().copied(),
        NthWeekday::Second => matches.get(1).copied(),
        NthWeekday::Third => matches.get(2).copied(),
        NthWeekday::Fourth => matches.get(3).copied(),
        NthWeekday::Last => matches.last().copied(),
    }
}

fn maintenance_matches_date(schedule: &MaintenanceEntrySchedule, date: Date) -> Option<(u8, u8)> {
    match schedule {
        MaintenanceEntrySchedule::ManualOnly => None,
        MaintenanceEntrySchedule::Daily { hour, minute } => Some((*hour, *minute)),
        MaintenanceEntrySchedule::Weekly {
            weekday,
            hour,
            minute,
        } => (Weekday::from_jiff(date.weekday()) == *weekday).then_some((*hour, *minute)),
        MaintenanceEntrySchedule::Monthly {
            nth,
            weekday,
            hour,
            minute,
        } => {
            let expected = nth_weekday_of_month(date.year(), date.month(), *nth, *weekday)?;
            (date == expected).then_some((*hour, *minute))
        }
    }
}

/// Local dates from `after_exclusive` to `up_to_inclusive` (padded by a day on both sides for
/// zone offsets) on which `matches` yields a time, resolved to the slots in
/// `(after_exclusive, up_to_inclusive]`, sorted and deduplicated.
fn slots_between(
    after_exclusive: OffsetDateTime,
    up_to_inclusive: OffsetDateTime,
    tz_spec: &TimezoneSpec,
    matches: impl Fn(Date) -> Option<(u8, u8)>,
) -> Vec<OffsetDateTime> {
    if up_to_inclusive <= after_exclusive {
        return Vec::new();
    }

    let tz = tz_spec.resolve();
    let start_date = tz.to_datetime(to_jiff_ts(after_exclusive)).date();
    let end_date_base = tz.to_datetime(to_jiff_ts(up_to_inclusive)).date();
    let mut cur_date = start_date.yesterday().unwrap_or(start_date);
    let end_date = end_date_base.tomorrow().unwrap_or(end_date_base);

    let mut slots = Vec::new();
    while cur_date <= end_date {
        if let Some((hour, minute)) = matches(cur_date) {
            let slot = resolve_local_slot(cur_date, hour, minute, &tz);
            if slot > after_exclusive && slot <= up_to_inclusive {
                slots.push(slot);
            }
        }
        let Some(next) = cur_date.tomorrow().ok() else {
            break;
        };
        cur_date = next;
    }
    slots.sort_unstable();
    slots.dedup();
    slots
}

/// Returns all scheduled backup timestamps strictly after `after_exclusive` and `<= up_to_inclusive`,
/// sorted chronologically.
pub fn backup_slots_between(
    schedule: &Schedule,
    after_exclusive: OffsetDateTime,
    up_to_inclusive: OffsetDateTime,
    tz_spec: &TimezoneSpec,
) -> Vec<OffsetDateTime> {
    if matches!(schedule, Schedule::ManualOnly) {
        return Vec::new();
    }
    slots_between(after_exclusive, up_to_inclusive, tz_spec, |date| {
        schedule_matches_date(schedule, date)
    })
}

/// Returns the next `count` scheduled slots strictly after `after_exclusive` (for the schedule preview).
pub fn next_backup_slots(
    schedule: &Schedule,
    after_exclusive: OffsetDateTime,
    count: usize,
    tz_spec: &TimezoneSpec,
) -> Vec<OffsetDateTime> {
    if count == 0 || matches!(schedule, Schedule::ManualOnly) {
        return Vec::new();
    }

    let tz = tz_spec.resolve();
    let start_date = tz.to_datetime(to_jiff_ts(after_exclusive)).date();
    let mut cur_date = start_date.yesterday().unwrap_or(start_date);
    let mut slots = Vec::with_capacity(count);
    // Search up to 400 days ahead (covers EveryNDays up to 365 days).
    for _ in 0..800 {
        if let Some((hour, minute)) = schedule_matches_date(schedule, cur_date) {
            let slot = resolve_local_slot(cur_date, hour, minute, &tz);
            if slot > after_exclusive {
                slots.push(slot);
                if slots.len() >= count {
                    break;
                }
            }
        }
        let Some(next) = cur_date.tomorrow().ok() else {
            break;
        };
        cur_date = next;
    }
    slots
}

/// Returns all maintenance timestamps strictly after `after_exclusive` and `<= up_to_inclusive`.
pub fn maintenance_slots_between(
    schedule: &MaintenanceEntrySchedule,
    after_exclusive: OffsetDateTime,
    up_to_inclusive: OffsetDateTime,
    tz_spec: &TimezoneSpec,
) -> Vec<OffsetDateTime> {
    if matches!(schedule, MaintenanceEntrySchedule::ManualOnly) {
        return Vec::new();
    }
    slots_between(after_exclusive, up_to_inclusive, tz_spec, |date| {
        maintenance_matches_date(schedule, date)
    })
}

/// Returns the next scheduled maintenance slot strictly after `after_exclusive`.
pub fn next_maintenance_slot(
    schedule: &MaintenanceEntrySchedule,
    after_exclusive: OffsetDateTime,
    tz_spec: &TimezoneSpec,
) -> Option<OffsetDateTime> {
    if matches!(schedule, MaintenanceEntrySchedule::ManualOnly) {
        return None;
    }
    let tz = tz_spec.resolve();
    let start_date = tz.to_datetime(to_jiff_ts(after_exclusive)).date();
    let mut cur_date = start_date.yesterday().unwrap_or(start_date);
    for _ in 0..400 {
        if let Some((hour, minute)) = maintenance_matches_date(schedule, cur_date) {
            let slot = resolve_local_slot(cur_date, hour, minute, &tz);
            if slot > after_exclusive {
                return Some(slot);
            }
        }
        cur_date = cur_date.tomorrow().ok()?;
    }
    None
}

/// Maximum supported finite pause duration in hours (10 years).
pub const MAX_PAUSE_HOURS: u32 = 24 * 365 * 10;

/// Preset durations for pausing schedules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PauseDuration {
    Hours { hours: u32 },
    UntilTomorrow,
    Indefinite,
}

/// Validates a [`PauseDuration`] before persisting or passing it to [`compute_pause_until`].
pub fn validate_pause_duration(duration: PauseDuration) -> Result<(), crate::model::ModelError> {
    if let PauseDuration::Hours { hours } = duration
        && (hours == 0 || hours > MAX_PAUSE_HOURS)
    {
        return Err(crate::model::ModelError::InvalidBackup(format!(
            "pause hours must be between 1 and {MAX_PAUSE_HOURS}, got {hours}"
        )));
    }
    Ok(())
}

/// Computes `paused_until` for a pause started at `now`.
///
/// For `PauseDuration::UntilTomorrow`, skips remaining slots today and ends at the earliest
/// scheduled slot on the next calendar day (e.g. tomorrow `08:00` if daily `08:00`), or `00:00`
/// local time tomorrow if no slot exists tomorrow.
pub fn compute_pause_until(
    now: OffsetDateTime,
    duration: PauseDuration,
    schedules: &[&Schedule],
    tz_spec: &TimezoneSpec,
) -> Option<OffsetDateTime> {
    match duration {
        PauseDuration::Hours { hours } => {
            let clamped = hours.min(MAX_PAUSE_HOURS);
            let secs = i64::from(clamped).checked_mul(3600).unwrap_or(i64::MAX);
            Some(
                now.checked_add(time::Duration::seconds(secs))
                    .unwrap_or(now),
            )
        }
        PauseDuration::Indefinite => None,
        PauseDuration::UntilTomorrow => {
            let tz = tz_spec.resolve();
            let local_today = tz.to_datetime(to_jiff_ts(now)).date();
            let tomorrow = local_today.tomorrow().unwrap_or(local_today);
            let mut earliest: Option<OffsetDateTime> = None;
            for sched in schedules {
                if let Some((hour, minute)) = schedule_matches_date(sched, tomorrow) {
                    let slot = resolve_local_slot(tomorrow, hour, minute, &tz);
                    earliest = Some(match earliest {
                        Some(cur) if cur <= slot => cur,
                        _ => slot,
                    });
                }
            }
            Some(earliest.unwrap_or_else(|| resolve_local_midnight(tomorrow, &tz)))
        }
    }
}

/// Returns whether `pause` is currently active at `now`.
pub fn is_pause_active(pause: Option<&PauseState>, now: OffsetDateTime) -> bool {
    let Some(p) = pause else {
        return false;
    };
    match p.paused_until {
        None => true,
        Some(until) => now < until,
    }
}

/// Computes the next retry timestamp from the end of a failed transient attempt.
///
/// `attempt` is 1-based after failure: 1 -> +5m, 2 -> +15m, 3 -> +60m, >=4 -> `None` (wait next regular slot).
pub fn compute_backoff_until(
    attempt: u32,
    attempt_finished_at: OffsetDateTime,
) -> Option<OffsetDateTime> {
    let idx = attempt.checked_sub(1)? as usize;
    let delay_secs = *BACKOFF_DELAYS_SECS.get(idx)?;
    Some(attempt_finished_at + time::Duration::seconds(delay_secs))
}

/// `backoff_attempt` value meaning "all retries used, wait for the next regular slot strictly
/// after `last_failure_at`"; see [`evaluate_backup_schedule`].
pub const BACKOFF_ATTEMPT_WAIT_FOR_NEXT_SLOT: u32 = BACKOFF_DELAYS_SECS.len() as u32 + 1;

/// Records a transient failure finished at `now` on a backup's backoff state (5/15/60 min).
pub fn record_backup_backoff_failure(
    rs: &mut BackupRuntimeState,
    now: OffsetDateTime,
    kind: Option<crate::history::BackoffKind>,
) {
    rs.backoff_attempt = rs.backoff_attempt.saturating_add(1);
    rs.last_failure_at = Some(now);
    rs.backoff_until = compute_backoff_until(rs.backoff_attempt, now);
    rs.backoff_kind = kind;
}

/// Anchor for backup catch-up: the later of the last success and the initial registration
/// baseline, `now` when neither exists.
pub fn catchup_anchor(
    last_success_at: Option<OffsetDateTime>,
    baseline_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> OffsetDateTime {
    match (last_success_at, baseline_at) {
        (Some(a), Some(b)) => a.max(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => now,
    }
}

/// Anchors from which due prune and check slots count: the last run of each kind, else the
/// registration baseline, else `now`.
pub fn maintenance_anchors(
    state: &DestinationMaintenanceState,
    now: OffsetDateTime,
) -> (OffsetDateTime, OffsetDateTime) {
    (
        state.last_prune_at.or(state.baseline_at).unwrap_or(now),
        state.last_check_at.or(state.baseline_at).unwrap_or(now),
    )
}

/// Forgets a backup's failures: no retry pending, the next regular slot starts normally.
pub fn clear_backup_backoff(rs: &mut BackupRuntimeState) {
    rs.backoff_attempt = 0;
    rs.backoff_until = None;
    rs.backoff_kind = None;
    rs.last_failure_at = None;
}

/// Marks the currently due slot of a backup as handled at `now` (e.g. the user cancelled the
/// run): no retry until the next regular slot strictly after `now`.
pub fn mark_backup_slot_handled(rs: &mut BackupRuntimeState, now: OffsetDateTime) {
    rs.backoff_attempt = BACKOFF_ATTEMPT_WAIT_FOR_NEXT_SLOT;
    rs.backoff_until = None;
    rs.backoff_kind = None;
    rs.last_failure_at = Some(now);
}

/// Records a failed maintenance attempt finished at `now` (backoff 5/15/60 min).
pub fn record_maintenance_backoff_failure(
    ds: &mut DestinationMaintenanceState,
    now: OffsetDateTime,
) {
    ds.maintenance_backoff_attempt = ds.maintenance_backoff_attempt.saturating_add(1);
    ds.last_maintenance_failure_at = Some(now);
    ds.maintenance_backoff_until = compute_backoff_until(ds.maintenance_backoff_attempt, now);
}

/// Forgets a destination's maintenance failures: no retry pending.
pub fn clear_maintenance_backoff(ds: &mut DestinationMaintenanceState) {
    ds.maintenance_backoff_attempt = 0;
    ds.maintenance_backoff_until = None;
    ds.last_maintenance_failure_at = None;
}

/// Marks the currently due maintenance work of a destination as handled at `now` (e.g. the user
/// cancelled the run): no retry until the next regular prune/check slot strictly after `now`.
pub fn mark_maintenance_slot_handled(ds: &mut DestinationMaintenanceState, now: OffsetDateTime) {
    ds.maintenance_backoff_attempt = BACKOFF_ATTEMPT_WAIT_FOR_NEXT_SLOT;
    ds.maintenance_backoff_until = None;
    ds.last_maintenance_failure_at = Some(now);
}

/// Live system condition snapshot from UPower / NetworkManager.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SystemEnvironment {
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
    pub network_connected: bool,
    pub network_metered: bool,
    pub active_networks: Vec<String>,
}

impl SystemEnvironment {
    /// Default connected AC environment (used when UPower/NM are absent in headless tests).
    pub fn desktop_connected() -> Self {
        Self {
            on_battery: false,
            battery_percent: None,
            network_connected: true,
            network_metered: false,
            active_networks: Vec::new(),
        }
    }
}

/// Granular reason a backup cannot start right now, ordered strictly by the precedence in `docs/spec/ui/zustaende.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UnmetPrecondition {
    SourceUnavailable,
    DestinationUnreachable,
    OnBattery,
    NoNetwork,
    MeteredNetwork,
    AllowedNetworkMismatch { allowed: Vec<String> },
    Queued,
    Backoff,
}

impl From<&UnmetPrecondition> for PendingReasonCode {
    fn from(item: &UnmetPrecondition) -> Self {
        match item {
            UnmetPrecondition::SourceUnavailable => Self::SourceUnavailable,
            UnmetPrecondition::DestinationUnreachable => Self::DestinationUnreachable,
            UnmetPrecondition::OnBattery => Self::OnBattery,
            UnmetPrecondition::NoNetwork => Self::NoNetwork,
            UnmetPrecondition::MeteredNetwork => Self::Metered,
            UnmetPrecondition::AllowedNetworkMismatch { .. } => Self::WrongNetwork,
            UnmetPrecondition::Queued => Self::Queue,
            UnmetPrecondition::Backoff => Self::Backoff,
        }
    }
}

impl UnmetPrecondition {
    /// Maps into the coarse [`PendingReason`] persisted on [`crate::model::Run`].
    pub fn to_pending_reason(&self) -> PendingReason {
        match self {
            Self::SourceUnavailable => PendingReason::SourceUnavailable,
            Self::DestinationUnreachable => PendingReason::DestinationUnreachable,
            Self::OnBattery
            | Self::NoNetwork
            | Self::MeteredNetwork
            | Self::AllowedNetworkMismatch { .. } => PendingReason::ConditionNotMet,
            Self::Queued => PendingReason::Queued,
            Self::Backoff => PendingReason::Backoff,
        }
    }

    pub fn sort_rank(&self) -> u8 {
        match self {
            Self::SourceUnavailable => 0,
            Self::DestinationUnreachable => 1,
            Self::OnBattery => 2,
            Self::NoNetwork => 3,
            Self::MeteredNetwork => 4,
            Self::AllowedNetworkMismatch { .. } => 5,
            Self::Queued => 6,
            Self::Backoff => 7,
        }
    }
}

/// Evaluates [`RunConditions`] against [`SystemEnvironment`] in priority order.
pub fn evaluate_run_conditions(
    conditions: &RunConditions,
    env: &SystemEnvironment,
) -> Vec<UnmetPrecondition> {
    let mut unmet = Vec::new();
    if conditions.require_ac_power && env.on_battery {
        unmet.push(UnmetPrecondition::OnBattery);
    }
    if conditions.require_network && !env.network_connected {
        unmet.push(UnmetPrecondition::NoNetwork);
    }
    if conditions.skip_on_metered_network && env.network_metered {
        unmet.push(UnmetPrecondition::MeteredNetwork);
    }
    if !conditions.allowed_networks.is_empty() {
        let matches_any = conditions.allowed_networks.iter().any(|allowed| {
            env.active_networks
                .iter()
                .any(|active| active.eq_ignore_ascii_case(allowed))
        });
        if !matches_any {
            unmet.push(UnmetPrecondition::AllowedNetworkMismatch {
                allowed: conditions.allowed_networks.clone(),
            });
        }
    }
    unmet
}

/// Combines source/destination availability, run conditions, queue, and backoff into the
/// ordered `(primary, all_reasons)` precedence list (see `docs/spec/ui/zustaende.md`).
pub fn collect_pending_preconditions(
    source_available: bool,
    destination_reachable: bool,
    conditions: &RunConditions,
    env: &SystemEnvironment,
    destination_busy: bool,
    in_backoff: bool,
) -> Vec<UnmetPrecondition> {
    let mut items = Vec::new();
    if !source_available {
        items.push(UnmetPrecondition::SourceUnavailable);
    }
    if !destination_reachable {
        items.push(UnmetPrecondition::DestinationUnreachable);
    }
    items.extend(evaluate_run_conditions(conditions, env));
    if destination_busy {
        items.push(UnmetPrecondition::Queued);
    }
    if in_backoff {
        items.push(UnmetPrecondition::Backoff);
    }
    items.sort_by_key(UnmetPrecondition::sort_rank);
    items
}

/// Evaluates whether destination maintenance (AC power and an unmetered connection) may run.
pub fn maintenance_conditions_met(env: &SystemEnvironment) -> bool {
    !env.on_battery && !env.network_metered
}

/// Inputs for evaluating a single backup's schedule state at `now`.
#[derive(Debug, Clone)]
pub struct BackupScheduleEvalInput<'a> {
    pub backup: &'a Backup,
    pub runtime_state: &'a BackupRuntimeState,
    pub last_success_at: Option<OffsetDateTime>,
    /// Optional time after which overdue counting restarted (e.g. end of last pause).
    pub overdue_anchor_at: Option<OffsetDateTime>,
    pub global_pause: Option<&'a PauseState>,
    pub global_overdue_threshold: u32,
    pub source_available: bool,
    pub destination_reachable: bool,
    pub destination_busy: bool,
    pub has_permanent_error: bool,
}

/// Result of evaluating a backup's schedule at `now`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupScheduleEvaluation {
    pub backup_id: BackupId,
    pub is_paused: bool,
    pub paused_since: Option<OffsetDateTime>,
    pub paused_until: Option<OffsetDateTime>,
    /// Latest scheduled slot `<= now` since last success/baseline (if any -> backup is due/catch-up).
    pub due_slot: Option<OffsetDateTime>,
    pub next_scheduled_at: Option<OffsetDateTime>,
    pub missed_slots_for_overdue: usize,
    pub is_overdue: bool,
    pub in_backoff: bool,
    pub backoff_exhausted_until_next_slot: bool,
    pub unmet_preconditions: Vec<UnmetPrecondition>,
    pub should_start_now: bool,
}

/// Evaluates due state, overdue status, pause status, backoff, and unmet preconditions for a backup.
pub fn evaluate_backup_schedule(
    input: &BackupScheduleEvalInput<'_>,
    env: &SystemEnvironment,
    now: OffsetDateTime,
    tz_spec: &TimezoneSpec,
) -> BackupScheduleEvaluation {
    let backup = input.backup;
    let rs = input.runtime_state;

    let backup_paused = is_pause_active(rs.pause.as_ref(), now);
    let global_paused = is_pause_active(input.global_pause, now);
    let is_paused = backup_paused || global_paused;

    let (paused_since, paused_until) = match (backup_paused, global_paused) {
        (true, _) => (
            rs.pause.as_ref().map(|p| p.paused_since),
            rs.pause.as_ref().and_then(|p| p.paused_until),
        ),
        (false, true) => (
            input.global_pause.map(|p| p.paused_since),
            input.global_pause.and_then(|p| p.paused_until),
        ),
        (false, false) => (None, None),
    };

    let next_scheduled_at = next_backup_slots(&backup.schedule, now, 1, tz_spec)
        .into_iter()
        .next();

    let catchup_anchor = catchup_anchor(input.last_success_at, rs.baseline_at, now);

    let due_slots = backup_slots_between(&backup.schedule, catchup_anchor, now, tz_spec);
    let due_slot = due_slots.last().copied();

    // Overdue anchor resets after an expired or resumed pause and after success.
    let mut overdue_anchor = catchup_anchor;
    if let Some(oa) = rs.overdue_anchor_at {
        overdue_anchor = overdue_anchor.max(oa);
    }
    if let Some(oa) = input.overdue_anchor_at {
        overdue_anchor = overdue_anchor.max(oa);
    }
    if let Some(ref p) = rs.pause
        && let Some(until) = p.paused_until
        && now >= until
    {
        overdue_anchor = overdue_anchor.max(until);
    }
    if let Some(gp) = input.global_pause
        && let Some(until) = gp.paused_until
        && now >= until
    {
        overdue_anchor = overdue_anchor.max(until);
    }

    let missed_slots_for_overdue = if is_paused || matches!(backup.schedule, Schedule::ManualOnly) {
        0
    } else {
        backup_slots_between(&backup.schedule, overdue_anchor, now, tz_spec).len()
    };

    let threshold = backup.effective_overdue_threshold(input.global_overdue_threshold) as usize;
    let is_overdue = !is_paused
        && !matches!(backup.schedule, Schedule::ManualOnly)
        && missed_slots_for_overdue >= threshold;

    // Backoff check: after 3 exhausted retries (`backoff_attempt >= 4`), wait until a newer
    // regular slot strictly after `last_failure_at`.
    let mut in_backoff = false;
    let mut backoff_exhausted = false;
    let newer_slot_arrived = match (due_slot, rs.last_failure_at) {
        (Some(slot), Some(failed_at)) => slot > failed_at,
        _ => false,
    };
    if !newer_slot_arrived {
        if let Some(backoff_until) = rs.backoff_until {
            if now < backoff_until {
                in_backoff = true;
            }
        } else if rs.backoff_attempt >= BACKOFF_ATTEMPT_WAIT_FOR_NEXT_SLOT && due_slot.is_some() {
            backoff_exhausted = true;
        }
    }

    let unmet = if due_slot.is_some() && !is_paused && !input.has_permanent_error {
        collect_pending_preconditions(
            input.source_available,
            input.destination_reachable,
            &backup.run_conditions,
            env,
            input.destination_busy,
            in_backoff,
        )
    } else {
        Vec::new()
    };

    let should_start_now = due_slot.is_some()
        && !is_paused
        && !input.has_permanent_error
        && !backoff_exhausted
        && unmet.is_empty();

    BackupScheduleEvaluation {
        backup_id: backup.id.clone(),
        is_paused,
        paused_since,
        paused_until,
        due_slot,
        next_scheduled_at,
        missed_slots_for_overdue,
        is_overdue,
        in_backoff,
        backoff_exhausted_until_next_slot: backoff_exhausted,
        unmet_preconditions: unmet,
        should_start_now,
    }
}

/// Maintenance need for a single destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationMaintenanceNeed {
    pub destination_id: DestinationId,
    pub has_pending_forget: bool,
    pub prune_due: bool,
    pub check_due: bool,
    pub next_prune_at: Option<OffsetDateTime>,
    pub next_check_at: Option<OffsetDateTime>,
    pub in_backoff: bool,
    pub due_now: bool,
}

/// Inputs for evaluating maintenance on a single destination.
#[derive(Debug, Clone)]
pub struct DestinationMaintenanceEvalInput<'a> {
    pub destination: &'a Destination,
    pub state: &'a DestinationMaintenanceState,
    pub has_pending_forget: bool,
    pub global_pause: Option<&'a PauseState>,
    pub destination_busy: bool,
    pub destination_reachable: bool,
}

/// Evaluates whether maintenance (`pending_forget` -> `prune` -> `check`) is due on `destination`.
pub fn evaluate_destination_maintenance(
    input: &DestinationMaintenanceEvalInput<'_>,
    env: &SystemEnvironment,
    now: OffsetDateTime,
    tz_spec: &TimezoneSpec,
) -> DestinationMaintenanceNeed {
    let destination = input.destination;
    let state = input.state;
    let paused = is_pause_active(input.global_pause, now);
    let (prune_anchor, check_anchor) = maintenance_anchors(state, now);

    let prune_slots =
        maintenance_slots_between(&destination.maintenance.prune, prune_anchor, now, tz_spec);
    let check_slots =
        maintenance_slots_between(&destination.maintenance.check, check_anchor, now, tz_spec);
    let prune_due = !prune_slots.is_empty();
    let check_due = !check_slots.is_empty();

    let next_prune_at = next_maintenance_slot(&destination.maintenance.prune, now, tz_spec);
    let next_check_at = next_maintenance_slot(&destination.maintenance.check, now, tz_spec);

    // An open `forget` alone does not start maintenance: it is caught up by the backup's next
    // run or by the next scheduled (or manual) maintenance run, which works it off first
    // (catch-up at the next backup or maintenance run).
    let any_work = prune_due || check_due;

    let latest_due_slot = prune_slots
        .last()
        .copied()
        .into_iter()
        .chain(check_slots.last().copied())
        .max();
    let newer_slot_arrived = match (latest_due_slot, state.last_maintenance_failure_at) {
        (Some(slot), Some(failed_at)) => slot > failed_at,
        _ => false,
    };

    let mut in_backoff = false;
    let mut backoff_exhausted = false;
    if any_work && !newer_slot_arrived {
        if let Some(backoff_until) = state.maintenance_backoff_until {
            if now < backoff_until {
                in_backoff = true;
            }
        } else if state.maintenance_backoff_attempt >= BACKOFF_ATTEMPT_WAIT_FOR_NEXT_SLOT {
            backoff_exhausted = true;
        }
    }

    let due_now = any_work
        && !paused
        && !input.destination_busy
        && input.destination_reachable
        && !in_backoff
        && !backoff_exhausted
        && maintenance_conditions_met(env);

    DestinationMaintenanceNeed {
        destination_id: destination.id.clone(),
        has_pending_forget: input.has_pending_forget,
        prune_due,
        check_due,
        next_prune_at,
        next_check_at,
        in_backoff,
        due_now,
    }
}

/// Planned action per destination for a scheduler tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestinationTickAction {
    StartBackup {
        backup_id: BackupId,
        destination_id: DestinationId,
    },
    StartMaintenance {
        destination_id: DestinationId,
        run_prune: bool,
        run_check: bool,
    },
}

/// Selects at most one startable action per idle destination:
/// - Backups with earliest due slot run before maintenance.
/// - Remaining due backups on a busy destination report `Queued`.
pub fn plan_destination_actions(
    evaluations: &[BackupScheduleEvaluation],
    maintenance_needs: &[DestinationMaintenanceNeed],
    busy_destinations: &HashSet<DestinationId>,
    backup_dest_map: &HashMap<BackupId, DestinationId>,
) -> Vec<DestinationTickAction> {
    let mut actions = Vec::new();
    let mut claimed_destinations = busy_destinations.clone();

    // Sort candidate backups deterministically: earliest due_slot first, then backup_id.
    let mut candidates: Vec<&BackupScheduleEvaluation> =
        evaluations.iter().filter(|e| e.should_start_now).collect();
    candidates.sort_by(|a, b| {
        a.due_slot
            .cmp(&b.due_slot)
            .then_with(|| a.backup_id.cmp(&b.backup_id))
    });

    for eval in candidates {
        let Some(dest_id) = backup_dest_map.get(&eval.backup_id) else {
            continue;
        };
        if claimed_destinations.contains(dest_id) {
            continue;
        }
        claimed_destinations.insert(dest_id.clone());
        actions.push(DestinationTickAction::StartBackup {
            backup_id: eval.backup_id.clone(),
            destination_id: dest_id.clone(),
        });
    }

    for maint in maintenance_needs {
        if !maint.due_now || claimed_destinations.contains(&maint.destination_id) {
            continue;
        }
        claimed_destinations.insert(maint.destination_id.clone());
        actions.push(DestinationTickAction::StartMaintenance {
            destination_id: maint.destination_id.clone(),
            run_prune: maint.prune_due,
            run_check: maint.check_due,
        });
    }

    actions
}
