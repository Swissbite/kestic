# D-Bus-API Daemon ↔ GUI

Verbindliche Spezifikation für v1 (F14, U17, U18, M1-S09, AC-M1-09-5). Jede Erweiterung in späteren Meilensteinen wird hier nachgeführt.

## Grundsätze

- **Bus-Name (Single-Instance-Lock):** `net.daester.Kestic` (Session-Bus, `DO_NOT_QUEUE`). Ein zweiter Daemon erkennt `AlreadyRunning` und beendet sich sofort mit Exit 0 (F14, AC-M1-09-1).
- **Objektpfad:** `/net/daester/Kestic`
- **Interface (versioniert):** `net.daester.Kestic1` — inkompatible Änderungen nur über ein neues Interface (`net.daester.Kestic2`).
- **Alleiniger Schreiber:** Nur der Daemon schreibt `config.toml`, `history.db`, Excludes und Logs (F14, AC-M1-09-3). Die GUI liest Zustand über `GetState` / Signale und schreibt ausschliesslich über die D-Bus-Methoden. `core` validiert jede Änderung serverseitig; ungültige Eingaben liefern einen D-Bus-Fehler, ohne `config.toml` zu verändern (AC-M1-09-2).
- **Secrets:** Passwörter werden nur für den Moment der Einrichtung/Prüfung/Änderung über den Session-Bus entgegengenommen, sofort über `SecretStore` (KWallet / Secret Service) abgelegt bzw. gelöscht und **nie** in `config.toml`, `GetState` oder Signalen zurückgegeben (NFR-02).
- **Daemon-Start:** Die GUI startet bei Bedarf `kestic daemon` ohne `--autostart` (M1-S21). Autostart nutzt `kestic daemon --autostart` und beendet sich ohne `config.toml` sofort mit Exit 0 (M1-S12).

## Fehler-Namen (`net.daester.Kestic.Error.*`)

| Fehler-Name | Bedeutung |
| --- | --- |
| `net.daester.Kestic.Error.InvalidInput` | Validierung in `core` oder D-Bus-Argument fehlgeschlagen (AC-M1-09-2) |
| `net.daester.Kestic.Error.NotFound` | Ziel-, Backup- oder Lauf-ID unbekannt |
| `net.daester.Kestic.Error.InUse` | Ziel wird noch von Backups referenziert und kann nicht entfernt werden (AC-M1-19-4) |
| `net.daester.Kestic.Error.ManualRunBlocked` | Quelle nicht verfügbar (`source_unavailable`) oder Ziel nicht erreichbar (`destination_unreachable`) vor manuellem Start (FR-30, AC-M1-09-6) |
| `net.daester.Kestic.Error.DestinationUnreachable` | Vorab-Check (`P32`) fehlgeschlagen (Timeout 10 s / ≤ 15 s) |
| `net.daester.Kestic.Error.WrongPassword` | Repository-Passwort abgelehnt (Exit 12, AC-M1-06-3, AC-M1-20-2) |
| `net.daester.Kestic.Error.AuthFailed` | Backend-Anmeldung (z. B. REST 401/403) fehlgeschlagen (AC-M1-06-6) |
| `net.daester.Kestic.Error.SecretStoreLocked` | Passwortspeicher gesperrt oder nicht erreichbar |
| `net.daester.Kestic.Error.ResticUnavailable` | `restic` fehlt, ist `< 0.18.0` oder nicht ausführbar |
| `net.daester.Kestic.Error.AlreadyRunning` | Auf dem betroffenen Ziel bzw. Backup läuft bereits ein Lauf |
| `net.daester.Kestic.Error.Internal` | Unerwarteter I/O-, SQLite- oder Prozessfehler |

## Methoden auf `net.daester.Kestic1`

Strukturierte DTOs werden als UTF-8-JSON übergeben (mit festem Schema in `kestic_daemon::dbus_api`), sodass optionale Felder erweiterbar bleiben, ohne das D-Bus-Interface für jeden Typ zu brechen.

| Methode | Signatur | Zweck | Quelle |
| --- | --- | --- | --- |
| `DaemonVersion` | `() -> s` | Gibt die Daemon-Version (`CARGO_PKG_VERSION`) zurück | E11 |
| `GetState` | `() -> s` (`DaemonStateDto`-JSON) | Vollständiger Zustand: `daemon_version`, `config` (ohne Secrets), `restic_status`, `secret_store_status`, `availability`, `backups` (mit letztem/laufendem Lauf), `destinations`, `recent_runs`, `overall_status` | F14, §6.1, §6.2 |
| `GetRunDetail` | `(run_id: s) -> s` (`RunDetailDto`-JSON) | Einzelner Lauf mit redacted Logzeilen, ausgeführten Befehlen (`CommandRecord`) und transparentem Shell-Aufruf ohne Secrets (`FR-34`) | FR-18, FR-34, M1-S14 |
| `ListBackupSnapshots` | `(backup_id: s) -> s` (`BackupSnapshotsDto`-JSON) | Snapshots (`kestic:<backup_id>`) auf dem aktuellen Ziel sowie Abschnitt `previous_destinations` für früher genutzte, noch eingerichtete Ziele | FR-56, AC-M1-22-4 |
| `TestDestinationConnection` | `(dest_json: s, mode: s, repo_password: s, rest_password: s) -> s` (`ConnectionTestResultDto`-JSON) | Prüft Vorab-Erreichbarkeit (`P32`), `init` (`"init"`) oder `cat --no-lock config` (`"connect"`), **ohne** Config oder Secrets zu speichern (ausser `init` im Assistenten/Test beim Anlegen via `AddDestination`) | FR-01, FR-15, §8.9, §9.1 |
| `AddDestination` | `(dest_json: s, mode: s, repo_password: s, rest_password: s) -> s` (neue `destination_id`) | Prüft Verbindung (`init` / `connect`), legt Secrets bei Erfolg im `SecretStore` ab und speichert `config.toml` | FR-01, FR-15, AC-M1-06-1..6 |
| `UpdateDestination` | `(dest_json: s, mode: s, repo_password: s, rest_password: s, verify_connection: b) -> ()` | Prüft bei geänderten Verbindungsdaten bzw. Modus `"init"` zuerst Eindeutigkeit des Speicherorts, Vorab-Check + Einrichtung/Passwort (`"init"` / `"connect"`); speichert nur bei Erfolg in `config.toml` und `SecretStore` | U18, FR-56, AC-M1-19-6 |
| `UpdateDestinationPassword` | `(dest_id: s, new_password: s) -> ()` | Prüft `new_password` per `cat --no-lock config` (ohne `restic key`) und aktualisiert `SecretStore` nur bei Erfolg | FR-57, P40, AC-M1-20-1..3 |
| `RemoveDestination` | `(dest_id: s) -> ()` | Entfernt unbenutztes Ziel aus `config.toml` und löscht Secrets; verweigert mit `Error.InUse`, solange Backups darauf zeigen | FR-56, P39, AC-M1-19-3..5 |
| `AddBackup` | `(backup_json: s) -> s` (neue `backup_id`) | Validiert Backup + Zielreferenz in `core` und speichert `config.toml` | F14, M1-S13 |
| `UpdateBackup` | `(backup_json: s) -> ()` | Explizites Speichern von Backup-Änderungen (Quelle, Ausschlüsse, Einstellungen); `BackupId` bleibt unverändert | U18, FR-56, AC-M1-19-1 |
| `RenameBackup` | `(backup_id: s, new_name: s) -> ()` | Benennt Backup um (`BackupId` unverändert) und speichert `config.toml` | FR-56, AC-M1-19-1 |
| `ReassignBackup` | `(backup_id: s, new_dest_id: s) -> ()` | Weist Backup einem anderen Ziel zu (`BackupId` bleibt gleich; alte Snapshots & `kestic-meta:<id>` bleiben im alten Ziel) | FR-56, AC-M1-22-1..3 |
| `RemoveBackup` | `(backup_id: s) -> ()` | Entfernt Backup nur aus `config.toml`; Snapshots bleiben im Ziel | FR-56, P39, AC-M1-19-2 |
| `SetResticProvider` | `(provider_json: s) -> s` (`ResticStatusDto`-JSON) | Prüft `ResticProvider` (`System` / `Custom`), speichert bei Erfolg in `config.toml` | FR-33, M1-S04 |
| `SetExpertMode` | `(enabled: b) -> ()` | Speichert `expert_mode` in `config.toml` | E16, K2 |
| `StartBackup` | `(backup_id: s) -> s` (`run_id`) | Führt Vorab-Verfügbarkeitscheck für Quelle & Ziel (`P32`) aus; bei Nichtverfügbarkeit **kein** restic-Prozess, Signal `ManualRunBlocked` + Notification (falls Fenster nicht aktiv) + Fehler `Error.ManualRunBlocked`; sonst asynchroner Backup-Lauf | FR-13b, FR-30, AC-M1-09-6/7 |
| `StartAllBackups` | `() -> s` (`StartAllResultDto`-JSON) | Startet alle verfügbaren Backups und liefert Liste gestarteter sowie übersprungener Backups inkl. Grund | FR-13b, §7.1 |
| `CancelRun` | `(run_id: s) -> ()` | Sendet `SIGTERM` an den laufenden restic-Prozess (`CancelHandle`), setzt Lauf auf `cancelled` | FR-21, AC-M1-10-1/3 |
| `CheckAvailability` | `(force: b) -> s` (`AvailabilityMapDto`-JSON) | Aktualisiert Quellen-/Zielverfügbarkeit; ohne `force` höchstens alle 60 s über `Clock` (`P32`) | FR-30, AC-M1-23-1..4 |
| `WindowActive` | `(active: b, page: s) -> ()` | GUI meldet aktives Fenster und sichtbare Seite; bei `active = true` wird `CheckAvailability(false)` angestossen | U17, FR-55, AC-M1-23-3 |
| `RegisterGui` | `() -> ()` | Registriert die langlebige D-Bus-Verbindung der laufenden GUI (`NameHasOwner`-Lebendprüfung durch den Daemon vor Tray-Spawn von Fenster / `--confirm-quit`) | §10.1, AC-M1-11-2/3 |
| `ConfirmQuit` | `() -> ()` | Beendet den Daemon sauber nach Bestätigung durch den Benutzer (bricht ggf. laufenden Lauf via `SIGTERM` ab) | NFR-11, §9.5, AC-M1-11-3 |
| `UnlockSecretStore` | `() -> b` | Öffnet bei gesperrtem Passwortspeicher den interaktiven System-Entsperrdialog (`org.freedesktop.Secret.Prompt`), aktualisiert den Daemon-Status und liefert `true` bei Erfolg | ADR-0004, §6.1 |
| `DetectSource` | `(path: s) -> s` (`Source`-JSON) | Erkennt anhand von `/proc/self/mountinfo` und FS-UUID / Mount-Quelle + Dateisystemtyp, ob `path` ein `Folder` oder ein `Drive` ist (inkl. `identity` und `drive_label`) | FR-29, F10, M2-S09 |
| `CheckNestedSources` | `(candidate_id: s, candidate_path: s, destination_id: s) -> s` (`Vec<NestedSourceConflict>`-JSON) | Prüft nach Pfadkomponenten, ob eine Quelle sich mit einem anderen Backup auf demselben Ziel überschneidet (nicht-blockierender Hinweis) | FR-31, M2-S10 |
| `LintExcludes` | `(excludes_json: s) -> s` (`Vec<ExcludeLintWarning>`-JSON) | Prüft Ausschluss-Regeln auf wirkungslose `!`-Negationen unter ganz ausgeschlossenen Ordnern und liefert Korrekturvorschlag (`dir/*`) | FR-02, M2-S03 |
| `TestExcludePath` | `(backup_id: s, excludes_json: s, test_path: s) -> s` (`ExcludePathTestResult`-JSON) | Wertet einen Testpfad lokal anhand der Entwurfs-Ausschlussregeln aus («wird gesichert» / «ausgeschlossen durch Regel / Zeile») | FR-02, M2-S02 |
| `PreviewRetention` | `(backup_id: s, retention_json: s) -> s` (`ForgetPreviewResult`-JSON) | Führt `restic forget --dry-run --no-lock --json` mit den übergebenen Aufbewahrungsregeln aus (`keep`/`remove` + `reasons`) | FR-05, NFR-08, P41, M2-S06 |
| `StartBackupDryRun` | `(backup_id: s, excludes_json: s) -> s` (`preview_id`) | Startet Vorab-Erreichbarkeitsprüfung (`P32`) und asynchron `restic backup --dry-run -vv --json`; optionaler `excludes_json`-Entwurf (leer = gespeicherte Regeln) | FR-17, P32, M2-S04 |
| `CancelBackupDryRun` | `(preview_id: s) -> ()` | Bricht eine laufende Trockenlauf-Vorschau ab (`CancelHandle`) | FR-17, M2-S04 |
| `GetBackupDryRunStatus` | `(preview_id: s) -> s` (`BackupDryRunStatusDto`-JSON) | Liefert Status (`"running"` \| `"done"` \| `"cancelled"` \| `"error"`), Ergebnis oder Fehler einer Vorschau | FR-17, M2-S04 |
| `FormatExcludeRuleForItem` | `(item_path: s) -> s` (`ExcludeRulePreviewDto`-JSON) | Formatiert die effektive Ausschluss-Regel für einen unlesbaren Pfad (Ordner → `$HOME/…`/absolut als Ordner-Regel, Datei → Expertenregel) | F4, P8, M2-S08 |
| `AddExcludeRuleForItem` | `(backup_id: s, item_path: s) -> s` (`ExcludeRulePreviewDto`-JSON) | Fügt einen Pfad aus Exit 3 (`unreadable_files`) als Ausschluss-Regel hinzu, speichert `config.toml` und erzeugt die Exclude-Datei neu | F4, F14, M2-S08 |
| `RunDestinationMaintenance` | `(destination_id: s) -> s` (`run_id`) | Führt einen Wartungslauf am Ziel aus und holt dabei alle Backups mit Status `pending_forget` («forget offen») nach | F4, P38, M2-S07 |

Ergänzte DTO-Felder ab M2: `BackupSummaryDto` trägt zusätzlich `pending_forget` (M2-S07), `nested_warnings` (M2-S10) und `exclude_lint_warnings` (M2-S03); `BackupAvailabilityDto.reason_text` enthält den lokalisierten Laufwerks- oder Quellenfehler (`M2-S09`).

## Signale auf `net.daester.Kestic1`

| Signal | Signatur | Inhalt | Quelle |
| --- | --- | --- | --- |
| `StatusChanged` | `(overall_status_json: s)` | Gesamtstatus und aktualisierte Verfügbarkeit/Zustandsleiste nach jeder Konfigurations-, Lauf- oder Verfügbarkeitsänderung | F14, AC-M1-09-4 |
| `RunUpdated` | `(run_json: s)` | Neuer oder aktualisierter Lauf (`pending`, `running`, `ok`, `warning`, `error`, `cancelled`, `interrupted`) | F14, FR-12, AC-M1-09-4 |
| `RunProgress` | `(run_id: s, percent_done: d, bytes_done: t, total_bytes: t, speed_bps: t, seconds_remaining: t)` | Laufender Fortschritt aus dem restic-JSON-Stream (auch für Fortschrittsanzeige/Taskbar) | F14, FR-12, FR-53 |
| `ManualRunBlocked` | `(backup_id: s, reason: s)` | Manueller Start wegen Vorab-Check blockiert; `reason` = `"source_unavailable"` \| `"destination_unreachable"`. Die offene GUI zeigt Meldung im Fenster; ohne offenes Fenster sendet der Daemon zusätzlich Notification (`ManualRunBlocked`) | FR-30, AC-M1-09-6/7 |
| `ShowWindowRequested` | `()` | Angefordert durch Tray-Klick / «Kestic öffnen» | §10.1, AC-M1-11-2 |
| `QuitConfirmationRequested` | `(has_running_run: b, running_backup_name: s)` | Ausgelöst durch Tray «Kestic beenden…»; GUI öffnet Bestätigungsdialog mit Warnung und ruft erst danach `ConfirmQuit` auf | NFR-11, §9.5, AC-M1-11-3 |
