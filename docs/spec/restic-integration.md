# restic-Integration

Besitzer aller restic-Fakten, die Kestic nutzt (E14). Geprüft am 2026-11-15 mit restic 0.19.1 gegen Temp-Repos bzw. in der [restic-Doku](https://restic.readthedocs.io/en/stable/), sofern nicht anders vermerkt.

## Folgen für Kestic

Folgen für Kestic:

- **Ausschluss-Regeln:** Globs, kein Regex; Env-Variablen nur in Exclude-Dateien → Kestic schreibt Regeln als Exclude-Datei pro Backup. Negation wirkt nicht unter ganz ausgeschlossenen Ordnern → Linter-Regel (FR-27) und Hinweis im Editor (FR-02).
- **Integration:** CLI mit `--json`, Parser tolerant (NFR-10), Exit-Codes explizit abbilden (FR-37).
- **Locks (P41):** Vorschauen (`forget`/`prune --dry-run`) mit `--no-lock`; veralteter Lock entsteht nur bei hartem Abbruch (SIGKILL, Absturz), Tests legen ihn so an. Backups pro Ziel parallel möglich, `forget`/`prune`/`check` exklusiv → Warteschlange pro Ziel ([domain-model.md](domain-model.md)).
- **Leerer Mountpoint** ergibt Exit 0 mit leerem Snapshot → Vorabprüfung (FR-29).
- **Keys:** Master-Key ändert sich nie → Wiederherstellungs-Kit als eigener, widerrufbarer Key (FR-35).
- **Backends & Releases:** Grundlage für Onboarding ([architecture.md](architecture.md)) und Download inkl. Signaturprüfung (FR-33).
- **Mindestversion 0.18.0** (Fehler als JSON, Exit-Codes 10–12, `check --json`). Ubuntu 24.04 und Derivate (inkl. TUXEDO OS, KDE neon) liefern 0.16.4 → FR-33 ist MVP.
- **Getestete Versionen:** 0.18.0 (Mindestversion) und 0.20.0 (aktuell, `RESTIC_LATEST_VERSION` in `scripts/bootstrap.sh`); `just verify` führt die Integrationstests gegen beide aus.
- **Fehler älterer restic-Versionen** behebt restic, nicht Kestic. Ein Beispiel ist restic/restic#22180, behoben in 0.20.0: Ein präparierter Snapshot mit Symlink und Ordner gleichen Namens lässt `restore` ausserhalb des Ziels schreiben. Kestic fängt solche Fehler nicht ab; jede unterstützte Version ab 0.18.0 sichert und stellt wieder her, ohne Sperre, Warnung oder Download-Pflicht. Ist die verwendete Version älter als die aktuell getestete, zeigt die Einstellung «restic-Quelle» nur den Zusatz «(veraltet)» (AC-M4-05-22).
- **Fortschritt beim Sichern:** Die Prozentzahl gilt erst, wenn restic mit der Suche fertig ist; bis dahin zeigt Kestic keine Prozentzahl (Abschnitt «Fortschritt beim Sichern»).
- **Ergebnis eines Wartungslaufs:** entfernte Snapshots aus `forget --json`, freigegebener Platz ab restic 0.20.0 aus `prune --json`, bei älteren Versionen aus der Textzeile `total prune:` (einzige Ausnahme von «nur JSON», tolerant ausgewertet), Fehlerzahl aus `check --json` (Abschnitt «Wartung»).
- **Source of Truth (E14):** Dieses Dokument ist Besitzer aller restic-Fakten, die Kestic nutzt (Exit-Codes, Ausschluss-Semantik, Versionen), jeweils mit Datum.

## Fakten

### Ausschluss-Muster

- Globs nach Go `filepath.Match` plus `**` als ganze Pfadkomponente, **kein Regex** ([040_backup](https://restic.readthedocs.io/en/stable/040_backup.html)).
- Ohne führenden `/` matcht ein Muster auf jeder Ebene: `.git` schliesst alle `.git`-Ordner aus.
- Env-Variablen werden in Exclude-**Dateien** expandiert (`os.ExpandEnv`, `$$` für ein literales `$`, `~` wird nicht expandiert), **nicht** in `--exclude` auf der Kommandozeile.
- Negation `!` schliesst wieder ein, aber **nicht unterhalb eines ganz ausgeschlossenen Ordners**: `dir` + `!dir/inner` → nichts gesichert; `dir/*` + `!dir/inner` → `inner` gesichert.
- Weitere Optionen: `--iexclude`, `--iexclude-file`, `--exclude-caches` (CACHEDIR.TAG), `--exclude-if-present <file>[:header]`, `--exclude-larger-than`, `--one-file-system`, `--skip-if-unchanged`.

### Credentials

- `RESTIC_REST_USERNAME`/`RESTIC_REST_PASSWORD` → REST-Zugang muss nicht in die Repo-URL.
- `RESTIC_PASSWORD_COMMAND` holt das Repo-Passwort aus einem beliebigen Programm (Secret-Store, Passwortmanager). Für Backend-Credentials gibt es keine `_COMMAND`-Variante.
- S3: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, optional `AWS_DEFAULT_REGION`; Repository `s3:<endpoint mit Schema>/<bucket>[/<pfad>]`, z. B. `s3:https://s3.example.com:9000/backup`. Vor dem Setzen eigener Werte entfernt Kestic jede geerbte Variable, deren Name mit `AWS_` oder `RESTIC_AWS_` beginnt (Zugangsdaten, Profil, Rolle, Region), aus der Umgebung von restic, bei Läufen wie beim Verbindungstest: Zugangsdaten kommen nur aus dem Passwortspeicher (FR-08). Fehlt der Bucket, meldet der Verbindungstest das mit Namen («Bucket ‹name› gibt es noch nicht; Kestic legt ihn beim Anlegen des Ziels an.», Hinweis `bucket_missing`); `init` legt ihn beim Anlegen an. Der Pfad im Bucket enthält keine Leerzeichen, Anführungszeichen, `#`, `?` oder `%`: restic liest die Adresse als URL und wiederholt sie ungequotet in den Meldungen, die Kestic auswertet; das Formular weist darauf hin.
- SFTP: Repository `sftp://<benutzer>@<host>:<port>/<pfad>`; ein Schrägstrich nach dem Host heisst «relativ zum Heimordner», ein absoluter Pfad braucht zwei (`sftp://u@h:22//srv/repo`) **[angenommen, prüft M5-S01 an der restic-Doku und im Test]**. Den Port gibt restic selbst an ssh weiter. ssh-Optionen über `-o sftp.args="…"` (Kestics Werte: [architecture.md](architecture.md) «SFTP und Host-Schlüssel»).
- Kestic setzt `RESTIC_PASSWORD_COMMAND`, `RESTIC_REPOSITORY` und die Backend-Variablen selbst. Benutzer-Variablen des generischen Formulars, die damit kollidieren (`RESTIC_PASSWORD*`, `RESTIC_REPOSITORY*`) oder den Prozess umlenken (`PATH`, `HOME`, `LD_*`), weist `core` ab (AC-M5-03-7).

### Integration & Locks

- Code liegt unter `internal/`, keine dokumentierte Library-API. Dokumentierte Schnittstelle ist die JSON-Ausgabe (`--json`, [075_scripting](https://restic.readthedocs.io/en/stable/075_scripting.html)): rückwärtskompatibel gedacht, neue Felder/Message-Typen jederzeit möglich, `prune` erst ab 0.20.0 mit JSON (davor Textausgabe, Abschnitt «Wartung»), unbekannte Exit-Codes = Fehler.
- `backup` nimmt einen nicht-exklusiven Lock (mehrere Backups ins selbe Repo gleichzeitig möglich); `forget`, `prune`, `check` nehmen einen exklusiven und brechen bei laufendem Backup mit Exit 11 ab.
- Lock-Verhalten (geprüft 2026-11-15, restic 0.19.1): SIGTERM → restic räumt auf und gibt den Lock frei; SIGKILL → Lock bleibt liegen, `restic unlock` entfernt ihn (Prozess auf demselben Host läuft nicht mehr). Ein so liegengebliebener **exklusiver** Lock (z. B. von `check`) blockiert danach `backup` und `forget` mit Exit 11, bis `unlock` läuft. Die Lock-Datei enthält System-Hostname, Benutzer, PID und `exclusive`; `RESTIC_HOST`/`--host` ändern nur den Snapshot-Host, nicht den Lock.
- `forget --dry-run` und `prune --dry-run` nehmen ebenfalls einen Lock und scheitern während eines Backups mit Exit 11; mit `--no-lock` laufen sie. `restore --dry-run` läuft während eines Backups.

### Exit-Codes & fehlende Quellen

- 0 ok, 1 Fehler, 3 unvollständig, 10 Repo fehlt, 11 Lock, 12 falsches Passwort (Versionen siehe Abschnitt «Versionen»).
- Eine von mehreren Quellen fehlt → Snapshot wird erstellt, Exit 3. Alle fehlen → Exit 1, kein Snapshot.
- **Falle:** leeres Verzeichnis, z. B. nicht gemounteter Mountpoint → Exit 0, Snapshot mit 0 B. Vor dem Backup selbst prüfen.

### Hostname, Gruppierung & Tags

- Jeder Snapshot speichert den Hostnamen (`--host` überschreibt, Default `$RESTIC_HOST` bzw. System-Hostname).
- `backup` (Parent-Suche) und `forget` gruppieren per Default nach `host,paths`. Nach einem Hostname-Wechsel: `no parent snapshot found, will read all files` (vollständiges Neueinlesen) und `forget` bildet getrennte Gruppen → alte Snapshots werden nie mehr bereinigt.
- `forget` gruppiert per Default **nicht** nach Tags.
- Robustes Muster: pro Snapshot-Reihe ein eindeutiger Tag und `--group-by tags` für `backup` und `forget`.

### Repo-Identität und Umzug

_Geprüft 2026-11-15, restic 0.19.1._

- Jedes Repo hat eine eindeutige **Repo-ID** (`restic cat config`, Feld `id`).
- Wird das Repo-Verzeichnis kopiert oder ist es unter neuer URL bzw. neuem Pfad erreichbar (z. B. `rsync` auf neue Disk, NAS mit neuer IP), bleiben **Repo-ID und Snapshot-IDs gleich**.
- `restic copy` in ein neues Repo: **neue Repo-ID und neue Snapshot-IDs**; Tags bleiben erhalten, jeder kopierte Snapshot trägt im Feld `original` die ID des Quell-Snapshots. Ein folgendes `backup` mit `--group-by tags` findet dort seinen Parent. Die Keys sind die des Ziel-Repos, alte Passwörter bzw. Recovery-Keys gelten dort nicht.
- Mehrfaches `copy` (A → B → C): `original` in C zeigt weiterhin auf den Snapshot in A (restic überschreibt ein gesetztes `original` nicht). Snapshots, die nie kopiert wurden, haben kein `original`; für Abgleiche daher «`original`, sonst `id`» verwenden.

### Aufbewahrung (`forget`)

- `--keep-last/-hourly/-daily/-weekly/-monthly/-yearly n` (je auch `unlimited`), `--keep-within <dauer>`, `--keep-within-hourly/-daily/-weekly/-monthly/-yearly <dauer>` (z. B. `1y5m7d2h`), `--keep-tag`.
- Zeiträume zählen **ab dem neuesten Snapshot**, nicht ab jetzt.
- `--keep-daily 7` = die 7 jüngsten Tage **mit Snapshots** (bei Lücken weiter zurück); `--keep-within-daily 7d` = je einer pro Tag der letzten 7 Kalendertage.
- Regeln gelten pro Gruppe und sind kombinierbar; reichen die Snapshots für eine Zeitraum-Regel nicht, bleibt der älteste erhalten.
- `forget --dry-run` zeigt pro Snapshot die Gründe («Reasons»). `--unsafe-allow-remove-all` erlaubt das Löschen aller Snapshots einer Gruppe.

### Keys & Master-Key

- Ein Repo kann mehrere Keys haben (`key add/list/passwd/remove`); jeder schützt denselben **Master-Key**, der sich nie ändert ([design.rst](https://github.com/restic/restic/blob/master/doc/design.rst)).
- Test: Zusatz-Key anlegen → Passwort des Haupt-Keys ändern → Zusatz-Key öffnet das Repo weiterhin, altes Passwort liefert Exit 12, Master-Key identisch.
- **Folge:** Ein Passwortwechsel sperrt niemanden aus, der den Master-Key schon hatte; das geht nur mit neuem Repo + `copy`. Ein separater, widerrufbarer Recovery-Key ist darum nicht unsicherer als ein Kit, das ans Passwort gebunden ist.

### Backends

- Nativ: Local, SFTP (braucht `ssh`), REST-Server, S3 und kompatible, Swift, B2, Azure, GCS. rclone braucht das `rclone`-Binary.
- Doku-Reihenfolge: Local, SFTP, REST, S3, MinIO, S3-kompatibel, Wasabi, Alibaba OSS, Swift, B2, Azure, GCS, rclone ([030_preparing_a_new_repo](https://restic.readthedocs.io/en/stable/030_preparing_a_new_repo.html)).
- Doku rät von Repos auf CIFS/SMB-Freigaben ab (Kompatibilitätsprobleme älterer Kernel).

### Releases & Verifikation

- Release-Assets: `restic_<v>_linux_amd64.bz2`, `restic_<v>_linux_arm64.bz2`, `SHA256SUMS`, `SHA256SUMS.asc`.
- Signatur-Key-Fingerprint: `CF8F 18F2 8445 7597 3F79 D4E1 91A6 868B D3F7 A907` ([020_installation](https://restic.readthedocs.io/en/stable/020_installation.html)).
- `self-update` nur bei offiziellen Release-Builds. Kestic nutzt es nicht: Updates des App-verwalteten restic laufen über denselben Download wie die Erstinstallation, damit es einen Vertrauensanker und einen testbaren Weg gibt ([architecture.md](architecture.md) «restic-Bereitstellung»).
- **[angenommen, prüft M5-S06]** `releases/latest` der GitHub-API nennt nur veröffentlichte Releases ohne Entwürfe und Vorabversionen; Asset-Downloads leiten auf `*.githubusercontent.com` weiter; `gpgv --status-fd 1` meldet eine gültige Signatur als `[GNUPG:] VALIDSIG <fingerprint> …`. M5-S06 prüft das und trägt hier Datum und Version nach.

### Fortschritt beim Sichern (`backup --json`)

_Geprüft 2026-11-20 im Quellcode von restic 0.18.0 und 0.19.1 (`internal/ui/backup/progress.go`, `json.go`; Logik in beiden Versionen gleich) und an einem echten inkrementellen Lauf (0.19.1, 28 s, rund 288 GB, 166 000 Dateien)._

- restic sucht die Dateien und sichert gleichzeitig. `total_files` und `total_bytes` der `status`-Zeilen wachsen, bis die Suche fertig ist. `percent_done` ist `bytes_done / total_bytes` und kann deshalb fallen (beobachtet: 100 % → 88,6 % → 100 %).
- `seconds_remaining` setzt restic erst, wenn die Suche fertig ist, und nur bei einer Rate über 1 KiB/s. Der Wert ist eine ganze Zahl und fehlt bei 0 (`omitempty`). Fehlt er, läuft also die Suche, oder es bleibt weniger als 1 s, oder die Rate ist zu klein. Im beobachteten Lauf war die Suche nach 25 s fertig, `seconds_remaining` kam trotzdem nie.
- Ein eigenes Signal «Suche fertig» (`verbose_status` mit `action: scan_finished`) gibt es nur mit `-vv`. Dann schreibt restic eine Zeile pro Datei, auch für unveränderte.
- `data_added` steht nur in `summary`, nicht in `status`.

Folgen für Kestic:

- Ein Lauf ist in der Phase `scanning`, bis die erste `status`-Zeile mit `seconds_remaining > 0` kommt. Danach ist er bis zum Ende in `scan_finished`; ein späteres Fehlen von `seconds_remaining` schaltet nicht zurück.
- In `scanning` zeigt Kestic keine Prozentzahl, sondern die durchgesehene Menge (`bytes_done`, `files_done`).
- In `scan_finished` zeigt Kestic `percent_done`, nie kleiner als der zuletzt gezeigte Wert, und die Restzeit aus `seconds_remaining`, wenn vorhanden.
- Endet ein Lauf, bevor `seconds_remaining` kommt (häufig bei schnellen inkrementellen Läufen), bleibt er bis zum Ende in `scanning`. Die neu geschriebene Menge steht nach dem Lauf aus `summary.data_added` auf der Karte.
- Normale Läufe laufen ohne `-vv` (zu viel Ausgabe); `-vv` nur für den Dry-Run.

### Wartung (`forget`, `prune`, `check`)

_Geprüft 2026-11-21 mit restic 0.18.0 und 0.19.1 an echten Temp-Repos; Ausgaben als Fixtures in `core/tests/fixtures/` (`forget_remove_two_restic_*.json`, `prune_restic_*.txt`, `check_summary_restic_*.json`). `prune --json` geprüft 2026-11-22 mit restic 0.20.0 an echten Temp-Repos (mit und ohne Umpacken), Fixture `prune_restic_0.20.0.json`._

- `forget --json` schreibt eine Zeile mit einem JSON-Array von Gruppen; jede Gruppe hat `keep`, `remove` (Snapshots, sonst `null`) und `reasons`. Einen freigegebenen Platz nennt `forget` nicht.
- `prune` bis 0.19.x kennt kein JSON: Auch mit `--json` schreibt es dieselbe Textausgabe auf stdout. Die Zusammenfassung enthält die Zeile `total prune: <n> blobs / <Grösse>` (dazu `remaining:` und `unused size after prune:`); die Grösse ist `<n> B` oder eine Zahl mit drei Nachkommastellen und `KiB`, `MiB`, `GiB` oder `TiB` (Faktor 1024), in 0.18.0 und 0.19.1 gleich.
- `prune --json` ab 0.20.0 schreibt statt der Textausgabe eine `summary`-Zeile `{"message_type":"summary","blobs":{…},"bytes":{…},"packfiles":{…}}`. `bytes.remove_total` (= `bytes.remove` + `bytes.repack_remove`) ist die Grösse, die die Textzeile `total prune:` nennt, in Bytes; Kestic nutzt nur dieses Feld. Weitere Felder von `bytes` sind u. a. `used`, `unused`, `total`, `repack`, `remaining`, `remaining_unused`.
- `check --json` schreibt eine `summary`-Zeile: `{"message_type":"summary","num_errors":0,"broken_packs":null,"suggest_repair_index":false,"suggest_prune":false}`. Backup-Zusammenfassungen tragen ebenfalls `message_type: "summary"`, aber kein `num_errors`.

Folgen für Kestic:

- Entfernte Snapshots je Backup: Summe der `remove`-Listen; ohne lesbare Ergebniszeile keine Zahl.
- Freigegebener Platz: Ab restic 0.20.0 startet Kestic das echte `prune` mit `--json` und liest `bytes.remove_total` aus der `summary`-Zeile; unbekannte Felder werden ignoriert. Bei 0.18.x und 0.19.x bleibt es ohne `--json`, und Kestic wertet die Zeile `total prune:` tolerant aus (Nachkommastellen abgeschnitten). Die Vorschau (`prune --dry-run --no-lock`) bleibt in allen Versionen ohne `--json`. Fehlt das Ergebnis oder ist es nicht lesbar (andere Version, andere Einheit, fehlendes Feld), gibt es keine Zahl und keinen Fehler; der Lauf bleibt `ok`.
- Prüfung: nur eine `summary`-Zeile mit `num_errors` gilt als Ergebnis von `check`.

### Versionen

| Feature | ab Version |
| --- | --- |
| Negative Excludes `!`, `diff --json` | 0.13.0 |
| Repo-Format v2 / Kompression | 0.14.0 |
| `rewrite` | 0.15.0 |
| `restore --json` | 0.16.0 |
| `RESTIC_REST_USERNAME`/`_PASSWORD` | 0.16.1 |
| `backup --skip-if-unchanged`, `restore --overwrite/--delete/--dry-run`, `ls --ncdu`, `version --json`, Exit 10 / 11 | 0.17.0 |
| Exit 12 | 0.17.1 |
| `check --json`, `tag --json`, fatale Fehler als JSON (`exit_error`) | 0.18.0 |
| Exit 3 auch für fehlende Quellpfade | 0.19.0 |
| `prune --json` | 0.20.0 |

Quellen: [CHANGELOG](https://github.com/restic/restic/blob/master/CHANGELOG.md), [Repology](https://repology.org/project/restic/versions), [Launchpad](https://launchpad.net/ubuntu/+source/restic), [packages.debian.org](https://packages.debian.org/search?keywords=restic).

| Distribution (Stand 2026-11-15) | restic |
| --- | --- |
| Ubuntu 26.10, Debian sid, Fedora 43/44, Arch, openSUSE Tumbleweed | 0.19.1 |
| Ubuntu 26.04 LTS | 0.18.1 |
| Debian 13, openSUSE Leap 15.6 (Backports) | 0.18.0 |
| Ubuntu 24.04 LTS und Derivate (Mint 22.3, KDE neon, Pop!_OS, Zorin OS 18.1, elementary OS 8.1, TUXEDO OS) | 0.16.4 |
| Debian 12 | 0.14.0 |

Debian testing («forky») hat aktuell kein restic, Debian 13 keine Backports.

## Fehlerabbildung (FR-37)

Gerüst; wächst je Meilenstein, vollständig in M6-S11. Unbekannte Exit-Codes gelten als Fehler (NFR-10).

| Fall | Status | Text (DE) | Handlungsoption |
| --- | --- | --- | --- |
| Exit 0 | `ok` | – | – |
| Exit 1 | `Fehler` | nach Meldung eingeteilt: Ziel nicht erreichbar / DNS / Auth / sonst (P32, M1-S15) | je Fall (M1: Anzeige + Rohmeldung; Backoff/Retry folgt ab M3) |
| Exit 3 | `Warnung` | Snapshot unvollständig: Liste der nicht gelesenen Dateien | «zu Ausschluss-Regeln hinzufügen» (F4) |
| Exit 10 | `Fehler` | Repository fehlt | – (M1-S15: Fehler ohne direkte Recovery-Aktion; in «Verbindung bearbeiten» lässt sich bei fehlendem Repo neu initialisieren) |
| Exit 11 | M1: `Fehler`; ab M3: vorübergehend → `ausstehend` + Backoff (`backup`) bzw. `Warnung` + «forget offen» (`forget`) (P2, F2) | Repository gesperrt | ggf. `unlock` bei veraltetem Lock (FR-19, P16) |
| Exit 12 | `Fehler` (dauerhaft, sofort Alert, F2) | «Gespeichertes Passwort passt nicht» | «Gespeichertes Passwort aktualisieren…» ersetzt «Jetzt sichern»; «Trotzdem jetzt sichern» im ⋮-Menü |
| unbekannter Code n | `Fehler` | «Unbekannter Fehler (Exit n)» + Rohmeldung (P6) | – |
| JSON `exit_error` (ab 0.18.0) | wie Exit-Code | Meldung aus `exit_error` | je Fall |
| SFTP: `Host key verification failed` | `Fehler` (dauerhaft, F2) | Kestic prüft selbst nach (`ssh-keygen -F`, `ssh-keyscan`): «Unbekannter Server» bzw. «Der Server meldet sich anders als bisher» (AC-M5-01-4, AC-M5-01-7) | Verbindung prüfen; bei geändertem Schlüssel keine |
| SFTP: `Permission denied (publickey…)` | `Fehler` (dauerhaft, F2) | Anmeldung mit SSH-Schlüssel abgelehnt (`ssh_key_rejected`, AC-M5-01-3) | Hinweis auf `ssh-copy-id` und ssh-agent |
| restic zu alt / nicht gefunden | blockierend | «restic x ist zu alt, mindestens 0.18.0 nötig» bzw. «restic nicht gefunden» (FR-33, P31) | Einstellung «restic-Quelle» |
| Passwortspeicher gesperrt | blockierend | in M6-S11 festzulegen | in M6-S11 festzulegen |

Texte DE/EN und alle weiteren Fälle (DNS, Auth am Backend, Timeout) ergänzt die jeweilige Story; Abschluss M6-S11.
