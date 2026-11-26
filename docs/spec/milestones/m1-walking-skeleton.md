# M1 Walking Skeleton

Teil von [backlog.md](../backlog.md).

**Ziel:** Ein Backup läuft durch alle Schichten: GUI → D-Bus → Daemon → restic → Historie → Tray und Notification. Minimal, aber echt und als `.deb` installierbar.

**Voraussetzung:** M0. **Umfang:** Ziel-Typen Lokal und REST-Server; ein Backup mit einer Quelle; manueller Start; Ausschluss-Regeln nur als Textfeld (Expertensyntax).

## Anforderungen

FR-01 (Lokal, REST), FR-08, FR-13b, FR-15, FR-18, FR-21, FR-28, FR-33 (nur Erkennung System-restic + eigener Pfad), FR-30 (Verfügbarkeitsstatus, manueller Start), FR-34, FR-37 (Grundgerüst), FR-42, FR-56, FR-57 · NFR-01, NFR-02, NFR-03, NFR-04, NFR-05, NFR-06 (Mechanismus), NFR-07, NFR-10, NFR-11 (Grundform) · F13, F14, F15, F17, F19 · P24, P31, P32, P34, P36, P38, P41 · K1, K3, U6, U18, U19 · P39, P40

## Stories

| ID | Story |
| --- | --- |
| M1-S01 | `core`: Domänenmodell Ziel / Backup / Lauf, IDs (F15), Glossar-Begriffe als Typen |
| M1-S02 | `core`: Config TOML lesen/schreiben mit `config_version` (F13, NFR-03) |
| M1-S03 | `core`: restic-Runner – Aufruf bauen (Binary, Argumente, Env, `nice`/`ionice`), JSON-Stream tolerant parsen, Exit-Codes abbilden (NFR-10, FR-42) |
| M1-S04 | `core`: restic-Quelle – System-restic finden, Version per `version --json`, Fallback Text von `restic version` (P31), Mindestversion 0.18.0, eigener Pfad (FR-33 Teil, NFR-05) |
| M1-S05 | `core`: Secrets über Trait + Secret Service; Helfer `kestic secret get` (FR-08, NFR-02) |
| M1-S06 | `core`: Ziel anlegen (`init`) oder verbinden (Passwort prüfen) für Lokal und REST mit `rest-server`; Vorab-Check «Ziel erreichbar» (FR-01, FR-15, P32) |
| M1-S07 | `core`: Backup mit Tag `kestic:<backup-id>`, `--group-by tags`, Ausschluss-Datei unter `~/.local/state/kestic/excludes/` (FR-28, F13) |
| M1-S08 | `core`: Lauf-Historie (SQLite) und Log pro Lauf, Secrets redacted, Log-Aufbewahrung (FR-18, F13) |
| M1-S09 | `daemon`: D-Bus-API v1 (F14) – Ziele/Backups verwalten, Lauf starten/abbrechen, Status-Signale; schreibt als Einziger Config und Zustand |
| M1-S10 | `daemon`: Lauf abbrechen (SIGTERM, restic gibt den Lock frei), Status `abgebrochen`; beim Start `läuft` → `unterbrochen` (FR-21) |
| M1-S11 | `daemon`: Tray mit Status (SNI `Passive` / `Active` / `NeedsAttention`, K1 = B), «Jetzt sichern», «Kestic öffnen», «Kestic beenden…» mit Warnung (NFR-11); Notification bei Fehler/Warnung (F3), `kestic.notifyrc` mit Hint `desktop-entry` (U6) |
| M1-S12 | `daemon`: Autostart-Modus `--autostart` (ohne Config sofort beenden), sonst bleiben (E11) |
| M1-S13 | `gui`: Hauptfenster mit `Kirigami.NavigationTabBar` (K3), in M1 4 Bereiche (Übersicht, Backups, Ziele, Verlauf; «Wiederherstellen» kommt mit M4, U1 in [ui/index.md](../ui/index.md)); Hamburger-Menü; Zustandsleiste im Fensterkopf mit blockierenden Zuständen Daemon → Passwortspeicher → restic und Selbsttest beim Start (U19); Listen der Ziele und Backups, Formulare Ziel (Lokal, REST) und Backup (Quelle, Ausschluss-Textfeld), «Jetzt sichern», Verlauf |
| M1-S14 | `gui`: Befehlstransparenz im ⋮-Menü – Befehl anzeigen und kopieren, ohne Secrets (FR-34, F17, K2) |
| M1-S15 | Fehlertexte Grundgerüst: Exit 1/3/10/11/12 → Text + Aktion (FR-37) |
| M1-S16 | i18n: alle Strings übersetzbar (DE/EN) (NFR-06) |
| M1-S17 | `.deb` mit Daemon-Autostart (`TryExec=/usr/bin/kestic`, P34), Kirigami- und Kirigami-Addons-`Depends`, `kestic.notifyrc` (U6), `Recommends: restic (>= 0.18.0)` (NFR-01) |
| M1-S18 | `core`: Clock-Trait – injizierbare Uhr für alle zeitabhängigen Teile (P36) |
| M1-S19 | Ziel und Backup bearbeiten und entfernen (FR-56, P39, U18): explizites Speichern über `UpdateBackup` / `UpdateDestination`, Umbenennen per Dialog mit einem Feld, «Aus Kestic entfernen…» (Backup) und «Ziel entfernen» per Bestätigungsdialog ohne Dry-Run |
| M1-S20 | Gespeichertes Passwort eines Ziels ändern (FR-57, P40) |
| M1-S21 | `gui`: startet den Daemon bei Bedarf, Warnung «Hintergrunddienst läuft nicht» |
| M1-S22 | Backup einem anderen Ziel zuweisen (FR-56) |
| M1-S23 | `daemon`: Verfügbarkeitsstatus für Quelle und Ziel (Grundlage für deaktiviertes «Jetzt sichern», FR-30): Quelle per Pfadprüfung (Laufwerk-Erkennung mit FS-UUID und «eingehängt» folgt in M2-S09), Ziel aus dem letzten Vorab-Check (P32), aktualisiert beim Öffnen von Fenster oder Tray-Menü, höchstens alle 60 s; Aktualisierung bei Netzwerkwechsel folgt in M3 |

## Akzeptanzkriterien

Test-Isolation gemäss [test-strategy.md](../test-strategy.md); Temp-Repos, lokaler `rest-server` auf Zufallsport; SSH zusätzlich mit `-F <Test-ssh_config>` und `-o IdentitiesOnly=yes` (AC-M1-Q-1). «Integration» = gegen restic 0.18.0 **und** aktuell (Q2).

### M1-S01 Domänenmodell
- **AC-M1-01-1** (F15) – *Gegeben* neues Backup und neues Ziel, *wenn* angelegt, *dann* erhält jedes eine ID aus 8 Zeichen Base32 (`[a-z2-7]{8}`); zwei nacheinander erzeugte IDs sind verschieden · Test: Unit
- **AC-M1-01-2** (F15) – *Gegeben* bestehendes Backup, *wenn* umbenannt oder Quelle geändert, *dann* bleibt die ID gleich · Test: Unit
- **AC-M1-01-3** (Glossar, P38) – *Gegeben* das Modell, *wenn* ein Lauf erzeugt wird, *dann* hat er genau eine Art aus `RunKind::Backup`, `RunKind::Maintenance`, `RunKind::Restore`, `RunKind::Meta`; Backup-Lauf gehört zu genau einem Backup; Restore-Lauf zu genau einem Ziel, mit Backup-Bezug nur, wenn der Snapshot einem Backup zugeordnet ist (fremde und verwaiste Snapshots: keiner); Wartungs- und Metadaten-Läufe zu genau einem Ziel; Status ist einer von `ausstehend`, `läuft`, `ok`, `Warnung`, `Fehler`, `abgebrochen`, `unterbrochen` · Test: Unit

### M1-S02 Config TOML
- **AC-M1-02-1** (F13, NFR-03) – *Gegeben* Ziel und Backup, *wenn* Config gespeichert und geladen wird, *dann* sind alle Felder identisch (Roundtrip); Datei `$XDG_CONFIG_HOME/kestic/config.toml` mit `config_version` · Test: Unit
- **AC-M1-02-2** (NFR-03, FR-08) – *Gegeben* Ziel mit Repo-Passwort und REST-Zugangsdaten, *wenn* gespeichert, *dann* enthält `config.toml` keinen Secret-Wert, nur Secret-Referenzen · Test: Unit
- **AC-M1-02-3** (F13) – *Gegeben* `config.toml` mit unbekannter, höherer `config_version`, *wenn* geladen, *dann* lehnt Kestic sie mit Fehlermeldung ab und überschreibt sie nicht · Test: Unit

### M1-S03 restic-Runner
- **AC-M1-03-1** (NFR-10) – *Gegeben* restic-JSON mit unbekanntem Feld und unbekanntem `message_type`, *wenn* geparst, *dann* werden beide ignoriert und bekannte Felder (z. B. Snapshot-ID, Dateizahlen) korrekt gelesen · Test: Unit
- **AC-M1-03-2** (NFR-10) – *Gegeben* Exit 0, 1, 3, 10, 11, 12 bzw. undokumentiert (z. B. 99), *wenn* der Runner abbildet, *dann* `ok`, `Fehler`, `Warnung`, `Fehler` (Repo fehlt), `Fehler` (Lock), `Fehler` (falsches Passwort) bzw. `Fehler` mit Rohmeldung · Test: Unit + Integration
- **AC-M1-03-3** (FR-42, P3) – *Gegeben* Backup-Lauf, *wenn* restic startet, *dann* zeigt Feld 19 von `/proc/<pid>/stat` nice 10, und `ioprio_get` bzw. `ionice -p <pid>` meldet IO-Klasse idle · Test: Integration
- **AC-M1-03-4** (NFR-02) – *Gegeben* Lauf mit Repo- und REST-Passwort, *wenn* restic läuft, *dann* steht keines in `/proc/<pid>/cmdline` · Test: Integration

### M1-S04 restic-Quelle
- **AC-M1-04-1** (FR-33, NFR-05, P31) – *Gegeben* Attrappe im `PATH` wie echtes restic 0.16.4 (`restic version --json` endet mit Fehler, `restic version` gibt «restic 0.16.4 compiled with go…»), *wenn* Kestic die restic-Quelle prüft, *dann* meldet es «restic 0.16.4 ist zu alt, mindestens 0.18.0 nötig» mit Begründung und startet keinen Lauf · Test: Unit
- **AC-M1-04-2** (FR-33, NFR-05) – *Gegeben* restic 0.18.0 bzw. aktuell im `PATH`, *dann* ist die Quelle gültig und die Version wird angezeigt · Test: Integration
- **AC-M1-04-3** (FR-33) – *Gegeben* «Eigener Pfad» nicht existierend bzw. nicht ausführbar, *wenn* gespeichert, *dann* lehnt Kestic mit Meldung ab · Test: Unit
- **AC-M1-04-4** (FR-33) – *Gegeben* kein restic im `PATH`, kein eigener Pfad, *dann* Meldung «restic nicht gefunden» mit Hinweis auf Mindestversion 0.18.0 · Test: Unit
- **AC-M1-04-5** (P31) – *Gegeben* `restic version --json` scheitert, *dann* wertet Kestic den Text von `restic version` aus; ist auch der unlesbar, gilt die Quelle als ungültig mit «Version von restic nicht erkennbar» · Test: Unit

### M1-S05 Secrets
- **AC-M1-05-1** (FR-08) – *Gegeben* Secret Service in `dbus-run-session` (bzw. Datei-Backend als Fallback, Q5), *wenn* ein Secret für `<ziel-id>` gespeichert und gelesen wird, *dann* ist der Wert identisch · Test: D-Bus
- **AC-M1-05-2** (NFR-02, F19, P4) – *Gegeben* gespeichertes Repo-Passwort für `<ziel-id>`, *wenn* `kestic secret get <ziel-id>`, *dann* genau das Passwort auf stdout, Exit 0; unbekannte `<ziel-id>` → Exit 1, leere stdout · Test: D-Bus
- **AC-M1-05-3** (NFR-02) – *Gegeben* ein Lauf, *wenn* restic startet, *dann* ist `RESTIC_PASSWORD_COMMAND` = `kestic secret get <ziel-id>` und `RESTIC_PASSWORD` nicht gesetzt · Test: Integration
- **AC-M1-05-4** – *Gegeben* Testsuite, *wenn* sie läuft, *dann* greift kein Test auf den echten Session-Bus bzw. die echte KWallet zu (`DBUS_SESSION_BUS_ADDRESS` zeigt auf den Test-Bus) · Test: Unit (Test-Harness)

### M1-S06 Ziel anlegen oder verbinden
- **AC-M1-06-1** (FR-01, FR-15) – *Gegeben* leerer Ordner, *wenn* Ziel Lokal mit «neu initialisieren» angelegt wird, *dann* existiert ein Repository und `restic snapshots` endet mit Exit 0 · Test: Integration
- **AC-M1-06-2** (FR-01, FR-15) – *Gegeben* lokaler `rest-server` mit Auth auf Zufallsport, *wenn* Ziel REST mit Benutzer und Passwort angelegt wird, *dann* wird initialisiert, Zugangsdaten über `RESTIC_REST_USERNAME`/`RESTIC_REST_PASSWORD`, nicht in der Repo-URL · Test: Integration
- **AC-M1-06-3** (FR-15) – *Gegeben* bestehendes Repo, *wenn* «verbinden» mit falschem Passwort, *dann* «Passwort falsch» (Exit 12), Ziel nicht gespeichert · Test: Integration
- **AC-M1-06-4** (FR-15) – *Gegeben* bestehendes Repo, *wenn* «verbinden» mit korrektem Passwort, *dann* gespeichert ohne `init` · Test: Integration
- **AC-M1-06-5** (FR-01, P32) – *Gegeben* REST-Ziel mit gestopptem `rest-server`, *wenn* Ziel angelegt oder Lauf gestartet wird, *dann* scheitert der Vorab-Check (Connect-Timeout 10 s), «Ziel nicht erreichbar» spätestens nach 15 s, kein restic-Prozess, neues Ziel nicht gespeichert · Test: Integration
- **AC-M1-06-6** (FR-01) – *Gegeben* laufender `rest-server`, falsche Zugangsdaten im Formular, *wenn* angelegt, *dann* verständlicher Anmeldefehler, Ziel nicht gespeichert · Test: Integration
- **AC-M1-06-7** (P32) – *Gegeben* Lokal-Ziel, dessen Ordner fehlt, *wenn* Lauf gestartet, *dann* meldet der Vorab-Check «Ziel nicht erreichbar», kein restic-Prozess · Test: Integration

### M1-S07 Backup ausführen
- **AC-M1-07-1** (FR-28) – *Gegeben* Backup `<backup-id>` mit einer Quelle, *wenn* es läuft, *dann* trägt der Snapshot genau den Tag `kestic:<backup-id>` · Test: Integration
- **AC-M1-07-2** (FR-28) – *Gegeben* erster Snapshot, *wenn* ein zweiter Lauf mit anderem `RESTIC_HOST` startet, *dann* findet restic den Parent (`--group-by tags`), Ausgabe ohne «no parent snapshot found» · Test: Integration
- **AC-M1-07-3** (F13) – *Gegeben* Ausschluss-Regel im Textfeld, *wenn* das Backup läuft, *dann* existiert `$XDG_STATE_HOME/kestic/excludes/<backup-id>.txt` mit diesen Regeln, wird per `--exclude-file` übergeben, ausgeschlossene Pfade fehlen im Snapshot · Test: Integration
- **AC-M1-07-4** (FR-28) – *Gegeben* zwei Backups auf dasselbe Ziel, je einmal gelaufen, *dann* liefert die Snapshot-Abfrage pro Backup nur Snapshots mit eigenem Tag · Test: Integration

### M1-S08 Lauf-Historie und Logs
- **AC-M1-08-1** (FR-18) – *Gegeben* abgeschlossener Lauf, *wenn* der Daemon neu startet, *dann* zeigt die Historie den Lauf mit Zeit, Dauer, Status, Exit-Code; Daten unter `$XDG_STATE_HOME/kestic/` · Test: Integration
- **AC-M1-08-2** (FR-18, NFR-02) – *Gegeben* Lauf mit Repo- und REST-Passwort, *dann* kommt kein Secret-Wert in Log oder Historie vor · Test: Integration
- **AC-M1-08-3** (FR-34) – *Gegeben* ein Lauf, *wenn* gespeichert, *dann* enthält die Historie jeden ausgeführten restic-Befehl mit Zeitpunkt, Dauer, Exit-Code · Test: Integration
- **AC-M1-08-4** (F13) – *Gegeben* Backup mit 120 Läufen, davon 30 älter als 90 Tage (simulierte Uhr), *wenn* die Log-Aufbewahrung läuft, *dann* werden genau die Logs gelöscht, die älter als 90 Tage **und** nicht unter den letzten 100 Läufen sind (hier 20); die übrigen 100 bleiben · Test: Unit
- **AC-M1-08-5** (FR-18) – *Gegeben* ein abgeschlossener Backup-Lauf, *wenn* sein Detail öffnet, *dann* zeigt es Titel, Statuszeile und Tab «Ergebnis» gemäss [hauptbereiche.md](../ui/hauptbereiche.md) §7.8; Wartung, Restore und Metadaten folgen mit M3, M4 und M6 · Test: GUI-Smoke

### M1-S09 D-Bus-API v1
- **AC-M1-09-1** (F14) – *Gegeben* laufender Daemon, *wenn* ein zweiter startet, *dann* beendet sich der zweite (Bus-Name belegt) · Test: D-Bus
- **AC-M1-09-2** (F14) – *Gegeben* ungültige Backup-Änderung über D-Bus (z. B. leere Quelle), *dann* lehnt der Daemon mit Fehler ab, `config.toml` unverändert · Test: D-Bus
- **AC-M1-09-3** (F14) – *Gegeben* gültige Backup-Änderung über D-Bus, *wenn* der Aufruf zurückkehrt, *dann* hat der Daemon `config.toml` geschrieben, die GUI die Datei nicht geöffnet · Test: D-Bus
- **AC-M1-09-4** (F14) – *Gegeben* laufendes Backup, *wenn* sich sein Status ändert, *dann* sendet der Daemon ein Status-Signal auf dem versionierten Interface (`…Kestic1`) · Test: D-Bus
- **AC-M1-09-5** (F14) – *Gegeben* M1 abgeschlossen, *dann* beschreibt `docs/spec/dbus-api.md` alle Methoden und Signale von v1, darunter `ManualRunBlocked` · Test: manuell (Review)
- **AC-M1-09-6** (F14, FR-30) – *Gegeben* Backup, dessen Quelle zwischen Anzeige und Klick verschwindet, *wenn* die GUI den manuellen Start aufruft, *dann* sendet der Daemon das D-Bus-Signal `ManualRunBlocked` mit Backup-ID und Grund (`source_unavailable` bzw. `destination_unreachable`) auf `…Kestic1`, kein restic-Prozess, und die offene GUI zeigt eine Meldung im Fenster · Test: D-Bus + GUI-Smoke
- **AC-M1-09-7** (FR-30) – *Gegeben* derselbe Fall ohne offenes Fenster (Start aus dem Tray), *dann* sendet der Daemon neben dem Signal eine Desktop-Benachrichtigung (notifyrc-Ereignis `ManualRunBlocked`) mit Grund · Test: D-Bus (Notification-Fake)

### M1-S10 Lauf abbrechen und unterbrochene Läufe
- **AC-M1-10-1** (FR-21) – *Gegeben* laufendes Backup, *wenn* «Abbrechen», *dann* erhält restic SIGTERM, endet, Lauf `abgebrochen` · Test: Integration
- **AC-M1-10-2** (FR-21) – *Gegeben* Lauf mit Status `läuft` in der Historie (Daemon hart beendet), *wenn* der Daemon neu startet, *dann* setzt er ihn auf `unterbrochen` · Test: Integration
- **AC-M1-10-3** (FR-21) – *Gegeben* laufendes Backup, *wenn* per «Abbrechen» (SIGTERM) beendet, *dann* liefert `restic list locks --no-lock` keinen Lock mehr (restic gibt ihn beim Aufräumen frei) · Test: Integration
- **AC-M1-10-4** (FR-21) – *Gegeben* veralteter exklusiver Lock (SIGKILL eines laufenden `restic check` oder Lock-Datei-Fixture mit Hostname des Testsystems und nicht laufender PID), *wenn* ein Backup-Lauf auf dasselbe Ziel startet, *dann* endet restic mit Exit 11 **[verifiziert, restic 0.19.1: per SIGKILL liegengebliebener exklusiver Lock von `check` blockiert `backup` und `forget` mit Exit 11 bis `restic unlock`]**, Kestic zeigt den Fehler gemäss Fehlertabelle; Anzeige mit Aktion `unlock` (FR-19) folgt in M3 · Test: Integration

### M1-S11 Tray und Notification
- **AC-M1-11-1** (NFR-11, K1) – *Gegeben* laufender Daemon mit Config, *wenn* der Tray abgefragt wird, *dann* ist ein StatusNotifierItem registriert, ohne Option zum Ausblenden; SNI-Status und Overlay folgen dem Gesamtstatus (schlechtester Zustand gewinnt, Lauf oder Pause erzwingt mindestens `Active`; [tray-und-benachrichtigungen.md](../ui/tray-und-benachrichtigungen.md) §10.1): blockierender Zustand (z. B. Passwortspeicher gesperrt, restic zu alt) und Fehler → `NeedsAttention` + Overlay Fehler; überfällig → `NeedsAttention` + Overlay überfällig; Warnung (Exit 3, Teilerfolg) → `Active` + Overlay Warnung; läuft (Backup, Restore, Wartung) → `Active` + Overlay läuft; pausiert → `Active` + Overlay pausiert; wartet, unterbrochen, nie gelaufen, nur manuell, ok → `Passive` ohne Overlay. In M1 mit echten Läufen geprüft: ok, läuft (Backup), Fehler, Warnung, blockierend, nie gelaufen, nur manuell; überfällig, pausiert, wartet, Wartung und Restore per Zustands-Fake (echt ab M3 bzw. M4) · Test: D-Bus (SNI `Status`, Overlay-Icon) je Zeile
- **AC-M1-11-2** (NFR-11, FR-13b) – *Gegeben* Tray-Menü, *dann* enthält es «Jetzt sichern», «Laufendes Backup abbrechen» (FR-21, deaktiviert ohne laufenden Lauf), «Kestic öffnen», «Kestic beenden…»; «Jetzt sichern» startet einen Lauf; «Pausieren» (ab M3), «Dateien wiederherstellen…» (ab M4), «Kit erzeugen…» (ab M6) und «restic aktualisieren…» (nach dem MVP, FR-51) fehlen in M1 · Test: D-Bus
- **AC-M1-11-3** (NFR-11) – *Gegeben* Tray-Menü, *wenn* «Kestic beenden…», *dann* zuerst Warnung «Geplante Backups laufen erst wieder, wenn Kestic gestartet ist», Daemon endet erst nach Bestätigung · Test: D-Bus + manuell
- **AC-M1-11-4** (F3) – *Gegeben* Lauf endet mit `Fehler` bzw. `Warnung`, *dann* Notification über `org.freedesktop.Notifications` mit Hint `desktop-entry` = App-ID; bei `ok` keine Notification, nur Tray-Status · Test: D-Bus (Notification-Fake)
- **AC-M1-11-5** (NFR-11) – *Gegeben* laufendes Backup, *wenn* der Tray abgefragt wird, *dann* Status `läuft` mit Fortschritt · Test: D-Bus + manuell
- **AC-M1-11-6** ([tray-und-benachrichtigungen.md](../ui/tray-und-benachrichtigungen.md) §10.1, nachgetragen 2026-11-21) – *Gegeben* laufender Daemon, *wenn* das Tray-Menü abgefragt wird, *dann* ist die letzte Zeile (a) durch einen Trenner abgesetzt und (b) die aktive Versionszeile «Kestic ‹Version› · ‹Build-Stand›» des Daemons; (c) `GetState` nennt denselben Build-Stand in `daemon_build_commit`; *wenn* sie geklickt wird, *dann* (d) sendet der Daemon `ShowWindowRequested` und `ShowAboutRequested` und (e) startet ohne angemeldete GUI eine mit `--show-about` · Test: D-Bus

### M1-S12 Autostart-Modus
- **AC-M1-12-1** (E11) – *Gegeben* keine `config.toml`, *wenn* `kestic daemon --autostart` startet, *dann* Exit 0 ohne Tray-Eintrag und ohne Bus-Namen · Test: D-Bus
- **AC-M1-12-2** (E11) – *Gegeben* keine `config.toml`, *wenn* `kestic daemon` (ohne `--autostart`) startet, *dann* läuft er, belegt den Bus-Namen und nimmt eine erste Config über D-Bus entgegen · Test: D-Bus
- **AC-M1-12-3** (E11) – *Gegeben* gültige `config.toml`, *wenn* `kestic daemon --autostart` startet, *dann* läuft er und zeigt den Tray · Test: D-Bus

### M1-S13 GUI-Grundscreens
- **AC-M1-13-1** (K3) – *Gegeben* ein Ziel und ein Backup, *wenn* die GUI offscreen startet, *dann* hat das Hauptfenster eine `Kirigami.NavigationTabBar` mit genau Übersicht, Backups, Ziele, Verlauf (kein «Wiederherstellen» in M1, kommt mit M4) und ein Hamburger-Menü ohne «Neues Backup…» und «Ziel hinzufügen…» (beide nur im Listenkopf, Ctrl+N wirkt überall); Ziel-Liste, Backup-Liste, Ziel-Formular (Lokal, REST), Backup-Formular und Verlauf laden ohne QML-Fehler, je ein Screenshot als Artefakt · Test: GUI-Smoke
- **AC-M1-13-5** (U19) – *Gegeben* kein erreichbarer Daemon bzw. gesperrter Passwortspeicher (Fake) bzw. restic-0.16.4-Attrappe, *wenn* die GUI startet (Selbsttest), *dann* zeigt die Zustandsleiste genau den ersten nicht erfüllten Zustand in der Reihenfolge Daemon → Passwortspeicher → restic mit Aktion (`[Hintergrunddienst starten]`, `[Entsperren…]`; restic in M1: Verweis auf Einstellung «restic-Quelle», `[restic herunterladen…]` deaktiviert, Download kommt mit M5); betroffene Aktionen deaktiviert · Test: GUI-Smoke + D-Bus (Fake)
- **AC-M1-13-2** (FR-13b) – *Gegeben* Backup in der Liste, *wenn* «Jetzt sichern», *dann* ruft die GUI die D-Bus-Startmethode auf, Lauf erscheint in der Historie · Test: GUI-Smoke mit D-Bus-Fake
- **AC-M1-13-3** (NFR-09) – *Gegeben* jeder Screen, *wenn* nur mit Tastatur bedient (Tab, Enter, Esc), *dann* sind alle Felder und Knöpfe erreichbar und auslösbar · Test: manuell
- **AC-M1-13-4** (FR-01) – *Gegeben* Ziel-Formular mit leerem Pflichtfeld (z. B. REST-Adresse), *dann* ist «Speichern» nicht möglich, Feld markiert · Test: GUI-Smoke
- **AC-M1-13-6** ([struktur.md](../ui/struktur.md) §4.1) – *Gegeben* das Hauptfenster mit Unterseiten und Assistenten, *wenn* der Fenstertitel abgefragt wird, *dann* lautet er immer «Kestic» (Einstellungsfenster «Kestic einrichten», Fenster «Über Kestic»); Tab «Backups» trägt `emblem-important`, wenn ein Backup Fehler hat oder überfällig ist (überfällig in M1 per Zustands-Fake, echt ab M3), Tab «Ziele», wenn ein Ziel ein Problem hat · Test: GUI-Smoke
- **AC-M1-13-7** ([nebenseiten.md](../ui/nebenseiten.md) §8.15) – *Gegeben* M1, *wenn* «Ziel hinzufügen» bzw. «Neues Backup» geöffnet wird, *dann* erscheinen reduzierte Assistenten: Ziel-Typ (nur Lokal, REST) → Ziel-Details mit Prüfung → Backup-Passwort; bzw. Quelle → Ziel wählen → Zusammenfassung mit Ausschluss-Textfeld in restic-Syntax · Test: GUI-Smoke (offscreen + Screenshot)
- **AC-M1-13-8** ([struktur.md](../ui/struktur.md) §4.1, nachgetragen 2026-11-21) – *Gegeben* das Hauptfenster breit und schmal, *dann* (a) steht der Knopf `☰` sichtbar am rechten Ende der obersten Kopfzeile, mit Namen für Screenreader; *wenn* er mit der Maus geklickt wird, *dann* (b) öffnet sich das Menü mit genau «Kestic einrichten…» und «Über Kestic», und (c) «Über Kestic» öffnet sich als eigenes Fenster «Über Kestic», nicht als Spalte im Hauptfenster, und zeigt keine E-Mail-Adresse; *wenn* nur die Tastatur benutzt wird, *dann* (d) erreicht Tab den Knopf, Leertaste öffnet das Menü, Pfeiltasten erreichen «Über Kestic», Enter öffnet es, und (e) Esc schliesst nur das offene Menü, der Fokus bleibt auf `☰`; (f) Ctrl+Shift+, öffnet die Einstellungen, auch bevor das Menü je offen war; (g) Esc schliesst das Fenster «Über Kestic», das Hauptfenster ist wieder aktiv und der Fokus steht auf `☰` · Test: GUI-Smoke (offscreen + Screenshots; schlägt fehl, wenn der Knopf fehlt)
- **AC-M1-13-9** ([ablaeufe.md](../ui/ablaeufe.md) «Über Kestic», nachgetragen 2026-11-21) – *Gegeben* «Über Kestic», *dann* zeigt es Version und Build-Stand von Anwendung und Hintergrunddienst; *wenn* der Hintergrunddienst einen anderen Build ausführt, *dann* (a) erscheint der Hinweis, Kestic neu zu öffnen, (b) ist ein sauberer Hash ein Link auf seinen Commit auf GitHub und (c) steht ein `-dirty`-Stand ohne Link; (d) mit `--show-about` gestartet, öffnet Kestic das Fenster «Über Kestic»; (e) auf Deutsch sind auch Kirigamis eigene Texte der Seite deutsch («kompiliert gegen», nie «built against»), über das Menü geöffnet wie beim Start mit `--show-about`. Regeln für Link, Anzeige und Abweichung · Test: Unit (`core`) + GUI-Smoke

### M1-S14 Befehlstransparenz
- **AC-M1-14-1** (FR-34, F17) – *Gegeben* ein Lauf, *wenn* «Befehl kopieren», *dann* enthält die Zwischenablage den restic-Aufruf mit Binary-Pfad, allen Argumenten, `nice`/`ionice`, Pfad der Ausschluss-Datei und `RESTIC_PASSWORD_COMMAND='kestic secret get <ziel-id>'` · Test: Unit
- **AC-M1-14-2** (FR-34, NFR-02) – *Gegeben* REST-Ziel, *wenn* der Befehl erzeugt wird, *dann* enthält er `RESTIC_REST_PASSWORD="$(kestic secret get <ziel-id> rest)"`, keinen Secret-Wert im Klartext · Test: Unit
- **AC-M1-14-3** (FR-34) – *Gegeben* kopierter Befehl gegen Temp-Repo, *wenn* in einer Shell (mit Test-Secret-Service) ausgeführt, *dann* gleicher Exit-Code wie der Lauf in Kestic · Test: Integration
- **AC-M1-14-4** (K2) – *Gegeben* Expertenmodus aus, *wenn* ein Lauf angesehen wird, *dann* ist «Befehl anzeigen» im ⋮-Menü vorhanden und funktioniert wie mit Expertenmodus · Test: GUI-Smoke

### M1-S15 Fehlertexte
- **AC-M1-15-1** (FR-37) – *Gegeben* Lauf endet mit Exit 1, 3, 10, 11 bzw. 12, *dann* erscheinen Text und Handlungsoption aus der Tabelle in `restic-integration.md` (z. B. Exit 12 → «Gespeichertes Passwort passt nicht» mit Aktion «Gespeichertes Passwort aktualisieren…») · Test: Unit
- **AC-M1-15-2** (FR-37) – *Gegeben* unbekannter Exit-Code, *dann* «Unbekannter Fehler» mit Rohmeldung von restic · Test: Unit
- **AC-M1-15-3** (FR-37) – *Gegeben* restic liefert mit `--json` eine `exit_error`-Meldung auf stderr, *dann* wird deren Text als Rohmeldung übernommen · Test: Integration

### M1-S16 i18n
- **AC-M1-16-1** (NFR-06) – *Gegeben* Quellcode (Rust und QML), *wenn* die String-Extraktion läuft, *dann* kein sichtbarer String ausserhalb der Übersetzungsfunktion · Test: Unit (Lint in `just verify`)
- **AC-M1-16-2** (NFR-06) – *Gegeben* `LANGUAGE=de` bzw. `LANGUAGE=en`, *wenn* GUI, Tray-Menü und eine Notification erzeugt werden, *dann* auf Deutsch bzw. Englisch · Test: GUI-Smoke + D-Bus

### M1-S17 Paket
- **AC-M1-17-1** (NFR-01) – *Gegeben* gebautes `.deb`, *wenn* `dpkg-deb --field` gelesen wird, *dann* `Depends` mit `qml6-module-org-kde-kirigami (>= 6.24)`, `qml6-module-org-kde-kirigamiaddons-datetime (>= 1.11)`, `qml6-module-org-kde-kirigamiaddons-tableview (>= 1.11)` und `Recommends` `restic (>= 0.18.0)` · Test: Unit (Paket-Prüfung)
- **AC-M1-17-2** (NFR-01, E11, U6) – *Gegeben* das `.deb`, *wenn* der Inhalt gelistet wird, *dann* enthält es `/etc/xdg/autostart/<app-id>.daemon.desktop` mit `Exec=kestic daemon --autostart`, die GUI-`.desktop`-Datei und `/usr/share/knotifications6/kestic.notifyrc` · Test: Unit (Paket-Prüfung)
- **AC-M1-17-3** (NFR-01, NFR-04) – *Gegeben* TUXEDO OS, *wenn* `.deb` installiert, Session neu gestartet und je ein Backup Lokal und REST manuell ausgeführt wird, *dann* laufen beide durch, auch bei geschlossenem Hauptfenster · Test: manuell
- **AC-M1-17-4** (NFR-01, P34) – *Gegeben* installiertes `.deb`, *wenn* `apt remove` und Session-Neustart, *dann* ist `/etc/xdg/autostart/<app-id>.daemon.desktop` noch da mit `TryExec=/usr/bin/kestic`, und kein Kestic-Prozess startet · Test: manuell
- **AC-M1-17-5** (NFR-01, P34) – *Gegeben* entferntes Paket, *wenn* `apt purge`, *dann* ist der Autostart-Eintrag unter `/etc/xdg/autostart` weg · Test: manuell
- **AC-M1-17-6** (P34) – *Gegeben* gebautes `.deb`, *dann* enthält der Autostart-Eintrag `TryExec=/usr/bin/kestic` · Test: Unit (Paket-Prüfung)

### M1-S18 Clock-Trait
- **AC-M1-18-1** (P36) – *Gegeben* simulierte Uhr auf 2026-12-01 12:00, *wenn* `core` einen Lauf-Zeitstempel vergibt, *dann* 2026-12-01 12:00; nach Vorstellen um 2 Stunden ist der nächste 14:00 · Test: Unit
- **AC-M1-18-2** (P36) – *Gegeben* Quellcode von `core` und `daemon`, *dann* liest kein zeitabhängiger Teil die Systemuhr direkt, nur über den Clock-Trait; dessen Produktiv-Implementierung nutzt die Systemuhr · Test: Unit (Lint/Review in `just verify`)

### M1-S19 Ziel und Backup bearbeiten und entfernen
- **AC-M1-19-1** (FR-56, U18) – *Gegeben* ein Backup, *wenn* Quelle oder Ausschluss-Regeln geändert und per `[Speichern]` gespeichert werden bzw. der Name im Dialog «Umbenennen» (ein Feld) bestätigt wird, *dann* ruft die GUI `UpdateBackup` auf, der Daemon schreibt `config.toml`, Backup-ID bleibt gleich · Test: D-Bus + GUI-Smoke
- **AC-M1-19-2** (FR-56, P39) – *Gegeben* Backup mit zwei Snapshots, *wenn* «Aus Kestic entfernen…», *dann* Bestätigungsdialog mit Folge «Die Snapshots bleiben im Ziel erhalten» (ohne Dry-Run); nach Bestätigung fehlt das Backup in `config.toml`, `restic snapshots --tag kestic:<backup-id>` liefert weiterhin 2 Snapshots · Test: Integration + GUI-Smoke
- **AC-M1-19-3** (FR-56, P39) – *Gegeben* Ziel ohne Backups, *wenn* «Ziel entfernen» bestätigt wird, *dann* fehlt es in `config.toml`, seine Secrets sind aus dem Secret Service entfernt, Repository unverändert (`restic snapshots` weiterhin Exit 0) · Test: Integration
- **AC-M1-19-4** (FR-56, P39) – *Gegeben* Ziel mit zugeordneten Backups, *wenn* «Ziel entfernen», *dann* verweigert Kestic mit Hinweis auf diese Backups (erst entfernen oder einem anderen Ziel zuweisen) · Test: D-Bus
- **AC-M1-19-5** (FR-56) – *Gegeben* Bestätigungsdialog «Aus Kestic entfernen…» bzw. «Ziel entfernen», *wenn* abgebrochen, *dann* `config.toml` und Secret Service unverändert · Test: D-Bus
- **AC-M1-19-6** (FR-56, U18) – *Gegeben* REST-Ziel, *wenn* Adresse oder Zugangsdaten geändert und gespeichert werden, *dann* zuerst Verbindungstest (Vorab-Check P32, Passwortprüfung); nur bei Erfolg `UpdateDestination` und Speichern, sonst alte Verbindung unverändert und Fehler angezeigt; der Repo-ID-Abgleich (P42) kommt mit M5-S09 · Test: Integration
- **AC-M1-19-7** (U18) – *Gegeben* Backup- oder Ziel-Editor mit ungespeicherten Änderungen, *wenn* verlassen, *dann* Rückfrage «Änderungen speichern?» mit `[Speichern]` und `[Verwerfen]`; bis zum Speichern ist `config.toml` unverändert · Test: GUI-Smoke + D-Bus

### M1-S20 Gespeichertes Passwort ändern
- **AC-M1-20-1** (FR-57, P40) – *Gegeben* Lauf mit Exit 12 wegen falschem Passwort im Secret Service, *wenn* der Benutzer über «Passwort ändern» das richtige eingibt, *dann* prüft Kestic es gegen das Repo, speichert es im Secret Service, der nächste Lauf endet mit `ok` · Test: Integration
- **AC-M1-20-2** (FR-57, P40) – *Gegeben* falsches Passwort im Dialog, *wenn* gespeichert, *dann* «Passwort falsch», Eintrag im Secret Service unverändert · Test: Integration
- **AC-M1-20-3** (P40) – *Gegeben* «Passwort ändern» abgeschlossen, *dann* lief kein `restic key`-Kommando; Keys unverändert (`restic key list`) · Test: Integration

### M1-S21 GUI startet Daemon
- **AC-M1-21-1** – *Gegeben* kein laufender Daemon, *wenn* die GUI startet, *dann* startet sie `kestic daemon` (ohne `--autostart`), Bus-Name innerhalb 5 s belegt · Test: D-Bus
- **AC-M1-21-2** – *Gegeben* Daemon nicht startbar (z. B. ungültiger Binary-Pfad im Test), *wenn* die GUI startet, *dann* zeigt die Zustandsleiste dauerhaft «Der Hintergrunddienst läuft nicht. Geplante Backups finden nicht statt.» mit Aktion «Hintergrunddienst starten» · Test: GUI-Smoke
- **AC-M1-21-3** – *Gegeben* laufender Daemon, *wenn* die GUI startet, *dann* kein zweiter Daemon, keine Warnung · Test: D-Bus

### M1-S22 Backup einem anderen Ziel zuweisen
- **AC-M1-22-1** (FR-56) – *Gegeben* Backup mit zwei Snapshots in Ziel A und Ziel B, *wenn* Ziel B zugewiesen und gespeichert wird, *dann* findet der nächste Lauf in B keinen Parent und legt dort den ersten Snapshot der Reihe `kestic:<backup-id>` an; in A liefert `restic snapshots --tag kestic:<backup-id>` weiterhin 2 · Test: Integration
- **AC-M1-22-2** (FR-56, P39) – *Gegeben* A hatte nur dieses Backup, *wenn* es B zugewiesen ist, *dann* lässt sich A über «Ziel entfernen» entfernen · Test: D-Bus
- **AC-M1-22-3** (FR-56, FR-49) – *Gegeben* A enthält Metadaten-Snapshots `kestic-meta:<backup-id>`, *wenn* das Backup B zugewiesen wird, *dann* bleiben sie in A unverändert (`restic snapshots --tag kestic-meta:<backup-id>` gleiche Anzahl), und eine mit A verbundene Kestic-Instanz bietet das Backup gemäss FR-49 zum Übernehmen an · Test: Integration (FR-49-Teil ab M6)
- **AC-M1-22-4** (FR-56) – *Gegeben* Backup `b1` wurde von Ziel A nach B zugewiesen und A ist weiter eingerichtet, *wenn* die Snapshot-Liste von `b1` öffnet, *dann* zeigt sie die Snapshots auf B und einen Abschnitt «Frühere Snapshots auf ‹A›» mit den alten Snapshots (Wiederherstellen daraus ab M4); nach Entfernen von A entfällt der Abschnitt · Test: Integration + GUI-Smoke
- **AC-M1-22-5** (FR-56) – *Gegeben* die Seite «Ziel» eines Backups, *wenn* sie öffnet, *dann* listet sie das aktuelle und die übrigen Ziele ohne Vorauswahl ([nebenseiten.md](../ui/nebenseiten.md) §8.8), und «Ziel wechseln…» ist erst nach der Wahl eines anderen Ziels aktiv · Test: GUI-Smoke

### M1-S23 Verfügbarkeitsstatus
- **AC-M1-23-1** (FR-30, FR-29) – *Gegeben* Backup mit fehlendem Quellpfad, *wenn* Fenster oder Tray-Menü geöffnet werden, *dann* meldet der Daemon «Quelle nicht verfügbar», «Jetzt sichern» ist in GUI und Tray deaktiviert, Grund daneben und im Tooltip · Test: D-Bus + GUI-Smoke
- **AC-M1-23-2** (FR-30, P32) – *Gegeben* REST-Ziel mit gestopptem `rest-server`, *wenn* das Fenster geöffnet wird, *dann* führt der Daemon den Vorab-Check aus, und spätestens nach 15 s ist «Jetzt sichern» für alle Backups dieses Ziels mit «Ziel nicht erreichbar» deaktiviert · Test: Integration (`rest-server`) + D-Bus
- **AC-M1-23-3** (FR-30) – *Gegeben* Fenster innerhalb von 60 s zweimal geöffnet (simulierte Uhr, M1-S18), *dann* läuft der Vorab-Check nur einmal; nach mehr als 60 s erneut · Test: Unit
- **AC-M1-23-4** (FR-30) – *Gegeben* der Server läuft wieder, *wenn* nach mehr als 60 s das Tray-Menü geöffnet wird, *dann* ist «Jetzt sichern» wieder aktiv · Test: Integration (`rest-server`)

### Querschnitt
- **AC-M1-Q-1** (NFR-07) – *Gegeben* gesamte Testsuite, *wenn* `just verify` läuft, *dann*: `~/.config/kestic`, `~/.local/state/kestic`, `~/.local/share/kestic`, `~/.config/autostart` und `~/.ssh/known_hosts` des ausführenden Benutzers (Home aus `/etc/passwd`) bleiben unverändert; jeder SSH-Aufruf von Kestic enthält `-o UserKnownHostsFile=` mit Pfad im Test-Tempdir, in Tests zusätzlich `-F <Test-ssh_config>` und `-o IdentitiesOnly=yes` (weder echtes `~/.ssh/config` noch echte Schlüssel werden gelesen); `XDG_RUNTIME_DIR` zeigt auf ein Tempdir; kein Test läuft als root · Test: Unit (Test-Harness vergleicht vorher/nachher, prüft SSH-Argumente)
- **AC-M1-Q-2** (Q2) – *Gegeben* `just verify`, *dann* laufen die Integrationstests je einmal gegen restic 0.18.0 und aktuell · Test: Integration

## Exit-Kriterien

**Stand (2026-11-17): M1 abgeschlossen.** Alle 23 Stories (`M1-S01`–`M1-S23`), `just verify` (inkl. Integrationstests gegen restic 0.18.0 und 0.19.1) sowie alle manuellen Prüfpunkte in [manual-acceptance.md](../manual-acceptance.md) sind erfüllt und abgenommen:

- Auf TUXEDO installiert: Backup Lokal und REST manuell gestartet, Fortschritt im Tray, Ergebnis in Historie, Fehler als Notification.
- `just verify` grün inkl. Tests gegen restic 0.18.0 und aktuell.
- Manuelle Abnahme: Tray, Notification, Look & Feel des Hauptfensters (Tab-Leiste, Zustandsleiste, Listen, Formulare).
