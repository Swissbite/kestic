// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Screenshots shown on the site. The files are not committed: the build copies
// them from the GUI smoke tests (`target/smoke-artifacts/` locally, the
// `smoke-artifacts` artifact in CI). A missing file or a changed size fails the
// build, so the page never shows a stale picture.

export interface Screenshot {
  /** File name in the smoke artifact directory. */
  readonly file: string;
  readonly caption: string;
  readonly alt: string;
  readonly width: number;
  readonly height: number;
}

const wide = { width: 1180, height: 740 } as const;

export const heroShot: Screenshot = {
  file: "m3-overview-status-wide-en.png",
  caption: "Overview: one backup overdue, one waiting for AC power, one done.",
  alt: "Kestic overview with three backup cards: one overdue after four missed runs, one backed up just now, and one waiting for AC power.",
  ...wide,
};

export const galleryShots: readonly Screenshot[] = [
  {
    file: "m3-schedule-defaults-wide-en.png",
    caption: "Schedule with the next three runs.",
    alt: "Schedule page set to daily at 12:00, listing the next three scheduled runs.",
    ...wide,
  },
  {
    file: "m3-conditions-defaults-wide-en.png",
    caption: "Run conditions: power, network, allowed Wi-Fi networks.",
    alt: "Run conditions page with AC power and network required, metered networks unchecked, and a list of Wi-Fi networks.",
    ...wide,
  },
  {
    file: "m2-dry-run-preview-en.png",
    caption: "Backup preview before the first run.",
    alt: "Backup preview listing the files to back up, their total size, and the estimated data added.",
    ...wide,
  },
  {
    file: "m2-full-config-wide-en.png",
    caption: "Full configuration, with a warning about overlapping sources.",
    alt: "Full configuration of a backup with a warning that its source overlaps with another backup on the same destination.",
    ...wide,
  },
  {
    file: "m1-history-run-detail.png",
    caption: "History: every run with its result, log, and commands.",
    alt: "History page showing a successful run with tabs for result, log, and commands.",
    ...wide,
  },
  {
    file: "m3-pause-menu-wide-en.png",
    caption: "Pause schedules for an hour, until tomorrow, or until you resume.",
    alt: "Pause menu with the options 1 hour, 3 hours, until tomorrow, and until I resume.",
    ...wide,
  },
];

export const allShots: readonly Screenshot[] = [heroShot, ...galleryShots];
