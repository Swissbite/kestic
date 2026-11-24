// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Screenshots shown on the site. The files are not committed: the build copies
// them from the GUI smoke tests (`target/smoke-artifacts/` locally, the
// `smoke-artifacts` artifact in CI). A missing file or a changed size fails the
// build, so the page never shows a stale picture.

/** Region of the screenshot shown on the page, in image pixels. The lightbox shows the whole image. */
export interface Crop {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface Screenshot {
  /** File name in the smoke artifact directory. */
  readonly file: string;
  readonly caption: string;
  readonly alt: string;
  readonly width: number;
  readonly height: number;
  readonly crop: Crop;
}

const wide = { width: 1180, height: 740 } as const;

export const heroShot: Screenshot = {
  file: "m3-overview-status-wide-en.png",
    crop: { x: 0, y: 0, width: 1180, height: 420 },
  caption: "Overview: one backup overdue, one waiting for AC power, one done.",
  alt: "Kestic overview with three backup cards: one overdue after four missed runs, one backed up just now, and one waiting for AC power.",
  ...wide,
};

export const galleryShots: readonly Screenshot[] = [
  {
    file: "m4-restore-snapshot-list-wide-en.png",
    crop: { x: 0, y: 62, width: 870, height: 580 },
    caption: "Restore: every backup with its snapshots, newest first.",
    alt: "Restore tab listing the backups Home and Photos, the unassigned and orphaned snapshots, and the three snapshots of Home with time, data added, and host.",
    ...wide,
  },
  {
    file: "m4-restore-browse-wide-en.png",
    crop: { x: 580, y: 110, width: 600, height: 400 },
    caption: "Browse a snapshot and mark what to restore.",
    alt: "Folder list of a snapshot with a breadcrumb, showing files and folders with size and modification date, each with a checkbox.",
    ...wide,
  },
  {
    file: "m4-restore-dialog-wide-en-overwrite.png",
    crop: { x: 290, y: 171, width: 600, height: 400 },
    caption: "Restore to the original location only after a preview.",
    alt: "Restore dialog set to the original location with the overwrite mode Always, warning that one file will be overwritten and that this cannot be undone, and listing that file.",
    ...wide,
  },
  {
    file: "m4-restore-compare-wide-en.png",
    crop: { x: 580, y: 110, width: 600, height: 400 },
    caption: "Compare two snapshots: new, removed, and changed files.",
    alt: "Comparison of two snapshots with nine changes, filter chips for new, removed, and changed files, and a list of the changed paths.",
    ...wide,
  },
  {
    file: "m3-schedule-defaults-wide-en.png",
    crop: { x: 580, y: 110, width: 600, height: 400 },
    caption: "Schedule with the next three runs.",
    alt: "Schedule page set to daily at 12:00, listing the next three scheduled runs.",
    ...wide,
  },
  {
    file: "m3-conditions-defaults-wide-en.png",
    crop: { x: 580, y: 110, width: 600, height: 400 },
    caption: "Run conditions: power, network, allowed Wi-Fi networks.",
    alt: "Run conditions page with AC power and network required, metered networks unchecked, and a list of Wi-Fi networks.",
    ...wide,
  },
  {
    file: "m2-dry-run-preview-en.png",
    crop: { x: 580, y: 62, width: 600, height: 400 },
    caption: "Backup preview before the first run.",
    alt: "Backup preview listing the files to back up, their total size, and the estimated data added.",
    ...wide,
  },
  {
    file: "m2-full-config-wide-en.png",
    crop: { x: 0, y: 110, width: 600, height: 400 },
    caption: "Full configuration, with a warning about overlapping sources.",
    alt: "Full configuration of a backup with a warning that its source overlaps with another backup on the same destination.",
    ...wide,
  },
  {
    file: "m1-history-run-detail.png",
    crop: { x: 0, y: 62, width: 870, height: 580 },
    caption: "History: every run with its result, log, and commands.",
    alt: "History page showing a successful run with tabs for result, log, and commands.",
    ...wide,
  },
  {
    file: "m3-pause-menu-wide-en.png",
    crop: { x: 0, y: 0, width: 600, height: 400 },
    caption: "Pause schedules for an hour, until tomorrow, or until you resume.",
    alt: "Pause menu with the options 1 hour, 3 hours, until tomorrow, and until I resume.",
    ...wide,
  },
];

export const allShots: readonly Screenshot[] = [heroShot, ...galleryShots];
