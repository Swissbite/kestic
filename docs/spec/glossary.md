# Glossar

Verbindliche Begriffe für UI, Spec und Code. Nicht verwenden: «Profil», «Job», «Ignore», «Advanced-Modus».

| UI (DE) | UI (EN) | Code | restic |
| --- | --- | --- | --- |
| Ziel | Destination | `Destination` | Repository |
| Backup | Backup | `Backup` | Snapshot-Reihe, erkannt am Tag `kestic:<backup-id>` |
| Quelle | Source | `Source` | Backup-Pfad |
| Ausschluss-Regeln | Exclude rules | `ExcludeRules` | `--exclude`, `--iexclude`, `--exclude-file`, `--exclude-caches`, `--exclude-if-present`, `--exclude-larger-than` |
| Zeitplan | Schedule | `Schedule` | – |
| Laufbedingungen | Run conditions | `RunConditions` | – |
| Aufbewahrung | Retention | `RetentionPolicy` | `forget --keep-*` |
| Wartung | Maintenance | `Maintenance` | `prune`, `check` |
| Lauf | Run | `Run` | ein restic-Aufruf bzw. eine Folge (backup → forget) |
| Snapshot | Snapshot | `Snapshot` | Snapshot |
| Wiederherstellungs-Kit | Recovery kit | `RecoveryKit` | zusätzlicher Key (`key add`) |
| Hintergrunddienst | Background service | `daemon` | – |
| Übersicht | Overview | – (Screen) | – |
| Verlauf | History | `RunHistory` | – (Lauf-Historie, FR-18) |
| Aufräumen | Clean up | `Maintenance::Prune` | `prune` |
| Prüfen | Verify | `Maintenance::Check` | `check` |
| Nicht zugeordnet | Unassigned | – | Snapshots ohne Kestic-Tag (FR-36) |
| Wartet | Waiting | Status `ausstehend` | – (UI-Wort für `ausstehend`) |
| Ausnahme | Exception | `ExcludeRule::Negated` | `!`-Negation in Ausschluss-Regeln |
| Entfernen | Remove | – | nur aus der Kestic-Konfiguration nehmen; Daten im Ziel bleiben (FR-56) |
| Löschen | Delete | – | Daten im Ziel vernichten (Snapshots, FR-48 «Gefahrenbereich») |
| Gefahrenbereich | Danger Zone | – | abgesetzter Bereich für Löschen (FR-48) |
| Sperre | Lock | – | restic-Lock (Exit 11, FR-19) |
| Fortschrittsanzeige | Progress view | – | Eintrag im Plasma-Benachrichtigungsbereich (FR-53) |
| Passwortspeicher | Password store | `SecretStore` | Secret Service / KWallet |
| Systemabschnitt | System Tray | – | Plasma-Bereich für Status-Symbole (StatusNotifierItem), in dem der Kestic-Tray-Eintrag erscheint |
| Expertenmodus | Expert mode | `expert_mode` | – (ein einziger Schalter; Darstellungsart: volle Ausschluss-Syntax und Expertenoptionen in-place; E16, K2) |
| Metadaten-Snapshot | Metadata snapshot | `MetaSnapshot` | Snapshot mit Tag `kestic-meta:<backup-id>` (E10, FR-49) |
| überfällig | overdue | `is_overdue` | – (zwei Termine des Zeitplans ohne erfolgreichen Lauf verpasst; Anzahl einstellbar) |
| verwaist | orphaned | – | Snapshots mit Tag `kestic:<id>`, deren Backup-ID weder in der Config noch in einem Metadaten-Snapshot des Ziels vorkommt (eine Definition für FR-49, M4, M6) |
| Lauf-Arten | Run kinds | `RunKind` (einziger Bezeichner im Code; UI-Wörter: Backup, Wartung, Wiederherstellen, Metadaten) | `Backup` (backup → forget), `Maintenance` (offene forget → prune → check), `Restore`, `Meta` (Metadaten-Snapshot); Regeln [domain-model.md](domain-model.md) (P38) |
| Wartungslauf | Maintenance run | `Run` (Art `Maintenance`) | Lauf auf einem Ziel: offene `forget` → `prune` → `check` |
| restic-Quelle | restic source | `ResticProvider` | – (System / App-verwaltet / eigener Pfad) |
