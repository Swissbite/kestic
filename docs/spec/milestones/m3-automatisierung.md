# M3 Automatisierung

Teil von [backlog.md](../backlog.md).

**Ziel:** Backups laufen von selbst, zuverlässig und sichtbar: Zeitplan, Laufbedingungen, Neuversuch, Überfälligkeit, Wartung pro Ziel, Ruhezustand.

**Voraussetzung:** M2.

## Anforderungen

FR-03, FR-11, FR-12, FR-13, FR-13a, FR-13b (Übersteuern), FR-19, FR-29 (zeitplangesteuert), FR-30, FR-45, FR-53 · NFR-04, NFR-08 (für `prune`), NFR-11 · E11, E13, F1, F2, F3, F9, F11 · P10, P13, P14, P15, P16, P32, P33, P35, P36, P41 · U18 (Editoren Zeitplan, Laufbedingungen) · baut auf M1-S23 (Verfügbarkeitsstatus) auf

## Stories

| ID | Story |
| --- | --- |
| M3-S01 | Scheduler in `core` auf Basis des Clock-Traits aus M1 (P36): Kalender-Zeitpläne, lokale Zeitzone, DST, verpasste Termine einmal nachholen (FR-03, F1) |
| M3-S02 | Status `ausstehend` mit Grund; Überfälligkeit = zwei verpasste Termine (Glossar) |
| M3-S03 | Laufbedingungen: Strom (UPower), Netzwerk/getaktet/SSID (NetworkManager), Neuversuch bei Signal (FR-13) |
| M3-S04 | Ziel nicht erreichbar (Vorab-Check P32), Backoff 5/15/60 Min. ab Ende des letzten Versuchs, dann nächster Termin; Lock (Exit 11) als vorübergehender Fehler, ohne `--retry-lock` (P33); dauerhafte Fehler sofort melden (FR-30, F2, E13) |
| M3-S05 | Warteschlange pro Ziel; Wartungslauf: zuerst offene `forget` aller Backups (F4), dann `prune` wöchentlich, `check` monatlich, nur am Strom und nicht getaktet (FR-45, FR-11, F11) |
| M3-S06 | Veraltete Locks erkennen, `unlock` anbieten (FR-19) |
| M3-S07 | Pausieren pro Backup und global, «für n Stunden», keine Überfällig-Warnung während Pause, Tray «pausiert seit …» (FR-13a) |
| M3-S08 | Monitoring: Benachrichtigungen gemäss F3 (abschaltbar pro Art), Überfällig-Warnung, Status-Übersicht in GUI und Tray (FR-12) |
| M3-S09 | Ruhezustand: Portal-Inhibit (Idle + Suspend) während Lauf, logind `sleep`-Delay, Lauf → `unterbrochen`, nach Aufwachen `ausstehend` (F9) |
| M3-S10 | Nachholen beim Daemon-Start und nach Aufwachen (logind `PrepareForSleep`) |
| M3-S11 | Paket-Update: Daemon erkennt ersetztes Binary, startet nach laufendem Lauf neu (E11) |
| M3-S12 | Fortschrittsanzeige in Plasma über `org.kde.JobViewServerV2` für jeden Lauf (FR-53) |
| M3-S13 | Zeitplangesteuerte Quellen-Prüfung: fälliges Backup mit nicht verfügbarer Quelle bleibt `ausstehend`, Neuversuch bei Mount-Wechsel (UDisks2- bzw. mountinfo-Signal), Alert erst bei «überfällig» (FR-29, P36) |
| M3-S14 | GUI-Editoren für Zeitplan (Default täglich 12:00, P35), Laufbedingungen, Pause und Wartung (Default `prune` sonntags 12:00, `check` am ersten Sonntag im Monat 12:00, P15); Zeitplan und Laufbedingungen mit explizitem `[Speichern]` = `UpdateBackup` (U18) |

## Akzeptanzkriterien

Test-Isolation gemäss [test-strategy.md](../test-strategy.md). Zeitzone in allen Uhr-Tests: Europe/Zurich. UPower, NetworkManager, logind, Notifications und Portal als D-Bus-Fakes.

### M3-S01 Scheduler
- **AC-M3-01-1** (FR-03, F1) – *Gegeben* «täglich 12:00», letzter Erfolg Mo 12:00, *wenn* die simulierte Uhr Di 12:00 erreicht, *dann* genau ein Lauf fällig (`ausstehend` → `läuft`) · Test: Unit (simulierte Uhr)
- **AC-M3-01-2** (FR-03, F1) – *Gegeben* «Mo/Mi/Fr 20:00», *wenn* die Uhr von Mo 20:01 bis Fr 20:01 läuft, *dann* (a) Fälligkeiten genau Mi 20:00 und Fr 20:00, und (b) keine Di und Do · Test: Unit (simulierte Uhr)
- **AC-M3-01-3** (FR-03, F1) – *Gegeben* «alle 3 Tage ab 2026-12-01, 09:00», *wenn* die Uhr bis 2026-12-10 09:01 läuft, *dann* Termine 01., 04., 07., 10. Dezember 09:00 · Test: Unit (simulierte Uhr)
- **AC-M3-01-4** (F1, verpasste Termine) – *Gegeben* «täglich 12:00», letzter Erfolg So 12:00, Rechner aus Mo 11:00 bis Di 14:00 (Mo 12:00 und Di 12:00 verpasst), *wenn* der Daemon Di 14:00 startet, *dann* (a) **genau ein** Nachhol-Lauf (nicht zwei), und (b) gilt das Backup bis zu dessen Erfolg als überfällig (zwei verpasste Termine) · Test: Unit (simulierte Uhr)
- **AC-M3-01-5** (F1, DST-Lücke) – *Gegeben* «täglich 02:30», *wenn* die Uhr über So 2027-03-28 läuft (02:00 → 03:00), *dann* (a) Lauf um 03:00 fällig (nächste gültige Minute), und (b) genau einmal · Test: Unit (simulierte Uhr)
- **AC-M3-01-6** (F1, P13, DST-Rückstellung) – *Gegeben* «täglich 02:30», *wenn* die Uhr über So 2026-10-25 läuft (02:30 zweimal), *dann* an diesem Tag genau einmal fällig · Test: Unit (simulierte Uhr)
- **AC-M3-01-7** (F1, Suspend-Lücke) – *Gegeben* «täglich 12:00», Suspend Mo 10:00 bis Mo 15:00, *wenn* das Aufwachen simuliert wird, *dann* genau ein Lauf fällig · Test: Unit (simulierte Uhr + logind-Fake)
- **AC-M3-01-8** (F1, Zeitplan-Anker) – *Gegeben* ein neuer oder geänderter Zeitplan, *wenn* er gespeichert wird, *dann* zählen Termine erst ab diesem Zeitpunkt: Ziel ohne bisheriges `check`, um 16:31 «Prüfen: täglich 16:32» gespeichert → (a) kein Lauf um 16:31, (b) erster `check` um 16:32; Backup «täglich 12:00» mit letztem Erfolg gestern 12:00, heute 15:00 auf «täglich 14:00» geändert → (c) kein sofortiger Lauf, (d) nächster Lauf morgen 14:00, (e) die Überfällig-Zählung beginnt bei der Änderung neu; Wechsel von «nur manuell» auf «täglich 12:00» um 15:00 → (f) kein sofortiger Lauf, (g) erster Lauf morgen 12:00; und (h) ändert sich nur der Prüf-Zeitplan, holt der Aufräum-Zeitplan einen verpassten Termin weiterhin nach · Test: Unit (simulierte Uhr) + D-Bus

### M3-S02 Status `ausstehend` und Überfälligkeit
- **AC-M3-02-1** (Glossar §5.1) – *Gegeben* fälliges Backup, das nicht starten kann, *dann* Status `ausstehend` mit genau einem Grund aus: Laufbedingung nicht erfüllt, Quelle nicht verfügbar, Ziel nicht erreichbar, Warteschlange, Backoff nach Fehler · Test: Unit (je Grund ein Fall)
- **AC-M3-02-2** (Glossar «überfällig») – *Gegeben* «Mo/Mi/Fr 20:00», letzter Erfolg Mo 20:00, *wenn* Mi 20:00 und Fr 20:00 ohne Erfolg verstreichen, *dann* (a) überfällig ab Fr 20:00, und (b) Mi 20:01 noch nicht · Test: Unit (simulierte Uhr)
- **AC-M3-02-3** (Glossar «überfällig», Anzahl einstellbar) – *Gegeben* Einstellung «3 verpasste Termine» und «täglich 12:00», *dann* (a) nach zwei Terminen nicht überfällig, und (b) nach dem dritten überfällig · Test: Unit
- **AC-M3-02-4** (Glossar, negativ) – *Gegeben* überfälliges Backup, *wenn* ein Lauf mit `ok` oder `Warnung` endet, *dann* (a) nicht mehr überfällig, und (b) bei `Fehler` bleibt es überfällig · Test: Unit
- **AC-M3-02-5** ([zustaende.md](../ui/zustaende.md) §6.3) – *Gegeben* ein Backup wartet gleichzeitig wegen fehlender Quelle, Akku und Backoff, *wenn* der Status angezeigt wird, *dann* (a) erscheint «Wartet: Quelle nicht verfügbar» (Rangfolge: Quelle > Ziel > Laufbedingung (Strom > Netzwerk > getaktet > erlaubtes Netzwerk) > Warteschlange > Backoff), (b) stehen die übrigen Gründe im Tooltip, und (c) gilt dies gleich in GUI, Tray und Verlauf · Test: Unit + GUI-Smoke

### M3-S03 Laufbedingungen
- **AC-M3-03-1** (FR-13, Default) – *Gegeben* neues Backup, *dann* (a) «nur am Strom» aktiv, und (b) «nur mit Netzwerk» aktiv, sofern das Ziel ein Netzwerk-Ziel ist; bei einem lokalen Ordner oder Laufwerk gibt es keine Netzwerk-Bedingungen (Entscheid 2026-11-20) · Test: Unit
- **AC-M3-03-2** (FR-13, Strom) – *Gegeben* fälliges Backup und UPower `OnBattery=true`, *dann* (a) `ausstehend` (Grund Laufbedingung), und (b) startet der Lauf bei `OnBattery=false` ohne Warten auf den nächsten Termin · Test: D-Bus (Fake)
- **AC-M3-03-3** (FR-13, getaktet) – *Gegeben* «nicht bei getakteter Verbindung» und NetworkManager `Metered=yes`, *dann* (a) `ausstehend`, und (b) startet es bei `Metered=no` · Test: D-Bus (Fake)
- **AC-M3-03-4** (FR-13, Netzwerk X) – *Gegeben* «nur im Netzwerk Heim», aktive Verbindung «Café», *dann* (a) `ausstehend`, und (b) startet der Lauf bei Wechsel auf «Heim» · Test: D-Bus (Fake)
- **AC-M3-03-5** (FR-13, kein Netzwerk) – *Gegeben* «nur mit Netzwerk», NetworkManager ohne Verbindung, *dann* `ausstehend` · Test: D-Bus (Fake)
- **AC-M3-03-6** (FR-13, Alert-Logik) – *Gegeben* Laufbedingung über zwei Termine nicht erfüllt, *dann* (a) genau eine Benachrichtigung «überfällig», und (b) vorher keine · Test: D-Bus (Notification-Fake, simulierte Uhr)
- **AC-M3-03-7** (FR-13b) – *Gegeben* UPower `OnBattery=true` und «nur am Strom», *wenn* manuell gestartet, *dann* (a) zeigt Kestic die nicht erfüllte Bedingung und bietet «trotzdem starten», (b) läuft das Backup nach Bestätigung, (c) bleibt es ohne Bestätigung `ausstehend`, und (d) gilt die Übersteuerung nur für diesen Lauf · Test: D-Bus (Fake) + GUI-Smoke

### M3-S04 Ziel nicht erreichbar, Backoff, Lock
- **AC-M3-04-1** (FR-30, F2, P32) – *Gegeben* gestoppter `rest-server` (Verbindung sofort abgelehnt), *wenn* der Termin 12:00 erreicht wird, *dann* (a) scheitert der Vorab-Check, (b) entsteht kein restic-Prozess, (c) wird der Status `ausstehend` (Grund Ziel nicht erreichbar), und (d) erfolgen Neuversuche 12:05, 12:20, 13:20 sowie danach erst am nächsten regulären Termin · Test: Integration (rest-server) + simulierte Uhr
- **AC-M3-04-6** (F2, Backoff ab Ende des Versuchs) – *Gegeben* Vorab-Check scheitert jeweils erst nach Connect-Timeout 10 s, *wenn* der Termin 12:00:00 erreicht wird, *dann* (a) erfolgen Neuversuche 12:05:10, 12:20:20 und 13:20:30 · Test: Unit (simulierte Uhr, Check-Fake mit 10 s Dauer)
- **AC-M3-04-7** (P32) – *Gegeben* REST-Ziel mit gestopptem `rest-server`, echte Uhr, *wenn* ein geplanter Lauf fällig wird, *dann* (a) steht der Status spätestens 15 s nach Fälligkeit auf `ausstehend` (Ziel nicht erreichbar) · Test: Integration (Zeitmessung)
- **AC-M3-04-2** (FR-30) – *Gegeben* `ausstehend` wegen Ziel nicht erreichbar, *wenn* NetworkManager einen Netzwerkwechsel meldet und `rest-server` wieder läuft, *dann* (a) startet der Lauf sofort · Test: Integration + D-Bus (Fake)
- **AC-M3-04-3** (FR-30, Alert) – *Gegeben* Ziel nicht erreichbar bei geplantem Lauf, *dann* (a) keine Benachrichtigung bis überfällig, (b) gilt beim **manuellen** Start FR-30 («Jetzt sichern» deaktiviert mit sichtbarem Grund), und (c) erscheint `ManualRunBlocked` nur bei Zustandswechsel zwischen Anzeige und Klick · Test: Integration + Notification-Fake
- **AC-M3-04-4** (E13, F2: `backup` Exit 11 → `ausstehend` + Backoff) – *Gegeben* zweiter Prozess hält **exklusiven** Lock (laufendes `restic check`), *wenn* ein geplanter Backup-Lauf startet und `backup` mit Exit 11 endet, *dann* (a) `ausstehend` (Backoff nach Fehler), (b) Neuversuche 5/15/60 Min. ab Ende des letzten Versuchs, und (c) keine sofortige Benachrichtigung · Test: Integration + simulierte Uhr
- **AC-M3-04-8** (E13, F2, F4: `forget` Exit 11 → `Warnung` + «forget offen») – *Gegeben* zweiter laufender `restic backup --stdin` hält **nicht-exklusiven** Lock, *wenn* ein Kestic-Backup läuft, *dann* (a) `backup` Exit 0, (b) `forget` Exit 11, (c) Lauf `Warnung` mit «forget offen», und (d) kein Backoff für das Backup · Test: Integration
- **AC-M3-04-11** (FR-30) – *Gegeben* Verfügbarkeitsstatus aus M1-S23 (Aktualisierung beim Öffnen von Fenster oder Tray-Menü) und Ziel mit gestopptem Server, *wenn* NetworkManager einen Netzwerkwechsel meldet, *dann* (a) erneuert der Daemon zusätzlich den Vorab-Check (insgesamt höchstens einmal pro 60 s), (b) ist «Jetzt sichern» danach mit «Ziel nicht erreichbar» deaktiviert, und (c) ist «Jetzt sichern» nach dem nächsten Check wieder aktiv, wenn der Server wieder läuft · Test: D-Bus (NM-Fake) + Integration (`rest-server`)
- **AC-M3-04-9** (P33) – *Gegeben* alle von Kestic erzeugten restic-Aufrufe (`backup`, `forget`, `prune`, `check`), *dann* (a) enthält keiner `--retry-lock` · Test: Unit
- **AC-M3-04-5** (F2, dauerhafter Fehler, negativ) – *Gegeben* falsches Repo-Passwort im Secret-Store, *wenn* der Lauf mit Exit 12 endet, *dann* (a) `Fehler`, (b) sofortige Benachrichtigung, und (c) kein Neuversuch · Test: Integration

### M3-S05 Warteschlange und Wartung
- **AC-M3-05-1** (§5.1 Warteschlange) – *Gegeben* Backups A und B auf demselben Ziel, beide 12:00 fällig, *dann* (a) laufen sie nacheinander, und (b) ist B währenddessen `ausstehend` (Warteschlange) · Test: Integration
- **AC-M3-05-2** (FR-45, F11, P15) – *Gegeben* Ziel ohne eigene Einstellung, *dann* (a) `prune` sonntags 12:00, (b) `check` am ersten Sonntag im Monat 12:00, und (c) im Dezember 2026 `check` am So 2026-12-06 12:00 · Test: Unit (simulierte Uhr)
- **AC-M3-05-3** (FR-45) – *Gegeben* laufendes Backup auf Ziel Z, *wenn* der Wartungslauf für Z fällig wird, *dann* (a) startet er erst nach Ende des Backups, und (b) startet während eines Wartungslaufs kein Backup auf Z · Test: Integration
- **AC-M3-05-4** (Wartungslauf, F4) – *Gegeben* Backup A mit offenem `forget` (letzter Lauf `Warnung` wegen Exit 11), *wenn* der Wartungslauf startet, *dann* (a) zuerst `forget --tag kestic:<A> --group-by tags`, und (b) danach `prune` · Test: Integration
- **AC-M3-05-5** (F11, negativ) – *Gegeben* fälliger Wartungslauf und UPower `OnBattery=true` oder NetworkManager `Metered=yes`, *dann* (a) `ausstehend`, und (b) startet er bei Strom und nicht getaktet · Test: D-Bus (Fake)
- **AC-M3-05-6** (FR-11) – *Gegeben* fälliges `check`, *dann* (a) `restic check` ohne `--read-data`, und (b) Ergebnis als Wartungslauf in der Historie des Ziels · Test: Integration
- **AC-M3-05-7** (NFR-08 für `prune`, P10, P41) – *Gegeben* der Benutzer aktiviert oder ändert den `prune`-Zeitplan eines Ziels, *dann* (a) erscheinen vorher Vorschau (`prune --dry-run --no-lock`) und Bestätigung mit klarer Folge, und (b) fragen geplante `prune`-Läufe danach nicht erneut · Test: GUI-Smoke + Integration
- **AC-M3-05-8** (NFR-08, P41) – *Gegeben* laufendes Backup auf demselben Ziel (nicht-exklusiver Lock), *wenn* die `prune`-Vorschau angefordert wird, *dann* (a) liefert sie ein Ergebnis statt Exit 11, und (b) enthält der Aufruf `--no-lock` · Test: Integration (Temp-Repo)
- **AC-M3-05-9** ([ui/hauptbereiche.md](../ui/hauptbereiche.md) §7.8, Lauf-Detail Wartung) – *Gegeben* ein Wartungslauf mit offenem `forget` für Backup A (2 Snapshots entfernt), erfolgreichem `prune` (1.3 MB freigegeben) und `check`, *wenn* sein Lauf-Detail geöffnet wird, *dann* (a) Titel «Wartung vom ‹Datum›», (b) Statuszeile «Aufgeräumt und geprüft · ‹Dauer›», (c) darunter der Name des Ziels, und im Tab «Ergebnis» (d) «A: 2 Snapshots entfernt», (e) «Aufräumen: 1.3 MB freigegeben» (nur wenn restic den Platz nennt, [restic-integration.md](../restic-integration.md) §Wartung) und (f) «Prüfen: keine Fehler gefunden» · Test: Unit + GUI-Smoke

### M3-S06 Veraltete Locks
- **AC-M3-06-1** (FR-19, P16) – *Gegeben* veralteter Lock vom eigenen Host, erzeugt durch SIGKILL eines laufenden exklusiven `restic check` (bzw. Lock-Datei-Fixture mit eigenem Hostnamen und nicht existierender PID), *wenn* der nächste Lauf startet, *dann* (a) endet `backup` mit Exit 11 **[verifiziert, restic 0.19.1: per SIGKILL liegengebliebener exklusiver Lock von `check` blockiert `backup` und `forget` mit Exit 11 bis `restic unlock`]**, (b) erkennt Kestic den veralteten Lock und bietet `unlock` an, (c) läuft danach der Lauf, und (d) hinterlässt ein mit SIGTERM abgebrochener Lauf keinen Lock (restic gibt ihn frei) und löst kein Angebot aus · Test: Integration
- **AC-M3-06-2** (FR-19, P16, negativ) – *Gegeben* Lock eines laufenden Backups (anderer Prozess aktiv), *dann* (a) **kein** `unlock`-Angebot · Test: Integration
- **AC-M3-06-3** (FR-19, P16, negativ) – *Gegeben* Lock mit fremdem Hostnamen, *dann* (a) **kein** `unlock`-Angebot, auch ohne passenden lokalen Prozess · Test: Integration (Lock-Datei mit fremdem Hostnamen als Fixture direkt im Repo; `RESTIC_HOST` ändert den Hostnamen im Lock nicht)

### M3-S07 Pausieren
- **AC-M3-07-1** (FR-13a) – *Gegeben* Backup A pausiert, *wenn* sein Termin erreicht wird, *dann* (a) kein Lauf für A, und (b) läuft Backup B auf demselben Ziel normal · Test: Unit (simulierte Uhr)
- **AC-M3-07-2** (FR-13a, P14, global) – *Gegeben* globale Pause, *dann* (a) kein geplanter Lauf, (b) kein Wartungslauf, und (c) startet ein manueller Lauf (FR-13b) trotzdem · Test: Unit
- **AC-M3-07-3** (FR-13a, P14, «für n Stunden») – *Gegeben* Pause «für 4 Stunden» ab 10:00, *wenn* die Uhr 14:00 erreicht, *dann* (a) ist die Pause aufgehoben, und (b) wird ein in der Pause verpasster Termin einmal nachgeholt · Test: Unit (simulierte Uhr)
- **AC-M3-07-4** (FR-13a, P14, Überfälligkeit) – *Gegeben* Pause über drei Termine, *dann* (a) erscheint während der Pause keine Überfällig-Warnung, (b) zeigt das Tray «pausiert seit <Zeitpunkt>» mit SNI-Status `Active` (K1 = B, Pause im Panel), und (c) zählen nach der Pause nur Termine danach für «überfällig» · Test: Unit + D-Bus (Notification-Fake)

### M3-S08 Monitoring
- **AC-M3-08-1** (FR-12, F3) – *Gegeben* Standard-Einstellungen, *dann* (a) erzeugen `Fehler`, `Warnung` und «überfällig» je eine Benachrichtigung, und (b) erzeugt `ok` nur einen Eintrag in Tray und Historie · Test: D-Bus (Notification-Fake)
- **AC-M3-08-2** (F3, abschaltbar) – *Gegeben* Ereignis-Art «Warnung» abgeschaltet, *wenn* ein Lauf mit Exit 3 endet, *dann* (a) keine Benachrichtigung, und (b) steht `Warnung` trotzdem in Historie und Tray · Test: D-Bus (Notification-Fake)
- **AC-M3-08-3** (FR-12) – *Gegeben* drei Backups (`ok`, `ausstehend`, überfällig), *dann* zeigt die Status-Übersicht alle drei mit Status, Grund und nächstem Termin · Test: GUI-Smoke (Screenshot)
- **AC-M3-08-4** (NFR-11, NFR-04) – *Gegeben* Hauptfenster geschlossen, *wenn* ein geplanter Lauf läuft, *dann* (a) zeigt das Tray Status und Fortschritt, und (b) steht der Lauf danach in der Historie · Test: D-Bus + manuell
- **AC-M3-08-5** (FR-37) – *Gegeben* ein Backup mit dauerhaftem Fehler (Exit 12), *wenn* Übersicht oder Detail angezeigt wird, *dann* (a) ersetzt «Gespeichertes Passwort aktualisieren…» den Knopf «Jetzt sichern», und (b) steht «Trotzdem jetzt sichern» im ⋮-Menü · Test: GUI-Smoke
- **AC-M3-08-6** (FR-13b) – *Gegeben* drei Backups, davon eins mit fehlender Quelle und eins mit Exit 12, *wenn* «Alle jetzt sichern» (Übersicht oder Tray) gewählt wird, *dann* (a) startet nur das dritte, und (b) nennt eine Meldung (auch im Verlauf) die zwei übersprungenen mit Grund · Test: D-Bus + GUI-Smoke

### M3-S09 Ruhezustand
- **AC-M3-09-1** (F9) – *Gegeben* ein Lauf startet, *dann* (a) hält Kestic ein Portal-Inhibit (Idle + Suspend), (b) hält Kestic einen logind-Inhibitor `sleep` im Modus `delay`, und (c) werden nach Lauf-Ende beide freigegeben · Test: D-Bus (Portal- und logind-Fake)
- **AC-M3-09-2** (F9, Deckel zu) – *Gegeben* laufendes Backup, *wenn* logind `PrepareForSleep(true)` sendet, *dann* (a) SIGTERM an restic, (b) Lauf `unterbrochen`, und (c) `delay`-Inhibitor freigegeben · Test: D-Bus (Fake) + Integration
- **AC-M3-09-3** (F9) – *Gegeben* `unterbrochen`er Lauf, *wenn* `PrepareForSleep(false)` folgt, *dann* (a) `ausstehend`, und (b) Start sobald Laufbedingungen erfüllt · Test: D-Bus (Fake)
- **AC-M3-09-4** (F9, manuell) – Deckel schliessen während eines echten Laufs; nach dem Aufwachen läuft das Backup weiter bzw. neu an; PowerDevil verhindert den automatischen Ruhezustand bei Inaktivität während eines Laufs · Test: manuell (→ manual-acceptance.md)

### M3-S10 Nachholen
- **AC-M3-10-1** (§5.2, NFR-11) – *Gegeben* überfälliges Backup, *wenn* der Daemon startet und die Laufbedingungen erfüllt sind, *dann* (a) genau ein Nachhol-Lauf, und (b) Benachrichtigung «überfällig» · Test: Unit + D-Bus (Notification-Fake)
- **AC-M3-10-2** (§5.2, negativ) – *Gegeben* überfälliges Backup, Laufbedingung beim Daemon-Start nicht erfüllt, *dann* (a) `ausstehend` (Laufbedingung), und (b) kein Lauf · Test: Unit + D-Bus (Fake)
- **AC-M3-10-3** (§5.1) – *Gegeben* Lauf mit Status `läuft` in der Historie, *wenn* der Daemon startet, *dann* setzt er ihn auf `unterbrochen`, bevor er neue Läufe plant · Test: Unit

### M3-S11 Paket-Update
- **AC-M3-11-1** (E11) – *Gegeben* Daemon läuft aus Binary im Temp-Verzeichnis, *wenn* die Datei per `rename` ersetzt wird und kein Lauf aktiv ist, *dann* (a) startet er sich mit dem neuen Binary neu · Test: Integration (Temp-Verzeichnis)
- **AC-M3-11-2** (E11) – *Gegeben* laufendes Backup, *wenn* das Binary ersetzt wird, *dann* (a) läuft das Backup zu Ende, und (b) erfolgt der Neustart danach · Test: Integration
- **AC-M3-11-3** (E11, negativ) – *Gegeben* andere Datei im selben Verzeichnis ersetzt, *dann* (a) kein Neustart · Test: Integration
- **AC-M3-11-4** (E11, Build-Stand) – *Gegeben* der Daemon startet (nach einem Paket-Update schreibt der neu gestartete Prozess dieselbe Zeile), *dann* nennt seine Startzeile im Journal (a) die Paketversion, (b) den Git-Hash des gebauten Stands, (c) dazu `-dirty`, wenn beim Build versionierte Quelldateien geändert waren, (d) `unknown` statt des Hashs, wenn ohne `git` oder ausserhalb eines eigenen Checkouts gebaut wurde, und (e) die PID · Test: Integration

### M3-S12 Fortschrittsanzeige in Plasma
- **AC-M3-12-1** (FR-53) – *Gegeben* startender Backup-Lauf, *wenn* der Daemon ihn anmeldet, *dann* (a) erhält der `JobViewServerV2`-Fake `requestView` mit der Desktop-ID von Kestic, (b) erhält er danach Fortschritt (Prozent), und (c) erhält er beim Ende den Abschluss mit Status · Test: D-Bus (Fake)
- **AC-M3-12-2** (FR-53, FR-21) – *Gegeben* laufender Lauf mit Fortschrittsanzeige in Plasma, *wenn* dort abgebrochen, *dann* (a) endet der Lauf mit `abgebrochen` · Test: D-Bus (Fake) + manuell

### M3-S13 Zeitplangesteuerte Quellen-Prüfung
- **AC-M3-13-1** (FR-29, P36) – *Gegeben* Quellentyp «Laufwerk», Datenträger nicht eingehängt, *wenn* der Termin 12:00 erreicht wird, *dann* (a) kein restic-Prozess, (b) `ausstehend` (Quelle nicht verfügbar), und (c) keine Benachrichtigung · Test: Unit (simulierte Uhr, mountinfo-Fake)
- **AC-M3-13-2** (FR-29) – *Gegeben* dieser Status, *wenn* UDisks2 das Einhängen mit passender UUID meldet, *dann* startet der Lauf ohne Warten auf den nächsten Termin · Test: D-Bus (UDisks2-Fake)
- **AC-M3-13-3** (FR-29, negativ) – *Gegeben* derselbe Status, *wenn* ein anderer Datenträger (andere UUID) eingehängt wird, *dann* bleibt `ausstehend` · Test: D-Bus (UDisks2-Fake)
- **AC-M3-13-4** (FR-29, Glossar «überfällig») – *Gegeben* «täglich 12:00», Quelle über zwei Termine nicht verfügbar, *dann* (a) mit dem zweiten verpassten Termin genau eine Benachrichtigung «überfällig», und (b) vorher keine · Test: Unit (simulierte Uhr) + D-Bus (Notification-Fake)
- **AC-M3-13-5** (FR-29) – *Gegeben* Quellentyp «Ordner», Quellpfad fehlt, *wenn* der Termin erreicht wird, *dann* (a) `ausstehend` (Quelle nicht verfügbar), und (b) prüft jeder weitere Termin erneut · Test: Unit (simulierte Uhr)

### M3-S14 GUI-Editoren für Zeitplan, Laufbedingungen, Pause und Wartung
- **AC-M3-14-1** (FR-03, P35) – *Gegeben* neues Backup, *wenn* der Zeitplan-Editor öffnet, *dann* (a) «täglich 12:00» vorausgewählt, und (b) wählbar «täglich um», «an Wochentagen um», «alle n Tage ab Datum» · Test: GUI-Smoke (Screenshot)
- **AC-M3-14-2** (FR-03, negativ) – *Gegeben* «an Wochentagen» ohne Wochentag bzw. «alle 0 Tage», *dann* (a) Speichern gesperrt, und (b) Feld markiert · Test: GUI-Smoke + Unit
- **AC-M3-14-3** (FR-13) – *Gegeben* Laufbedingungen-Editor eines neuen Backups auf einem Netzwerk-Ziel (bei lokalem Ziel: nur «nur am Strom» und ein Hinweis, Entscheid 2026-11-20), *dann* (a) «nur am Strom» und «nur mit Netzwerk» aktiv, und (b) wählbar «nicht bei getakteter Verbindung» und «nur in Netzwerk …» (Auswahl aus bekannten Verbindungen) · Test: GUI-Smoke (offscreen + Screenshot)
- **AC-M3-14-4** (FR-13a) – *Gegeben* Pause-Menü, *dann* (a) «1 Stunde», «3 Stunden», «bis morgen», «bis ich fortsetze», und (b) je pro Backup und global · Test: GUI-Smoke (offscreen + Screenshot)
- **AC-M3-14-5** (FR-45, FR-11, P15) – *Gegeben* Wartungs-Editor eines neuen Ziels, *dann* (a) «Aufräumen: sonntags 12:00» und «Prüfen: erster Sonntag im Monat 12:00», und (b) lösen Änderungen am Aufräum-Zeitplan die Bestätigung gemäss AC-M3-05-7 aus · Test: GUI-Smoke (offscreen + Screenshot)
- **AC-M3-14-6** (NFR-09) – *Gegeben* jeder der vier Editoren, *wenn* nur Tastatur, *dann* (a) alle Felder erreichbar, und (b) jedes Bedienelement mit zugänglichem Namen · Test: GUI-Smoke (Fokus-Reihenfolge, Accessible-Namen) + manuell
- **AC-M3-14-7** (F14) – *Gegeben* Änderung in einem Editor, *wenn* gespeichert, *dann* (a) per D-Bus an den Daemon, (b) aktualisiert der Daemon Config und Zeitplan, und (c) stimmt der nächste Termin in der Übersicht damit überein · Test: D-Bus (dbus-run-session)
- **AC-M3-14-8** (U18) – *Gegeben* geänderter Zeitplan bzw. geänderte Laufbedingungen, *wenn* `[Speichern]`, *dann* (a) `UpdateBackup` per D-Bus, (b) validiert `core`, (c) erst danach in `config.toml`, und (d) vorher unverändert mit Zwischenstand nur in der GUI · Test: D-Bus (dbus-run-session)
- **AC-M3-14-9** (U18) – *Gegeben* ungespeicherte Änderungen im Zeitplan- bzw. Laufbedingungen-Editor, *wenn* verlassen, *dann* (a) Rückfrage «Änderungen speichern?» mit `[Speichern]` und `[Verwerfen]`, und (b) lässt «Verwerfen» `config.toml` unverändert · Test: GUI-Smoke

## Exit-Kriterien

- Ein Backup läuft eine Woche lang nach Zeitplan auf dem Laptop (parallel zu M4/M5 möglich), inkl. Akku/Netzwechsel und Suspend (manuelle Abnahme).
- `just verify` grün; Scheduler-Tests mit simulierter Uhr.
- Manuelle Abnahme: Deckel zu während Lauf, PowerDevil/Portal-Inhibit, Hotspot.
