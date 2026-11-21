# Teststrategie

Jedes Akzeptanzkriterium (AC) in [milestones/](milestones/) wird durch einen automatisierten Test belegt, ausser es ist als «Test: manuell» markiert (dann [manual-acceptance.md](manual-acceptance.md)). Rückverfolgung: FR/NFR → Story → AC → Test; Test-Namen oder Kommentare nennen die AC-ID.

## Prüfbefehl

`just verify` = `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` + `cargo deny check` (Lizenzen + Advisories, Q1) + Integrationstests gegen restic 0.18.0 **und** die aktuelle Version (`scripts/bootstrap.sh` lädt beide, Q2) + QML-Smoke + Offscreen-Screenshots pro Screen als Artefakt für die menschliche Abnahme (Q3) + `reuse lint` + Grössenprüfung gemäss [architecture.md](architecture.md) §Code-Struktur. Grün = Story erledigt.

## AC-Struktur und Testqualität

- **Nummerierte Teil-Aussagen:** Jede Aussage nach «dann» in Akzeptanzkriterien wird einzeln mit `(a)`, `(b)`, `(c)`, ... nummeriert; jede Teil-Aussage hat im zugehörigen Test eine eigene Assertion.
- **Test-Zuschnitt:** Eine Testfunktion pro AC oder pro Story; keine Sammeltests über mehrere Stories hinweg.
- **Strenge Assertions:** Keine Assertion mit `||` als Rückfallebene und kein `let _ =` auf Messwerte oder Prüfergebnisse.
- **Wirksamkeitsprobe:** Jeder Test muss fehlschlagen, wenn man das geprüfte Feature entfernt.

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

- restic gegen Temp-Repo. Pro Testlauf und restic-Version legt `kestic-test-support` einmal ein Vorlage-Repo an (`restic init`, Schlüssel mit billigen scrypt-Parametern); Tests kopieren es (`copy_template_repo`) und erhalten pro Kopie ein eigenes Passwort und eine eigene Repo-ID. Ein echtes `restic init` läuft nur in Tests, die das Anlegen eines Repos prüfen.
- Tests, die das `kestic`-Binary starten (GUI-Smoke, `kestic secret get`, Tray mit `--simulate-status`), brauchen den Build mit dem Cargo-Feature `test-harness` (`cargo build --bin kestic --features kestic-gui/test-harness`, im `justfile` eingebaut); ohne ihn bricht `find_kestic_binary` mit einem Hinweis ab.
- Testpasswörter und Backend-Zugangsdaten werden zur Laufzeit erzeugt (`strong_password`: Shell-Sonderzeichen, Anführungszeichen, Leerzeichen innen, Umlaute, Emoji), nie als Literal im Test. Leck-Prüfungen nutzen `assert_no_secret`, das auch maskierte Formen findet und das Geheimnis nie ausgibt.
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
- **Grössengrenzen ohne Ausnahme:** Funktionslänge über Clippy-Lint `too_many_lines` (Schwelle 150 Zeilen in `clippy.toml`) und maximale Dateigrösse von 1500 Zeilen pro `.rs`- und `.qml`-Datei unter `src/` (ohne Tests und generierten Code), geprüft durch `just verify` gemäss [architecture.md](architecture.md) §Code-Struktur.

## Reproduzierbarkeit

`rust-toolchain.toml`, `Cargo.lock`, CXX-Qt-Version, `scripts/bootstrap.sh`: installiert Abhängigkeiten (apt, rustup) und lädt restic 0.18.0 und aktuell sowie `rest-server` geprüft (SHA256 + PGP) und rclone ≥ 1.65 (mindestens SHA256; PGP, wenn das Projekt Signaturen veröffentlicht – Ergebnis im Spike-Bericht). Download-Mechanismus wie FR-33 wiederverwenden.

## Lehren aus der manuellen Abnahme (Entscheid 2026-11-20)

- Jede Erkenntnis aus einer manuellen Abnahme wird, wo möglich, ein automatisierter Test (Name mit `acceptance`; der Kommentar beschreibt das geprüfte Verhalten, Datum und Befund stehen in Commit-Message und [manual-acceptance.md](manual-acceptance.md)).
- Fakes können neben dem Normalfall auch **Schwall** (viele Signale kurz hintereinander), **Verzögerung** (langsame oder ausbleibende Antwort) und **Fehler** (typische restic-Meldungen wie «no space left on device»).
- Configs **aller Vorversionen** bleiben als Fixtures erhalten und werden in einem Test geladen (Migration).
- Smoke-Screenshots entstehen in **hell und dunkel**, mit fokussierten Elementen, dazu Text-Snapshots der sichtbaren Texte (de/en). Zugängliche Namen prüft der Smoke über Qts Accessibility-Schnittstelle (`QAccessible`, wie über AT-SPI exportiert); eine automatische Kontrastprüfung folgt, wenn die Laufzeit vertretbar bleibt. Orca bleibt eine Stichprobe in der manuellen Abnahme.

## Grenze der Automatisierung (Q8)

Logik, Integration und Packaging sind voll automatisierbar. Nicht automatisierbar und daher in [manual-acceptance.md](manual-acceptance.md): Look & Feel, Deckel zu während eines Laufs, PowerDevil und Portal-Inhibit (F9), echte KWallet-Entsperrung beim Login bzw. KWallet ohne PAM, Plasma-Autostart-Seite (E11), Hotspot-Erkennung, echtes Netzwerk-Ziel (z. B. NAS), Verhalten unterwegs.
