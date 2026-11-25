# Kestic

A KDE Plasma desktop app for easy backups with [restic](https://restic.net). Pick what and where. Kestic does the rest and shows you the restic command behind every run.

**Status:** Milestones M0 (Spike), M1 (Walking Skeleton), M2 (Backup Configuration: exclude rules with simple/expert modes & negation linter, dry-run preview, retention rules & preview, automatic post-backup `forget` with lock catch-up, Exit 3 unreadable file handling, drive mount/UUID source checks, nested source overlap warnings, and full configuration overview), and M3 (Automation: schedules with catch-up after sleep or shutdown, run conditions for AC power, network, metered connections and allowed Wi-Fi networks, retries and overdue detection, per-destination maintenance with clean up and verify, pausing schedules, and sleep handling during runs: idle/suspend inhibit and a clean interruption on lid close) are complete and manually accepted on TUXEDO OS. M4 (Restore: a Restore tab with the snapshots of every backup, comparing two snapshots, browsing a snapshot folder by folder and searching the snapshots of a backup, restoring into a new folder by default or to the original location or a folder with files in it only after a preview, with existing files never overwritten unless you pick an overwrite mode, a home folder under a different user name mapped to yours, unassigned and orphaned snapshots that Kestic only reads, and «Restore Files…» in the tray) is complete; its manual acceptance is pending.

- **Website:** [swissbite.github.io/kestic](https://swissbite.github.io/kestic/), a showcase built from [site/](site/) with screenshots rendered by the GUI test suite.

- **Specification (German):** [docs/spec/](docs/spec/) — start at [backlog.md](docs/spec/backlog.md) and [requirements.md](docs/spec/requirements.md). Internal working specifications under `docs/spec/` are maintained exclusively in German.
- **Agent & contributor guide:** [AGENTS.md](AGENTS.md)
- **UI specification (German):** [docs/spec/ui/index.md](docs/spec/ui/index.md)

## Tech Stack

- **Core, daemon & GUI bridge:** Rust (`edition = "2024"`, MSRV `1.100`), CXX-Qt `0.10.0`, `cxx-kde-frameworks` (KI18n), `zbus 6.3.0`, SQLite (`rusqlite`)
- **UI:** Kirigami (`>= 6.24`), Qt 6 (`6.10` on reference system TUXEDO OS / Plasma `6.6`)
- **Backup engine:** `restic >= 0.18.0`

## Building & Testing

1. Install system dependencies, Rust toolchain, `cargo-deny`, `cargo-deb`, and isolated test binaries (`restic` `0.18.0` + latest, `rest-server`, `rclone`) via:

   ```bash
   ./scripts/bootstrap.sh
   ```

2. Run the full verification suite (`fmt`, `clippy -D warnings`, `.deb` packaging, QML offscreen smoke tests, unit & dual-version integration tests, `cargo deny check`, `reuse lint`):

   ```bash
   just verify
   ```

3. Build only the Debian package:

   ```bash
   just deb
   ```

   The resulting `.deb` package is written under `target/debian/`.

## Acknowledgements

Kestic does not back up anything by itself: every backup, check and restore is done by [restic](https://restic.net), which Kestic runs and shows you the command for. Thanks to the restic authors and contributors for a backup tool that is fast, secure and well documented.

Kestic is an independent project, not affiliated with or endorsed by the restic project. The name «restic» refers to that project and belongs to its authors.

## License

`GPL-3.0-or-later` (REUSE 3.0 compliant; build/CI scripts use `BSD-2-Clause` and docs/specs use `CC-BY-SA-4.0`, see [LICENSES/](LICENSES/) and [.reuse/dep5](.reuse/dep5)).
