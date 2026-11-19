// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! StatusNotifierItem tray integration via `ksni 0.3.7`
//! (`K1`, `NFR-11`, `M1-S11`, `AC-M1-11-1..5`, `AC-M1-23-4`, `tray-und-benachrichtigungen.md` §10.1).

use crate::dbus_api::{ActiveRunProgressDto, OverallStatusKind};
use crate::i18n::i18n;
use kestic_core::scheduler::PauseDuration;
use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem, SubMenu};
use std::sync::{Arc, mpsc};

/// Status line of the active run: «‹Backup›: backing up · n%» for backups, maintenance
/// wording («Maintaining ‹Destination›») for destination maintenance (no backup id).
pub(crate) fn active_run_line(active: &ActiveRunProgressDto) -> String {
    if active.backup_id.is_none() {
        return i18n("Maintaining %1").subs(&[&active.backup_name]);
    }
    let pct = active.percent_done.round().clamp(0.0, 100.0) as u32;
    i18n("%1: backing up · %2%").subs(&[&active.backup_name, &pct.to_string()])
}

/// Mapped StatusNotifierItem visibility state per `tray-und-benachrichtigungen.md` §10.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayStatus {
    Passive,
    Active,
    NeedsAttention,
}

impl TrayStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passive => "Passive",
            Self::Active => "Active",
            Self::NeedsAttention => "NeedsAttention",
        }
    }
}

impl From<TrayStatus> for ksni::Status {
    fn from(value: TrayStatus) -> Self {
        match value {
            TrayStatus::Passive => ksni::Status::Passive,
            TrayStatus::Active => ksni::Status::Active,
            TrayStatus::NeedsAttention => ksni::Status::NeedsAttention,
        }
    }
}

/// Action triggered by clicking the tray icon or a tray menu item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    StartBackup(String),
    StartAllBackups,
    CancelRun(String),
    PauseGlobal(PauseDuration),
    ResumeGlobal,
    OpenWindow,
    RequestQuit,
}

/// Per-backup menu entry rendered inside `"Back Up Now"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayBackupItem {
    pub backup_id: String,
    pub label: String,
    pub enabled: bool,
}

/// Computes `(TrayStatus, overlay_icon_name)` strictly following `tray-und-benachrichtigungen.md` §10.1:
/// - Blocking state or Error -> `NeedsAttention` + `"state-error"`
/// - Overdue -> `NeedsAttention` + `"alarm-symbolic"`
/// - Warning -> `Active` + `"state-warning"`
/// - Running (Backup/Restore/Maintenance) -> `Active` + `"state-sync"`
/// - Paused -> `Active` + `"state-pause"`
/// - Waiting / Interrupted / Cancelled / NeverRun / ManualOnly / Ok -> `Passive` + `""`
///   unless `running` or `paused` forces at least `Active`.
pub fn compute_tray_presentation(
    worst_kind: OverallStatusKind,
    has_running_run: bool,
    is_paused: bool,
) -> (TrayStatus, &'static str) {
    match worst_kind {
        OverallStatusKind::Blocked | OverallStatusKind::Error => {
            (TrayStatus::NeedsAttention, "state-error")
        }
        OverallStatusKind::Overdue => (TrayStatus::NeedsAttention, "alarm-symbolic"),
        OverallStatusKind::Warning => (TrayStatus::Active, "state-warning"),
        OverallStatusKind::Running => (TrayStatus::Active, "state-sync"),
        OverallStatusKind::Paused => (TrayStatus::Active, "state-pause"),
        OverallStatusKind::Pending
        | OverallStatusKind::Interrupted
        | OverallStatusKind::Cancelled
        | OverallStatusKind::NeverRun
        | OverallStatusKind::ManualOnly
        | OverallStatusKind::Ok => {
            if has_running_run {
                (TrayStatus::Active, "state-sync")
            } else if is_paused {
                (TrayStatus::Active, "state-pause")
            } else {
                (TrayStatus::Passive, "")
            }
        }
    }
}

/// Snapshot of tray metadata and M1/M3 menu state presented via `org.kde.StatusNotifierItem`.
#[derive(Debug, Clone)]
pub struct KesticTrayState {
    pub status: TrayStatus,
    pub icon_name: String,
    pub icon_theme_path: String,
    pub overlay_icon_name: String,
    pub tooltip_title: String,
    pub tooltip_description: String,
    pub status_line: String,
    pub running_run: Option<ActiveRunProgressDto>,
    pub paused_line: Option<String>,
    pub next_backup_line: Option<String>,
    pub global_paused: bool,
    pub backups: Vec<TrayBackupItem>,
    pub blocking_active: bool,
}

impl Default for KesticTrayState {
    fn default() -> Self {
        Self {
            status: TrayStatus::Passive,
            icon_name: "kestic-symbolic".to_string(),
            icon_theme_path: String::new(),
            overlay_icon_name: String::new(),
            tooltip_title: "Kestic".to_string(),
            tooltip_description: i18n("Backups ready").into(),
            status_line: i18n("All data backed up").into(),
            running_run: None,
            paused_line: None,
            next_backup_line: None,
            global_paused: false,
            backups: Vec::new(),
            blocking_active: false,
        }
    }
}

type AboutToShowHook = Arc<dyn Fn() -> Option<KesticTrayState> + Send + Sync>;

struct KesticTray {
    state: KesticTrayState,
    action_tx: Option<mpsc::Sender<TrayAction>>,
    quit_tx: Option<mpsc::Sender<()>>,
    about_to_show: Option<AboutToShowHook>,
}

impl KesticTray {
    fn emit(&self, action: TrayAction) {
        if let Some(tx) = &self.action_tx {
            let _ = tx.send(action.clone());
        }
        if matches!(action, TrayAction::RequestQuit)
            && let Some(tx) = &self.quit_tx
            && self.action_tx.is_none()
        {
            let _ = tx.send(());
        }
    }

    fn build_status_header_items(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: self.state.status_line.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
        ];

        if let Some(active) = &self.state.running_run {
            let label = active_run_line(active);
            items.push(
                StandardItem {
                    label,
                    icon_name: "state-sync".to_string(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }

        if let Some(paused_line) = &self.state.paused_line {
            items.push(
                StandardItem {
                    label: paused_line.clone(),
                    icon_name: "state-pause".to_string(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }

        if let Some(next_line) = &self.state.next_backup_line {
            items.push(
                StandardItem {
                    label: next_line.clone(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }

        items
    }

    fn build_backup_now_submenu(&self) -> MenuItem<Self> {
        // "Back Up Now" submenu (M1-S11, AC-M1-11-2)
        let mut backup_submenu: Vec<MenuItem<Self>> = Vec::new();
        let any_enabled =
            !self.state.blocking_active && self.state.backups.iter().any(|b| b.enabled);
        backup_submenu.push(
            StandardItem {
                label: i18n("All Backups").into(),
                icon_name: "media-playback-start".to_string(),
                enabled: any_enabled,
                activate: Box::new(|this: &mut Self| {
                    this.emit(TrayAction::StartAllBackups);
                }),
                ..Default::default()
            }
            .into(),
        );
        if !self.state.backups.is_empty() {
            backup_submenu.push(MenuItem::Separator);
            for entry in &self.state.backups {
                let id = entry.backup_id.clone();
                backup_submenu.push(
                    StandardItem {
                        label: entry.label.clone(),
                        enabled: !self.state.blocking_active && entry.enabled,
                        activate: Box::new(move |this: &mut Self| {
                            this.emit(TrayAction::StartBackup(id.clone()));
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }
        }

        SubMenu {
            label: i18n("Back Up Now").into(),
            icon_name: "media-playback-start".to_string(),
            enabled: !self.state.blocking_active && !self.state.backups.is_empty(),
            submenu: backup_submenu,
            ..Default::default()
        }
        .into()
    }

    fn build_schedule_pause_item(&self) -> MenuItem<Self> {
        // Pause / Resume Schedules (M3-S07, AC-M3-14-4, §10.1)
        if self.state.global_paused {
            StandardItem {
                label: i18n("Resume Schedules").into(),
                icon_name: "media-playback-start".to_string(),
                activate: Box::new(|this: &mut Self| {
                    this.emit(TrayAction::ResumeGlobal);
                }),
                ..Default::default()
            }
            .into()
        } else {
            SubMenu {
                label: i18n("Pause Schedules").into(),
                icon_name: "media-playback-pause".to_string(),
                submenu: vec![
                    StandardItem {
                        label: i18n("1 Hour").into(),
                        activate: Box::new(|this: &mut Self| {
                            this.emit(TrayAction::PauseGlobal(PauseDuration::Hours { hours: 1 }));
                        }),
                        ..Default::default()
                    }
                    .into(),
                    StandardItem {
                        label: i18n("3 Hours").into(),
                        activate: Box::new(|this: &mut Self| {
                            this.emit(TrayAction::PauseGlobal(PauseDuration::Hours { hours: 3 }));
                        }),
                        ..Default::default()
                    }
                    .into(),
                    StandardItem {
                        label: i18n("Until Tomorrow").into(),
                        activate: Box::new(|this: &mut Self| {
                            this.emit(TrayAction::PauseGlobal(PauseDuration::UntilTomorrow));
                        }),
                        ..Default::default()
                    }
                    .into(),
                    StandardItem {
                        label: i18n("Until I Resume").into(),
                        activate: Box::new(|this: &mut Self| {
                            this.emit(TrayAction::PauseGlobal(PauseDuration::Indefinite));
                        }),
                        ..Default::default()
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into()
        }
    }

    fn build_cancel_item(&self) -> MenuItem<Self> {
        let running_id = self.state.running_run.as_ref().map(|r| r.run_id.clone());
        StandardItem {
            label: i18n("Cancel Running Backup").into(),
            icon_name: "process-stop".to_string(),
            enabled: running_id.is_some(),
            activate: Box::new(move |this: &mut Self| {
                if let Some(ref run_id) = running_id {
                    this.emit(TrayAction::CancelRun(run_id.clone()));
                }
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for KesticTray {
    fn id(&self) -> String {
        "kestic".to_string()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn title(&self) -> String {
        "Kestic".to_string()
    }

    fn status(&self) -> ksni::Status {
        self.state.status.into()
    }

    fn icon_theme_path(&self) -> String {
        self.state.icon_theme_path.clone()
    }

    fn icon_name(&self) -> String {
        self.state.icon_name.clone()
    }

    fn overlay_icon_name(&self) -> String {
        self.state.overlay_icon_name.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: self.state.icon_name.clone(),
            icon_pixmap: Vec::new(),
            title: self.state.tooltip_title.clone(),
            description: self.state.tooltip_description.clone(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.emit(TrayAction::OpenWindow);
    }

    fn menu_about_to_show(&mut self) {
        if let Some(hook) = &self.about_to_show
            && let Some(fresh) = hook()
        {
            self.state = fresh;
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items = self.build_status_header_items();
        items.push(MenuItem::Separator);
        items.push(self.build_backup_now_submenu());
        items.push(self.build_schedule_pause_item());
        items.push(self.build_cancel_item());
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: i18n("Open Kestic").into(),
                icon_name: "kestic".to_string(),
                activate: Box::new(|this: &mut Self| {
                    this.emit(TrayAction::OpenWindow);
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: i18n("Quit Kestic…").into(),
                icon_name: "application-exit".to_string(),
                activate: Box::new(|this: &mut Self| {
                    this.emit(TrayAction::RequestQuit);
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

/// Handle controlling a running Kestic `StatusNotifierItem` service.
pub struct TrayHandle {
    inner: ksni::blocking::Handle<KesticTray>,
}

impl TrayHandle {
    /// Spawns the `ksni` SNI service with M0 legacy `quit_tx` compatibility.
    pub fn spawn(
        state: KesticTrayState,
        assume_sni_available: bool,
        quit_tx: Option<mpsc::Sender<()>>,
    ) -> Result<Self, ksni::Error> {
        Self::spawn_with_actions(state, assume_sni_available, None, quit_tx, None)
    }

    /// Spawns the `ksni` SNI service wired to full daemon actions and `menu_about_to_show` hook.
    pub fn spawn_with_actions(
        state: KesticTrayState,
        assume_sni_available: bool,
        action_tx: Option<mpsc::Sender<TrayAction>>,
        quit_tx: Option<mpsc::Sender<()>>,
        about_to_show: Option<AboutToShowHook>,
    ) -> Result<Self, ksni::Error> {
        let tray = KesticTray {
            state,
            action_tx,
            quit_tx,
            about_to_show,
        };
        let inner = if assume_sni_available {
            tray.assume_sni_available(true).spawn()?
        } else {
            tray.spawn()?
        };
        Ok(Self { inner })
    }

    /// Updates tray state (`Status`, overlay icon, tooltip) dynamically.
    pub fn update_state(&self, status: TrayStatus, overlay_icon_name: &str, description: &str) {
        let overlay = overlay_icon_name.to_string();
        let desc = description.to_string();
        let _ = self.inner.update(|tray| {
            tray.state.status = status;
            tray.state.overlay_icon_name = overlay;
            tray.state.tooltip_description = desc;
        });
    }

    /// Replaces the complete tray state snapshot (`Status`, overlay, tooltip, and menu entries).
    pub fn set_full_state(&self, next: KesticTrayState) {
        let _ = self.inner.update(|tray| {
            tray.state = next;
        });
    }

    /// Gracefully shuts down the `ksni` D-Bus service.
    pub fn shutdown(self) {
        self.inner.shutdown().wait();
    }
}
