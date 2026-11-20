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
| `PreviewRetention` | `(backup_id: s, retention_json: s) -> s` (`ForgetPreviewResult`-JSON) | Führt `restic forget --dry-run --no-lock --json` mit den übergebenen Aufbewahrungsregeln synchron aus (`keep`/`remove` + `reasons`) | FR-05, NFR-08, P41, M2-S06 |
| `StartRetentionPreview` | `(backup_id: s, retention_json: s) -> s` (`preview_id`) | Startet Vorab-Erreichbarkeitsprüfung (`P32`) und asynchron `restic forget --dry-run --no-lock --json` | FR-05, P32, P41, M2-S06 |
| `CancelRetentionPreview` | `(preview_id: s) -> ()` | Bricht eine laufende Aufbewahrungsvorschau ab (`CancelHandle`) | FR-05, M2-S06 |
| `GetRetentionPreviewStatus` | `(preview_id: s) -> s` (`RetentionPreviewStatusDto`-JSON) | Liefert Status (`"running"` \| `"done"` \| `"cancelled"` \| `"error"`), Ergebnis oder Fehler einer Aufbewahrungsvorschau | FR-05, M2-S06 |
| `StartBackupDryRun` | `(backup_id: s, excludes_json: s) -> s` (`preview_id`) | Startet Vorab-Erreichbarkeitsprüfung (`P32`) und asynchron `restic backup --dry-run -vv --json`; optionaler `excludes_json`-Entwurf (leer = gespeicherte Regeln) | FR-17, P32, M2-S04 |
| `CancelBackupDryRun` | `(preview_id: s) -> ()` | Bricht eine laufende Trockenlauf-Vorschau ab (`CancelHandle`) | FR-17, M2-S04 |
| `GetBackupDryRunStatus` | `(preview_id: s) -> s` (`BackupDryRunStatusDto`-JSON) | Liefert Status (`"running"` \| `"done"` \| `"cancelled"` \| `"error"`), Ergebnis oder Fehler einer Vorschau | FR-17, M2-S04 |
| `FormatExcludeRuleForItem` | `(item_path: s) -> s` (`ExcludeRulePreviewDto`-JSON) | Formatiert die effektive Ausschluss-Regel für einen unlesbaren Pfad (Ordner → `$HOME/…`/absolut als Ordner-Regel, Datei → Expertenregel) | F4, P8, M2-S08 |
| `AddExcludeRuleForItem` | `(backup_id: s, item_path: s) -> s` (`ExcludeRulePreviewDto`-JSON) | Fügt einen Pfad aus Exit 3 (`unreadable_files`) als Ausschluss-Regel hinzu, speichert `config.toml` und erzeugt die Exclude-Datei neu | F4, F14, M2-S08 |
| `RunDestinationMaintenance` | `(destination_id: s) -> s` (`run_id`) | Führt einen Wartungslauf am Ziel aus und holt dabei alle Backups mit Status `pending_forget` («forget offen») nach | F4, P38, M2-S07 |
| `PauseGlobal` | `(duration_json: s) -> ()` | Pausiert alle Zeitpläne und Wartungsläufe global (`PauseDuration`-JSON: `hours`, `until_tomorrow`, `indefinite`) | FR-13a, P14, M3-S07 |
| `ResumeGlobal` | `() -> ()` | Hebt die globale Pause auf und startet die Überfälligkeits-Zählung ab diesem Zeitpunkt neu | FR-13a, P14, M3-S07 |
| `PauseBackup` | `(backup_id: s, duration_json: s) -> ()` | Pausiert ein einzelnes Backup gemäss `PauseDuration`-JSON; andere Backups auf demselben Ziel laufen weiter | FR-13a, M3-S07 |
| `ResumeBackup` | `(backup_id: s) -> ()` | Hebt die Pause eines einzelnen Backups auf und setzt den Überfälligkeits-Anker bei Bedarf neu | FR-13a, M3-S07 |
| `StartBackupOverride` | `(backup_id: s) -> s` (`run_id`) | Manueller Einzel-Start mit einmaliger Übersteuerung nicht erfüllter Laufbedingungen (`FR-13b`); Quellen-/Ziel-Vorab-Check gilt weiterhin | FR-13b, M3-S03 |
| `UnlockDestination` | `(destination_id: s) -> u` (Anzahl entfernter Locks) | Prüft veraltete lokale Locks (`hostname` + tote PID) und führt nur dann `restic unlock` aus (`P16`) | FR-19, P16, M3-S06 |
| `PreviewDestinationMaintenance` | `(destination_id: s) -> s` (`PrunePreviewSummary`-JSON) | Führt `restic prune --dry-run --no-lock` vor Aktivierung/Änderung des Aufräum-Zeitplans aus | NFR-08, P10, P41, M3-S05 |
| `UpdateDestinationMaintenance` | `(destination_id: s, maintenance_json: s) -> ()` | Validiert `MaintenanceSchedule` in `core` und speichert es in `config.toml` | FR-45, FR-11, P15, M3-S14 |
| `UpdateSettings` | `(default_overdue_threshold: u, notifications_json: s) -> ()` | Aktualisiert globale Überfälligkeits-Schwelle und F3-Benachrichtigungs-Toggles in `config.toml` | F3, §9.6, M3-S08 |
| `ListKnownNetworks` | `() -> s` (`Vec<String>`-JSON) | Liest bekannte Netzwerkverbindungen aus `org.freedesktop.NetworkManager.Settings` für die Laufbedingungen | FR-13, M3-S14 |

Ergänzte DTO-Felder ab M2/M3: `BackupSummaryDto` trägt zusätzlich `pending_forget` (M2-S07), `nested_warnings` (M2-S10), `exclude_lint_warnings` (M2-S03) sowie `is_paused`, `paused_since_unix`, `paused_until_unix`, `next_scheduled_at_unix`, `pending_primary_code`, `pending_reason_codes`, `pending_retry_at_unix`, `missed_slots_count`, `has_unmet_run_condition`, `unmet_run_conditions` und `battery_percent` (M3); `BackupAvailabilityDto.source_status` nennt den Laufwerks- oder Quellenfehler als Code (`M2-S09`); `DestinationSummaryDto` trägt Wartungszeitpunkte (`next_prune_at_unix`, `next_check_at_unix`, `last_prune_at_unix`, `last_check_at_unix`, `last_maintenance_run`, `maintenance_running`, `pending_forget_count`, `stale_lock_count`, `maintenance_pending_reason`, `maintenance_retry_at_unix`); `OverallStatusDto` trägt Pause-Metadaten (`paused_since_unix`, `paused_until_unix`, `next_scheduled_backup_name`, `next_scheduled_at_unix`).

**Sprachunabhängige Codes (Variante A+, Entscheid 2026-11-20):** Die API ist privat (Daemon und GUI im selben `.deb`, keine anderen Aufrufer), Rückwärtskompatibilität ist deshalb kein Ziel. Vertrag sind Codes, nicht Texte:

- **Keine Texte in Daemon-Sprache in DTOs.** Die DTOs tragen Codes, Zahlen und Rohdaten. Texte entstehen erst dort, wo sie angezeigt werden: die GUI übersetzt in ihrer Sprache (KI18n), der Daemon für Tray, Benachrichtigungen und Lauf-Historie in seiner (`daemon/src/i18n.rs`). Beide lesen `po/de/kestic.po`. Die GUI kennt keinen Rückfall auf Daemon-Texte.
- **Typisierte Codes in `core`.** Jeder Code ist ein Rust-Enum in `kestic_core::codes` und wird als `snake_case`-String serialisiert. Unbekannte Codes sind ein Deserialisierungsfehler, kein stiller Rückfall.
- **Genau eine Abbildung Code → msgid.** `kestic_core::messages` liefert pro Code die englische msgid samt Parametern (`Message`); Rohdaten, die unverändert angezeigt werden, kommen als `UiText::Raw`. Die msgids sind mit `i18n_noop("…")` markiert und werden mit `xgettext --keyword=i18n_noop:1` extrahiert. Daemon und GUI haben keine eigenen `match`-Tabellen über Codes.
- **Rohdaten bleiben Daten.** `error_detail`, `error_raw_detail`, `ResticStatusDto.detail`, `SecretStoreStatusDto.detail`, `ConnectionTestResultDto.detail`, `SkippedBackupDto.error_detail`, `DestinationAvailabilityDto.detail`, `BackupSnapshotsDto.current_error` und `config_load_error` enthalten den unübersetzten Text von restic, System oder Daemon und dienen als Detail bzw. Parameter.

| Feld | Enum (`kestic_core`) | Codes |
| --- | --- | --- |
| `BackupSummaryDto.error_kind`, `RunDetailDto.error_kind` (Detail: `error_detail` bzw. `error_raw_detail`, dazu `error_exit_code`) | `codes::ErrorKind` | `wrong_password`, `repo_missing`, `locked`, `destination_unreachable` (inkl. DNS), `source_unavailable`, `auth_failed`, `incomplete_snapshot`, `retention_pending`, `cancelled`, `unknown` (alles Nichterkannte, nie ein vorübergehender Code) |
| `SkippedBackupDto.error_kind` | `codes::ErrorKind` | bei `reason = permanent_error` der `error_kind` des letzten Laufs; bei `reason = error` `secret_store_locked` \| `restic_unavailable` \| `auth_failed` \| `wrong_password` \| `destination_unreachable` \| `unknown` (dann `error_detail` als Rohtext) |
| `SkippedBackupDto.reason` | `codes::SkipReason` | `permanent_error`, `source_unavailable`, `destination_unreachable`, `condition_not_met`, `error` |
| `BackupSummaryDto.error_action`, `RunDetailDto.error_action` | `restic::RecoveryAction` | `update_stored_password`, `add_to_excludes`, `check_restic_source`, `unlock_repository` |
| `BackupSummaryDto.pending_primary_code`, `pending_reason_codes` (nächster Versuch bei `backoff`: `pending_retry_at_unix`); `DestinationSummaryDto.maintenance_pending_reason` (fällige Wartung `ausstehend`, AC-M3-05-5; nächster Versuch: `maintenance_retry_at_unix`) | `codes::PendingReasonCode` | `source_unavailable`, `destination_unreachable`, `on_battery`, `no_network`, `metered`, `wrong_network`, `queue`, `backoff` |
| `BackupSummaryDto.unmet_run_conditions` (Parameter: `battery_percent`, erlaubte Netzwerke aus `backup.run_conditions`) | `codes::PendingReasonCode` | `on_battery`, `no_network`, `metered`, `wrong_network` |
| `BackupAvailabilityDto.blocked_reason`; Signal `ManualRunBlocked.reason` und Fehler `Error.ManualRunBlocked` als String | `codes::BlockedReason` | `source_unavailable`, `destination_unreachable`, `condition_not_met` |
| `BackupAvailabilityDto.source_status` | `codes::SourceStatus` | `path_missing`, `drive_not_mounted`, `drive_mismatch`, `drive_empty` |
| `ResticStatusDto.error_kind` (Parameter: `binary_path`, `found_version`, `detail`) | `codes::ResticStatusCode` | `not_found`, `too_old`, `invalid_custom_path`, `unreadable_version` |
| `BlockingStateDto.kind` (Parameter für `restic_unavailable` aus `ResticStatusDto`) | `codes::BlockingKind` | `secret_store_locked`, `restic_unavailable` |
| `ConnectionTestResultDto.error_kind` (Parameter bzw. Rohtext: `detail`) | `codes::ConnectionTestCode` | `invalid_input`, `duplicate_location` (`detail` = Name des Ziels), `destination_unreachable`, `repository_exists`, `not_writable`, `missing_password`, `secret_store_locked`, `internal`, `restic_unavailable`, `wrong_password`, `auth_failed`, `repository_missing`, `command_failed`, `daemon_unreachable` (nur GUI-seitig) |
| `OverallStatusDto.kind` | `codes::OverallStatusKind` | `blocked`, `error`, `overdue`, `warning`, `running`, `paused`, `pending`, `interrupted`, `cancelled`, `never_run`, `manual_only`, `ok` |
| `OverallStatusDto.tray_status` | `daemon::tray::TrayStatus` | `Passive`, `Active`, `NeedsAttention` (StatusNotifierItem-Werte) |

Überschrift und Untertitel des Gesamtstatus (Übersicht und Tray) berechnen GUI und Daemon mit derselben Funktion `kestic_core::messages::overall_headline` bzw. `overall_subtitle` aus `OverallStatusDto` (Art, Zähler, aktiver Lauf) und `BlockingStateDto`; das DTO trägt keine `headline`, `subtitle` oder `paused_label`.

## Signale auf `net.daester.Kestic1`

| Signal | Signatur | Inhalt | Quelle |
| --- | --- | --- | --- |
| `StatusChanged` | `(overall_status_json: s)` | Gesamtstatus und aktualisierte Verfügbarkeit/Zustandsleiste nach jeder Konfigurations-, Lauf- oder Verfügbarkeitsänderung | F14, AC-M1-09-4 |
| `RunUpdated` | `(run_json: s)` | Neuer oder aktualisierter Lauf (`pending`, `running`, `ok`, `warning`, `error`, `cancelled`, `interrupted`) | F14, FR-12, AC-M1-09-4 |
| `RunProgress` | `(run_id: s, percent_done: d, bytes_done: t, total_bytes: t, speed_bps: t, seconds_remaining: t)` | Laufender Fortschritt aus dem restic-JSON-Stream (auch für Fortschrittsanzeige/Taskbar) | F14, FR-12, FR-53 |
| `ManualRunBlocked` | `(backup_id: s, reason: s)` | Manueller Start wegen Vorab-Check oder Laufbedingung blockiert; `reason` = `"source_unavailable"` \| `"destination_unreachable"` \| `"condition_not_met"`. Die offene GUI zeigt Meldung bzw. Rückfrage im Fenster; ohne offenes Fenster sendet der Daemon zusätzlich Notification (`ManualRunBlocked`) | FR-30, FR-13b, AC-M1-09-6/7 |
| `ShowWindowRequested` | `()` | Angefordert durch Tray-Klick / «Kestic öffnen» | §10.1, AC-M1-11-2 |
| `QuitConfirmationRequested` | `(has_running_run: b, running_backup_name: s)` | Ausgelöst durch Tray «Kestic beenden…»; GUI öffnet Bestätigungsdialog mit Warnung und ruft erst danach `ConfirmQuit` auf | NFR-11, §9.5, AC-M1-11-3 |
