# Kestic

Native KDE Plasma backup manager powered by [restic](https://restic.net): as simple as a beginner-friendly backup tool, as powerful as restic itself.

**Status:** Milestones M0 (Spike), M1 (Walking Skeleton), and M2 (Backup Configuration: exclude rules with simple/expert modes & negation linter, dry-run preview, retention rules & preview, automatic post-backup `forget` with lock catch-up, Exit 3 unreadable file handling, drive mount/UUID source checks, nested source overlap warnings, and full configuration overview) are complete and manually accepted on TUXEDO OS.

- **Specification (German):** [docs/spec/](docs/spec/) — start at [backlog.md](docs/spec/backlog.md) and [requirements.md](docs/spec/requirements.md). Internal working specifications under `docs/spec/` are maintained exclusively in German.
- **Agent & contributor guide:** [AGENTS.md](AGENTS.md)
- **UI specification (German):** [docs/spec/ui/index.md](docs/spec/ui/index.md)

## Tech Stack

- **Core, daemon & GUI bridge:** Rust (`edition = "2024"`, MSRV `1.85`), CXX-Qt `0.10.0`, `cxx-kde-frameworks` (KI18n), `zbus 6.3.0`, SQLite (`rusqlite`)
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

## License

`GPL-3.0-or-later` (REUSE 3.0 compliant; build/CI scripts use `BSD-2-Clause` and docs/specs use `CC-BY-SA-4.0`, see [LICENSES/](LICENSES/) and [.reuse/dep5](.reuse/dep5)).
