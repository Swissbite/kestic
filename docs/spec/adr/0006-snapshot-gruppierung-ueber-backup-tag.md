# ADR 0006: Snapshot-Gruppierung über den Backup-Tag

- Status: angenommen
- Datum: 2026-11-15

## Kontext

restic gruppiert per Default nach `host,paths`; ein Hostname-Wechsel führt zu vollständigem Neueinlesen und getrennten Aufbewahrungsgruppen ([restic-integration.md](../restic-integration.md)).

## Entscheid

Jedes Backup trägt den unveränderlichen Tag `kestic:<backup-id>`; `backup` und `forget` laufen mit `--group-by tags` (FR-28). Metadaten-Snapshots tragen `kestic-meta:<backup-id>` (FR-49).

## Konsequenzen

Der Hostname ist für die Logik irrelevant. Fremde Snapshots ohne Kestic-Tag erscheinen als «nicht zugeordnet» (FR-36).
