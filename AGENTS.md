# AGENTS.md – Kestic

Einstieg für alle Agenten (Claude Code, Antigravity u. a.). Zuerst lesen, dann nur die Spec-Teile laden, die die aktuelle Story braucht.

## Ziel

Kestic ist eine native KDE-Plasma-Desktop-App, die restic-Backups für Endnutzer einfach macht und gleichzeitig die volle restic-Konfiguration erlaubt («So einfach wie Déjà Dup, so mächtig wie restic»). Stack: Rust + CXX-Qt + Kirigami (QML). Lizenz: `GPL-3.0-or-later`, REUSE 3.0.

## Spec – was wann laden

| Datei | Inhalt | Laden, wenn … |
| --- | --- | --- |
| [docs/spec/backlog.md](docs/spec/backlog.md) | Meilensteine, Reihenfolge, Definition of Done | immer (kurz) |
| `docs/spec/milestones/<mx>.md` | Stories + Akzeptanzkriterien des aktuellen Meilensteins | immer, nur der aktuelle |
| [docs/spec/glossary.md](docs/spec/glossary.md) | verbindliche Begriffe UI ↔ Code ↔ restic | immer (Begriffe nie selbst erfinden) |
| [docs/spec/requirements.md](docs/spec/requirements.md) | FR/NFR mit IDs, Prioritäten, Festlegungen | die IDs, die die Story nennt |
| [docs/spec/decisions.md](docs/spec/decisions.md), [docs/spec/adr/](docs/spec/adr/) | Entscheide und Begründungen | wenn eine Story einen Entscheid berührt |
| [docs/spec/domain-model.md](docs/spec/domain-model.md) | Ziel, Backup, Lauf, Zustände, Config, Persistenz | `core`-Stories |
| [docs/spec/architecture.md](docs/spec/architecture.md) | Komponenten, Daemon, Packaging, Secrets, Zeitsteuerung, Tech-Stack, Code-Struktur, Bedrohungsmodell | `daemon`-, Packaging-, System-Stories, Modulablage |
| [docs/spec/restic-integration.md](docs/spec/restic-integration.md) | restic-Befehle, Flags, JSON, Exit-Codes → Meldungen, Mindestversion | alles, was restic aufruft |
| [docs/spec/dbus-api.md](docs/spec/dbus-api.md) | Daemon ↔ GUI | D-Bus-, GUI-Stories |
| [docs/spec/ui/index.md](docs/spec/ui/index.md) + ein Teil aus `docs/spec/ui/` | GUI-Spec (Index + Teil laut «Teile»-Tabelle) | GUI-Stories |
| [docs/spec/test-strategy.md](docs/spec/test-strategy.md) | Testebenen, AC-Struktur, Testqualität, Fixtures, Isolation | jede Story mit Tests |
| [docs/spec/manual-acceptance.md](docs/spec/manual-acceptance.md) | Prüfungen, die ein Mensch macht | Meilenstein-Abschluss |
| [docs/spec/questions.md](docs/spec/questions.md) | offene Fragen (Stop & Ask) | vor jeder Story kurz prüfen |

Sprache der Spec: Deutsch (Schweizer Rechtschreibung, ohne Eszett). Code, Bezeichner und `msgid` auf Englisch, deutsche Übersetzung in `.po` (U11).

## Skills – wann welchen laden

Projektspezifische Skills liegen agenten-neutral unter [.agents/skills/](.agents/skills/) (via Symlink auch unter `.gemini/skills/` und `.claude/skills/` eingebunden). Vor der Arbeit an passenden Stories die zugehörige `SKILL.md` lesen:

| Skill | Datei | Laden, wenn … |
| --- | --- | --- |
| Story-Orchestrierung | [kestic-story-execution](.agents/skills/kestic-story-execution/SKILL.md) | neue Story oder Meilenstein startet (Subagent-Recherche, Parallelisierung) |
| CXX-Qt & Kirigami | [kestic-cxxqt-kirigami](.agents/skills/kestic-cxxqt-kirigami/SKILL.md) | `gui/`, QML, `QAbstractListModel`, Offscreen-Smoke, Barrierefreiheit (NFR-09) |
| D-Bus, Daemon & Fakes | [kestic-dbus-and-fakes](.agents/skills/kestic-dbus-and-fakes/SKILL.md) | `daemon/`, `zbus`, Tray, Notifications, Secret Service, System-Fakes (UPower/NM/logind/UDisks2) |
| restic-Integration | [kestic-restic-integration](.agents/skills/kestic-restic-integration/SKILL.md) | `restic`-Aufrufe, JSON-Parsing, Exit-Codes, SFTP/S3/REST-Tests, Download/PGP |
| i18n & Packaging | [kestic-i18n-packaging](.agents/skills/kestic-i18n-packaging/SKILL.md) | neue/geänderte UI-Strings (`po/de/kestic.po`), `packaging/`, `.deb` |
| Manuelle Abnahme | [kestic-manual-acceptance](.agents/skills/kestic-manual-acceptance/SKILL.md) | Punkte in [docs/spec/manual-acceptance.md](docs/spec/manual-acceptance.md) mit dem Menschen geprüft werden |

## Arbeitsweise

1. Story aus dem aktuellen Meilenstein wählen (Reihenfolge laut `backlog.md`, Abhängigkeiten beachten).
2. **Recherche & Parallelisierung:** Bei Stories mit mehreren Spec- oder Code-Abhängigkeiten zuerst mit Read-Only-Research-Subagenten eine kompakte AC-/UI-/Glossar-Checkliste extrahieren (falls das Agent-System Subagenten unterstützt). Unabhängige Teilaufgaben (`core` vs. `daemon` vs. `gui`, separate Testdateien) sinnvoll parallelisieren, jedoch nie zwei Schreibzugriffe gleichzeitig auf dieselbe Datei.
3. Akzeptanzkriterien der Story lesen; jedes AC wird durch einen automatisierten Test belegt (ausser «Test: manuell»). Test-Namen bzw. Kommentare nennen die AC-ID; jede nummerierte Teil-Aussage `(a)`, `(b)`, ... hat eine eigene Assertion, ohne `||`-Rückfallebene und ohne `let _ =` auf Messwerte (Details in [docs/spec/test-strategy.md](docs/spec/test-strategy.md)).
4. Umsetzen in kleinen Schritten gemäss [docs/spec/architecture.md](docs/spec/architecture.md) §Code-Struktur (`core/src/<bereich>.rs`, `daemon/src/service/<bereich>.rs`, `gui/src/models/<modell>.rs`, eigene QML-Datei unter `gui/src/qml/` statt `Main.qml`); GUI-Schicht dünn, Logik in `core`; während der Arbeit gezielte Teil-Tests (`cargo test -p …`) für schnelles Feedback nutzen.
5. Vor Abschluss einer GUI-Story erzeugte Offscreen-Screenshots unter `target/smoke-artifacts/` visuell prüfen und neue UI-Strings direkt in `po/de/kestic.po` nachführen.
6. `just verify` ausführen. **Grün = Story erledigt.** Sonst nicht als erledigt melden.
7. Manuelle ACs nicht selbst abhaken, sondern in `manual-acceptance.md` offen lassen (Prüfung mit Schritt-für-Schritt-Anleitung gemäss `kestic-manual-acceptance`).
8. **Meilenstein-Abschluss (Doku-Sync):** Bei Abschluss eines Meilensteins wird das englische [README.md](README.md) (und etwaige englische Einstiegsdoku) mit dem neuen Meilenstein-Stand abgeglichen; `docs/spec/` bleibt dabei einsprachig Deutsch.

## Prüfbefehl

`just verify` = `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` + `cargo deny check` (Lizenzen + Advisories) + Integrationstests gegen restic 0.18.0 **und** die aktuelle Version + QML-Smoke + Offscreen-Screenshots pro Screen (Artefakt für die menschliche Abnahme) + `reuse lint` + Grössenprüfung gemäss [docs/spec/architecture.md](docs/spec/architecture.md) §Code-Struktur.

## Harte Regeln

### Test-Isolation

Tests berühren nie echte Benutzerdaten oder -konfiguration:

- `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME`, `XDG_RUNTIME_DIR` zeigen auf ein Tempdir.
- Eigener `dbus-run-session`; nie die D-Bus-Session des Benutzers, nie die echte KWallet.
- Nur Temp-Repos und lokale Testserver auf Zufallsport (`rest-server`, `rclone serve s3`, Benutzer-`sshd`); nie echte Repos.
- SSH nur mit `-o UserKnownHostsFile=<Tempdir-Pfad>`, `-F <Test-ssh_config>`, `-o IdentitiesOnly=yes` (OpenSSH löst `~` über `/etc/passwd` auf, nicht über `$HOME`).
- Nichts unter `/etc/xdg/autostart` voraussetzen oder schreiben; keine echten Autostart- oder systemd-Einträge anlegen.
- Tests laufen nicht als root.

### Testqualität

- Eine Testfunktion pro AC oder pro Story (keine Sammeltests über mehrere Stories); jede Teil-Aussage `(a)`, `(b)`, ... eines ACs erhält eine eigene Assertion.
- Keine Assertion mit `||` als Rückfallebene, kein `let _ =` auf Messwerte; jeder Test muss fehlschlagen, wenn man das Feature entfernt ([docs/spec/test-strategy.md](docs/spec/test-strategy.md)).

### Zeitquelle (P36)

Keine direkte Systemuhr-Abfrage (`SystemTime::now()`, `OffsetDateTime::now_utc()` o. ä.) in `core` oder `daemon`; jede zeitabhängige Logik nutzt den `Clock`-Trait aus `core/src/clock.rs`.

### Warnungen & Lints (NFR-12)

Jede Warnung in Rust (`clippy -D warnings`), C++ (`build.rs`), QML (`stderr` im Offscreen-Smoke), gettext/PO oder `reuse lint`, die sich sauber beheben lässt, muss an der Ursache behoben werden (Refactoring/Parameter-Structs, toten Code entfernen, Drittanbieter-/Systemheader via `-isystem` einbinden). Keine Warnungsunterdrückungen (`#[allow(...)]`, `#![allow(...)]`, `#pragma`, `#[ignore]` o. ä.) als Abkürzung — Ausnahmen nur lokal für unveränderliche externe Protokollsignaturen (z. B. `org.freedesktop.Notifications.Notify`) oder Cargo-Test-Hilfsmodule (`tests/common/mod.rs`), nie modul-/crateweit im Produktionscode.

Zusätzlich gelten ohne Ausnahmen die Grössengrenzen aus [docs/spec/architecture.md](docs/spec/architecture.md) §Code-Struktur: `too_many_lines` (Schwelle 150 Zeilen in `clippy.toml`) und maximal 1500 Zeilen pro `.rs`- und `.qml`-Datei unter `src/` (ohne Tests und generierten Code), geprüft durch `just verify`.

### Stack- & Werkzeug-Leitplanken (aus M0)

- **Workspace-Grenzen:** Datei-Lesetools (`view_file`) nur im Repo aufrufen; bei Bedarf für System-QML (`/usr/lib/...`) oder `~/.cargo/...` über die Shell (`run_command`) nachschauen oder den passenden Skill laden.
- **Umgebung:** Falls `cargo`/`just` in einer Non-Interactive-Shell fehlen, `~/.cargo/bin` in `PATH` einbinden; `rust-version = "1.85"` in `Cargo.toml` bei `clippy` beachten.
- **D-Bus-Verbindungslebensdauer:** Zustandsbehaftete Handles (`JobViewHandle`, `PortalInhibitGuard`, später `logind`-Inhibitor) müssen ihre `zbus::Connection` dauerhaft halten.
- **i18n-Tests:** `glibc` ignoriert `LANGUAGE` bei `LANG=C.UTF-8` → Tests mit `LANG=en_US.UTF-8` und `LANGUAGE=de|en` ausführen.

### Stop & Ask

Bei diesen Punkten **nicht** selbst entscheiden:

- Spec ändern (inkl. Akzeptanzkriterien),
- neue Abhängigkeit (Crate, Systempaket, Werkzeug),
- lizenzrelevante Crates,
- destruktive Operationen gegen Repos, die keine Temp-Repos sind,
- Abweichen von gepinnten Versionen.

Mechanik: Frage in [docs/spec/questions.md](docs/spec/questions.md) eintragen (Datum, Story, Frage, Optionen, Empfehlung), Story als blockiert markieren, mit der nächsten unabhängigen Story weitermachen.

### Secrets

Secrets nie in argv, Logs, Config oder Testausgaben. Repo-Passwort nur über `RESTIC_PASSWORD_COMMAND` → `kestic secret get <ziel-id>`; Backend-Credentials nur per Umgebung im Kindprozess (NFR-02).

### Begriffe

Nur Begriffe aus [docs/spec/glossary.md](docs/spec/glossary.md). Nicht verwenden: «Profil», «Job», «Ignore», «Advanced-Modus».

### Lizenz

Jede neue Datei mit SPDX-Header gemäss Lizenzregel in [docs/spec/decisions.md](docs/spec/decisions.md); `reuse lint` muss grün sein.
