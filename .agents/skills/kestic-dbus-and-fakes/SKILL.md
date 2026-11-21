---
name: kestic-dbus-and-fakes
description: Patterns and pitfalls for zbus 6.3.0, daemon lifecycle, Tray (ksni), Notifications, Secret Service, Plasma JobView, Portal Inhibit, and D-Bus system fakes (UPower, NetworkManager, logind, UDisks2) for M1–M6.
---

# Kestic D-Bus, Daemon & System Fakes Skill

Use this skill for all `daemon/` stories, D-Bus API work (`docs/spec/dbus-api.md`), Secret Service integration, Tray/Notifications, and D-Bus fake test harnesses.

## `zbus 6.3.0` Rules (from M0)

- Kestic uses `zbus = "6.3.0"` (never `zbus 5` APIs or `zbus::blocking`). Use `zbus::block_on(...)` when bridging from sync code, `zbus::Connection`, `zbus::Proxy`, `Variant`, and `OwnedVariant` / `OwnedValue` (see `daemon/src/secrets.rs`).
- **Connection lifetime rule (critical M0 finding):** Stateful session-bound D-Bus objects **must** own and retain their `zbus::Connection` for the entire operation lifetime:
  - `daemon/src/job_view.rs` (`JobViewHandle` on `org.kde.JobViewServerV2` → `org.kde.JobViewV3`)
  - `daemon/src/inhibit.rs` (`PortalInhibitGuard` on `org.freedesktop.portal.Inhibit`)
  - M3 `logind` sleep delay inhibitor (`Inhibit` returns a file descriptor that must also stay open until `PrepareForSleep(true)` cleanup finishes).
  Dropping the `zbus::Connection` early causes Plasma `kuiserver` / `xdg-desktop-portal` to treat the client as crashed and immediately abort/release the handle.

## Daemon Lifecycle & Single-Instance

- Single-instance lock via D-Bus name `net.daester.Kestic`; versioned control interface `…Kestic1` (`docs/spec/dbus-api.md`).
- **Sole writer invariant:** Only `daemon` writes `config.toml` and state (`F14`). GUI validates/submits changes over D-Bus (`UpdateBackup`, `UpdateDestination`, etc.) and never mutates config files directly.
- `--autostart` mode (M1-S12): exit `0` immediately without claiming the bus name or showing a tray icon when `config.toml` does not exist; without `--autostart` stay alive so the GUI can submit initial config.
- Update `docs/spec/dbus-api.md` whenever methods or signals on `…Kestic1` are added or changed (`AC-M1-09-5`).

## Tray & Notifications

- Tray uses `ksni = "0.3.7"` (`daemon/src/tray.rs`) with `icon_name = "kestic-symbolic"`, `IconThemePath`, and SNI `Status` (`Passive`, `Active`, `NeedsAttention`) + overlay icons per `docs/spec/ui/tray-und-benachrichtigungen.md` §10.1.
- Notifications use `notify-rust = "4.19.0"` with `desktop-entry` hint = `"net.daester.Kestic"`.
- **M0 finding (`AC-M0-11-4`):** `kestic.notifyrc` without `KNotifications` does **not** filter `notify-rust` popups per event in Plasma 6.6. Therefore Kestic's own per-event notification toggles (F3) are mandatory before calling `notify-rust`.

## Isolated D-Bus & Secret Service Testing

- Never touch the developer's live session bus or real KWallet (`AC-M1-Q-1`).
- Run D-Bus tests in an isolated `dbus-run-session` (or dedicated test bus) with `DBUS_SESSION_BUS_ADDRESS` pointing strictly to the test bus.
- For headless Secret Service tests (`SecretServiceStore` in `daemon/src/secrets.rs`), use `kestic_test_support::keyring::HeadlessKeyring` (crate `test-support`). It:
  1. Clears the inherited host environment (`GNOME_KEYRING_CONTROL`, `SSH_AUTH_SOCK`, the live `DBUS_SESSION_BUS_ADDRESS`).
  2. Starts `gnome-keyring-daemon --foreground --replace --unlock --components=secrets --control-directory <short /tmp path>` with an empty password on `stdin`.
  3. Keeps the `--control-directory` path short (`< 107` bytes total `sun_path` socket limit).
- `run_isolated_test` (daemon tests) starts this keyring on every private bus; in-process daemons use `common::isolated_secret_store()`, which panics outside such a test. GUI smoke tests get it via `SmokeFixture` and seed passwords with `fx.keyring.store_secret(...)`.
- There is no secret-file override in `kestic` itself: the daemon and `kestic secret get` always read the Secret Service of the session bus.

## System Service Fakes (M3 & M5)

Abstract external desktop services behind traits in `core/` and test `daemon/` against D-Bus fakes on the isolated test bus:
- **UPower (M3-S03):** `OnBattery` property & `PropertiesChanged`.
- **NetworkManager (M3-S03, M3-S04):** connectivity state, `Metered` property, active connection / SSID changes.
- **systemd-logind (M3-S09, M3-S10):** `Inhibit("sleep", ..., "delay")` FD + `PrepareForSleep(bool)` signal.
- **UDisks2 (M3-S13, M5-S05):** removable media & mount/unmount signals.
- **Notifications & JobViewServerV2 & Portal Inhibit:** fake D-Bus servers asserting exact call payloads, hints, percentages, and `cancelRequested`.
