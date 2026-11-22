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
| M4-S01 | Snapshot-Liste pro Backup (Zeit, Grösse, Host), Metadaten-Snapshots ausgeblendet; Tab «Wiederherstellen» (FR-06) |
| M4-S02 | Unterschiede zwischen zwei Snapshots (`diff --json`) (FR-07) |
| M4-S03 | Browsing: Ordnerliste mit Breadcrumb eines Snapshots via `ls --json`, lazy geladen (FR-10) |
| M4-S04 | Suche über Snapshots (`find`) (FR-10) |
| M4-S05 | Wiederherstellen: Default neuer Ordner; Originalort oder nicht leerer Ordner nur mit Wahl `never` (Default) / `if-newer` / `if-changed` / `always` und Pflicht-Vorschau `--dry-run`, vom Daemon erzwungen (F5, NFR-08) |
| M4-S06 | Pfad-Abbildung bei anderem Benutzernamen auf den Heimordner des Benutzers (F6) |
| M4-S07 | Fremde Snapshots (ohne Kestic-Tag) als «nicht zugeordnet» anzeigen, durchsuchen, wiederherstellen (FR-36) |

## Akzeptanzkriterien

Test-Isolation gemäss [test-strategy.md](../test-strategy.md); nur Temp-Repos. «Integration» = gegen echtes restic mit Temp-Repo.

### M4-S00 Strukturelle Vorbereitung
- **AC-M4-00-1** (NFR-12) – *Gegeben* die bisher gewachsenen Quelldateien `daemon/src/service.rs`, `gui/src/backend.rs`, `gui/src/models.rs`, `core/src/model.rs` und `gui/src/qml/Main.qml`, *wenn* gemäss [architecture.md](../architecture.md) §Code-Struktur aufgeteilt wird, *dann* (a) liegen alle Module, Modelle und Dialoge an den vorgegebenen Orten, (b) überschreitet keine `.rs`- oder `.qml`-Datei unter `src/` 1500 Zeilen und keine Funktion 150 Zeilen, und (c) bleiben die öffentliche API von `core` und das D-Bus-Interface unverändert · Test: Unit + Clippy/Grössenprüfung
- **AC-M4-00-2** (NFR-12) – *Gegeben* die aufgeteilte Codebasis, *wenn* die gesamte Suite läuft, *dann* (a) bleibt das Verhalten unverändert, (b) ändern sich bestehende Tests höchstens bei den `use`-Pfaden, und (c) ist `just verify` vollständig grün · Test: Integration + GUI-Smoke (`just verify`)

### M4-S01 Snapshot-Liste
- **AC-M4-01-1** (FR-06) – *Gegeben* drei Snapshots von `b1` (Tag `kestic:b1`), *wenn* die Liste von `b1` lädt, *dann* (a) zeigt sie genau diese drei Snapshots, (b) sind sie neueste zuerst sortiert, und (c) trägt jeder Eintrag Zeitpunkt, Grösse und Host, und (d) ist die Grösse die neu geschriebene Datenmenge (`data_added`, «312 MB neu»); fehlt `data_added` bei vorhandener Zusammenfassung, gilt `total_bytes_processed` · Test: Integration
- **AC-M4-01-2** (FR-06, E10) – *Gegeben* zusätzlich zwei Metadaten-Snapshots `kestic-meta:b1`, *wenn* die Liste von `b1` lädt, *dann* (a) erscheinen sie nicht in der Liste, und (b) fliessen sie nicht in die Anzahl ein · Test: Integration
- **AC-M4-01-3** (FR-06) – *Gegeben* Snapshots von `b1` und `b2` im selben Repo, *wenn* die Liste von `b1` lädt, *dann* (a) enthält sie keinen Snapshot mit `kestic:b2` · Test: Integration
- **AC-M4-01-4** (FR-06, NFR-09) – *Gegeben* die offene Liste, *wenn* nur die Tastatur benutzt wird, *dann* (a) lassen sich alle Snapshots wählen, und (b) lassen sich Unterschiede, Durchsuchen und Wiederherstellen auslösen · Test: GUI-Smoke (offscreen + Screenshot), manuell
- **AC-M4-01-5** (FR-06, P17) – *Gegeben* ein Snapshot-JSON ohne Zusammenfassung (Fixture, wie ältere restic-Versionen bzw. fremde Snapshots), *wenn* die Liste lädt, *dann* (a) zeigt die Grösse «unbekannt», (b) erfolgt kein `stats`-Aufruf, und (c) tritt kein Fehler auf · Test: Unit (Fixture)
- **AC-M4-01-6** (K3, FR-10) – *Gegeben* ein Build mit M4, *wenn* das Hauptfenster startet, *dann* (a) zeigt die Tab-Leiste «Wiederherstellen» zwischen «Übersicht» und «Backups», (b) öffnet der Bereich die Snapshot-Auswahl, und (c) fehlt er in einem Build nur mit M1 · Test: GUI-Smoke (offscreen + Screenshot)

### M4-S02 Unterschiede zwischen Snapshots
- **AC-M4-02-1** (FR-07) – *Gegeben* Snapshot A mit `docs/a.txt` und B, in dem `a.txt` geändert, `b.txt` neu, `c.txt` entfernt ist, *wenn* A → B angezeigt wird, *dann* (a) erscheinen `a.txt` als geändert, `b.txt` als neu und `c.txt` als entfernt, und (b) ist das Ergebnis deckungsgleich mit `restic diff --json A B` · Test: Integration
- **AC-M4-02-2** (FR-07, NFR-10) – *Gegeben* `diff`-JSON mit unbekanntem Feld bzw. Message-Typ, *wenn* geparst wird, *dann* (a) entsteht die Anzeige ohne Fehler, und (b) wird Unbekanntes ignoriert · Test: Unit
- **AC-M4-02-3** (FR-07) – *Gegeben* zwei identische Snapshots, *wenn* Unterschiede angezeigt werden, *dann* (a) erscheint «keine Unterschiede» statt einer leeren Liste · Test: Integration

### M4-S03 Browsing
- **AC-M4-03-1** (FR-10) – *Gegeben* ein Snapshot von `/tmp/…/src` mit Unterordnern, *wenn* er durchsucht wird, *dann* (a) zeigt die Ordnerliste mit Breadcrumb Ordner und Dateien mit Name, Grösse und Änderungszeit gemäss `restic ls --json`, und (b) wird `restic mount` nicht verwendet · Test: Integration
- **AC-M4-03-2** (FR-10) – *Gegeben* ein Snapshot mit 100 000 Dateien in 1 000 Ordnern, *wenn* der Wurzelordner öffnet, *dann* (a) ruft Kestic `restic ls` mit der Snapshot-ID und genau einem Ordner-Argument ohne Rekursion auf (nicht den ganzen Baum vorab), (b) entspricht die Zahl der geladenen Einträge dem Inhalt dieser einen Ebene (nicht 100 000), und (c) reagiert die Ansicht weiter auf Eingaben · Test: Integration, GUI-Smoke
- **AC-M4-03-3** (FR-10, NFR-09) – *Gegeben* die offene Snapshot-Ansicht, *wenn* nur die Tastatur benutzt wird, *dann* (a) geht es per Ordnerliste in Unterordner, (b) geht es per Breadcrumb in jede übergeordnete Ebene zurück (keine Baumansicht zum Auf- und Zuklappen), und (c) lassen sich Einträge zum Wiederherstellen markieren · Test: GUI-Smoke, manuell

### M4-S04 Suche über Snapshots
- **AC-M4-04-1** (FR-10) – *Gegeben* `bericht.pdf` in zwei von drei Snapshots von `b1` unter verschiedenen Pfaden, *wenn* danach gesucht wird, *dann* (a) erscheinen genau zwei Treffer mit Snapshot und Pfad, und (b) öffnet sich ein Treffer direkt in der Ordnerliste mit Breadcrumb · Test: Integration
- **AC-M4-04-2** (FR-10) – *Gegeben* ein Suchbegriff ohne Treffer, *wenn* gesucht wird, *dann* (a) erscheint «keine Treffer», und (b) tritt kein Fehler auf · Test: Integration
- **AC-M4-04-3** (FR-10, E10) – *Gegeben* Metadaten-Snapshots `kestic-meta:b1`, *wenn* gesucht wird, *dann* (a) liefern sie keine Treffer · Test: Integration
- **AC-M4-04-4** (FR-10, P18) – *Gegeben* `bericht.pdf` in Snapshots von `b1` und `b2` im selben Ziel, *wenn* in `b1` gesucht wird, *dann* (a) erscheinen nur Treffer aus Snapshots mit `kestic:b1` · Test: Integration

### M4-S05 Wiederherstellen
- **AC-M4-05-1** (F5) – *Gegeben* ein Snapshot mit `docs/a.txt`, *wenn* der Restore-Dialog öffnet, *dann* (a) ist ein **neuer Ordner** als Ziel vorausgewählt, und (b) nicht der Originalort · Test: GUI-Smoke
- **AC-M4-05-2** (F5) – *Gegeben* Restore in einen neuen, leeren Ordner, *wenn* wiederhergestellt wird, *dann* (a) liegt `a.txt` mit identischem Inhalt dort, und (b) endet der Lauf mit `ok` · Test: Integration
- **AC-M4-05-3** (F5, NFR-08) – *Gegeben* Restore an den Originalort mit abweichender `a.txt`, *wenn* der Modus nicht geändert wird, *dann* (a) gilt `never`, (b) bleibt `a.txt` unverändert, und (c) nennt die Vorschau sie «nicht überschrieben» · Test: Integration
- **AC-M4-05-4** (F5, NFR-08) – *Gegeben* Restore an den Originalort oder in einen nicht leeren Ordner, *wenn* «wiederherstellen» gewählt wird, *dann* (a) läuft zuerst `restore --dry-run`, (b) zählt und listet die Vorschau je Datei die vier Kategorien «neu / überschrieben / nicht überschrieben / unverändert», (c) wird erst nach Bestätigung mit klarer Folge («n Dateien werden überschrieben, nicht rückgängig») geschrieben, und (d) trennt Kestic «nicht überschrieben» von «unverändert» mit einem zweiten Dry-Run mit `--overwrite always` (nur wenn der Modus nicht `always` ist) · Test: Integration, GUI-Smoke
- **AC-M4-05-5** (F5) – *Gegeben* Originalort mit älterer bzw. neuerer bzw. geänderter `a.txt`, *wenn* mit `if-newer`, `if-changed` bzw. `always` wiederhergestellt wird, *dann* (a) entspricht das Ergebnis jeweils `restic restore --overwrite <modus>` · Test: Integration
- **AC-M4-05-6** (F5, NFR-08) – *Gegeben* ein beliebiger Restore-Aufruf durch Kestic, *wenn* der restic-Befehl geprüft wird, *dann* (a) enthält er immer explizit `--overwrite`, und (b) nutzt er nie den restic-Default `always` implizit, und (c) enthält er weder `--sparse` noch `--verify` · Test: Unit
- **AC-M4-05-7** (F5, NFR-08) – *Gegeben* der Benutzer bricht die Bestätigung ab, *wenn* der Dialog schliesst, *dann* (a) ist am Zielort keine Datei verändert · Test: Integration
- **AC-M4-05-8** (FR-10) – *Gegeben* in der Ordnerliste ist nur `docs/` markiert, *wenn* wiederhergestellt wird, *dann* (a) werden nur Dateien unter `docs/` geschrieben · Test: Integration
- **AC-M4-05-9** (P38) – *Gegeben* ein laufender Backup-Lauf auf dem Ziel, *wenn* ein Restore startet, *dann* (a) läuft er sofort parallel (nicht-exklusiver Lock), und (b) hat er die Art `Restore` · Test: Integration
- **AC-M4-05-10** (P38, ab M3) – *Gegeben* ein laufender Wartungslauf (`prune`) auf dem Ziel, *wenn* ein Restore startet, *dann* (a) steht er auf `ausstehend` (Grund «Warteschlange»), und (b) startet er erst nach der Wartung. Prüfbar erst mit Wartungslauf und Warteschlange aus M3; M4 selbst hängt nur von M1 ab · Test: Integration
- **AC-M4-05-11** (P19 b) – *Gegeben* ein Restore an den Originalort mit `always`, *wenn* danach ein neuer Restore-Dialog öffnet, *dann* (a) ist wieder `never` vorausgewählt · Test: GUI-Smoke
- **AC-M4-05-12** (P38) – *Gegeben* ein Restore aus einem Snapshot mit `kestic:b1`, *wenn* der Lauf angelegt wird, *dann* (a) hat er die Art `Restore`, (b) gehört er zum Ziel, (c) trägt er den Backup-Bezug `b1`, und (d) erscheint er im Verlauf von `b1` und des Ziels · Test: Unit
- **AC-M4-05-13** (F5) – *Gegeben* ein Snapshot von Backup «Persönlicher Ordner» vom 2026-11-15 12:00, *wenn* der Restore-Dialog den neuen Ordner vorschlägt, *dann* (a) heisst er `~/Wiederhergestellt/Persönlicher Ordner 2026-11-15 12-00` (Snapshot-Datum und -Uhrzeit, ohne `:`), (b) erhält er bei einem bereits vorhandenen Ordner gleichen Namens das Suffix « (2)», (c) heisst er bei einem Snapshot ohne Backup-Bezug `<Ziel-Name> nicht zugeordnet <Kurz-ID>`, (d) sind `/`, Steuerzeichen und ein führender Punkt aus dem Namen entfernt, und (e) legt der Daemon `~/Wiederhergestellt` erst mit «Jetzt wiederherstellen» an · Test: Unit, Integration (Temp-Home)
- **AC-M4-05-14** (F5, NFR-08) – *Gegeben* ein Restore an den Originalort oder in einen vorhandenen, nicht leeren Ordner, *wenn* `StartRestore` ohne `preview_id`, mit der `preview_id` einer Vorschau mit anderen Parametern (Snapshot, Auswahl, Ziel oder Modus), mit der `preview_id` einer nicht fertigen Vorschau oder mit der einer Vorschau aufgerufen wird, die nach `Clock` älter als 10 Minuten ist, *dann* (a) antwortet der Daemon jeweils mit `PreviewRequired`, (b) startet kein restic-Prozess, und (c) entsteht kein Lauf. Mit passender, fertiger, frischer Vorschau (d) startet der Lauf · Test: Integration
- **AC-M4-05-15** (F5, NFR-08) – *Gegeben* der Standardfall (neuer, leerer Ordner) bzw. ein vorhandener, nicht leerer Ordner als Ziel, *wenn* der Restore-Dialog bzw. die Vorschau läuft, *dann* (a) liefert die Vorschau beim Standardziel nur Vorschlag und Prüfung, ohne `restore --dry-run` und mit leeren Zählern, und `StartRestore` braucht dort keine `preview_id`, (b) ist bei einem nicht leeren Ordner `never` vorausgewählt und die Vorschau Pflicht, (c) bleibt «wiederherstellen» in der GUI bis zur fertigen Vorschau gesperrt, und (d) meldet die Prüfung bei einer Datei als Ziel `not_a_directory`, bei einem nicht beschreibbaren Ordner `not_writable` und bei einem Ziel im Repository-Pfad eines lokalen Ziels `inside_repository` (je als Status, nicht als Fehler) · Test: Integration, GUI-Smoke
- **AC-M4-05-16** (F5, NFR-08) – *Gegeben* ein Snapshot mit Dateien einer fremden uid bzw. mit Hardlinks, *wenn* die Vorschau läuft, *dann* (a) enthält sie bei abweichender uid des Snapshots die Notiz `ownership_not_restored` («Besitzer und Gruppen werden nicht übernommen, die Dateien gehören Ihnen»), ohne dass der Lauf von `ok` abweicht, (b) enthält sie diese Notiz bei gleicher uid nicht, (c) enthält sie beim Überschreiben einer Datei mit mehreren Hardlinks (`ls`: mehr als ein Link) die Notiz `hard_links_affected`, (d) enthält sie diese Notiz ohne Überschreiben nicht, und (e) wird ein Symlink als Symlink wiederhergestellt, nicht aufgelöst · Test: Integration
- **AC-M4-05-17** (F5) – *Gegeben* Pfade aus der GUI (Auswahl, Ordner, Ziel), *wenn* ein Restore oder eine Abfrage angefordert wird, *dann* (a) weist der Daemon einen Pfad mit `..`-Segment, einen relativen, einen leeren und einen mit NUL jeweils mit `InvalidInput` ab, ohne restic aufzurufen, (b) läuft jeder Restore als `<snapshot>:<gemeinsamer Vorfahre der Auswahl>` mit dem (abgebildeten) Vorfahren als `--target`, (c) ist das Ziel nie `/`, auch bei einem Snapshot mit Wurzel `/` (JSON-Fixture), und (d) wird ein Ordner, der im Snapshot fehlt, als Fehler `path_not_in_snapshot` gemeldet, nicht als Erfolg mit 0 Dateien · Test: Unit, Integration
- **AC-M4-05-18** (FR-10) – *Gegeben* Dateien `[x].txt`, `*star?.txt`, `back\slash`, `-dash`, eine Datei mit Zeilenumbruch im Namen, eine mit Leerzeichen und eine mit Umlaut, *wenn* jede einzeln gewählt und wiederhergestellt wird, *dann* (a) kommt genau diese eine Datei an, (b) steht ein einzelner gewählter Ordner als `<snapshot>:<ordner>` (wörtlich) im Aufruf, (c) stehen mehrere Einträge als `--include=<Muster>` mit Maskierung von `\ * ? [` im Aufruf, und (d) laufen Namen mit Zeilenumbruch über `--include-file` im Staging-Verzeichnis des Daemons (Rechte 0600, nach dem Lauf gelöscht) · Test: Unit (`escape_glob`), Integration (beide restic-Versionen)
- **AC-M4-05-19** (FR-10) – *Gegeben* ein Restore, dessen Auswahl nichts trifft (restic: Exit 0 ohne Zahlen), *wenn* der Lauf endet, *dann* (a) endet er mit `Warnung` («Nichts wiederhergestellt»), und (b) nie mit `ok` · Test: Integration
- **AC-M4-05-20** (P38, ab M3) – *Gegeben* ein laufender Restore auf dem Ziel, *wenn* ein Wartungslauf startet, *dann* (a) steht er auf `ausstehend` (Grund «Warteschlange»), (b) startet er erst nach dem Restore, und (c) endet kein restic-Aufruf mit Exit 11. Die Buchführung für Restores ist eigenständig und nutzt nicht den Ein-Lauf-Anspruch pro Ziel der Backups · Test: Integration
- **AC-M4-05-21** (F5) – *Gegeben* ein Zielordner, der ein Symlink auf einen anderen Ordner ist, *wenn* die Vorschau läuft, *dann* (a) ist er als Ziel zulässig, und (b) nennt die Vorschau den aufgelösten Zielpfad · Test: Integration

### M4-S06 Pfad-Abbildung bei anderem Benutzernamen
- **AC-M4-06-1** (F6) – *Gegeben* ein Snapshot mit `/home/alt/Dokumente/a.txt`, aktueller Benutzer `neu`, *wenn* «Originalort» gewählt wird, *dann* (a) schlägt Kestic `Dokumente/a.txt` unter dem Heimordner des Benutzers vor (hier `/home/neu/Dokumente/a.txt`), und (b) zeigt es die Abbildung `alt → neu` · Test: Unit, GUI-Smoke
- **AC-M4-06-2** (F6) – *Gegeben* derselbe Snapshot, *wenn* der Benutzer das vorgeschlagene Ziel ändert, *dann* (a) gilt das gewählte Ziel · Test: Unit
- **AC-M4-06-3** (F6) – *Gegeben* ein Pfad ausserhalb von `/home/<name>/` (z. B. `/media/daten/Fotos`), *wenn* «Originalort» gewählt wird, *dann* (a) bleibt er unverändert · Test: Unit
- **AC-M4-06-4** (F6, P19 a) – *Gegeben* ein Snapshot mit `/home/alt/Projekte/a.txt`, Benutzer `neu`, `/home/neu/Projekte` existiert nicht, *wenn* «Originalort» gewählt wird, *dann* (a) verlangt Kestic einen Zielordner, (b) legt es `/home/neu/Projekte` nie an, (c) startet es keinen Restore, bis ein Ziel gewählt ist, und (d) prüft der Daemon den abgebildeten Ordner beim Vorschlag und erneut direkt vor dem Start, ohne restic-Prozess, wenn er dann fehlt (`missing_mapped_dir`) · Test: Unit, Integration (Temp-Verzeichnis)
- **AC-M4-06-5** (F6) – *Gegeben* ein Snapshot mit `/home/alt/Dokumente/a.txt` und der Heimordner `/var/home/neu` des aktuellen Benutzers, *wenn* «Originalort» gewählt wird, *dann* (a) schlägt Kestic `/var/home/neu/Dokumente/a.txt` vor, nicht `/home/neu/…` · Test: Unit
- **AC-M4-06-6** (F6) – *Gegeben* ein Snapshot mit Pfaden unter `/home/alt/…`, dessen `username` nicht `alt` ist, *wenn* «Originalort» gewählt wird, *dann* (a) macht Kestic keinen stillen Vorschlag, und (b) zeigt es einen Hinweis; der alte Name stammt aus den gewählten Pfaden und wird gegen `snapshot.username` geprüft · Test: Unit
- **AC-M4-06-7** (F6) – *Gegeben* die Pfade `/home/alt/../neu/x`, `/home/alt/..`, `/home/altx/a.txt` und `/home/alt` (ohne Unterpfad), *wenn* abgebildet wird, *dann* (a) bildet Kestic keinen davon ausserhalb des Heimordners des Benutzers ab, und (b) bildet es `/home/altx/a.txt` nicht als Pfad von `alt` ab · Test: Unit

### M4-S07 Fremde Snapshots
- **AC-M4-07-1** (FR-36) – *Gegeben* ein per restic-CLI ohne Kestic-Tag angelegter Snapshot, *wenn* das Ziel öffnet, *dann* (a) erscheint er unter «nicht zugeordnet», und (b) erscheint er in keiner Backup-Liste · Test: Integration
- **AC-M4-07-2** (FR-36, P38) – *Gegeben* ein fremder Snapshot, *wenn* er durchsucht und eine Datei daraus wiederhergestellt wird, *dann* (a) gelten AC-M4-03-1 und AC-M4-05-1 bis -05-7, und (b) gehört der Lauf der Art `Restore` zum Ziel ohne Backup-Bezug · Test: Integration
- **AC-M4-07-3** (FR-36, E10) – *Gegeben* Metadaten-Snapshots `kestic-meta:b1` und Snapshots mit `kestic:b1`, *wenn* «nicht zugeordnet» lädt, *dann* (a) erscheint keiner davon dort · Test: Integration
- **AC-M4-07-4** (FR-36, E10, Glossar «verwaist») – *Gegeben* ein Snapshot mit `kestic:x9`, `x9` weder in der Config noch in einem Metadaten-Snapshot (`kestic-meta:*`) des Ziels, *wenn* das Ziel öffnet, *dann* (a) erscheint er als «verwaist», und (b) nicht unter «nicht zugeordnet» · Test: Integration
- **AC-M4-07-5** (Glossar «verwaist», P20) – *Gegeben* ein Snapshot mit `kestic:x9` und ein Metadaten-Snapshot `kestic-meta:x9`, `x9` nicht in der Config, *wenn* das Ziel öffnet, *dann* (a) gilt er **nicht** als «verwaist» (Backup übernehmbar, FR-49), und (b) sind verwaiste Snapshots wie fremde durchsuch- und wiederherstellbar · Test: Integration
- **AC-M4-07-6** (FR-36) – *Gegeben* ein fremder bzw. verwaister Snapshot, *wenn* er angezeigt, durchsucht, verglichen und wiederhergestellt wird, *dann* (a) ruft Kestic nur `snapshots`, `ls`, `find`, `diff` und `restore` auf, und (b) ruft es nie `tag`, `forget` oder `rewrite` auf ihn auf · Test: Unit (Befehlsprüfung), Integration
- **AC-M4-07-7** (FR-36, E10) – *Gegeben* Snapshots mit `kestic-meta:b1`, mit `kestic:b1` (`b1` in der Config), mit `kestic:x9` (ohne Metadaten-Snapshot, `x9` nicht in der Config), ohne Kestic-Tag und ein Snapshot mit den eigenen Tags `daily` und ohne Kestic-Tag, *wenn* das Ziel öffnet, *dann* (a) ist der erste ausgeblendet, (b) gehört der zweite zu Backup `b1`, (c) ist der dritte «verwaist», (d) ist der vierte «nicht zugeordnet», und (e) ist der fünfte ebenfalls «nicht zugeordnet» (eigene Tags ändern die Einordnung nicht). Die Reihenfolge der Einordnung ist Metadaten-Snapshot, Backup, verwaist, nicht zugeordnet · Test: Unit
- **AC-M4-07-8** (FR-36) – *Gegeben* fremde und verwaiste Snapshots in der Liste, *wenn* das Ziel öffnet, *dann* (a) trägt jeder den Hinweis «Von Kestic nicht verwaltet» · Test: GUI-Smoke (offscreen + Screenshot)

## Exit-Kriterien

- Datei und Ordner aus einem Snapshot an neuen Ort und an Originalort (mit Vorschau) wiederhergestellt.
- `just verify` grün; Screenshots Snapshot-Liste, Diff, Browser, Restore-Dialog.
- Manuelle Abnahme: Restore aus einem echten Netzwerk-Ziel (z. B. NAS).
