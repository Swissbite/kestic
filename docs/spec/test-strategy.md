# Teststrategie

Jedes Akzeptanzkriterium (AC) in [milestones/](milestones/) wird durch einen automatisierten Test belegt, ausser es ist als «Test: manuell» markiert (dann [manual-acceptance.md](manual-acceptance.md)). Rückverfolgung: FR/NFR → Story → AC → Test; Test-Namen oder Kommentare nennen die AC-ID.

## Prüfbefehl

`just verify` = `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` + `cargo deny check` (Lizenzen + Advisories, Q1) + Integrationstests gegen restic 0.18.0 **und** die aktuelle Version (`scripts/bootstrap.sh` lädt beide, Q2) + QML-Smoke + Offscreen-Screenshots pro Screen als Artefakt für die menschliche Abnahme (Q3) + `reuse lint`. Grün = Story erledigt.

## Testebenen

| Ebene | Inhalt | Werkzeuge |
| --- | --- | --- |
| Unit | reine Logik in `core` (Modell, Config, Parser, Scheduler mit injizierbarer Uhr P36, Regeln) | `cargo test` |
| Integration | gegen echtes restic und Temp-Repo; REST über `rest-server`, S3 über `rclone serve s3`, SFTP über Benutzer-`sshd`, jeweils lokal auf Zufallsport mit Auth | `cargo test` (Integrationstests), `bootstrap.sh`-Binaries |
| D-Bus | Daemon-API und Systemdienste in eigenem `dbus-run-session`; Notifications, UPower, NetworkManager, logind, UDisks2, Portal als D-Bus-Fakes über Traits | `zbus`, Fakes |
| GUI-Smoke | `QT_QPA_PLATFORM=offscreen`: App startet, Screens laden ohne QML-Fehler, Model gebunden, Screenshot je Screen | QML-Test, Screenshot-Artefakte |
| manuell | Look & Feel, Hardware- und Session-Verhalten | [manual-acceptance.md](manual-acceptance.md) |

«Integration» heisst in den Meilensteinen: gegen restic 0.18.0 **und** die aktuelle Version.

## Echte Abhängigkeiten statt Mocks, wo möglich

- restic gegen Temp-Repo (`restic init` im Tempdir).
- REST-Backend: `rest-server` lokal auf Zufallsport mit Auth.
- S3: `rclone serve s3` (ab rclone 1.65, experimentell; Ubuntu 24.04 liefert 1.60.1 → `bootstrap.sh` lädt rclone selbst und prüft SHA256; Ersatz bei Problemen: Garage). MinIO entfällt: keine fertigen Community-Binaries mehr seit Ende 2025, Repo nicht mehr gepflegt ([linuxiac](https://linuxiac.com/minio-with-yet-another-controversial-move/), [GitHub minio](https://github.com/minio/minio)).
- SFTP: `sshd` als normaler Benutzer auf Zufallsport mit Temp-Schlüsseln, nach dem Test beendet.
- Secret Service: Trait mit In-Memory-Implementierung für Unit-Tests + Integrationstest in `dbus-run-session`; akzeptierter Fallback laut Spike: Datei-Backend hinter dem Secret-Trait (Q5).
- Snapshots ohne Zusammenfassung (ältere restic-Versionen): Snapshot-JSON als Fixture statt altem Binary.

## Test-Isolation (harte Regel)

Tests berühren nie echte Benutzerdaten oder -konfiguration:

- `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME`, `XDG_RUNTIME_DIR` auf ein Tempdir.
- `known_hosts` nur über den von Kestic übergebenen Pfad (P24); in Tests zusätzlich `-F <Test-ssh_config>` und `-o IdentitiesOnly=yes`, weil OpenSSH `~` über `/etc/passwd` auflöst **[angenommen]**.
- Eigener `dbus-run-session`; kein Zugriff auf die echte KWallet, echte Repos oder `~/.local/share/kestic`.
- Tests setzen nichts unter `/etc/xdg/autostart` voraus.
- Tests laufen nicht als root (Rechte-Tests mit `chmod 000` wären sonst wirkungslos).

## Warnungsfreiheit und Lints (NFR-12)

Alle Build-, Lint- und Smoke-Stufen laufen warnungsfrei (`cargo clippy --workspace --all-targets -- -D warnings`, C++-Compiler für CXX-Qt/Smoke-Helper, `QT_QPA_PLATFORM=offscreen` ohne Qt-/QML-Warnungen auf `stderr`, `.po`-Validierung, `reuse lint`):

- **Ursache beheben statt abschalten:** Warnungen durch sauberes Refactoring lösen (z. B. Kontext-/Optionen-Structs bei vielen Parametern, ungenutzten Code entfernen, Drittanbieter-/Systemheader wie KF6 in `gui/build.rs` über `-isystem` statt `-I` einbinden, damit Warnungen im eigenen Code aktiv bleiben).
- **Keine Unterdrückungen:** Keine `#[allow(...)]`, `#![allow(...)]`, `#pragma`, QML-Filter oder `#[ignore]`, um reale Warnungen oder Tests zu verstecken.
- **Enge Ausnahmen:** Nur dort zulässig, wo eine externe Schnittstelle die Signatur erzwingt (z. B. `org.freedesktop.Notifications.Notify` mit 8 D-Bus-Parametern in Test-Fakes) oder Cargo jedes Test-Binary separat kompiliert (`#![allow(dead_code)]` in `tests/common/mod.rs`); nie modul-/crateweit im Produktionscode.

## Reproduzierbarkeit

`rust-toolchain.toml`, `Cargo.lock`, gepinnte CXX-Qt-Version, `scripts/bootstrap.sh`: installiert Abhängigkeiten (apt, rustup) und lädt restic 0.18.0 und aktuell sowie `rest-server` geprüft (SHA256 + PGP) und rclone ≥ 1.65 (mindestens SHA256; PGP, wenn das Projekt Signaturen veröffentlicht – Ergebnis im Spike-Bericht). Download-Mechanismus wie FR-33 wiederverwenden.

## Grenze der Automatisierung (Q8)

Logik, Integration und Packaging sind voll automatisierbar. Nicht automatisierbar und daher in [manual-acceptance.md](manual-acceptance.md): Look & Feel, Deckel zu während eines Laufs, PowerDevil und Portal-Inhibit (F9), echte KWallet-Entsperrung beim Login bzw. KWallet ohne PAM, Plasma-Autostart-Seite (E11), Hotspot-Erkennung, echtes Netzwerk-Ziel (z. B. NAS), Verhalten unterwegs.
