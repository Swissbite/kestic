# AGENTS.md – Kestic

Einstieg für alle Agenten (Claude Code, Antigravity u. a.). Zuerst lesen, dann nur die Spec-Teile laden, die die aktuelle Story braucht.

## Ziel

Kestic ist eine native KDE-Plasma-Desktop-App, die restic-Backups für Endnutzer einfach macht und gleichzeitig die volle restic-Konfiguration erlaubt («So einfach wie Déjà Dup, so mächtig wie restic»). Stack: Rust + CXX-Qt + Kirigami (QML). Lizenz: `GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL`, REUSE 3.0.

## Spec – was wann laden

| Datei | Inhalt | Laden, wenn … |
| --- | --- | --- |
| [docs/spec/backlog.md](docs/spec/backlog.md) | Meilensteine, Reihenfolge, Definition of Done | immer (kurz) |
| `docs/spec/milestones/<mx>.md` | Stories + Akzeptanzkriterien des aktuellen Meilensteins | immer, nur der aktuelle |
| [docs/spec/glossary.md](docs/spec/glossary.md) | verbindliche Begriffe UI ↔ Code ↔ restic | immer (Begriffe nie selbst erfinden) |
| [docs/spec/requirements.md](docs/spec/requirements.md) | FR/NFR mit IDs, Prioritäten, Festlegungen | die IDs, die die Story nennt |
| [docs/spec/decisions.md](docs/spec/decisions.md), [docs/spec/adr/](docs/spec/adr/) | Entscheide und Begründungen | wenn eine Story einen Entscheid berührt |
| [docs/spec/domain-model.md](docs/spec/domain-model.md) | Ziel, Backup, Lauf, Zustände, Config, Persistenz | `core`-Stories |
| [docs/spec/architecture.md](docs/spec/architecture.md) | Komponenten, Daemon, Packaging, Secrets, Zeitsteuerung, Tech-Stack, Bedrohungsmodell | `daemon`-, Packaging-, System-Stories |
| [docs/spec/restic-integration.md](docs/spec/restic-integration.md) | restic-Befehle, Flags, JSON, Exit-Codes → Meldungen, Mindestversion | alles, was restic aufruft |
| [docs/spec/dbus-api.md](docs/spec/dbus-api.md) | Daemon ↔ GUI | D-Bus-, GUI-Stories |
| [docs/spec/ui/index.md](docs/spec/ui/index.md) + ein Teil aus `docs/spec/ui/` | GUI-Spec (Index + Teil laut «Teile»-Tabelle) | GUI-Stories |
| [docs/spec/test-strategy.md](docs/spec/test-strategy.md) | Testebenen, Fixtures, Isolation | jede Story mit Tests |
| [docs/spec/manual-acceptance.md](docs/spec/manual-acceptance.md) | Prüfungen, die ein Mensch macht | Meilenstein-Abschluss |
| [docs/spec/questions.md](docs/spec/questions.md) | offene Fragen (Stop & Ask) | vor jeder Story kurz prüfen |

Sprache der Spec: Deutsch (Schweizer Rechtschreibung, ohne Eszett). Code, Bezeichner und `msgid` auf Englisch, deutsche Übersetzung in `.po` (U11).

## Arbeitsweise

1. Story aus dem aktuellen Meilenstein wählen (Reihenfolge laut `backlog.md`, Abhängigkeiten beachten).
2. Akzeptanzkriterien der Story lesen; jedes AC wird durch einen automatisierten Test belegt (ausser «Test: manuell»). Test-Namen bzw. Kommentare nennen die AC-ID.
3. Umsetzen in kleinen Schritten; GUI-Schicht dünn, Logik in `core`.
4. `just verify` ausführen. **Grün = Story erledigt.** Sonst nicht als erledigt melden.
5. Manuelle ACs nicht selbst abhaken, sondern in `manual-acceptance.md` offen lassen.

## Prüfbefehl

`just verify` = `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` + `cargo deny check` (Lizenzen + Advisories) + Integrationstests gegen restic 0.18.0 **und** die aktuelle Version + QML-Smoke + Offscreen-Screenshots pro Screen (Artefakt für die menschliche Abnahme) + `reuse lint`.

`justfile`, `rust-toolchain.toml` und `scripts/bootstrap.sh` entstehen in M0 (Stories M0-S01, M0-S02). Bis dahin gibt es keinen Code.

## Harte Regeln

### Test-Isolation

Tests berühren nie echte Benutzerdaten oder -konfiguration:

- `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME`, `XDG_RUNTIME_DIR` zeigen auf ein Tempdir.
- Eigener `dbus-run-session`; nie die D-Bus-Session des Benutzers, nie die echte KWallet.
- Nur Temp-Repos und lokale Testserver auf Zufallsport (`rest-server`, `rclone serve s3`, Benutzer-`sshd`); nie echte Repos.
- SSH nur mit `-o UserKnownHostsFile=<Tempdir-Pfad>`, `-F <Test-ssh_config>`, `-o IdentitiesOnly=yes` (OpenSSH löst `~` über `/etc/passwd` auf, nicht über `$HOME`).
- Nichts unter `/etc/xdg/autostart` voraussetzen oder schreiben; keine echten Autostart- oder systemd-Einträge anlegen.
- Tests laufen nicht als root.

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
