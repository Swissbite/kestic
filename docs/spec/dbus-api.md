# D-Bus-API Daemon ↔ GUI

Gerüst (F14, U17, U18). Details, Signaturen und Fehlerfälle werden in M1-S09 festgelegt; jede Erweiterung wird hier nachgeführt.

## Grundsätze

- **Nur der Daemon schreibt** Config und Zustand; die GUI ändert ausschliesslich über D-Bus-Aufrufe, `core` validiert (F14).
- **Bus-Name als Single-Instance-Lock:** Autostart und GUI starten keinen zweiten Daemon; ein zweiter Start erkennt den belegten Namen und beendet sich (F14).
- **Interface versioniert:** `<app-id>.Kestic1` (Name nach der App-ID, Reverse-DNS, siehe [architecture.md](architecture.md)); inkompatible Änderungen nur über ein neues Interface (`…Kestic2`).
- **Daemon-Start:** Die GUI startet den Daemon bei Bedarf (ohne `--autostart`) und zeigt an, wenn er nicht läuft ([architecture.md](architecture.md), Abschnitt «Komponenten und Ausführung»).

## Methoden (bekannt)

| Methode | Zweck | Quelle |
| --- | --- | --- |
| Ziele und Backups verwalten (anlegen, lesen, entfernen) | Konfiguration ändern | F14, FR-56 |
| `UpdateBackup` | Backup-Änderungen explizit speichern (`[Speichern]`); Zwischenstand lebt nur in der GUI | U18, FR-56 |
| `UpdateDestination` | Ziel-/Verbindungsänderungen speichern, erst nach Verbindungstest und Repo-ID-Abgleich | U18, FR-56, P42 |
| Lauf starten | «Jetzt sichern» bzw. «Alle jetzt sichern» | FR-13b |
| Lauf abbrechen | laufenden Lauf abbrechen (SIGTERM an restic) | FR-21 |
| `WindowActive(bool, page)` | GUI meldet, ob ihr Fenster aktiv ist und welche Seite; Hinweise im Fenster statt doppelt | U17 |

## Signale (bekannt)

| Signal | Inhalt | Quelle |
| --- | --- | --- |
| Status-Signale | Gesamtstatus, Lauf-Status und Fortschritt je Lauf | F14, FR-12 |
| `ManualRunBlocked(backup_id, reason)` | manueller Start scheitert am Vorab-Check, weil sich der Zustand zwischen Anzeige und Klick geändert hat; `reason` = `source_unavailable` \| `destination_unreachable`; spätestens nach 15 s, kein restic-Prozess. Die offene GUI zeigt eine Meldung im Fenster; ist kein Fenster offen, sendet der Daemon eine Desktop-Benachrichtigung | FR-30 |

## In M1-S09 festzulegen

- Genauer Bus-Name, Objektpfade, Signaturen und Fehler-Namen.
- Abfrage der Daemon-Version durch die GUI (Paket-Update, E11).
- Verfügbarkeitsstatus für Quelle und Ziel (FR-30, M1-S23).
