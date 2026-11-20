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

## Belege nach Entfernen des Spike-Fensters

Das Prototyp-Fenster des Spikes (`SpikeMain.qml`, `SpikeListModel`, `kestic --smoke-test`) ist seit 2026-11-20 entfernt; die GUI hat nur noch das Produktfenster `Main.qml`. Die M0-ACs, die das Prototyp-Fenster prüften, belegen jetzt diese Tests am Produktfenster:

| AC | Test |
| --- | --- |
| AC-M0-04-1 | `daemon/tests/m3_gui_smoke.rs` `every_page_shows_simple_texts_in_the_gui_language` (Liste `backupsListView` zeigt genau die Zeilen des Rust-Modells `backupsModel`); Version gepinnt: `core/tests/m0_s03_cxx_qt_pin.rs` |
| AC-M0-06-1 | `just smoke` (Hauptfenster offscreen, Screenshot `target/smoke-artifacts/m0-s06-main.png`) und jeder GUI-Smoke in `core/tests/m1_gui_smoke.rs` (Abbruch bei QML-Warnung) |
| AC-M0-09-1 | `core/tests/m1_s16_i18n.rs` `ac_m1_16_1_and_16_2_ki18n_en_and_de_in_gui_and_daemon` und `every_page_shows_simple_texts_in_the_gui_language` (Texte aus Rust und QML auf Englisch und Deutsch); Lint: `core/tests/m0_s09_i18n.rs` `nfr_06_gettext_xgettext_and_po_lint` |
| AC-M0-11-1, AC-M0-11-7 | `core/tests/m1_gui_smoke.rs` `m1_gui_smoke_welcome_wide_narrow_en_and_de` (Tab-Leiste breit oben, schmal unten, kein Label abgeschnitten, EN und DE; bis M3 vier Bereiche, «Wiederherstellen» kommt mit M4) |
| AC-M0-11-8 | `daemon/tests/m3_gui_smoke.rs` `ac_m0_11_8_ctrl_shift_b_starts_back_up_now_in_the_detail_page` (Tastendruck im Backup-Detail startet «Jetzt sichern», die Aktion zeigt das Kürzel; auf der Übersicht startet es nichts). Konfliktprüfung mit globalen Plasma-Kürzeln: im Spike manuell abgenommen. |

## Exit-Kriterien

- Spike-Kriterien 1–7 (unten) grün (Kriterium 7 = GUI-Grundlagen, M0-S11).
- ADR-0001 (Stack und Versionen) geschrieben, Versionen gepinnt.
- Offene Prüfungen (unten) mit Ergebnis dokumentiert.
- Testserver festgelegt: S3 über `rclone serve s3` (experimentell; Ersatz bei Problemen: Garage), SFTP über Benutzer-`sshd`. MinIO entfällt (seit Ende 2025 keine Community-Binaries mehr).

## Spike-Kriterien und offene Prüfungen

**Spike-Kriterien** – alle grün (Versionen gepinnt und ADR-0001 finalisiert):

1. **Grün:** Kirigami-Fenster mit Rust-ListModel (`CXX-Qt =0.10.0`), Version festgelegt und begründet (ADR-0001).
2. **Grün:** `.deb` via `cargo-deb 3.8.0` mit manuellem QML-`Depends` gebaut, auf TUXEDO OS 24.04.5 installiert, aus Startmenü gestartet, `apt remove` (`TryExec`-Conffile erhalten) und `apt purge` (Conffile entfernt) manuell abgenommen (`AC-M0-05-1`–`05-3`).
3. **Grün:** QML-Smoke-Test mit `QT_QPA_PLATFORM=offscreen` + Screenshot-Artefakten läuft in `just verify` (`AC-M0-06-1`).
4. **Grün:** Daemon-Prototyp mit Single-Instance über D-Bus (`net.daester.Kestic`), Tray-Icon (`ksni 0.3.7`) und Notification (`notify-rust 4.19.0`) im `dbus-run-session`-Test belegt (`AC-M0-07-1`, `AC-M0-07-2`).
5. **Grün:** Secret-Roundtrip (schreiben/lesen/ersetzen/löschen) in `dbus-run-session` mit headless `gnome-keyring-daemon` und direktem `zbus 6.3.0`-Client `SecretServiceStore`; `InMemorySecretStore`/`FileSecretStore` als reine Unit-Test-Backends hinter `SecretStore` (`AC-M0-08-1`, `AC-M0-08-2`).
6. **Grün:** i18n über KI18n (`cxx-kde-frameworks` rev `5c51af42`) in Rust (`SpikeListModel`) und QML (`Main.qml`) mit `LANGUAGE=en` und `LANGUAGE=de` belegt (`AC-M0-09-1`); String-Lint über GNU gettext (`xgettext`, `msgcmp`, `msgattrib`, `msgfmt --check`) festgelegt und in `just verify` integriert (`NFR-06`).
7. **Grün:** GUI-Grundlagen (`AC-M0-11-1`–`11-8`): `NavigationTabBar` + Breadcrumb, Tab-Leiste bei 26 `gridUnit` EN & DE, SNI `Passive`/`Active`/`NeedsAttention` + `IconThemePath`-Umfärbung in Plasma 6.6, `JobViewServerV2` aus Rust inkl. «Abbrechen», `notifyrc`-Prüfung (ergibt «wirkt nicht» → F3-Schalter in Kestic nötig), KCM-Modulnamen, Kürzel `Ctrl+Shift+B`.

«Nachholen nach Suspend» ist Feature-Arbeit, kein Spike-Kriterium (Q5).

**Noch offene Prüfungen** (alle Spike-relevanten Annahmen geprüft; offene Punkte liegen erst in späteren Phasen):

| Punkt | Stand |
| --- | --- |
| Headless Keyring in `dbus-run-session` | **geprüft (M0-S08):** gelöst über `gnome-keyring-daemon --foreground --replace --unlock --components=secrets --control-directory <kurzer /tmp-Pfad>` mit leerem Passwort auf `stdin` und bereinigten Host-Env-Vars (`GNOME_KEYRING_CONTROL`, `SSH_AUTH_SOCK`, `DBUS_SESSION_BUS_ADDRESS`; kurzer Pfad nötig wegen 107-Byte-`sun_path`-Limit des `control`-Sockets). Direkter `org.freedesktop.secrets`-Client (`SecretServiceStore`) über `zbus 6.3.0` (`plain`-Session, kein `zbus 5` / keine Pre-Release-Abhängigkeit) schliesst Set/Get/Replace/Delete in 0,10 s ohne Hänger ab (`AC-M0-08-1`, `AC-M0-08-2`); `FileSecretStore` und `InMemorySecretStore` liegen zusätzlich hinter dem `SecretStore`-Trait (`core/src/secrets.rs`), gesperrte Collections liefern sauber `LockedOrPromptRequired` |
| QML-Offscreen-Smoke-Test, KI18n & `cargo-deb` + CXX-Qt-Build | **geprüft (M0-S03–S06, M0-S09):** `cxx-qt =0.10.0` (inkl. `cxx-qt-lib-extras` für `QApplication` + `org.kde.desktop`) und `cxx-kde-frameworks` (`5c51af42`, KI18n) bauen warnungsfrei mit `lld` gegen Qt 6.10.2 / KF 6.24; `QT_QPA_PLATFORM=offscreen` + `QQuickWindow::grabWindow()` rendert EN & DE ohne QML-Warnungen als PNG-Artefakte in `just verify` (`AC-M0-06-1`, `AC-M0-09-1`; Hinweis: glibc/gettext ignoriert `LANGUAGE` bei `LANG=C.UTF-8`, daher Tests mit `LANG=en_US.UTF-8`). `NFR-06`-String-Lint über `xgettext` (`--language=C` für Rust + `--language=JavaScript` für QML) + `msgcmp` + `msgattrib` + `msgfmt --check` läuft in `just verify`. `cargo-deb 3.8.0` erzeugt `.deb` inkl. QML-`Depends`, `Recommends`, `kestic.notifyrc`, `kestic.mo`, `kestic-symbolic.svg` und `/etc/xdg/autostart`-Conffile; Installation, Startmenü-Start, `apt remove` und `apt purge` auf TUXEDO OS 24.04.5 manuell abgenommen (`AC-M0-05-1`–`05-3`) |
| KWallet-Abfrage ohne PAM-Entsperrung | **teilgeprüft (M1):** `LockedOrPromptRequired` + interaktives `org.freedesktop.Secret.Prompt` (`unlock_interactive()`) per D-Bus-Fake (`daemon/tests/m0_s08_checks.rs`) und beim Live-Unlock auf TUXEDO OS bestätigt; isolierter Test ohne PAM-Entsperrung bleibt für M6 |
| PowerDevil respektiert das Portal-Inhibit bei Inaktivität (F9) | **geprüft (M0-S10, manuell abgenommen):** `org.freedesktop.portal.Inhibit` (`flags = 12`, Idle + Suspend) inkl. `Request.Close()` über `zbus 6.3.0` im D-Bus-Test **und** live unter PowerDevil (1-Minuten-Ruhezustand verhindert, `ListInhibitions` aktiv) belegt (`AC-M0-10-1`). **Wichtiges Finding:** `PortalInhibitGuard` muss die `zbus::Connection` dauerhaft halten, sonst hebt `xdg-desktop-portal` den Inhibit beim Verbindungsabbau sofort auf |
| Plasma-Autostart-Seite schaltet `/etc/xdg/autostart`-Eintrag per `Hidden=true` ab (E11) | **teilgeprüft (M0-S05, M0-S11, M1-S17):** `apt remove`/`apt purge` auf TUXEDO OS erneut manuell abgenommen (`AC-M1-17-4`, `AC-M1-17-5`) und Öffnen von `kcm_autostart` belegt (`AC-M0-11-6`); manuelles Abschalten in `kcm_autostart` (`Hidden=true` in `~/.config/autostart`) als XDG-Standard noch nicht separat getestet |
| Erkennung ersetztes Binary nach Paket-Update (E11) | **geprüft (M0-S10):** Erkennung über `/proc/self/exe` (`" (deleted)"`) + Inode-Vergleich `(st_dev, st_ino)` per Integrationstest mit atomarem `rename` im Temp-Verzeichnis belegt (`AC-M0-10-2`); funktioniert ohne zusätzliche `inotify`-Abhängigkeit |
| SNI `Passive` zeigt den Eintrag im ausgeklappten Bereich (K1) | **geprüft (M0-S07, M0-S11, manuell abgenommen):** Single-Instance-Lock (`net.daester.Kestic`), `ksni 0.3.7` (`IconThemePath`, `kestic-symbolic.svg`) und `notify-rust 4.19.0` im `dbus-run-session`-Test belegt (`AC-M0-07-1`, `AC-M0-07-2`); live in Plasma 6.6.6 wandert `Passive` ins `^`-Überlaufmenü und `Active`/`NeedsAttention` zurück ins Panel (`AC-M0-11-5`) |
| GUI-Grundlagen (Kriterium 7) | **vollständig geprüft (M0-S11, automatisch + manuell abgenommen):** `NavigationTabBar` (5 Bereiche) + `pageStack`-Spalten + `ApplicationHeaderStyle.Breadcrumb` in Kirigami 6.24 ohne QML-Warnungen (`AC-M0-11-1`; `comp.createObject(pageStack, props)` nötig unter Qt 6.10). Bei 26 `gridUnit` Breite ist kein EN- oder DE-Tab-Label abgeschnitten (`AC-M0-11-7`; Hinweis: `NavigationTabBar.maximumContentWidth = gridUnit * 5 * count` bricht «Wiederherstellen» in zwei Zeilen `Wiederherstel` / `len` um → für M1 breiteres `maximumContentWidth` im Wide-Layout vorsehen). `kestic-symbolic.svg` (`.ColorScheme-Text`) färbt sich bei Breeze ↔ Breeze Dunkel live um (`AC-M0-11-3`). `org.kde.JobViewServerV2` → `org.kde.JobViewV3` zeigt live Fortschritt, Geschwindigkeit und löst «Abbrechen» (`cancelRequested`) aus (`AC-M0-11-2`; **wichtiges Finding:** `JobViewHandle` muss die `zbus::Connection` dauerhaft halten, sonst meldet `kuiserver` sofort *Fehlgeschlagen*). `kestic.notifyrc` ohne KNotifications ergibt **wirkt nicht** (`AC-M0-11-4`: erscheint unter *Systemdienste*, `desktop-entry` unter *Anwendungen*, Ereignis-Schalter unterdrückt `notify-rust` nicht → F3-Schalter in Kestic nötig). KCM-Module `kcm_autostart` und `kcm_notifications` öffnen beide korrekt (`AC-M0-11-6`). `Ctrl+Shift+B` löst im GUI-Smoke und live «Jetzt sichern» aus ohne Konflikt (`AC-M0-11-8`) |
| rest-server und rclone veröffentlichen PGP-Signaturen | **geprüft (M0-S02, manuell abgenommen):** `rest-server` v0.14.0 ist **signiert (PGP)** (`SHA256SUMS` + `SHA256SUMS.asc`, Key `CF8F 18F2 8445 7597 3F79 D4E1 91A6 868B D3F7 A907`, identisch mit restic); `rclone` v1.76.0 ist **signiert (PGP)** (clearsigned `SHA256SUMS`, Key `FBF7 37EC E9F8 AB18 604B D2AC 9393 5E02 FF3B 54FA`, Nick Craig-Wood). `scripts/bootstrap.sh` prüft beide per PGP-Fingerprint + SHA256 (`AC-M0-02-6`) |
| libsecret im Flatpak (Speicherort) | Phase 2 |
