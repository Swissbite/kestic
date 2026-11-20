// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Turns the `status` lines of `restic backup --json` into progress that does not jump
//! (`docs/spec/restic-integration.md` «Fortschritt beim Sichern»).
//!
//! restic finds files and backs them up at the same time, so `total_bytes` grows during the
//! run and `percent_done` can fall. The scan is over once restic sends `seconds_remaining`;
//! only then the percentage is valid. Until then the run is in phase `scanning` and shows the
//! amount looked through instead of a percentage.

use crate::restic::ResticStatusMessage;
use kestic_i18n::codes::RunProgressPhase;

/// Progress of a running backup as shown to the user.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BackupProgress {
    pub phase: RunProgressPhase,
    /// 0–100; 0 while scanning, never falls once the scan is finished.
    pub percent_done: f64,
    /// Bytes looked through so far.
    pub bytes_done: u64,
    /// Total bytes; 0 while scanning (the total still grows).
    pub total_bytes: u64,
    /// Files looked through so far.
    pub files_done: u64,
    /// Bytes per second after the scan; 0 while scanning or when unknown.
    pub speed_bps: u64,
    /// Seconds left after the scan; `None` when restic does not know.
    pub seconds_remaining: Option<u64>,
}

/// Keeps the phase and the highest percentage shown across the `status` lines of one run.
#[derive(Debug, Clone, Default)]
pub struct BackupProgressTracker {
    phase: RunProgressPhase,
    shown_percent: f64,
}

impl BackupProgressTracker {
    /// Tracker for a new run (phase `scanning`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Current phase.
    pub fn phase(&self) -> RunProgressPhase {
        self.phase
    }

    /// Progress for one `status` line. `fallback_elapsed_secs` is used for the speed while
    /// restic does not send `seconds_elapsed` yet (first second of the run).
    pub fn update(
        &mut self,
        status: &ResticStatusMessage,
        fallback_elapsed_secs: u64,
    ) -> BackupProgress {
        let seconds_remaining = status.seconds_remaining.filter(|s| *s > 0);
        if seconds_remaining.is_some() {
            self.phase = RunProgressPhase::ScanFinished;
        }
        let bytes_done = status.bytes_done.unwrap_or(0);
        let files_done = status.files_done.unwrap_or(0);
        match self.phase {
            RunProgressPhase::Scanning => BackupProgress {
                phase: RunProgressPhase::Scanning,
                bytes_done,
                files_done,
                ..BackupProgress::default()
            },
            RunProgressPhase::ScanFinished => {
                let percent = (status.percent_done * 100.0).clamp(0.0, 100.0);
                self.shown_percent = self.shown_percent.max(percent);
                let elapsed = status
                    .seconds_elapsed
                    .filter(|s| *s > 0)
                    .unwrap_or(fallback_elapsed_secs)
                    .max(1);
                BackupProgress {
                    phase: RunProgressPhase::ScanFinished,
                    percent_done: self.shown_percent,
                    bytes_done,
                    total_bytes: status.total_bytes.unwrap_or(0),
                    files_done,
                    speed_bps: bytes_done / elapsed,
                    seconds_remaining,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(percent: f64, remaining: Option<u64>) -> ResticStatusMessage {
        ResticStatusMessage {
            percent_done: percent,
            bytes_done: Some(10),
            total_bytes: Some(20),
            files_done: Some(3),
            seconds_elapsed: Some(2),
            seconds_remaining: remaining,
            ..ResticStatusMessage::default()
        }
    }

    #[test]
    fn zero_seconds_remaining_does_not_end_the_scan() {
        let mut t = BackupProgressTracker::new();
        let p = t.update(&status(0.5, Some(0)), 1);
        assert_eq!(p.phase, RunProgressPhase::Scanning);
        assert_eq!(p.percent_done, 0.0);
        assert_eq!(p.total_bytes, 0);
        assert_eq!(p.speed_bps, 0);
    }

    #[test]
    fn scan_stays_finished_without_seconds_remaining() {
        let mut t = BackupProgressTracker::new();
        t.update(&status(0.5, Some(3)), 1);
        let p = t.update(&status(0.6, None), 1);
        assert_eq!(t.phase(), RunProgressPhase::ScanFinished);
        assert_eq!(p.percent_done, 60.0);
        assert_eq!(p.seconds_remaining, None);
    }

    #[test]
    fn percent_above_one_is_capped_at_hundred() {
        let mut t = BackupProgressTracker::new();
        let p = t.update(&status(1.2, Some(1)), 1);
        assert_eq!(p.percent_done, 100.0);
    }

    #[test]
    fn speed_without_restic_elapsed_time_uses_the_fallback() {
        let mut t = BackupProgressTracker::new();
        let st = ResticStatusMessage {
            seconds_elapsed: None,
            ..status(0.5, Some(4))
        };
        let p = t.update(&st, 5);
        assert_eq!(p.speed_bps, 2);
    }

    #[test]
    fn speed_after_scan_uses_restic_elapsed_time() {
        let mut t = BackupProgressTracker::new();
        let p = t.update(&status(0.5, Some(4)), 9);
        assert_eq!(p.speed_bps, 5);
        assert_eq!(p.seconds_remaining, Some(4));
        assert_eq!(p.total_bytes, 20);
    }
}
