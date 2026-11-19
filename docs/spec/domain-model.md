# Domänenmodell

Begriffe: [glossary.md](glossary.md).

## Modell

```
Ziel 1 ──── n Backup
Ziel 1 ──── n Lauf      (alle Arten: Backup, Maintenance, Restore, Meta)
Backup 1 ──── n Lauf    (Backup-Läufe; Restore optional mit Backup-Bezug)
```

- **Ziel** = ein restic-Repository. Enthält: Backend-Typ + Adresse, Secret-Referenzen (KWallet), Key-ID des Wiederherstellungs-Kits, Wartung: `prune`-Zeitplan, `check`-Zeitplan. `forget`, `prune` und `check` brauchen einen exklusiven Lock.
- **Backup** = Paar Quelle → Ziel. Enthält: Name, Quellpfad, Quellentyp (Ordner | Laufwerk, automatisch erkannt), Ausschluss-Regeln, Zeitplan, Laufbedingungen, Aufbewahrung, interne ID (Tag `kestic:<backup-id>`).
- **IDs:** `backup-id` und `ziel-id` sind kurze zufällige IDs (8 Zeichen Base32, z. B. `k7m2q9xa`), stabil, im Tag `kestic:<backup-id>` (F15).
- **Lauf** = eine Ausführung, hat eine Art (Glossar «Lauf-Arten»): Backup-Lauf (`backup` → `forget`) gehört zu einem Backup; Restore gehört zu einem Ziel, mit Bezug zu einem Backup, wenn der Snapshot einem zugeordnet ist (fremde und verwaiste Snapshots haben keinen); Wartungslauf (offene `forget`, `prune`, `check`) und Metadaten-Lauf gehören zu einem Ziel. Enthält: Zeit, Dauer, ausgeführte Befehle (FR-34), Statistik aus `--json`, Log, Status:
  - `ausstehend` (fällig, kann noch nicht starten) → `läuft` → `ok` | `Warnung` (z. B. Exit 3) | `Fehler` | `abgebrochen` (Benutzer) | `unterbrochen` (Logout, Shutdown, Suspend, Prozessende).
  - Grund bei `ausstehend` wird angezeigt: Laufbedingung nicht erfüllt (FR-13), Quelle nicht verfügbar (FR-29), Ziel nicht erreichbar (FR-30), Warteschlange, Backoff nach Fehler (F2). Alert erst bei «überfällig».
  - Findet der Daemon beim Start einen Lauf im Status `läuft`, setzt er ihn auf `unterbrochen` (ein beim Logout beendeter Prozess kann das nicht mehr selbst schreiben).
  - Bei `abgebrochen`/`unterbrochen`: SIGTERM an restic, Lock-Status prüfen, ggf. FR-19 anbieten. Nächster Lauf setzt dank Deduplizierung effizient fort.
- **UI:** Liste von Backups. Neues Backup = Quelle wählen + Ziel **aus bestehenden wählen** oder **neu anlegen** (Repo verbinden oder `init`, FR-15).
- **Warteschlange pro Ziel (P38):** Backup- und Metadaten-Läufe in der Warteschlange des Ziels; Backups auf dasselbe Ziel dürfen parallel laufen (nicht-exklusiver Lock, [restic-integration.md](restic-integration.md)); Wartung exklusiv: `forget`/`prune`/`check` laufen erst, wenn kein Backup auf dem Ziel läuft. Restore nimmt nur einen nicht-exklusiven Lock und darf parallel zu Backups laufen, nicht während Wartung. MVP: Backups pro Ziel nacheinander (einfacher, kein Nachteil bei einem Laptop).
- **Ein Repo, mehrere Rechner mit Kestic (E13):** MVP unterstützt, solange jeder Rechner eigene Backups (eigene IDs) hat. Exit 11 (Lock) ist ein vorübergehender Fehler wie Netzwerkfehler (F2). Gemeinsames Bearbeiten derselben Backup-Definition von zwei Rechnern: ausser Scope.

## Persistenz und Konfiguration

- **Config** `~/.config/kestic/config.toml` (TOML; KDE-Standard wäre KConfig/INI, für Rust ist TOML + `serde` naheliegend, KConfig-Bindings sind nicht belegt) mit `config_version` + Migration (F13); keine Secrets in der Datei. Config mit höherer `config_version` als bekannt: ablehnen mit Meldung, Datei nicht überschreiben (P1). Ältere Config wird beim Laden im Speicher migriert und erst beim nächsten Speichern mit aktueller Version geschrieben. Versionen: `1` = M1 (Aufbewahrung noch nicht einstellbar), `2` = ab M2 (mindestens eine aktive Aufbewahrungsregel), `3` = ab 2026-11-20 (Netzwerk-Laufbedingungen nur bei Netzwerk-Zielen). Migration 1 → 2: leere Aufbewahrung wird zu `keep_last = unlimited` (alle Snapshots behalten, wie in M1), vorhandene Regeln bleiben unverändert. Migration 2 → 3: bei Backups auf einem lokalen Ordner oder Laufwerk entfallen «nur mit Netzwerk», «nicht bei getakteter Verbindung» und erlaubte Netzwerke.
- **Zustand, Historie, Logs** unter `~/.local/state/kestic/`: SQLite für die Historie, Logs als Dateien pro Lauf, gelöscht, wenn älter als 90 Tage **und** nicht unter den letzten 100 Läufen des Backups (F13). Dort auch der Vermerk «forget offen» pro Backup (P11) und pro Ziel die zuletzt gelesene Snapshot-Liste (IDs, `original`, Tags) für den Repo-Abgleich (P42), aktualisiert nach jedem erfolgreichen Backup-, Wartungs- und Metadaten-Lauf.
- **Generierte Exclude-Dateien** `~/.local/state/kestic/excludes/<backup-id>.txt` (stabiler Pfad für FR-34 und FR-24) (F13).
