# ADR 0007: Wiederherstellungs-Kit als eigener restic-Key

- Status: angenommen
- Datum: 2026-11-15

## Kontext

Ein Repo kann mehrere Keys haben, alle schützen denselben Master-Key, der sich nie ändert ([restic-integration.md](../restic-integration.md)). Ohne gesichertes Passwort ist ein Backup nach Geräteverlust wertlos.

## Entscheid

Beim Anlegen oder Verbinden eines Ziels erzeugt Kestic einen zusätzlichen Recovery-Key (`key add`), unabhängig vom Alltags-Passwort und widerrufbar; die Passphrase wird nicht gespeichert (FR-35).

## Konsequenzen

Passwortwechsel lässt das Kit gültig; «Kit erneuern» = neuer Key, alter entfernt. Nach Umzug in ein Repo mit anderen Keys ist ein neues Kit nötig (P42).
