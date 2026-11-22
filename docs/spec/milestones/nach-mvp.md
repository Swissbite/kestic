# Nach MVP

Teil von [backlog.md](../backlog.md).

Noch nicht in Meilensteine geschnitten. Reihenfolge innerhalb der Gruppen offen.

## Phase 2 (P2)

| ID | Anforderung |
| --- | --- |
| FR-14 | `restic copy` zwischen Zielen |
| FR-16 | Ausschluss-Regeln rückwirkend anwenden (`rewrite --forget`) |
| FR-20 | Pre/Post-Hooks |
| FR-22 | Trigger «externe Disk eingesteckt» |
| FR-23 | Import bestehender Konfiguration |
| FR-27 | Linter für Ausschluss-Regeln |
| FR-32 | Ausschluss-Regeln kopieren / Vorlagen |
| FR-40 | `check --read-data-subset` |
| FR-41 | Key-Management im GUI |
| FR-43 | Bandbreitenlimits |
| FR-44 | Fremde Snapshots einem Backup zuordnen |
| FR-46 | Snapshot dauerhaft behalten |
| FR-47 | Expertenoptionen für `prune` |
| FR-50 | Diagnose-Export |
| FR-54 | Fortschritt in der Taskleiste |
| FR-55 | Keine Systembenachrichtigung bei aktivem Fenster |
| – | SFTP mit SSH-Schlüssel, der eine Passphrase hat, ohne ssh-agent: Passphrase im Passwortspeicher (im MVP nur Schlüssel ohne Passphrase oder ssh-agent, [architecture.md](../architecture.md) «Secrets») |
| – | Flatpak ([architecture.md](../architecture.md) «Packaging», «Secrets») |
| – | Release-Prozess: SemVer, Changelog (Q7) |
| – | Projekt-Website auf GitHub Pages (inkl. automatisch generierter & aktualisierter GUI-Screenshots aus den QML-Offscreen-Smoke-Tests, optional über `kestic.daester.net` / GitHub Pages) |
| – | «Tastenkürzel einrichten…» im Hauptmenü: Editor für alle Tastenkürzel ([ui/stil-und-texte.md](../ui/stil-und-texte.md) §13.1) |

## Luxus

| ID | Anforderung |
| --- | --- |
| FR-09 | Auto-Exclude-Presets mit Ausnahmen |
| FR-25 | Snapshot in Dolphin browsen |
| FR-26 | Grössen-Explorer |

## Zuletzt (P3)

| ID | Anforderung |
| --- | --- |
| FR-48 | Danger Zone – Backup komplett löschen |
| FR-52 | Daemon als eigenes Binary ohne Qt |
