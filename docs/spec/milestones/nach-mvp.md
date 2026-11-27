# Nach MVP

Teil von [backlog.md](../backlog.md).

Noch nicht in Meilensteine geschnitten. Reihenfolge innerhalb der Gruppen offen.

## Phase 2 (P2)

| ID | Anforderung |
| --- | --- |
| FR-14 | `restic copy` zwischen Zielen; dazu ein umgezogenes Repo mit neuer Repo-ID übernehmen (Abgleich der Snapshots über `original`, Bestätigung, neues Kit; früher M5-S09) |
| FR-16 | Ausschluss-Regeln rückwirkend anwenden (`rewrite --forget`) |
| FR-20 | Pre/Post-Hooks |
| FR-22 | Trigger «externe Disk eingesteckt» |
| FR-23 | Import bestehender Konfiguration |
| FR-27 | Linter für Ausschluss-Regeln |
| FR-32 | Ausschluss-Regeln kopieren / Vorlagen |
| FR-40 | `check --read-data-subset` |
| FR-41 | Key-Management im GUI |
| FR-43 | Bandbreitenlimits |
| FR-51 | Wöchentliche Prüfung auf neue restic-Releases mit Hinweis im Tray (früher M5-S08); im MVP aktualisiert `[Jetzt aktualisieren]` |
| FR-44 | Fremde Snapshots einem Backup zuordnen |
| FR-46 | Snapshot dauerhaft behalten |
| FR-47 | Expertenoptionen für `prune` |
| FR-50 | Diagnose-Export |
| FR-54 | Fortschritt in der Taskleiste |
| FR-55 | Keine Systembenachrichtigung bei aktivem Fenster |
| – | SFTP mit SSH-Schlüssel, der eine Passphrase hat, ohne ssh-agent: Passphrase im Passwortspeicher (im MVP nur Schlüssel ohne Passphrase oder ssh-agent, [architecture.md](../architecture.md) «Secrets») |
| – | Flatpak ([architecture.md](../architecture.md) «Packaging», «Secrets») |
| – | Projekt-Website auf GitHub Pages (inkl. automatisch generierter & aktualisierter GUI-Screenshots aus den QML-Offscreen-Smoke-Tests, optional über `kestic.daester.net` / GitHub Pages) |
| – | «Tastenkürzel einrichten…» im Hauptmenü: Editor für alle Tastenkürzel ([ui/stil-und-texte.md](../ui/stil-und-texte.md) §13.1) |
| – | Wiederherstellen direkt in einen Netzwerkordner (`smb://` u. a.) aus dem System-Dateidialog; nicht über kio-fuse, das jede Datei erst lokal puffert (doppelter Platz) und bei Netzfehlern schlecht abbricht (Entscheid auf #154) |

## Luxus

| ID | Anforderung |
| --- | --- |
| FR-09 | Auto-Exclude-Presets mit Ausnahmen |
| FR-25 | Snapshot in Dolphin browsen |
| FR-26 | Grössen-Explorer |
| – | «Frühere Versionen…» im Kontextmenü von Dolphin: zeigt die Versionen einer Datei über alle Snapshots (gleiche Fassungen zusammengefasst) und bietet «Diese Version speichern unter…» über `restic dump`; überschreibt nie still |

## Zuletzt (P3)

| ID | Anforderung |
| --- | --- |
| FR-48 | Danger Zone – Backup komplett löschen |
| FR-52 | Daemon als eigenes Binary ohne Qt |

## Zurückgestellt aus dem M6-Kickoff (2026-11-26)

Nach M6 bzw. nach `v0.6.0` geplant; Details im jeweiligen Issue.

| Issue | Inhalt |
| --- | --- |
| #195 | SFTP: weniger Host-Schlüssel-Prüfungen pro Lauf, gleichzeitige Erreichbarkeitsprüfungen (erst messen) |
| #194 | Refactorings und Tests aus den Resten von M5 |
| #211 | Zeitplan: häufigere Optionen als täglich (stündlich, alle n Stunden) |
| #216 | i18n: Deutsch mit Eszett in `po/de`, Schweizer Deutsch als `de_CH` erzeugen, neue Sprache als Pull Request mit einer Datei |
| #217 | Website: Englisch, Deutsch und Schweizer Deutsch nach Browser-Sprache |
| #222 | UI-Spec-Lücken für die Zeit nach 1.0 (Hinweise auf veraltete Daten, Datums-Tooltips, Verlaufs-Chip, Tray ausblenden, Log folgen) |
| #83 | Spec bereinigen und ins Englische übersetzen |
| #207 | CA- und Client-Zertifikatsfelder für HTTP-Ziele (bis dahin: Zertifikatsspeicher des Systems, Handbuchkapitel «Eigener Server mit eigenem Zertifikat», klare Meldung, AC-M6-11-4 und AC-M6-13-5) |
| – (FR-35) | Kit **drucken**: `v0.6.0` zeigt das Kit und exportiert es als Datei |
| – (F2) | AC für «übrige Fehler» nachtragen: Platte voll und unbekannte Exit-Codes sperren den fälligen Termin nicht dauerhaft (gebaut und getestet, Nachvollziehbarkeit) |
| – (P5) | AC für die Fortschrittsregeln nachtragen (gebaut und getestet, Nachvollziehbarkeit) |
| – (FR-38) | «Erwartung für das erste Backup kommunizieren»: Dauer und Fortsetzen nach Unterbrechung (die Grössenschätzung hat AC-M6-01-5) |
| – (FR-14) | Erkennung umgezogener Repos beim Bearbeiten einer Verbindung (Abgleich der bekannten Snapshots, Hinweis «Umgezogenes Repository erkannt», Zustand «Kit fehlt» nach dem Umzug, Feld «Passwort des neuen Repositorys»): im MVP vergleicht Kestic nur die Repo-ID (FR-56) |
| – (U-Tasten) | Ctrl+R und F5 (Aktualisieren): die GUI aktualisiert über Signale des Daemons |

## Zurückgestellt aus dem UX-Review (M6-S24, #107)

Entscheide des Maintainers vom 2026-11-27 (Kommentar auf #107); die Kennungen verweisen auf den Kommentar «consolidated decisions».

| Kennung | Inhalt |
| --- | --- |
| B13 (ii) | «Mit restic prüfen» bei «Pfad testen»: `backup --dry-run -vv` auf genau diesen Pfad, damit auch Marker-Dateien und Grössenregeln geprüft sind (in M6 nennt «Pfad testen» nur, was es nicht prüft, AC-M6-47-3) |
| B13 (iii) | Ziele, die nur Anhängen erlauben (append-only, unveränderlich): ein Schalter, der `forget` und `prune` auslässt (in M6 eine bekannte Einschränkung im Handbuch, AC-M6-13-7) |
| B13 (iv) | `--iexclude` je Regel (Gross- und Kleinschreibung ignorieren); mehrere Marker-Dateien je Backup |
| B15 | Grund verpasster Termine (Rechner aus, Ruhezustand, Ziel nicht erreichbar, Laufbedingungen) auf der Karte und in der Benachrichtigung «überfällig» («4 Termine verpasst – Rechner war aus»); braucht eine Aufzeichnung je Termin im Planer (in M6 nur «Zuletzt erfolgreich», S42) |
| B19 | Filter «Gelöschte Dateien» im Bereich «Wiederherstellen»: vergleicht den neuesten Snapshot mit der Platte und listet, was dort fehlt |
