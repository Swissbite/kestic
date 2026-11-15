# M4 Restore

Teil von [backlog.md](../backlog.md).

**Ziel:** Daten kommen sicher zurück: Snapshots sehen, vergleichen, durchsuchen, wiederherstellen, ohne versehentlich zu überschreiben.

**Voraussetzung:** M1 (fachlich unabhängig von M2/M3, parallel möglich). M4 bringt den Bereich «Wiederherstellen» in die Tab-Leiste (in M1 fehlt er).

## Anforderungen

FR-06, FR-07, FR-10, FR-36 · NFR-08 (Restore mit Überschreiben), NFR-09 · F5, F6, P17–P20, P38

## Stories

| ID | Story |
| --- | --- |
| M4-S01 | Snapshot-Liste pro Backup (Zeit, Grösse, Host), Metadaten-Snapshots ausgeblendet (FR-06) |
| M4-S02 | Unterschiede zwischen zwei Snapshots (`diff --json`) (FR-07) |
| M4-S03 | Browsing: Ordnerliste mit Breadcrumb eines Snapshots via `ls --json`, lazy geladen (FR-10) |
| M4-S04 | Suche über Snapshots (`find`) (FR-10) |
| M4-S05 | Wiederherstellen: Default neuer Ordner; Originalort nur mit Wahl `never` (Default) / `if-newer` / `if-changed` / `always` und Vorschau `--dry-run` (F5, NFR-08) |
| M4-S06 | Pfad-Abbildung bei anderem Benutzernamen (F6) |
| M4-S07 | Fremde Snapshots (ohne Kestic-Tag) als «nicht zugeordnet» anzeigen, durchsuchen, wiederherstellen (FR-36) |

## Akzeptanzkriterien

Test-Isolation gemäss [test-strategy.md](../test-strategy.md); nur Temp-Repos. «Integration» = gegen echtes restic mit Temp-Repo.

### M4-S01 Snapshot-Liste
- **AC-M4-01-1** (FR-06) – *Gegeben* drei Snapshots von `b1` (Tag `kestic:b1`), *wenn* die Liste von `b1` lädt, *dann* zeigt sie genau diese drei, neueste zuerst, je mit Zeitpunkt, Grösse, Host · Test: Integration
- **AC-M4-01-2** (FR-06, E10) – *Gegeben* zusätzlich zwei Metadaten-Snapshots `kestic-meta:b1`, *wenn* die Liste von `b1` lädt, *dann* erscheinen sie weder in der Liste noch in der Anzahl · Test: Integration
- **AC-M4-01-3** (FR-06) – *Gegeben* Snapshots von `b1` und `b2` im selben Repo, *wenn* die Liste von `b1` lädt, *dann* enthält sie keinen Snapshot mit `kestic:b2` · Test: Integration
- **AC-M4-01-4** (FR-06, NFR-09) – *Gegeben* die offene Liste, *wenn* nur die Tastatur benutzt wird, *dann* lassen sich alle Snapshots wählen und Unterschiede, Durchsuchen, Wiederherstellen auslösen · Test: GUI-Smoke (offscreen + Screenshot), manuell
- **AC-M4-01-5** (FR-06, P17) – *Gegeben* ein Snapshot-JSON ohne Zusammenfassung (Fixture, wie ältere restic-Versionen bzw. fremde Snapshots), *wenn* die Liste lädt, *dann* zeigt die Grösse «unbekannt», ohne `stats`-Aufruf und ohne Fehler · Test: Unit (Fixture)

### M4-S02 Unterschiede zwischen Snapshots
- **AC-M4-02-1** (FR-07) – *Gegeben* Snapshot A mit `docs/a.txt` und B, in dem `a.txt` geändert, `b.txt` neu, `c.txt` entfernt ist, *wenn* A → B angezeigt wird, *dann* erscheinen sie als geändert / neu / entfernt, deckungsgleich mit `restic diff --json A B` · Test: Integration
- **AC-M4-02-2** (FR-07, NFR-10) – *Gegeben* `diff`-JSON mit unbekanntem Feld bzw. Message-Typ, *wenn* geparst wird, *dann* entsteht die Anzeige ohne Fehler, Unbekanntes wird ignoriert · Test: Unit
- **AC-M4-02-3** (FR-07) – *Gegeben* zwei identische Snapshots, *wenn* Unterschiede angezeigt werden, *dann* erscheint «keine Unterschiede» statt einer leeren Liste · Test: Integration

### M4-S03 Browsing
- **AC-M4-03-1** (FR-10) – *Gegeben* ein Snapshot von `/tmp/…/src` mit Unterordnern, *wenn* er durchsucht wird, *dann* zeigt die Ordnerliste mit Breadcrumb Ordner und Dateien mit Name, Grösse, Änderungszeit gemäss `restic ls --json`, ohne `restic mount` · Test: Integration
- **AC-M4-03-2** (FR-10) – *Gegeben* ein Snapshot mit 100 000 Dateien in 1 000 Ordnern, *wenn* der Wurzelordner öffnet, *dann* lädt nur die geöffnete Ebene (nicht der ganze Baum vorab) und die Ansicht reagiert weiter auf Eingaben · Test: Integration, GUI-Smoke
- **AC-M4-03-3** (FR-10, NFR-09) – *Gegeben* die offene Snapshot-Ansicht, *wenn* nur die Tastatur benutzt wird, *dann* geht es per Ordnerliste in Unterordner und per Breadcrumb in jede übergeordnete Ebene zurück (keine Baumansicht zum Auf- und Zuklappen), und Einträge lassen sich zum Wiederherstellen markieren · Test: GUI-Smoke, manuell
- **AC-M4-03-4** (K3, FR-10) – *Gegeben* ein Build mit M4, *wenn* das Hauptfenster startet, *dann* zeigt die Tab-Leiste «Wiederherstellen» zwischen «Übersicht» und «Backups», der Bereich öffnet die Snapshot-Auswahl; in einem Build nur mit M1 fehlt er · Test: GUI-Smoke (offscreen + Screenshot)

### M4-S04 Suche über Snapshots
- **AC-M4-04-1** (FR-10) – *Gegeben* `bericht.pdf` in zwei von drei Snapshots von `b1` unter verschiedenen Pfaden, *wenn* danach gesucht wird, *dann* erscheinen genau zwei Treffer mit Snapshot und Pfad, und ein Treffer öffnet sich direkt in der Ordnerliste mit Breadcrumb · Test: Integration
- **AC-M4-04-2** (FR-10) – *Gegeben* ein Suchbegriff ohne Treffer, *wenn* gesucht wird, *dann* erscheint «keine Treffer», kein Fehler · Test: Integration
- **AC-M4-04-3** (FR-10, E10) – *Gegeben* Metadaten-Snapshots `kestic-meta:b1`, *wenn* gesucht wird, *dann* liefern sie keine Treffer · Test: Integration
- **AC-M4-04-4** (FR-10, P18) – *Gegeben* `bericht.pdf` in Snapshots von `b1` und `b2` im selben Ziel, *wenn* in `b1` gesucht wird, *dann* nur Treffer aus Snapshots mit `kestic:b1` · Test: Integration

### M4-S05 Wiederherstellen
- **AC-M4-05-1** (F5) – *Gegeben* ein Snapshot mit `docs/a.txt`, *wenn* der Restore-Dialog öffnet, *dann* ist ein **neuer Ordner** als Ziel vorausgewählt, nicht der Originalort · Test: GUI-Smoke
- **AC-M4-05-2** (F5) – *Gegeben* Restore in einen neuen, leeren Ordner, *wenn* wiederhergestellt wird, *dann* liegt `a.txt` mit identischem Inhalt dort und der Lauf endet `ok` · Test: Integration
- **AC-M4-05-3** (F5, NFR-08) – *Gegeben* Restore an den Originalort mit abweichender `a.txt`, *wenn* der Modus nicht geändert wird, *dann* gilt `never`, `a.txt` bleibt unverändert und die Vorschau nennt sie «nicht überschrieben» · Test: Integration
- **AC-M4-05-4** (F5, NFR-08) – *Gegeben* Restore an den Originalort, *wenn* «wiederherstellen» gewählt wird, *dann* läuft zuerst `restore --dry-run`, die Vorschau listet je Datei «neu / überschrieben / unverändert», und geschrieben wird erst nach Bestätigung mit klarer Folge («n Dateien werden überschrieben, nicht rückgängig») · Test: Integration, GUI-Smoke
- **AC-M4-05-5** (F5) – *Gegeben* Originalort mit älterer bzw. neuerer bzw. geänderter `a.txt`, *wenn* mit `if-newer`, `if-changed` bzw. `always` wiederhergestellt wird, *dann* entspricht das Ergebnis jeweils `restic restore --overwrite <modus>` · Test: Integration
- **AC-M4-05-6** (F5, NFR-08) – *Gegeben* ein beliebiger Restore-Aufruf durch Kestic, *wenn* der restic-Befehl geprüft wird, *dann* enthält er immer explizit `--overwrite` (nie den restic-Default `always` implizit) · Test: Unit
- **AC-M4-05-7** (F5, NFR-08) – *Gegeben* der Benutzer bricht die Bestätigung ab, *wenn* der Dialog schliesst, *dann* ist am Zielort keine Datei verändert · Test: Integration
- **AC-M4-05-8** (FR-10) – *Gegeben* in der Ordnerliste ist nur `docs/` markiert, *wenn* wiederhergestellt wird, *dann* werden nur Dateien unter `docs/` geschrieben · Test: Integration
- **AC-M4-05-9** (P38) – *Gegeben* ein laufender Backup-Lauf auf dem Ziel, *wenn* ein Restore startet, *dann* läuft er sofort parallel als Art `Restore` (nicht-exklusiver Lock) · Test: Integration
- **AC-M4-05-10** (P38, ab M3) – *Gegeben* ein laufender Wartungslauf (`prune`) auf dem Ziel, *wenn* ein Restore startet, *dann* steht er auf `ausstehend` (Grund «Warteschlange») und startet erst nach der Wartung. Prüfbar erst mit Wartungslauf und Warteschlange aus M3; M4 selbst hängt nur von M1 ab · Test: Integration
- **AC-M4-05-11** (P19 b) – *Gegeben* ein Restore an den Originalort mit `always`, *wenn* danach ein neuer Restore-Dialog öffnet, *dann* ist wieder `never` vorausgewählt · Test: GUI-Smoke
- **AC-M4-05-12** (P38) – *Gegeben* ein Restore aus einem Snapshot mit `kestic:b1`, *wenn* der Lauf angelegt wird, *dann* hat er die Art `Restore`, gehört zum Ziel, trägt den Backup-Bezug `b1` und erscheint im Verlauf von `b1` und des Ziels · Test: Unit

### M4-S06 Pfad-Abbildung bei anderem Benutzernamen
- **AC-M4-06-1** (F6) – *Gegeben* ein Snapshot mit `/home/alt/Dokumente/a.txt`, aktueller Benutzer `neu`, *wenn* «Originalort» gewählt wird, *dann* schlägt Kestic `/home/neu/Dokumente/a.txt` vor und zeigt die Abbildung `alt → neu` · Test: Unit, GUI-Smoke
- **AC-M4-06-2** (F6) – *Gegeben* derselbe Snapshot, *wenn* der Benutzer das vorgeschlagene Ziel ändert, *dann* gilt das gewählte Ziel · Test: Unit
- **AC-M4-06-3** (F6) – *Gegeben* ein Pfad ausserhalb von `/home/<name>/` (z. B. `/media/daten/Fotos`), *wenn* «Originalort» gewählt wird, *dann* bleibt er unverändert · Test: Unit
- **AC-M4-06-4** (F6, P19 a) – *Gegeben* ein Snapshot mit `/home/alt/Projekte/a.txt`, Benutzer `neu`, `/home/neu/Projekte` existiert nicht, *wenn* «Originalort» gewählt wird, *dann* verlangt Kestic einen Zielordner, legt `/home/neu/Projekte` nicht an und startet keinen Restore, bis ein Ziel gewählt ist · Test: Unit, Integration (Temp-Verzeichnis)

### M4-S07 Fremde Snapshots
- **AC-M4-07-1** (FR-36) – *Gegeben* ein per restic-CLI ohne Kestic-Tag angelegter Snapshot, *wenn* das Ziel öffnet, *dann* erscheint er unter «nicht zugeordnet» und in keiner Backup-Liste · Test: Integration
- **AC-M4-07-2** (FR-36, P38) – *Gegeben* ein fremder Snapshot, *wenn* er durchsucht und eine Datei daraus wiederhergestellt wird, *dann* gelten AC-M4-03-1 und AC-M4-05-1 bis -05-7, und der Lauf der Art `Restore` gehört zum Ziel ohne Backup-Bezug · Test: Integration
- **AC-M4-07-3** (FR-36, E10) – *Gegeben* Metadaten-Snapshots `kestic-meta:b1` und Snapshots mit `kestic:b1`, *wenn* «nicht zugeordnet» lädt, *dann* erscheint keiner davon dort · Test: Integration
- **AC-M4-07-4** (FR-36, E10, Glossar «verwaist») – *Gegeben* ein Snapshot mit `kestic:x9`, `x9` weder in der Config noch in einem Metadaten-Snapshot (`kestic-meta:*`) des Ziels, *wenn* das Ziel öffnet, *dann* erscheint er als «verwaist», nicht unter «nicht zugeordnet» · Test: Integration
- **AC-M4-07-5** (Glossar «verwaist», P20) – *Gegeben* ein Snapshot mit `kestic:x9` und ein Metadaten-Snapshot `kestic-meta:x9`, `x9` nicht in der Config, *wenn* das Ziel öffnet, *dann* gilt er **nicht** als «verwaist» (Backup übernehmbar, FR-49); verwaiste Snapshots sind wie fremde durchsuch- und wiederherstellbar · Test: Integration

## Exit-Kriterien

- Datei und Ordner aus einem Snapshot an neuen Ort und an Originalort (mit Vorschau) wiederhergestellt.
- `just verify` grün; Screenshots Snapshot-Liste, Diff, Browser, Restore-Dialog.
- Manuelle Abnahme: Restore aus einem echten Netzwerk-Ziel (z. B. NAS).
