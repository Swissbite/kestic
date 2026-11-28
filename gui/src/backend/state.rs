// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Daemon connection, state synchronization, event polling, and availability checks
//! for [`super::qobject::KesticApp`].

use super::daemon_call::Lane;
use super::qobject;
use crate::daemon_client::initialize_and_sync;
use crate::daemon_events::{attach_gui_thread, request_refresh, start_daemon_in_background};
use crate::i18n::{daemon_unavailable_text, i18n};
use crate::overall_text::{
    back_up_all_hint, localized_blocking_message, overall_headline, overall_subtitle,
};
use crate::restic_section_view::restic_section_view;
use crate::state_cache::{lookup_backup_summary, shared_state};
use crate::status_texts::{
    format_overall_visual, localized_condition_override_message, localized_manual_block_message,
    localized_unmet_conditions,
};
use crate::summary_texts::format_pause_line;
use crate::time_texts::daemon_now;
use cxx_qt::Threading;
use cxx_qt_lib::QString;
use kestic_core::model::ResticProvider;
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::BlockedReason;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};

/// Counts the calls of `window_active`, so a queued call knows whether a newer one came.
static WINDOW_GENERATION: AtomicU64 = AtomicU64::new(0);

impl qobject::KesticApp {
    pub fn initialize(mut self: Pin<&mut Self>) {
        attach_gui_thread(self.qt_thread());
        initialize_and_sync();
        self.as_mut().apply_cached_state();
        let (show_quit, show_about, show_restore) = {
            let guard = shared_state().lock_unpoisoned();
            (
                guard.launch.show_quit_dialog_on_start,
                guard.launch.show_about_on_start,
                guard.launch.show_restore_on_start,
            )
        };
        self.as_mut().set_show_about_on_start(show_about);
        self.as_mut().set_show_restore_on_start(show_restore);
        if show_quit {
            let (has_running, running_name) = {
                let mut guard = shared_state().lock_unpoisoned();
                // The first state may still be on its way; it fills in the running backup.
                guard.quit_dialog_waits_for_state = guard.state.is_none();
                running_backup(guard.state.as_ref())
            };
            self.as_mut().set_quit_has_running_run(has_running);
            self.as_mut()
                .set_quit_running_backup_name(QString::from(&running_name));
            self.as_mut()
                .quit_confirmation_requested(has_running, QString::from(&running_name));
        }
    }

    /// Takes over what the background threads noted: they wake the Qt thread for it.
    pub fn poll_events(mut self: Pin<&mut Self>) {
        let (progress_dirty, blocked, show_win, show_about, quit_req, maint_preview, show_restore) = {
            let mut guard = shared_state().lock_unpoisoned();
            let pd = guard.progress_dirty;
            guard.progress_dirty = false;
            let blocked = guard.pending_blocked.take();
            let sw = guard.pending_show_window;
            guard.pending_show_window = false;
            let sa = std::mem::take(&mut guard.pending_show_about);
            let show_restore = std::mem::take(&mut guard.pending_show_restore);
            let qr = guard.pending_quit.take();
            let mp = guard.maintenance_preview.take();
            (pd, blocked, sw, sa, qr, mp, show_restore)
        };

        let fetched = std::mem::take(&mut shared_state().lock_unpoisoned().state_fetched);
        if fetched {
            self.as_mut().apply_cached_state();
        } else if progress_dirty {
            self.as_mut().models_updated();
            self.as_mut().apply_overall_texts();
        }

        if let Some((backup_id, reason)) = blocked {
            self.as_mut().handle_pending_blocked(backup_id, reason);
        }

        if show_win {
            self.as_mut().show_window_requested();
        }

        if show_about {
            self.as_mut().about_requested();
        }

        if show_restore {
            self.as_mut().restore_tab_requested();
        }

        if let Some((has_running, name)) = quit_req {
            self.as_mut().set_quit_has_running_run(has_running);
            self.as_mut()
                .set_quit_running_backup_name(QString::from(&name));
            self.as_mut()
                .quit_confirmation_requested(has_running, QString::from(&name));
        }

        self.as_mut().poll_restore();

        if let Some((dest_id, outcome)) = maint_preview {
            self.as_mut().set_maintenance_preview_loading(false);
            match outcome {
                Ok(json) => self.as_mut().maintenance_preview_ready(
                    QString::from(&dest_id),
                    QString::from(&json),
                    QString::from(""),
                ),
                Err(err) => self.as_mut().maintenance_preview_ready(
                    QString::from(&dest_id),
                    QString::from(""),
                    QString::from(&err),
                ),
            }
        }
    }

    fn handle_pending_blocked(mut self: Pin<&mut Self>, backup_id: String, reason: BlockedReason) {
        let (backup_name, unmet_text) = {
            let guard = shared_state().lock_unpoisoned();
            guard
                .state
                .as_ref()
                .and_then(|s| lookup_backup_summary(s, &backup_id))
                .map_or_else(
                    || (backup_id.clone(), None),
                    |b| (b.backup.name.clone(), localized_unmet_conditions(b)),
                )
        };
        if reason == BlockedReason::ConditionNotMet {
            let msg = localized_condition_override_message(&backup_name, unmet_text.as_deref());
            self.as_mut()
                .set_condition_override_backup_id(QString::from(&backup_id));
            self.as_mut()
                .set_condition_override_backup_name(QString::from(&backup_name));
            self.as_mut()
                .set_condition_override_message(QString::from(&msg));
            self.as_mut().condition_override_requested(
                QString::from(&backup_id),
                QString::from(&backup_name),
                QString::from(&msg),
            );
        } else {
            let msg = localized_manual_block_message(&backup_name, reason);
            self.as_mut()
                .set_manual_run_blocked_message(QString::from(&msg));
        }
    }

    fn apply_unreachable_state(mut self: Pin<&mut Self>) {
        let (starting, first_fetch) = {
            let guard = shared_state().lock_unpoisoned();
            (guard.daemon_starting, guard.first_fetch_pending)
        };
        if first_fetch && !starting {
            // The daemon runs; its first state is still on its way.
            self.as_mut().set_blocking_kind(QString::from(""));
            self.as_mut().set_blocking_message(QString::from(""));
            self.as_mut().set_overall_headline(QString::from(""));
            self.as_mut().set_overall_icon(QString::from(""));
            self.as_mut().set_overall_color_kind(QString::from("text"));
        } else if starting {
            let text = i18n("Starting the background service…").to_string();
            self.as_mut()
                .set_blocking_kind(QString::from("daemon_starting"));
            self.as_mut().set_blocking_message(QString::from(&text));
            self.as_mut().set_overall_headline(QString::from(&text));
            self.as_mut().set_overall_icon(QString::from("state-sync"));
            self.as_mut().set_overall_color_kind(QString::from("text"));
        } else {
            self.as_mut()
                .set_blocking_kind(QString::from("daemon_unreachable"));
            self.as_mut().set_blocking_message(QString::from(
                &i18n("The background service is not running. Scheduled backups will not run.")
                    .to_string(),
            ));
            self.as_mut()
                .set_overall_headline(QString::from(&daemon_unavailable_text()));
            self.as_mut().set_overall_icon(QString::from("state-error"));
            self.as_mut()
                .set_overall_color_kind(QString::from("negative"));
        }
        self.as_mut().set_has_config_file(false);
        self.as_mut().set_daemon_version(QString::from(""));
        self.as_mut().set_daemon_build_commit(QString::from(""));
        self.as_mut().set_overall_subtitle(QString::from(""));
        self.as_mut().set_backups_have_attention(false);
        self.as_mut().set_destinations_have_attention(false);
        self.as_mut().set_backups_count(0);
        self.as_mut().set_back_up_all_hint(QString::from(""));
        self.as_mut().set_global_paused(false);
        self.as_mut().set_global_paused_line(QString::from(""));
        self.as_mut().set_default_overdue_threshold(2);
        self.as_mut().set_notif_on_error(true);
        self.as_mut().set_notif_on_warning(true);
        self.as_mut().set_notif_on_overdue(true);
        self.as_mut().set_notif_on_maintenance_failed(true);
        self.as_mut().set_notif_on_restic_update(false);
        self.as_mut().set_restic_offer_switch(false);
        self.as_mut().set_restic_offer_remove(false);
        self.as_mut().update_restic_offer();
        self.as_mut().models_updated();
    }

    /// Headline and subtitle after a progress update: they name the running backup's phase
    /// and percentage, which change without a new state.
    fn apply_overall_texts(mut self: Pin<&mut Self>) {
        let texts = {
            let guard = shared_state().lock_unpoisoned();
            guard
                .state
                .as_ref()
                .filter(|_| guard.daemon_reachable)
                .map(|s| (overall_headline(s), overall_subtitle(s)))
        };
        if let Some((headline, subtitle)) = texts {
            self.as_mut().set_overall_headline(QString::from(&headline));
            self.as_mut().set_overall_subtitle(QString::from(&subtitle));
        }
    }

    pub(super) fn apply_cached_state(mut self: Pin<&mut Self>) {
        let (reachable, state_opt, loading) = {
            let guard = shared_state().lock_unpoisoned();
            (
                guard.daemon_reachable,
                guard.state.clone(),
                !guard.daemon_reachable && (guard.daemon_starting || guard.first_fetch_pending),
            )
        };

        self.as_mut().set_daemon_reachable(reachable);
        self.as_mut().set_state_loading(loading);

        let Some(state) = state_opt.filter(|_| reachable) else {
            self.as_mut().apply_unreachable_state();
            return;
        };
        let quit_waits =
            std::mem::take(&mut shared_state().lock_unpoisoned().quit_dialog_waits_for_state);
        if quit_waits {
            let (has_running, running_name) = running_backup(Some(&state));
            self.as_mut().set_quit_has_running_run(has_running);
            self.as_mut()
                .set_quit_running_backup_name(QString::from(&running_name));
        }

        let now = daemon_now(&state);
        self.as_mut().set_has_config_file(state.has_config_file);
        self.as_mut().set_expert_mode(state.config.expert_mode);
        self.as_mut()
            .set_daemon_version(QString::from(&state.daemon_version));
        self.as_mut()
            .set_daemon_build_commit(QString::from(&state.daemon_build_commit));

        self.as_mut().set_restic_valid(state.restic_status.valid);
        self.as_mut().set_restic_version(QString::from(
            state.restic_status.version.as_deref().unwrap_or(""),
        ));
        self.as_mut().set_restic_binary_path(QString::from(
            state.restic_status.binary_path.as_deref().unwrap_or(""),
        ));
        let provider_kind = match &state.config.restic_provider {
            ResticProvider::Automatic => "automatic",
            ResticProvider::System => "system",
            ResticProvider::AppManaged => "app_managed",
            ResticProvider::Custom { .. } => "custom",
        };
        self.as_mut()
            .set_restic_provider_kind(QString::from(provider_kind));
        self.as_mut()
            .show_restic_section(&restic_section_view(&state.restic_status));
        self.as_mut().update_restic_offer();

        // Blocking order: Daemon -> SecretStore -> restic (Daemon checked above)
        if let Some(b) = &state.blocking {
            self.as_mut()
                .set_blocking_kind(QString::from(b.kind.as_str()));
            self.as_mut()
                .set_blocking_message(QString::from(&localized_blocking_message(&state)));
        } else {
            self.as_mut().set_blocking_kind(QString::from(""));
            self.as_mut().set_blocking_message(QString::from(""));
        }

        let (o_icon, o_color) = format_overall_visual(state.overall.kind);
        self.as_mut()
            .set_overall_headline(QString::from(&overall_headline(&state)));
        self.as_mut()
            .set_overall_subtitle(QString::from(&overall_subtitle(&state)));
        self.as_mut().set_overall_icon(QString::from(o_icon));
        self.as_mut().set_overall_color_kind(QString::from(o_color));
        self.as_mut()
            .set_backups_have_attention(state.overall.backups_have_attention);
        self.as_mut()
            .set_destinations_have_attention(state.overall.destinations_have_attention);
        self.as_mut()
            .set_backups_count(i32::try_from(state.backups.len()).unwrap_or(0));
        let hint = back_up_all_hint(&state);
        self.as_mut().set_back_up_all_hint(QString::from(&hint));

        // Same formatter as the per-backup card so banner and card show identical times.
        let global_paused_line = if state.overall.paused {
            format_pause_line(
                state.overall.paused_since_unix,
                state.overall.paused_until_unix,
                now,
            )
        } else {
            String::new()
        };
        self.as_mut().set_global_paused(state.overall.paused);
        self.as_mut()
            .set_global_paused_line(QString::from(&global_paused_line));
        self.as_mut().set_default_overdue_threshold(
            i32::try_from(state.config.default_overdue_threshold).unwrap_or(2),
        );
        self.as_mut()
            .set_notif_on_error(state.config.notifications.on_error);
        self.as_mut()
            .set_notif_on_warning(state.config.notifications.on_warning);
        self.as_mut()
            .set_notif_on_overdue(state.config.notifications.on_overdue);
        self.as_mut()
            .set_notif_on_maintenance_failed(state.config.notifications.on_maintenance_failed);
        self.as_mut()
            .set_notif_on_restic_update(state.config.notifications.on_restic_update);

        self.as_mut().models_updated();
    }

    pub fn retry_start_daemon(mut self: Pin<&mut Self>) {
        start_daemon_in_background();
        self.as_mut().apply_cached_state();
    }

    pub fn unlock_secret_store(self: Pin<&mut Self>) {
        // Whether the user unlocked shows in the state the daemon publishes.
        // The daemon waits for the user to answer the password prompt.
        self.call_daemon_with(
            Lane::slow("unlock-password-store"),
            |client| client.unlock_secret_store().is_ok(),
            |_| false,
            |_, _| request_refresh(),
        );
    }

    pub fn window_active(self: Pin<&mut Self>, active: bool, page: &QString) {
        let page = page.to_string();
        let generation = WINDOW_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        // The daemon only notes the window; an activated window shows the state anew. Only the
        // newest state of the window counts: a call still queued when a newer one came is not
        // sent. A window that closes says so at once, ahead of calls still on their way.
        let lane = if active { Lane::window() } else { Lane::Alone };
        self.call_daemon(
            lane,
            move |client| {
                WINDOW_GENERATION.load(Ordering::SeqCst) == generation
                    && client.window_active(active, &page).is_ok()
            },
            move |_, _| {
                if active {
                    request_refresh();
                }
            },
        );
    }

    pub fn check_availability(self: Pin<&mut Self>, force: bool) {
        // The result of the check arrives with the state. A forced check probes every
        // destination and may take a while; other actions do not wait for it.
        self.call_daemon_with(
            Lane::slow("availability"),
            move |client| client.check_availability(force).is_ok(),
            |_| false,
            |_, _| request_refresh(),
        );
    }
}

/// Whether a backup runs according to `state`, and its name, for the quit dialog.
fn running_backup(state: Option<&kestic_daemon::dbus_api::DaemonStateDto>) -> (bool, String) {
    let active = state.and_then(|s| s.overall.active_run_progress.as_ref());
    (
        active.is_some(),
        active.map_or_default(|a| a.backup_name.clone()),
    )
}
