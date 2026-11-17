# M2 Backup-Konfiguration

Teil von [backlog.md](../backlog.md).

**Ziel:** Ein Backup lässt sich vollständig und verständlich konfigurieren: Ausschluss-Regeln (Hauptunterschied zu Déjà Dup), Aufbewahrung mit Vorschau, Quellen-Prüfung. Noch ohne Zeitplan.

**Voraussetzung:** M1.

## Anforderungen

FR-02, FR-04, FR-05, FR-17, FR-29 (Prüfung beim manuellen Lauf; zeitplangesteuert in M3, P36), FR-31 · NFR-08 (für `forget`), NFR-09 · E16, F4, F10, F12 · P8, P10, P11, P12, P32, P41 · U18

## Stories

| ID | Story |
| --- | --- |
| M2-S00 | Projekt-Setup GitHub: Remote `Swissbite/kestic` verknüpfen, `ubuntu:26.04`-Container-CI für `just verify`, `README.md` auf Englisch + Sync-Regel je Meilenstein |
| M2-S01 | Ausschluss-Regeln: Modell (einfache Ordner-Regeln + Expertenregeln), Erzeugung der Exclude-Datei (Env-Variablen, `!`, `**`) (FR-02) |
| M2-S02 | Ausschluss-Regeln GUI: einfacher Modus (Ordner wählen), Expertenmodus (Syntax), Expertenregeln im einfachen Modus schreibgeschützt (F12, E16) |
| M2-S03 | Hinweis im Editor: Negation unter ganz ausgeschlossenem Ordner wirkt nicht |
| M2-S04 | Vorschau / Dry-Run: was würde gesichert, Grösse (FR-17) |
| M2-S05 | Aufbewahrung: Modell aller `--keep-*`-Regeln, Vorlagen, Validierung (mind. eine Regel, nie `--unsafe-allow-remove-all`) (FR-05) |
| M2-S06 | Aufbewahrung GUI: nach Anzahl / nach Zeitraum, Erklärung «letzte n täglichen» vs. «tägliche der letzten n Tage», Vorschau per `forget --dry-run` mit Gründen (NFR-08) |
| M2-S07 | `forget` nach Backup mit Status `ok` oder `Warnung`; Teilerfolg → `Warnung`, Nachholen (F4) |
| M2-S08 | Exit 3: Liste nicht gelesener Dateien, Aktion «zu Ausschluss-Regeln hinzufügen» (F4) |
| M2-S09 | Quellen-Prüfung beim manuellen Lauf: Pfad vorhanden; «Laufwerk» automatisch erkennen (Mount, FS-UUID über Gerät bzw. Mount-Quelle + Dateisystemtyp), eingehängt + gleiche Kennung + nicht leer (FR-29, F10) |
| M2-S10 | Warnung bei verschachtelten Quellen auf dasselbe Ziel (FR-31) |
| M2-S11 | Übersicht der effektiven Konfiguration pro Backup (FR-04) |

## Akzeptanzkriterien

Test-Isolation gemäss [test-strategy.md](../test-strategy.md). `<state>` = `$XDG_STATE_HOME/kestic`.

### M2-S00 GitHub-Repository, Verify-CI & englisches README
- **AC-M2-00-1** – *Gegeben* das öffentliche GitHub-Repository `Swissbite/kestic`, *wenn* M2 startet, *dann* ist es als `origin` verknüpft, der abgeschlossene M1-Stand liegt auf `main`, und für den Meilenstein besteht auf GitHub ein schlankes Tracking-Issue (Feinsteuerung und Story-ACs bleiben allein in `docs/spec/`) · Test: manuell (Git-/GitHub-Prüfung)
- **AC-M2-00-2** – *Gegeben* ein Push oder Pull Request auf GitHub, *wenn* `.github/workflows/verify.yml` in einem sauberen `ubuntu:26.04`-Container (mit den Abhängigkeiten aus `scripts/bootstrap.sh` inkl. `gnome-keyring`, `gettext`, `desktop-file-utils`, `locales` mit `en_US.UTF-8` und QML-Laufzeitmodulen) läuft, *dann* läuft `just verify` grün durch und lädt das `.deb` sowie `target/smoke-artifacts/*.png` als Artefakte hoch · Test: Integration (GitHub-Actions-Lauf)
- **AC-M2-00-3** – *Gegeben* das öffentliche Repository, *wenn* `README.md` gelesen wird, *dann* ist es auf Englisch, beschreibt den aktuellen Stand (inkl. Build-, Test- und Paket-Hinweisen) und vermerkt, dass die interne Arbeitsspezifikation unter `docs/spec/` auf Deutsch geführt wird · Test: manuell (Review) + `reuse lint`
- **AC-M2-00-4** – *Gegeben* `AGENTS.md`, *wenn* ein Meilenstein abgeschlossen wird, *dann* ist dort verankert, das englische `README.md` (und etwaige englische Einstiegsdoku) jeweils mit dem neuen Meilenstein-Stand abzugleichen, während `docs/spec/` einsprachig Deutsch bleibt · Test: manuell (Review)

### M2-S01 Ausschluss-Regeln Modell und Exclude-Datei
- **AC-M2-01-1** (FR-02, F13) – *Gegeben* Backup mit Ausschluss-Regeln, *wenn* ein Lauf startet, *dann* liegt die Exclude-Datei unter `<state>/excludes/<backup-id>.txt`, und der restic-Aufruf enthält `--exclude-file` mit genau diesem Pfad · Test: Unit
- **AC-M2-01-2** (FR-02; Globs, Negation und Env-Expansion verhalten sich wie in [restic-integration.md](../restic-integration.md), ebenso AC-M2-01-3 bis -05) – *Gegeben* Quelle mit `a/.git/x`, `proj/b/.git/y`, `a/f.txt` und Regel `.git`, *wenn* gesichert, *dann* enthält der Snapshot `a/f.txt` und keinen `.git`-Pfad auf irgendeiner Ebene · Test: Integration (Temp-Repo)
- **AC-M2-01-3** (FR-02) – *Gegeben* Regel `**/build/**`, *wenn* gesichert, *dann* fehlen alle Dateien unter jedem `build`-Ordner, `build.txt` bleibt · Test: Integration (Temp-Repo)
- **AC-M2-01-4** (FR-02) – *Gegeben* Expertenregel `$TESTROOT/src/excl` und gesetztes `TESTROOT`, *wenn* gesichert, *dann* fehlt `src/excl` (Env-Variable in der Exclude-Datei expandiert) · Test: Integration (Temp-Repo)
- **AC-M2-01-5** (FR-02) – *Gegeben* Regeln `dir/*` und `!dir/inner`, *wenn* gesichert, *dann* ist `dir/inner/` im Snapshot, `dir/other.txt` nicht · Test: Integration (Temp-Repo)
- **AC-M2-01-6** (FR-02, negativ) – *Gegeben* Expertenregel `.*\.txt$`, *wenn* gesichert, *dann* sind alle `.txt`-Dateien im Snapshot (kein Regex), und Kestic übersetzt die Regel nicht stillschweigend in einen Glob · Test: Integration (Temp-Repo)
- **AC-M2-01-7** (FR-02, P8) – *Gegeben* `HOME=/home/alt` und einfache Ordner-Regel für `/home/alt/Downloads`, *wenn* die Exclude-Datei erzeugt wird, *dann* enthält sie `$HOME/Downloads` · Test: Unit
- **AC-M2-01-9** (FR-02, P8) – *Gegeben* einfache Ordner-Regel für `/media/daten/tmp` (ausserhalb `$HOME`), *dann* enthält die Exclude-Datei `/media/daten/tmp` als absoluten, verankerten Pfad · Test: Unit
- **AC-M2-01-10** (FR-02, P8) – *Gegeben* Exclude-Datei mit `$HOME/Downloads`, erzeugt unter `HOME=/home/alt`, *wenn* sie mit `HOME=/home/neu` und Quelle `/home/neu` verwendet wird, *dann* fehlt `/home/neu/Downloads` im Snapshot · Test: Integration (Temp-Repo, `HOME` je Lauf auf Tempdir)
- **AC-M2-01-8** (FR-02, negativ) – *Gegeben* Expertenregel mit Komma oder Leerzeichen im Pfad, *wenn* die Exclude-Datei erzeugt wird, *dann* steht sie unverändert auf eigener Zeile und schliesst genau diesen Pfad aus · Test: Integration (Temp-Repo)

### M2-S02 Ausschluss-Regeln GUI
- **AC-M2-02-1** (FR-02, F12) – *Gegeben* Expertenmodus aus, *wenn* der Editor öffnet, *dann* Ordner-Regeln editierbar und «Ordner hinzufügen» über den KDE-Dateidialog · Test: GUI-Smoke (offscreen + Screenshot)
- **AC-M2-02-2** (F12) – *Gegeben* 2 Ordner-Regeln und 3 Expertenregeln, Expertenmodus aus, *wenn* der Editor öffnet, *dann* 2 Ordner-Regeln editierbar und «weitere Regeln (3)» schreibgeschützt mit Link in den Expertenmodus · Test: GUI-Smoke
- **AC-M2-02-3** (E16) – *Gegeben* Expertenmodus ein, *wenn* der Editor öffnet, *dann* sind alle Regeln als Text in voller restic-Syntax editierbar; derselbe Schalter «Expertenmodus» wie für die Befehlstransparenz (FR-34) · Test: GUI-Smoke (offscreen + Screenshot)
- **AC-M2-02-4** (NFR-09) – *Gegeben* Editor in beiden Modi, *wenn* nur Tastatur (Tab, Enter, Leertaste, Esc), *dann* Regel hinzufügen, ändern, entfernen, speichern möglich; jedes Bedienelement mit zugänglichem Namen · Test: GUI-Smoke + manuell
- **AC-M2-02-5** (FR-02, F14) – *Gegeben* geänderte Regel, *wenn* gespeichert, *dann* schreibt der Daemon `config.toml` und erzeugt die Exclude-Datei neu; die GUI schreibt keine Datei · Test: D-Bus (dbus-run-session)
- **AC-M2-02-6** (U18) – *Gegeben* geänderte Ausschluss-Regeln, *wenn* `[Speichern]`, *dann* ruft die GUI per D-Bus `UpdateBackup` auf; der Zwischenstand lebt bis dahin nur in der GUI, `config.toml` unverändert; lehnt `core` ab, zeigt die GUI den Fehler und behält den Zwischenstand · Test: D-Bus (dbus-run-session)
- **AC-M2-02-7** (U18) – *Gegeben* ungespeicherte Änderungen, *wenn* der Editor verlassen wird, *dann* Rückfrage «Änderungen speichern?» mit `[Speichern]` und `[Verwerfen]`; ohne Änderungen keine Rückfrage · Test: GUI-Smoke

### M2-S03 Hinweis wirkungslose Negation
- **AC-M2-03-1** (FR-02) – *Gegeben* Regeln `dir` und `!dir/inner`, *wenn* der Editor prüft, *dann* Hinweis bei `!dir/inner`, dass die Negation unter ganz ausgeschlossenem Ordner nicht wirkt, mit Vorschlag `dir/*` · Test: Unit + GUI-Smoke
- **AC-M2-03-2** (FR-02, negativ) – *Gegeben* `dir/*` und `!dir/inner`, *dann* kein Hinweis · Test: Unit
- **AC-M2-03-3** (FR-02) – *Gegeben* Hinweis angezeigt, *wenn* trotzdem gespeichert wird, *dann* wird gespeichert (Hinweis, keine Sperre) · Test: Unit

### M2-S04 Vorschau / Dry-Run
- **AC-M2-04-1** (FR-17) – *Gegeben* Quelle mit bekannten Dateien und Ausschluss-Regeln, *wenn* «Vorschau», *dann* stimmen Liste und Gesamtgrösse mit `restic backup --dry-run -vv` (gleiche Argumente) überein; kein Snapshot entsteht · Test: Integration (Temp-Repo)
- **AC-M2-04-2** (FR-17) – *Gegeben* Backup ohne Snapshot im Ziel, *wenn* die Vorschau läuft, *dann* Grössenschätzung für das erste Backup · Test: Integration (Temp-Repo)
- **AC-M2-04-3** (FR-17, P32, negativ) – *Gegeben* REST-Ziel mit gestopptem `rest-server`, *wenn* «Vorschau», *dann* scheitert der Vorab-Check (Connect-Timeout 10 s), kein restic-Prozess, spätestens 15 s nach dem Klick «Ziel nicht erreichbar» gemäss FR-37 · Test: Integration (gestoppter `rest-server`, Zeitmessung)

### M2-S05 Aufbewahrung Modell und Validierung
- **AC-M2-05-1** (FR-05) – *Gegeben* je eine Regel `--keep-last`, `--keep-hourly`, `--keep-daily`, `--keep-weekly`, `--keep-monthly`, `--keep-yearly` mit Zahl und «unbegrenzt», *wenn* `forget` gebaut wird, *dann* Option mit `n` bzw. `unlimited` · Test: Unit
- **AC-M2-05-2** (FR-05) – *Gegeben* je eine Regel `--keep-within`, `--keep-within-hourly/-daily/-weekly/-monthly/-yearly` mit `30d`, `6m`, `2y`, *dann* Dauer unverändert im Aufruf; ungültige Dauer `30x` abgewiesen · Test: Unit
- **AC-M2-05-3** (FR-05) – *Gegeben* Vorlage «Standard», *dann* `--keep-daily 7 --keep-weekly 4 --keep-monthly 12` · Test: Unit
- **AC-M2-05-4** (FR-05, negativ) – *Gegeben* keine Regel, *wenn* gespeichert, *dann* lehnt `core` mit Meldung ab, Config unverändert · Test: Unit
- **AC-M2-05-5** (FR-05, negativ) – *Gegeben* beliebige gültige Aufbewahrung, *dann* enthält `forget` nie `--unsafe-allow-remove-all` · Test: Unit
- **AC-M2-05-6** (FR-05) – *Gegeben* Backup `<backup-id>`, *dann* enthält `forget` `--tag kestic:<backup-id> --group-by tags` und trifft keinen Snapshot mit `kestic-meta:<backup-id>` oder eines anderen Backups · Test: Integration (Temp-Repo mit zwei Backups + Metadaten-Snapshot)
- **AC-M2-05-7** (FR-05) – *Gegeben* je ein Snapshot (per `--time`, 12:00) am 2026-11-01, 02, 03, 04 und 2026-11-12, 13, 14, 15 (Lücke 5.–11. November), *wenn* `forget --dry-run` mit `--keep-daily 7` bzw. `--keep-within-daily 7d`, *dann* behält `--keep-daily 7` die 7 jüngsten Tage mit Snapshots (15., 14., 13., 12., 4., 3., 2.; entfernt 1.) und `--keep-within-daily 7d` nur Tage innerhalb 7 Tagen vor dem neuesten (15., 14., 13., 12.; entfernt 1.–4.) · Test: Integration (Temp-Repo)
- **AC-M2-05-8** (FR-05) – *Gegeben* neuester Snapshot 60 Tage alt und `--keep-within 30d`, *wenn* `forget` läuft, *dann* bleibt der neueste (Zeitraum ab neuestem Snapshot, nicht ab jetzt) · Test: Integration (Temp-Repo)

### M2-S06 Aufbewahrung GUI und Vorschau
- **AC-M2-06-1** (FR-05) – *Gegeben* Aufbewahrungs-Editor, *wenn* geöffnet, *dann* «nach Anzahl» und «nach Zeitraum» kombinierbar plus Vorlagen; bei `täglich` erklärt ein Text «die letzten n täglichen Snapshots» vs. «tägliche der letzten n Tage» · Test: GUI-Smoke
- **AC-M2-06-2** (FR-05, NFR-08, P41) – *Gegeben* geänderte Aufbewahrung und Ziel mit Snapshots, *wenn* «Vorschau», *dann* pro Snapshot «behalten» mit Grund (aus «Reasons» von `forget --dry-run --no-lock`) oder «entfernen»; im Repo wird nichts gelöscht · Test: Integration (Temp-Repo) + GUI-Smoke
- **AC-M2-06-3** (NFR-08, P10) – *Gegeben* Vorschau würde 5 Snapshots entfernen, *wenn* gespeichert, *dann* Bestätigung «5 Snapshots werden beim nächsten Lauf entfernt, nicht rückgängig»; ohne Bestätigung bleibt die alte Aufbewahrung · Test: GUI-Smoke
- **AC-M2-06-5** (NFR-08, P10, negativ) – *Gegeben* Vorschau entfernt keinen Snapshot, *wenn* gespeichert, *dann* keine Bestätigung; ein späteres `forget` nach einem Backup mit derselben Aufbewahrung fragt nicht erneut · Test: GUI-Smoke + Integration
- **AC-M2-06-4** (NFR-09) – *Gegeben* Aufbewahrungs-Editor, *wenn* nur Tastatur, *dann* Felder, Vorlagen, Vorschau und Bestätigung erreichbar · Test: GUI-Smoke + manuell
- **AC-M2-06-6** (NFR-08, P41) – *Gegeben* laufendes Backup auf demselben Ziel (nicht-exklusiver Lock, z. B. `restic backup --stdin` mit langsamer Quelle), *wenn* «Vorschau», *dann* Ergebnis statt Exit 11; Aufruf enthält `--no-lock` · Test: Integration (Temp-Repo)
- **AC-M2-06-7** (U18) – *Gegeben* geänderte Aufbewahrungsregeln, *wenn* `[Speichern]`, *dann* `UpdateBackup` per D-Bus, `core` validiert, erst danach steht die Änderung in `config.toml`; vorher unverändert · Test: D-Bus (dbus-run-session)
- **AC-M2-06-8** (U18) – *Gegeben* ungespeicherte Änderungen, *wenn* der Editor verlassen wird, *dann* Rückfrage «Änderungen speichern?» mit `[Speichern]` und `[Verwerfen]`; «Verwerfen» lässt `config.toml` unverändert · Test: GUI-Smoke

### M2-S07 `forget` nach Backup, Teilerfolg
- **AC-M2-07-1** (FR-05, F4) – *Gegeben* Backup endet mit Exit 0, *dann* folgt `forget --tag kestic:<backup-id> --group-by tags` mit den Regeln, Lauf `ok` · Test: Integration (Temp-Repo)
- **AC-M2-07-2** (F4) – *Gegeben* Backup endet mit Exit 3, *dann* läuft `forget` trotzdem, Lauf `Warnung` · Test: Integration (Temp-Repo, unlesbare Datei per `chmod 000`; nicht als root, sonst übersprungen)
- **AC-M2-07-3** (F4, P11) – *Gegeben* ein zweiter laufender `restic backup --stdin` (langsame Quelle) hält einen **nicht-exklusiven** Lock, *wenn* das Kestic-Backup läuft, *dann* `backup` Exit 0, anschliessendes `forget` Exit 11, Lauf `Warnung`, Backup im Zustand als «forget offen» vermerkt · Test: Integration (Temp-Repo, Fremd-Backup als Lock-Halter)
- **AC-M2-07-4** (F4) – *Gegeben* Backup mit «forget offen», *wenn* der nächste Backup- oder Wartungslauf am Ziel läuft, *dann* wird `forget` ausgeführt und die Markierung gelöscht · Test: Integration (Temp-Repo)
- **AC-M2-07-5** (F4, negativ) – *Gegeben* Backup endet mit Exit 1, *dann* kein `forget`, Lauf `Fehler` · Test: Integration (Temp-Repo)

### M2-S08 Exit 3: nicht gelesene Dateien
- **AC-M2-08-1** (F4) – *Gegeben* Datei ohne Leserecht (`chmod 000`) in der Quelle, *wenn* das Backup mit Exit 3 endet, *dann* listet das Lauf-Detail genau diesen Pfad unter «nicht gelesene Dateien» · Test: Integration (Temp-Repo; nicht als root, sonst übersprungen)
- **AC-M2-08-2** (F4) – *Gegeben* Liste aus AC-M2-08-1, *wenn* «zu Ausschluss-Regeln hinzufügen», *dann* enthält das Backup eine Regel für den Pfad, der nächste Lauf endet ohne diese Warnung · Test: Integration + GUI-Smoke

### M2-S09 Quellen-Prüfung und Laufwerk
- **AC-M2-09-1** (FR-29) – wie AC-M1-23-1, zusätzlich für Laufwerk-Quellen: *Gegeben* Quelle vom Typ «Laufwerk» nicht eingehängt, mit fremder FS-UUID bzw. leer, *wenn* das Backup in GUI oder Tray angezeigt wird, *dann* ist «Jetzt sichern» deaktiviert, Grund («Laufwerk nicht eingehängt», «anderes Laufwerk», «Quelle leer») daneben und im Tooltip; kein restic-Prozess · Test: GUI-Smoke + D-Bus (Mount-Fake)
- **AC-M2-09-2** (FR-29, FR-30) – wie AC-M1-09-6/-7, zusätzlich für Laufwerk-Quellen: *Gegeben* Laufwerk bei Anzeige eingehängt, vor dem Klick ausgehängt, *wenn* «Jetzt sichern» geklickt wird, *dann* `ManualRunBlocked` mit Grund `source_unavailable`, kein restic-Prozess · Test: D-Bus (Mount-Fake)
- **AC-M2-09-3** (F10) – *Gegeben* Quelle auf anderem Dateisystem als `/` bzw. `$HOME` laut `/proc/self/mountinfo` (auch Unterordner davon) mit Blockgerät, *wenn* das Backup angelegt wird, *dann* Quellentyp «Laufwerk» und FS-UUID des Geräts gespeichert (über `blkid` bzw. udev, nicht aus mountinfo) · Test: Unit (simuliertes Mountinfo + Geräte-Fake)
- **AC-M2-09-7** (F10) – *Gegeben* Quelle auf Dateisystem ohne UUID (sshfs, NFS, tmpfs laut mountinfo), *wenn* angelegt, *dann* Quellentyp «Laufwerk», Kennung = Mount-Quelle + Dateisystemtyp (z. B. `nas:/fotos` + `nfs4`) · Test: Unit (simuliertes Mountinfo)
- **AC-M2-09-4** (F10, FR-29) – *Gegeben* Quellentyp «Laufwerk» und manueller Lauf, *wenn* vorher (a) nicht eingehängt, (b) andere UUID bzw. andere Mount-Quelle/anderer Typ oder (c) Mountpoint leer, *dann* in jedem Fall kein restic-Prozess, Meldung «Quelle nicht verfügbar» mit dem jeweiligen Fall · Test: Unit (simuliertes Mountinfo + Geräte-Fake + Temp-Verzeichnis, drei Fälle)
- **AC-M2-09-5** (FR-29, negativ) – *Gegeben* leerer, nicht eingehängter Mountpoint als Quelle (ohne Prüfung entstünde laut restic ein 0-B-Snapshot mit Exit 0), *dann* ruft Kestic kein `backup` auf · Test: Integration (Temp-Repo, leerer Ordner als «Laufwerk»)
- **AC-M2-09-6** (F10) – *Gegeben* Quellentyp «Ordner» auf demselben Dateisystem wie `$HOME`, leer, *dann* läuft das Backup normal (Leer-Prüfung nur für «Laufwerk») · Test: Integration

### M2-S10 Verschachtelte Quellen
- **AC-M2-10-1** (FR-31) – *Gegeben* Backup `~` auf Ziel Z, *wenn* Backup `~/Bilder` auf Z angelegt wird, *dann* Warnung «`~/Bilder` ist bereits in Backup `~` enthalten»; Speichern bleibt möglich · Test: Unit + GUI-Smoke
- **AC-M2-10-2** (FR-31, negativ) – *Gegeben* dieselben Quellen auf verschiedenen Zielen, *dann* keine Warnung · Test: Unit
- **AC-M2-10-3** (FR-31, negativ) – *Gegeben* `/data/foto` und `/data/fotos`, *dann* keine Warnung (Vergleich nach Pfadkomponenten, nicht Zeichenkette) · Test: Unit

### M2-S11 Übersicht der effektiven Konfiguration
- **AC-M2-11-1** (FR-04) – *Gegeben* vollständig konfiguriertes Backup, *wenn* die Übersicht öffnet, *dann* zeigt sie Quelle, Quellentyp, Ziel, alle Ausschluss-Regeln (inkl. Expertenregeln), Aufbewahrung als Klartext, Optionen und Pfad der Exclude-Datei · Test: GUI-Smoke (Screenshot)
- **AC-M2-11-2** (FR-04, NFR-02) – *Gegeben* Übersicht, *dann* keine Secret-Werte, nur Hinweise wie «Passwort in KWallet» · Test: GUI-Smoke
- **AC-M2-11-3** (manuell) – Verständlichkeit von Ausschluss- und Aufbewahrungs-Editor durch eine Person ohne restic-Kenntnisse geprüft · Test: manuell (→ `manual-acceptance.md`)

## Exit-Kriterien

- Öffentliches GitHub-Repo (`Swissbite/kestic`), `ubuntu:26.04`-Verify-CI und englisches `README.md` (mit Sync-Regel in `AGENTS.md`) stehen.
- Backup mit einfachen und Expertenregeln, Aufbewahrung mit Vorschau, Quellen-Prüfung funktioniert.
- `just verify` grün; Screenshots für Editor, Aufbewahrung, Übersicht.
- Manuelle Abnahme: Verständlichkeit von Ausschluss- und Aufbewahrungs-Editor.
