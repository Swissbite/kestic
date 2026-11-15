# M0 Spike

Teil von [backlog.md](../backlog.md).

**Ziel:** Toolchain trägt; Risiken von CXX-Qt, Packaging, Secret Service, i18n und Daemon/Tray geklärt, bevor Feature-Arbeit beginnt. Ergebnis: Wegwerf-Code, festgelegte Versionen, ADR-0001.

**Voraussetzung:** keine. **Grundlage:** [architecture.md](../architecture.md) (Tech-Stack), Spike-Kriterien unten, [test-strategy.md](../test-strategy.md), E12, E17.

## Stories

| ID | Story |
| --- | --- |
| M0-S01 | Repo-Grundgerüst: Cargo-Workspace `core` / `daemon` / `gui`, `rust-toolchain.toml`, `justfile`, REUSE-Header, `LICENSES/` |
| M0-S02 | `scripts/bootstrap.sh`: Rust-Toolchain, Qt-/KF-Entwicklungspakete, restic 0.18.0 + aktuell, `rest-server`, `rclone` ≥ 1.65 als offizielles Release selbst geladen (Ubuntu 24.04 liefert 1.60.1; für `rclone serve s3` als S3-Testserver und als Ziel-Typ), `openssh-server` aus der Paketquelle (SFTP-Tests mit `sshd` auf Zufallsport ohne Root). Prüfregel: jedes heruntergeladene Binary per SHA256; restic zusätzlich per PGP; rest-server und rclone zusätzlich per PGP, wenn das Projekt Signaturen veröffentlicht (wird hier festgestellt). Fixture «Snapshot-JSON ohne Summary» statt restic-0.16.4-Binary |
| M0-S03 | CXX-Qt-Version festlegen (0.8.1 laut KDE-Tutorial vs. 0.10.0 aktuell) und pinnen |
| M0-S04 | Kirigami-Fenster mit Rust-ListModel (Spike-Kriterium 1) |
| M0-S05 | `.deb` via `cargo-deb` mit manuellem QML-`Depends`, Autostart-Eintrag `/etc/xdg/autostart` (Kriterium 2) |
| M0-S06 | QML-Smoke-Test offscreen + Screenshot (Kriterium 3, Q3) |
| M0-S07 | Daemon-Prototyp mit Tray (`ksni`) und Notification (`notify-rust`), Single-Instance über D-Bus-Namen (Kriterium 4) |
| M0-S08 | Secret-Roundtrip in `dbus-run-session` (Kriterium 5); Fallback Datei-Backend hinter Trait |
| M0-S09 | i18n: je ein String in Rust und QML über KI18n, DE/EN (Kriterium 6) |
| M0-S10 | Prüfungen: Portal-Inhibit + PowerDevil (F9), Erkennung ersetztes Binary nach Paket-Update (E11) |
| M0-S11 | GUI-Grundlagen aus [index.md](../ui/index.md) §15: `Kirigami.NavigationTabBar` + pageStack-Spalten + Breadcrumb in Kirigami 6.24; Tray-Icon-Umfärbung per `IconThemePath`; `org.kde.JobViewServerV2` aus Rust; `notifyrc`-Ereignisse ohne KNotifications; Modulnamen der Systemeinstellungen; Tab-Leiste bei Mindestbreite; SNI `Passive` in Plasma 6.6 (ausgeklappter Bereich); Kürzel Ctrl+Shift+B |

## Akzeptanzkriterien

### M0-S01 Repo-Grundgerüst
- **AC-M0-01-1** (Q1) – *Gegeben* frisch geklonter Workspace mit `core`, `daemon`, `gui`, *wenn* `just verify` läuft, *dann* laufen `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo deny check`, `reuse lint` mit Exit 0 · Test: Integration
- **AC-M0-01-2** – *Gegeben* neue Datei ohne SPDX-Header, *wenn* `just verify` läuft, *dann* schlägt `reuse lint` fehl · Test: Integration

### M0-S02 Bootstrap
- **AC-M0-02-1** (Q2) – *Gegeben* TUXEDO ohne Rust, restic, `rest-server`, `rclone`, *wenn* `scripts/bootstrap.sh` läuft, *dann* sind vorhanden: Rust-Toolchain gemäss `rust-toolchain.toml`, restic 0.18.0 und aktuell, `rest-server`, `rclone` ≥ 1.65 (offizielles Release, nicht Ubuntu-Paket 1.60.1), `sshd`; jedes heruntergeladene Binary per SHA256 geprüft, restic zusätzlich per PGP, rest-server und rclone zusätzlich per PGP, sofern der Spike-Bericht eine veröffentlichte Signatur festhält · Test: manuell (einmalig) + Integration in CI-Container
- **AC-M0-02-4** – *Gegeben* Testumgebung, *wenn* ein S3- bzw. SFTP-Test startet, *dann* läuft `rclone serve s3` bzw. `sshd` als normaler Benutzer auf Zufallsport mit Temp-Schlüsseln und wird nach dem Test beendet · Test: Integration
- **AC-M0-02-2** – *Gegeben* zweiter Lauf von `bootstrap.sh` bei vollständiger Installation, *dann* lädt und ändert das Skript nichts und endet mit Exit 0 · Test: Integration
- **AC-M0-02-3** – *Gegeben* heruntergeladenes Binary mit falscher Prüfsumme, *wenn* `bootstrap.sh` prüft, *dann* Abbruch mit Exit ≠ 0, nichts installiert · Test: Integration (Mock)
- **AC-M0-02-5** (P17) – *Gegeben* die Test-Fixtures, *wenn* `bootstrap.sh` gelaufen ist, *dann* liegt ein Snapshot-JSON ohne Feld `summary` (wie restic vor 0.17) als Fixture im Repo; kein restic-0.16.4-Binary wird geladen · Test: Unit (Fixture vorhanden, parsebar)
- **AC-M0-02-6** – *Gegeben* M0 abgeschlossen, *dann* nennt der Spike-Bericht für rest-server und rclone je «signiert (PGP)» oder «nur SHA256», und `bootstrap.sh` prüft genau so · Test: manuell (Review)

### M0-S03 CXX-Qt-Version
- **AC-M0-03-1** – *Gegeben* M0 abgeschlossen, *dann* ist die CXX-Qt-Version in `Cargo.toml` exakt gepinnt und ADR-0001 begründet die Wahl zwischen 0.8.1 und 0.10.0 · Test: manuell (Review)

### M0-S04 Kirigami mit Rust-ListModel
- **AC-M0-04-1** – *Gegeben* Rust-ListModel mit drei Einträgen, *wenn* das Kirigami-Fenster startet, *dann* zeigt die QML-Liste genau diese drei; die CXX-Qt-Version ist dabei exakt gepinnt (AC-M0-03-1) · Test: GUI-Smoke (offscreen + Screenshot)

### M0-S05 `.deb` und Autostart
- **AC-M0-05-1** (NFR-01) – *Gegeben* mit `cargo-deb` gebautes Paket, *wenn* es auf TUXEDO installiert wird, *dann*: `Depends` enthält `qml6-module-org-kde-kirigami (>= 6.24)`, `qml6-module-org-kde-kirigamiaddons-datetime (>= 1.11)`, `qml6-module-org-kde-kirigamiaddons-tableview (>= 1.11)`; `Recommends` `restic (>= 0.18.0)`; `kestic.notifyrc` liegt unter `/usr/share/knotifications6/`; die App startet aus dem Startmenü; `/etc/xdg/autostart/<app-id>.daemon.desktop` mit `Exec=kestic daemon --autostart` ist vorhanden · Test: manuell + Paketinhalt per `dpkg-deb -I`/`-c` (Integration)
- **AC-M0-05-2** (P34) – *Gegeben* installiertes Paket, *wenn* `apt remove` läuft, *dann* ist `/etc/xdg/autostart/<app-id>.daemon.desktop` als Conffile noch da, enthält `TryExec=/usr/bin/kestic`, und beim nächsten Login startet kein Kestic-Prozess · Test: manuell + Unit (Inhalt des Eintrags)
- **AC-M0-05-3** (P34) – *Gegeben* entferntes Paket, *wenn* `apt purge` läuft, *dann* ist `/etc/xdg/autostart/<app-id>.daemon.desktop` weg; ein Benutzer-Override mit `Hidden=true` in `~/.config/autostart` darf bleiben (wirkungslos ohne Systemeintrag) · Test: manuell

### M0-S06 QML-Smoke und Screenshot
- **AC-M0-06-1** (Q3) – *Gegeben* `QT_QPA_PLATFORM=offscreen`, *wenn* `just verify` läuft, *dann* startet die GUI, lädt das Hauptfenster ohne QML-Fehler und legt einen Screenshot als Artefakt ab · Test: GUI-Smoke

### M0-S07 Daemon-Prototyp
- **AC-M0-07-1** (F14) – *Gegeben* laufender Daemon in `dbus-run-session`, *wenn* ein zweiter startet, *dann* erkennt er den belegten D-Bus-Namen und endet mit Exit 0 · Test: D-Bus (dbus-run-session)
- **AC-M0-07-2** – *Gegeben* laufender Daemon, *wenn* er eine Test-Benachrichtigung sendet, *dann* erhält der Notification-Fake genau einen `Notify`-Aufruf, und ein StatusNotifierItem ist registriert · Test: D-Bus (Fake)

### M0-S08 Secret-Roundtrip
- **AC-M0-08-1** (Q5) – *Gegeben* M0 abgeschlossen, *dann* nennt ADR-0001 genau ein Secret-Backend für automatisierte Tests (headless Keyring in `dbus-run-session` oder Datei-Backend hinter dem Secret-Trait) mit Begründung · Test: manuell (Review)
- **AC-M0-08-2** (Q5) – *Gegeben* das Backend aus ADR-0001, *wenn* die Testsuite ein Secret schreibt und liest, *dann* ist der Wert identisch und der Test in `just verify` grün · Test: Integration

### M0-S09 i18n
- **AC-M0-09-1** (E12, NFR-06) – *Gegeben* je ein übersetzbarer String in Rust und QML mit DE-Übersetzung, *wenn* die App mit `LANGUAGE=de` bzw. `LANGUAGE=en` startet, *dann* erscheinen beide in der jeweiligen Sprache · Test: GUI-Smoke

### M0-S10 Prüfungen
- **AC-M0-10-1** (F9) – *Gegeben* aktiver Portal-Inhibit (Idle + Suspend), *wenn* die PowerDevil-Inaktivitätszeit abläuft, *dann* geht das Laptop nicht in den Ruhezustand; Ergebnis im Abschnitt «Spike-Kriterien und offene Prüfungen» unten · Test: manuell
- **AC-M0-10-2** (E11) – *Gegeben* laufender Daemon, *wenn* sein Binary per `rename` ersetzt wird (wie dpkg), *dann* erkennt der Daemon das; Ergebnis im Abschnitt «Spike-Kriterien und offene Prüfungen» unten · Test: Integration (Temp-Verzeichnis)

### M0-S11 GUI-Grundlagen
Jeder Prüfpunkt: Ergebnis im Spike-Bericht; Abweichungen in [index.md](../ui/index.md) nachgeführt.
- **AC-M0-11-1** – *Gegeben* Prototyp-Fenster mit `Kirigami.NavigationTabBar` (5 Bereiche), `pageStack`-Spalten und Breadcrumb in Kirigami 6.24, *wenn* es offscreen breit und schmal startet, *dann* lädt es ohne QML-Fehler, Screenshots zeigen Tab-Leiste und Breadcrumb (schmal: Tab-Leiste unten) · Test: GUI-Smoke
- **AC-M0-11-2** – *Gegeben* Daemon-Prototyp, *wenn* er über `org.kde.JobViewServerV2.requestView` (zbus, aus Rust) eine Fortschrittsanzeige anmeldet und Fortschritt meldet, *dann* zeigt Plasma den Fortschritt mit «Abbrechen» im Benachrichtigungsbereich · Test: manuell + D-Bus (Fake)
- **AC-M0-11-3** – *Gegeben* Tray-Icon mit eigenem `IconThemePath`, *wenn* zwischen hellem und dunklem Farbschema gewechselt wird, *dann* passt sich die Icon-Farbe an · Test: manuell
- **AC-M0-11-4** (U6) – *Gegeben* installierte `kestic.notifyrc` und Benachrichtigungen über `org.freedesktop.Notifications` mit Hint `desktop-entry`, ohne KNotifications, *wenn* der Benutzer in den Systemeinstellungen ein Ereignis abschaltet, *dann* kommt dafür kein Popup mehr; Ergebnis (wirkt / wirkt nicht) im Spike-Bericht · Test: manuell
- **AC-M0-11-5** (K1) – *Gegeben* Daemon-Prototyp in Plasma 6.6, *wenn* er den SNI-Status von `Active` auf `Passive` und zurück setzt, *dann* wandert das Icon in den ausgeklappten Bereich und bei `Active` wieder sichtbar ins Panel · Test: manuell
- **AC-M0-11-6** – *Gegeben* die Aktionen «Autostart-Einstellungen» und «Benachrichtigungs-Einstellungen», *wenn* ausgelöst, *dann* öffnet sich jeweils das richtige Modul der Systemeinstellungen; Modulnamen im Spike-Bericht · Test: manuell
- **AC-M0-11-7** – *Gegeben* Prototyp-Fenster in Mindestgrösse (26 `gridUnit` breit), *wenn* offscreen in DE und EN gerendert, *dann* ist kein Tab-Label abgeschnitten (insbesondere «Wiederherstellen») · Test: GUI-Smoke (Screenshot je Sprache)
- **AC-M0-11-8** (U12) – *Gegeben* Prototyp-Fenster, *wenn* Ctrl+Shift+B gedrückt wird, *dann* löst es «Jetzt sichern» aus; ein Konflikt mit einem globalen Plasma-Kürzel ist im Spike-Bericht festgehalten oder ausgeschlossen · Test: GUI-Smoke (Tastendruck) + manuell (Konfliktprüfung)

## Exit-Kriterien

- Spike-Kriterien 1–7 (unten) grün (Kriterium 7 = GUI-Grundlagen, M0-S11).
- ADR-0001 (Stack und Versionen) geschrieben, Versionen gepinnt.
- Offene Prüfungen (unten) mit Ergebnis dokumentiert.
- Testserver festgelegt: S3 über `rclone serve s3` (experimentell; Ersatz bei Problemen: Garage), SFTP über Benutzer-`sshd`. MinIO entfällt (seit Ende 2025 keine Community-Binaries mehr).

## Spike-Kriterien und offene Prüfungen

**Spike-Kriterien** – alle grün, dann Versionen pinnen und ADR-0001 finalisieren:

1. Kirigami-Fenster mit Rust-ListModel (CXX-Qt), Version festgelegt (0.8.1 laut KDE-Tutorial vs. 0.10.0 aktuell).
2. `.deb` via `cargo-deb` mit manuellem QML-`Depends` gebaut, installiert, gestartet.
3. QML-Smoke-Test mit `QT_QPA_PLATFORM=offscreen` läuft in `just verify`.
4. Daemon per XDG-Autostart gestartet, Tray-Icon (`ksni`) sichtbar, Notification über `notify-rust`.
5. Secret-Roundtrip (schreiben/lesen) in `dbus-run-session` mit headless Keyring; akzeptierter Fallback: Datei-Backend hinter dem Secret-Trait für Tests (Q5).
6. i18n: ein übersetzter String in Rust und in QML über den gewählten Mechanismus (E12, Q5).
7. GUI-Grundlagen aus [ui/index.md](../ui/index.md): `Kirigami.NavigationTabBar` + Breadcrumb, Tab-Leiste bei Mindestbreite, SNI `Passive` in Plasma 6.6, Icon-Umfärbung per `IconThemePath`, `org.kde.JobViewServerV2` aus Rust, `notifyrc` ohne KNotifications, Modulnamen der Systemeinstellungen, Kürzel Ctrl+Shift+B (M0-S11).

«Nachholen nach Suspend» ist Feature-Arbeit, kein Spike-Kriterium (Q5).

**Noch offene Prüfungen** (alle übrigen Annahmen sind in [restic-integration.md](../restic-integration.md) und [architecture.md](../architecture.md) geprüft):

| Punkt | Stand |
| --- | --- |
| Headless Keyring in `dbus-run-session` | erster Versuch (gnome-keyring + Python `secretstorage`) hing nach 30 s, vermutlich Entsperr-Dialog → im Spike lösen (z. B. Collection vorab anlegen, oder Trait mit Datei-Backend für Tests) |
| QML-Offscreen-Smoke-Test (Qt-QML-Werkzeuge und Rust-Toolchain sind auf dem Referenzsystem vorab zu installieren), `cargo-deb` + CXX-Qt-Build | Spike |
| KWallet-Abfrage ohne PAM-Entsperrung | manuelle Abnahme |
| PowerDevil respektiert das Portal-Inhibit bei Inaktivität (F9) | Spike / manuelle Abnahme |
| Plasma-Autostart-Seite schaltet `/etc/xdg/autostart`-Eintrag per `Hidden=true` ab (E11) | manuelle Abnahme |
| Erkennung ersetztes Binary nach Paket-Update (E11) | Spike |
| SNI `Passive` zeigt den Eintrag im ausgeklappten Bereich (K1) | Spike |
| GUI-Grundlagen (Kriterium 7) | Spike |
| rest-server und rclone veröffentlichen PGP-Signaturen | Spike (M0-S02) |
| libsecret im Flatpak (Speicherort) | Phase 2 |
