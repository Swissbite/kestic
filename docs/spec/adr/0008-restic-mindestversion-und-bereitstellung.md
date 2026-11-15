# ADR 0008: restic-Mindestversion 0.18.0 und Bereitstellung

- Status: angenommen
- Datum: 2026-11-15

## Kontext

Ab 0.18.0 kommen fatale Fehler als JSON, `check --json`; Exit-Codes 10–12 ab 0.17.x. Ubuntu 24.04 und Derivate liefern 0.16.4 ([restic-integration.md](../restic-integration.md)).

## Entscheid

Mindestversion 0.18.0 (NFR-05). System-restic bevorzugt (im `.deb` als `Recommends`), sonst lädt Kestic das offizielle Binary geprüft herunter (FR-33); Umstellung zurück auf System-restic jederzeit.

## Konsequenzen

Download mit `SHA256SUMS` + PGP-Signatur und fest eingebautem Fingerprint; Release-Prüfung höchstens wöchentlich, abschaltbar (FR-51).
