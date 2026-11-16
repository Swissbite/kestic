// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! StatusNotifierItem tray integration via `ksni 0.3.7` (K1, AC-M0-07-2, AC-M0-11-3, AC-M0-11-5).

use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem};
use std::sync::mpsc;

/// Mapped StatusNotifierItem visibility state per `tray-und-benachrichtigungen.md` §10.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayStatus {
    Passive,
    Active,
    NeedsAttention,
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

/// Snapshot of tray metadata presented via `org.kde.StatusNotifierItem`.
#[derive(Debug, Clone)]
pub struct KesticTrayState {
    pub status: TrayStatus,
    pub icon_name: String,
    pub icon_theme_path: String,
    pub overlay_icon_name: String,
    pub tooltip_title: String,
    pub tooltip_description: String,
}

impl Default for KesticTrayState {
    fn default() -> Self {
        Self {
            status: TrayStatus::Passive,
            icon_name: "kestic-symbolic".to_string(),
            icon_theme_path: String::new(),
            overlay_icon_name: String::new(),
            tooltip_title: "Kestic".to_string(),
            tooltip_description: "Backups ready".to_string(),
        }
    }
}

struct KesticTray {
    state: KesticTrayState,
    quit_tx: Option<mpsc::Sender<()>>,
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

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items = vec![
            StandardItem {
                label: self.state.tooltip_description.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
        ];

        if let Some(tx) = &self.quit_tx {
            let quit_tx = tx.clone();
            items.push(
                StandardItem {
                    label: "Quit Kestic...".to_string(),
                    icon_name: "application-exit".to_string(),
                    activate: Box::new(move |_| {
                        let _ = quit_tx.send(());
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        items
    }
}

/// Handle controlling a running Kestic `StatusNotifierItem` service.
pub struct TrayHandle {
    inner: ksni::blocking::Handle<KesticTray>,
}

impl TrayHandle {
    /// Spawns the `ksni` SNI service and registers it with `org.kde.StatusNotifierWatcher`.
    pub fn spawn(
        state: KesticTrayState,
        assume_sni_available: bool,
        quit_tx: Option<mpsc::Sender<()>>,
    ) -> Result<Self, ksni::Error> {
        let tray = KesticTray { state, quit_tx };
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

    /// Gracefully shuts down the `ksni` D-Bus service.
    pub fn shutdown(self) {
        self.inner.shutdown().wait();
    }
}
