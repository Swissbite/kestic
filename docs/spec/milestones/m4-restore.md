# M4 Restore

Teil von [backlog.md](../backlog.md).

**Ziel:** Daten kommen sicher zurück: Snapshots sehen, vergleichen, durchsuchen, wiederherstellen, ohne versehentlich zu überschreiben.

**Voraussetzung:** M1 (fachlich unabhängig von M2/M3, parallel möglich); `M4-S00` ist Voraussetzung für `M4-S01` und `M5-S01`. M4 bringt den Bereich «Wiederherstellen» in die Tab-Leiste (in M1 fehlt er). Ablage gemäss [architecture.md](../architecture.md) §Code-Struktur.

## Anforderungen

FR-06, FR-07, FR-10, FR-36 · NFR-08 (Restore mit Überschreiben), NFR-09, NFR-12 · F5, F6, P17–P20, P38

## Stories

| ID | Story |
| --- | --- |
| M4-S00 | Strukturelle Vorbereitung: `service.rs`, `backend.rs`, `models.rs`, `model.rs` und `Main.qml` gemäss §Code-Struktur aufteilen |
| M4-S01 | Snapshot-Liste pro Backup (Zeit, Grösse, Host), Metadaten-Snapshots ausgeblendet (FR-06) |
| M4-S02 | Unterschiede zwischen zwei Snapshots (`diff --json`) (FR-07) |
| M4-S03 | Browsing: Ordnerliste mit Breadcrumb eines Snapshots via `ls --json`, lazy geladen (FR-10) |
| M4-S04 | Suche über Snapshots (`find`) (FR-10) |
| M4-S05 | Wiederherstellen: Default neuer Ordner; Originalort nur mit Wahl `never` (Default) / `if-newer` / `if-changed` / `always` und Vorschau `--dry-run` (F5, NFR-08) |
| M4-S06 | Pfad-Abbildung bei anderem Benutzernamen (F6) |
| M4-S07 | Fremde Snapshots (ohne Kestic-Tag) als «nicht zugeordnet» anzeigen, durchsuchen, wiederherstellen (FR-36) |

## Akzeptanzkriterien

Test-Isolation gemäss [test-strategy.md](../test-strategy.md); nur Temp-Repos. «Integration» = gegen echtes restic mit Temp-Repo.

### M4-S00 Strukturelle Vorbereitung
- **AC-M4-00-1** (NFR-12) – *Gegeben* die bisher gewachsenen Quelldateien `daemon/src/service.rs`, `gui/src/backend.rs`, `gui/src/models.rs`, `core/src/model.rs` und `gui/src/qml/Main.qml`, *wenn* gemäss [architecture.md](../architecture.md) §Code-Struktur aufgeteilt wird, *dann* (a) liegen alle Module, Modelle und Dialoge an den vorgegebenen Orten, (b) überschreitet keine `.rs`- oder `.qml`-Datei unter `src/` 1500 Zeilen und keine Funktion 150 Zeilen, und (c) bleiben die öffentliche API von `core` und das D-Bus-Interface unverändert · Test: Unit + Clippy/Grössenprüfung
- **AC-M4-00-2** (NFR-12) – *Gegeben* die aufgeteilte Codebasis, *wenn* die gesamte Suite läuft, *dann* (a) bleibt das Verhalten unverändert, (b) ändern sich bestehende Tests höchstens bei den `use`-Pfaden, und (c) ist `just verify` vollständig grün · Test: Integration + GUI-Smoke (`just verify`)

### M4-S01 Snapshot-Liste
- **AC-M4-01-1** (FR-06) – *Gegeben* drei Snapshots von `b1` (Tag `kestic:b1`), *wenn* die Liste von `b1` lädt, *dann* (a) zeigt sie genau diese drei Snapshots, (b) sind sie neueste zuerst sortiert, und (c) trägt jeder Eintrag Zeitpunkt, Grösse und Host · Test: Integration
- **AC-M4-01-2** (FR-06, E10) – *Gegeben* zusätzlich zwei Metadaten-Snapshots `kestic-meta:b1`, *wenn* die Liste von `b1` lädt, *dann* (a) erscheinen sie nicht in der Liste, und (b) fliessen sie nicht in die Anzahl ein · Test: Integration
- **AC-M4-01-3** (FR-06) – *Gegeben* Snapshots von `b1` und `b2` im selben Repo, *wenn* die Liste von `b1` lädt, *dann* (a) enthält sie keinen Snapshot mit `kestic:b2` · Test: Integration
- **AC-M4-01-4** (FR-06, NFR-09) – *Gegeben* die offene Liste, *wenn* nur die Tastatur benutzt wird, *dann* (a) lassen sich alle Snapshots wählen, und (b) lassen sich Unterschiede, Durchsuchen und Wiederherstellen auslösen · Test: GUI-Smoke (offscreen + Screenshot), manuell
- **AC-M4-01-5** (FR-06, P17) – *Gegeben* ein Snapshot-JSON ohne Zusammenfassung (Fixture, wie ältere restic-Versionen bzw. fremde Snapshots), *wenn* die Liste lädt, *dann* (a) zeigt die Grösse «unbekannt», (b) erfolgt kein `stats`-Aufruf, und (c) tritt kein Fehler auf · Test: Unit (Fixture)

### M4-S02 Unterschiede zwischen Snapshots
- **AC-M4-02-1** (FR-07) – *Gegeben* Snapshot A mit `docs/a.txt` und B, in dem `a.txt` geändert, `b.txt` neu, `c.txt` entfernt ist, *wenn* A → B angezeigt wird, *dann* (a) erscheinen `a.txt` als geändert, `b.txt` als neu und `c.txt` als entfernt, und (b) ist das Ergebnis deckungsgleich mit `restic diff --json A B` · Test: Integration
- **AC-M4-02-2** (FR-07, NFR-10) – *Gegeben* `diff`-JSON mit unbekanntem Feld bzw. Message-Typ, *wenn* geparst wird, *dann* (a) entsteht die Anzeige ohne Fehler, und (b) wird Unbekanntes ignoriert · Test: Unit
- **AC-M4-02-3** (FR-07) – *Gegeben* zwei identische Snapshots, *wenn* Unterschiede angezeigt werden, *dann* (a) erscheint «keine Unterschiede» statt einer leeren Liste · Test: Integration

### M4-S03 Browsing
- **AC-M4-03-1** (FR-10) – *Gegeben* ein Snapshot von `/tmp/…/src` mit Unterordnern, *wenn* er durchsucht wird, *dann* (a) zeigt die Ordnerliste mit Breadcrumb Ordner und Dateien mit Name, Grösse und Änderungszeit gemäss `restic ls --json`, und (b) wird `restic mount` nicht verwendet · Test: Integration
- **AC-M4-03-2** (FR-10) – *Gegeben* ein Snapshot mit 100 000 Dateien in 1 000 Ordnern, *wenn* der Wurzelordner öffnet, *dann* (a) lädt nur die geöffnete Ebene (nicht der ganze Baum vorab), und (b) reagiert die Ansicht weiter auf Eingaben · Test: Integration, GUI-Smoke
- **AC-M4-03-3** (FR-10, NFR-09) – *Gegeben* die offene Snapshot-Ansicht, *wenn* nur die Tastatur benutzt wird, *dann* (a) geht es per Ordnerliste in Unterordner, (b) geht es per Breadcrumb in jede übergeordnete Ebene zurück (keine Baumansicht zum Auf- und Zuklappen), und (c) lassen sich Einträge zum Wiederherstellen markieren · Test: GUI-Smoke, manuell
- **AC-M4-03-4** (K3, FR-10) – *Gegeben* ein Build mit M4, *wenn* das Hauptfenster startet, *dann* (a) zeigt die Tab-Leiste «Wiederherstellen» zwischen «Übersicht» und «Backups», (b) öffnet der Bereich die Snapshot-Auswahl, und (c) fehlt er in einem Build nur mit M1 · Test: GUI-Smoke (offscreen + Screenshot)

### M4-S04 Suche über Snapshots
- **AC-M4-04-1** (FR-10) – *Gegeben* `bericht.pdf` in zwei von drei Snapshots von `b1` unter verschiedenen Pfaden, *wenn* danach gesucht wird, *dann* (a) erscheinen genau zwei Treffer mit Snapshot und Pfad, und (b) öffnet sich ein Treffer direkt in der Ordnerliste mit Breadcrumb · Test: Integration
- **AC-M4-04-2** (FR-10) – *Gegeben* ein Suchbegriff ohne Treffer, *wenn* gesucht wird, *dann* (a) erscheint «keine Treffer», und (b) tritt kein Fehler auf · Test: Integration
- **AC-M4-04-3** (FR-10, E10) – *Gegeben* Metadaten-Snapshots `kestic-meta:b1`, *wenn* gesucht wird, *dann* (a) liefern sie keine Treffer · Test: Integration
- **AC-M4-04-4** (FR-10, P18) – *Gegeben* `bericht.pdf` in Snapshots von `b1` und `b2` im selben Ziel, *wenn* in `b1` gesucht wird, *dann* (a) erscheinen nur Treffer aus Snapshots mit `kestic:b1` · Test: Integration

### M4-S05 Wiederherstellen
- **AC-M4-05-1** (F5) – *Gegeben* ein Snapshot mit `docs/a.txt`, *wenn* der Restore-Dialog öffnet, *dann* (a) ist ein **neuer Ordner** als Ziel vorausgewählt, und (b) nicht der Originalort · Test: GUI-Smoke
- **AC-M4-05-2** (F5) – *Gegeben* Restore in einen neuen, leeren Ordner, *wenn* wiederhergestellt wird, *dann* (a) liegt `a.txt` mit identischem Inhalt dort, und (b) endet der Lauf mit `ok` · Test: Integration
- **AC-M4-05-3** (F5, NFR-08) – *Gegeben* Restore an den Originalort mit abweichender `a.txt`, *wenn* der Modus nicht geändert wird, *dann* (a) gilt `never`, (b) bleibt `a.txt` unverändert, und (c) nennt die Vorschau sie «nicht überschrieben» · Test: Integration
- **AC-M4-05-4** (F5, NFR-08) – *Gegeben* Restore an den Originalort, *wenn* «wiederherstellen» gewählt wird, *dann* (a) läuft zuerst `restore --dry-run`, (b) listet die Vorschau je Datei «neu / überschrieben / unverändert», und (c) wird erst nach Bestätigung mit klarer Folge («n Dateien werden überschrieben, nicht rückgängig») geschrieben · Test: Integration, GUI-Smoke
- **AC-M4-05-5** (F5) – *Gegeben* Originalort mit älterer bzw. neuerer bzw. geänderter `a.txt`, *wenn* mit `if-newer`, `if-changed` bzw. `always` wiederhergestellt wird, *dann* (a) entspricht das Ergebnis jeweils `restic restore --overwrite <modus>` · Test: Integration
- **AC-M4-05-6** (F5, NFR-08) – *Gegeben* ein beliebiger Restore-Aufruf durch Kestic, *wenn* der restic-Befehl geprüft wird, *dann* (a) enthält er immer explizit `--overwrite`, und (b) nutzt er nie den restic-Default `always` implizit · Test: Unit
- **AC-M4-05-7** (F5, NFR-08) – *Gegeben* der Benutzer bricht die Bestätigung ab, *wenn* der Dialog schliesst, *dann* (a) ist am Zielort keine Datei verändert · Test: Integration
- **AC-M4-05-8** (FR-10) – *Gegeben* in der Ordnerliste ist nur `docs/` markiert, *wenn* wiederhergestellt wird, *dann* (a) werden nur Dateien unter `docs/` geschrieben · Test: Integration
- **AC-M4-05-9** (P38) – *Gegeben* ein laufender Backup-Lauf auf dem Ziel, *wenn* ein Restore startet, *dann* (a) läuft er sofort parallel (nicht-exklusiver Lock), und (b) hat er die Art `Restore` · Test: Integration
- **AC-M4-05-10** (P38, ab M3) – *Gegeben* ein laufender Wartungslauf (`prune`) auf dem Ziel, *wenn* ein Restore startet, *dann* (a) steht er auf `ausstehend` (Grund «Warteschlange»), und (b) startet er erst nach der Wartung. Prüfbar erst mit Wartungslauf und Warteschlange aus M3; M4 selbst hängt nur von M1 ab · Test: Integration
- **AC-M4-05-11** (P19 b) – *Gegeben* ein Restore an den Originalort mit `always`, *wenn* danach ein neuer Restore-Dialog öffnet, *dann* (a) ist wieder `never` vorausgewählt · Test: GUI-Smoke
- **AC-M4-05-12** (P38) – *Gegeben* ein Restore aus einem Snapshot mit `kestic:b1`, *wenn* der Lauf angelegt wird, *dann* (a) hat er die Art `Restore`, (b) gehört er zum Ziel, (c) trägt er den Backup-Bezug `b1`, und (d) erscheint er im Verlauf von `b1` und des Ziels · Test: Unit

### M4-S06 Pfad-Abbildung bei anderem Benutzernamen
- **AC-M4-06-1** (F6) – *Gegeben* ein Snapshot mit `/home/alt/Dokumente/a.txt`, aktueller Benutzer `neu`, *wenn* «Originalort» gewählt wird, *dann* (a) schlägt Kestic `/home/neu/Dokumente/a.txt` vor, und (b) zeigt es die Abbildung `alt → neu` · Test: Unit, GUI-Smoke
- **AC-M4-06-2** (F6) – *Gegeben* derselbe Snapshot, *wenn* der Benutzer das vorgeschlagene Ziel ändert, *dann* (a) gilt das gewählte Ziel · Test: Unit
- **AC-M4-06-3** (F6) – *Gegeben* ein Pfad ausserhalb von `/home/<name>/` (z. B. `/media/daten/Fotos`), *wenn* «Originalort» gewählt wird, *dann* (a) bleibt er unverändert · Test: Unit
- **AC-M4-06-4** (F6, P19 a) – *Gegeben* ein Snapshot mit `/home/alt/Projekte/a.txt`, Benutzer `neu`, `/home/neu/Projekte` existiert nicht, *wenn* «Originalort» gewählt wird, *dann* (a) verlangt Kestic einen Zielordner, (b) legt es `/home/neu/Projekte` nicht an, und (c) startet es keinen Restore, bis ein Ziel gewählt ist · Test: Unit, Integration (Temp-Verzeichnis)

### M4-S07 Fremde Snapshots
- **AC-M4-07-1** (FR-36) – *Gegeben* ein per restic-CLI ohne Kestic-Tag angelegter Snapshot, *wenn* das Ziel öffnet, *dann* (a) erscheint er unter «nicht zugeordnet», und (b) erscheint er in keiner Backup-Liste · Test: Integration
- **AC-M4-07-2** (FR-36, P38) – *Gegeben* ein fremder Snapshot, *wenn* er durchsucht und eine Datei daraus wiederhergestellt wird, *dann* (a) gelten AC-M4-03-1 und AC-M4-05-1 bis -05-7, und (b) gehört der Lauf der Art `Restore` zum Ziel ohne Backup-Bezug · Test: Integration
- **AC-M4-07-3** (FR-36, E10) – *Gegeben* Metadaten-Snapshots `kestic-meta:b1` und Snapshots mit `kestic:b1`, *wenn* «nicht zugeordnet» lädt, *dann* (a) erscheint keiner davon dort · Test: Integration
- **AC-M4-07-4** (FR-36, E10, Glossar «verwaist») – *Gegeben* ein Snapshot mit `kestic:x9`, `x9` weder in der Config noch in einem Metadaten-Snapshot (`kestic-meta:*`) des Ziels, *wenn* das Ziel öffnet, *dann* (a) erscheint er als «verwaist», und (b) nicht unter «nicht zugeordnet» · Test: Integration
- **AC-M4-07-5** (Glossar «verwaist», P20) – *Gegeben* ein Snapshot mit `kestic:x9` und ein Metadaten-Snapshot `kestic-meta:x9`, `x9` nicht in der Config, *wenn* das Ziel öffnet, *dann* (a) gilt er **nicht** als «verwaist» (Backup übernehmbar, FR-49), und (b) sind verwaiste Snapshots wie fremde durchsuch- und wiederherstellbar · Test: Integration

## Exit-Kriterien

- Datei und Ordner aus einem Snapshot an neuen Ort und an Originalort (mit Vorschau) wiederhergestellt.
- `just verify` grün; Screenshots Snapshot-Liste, Diff, Browser, Restore-Dialog.
- Manuelle Abnahme: Restore aus einem echten Netzwerk-Ziel (z. B. NAS).
