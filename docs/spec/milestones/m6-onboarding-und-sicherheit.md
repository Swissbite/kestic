# M6 Onboarding und Sicherheit

Teil von [backlog.md](../backlog.md).

**Ziel:** Ein Endnutzer richtet sein erstes Backup in 5 Schritten ein, sichert sein Passwort und kommt auf einem neuen Rechner wieder an seine Daten. Abschluss des MVP.

**Voraussetzung:** M2, M3, M4, M5.

## Anforderungen

FR-24, FR-35, FR-38, FR-39, FR-49, FR-56 · NFR-08, NFR-09 · E10, F7, F8, F16, F18, P25–P27, P29, P30, P35, P37, P38, P42, U15

## Stories

| ID | Story |
| --- | --- |
| M6-S01 | Onboarding in 5 Schritten mit Defaults (Quelle `$HOME`, Ziel-Typen sortiert nach bereit / braucht Installation, Details + Verbindungstest, Passwort + Kit, Zusammenfassung mit Grössenschätzung) (FR-38) |
| M6-S02 | Wiederherstellungs-Kit: Recovery-Key per `key add`, starke Passphrase, einmal anzeigen, drucken/exportieren, Bestätigung, nur Key-ID speichern (FR-35) |
| M6-S03 | Kit erneuern (neuer Key, alter entfernt), Hinweistexte zur Kit-Datei, Export nicht in eine Backup-Quelle (F8) |
| M6-S04 | Metadaten-Snapshot pro Backup (`kestic-meta:<backup-id>`), Aufbewahrung `--keep-last 10`, Flag «Metadaten ausstehend»; beim ersten Start nach dem Update für alle bestehenden Backups nachschreiben (E10, FR-49) |
| M6-S05 | Backups aus Ziel übernehmen, Warnung bei jungem Snapshot eines anderen Hosts, verwaiste Kestic-Snapshots anzeigen (FR-49) |
| M6-S06 | Neuer Rechner: Verbinden mit Alltags-Passwort oder Recovery-Passphrase, danach neues Alltags-Passwort + Angebot, das alte zu entfernen (FR-39, F7) |
| M6-S07 | Export als Shell-Script: Exclude-Dateien inline, Secrets per `read -s`, kein `kestic secret get` (FR-24, F16) |
| M6-S08 | Bedrohungsmodell in `architecture.md` (F18) |
| M6-S09 | Querschnitt: NFR-08 (destruktive Aktionen) und NFR-09 (Tastatur, Barrierefreiheit) über alle Screens prüfen, Checkliste je Screen |
| M6-S10 | Kit für bestehendes Ziel erzeugen (Ziele aus M1–M5 bzw. ohne Kit); Hinweis in Status und Tray, solange das Kit fehlt (FR-35) |
| M6-S11 | Fehlerabbildung vollständig: alle bekannten Fehler aus M1–M6 (Exit-Codes, JSON `exit_error`, Backend-Fehler wie DNS/Auth/Timeout, KWallet gesperrt) mit Text + Aktion (FR-37) |
| M6-S12 | Log-Ansicht aufbereiten: restic-JSON-Einträge im Lauf-Detail lesbar darstellen, strukturiert nach Schweregrad (Alle/Warnungen/Fehler) filtern und durchsuchen; Roh-JSON optional im Expertenmodus bzw. über Log-Datei (FR-18, NFR-10) |

## Akzeptanzkriterien

Test-Isolation gemäss [test-strategy.md](../test-strategy.md). Zusätzlich: Secret Service als In-Memory- bzw. Datei-Backend, nur Temp-Repos bzw. lokaler `rest-server`. «Zweite Kestic-Instanz» = Instanz mit eigenem, leerem Satz `XDG_*_HOME`. «Integration» = gegen echtes restic mit Temp-Repo.

### M6-S01 Onboarding
- **AC-M6-01-1** (FR-38) – *Gegeben* leere Konfiguration, *wenn* Kestic startet, *dann* führt das Onboarding in genau 5 Schritten (Quelle → Ziel-Typ → Ziel-Details → Passwort + Wiederherstellungs-Kit → Zusammenfassung) zu «Jetzt sichern» · Test: GUI-Smoke (offscreen + Screenshot je Schritt)
- **AC-M6-01-2** (FR-38) – *Gegeben* unveränderte Vorgaben, *wenn* die Zusammenfassung erscheint, *dann* gilt: Quelle `$HOME`, Zeitplan täglich 12:00 (P35), Aufbewahrung 7 täglich / 4 wöchentlich / 12 monatlich, Ausschluss-Regeln für `$HOME/.cache`, `$HOME/.local/share/Trash` und den Download-Ordner aus `user-dirs.dirs` (im Test `$HOME/Downloads`) sowie `--exclude-caches` · Test: Unit (erzeugte Konfiguration), GUI-Smoke
- **AC-M6-01-3** (FR-38, §5.7) – *Gegeben* `ssh` im `PATH`, `rclone` nicht, *wenn* «Ziel-Typ» erscheint, *dann* stehen alle bereiten Typen vor «Cloud-Dienste über rclone», das «braucht Installation» mit Paketname `rclone` zeigt · Test: Unit, GUI-Smoke
- **AC-M6-01-4** (FR-38, FR-15) – *Gegeben* Ziel-Details für einen lokalen `rest-server`, *wenn* der Verbindungstest läuft, *dann* wird ein bestehendes Repo als «verbinden», ein leerer Pfad als «neu initialisieren» (`init`) erkannt · Test: Integration (+ `rest-server`)
- **AC-M6-01-5** (FR-38, FR-17) – *Gegeben* Quelle mit bekannter Grösse, *wenn* die Zusammenfassung erscheint, *dann* zeigt sie die Grössenschätzung aus `backup --dry-run` · Test: Integration
- **AC-M6-01-6** (FR-38, FR-35) – *Gegeben* Schritt 4, *wenn* das Kit nicht als «sicher aufbewahrt» bestätigt ist, *dann* sind «Weiter» und «Jetzt sichern» gesperrt · Test: GUI-Smoke
- **AC-M6-01-7** (FR-38) – *Gegeben* ein Endnutzer ohne restic-Kenntnisse, *wenn* er das erste Backup «Home → externe Disk» einrichtet, *dann* gelingt das ohne Hilfe in höchstens 5 Schritten · Test: manuell
- **AC-M6-01-8** (FR-38, FR-35, U15) – *Gegeben* der Assistent bis Schritt 4, *wenn* vor «Jetzt sichern» abgebrochen wird, *dann* existieren weder neues Repo noch Recovery-Key noch Konfiguration; Repo (`init`) und Recovery-Key (`key add`) entstehen erst beim Abschluss · Test: Integration + GUI-Smoke
- **AC-M6-01-9** (FR-38) – *Gegeben* Schritt 5, *wenn* «Speichern, später sichern» gewählt wird, *dann* sind Ziel, Backup und Kit angelegt, aber kein Lauf startet; der nächste Lauf folgt dem Zeitplan · Test: Integration + GUI-Smoke

### M6-S02 Wiederherstellungs-Kit
- **AC-M6-02-1** (FR-35) – *Gegeben* ein Ziel wird per `init` neu angelegt, *wenn* das Kit erzeugt wird, *dann* zeigt `restic key list` zwei Keys und die Konfiguration enthält die Key-ID des Recovery-Keys · Test: Integration
- **AC-M6-02-2** (FR-35) – *Gegeben* ein bestehendes Repo wird verbunden, *wenn* Schritt 4 durchlaufen wird, *dann* entsteht ebenfalls ein Recovery-Key per `key add` · Test: Integration
- **AC-M6-02-3** (FR-35) – *Gegeben* das angezeigte Kit, *wenn* `restic snapshots` mit der Recovery-Passphrase läuft, *dann* Exit 0 · Test: Integration
- **AC-M6-02-4** (FR-35) – *Gegeben* ein erzeugtes Kit, *wenn* Config, Zustand, Logs, Lauf-Historie, Secret-Store und angezeigte Befehle (FR-34) nach der Recovery-Passphrase durchsucht werden, *dann* kommt sie nirgends vor · Test: Integration
- **AC-M6-02-5** (FR-35, P25) – *Gegeben* der Passphrase-Generator, *wenn* seine Parameter geprüft werden, *dann* gilt Länge × log2(Alphabetgrösse) ≥ 128 Bit, die Zeichen stammen aus dem kryptografisch sicheren Zufallsgenerator des Betriebssystems (`getrandom`/OS-RNG, kein seedbarer PRNG), und zwei erzeugte Passphrasen sind verschieden · Test: Unit + Code-Review
- **AC-M6-02-6** (FR-35) – *Gegeben* das Kit, *wenn* es angezeigt, gedruckt oder exportiert wird, *dann* enthält es Repo-Adresse und Recovery-Passphrase, zum Backend-Zugang nur einen Hinweis ohne Secret-Werte · Test: Unit
- **AC-M6-02-7** (FR-35) – *Gegeben* das Alltags-Passwort wird geändert (`key passwd`), *wenn* danach die Recovery-Passphrase genutzt wird, *dann* öffnet sie das Repo weiterhin · Test: Integration
- **AC-M6-02-8** (FR-35) – *Gegeben* ein abgeschlossenes Kit, *wenn* es später geöffnet werden soll, *dann* zeigt Kestic Key-ID und Erstelldatum, keine Passphrase, und bietet «Kit erneuern» an · Test: GUI-Smoke
- **AC-M6-02-9** (FR-35, NFR-02, P37) – *Gegeben* `$XDG_RUNTIME_DIR` auf Tempdir und ein Kit wird erzeugt (`key add`) bzw. ein neues Alltags-Passwort angelegt (`key add` in FR-39), *wenn* restic läuft, *dann* steht das neue Passwort nicht in `/proc/<pid>/cmdline`; restic erhält es per `--new-password-file` aus einer Datei mit Rechten 0600 unter `$XDG_RUNTIME_DIR/kestic/`, die nach dem Aufruf nicht mehr existiert (auch bei restic-Fehler) · Test: Integration

### M6-S03 Kit erneuern und Kit-Datei
- **AC-M6-03-1** (FR-35) – *Gegeben* Recovery-Key K1, *wenn* «Kit erneuern» bestätigt wird, *dann* wird K2 per `key add` angelegt, danach K1 per `key remove` entfernt; K1-Passphrase → Exit 12, K2 → Exit 0, Konfiguration enthält die Key-ID von K2 · Test: Integration
- **AC-M6-03-2** (FR-35) – *Gegeben* `key add` scheitert beim Erneuern, *wenn* der Vorgang abbricht, *dann* bleibt K1 erhalten und gültig · Test: Integration (Fehler simuliert)
- **AC-M6-03-3** (FR-35, NFR-08) – *Gegeben* «Kit erneuern», *wenn* der Bestätigungsdialog erscheint, *dann* nennt er die Folge «Das bisherige Wiederherstellungs-Kit wird ungültig»; Abbrechen ändert nichts · Test: GUI-Smoke
- **AC-M6-03-4** (F8) – *Gegeben* der Export des Kits als Datei, *wenn* der Dialog erscheint, *dann* steht dort «Enthält das Passwort im Klartext – ausdrucken oder auf einem getrennten, sicheren Medium aufbewahren, nicht im gesicherten Ordner.» · Test: GUI-Smoke
- **AC-M6-03-5** (F8) – *Gegeben* ein Backup mit Quelle `$HOME`, *wenn* das Kit nach `$HOME/Dokumente/kit.txt` exportiert werden soll, *dann* lehnt Kestic ab und schreibt nichts; ein Pfad ausserhalb aller Quellen wird akzeptiert · Test: Unit

### M6-S04 Metadaten-Snapshot
- **AC-M6-04-1** (E10, FR-49) – *Gegeben* Backup `b1` wird angelegt und das Ziel ist erreichbar, *dann* existiert ein Snapshot mit genau dem Tag `kestic-meta:b1`, dessen Inhalt (per `dump`) Name, Quelle, Ausschluss-Regeln, Zeitplan, Aufbewahrung enthält · Test: Integration
- **AC-M6-04-2** (E10) – *Gegeben* der Metadaten-Snapshot, *wenn* er nach Repo-Passwort, Recovery-Passphrase und Backend-Credentials durchsucht wird, *dann* kommt keines vor · Test: Integration
- **AC-M6-04-3** (E10) – *Gegeben* zwölf Änderungen an `b1`, sechs mit `--host alt`, sechs mit `--host neu` geschrieben, *wenn* `forget --tag kestic-meta:b1 --group-by tags --keep-last 10` läuft, *dann* bleiben genau 10 Snapshots `kestic-meta:b1` (eine Gruppe, hostunabhängig); der erzeugte Aufruf enthält `--group-by tags` · Test: Integration
- **AC-M6-04-4** (E10) – *Gegeben* Metadaten-Snapshots von `b1`, *wenn* die reguläre Aufbewahrung `forget --tag kestic:b1 --group-by tags` läuft, *dann* bleiben sie unberührt · Test: Integration
- **AC-M6-04-5** (E10, FR-49) – *Gegeben* gestoppter `rest-server`, *wenn* `b1` geändert wird, *dann* wird «Metadaten ausstehend» gesetzt; nach dem nächsten erfolgreichen Kontakt ist der Metadaten-Snapshot geschrieben und das Flag gelöscht · Test: Integration (`rest-server`)
- **AC-M6-04-6** (E10) – *Gegeben* `b1` und `b2` auf demselben Ziel, *wenn* beide geändert werden, *dann* entstehen getrennte Reihen `kestic-meta:b1` und `kestic-meta:b2` · Test: Integration
- **AC-M6-04-7** (E10) – *Gegeben* Metadaten-Snapshots, *wenn* die Snapshot-Liste (FR-06) oder «nicht zugeordnet» (FR-36) angezeigt wird, *dann* erscheinen sie dort nicht · Test: Integration, D-Bus
- **AC-M6-04-8** (E10) – *Gegeben* zwei Backups aus M1–M5 ohne Metadaten-Snapshot, *wenn* der Daemon erstmals mit M6-Funktion startet, *dann* setzt er für beide «Metadaten ausstehend», schreibt sie beim nächsten erfolgreichen Kontakt, und keines gilt danach als «verwaist» · Test: Integration
- **AC-M6-04-9** (E10, P27) – *Gegeben* ein Ziel mit Alltags- und Recovery-Key, *wenn* der Metadaten-Snapshot per `dump` gelesen wird, *dann* enthält er die Key-IDs beider Keys (keine Passwörter); damit erkennt Instanz B beim Verbinden mit der Recovery-Passphrase den alten Alltags-Key für das Angebot in AC-M6-06-4 · Test: Integration
- **AC-M6-04-10** (E10, P38) – *Gegeben* ein Metadaten-Snapshot wird geschrieben, *dann* erscheint er in der Historie als Lauf der Art `Meta` des Ziels und reiht sich in dessen Warteschlange ein (startet nicht während eines Wartungslaufs) · Test: Integration

### M6-S05 Backups aus Ziel übernehmen
- **AC-M6-05-1** (FR-49) – *Gegeben* ein Repo mit Backup `b1` aus Instanz A, *wenn* eine zweite Kestic-Instanz B das Ziel verbindet und «Backups übernehmen» wählt, *dann* hat B ein Backup mit ID `b1` mit gleichem Namen, Quelle, Ausschluss-Regeln, Zeitplan, Aufbewahrung · Test: Integration (zwei Instanzen)
- **AC-M6-05-2** (FR-49) – *Gegeben* B hat `b1` übernommen, *wenn* B das erste Backup ausführt, *dann* findet restic einen Parent (keine Meldung `no parent snapshot found`), und die Aufbewahrung behandelt Snapshots von A und B als eine Gruppe · Test: Integration (unterschiedliche `--host`)
- **AC-M6-05-3** (FR-49) – *Gegeben* der letzte Snapshot von `b1` stammt von Host `alt` und ist 3 Tage alt, *wenn* B auf Host `neu` übernehmen will, *dann* zeigt Kestic Host und Zeit und warnt «Übernehmen heisst: Der andere Rechner soll dieses Backup nicht mehr ausführen» · Test: Unit (injizierbare Uhr), GUI-Smoke
- **AC-M6-05-4** (FR-49) – *Gegeben* der letzte Snapshot ist 8 Tage alt oder vom selben Host, *wenn* übernommen wird, *dann* keine Warnung · Test: Unit
- **AC-M6-05-5** (FR-49, Glossar «verwaist») – *Gegeben* Snapshots mit `kestic:x9`, `x9` weder in der Config noch in einem `kestic-meta:x9`, *wenn* das Ziel verbunden wird, *dann* erscheinen sie als «verwaist», nicht unter «nicht zugeordnet»; mit vorhandenem `kestic-meta:x9` erscheinen sie unter «Backups übernehmen» · Test: Integration

### M6-S06 Neuer Rechner
- **AC-M6-06-1** (FR-39) – *Gegeben* eine leere Instanz B, *wenn* sie ein Ziel mit Adresse + Alltags-Passwort bzw. Adresse + Recovery-Passphrase verbindet, *dann* gelingt beides und die Snapshots sind sichtbar · Test: Integration
- **AC-M6-06-2** (FR-39, FR-37) – *Gegeben* falsche Passphrase, *wenn* verbunden wird, *dann* Exit 12, Kestic zeigt «Passwort falsch» mit Handlungsoption und speichert weder Ziel noch Secret · Test: Integration
- **AC-M6-06-3** (FR-39, F7) – *Gegeben* B verbindet mit der Recovery-Passphrase, *wenn* die Verbindung steht, *dann* legt Kestic per `key add` ein neues Alltags-Passwort an und speichert nur dieses im Secret Service; die Recovery-Passphrase liegt nicht dort und öffnet das Repo weiterhin · Test: Integration
- **AC-M6-06-4** (FR-39, F7, NFR-08) – *Gegeben* AC-M6-06-3, *wenn* Kestic anbietet, das alte Alltags-Passwort zu entfernen, und bestätigt wird, *dann* wird dieser Key per `key remove` entfernt (altes Passwort → Exit 12), Recovery-Key und neues Alltags-Passwort bleiben gültig; bei Ablehnung bleiben alle Keys · Test: Integration
- **AC-M6-06-5** (FR-39) – *Gegeben* B ist verbunden und hat `b1` übernommen, *wenn* eine Datei aus einem Snapshot von A in einen neuen Ordner wiederhergestellt wird, *dann* ist ihr Inhalt identisch · Test: Integration

### M6-S07 Export als Shell-Script
- **AC-M6-07-1** (FR-24, F16) – *Gegeben* `b1` mit Ausschluss-Regeln, *wenn* exportiert wird, *dann* enthält das Script die Regeln inline (Heredoc), die Repo-Adresse sowie `backup` und `forget` mit Tag `kestic:b1`, `--group-by tags` und der vollständigen Aufbewahrung (alle gesetzten `--keep-*`-Regeln, P30) · Test: Unit (Snapshot-Test)
- **AC-M6-07-2** (FR-24, F16) – *Gegeben* das Script, *wenn* es nach Secrets und `kestic secret get` durchsucht wird, *dann* findet sich keines; Secrets werden per `read -s` abgefragt · Test: Unit
- **AC-M6-07-3** (FR-24) – *Gegeben* ein `PATH` ohne `kestic`, *wenn* das Script mit Passwort über die Standardeingabe gegen ein Temp-Repo läuft, *dann* entsteht ein Snapshot mit `kestic:b1` und die Ausschluss-Regeln greifen · Test: Integration
- **AC-M6-07-4** (FR-24) – *Gegeben* das Script, *wenn* `bash -n` darauf läuft, *dann* Exit 0 · Test: Unit

### M6-S08 Bedrohungsmodell
- **AC-M6-08-1** (F18) – *Gegeben* `docs/spec/architecture.md`, *wenn* der Abschnitt «Bedrohungsmodell» gelesen wird, *dann* nennt er als akzeptiert, dass Prozesse desselben Benutzers Secrets lesen (Secret Service, `kestic secret get`, `/proc/<pid>/environ`), und als Risiken mit Gegenmassnahme Kit-Datei (F8) und Export-Script ohne Secrets (F16) · Test: manuell (Review)

### M6-S09 Querschnitt destruktive Aktionen und Barrierefreiheit
- **AC-M6-09-1** (NFR-08, P10) – *Gegeben* jede destruktive MVP-Aktion, *wenn* sie ausgelöst bzw. konfiguriert wird, *dann* erscheinen Vorschau bzw. Dry-Run und eine Bestätigung mit klarer Folge («nicht rückgängig»), Abbrechen ändert nichts: Restore mit Überschreiben, Key entfernen, manuelles `forget`/`prune` jeweils beim Auslösen; geplantes `forget`/`prune` einmalig beim Speichern der Aufbewahrung bzw. beim Aktivieren oder Ändern des Wartungs-Zeitplans, nicht bei jedem geplanten Lauf · Test: GUI-Smoke + Integration je Aktion
- **AC-M6-09-2** (NFR-09, P28) – *Gegeben* jeder MVP-Screen offscreen, *wenn* der Test die Fokuskette per simuliertem Tab durchläuft, *dann* erreicht er jedes aktivierbare Element genau einmal in sichtbarer Reihenfolge, und Enter bzw. Leertaste lösen es aus · Test: GUI-Smoke (automatisiert). Alltagsbedienung nur mit Tastatur · Test: manuell
- **AC-M6-09-3** (NFR-09, P28) – *Gegeben* jeder MVP-Screen offscreen, *wenn* der Test für jedes aktivierbare Element `Accessible.name` ausliest, *dann* ist er nicht leer, und Statusinformationen sind neben der Farbe auch als Text oder Icon vorhanden · Test: GUI-Smoke (automatisiert). Ausgabe mit Screenreader (Orca) · Test: manuell
- **AC-M6-09-4** (FR-56, NFR-08) – *Gegeben* «Aus Kestic entfernen…» (Backup) bzw. «Ziel entfernen», *wenn* die Aktion ausgelöst wird, *dann* erscheint nur ein Bestätigungsdialog ohne Vorschau oder Dry-Run mit der Folge «Die Snapshots bleiben im Ziel erhalten»; nach Bestätigung ist nur die Kestic-Konfiguration entfernt, Repo und Snapshots sind unverändert; Abbrechen ändert nichts · Test: GUI-Smoke + Integration

### M6-S10 Kit für bestehendes Ziel
- **AC-M6-10-1** (FR-35) – *Gegeben* ein Ziel ohne Recovery-Key-ID in der Konfiguration, *wenn* Kestic startet, *dann* zeigen Status-Übersicht und Tray «Wiederherstellungs-Kit fehlt» mit Aktion «Kit erzeugen» · Test: GUI-Smoke, D-Bus (Fake)
- **AC-M6-10-2** (FR-35) – *Gegeben* «Kit erzeugen» für dieses Ziel, *wenn* der Ablauf mit «sicher aufbewahrt» abschliesst, *dann* existiert ein zusätzlicher Key (`key list`), die Key-ID steht in der Konfiguration und der Hinweis verschwindet · Test: Integration
- **AC-M6-10-3** (FR-35, P42) – *Gegeben* ein Ziel im Zustand «Kit fehlt» nach einem Umzug (AC-M5-05-6), *wenn* «Kit erzeugen…» mit «sicher aufbewahrt» abschliesst, *dann* entsteht der Recovery-Key im neuen Repo, die Key-ID steht in der Konfiguration, und der nicht ausblendbare Hinweis verschwindet · Test: Integration
- **AC-M6-10-4** (FR-35) – *Gegeben* das Ziel ist nicht erreichbar bzw. das gespeicherte Passwort passt nicht (Exit 12), *wenn* Übersicht oder Tray angezeigt werden, *dann* ist «Kit erzeugen…»/«Kit erneuern…» deaktiviert mit sichtbarem Grund · Test: GUI-Smoke + D-Bus

### M6-S11 Fehlerabbildung vollständig
- **AC-M6-11-1** (FR-37) – *Gegeben* die Fehlertabelle in `restic-integration.md`, *wenn* M6 abgeschlossen ist, *dann* enthält sie mindestens Exit 1, 3, 10, 11, 12, unbekannte Codes, JSON `exit_error`, DNS-Fehler, Auth-Fehler am Backend, Timeout, KWallet gesperrt, restic zu alt bzw. nicht gefunden, je mit Text DE/EN und Handlungsoption · Test: Unit (Tabelle vollständig)
- **AC-M6-11-2** (FR-37) – *Gegeben* je Tabelleneintrag ein provozierter Fehler (Temp-Repo, gestoppter `rest-server`, falsches Passwort, gesperrter Secret-Fake), *wenn* er auftritt, *dann* zeigt Kestic genau Text und Aktion aus der Tabelle · Test: Integration

### M6-S12 Log-Ansicht aufbereiten
- **AC-M6-12-1** (FR-18, NFR-10, NFR-02) – *Gegeben* ein Lauf mit restic-JSON-Zeilen (`verbose_status`, `status`, `summary`, `error`/`exit_error`, `forget`), *wenn* die Log-Ansicht im Lauf-Detail geöffnet wird, *dann* zeigt sie verständliche, übersetzte Textzeilen statt roher JSON-Blöcke; unbekannte Message-Typen fallen tolerant auf einen lesbaren Fallback zurück, und Secrets bleiben redacted · Test: Unit + GUI-Smoke
- **AC-M6-12-2** (FR-18) – *Gegeben* Filter `[Alle ▾]` (Alle · Warnungen · Fehler) und Suchfeld in der Log-Ansicht, *wenn* nach Schweregrad oder Suchbegriff gefiltert wird, *dann* filtert Kestic anhand des strukturierten Log-Levels und des angezeigten Texts statt per Substring-Suche im rohen JSON · Test: Unit + GUI-Smoke
- **AC-M6-12-3** (FR-18) – *Gegeben* «Log kopieren» bzw. aktiver Expertenmodus, *wenn* der Benutzer das Log kopiert oder Rohdaten prüft, *dann* ist das kopierte Log direkt lesbar und frei von Secrets, während das Roh-Log weiterhin über «Log-Datei öffnen» (bzw. optional im Expertenmodus) zugänglich bleibt · Test: Unit + GUI-Smoke

## Exit-Kriterien (= MVP fertig)

- Neuer Benutzer richtet Backup ein, verliert «den Rechner» (neue Kestic-Instanz mit leerer Config) und stellt mit dem Kit wieder her.
- `just verify` grün; alle MVP-FRs und -NFRs mit Akzeptanztest abgedeckt.
- Manuelle Abnahme gemäss [manual-acceptance.md](../manual-acceptance.md) vollständig.
