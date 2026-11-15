# Manuelle Abnahme

Prüfungen, die ein Mensch auf dem Referenzsystem (TUXEDO OS mit Plasma) machen muss: alle ACs mit «Test: manuell» (auch kombiniert mit automatisierten Ebenen) und die manuellen Exit-Kriterien je Meilenstein. Der genaue Wortlaut steht im jeweiligen AC; Teil von [backlog.md](backlog.md).

## M0 Spike

Quelle: [milestones/m0-spike.md](milestones/m0-spike.md)

- [ ] **AC-M0-02-1** – wenn `scripts/bootstrap.sh` läuft, dann sind vorhanden: Rust-Toolchain gemäss `rust-toolchain.toml`, restic 0.18.0 und aktuell, `rest-server`, `rclone` ≥ 1.65 (offizielles Release, nicht Ubuntu-Paket 1.60.1), `sshd`; jedes heruntergeladene Binary per SHA256 geprüft, restic zusätzlich per PGP, rest-server und rclone zusätzlich per PGP, sofern der Spike-Bericht eine veröffentlichte Signatur festhält
- [ ] **AC-M0-02-6** – dann nennt der Spike-Bericht für rest-server und rclone je «signiert (PGP)» oder «nur SHA256», und `bootstrap.sh` prüft genau so
- [ ] **AC-M0-03-1** – dann ist die CXX-Qt-Version in `Cargo.toml` exakt gepinnt und ADR-0001 begründet die Wahl zwischen 0.8.1 und 0.10.0
- [ ] **AC-M0-05-1** – wenn es auf TUXEDO installiert wird, dann: `Depends` enthält `qml6-module-org-kde-kirigami (>= 6.24)`, `qml6-module-org-kde-kirigamiaddons-datetime (>= 1.11)`, `qml6-module-org-kde-kirigamiaddons-tableview (>= 1.11)`; `Recommends` `restic (>= 0.18.0)`; `kestic.notifyrc` liegt unter `/usr/share/knotifications6/`; die App startet aus dem Startmenü; `/etc/xdg/autostart/<app-id>.daemon.desktop` mit `Exec=kestic daemon --autostart` ist vorhanden
- [ ] **AC-M0-05-2** – wenn `apt remove` läuft, dann ist `/etc/xdg/autostart/<app-id>.daemon.desktop` als Conffile noch da, enthält `TryExec=/usr/bin/kestic`, und beim nächsten Login startet kein Kestic-Prozess
- [ ] **AC-M0-05-3** – wenn `apt purge` läuft, dann ist `/etc/xdg/autostart/<app-id>.daemon.desktop` weg; ein Benutzer-Override mit `Hidden=true` in `~/.config/autostart` darf bleiben (wirkungslos ohne Systemeintrag)
- [ ] **AC-M0-08-1** – dann nennt ADR-0001 genau ein Secret-Backend für automatisierte Tests (headless Keyring in `dbus-run-session` oder Datei-Backend hinter dem Secret-Trait) mit Begründung
- [ ] **AC-M0-10-1** – wenn die PowerDevil-Inaktivitätszeit abläuft, dann geht das Laptop nicht in den Ruhezustand; Ergebnis im Abschnitt «Spike-Kriterien und offene Prüfungen» unten
- [ ] **AC-M0-11-2** – wenn er über `org.kde.JobViewServerV2.requestView` (zbus, aus Rust) eine Fortschrittsanzeige anmeldet und Fortschritt meldet, dann zeigt Plasma den Fortschritt mit «Abbrechen» im Benachrichtigungsbereich
- [ ] **AC-M0-11-3** – wenn zwischen hellem und dunklem Farbschema gewechselt wird, dann passt sich die Icon-Farbe an
- [ ] **AC-M0-11-4** – wenn der Benutzer in den Systemeinstellungen ein Ereignis abschaltet, dann kommt dafür kein Popup mehr; Ergebnis (wirkt / wirkt nicht) im Spike-Bericht
- [ ] **AC-M0-11-5** – wenn er den SNI-Status von `Active` auf `Passive` und zurück setzt, dann wandert das Icon in den ausgeklappten Bereich und bei `Active` wieder sichtbar ins Panel
- [ ] **AC-M0-11-6** – wenn ausgelöst, dann öffnet sich jeweils das richtige Modul der Systemeinstellungen; Modulnamen im Spike-Bericht
- [ ] **AC-M0-11-8** – wenn Ctrl+Shift+B gedrückt wird, dann löst es «Jetzt sichern» aus; ein Konflikt mit einem globalen Plasma-Kürzel ist im Spike-Bericht festgehalten oder ausgeschlossen

## M1 Walking Skeleton

Quelle: [milestones/m1-walking-skeleton.md](milestones/m1-walking-skeleton.md)

- [ ] **AC-M1-09-5** – dann beschreibt `docs/spec/dbus-api.md` alle Methoden und Signale von v1, darunter `ManualRunBlocked`
- [ ] **AC-M1-11-3** – wenn «Kestic beenden…», dann zuerst Warnung «Geplante Backups laufen erst wieder, wenn Kestic gestartet ist», Daemon endet erst nach Bestätigung
- [ ] **AC-M1-11-5** – wenn der Tray abgefragt wird, dann Status `läuft` mit Fortschritt
- [ ] **AC-M1-13-3** – wenn nur mit Tastatur bedient (Tab, Enter, Esc), dann sind alle Felder und Knöpfe erreichbar und auslösbar
- [ ] **AC-M1-17-3** – wenn `.deb` installiert, Session neu gestartet und je ein Backup Lokal und REST manuell ausgeführt wird, dann laufen beide durch, auch bei geschlossenem Hauptfenster
- [ ] **AC-M1-17-4** – wenn `apt remove` und Session-Neustart, dann ist `/etc/xdg/autostart/<app-id>.daemon.desktop` noch da mit `TryExec=/usr/bin/kestic`, und kein Kestic-Prozess startet
- [ ] **AC-M1-17-5** – wenn `apt purge`, dann ist der Autostart-Eintrag unter `/etc/xdg/autostart` weg
- [ ] **Exit-Kriterium** – Manuelle Abnahme: Tray, Notification, Look & Feel des Hauptfensters (Tab-Leiste, Zustandsleiste, Listen, Formulare).

## M2 Backup-Konfiguration

Quelle: [milestones/m2-backup-konfiguration.md](milestones/m2-backup-konfiguration.md)

- [ ] **AC-M2-02-4** – wenn nur Tastatur (Tab, Enter, Leertaste, Esc), dann Regel hinzufügen, ändern, entfernen, speichern möglich; jedes Bedienelement mit zugänglichem Namen
- [ ] **AC-M2-06-4** – wenn nur Tastatur, dann Felder, Vorlagen, Vorschau und Bestätigung erreichbar
- [ ] **AC-M2-11-3** – (manuell) – Verständlichkeit von Ausschluss- und Aufbewahrungs-Editor durch eine Person ohne restic-Kenntnisse geprüft
- [ ] **Exit-Kriterium** – Manuelle Abnahme: Verständlichkeit von Ausschluss- und Aufbewahrungs-Editor.

## M3 Automatisierung

Quelle: [milestones/m3-automatisierung.md](milestones/m3-automatisierung.md)

- [ ] **AC-M3-08-4** – wenn ein geplanter Lauf läuft, dann zeigt das Tray Status und Fortschritt, danach steht der Lauf in der Historie
- [ ] **AC-M3-09-4** – (F9, manuell) – Deckel schliessen während eines echten Laufs; nach dem Aufwachen läuft das Backup weiter bzw. neu an; PowerDevil verhindert den automatischen Ruhezustand bei Inaktivität während eines Laufs
- [ ] **AC-M3-12-2** – wenn dort abgebrochen, dann endet der Lauf mit `abgebrochen`
- [ ] **AC-M3-14-6** – wenn nur Tastatur, dann alle Felder erreichbar, jedes Bedienelement mit zugänglichem Namen
- [ ] **Exit-Kriterium** – Ein Backup läuft eine Woche lang nach Zeitplan auf dem Laptop (parallel zu M4/M5 möglich), inkl. Akku/Netzwechsel und Suspend (manuelle Abnahme).
- [ ] **Exit-Kriterium** – Manuelle Abnahme: Deckel zu während Lauf, PowerDevil/Portal-Inhibit, Hotspot.

## M4 Restore

Quelle: [milestones/m4-restore.md](milestones/m4-restore.md)

- [ ] **AC-M4-01-4** – wenn nur die Tastatur benutzt wird, dann lassen sich alle Snapshots wählen und Unterschiede, Durchsuchen, Wiederherstellen auslösen
- [ ] **AC-M4-03-3** – wenn nur die Tastatur benutzt wird, dann geht es per Ordnerliste in Unterordner und per Breadcrumb in jede übergeordnete Ebene zurück (keine Baumansicht zum Auf- und Zuklappen), und Einträge lassen sich zum Wiederherstellen markieren
- [ ] **Exit-Kriterium** – Manuelle Abnahme: Restore aus einem echten Netzwerk-Ziel (z. B. NAS).

## M5 Ziele und restic-Bereitstellung

Quelle: [milestones/m5-ziele-und-restic.md](milestones/m5-ziele-und-restic.md)

- [ ] **Exit-Kriterium** – Manuelle Abnahme: echtes Netzwerk-Ziel (z. B. NAS) (REST oder SFTP).

## M6 Onboarding und Sicherheit

Quelle: [milestones/m6-onboarding-und-sicherheit.md](milestones/m6-onboarding-und-sicherheit.md)

- [ ] **AC-M6-01-7** – wenn er das erste Backup «Home → externe Disk» einrichtet, dann gelingt das ohne Hilfe in höchstens 5 Schritten
- [ ] **AC-M6-08-1** – wenn der Abschnitt «Bedrohungsmodell» gelesen wird, dann nennt er als akzeptiert, dass Prozesse desselben Benutzers Secrets lesen (Secret Service, `kestic secret get`, `/proc/<pid>/environ`), und als Risiken mit Gegenmassnahme Kit-Datei (F8) und Export-Script ohne Secrets (F16)
- [ ] **AC-M6-09-2** – wenn der Test die Fokuskette per simuliertem Tab durchläuft, dann erreicht er jedes aktivierbare Element genau einmal in sichtbarer Reihenfolge, und Enter bzw. Leertaste lösen es aus
- [ ] **AC-M6-09-3** – wenn der Test für jedes aktivierbare Element `Accessible.name` ausliest, dann ist er nicht leer, und Statusinformationen sind neben der Farbe auch als Text oder Icon vorhanden
