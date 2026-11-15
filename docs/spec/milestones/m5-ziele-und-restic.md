# M5 Ziele und restic-Bereitstellung

Teil von [backlog.md](../backlog.md).

**Ziel:** Alle restic-Backends sind nutzbar, und Kestic funktioniert auch dort, wo das System-restic zu alt ist (Ubuntu-24.04-Basis).

**Voraussetzung:** M1.

## Anforderungen

FR-01 (alle Ziel-Typen), FR-33 (Download, Umstellen), FR-51, FR-56 · NFR-05 · F20, P21–P24, P31, P32, P42, U18 · [architecture.md](../architecture.md) «Onboarding und Ziel-Typen» (Ziel-Typen)

## Stories

| ID | Story |
| --- | --- |
| M5-S01 | Ziel-Typ SFTP: Formular, Verbindungstest, Hinweis auf SSH-Schlüssel |
| M5-S02 | Ziel-Typ S3-kompatibel: Formular, Credentials im Secret Service |
| M5-S03 | Generisches Formular (URL + Umgebungsvariablen) für B2, Azure, GCS, Swift, rclone |
| M5-S04 | Erkennung «bereit» / «braucht Installation» (z. B. `ssh`, `rclone`), Hinweis mit Paketname, Discover-Link, `apt install` |
| M5-S05 | Externes Laufwerk / USB: Wechseldatenträger erkennen (UDisks2), Hinweis bei SMB/CIFS-Ordner |
| M5-S06 | restic herunterladen: Architektur (amd64/arm64), `SHA256SUMS` + `.asc` prüfen (Fingerprint im Code), atomar ersetzen, nie während Lauf (FR-33) |
| M5-S07 | Einstellung «restic-Quelle» (Automatisch / System / App-verwaltet / eigener Pfad), Anzeige beider Versionen, Umstellen zurück auf System, App-Binary löschen (FR-33) |
| M5-S08 | Prüfung auf neue Releases, höchstens wöchentlich, abschaltbar, Installation nach Klick (FR-51, F20) |
| M5-S09 | Verbindung bearbeiten: Repo-ID-Abgleich für alle Ziel-Typen (FR-56, P42, U18); ACs AC-M5-05-5 bis -7 und AC-M5-09-1 bis -9 |

## Akzeptanzkriterien

Test-Isolation gemäss [test-strategy.md](../test-strategy.md). Zusätzlich: kein echter Netzzugriff (Release-Mock), simulierte Uhr aus `core` (M1, P36); restic-Attrappen unter 0.17 verhalten sich wie das echte Binary (kein `version --json`, P31); SSH-Server im Test-Setup (`sshd` oder Container) mit `known_hosts`-Pfad im Tempdir.

### M5-S01 Ziel-Typ SFTP
- **AC-M5-01-1** (FR-01, P24) – *Gegeben* SSH-Server mit Schlüssel-Login, `ssh` im `PATH`, *wenn* im SFTP-Formular Host, Benutzer, Pfad angegeben und «Verbindung testen» gewählt wird, *dann* meldet Kestic Erfolg, erkennt «kein Repo vorhanden» und legt das Ziel per `init` an; jeder ssh-Aufruf von restic enthält `-o UserKnownHostsFile=<Tempdir-Pfad>` (über `sftp.args`) · Test: Integration (SSH)
- **AC-M5-01-2** (FR-01) – *Gegeben* derselbe Server mit bestehendem Repo, *wenn* der Test läuft und das Passwort stimmt, *dann* wird verbunden statt initialisiert, ssh nutzt nur den übergebenen `known_hosts`-Pfad · Test: Integration (SSH)
- **AC-M5-01-3** (FR-01) – *Gegeben* kein passender Schlüssel, *wenn* der Test läuft, *dann* verständliche Meldung mit Hinweis auf SSH-Schlüssel, kein Ziel angelegt · Test: Integration (SSH)
- **AC-M5-01-4** (FR-01, P24) – *Gegeben* unbekannter Host-Key, *wenn* der Test läuft, *dann* zeigt Kestic den Fingerprint und fragt; erst nach Bestätigung steht der Key unter dem übergebenen Pfad (`-o UserKnownHostsFile=<Pfad>` über `sftp.args`), bei Ablehnung bleibt die Datei unverändert und kein Ziel entsteht; das echte `~/.ssh/known_hosts` bleibt immer unberührt (OpenSSH löst `~` über `/etc/passwd` auf, nicht über `$HOME`) · Test: Integration (SSH)
- **AC-M5-01-5** (FR-30, P32) – *Gegeben* SFTP-Ziel mit gestopptem `sshd`, das der Verfügbarkeitsstatus (M1-S23) noch als erreichbar führt, *wenn* «Jetzt sichern» geklickt wird, *dann* scheitert der Vorab-Check mit `ConnectTimeout=10`, restic startet nicht, spätestens nach 15 s kommt `ManualRunBlocked` mit «Ziel nicht erreichbar», der Status (M1-S23) wird «nicht erreichbar» und «Jetzt sichern» ist danach mit diesem Grund deaktiviert (geplante Läufe mit `ausstehend`: M3) · Test: Integration (SSH)
- **AC-M5-01-6** (FR-56, U18) – *Gegeben* ein SFTP-Ziel, *wenn* Host oder Pfad geändert und `[Speichern]` gewählt wird, *dann* laufen zuerst Verbindungstest und Repo-ID-Abgleich (M5-S09); nur bei Erfolg ruft die GUI `UpdateDestination` auf und die Konfiguration enthält die neuen Werte; scheitert er, bleibt die Konfiguration unverändert und die Meldung nennt den Grund · Test: Integration (SSH) + D-Bus

### M5-S02 Ziel-Typ S3-kompatibel
- **AC-M5-02-1** (FR-01) – *Gegeben* S3-Testserver (`rclone serve s3` mit `--auth-key`) mit Bucket und Schlüssel, *wenn* Endpoint, Bucket, Access-Key, Secret-Key eingegeben und das Ziel angelegt wird, *dann* gelingt `init` und ein anschliessendes Backup erzeugt einen Snapshot im Bucket · Test: Integration (`rclone serve s3`)
- **AC-M5-02-2** (FR-01, FR-08) – *Gegeben* das S3-Ziel, *wenn* die Config gelesen wird, *dann* enthält sie Endpoint und Bucket, aber weder Access- noch Secret-Key; beide liegen im Secret Service und gehen an restic nur als Umgebungsvariablen, nie in argv · Test: Integration (`rclone serve s3`) + Unit (Runner)
- **AC-M5-02-3** (FR-01) – *Gegeben* falscher Secret-Key, *wenn* der Test läuft, *dann* Fehlermeldung mit Handlungsoption, kein Ziel angelegt · Test: Integration (`rclone serve s3`)
- **AC-M5-02-4** (FR-30, P32) – *Gegeben* S3-Ziel mit gestopptem Testserver, das der Status (M1-S23) noch als erreichbar führt, *wenn* «Jetzt sichern» geklickt wird, *dann* scheitert der Vorab-Check (TCP/HTTP, Connect-Timeout 10 s), restic startet nicht, spätestens nach 15 s kommt `ManualRunBlocked` mit «Ziel nicht erreichbar», der Status wird «nicht erreichbar» und «Jetzt sichern» ist danach deaktiviert (geplante Läufe mit `ausstehend`: M3) · Test: Integration (`rclone serve s3`)
- **AC-M5-02-5** (FR-56, U18) – *Gegeben* ein S3-Ziel, *wenn* Endpoint, Bucket oder Schlüssel geändert und gespeichert werden, *dann* wie AC-M5-01-6: Verbindungstest gegen `rclone serve s3` und Repo-ID-Abgleich (M5-S09), `UpdateDestination` nur bei Erfolg, geänderte Schlüssel nur im Secret Service; bei falschem Schlüssel wird nichts gespeichert · Test: Integration (`rclone serve s3`) + D-Bus

### M5-S03 Generisches Formular
- **AC-M5-03-1** (FR-01) – *Gegeben* das generische Formular, *wenn* eine Repo-URL mit `b2:`, `azure:`, `gs:`, `swift:` oder `rclone:` eingegeben wird, *dann* wird sie akzeptiert; ein unbekanntes Schema ergibt einen Validierungsfehler und «Anlegen» bleibt gesperrt · Test: Unit
- **AC-M5-03-2** (FR-01, FR-08) – *Gegeben* erfasste Umgebungsvariablen, eine davon als Secret markiert, *wenn* gespeichert und ein Lauf gestartet wird, *dann* steht der Secret-Wert nur im Secret Service, nicht in der Config, und restic erhält ihn als Umgebungsvariable · Test: Unit (Runner) + Integration
- **AC-M5-03-3** (FR-01) – *Gegeben* rclone-Ziel ohne `rclone` im `PATH`, *wenn* das Ziel angelegt werden soll, *dann* verweist Kestic auf die fehlende Installation (M5-S04), statt restic mit unverständlichem Fehler zu starten · Test: Unit (PATH-Fake)
- **AC-M5-03-4** (FR-01, P22) – *Gegeben* die Variablen `B2_ACCOUNT_KEY`, `AWS_SECRET_ACCESS_KEY`, `MY_TOKEN`, `RCLONE_PASSWORD`, `RESTIC_HOST`, *wenn* sie erfasst werden, *dann* ist «geheim» bei den ersten vier vorausgewählt (Name enthält `KEY`, `SECRET`, `TOKEN` bzw. `PASSWORD`), bei `RESTIC_HOST` nicht; je Variable umschaltbar · Test: Unit
- **AC-M5-03-5** (FR-30, P32) – *Gegeben* Ziel aus dem generischen Formular (B2, Azure, GCS, Swift oder rclone; im Test `rclone:` auf einen Remote bzw. ein Mock-Kommando, das nicht antwortet), *wenn* vor einem manuellen Lauf der Vorab-Check läuft, *dann* führt Kestic `restic cat config` aus, beendet den Prozess nach 15 s, startet kein `restic backup`, setzt den Status (M1-S23) auf «nicht erreichbar» und meldet «Ziel nicht erreichbar» (`ManualRunBlocked`); antwortet das Backend innerhalb von 15 s, startet der Lauf normal · Test: Integration (Mock)
- **AC-M5-03-6** (FR-56, U18) – *Gegeben* ein generisches Ziel, *wenn* URL oder Variablen geändert und gespeichert werden, *dann* laufen Vorab-Check (`restic cat config`, 15 s) und Repo-ID-Abgleich (M5-S09); nur bei Erfolg folgt `UpdateDestination`, sonst bleibt alles unverändert · Test: Integration (Mock) + D-Bus

### M5-S04 Erkennung «bereit» / «braucht Installation»
- **AC-M5-04-1** (§5.7) – *Gegeben* `ssh` im `PATH`, `rclone` nicht, *wenn* die Ziel-Typ-Liste entsteht, *dann* sind Externes Laufwerk, Ordner, NAS per SSH, REST-Server und Cloud-Speicher (S3, B2, Azure, Google Cloud, Swift) «bereit», rclone «braucht Installation», und alle «bereit» stehen davor · Test: Unit (PATH-Fake) + GUI-Smoke
- **AC-M5-04-2** (§5.7) – *Gegeben* `ssh` fehlt im `PATH`, *wenn* die Liste entsteht, *dann* ist «NAS / Server per SSH» «braucht Installation» · Test: Unit (PATH-Fake)
- **AC-M5-04-3** (§5.7) – *Gegeben* rclone «braucht Installation», *wenn* der Eintrag geöffnet wird, *dann* zeigt er Paketname `rclone`, Knopf «In Discover öffnen» (`appstream://`-URL) und `sudo apt install rclone` · Test: GUI-Smoke (Screenshot)
- **AC-M5-04-4** (§5.7) – *Gegeben* rclone wurde nachträglich in den `PATH` gelegt, *wenn* «erneut prüfen» gewählt wird, *dann* wechselt der Eintrag auf «bereit» · Test: Unit (PATH-Fake)

### M5-S05 Externes Laufwerk / USB
- **AC-M5-05-1** (§5.7) – *Gegeben* UDisks2-Fake meldet einen eingehängten Wechseldatenträger, *wenn* «Externes Laufwerk / USB» gewählt wird, *dann* erscheint er mit Name und Mountpoint zur Auswahl · Test: D-Bus (Fake)
- **AC-M5-05-2** (§5.7) – *Gegeben* ein Ordner auf einem CIFS/SMB-Mount (Mountinfo-Fake), *wenn* das Ziel angelegt wird, *dann* warnt Kestic, dass restic von Repos auf CIFS/SMB abrät; Anlegen bleibt nach Bestätigung möglich · Test: Unit (Mountinfo-Fake)
- **AC-M5-05-3** (§5.7) – *Gegeben* der Dateidialog liefert eine KIO-Adresse (z. B. `smb://…`) statt eines lokalen Pfads, *wenn* sie als Ziel übernommen werden soll, *dann* lehnt Kestic sie mit Hinweis ab · Test: Unit
- **AC-M5-05-4** (FR-56, U18) – *Gegeben* ein lokales Ziel (Laufwerk/Ordner), *wenn* der Pfad geändert und gespeichert wird, *dann* prüft Kestic, dass dort ein restic-Repo mit dem gespeicherten Passwort lesbar ist, und gleicht die Repo-ID ab (M5-S09); nur dann folgt `UpdateDestination`, sonst bleibt die Konfiguration unverändert · Test: Integration (Temp-Repo) + D-Bus
- **AC-M5-05-5** (FR-56, P42; Story M5-S09) – *Gegeben* ein lokales Ziel, dessen Repo-Verzeichnis per `cp -r` an einen neuen Pfad kopiert wurde, *wenn* die Verbindung auf den neuen Pfad geändert und gespeichert wird, *dann* erkennt Kestic die gleiche Repo-ID (`restic cat config`), speichert ohne Rückfrage, und das nächste Backup findet seinen Parent · Test: Integration
- **AC-M5-05-6** (FR-56, P42; Story M5-S09) – *Gegeben* ein Ziel mit 2 bekannten Snapshots und ein neues Repo, in das beide per `restic copy` übertragen wurden, *wenn* die Verbindung darauf geändert wird, *dann* meldet Kestic «Umgezogenes Repository erkannt: 2 von 2 Snapshots vorhanden» (Abgleich über Feld `original`), speichert erst nach Bestätigung und behält die Historie; Kit-Ablauf und Zustand «Kit fehlt» danach gemäss AC-M5-09-6 · Test: Integration + GUI-Smoke
- **AC-M5-05-7** (FR-56, P42, negativ; Story M5-S09) – *Gegeben* ein neues Repo ohne einen bzw. mit nur 1 von 2 bekannten Snapshots, *wenn* die Verbindung darauf geändert wird, *dann* bleibt Speichern gesperrt, Kestic nennt «anderes Backup-Ziel» bzw. «1 von 2 vorhanden» und bietet «Als neues Ziel hinzufügen» mit vorausgefüllten Daten an; die Konfiguration bleibt unverändert · Test: Integration + GUI-Smoke

### M5-S06 restic herunterladen
- **AC-M5-06-1** (FR-33) – *Gegeben* Release-Mock mit `restic_0.19.1_linux_amd64.bz2`, gültigen `SHA256SUMS` und `SHA256SUMS.asc` (signiert mit Test-Key, Fingerprint über das Cargo-Feature `test-trust-anchor` gesetzt), CPU amd64, *wenn* der Download läuft, *dann* liegt ein ausführbares Binary unter `~/.local/share/kestic/` (im Test `XDG_DATA_HOME` auf Tempdir) und `version --json` meldet 0.19.1 · Test: Integration (Release-Mock)
- **AC-M5-06-2** (FR-33) – *Gegeben* CPU arm64, *wenn* der Download-Plan entsteht, *dann* wird `restic_<v>_linux_arm64.bz2` gewählt · Test: Unit
- **AC-M5-06-3** (FR-33) – *Gegeben* der Produktions-Build, *wenn* der eingebaute Signatur-Fingerprint gelesen wird, *dann* lautet er `CF8F 18F2 8445 7597 3F79 D4E1 91A6 868B D3F7 A907` · Test: Unit
- **AC-M5-06-4** (FR-33) – *Gegeben* `SHA256SUMS.asc` mit anderem Key signiert, *wenn* geprüft wird, *dann* bricht Kestic mit Meldung ab, legt kein neues Binary ab, ein vorhandenes App-verwaltetes bleibt unverändert · Test: Integration (Release-Mock)
- **AC-M5-06-5** (FR-33) – *Gegeben* ein Archiv, dessen SHA256 nicht zu `SHA256SUMS` passt, *wenn* geprüft wird, *dann* wie AC-M5-06-4 · Test: Integration (Release-Mock)
- **AC-M5-06-6** (FR-33) – *Gegeben* kein Netz (Mock nicht erreichbar), *wenn* der Download startet, *dann* verständliche Fehlermeldung, Einstellung und vorhandenes Binary unverändert · Test: Integration
- **AC-M5-06-7** (FR-33) – *Gegeben* ein vorhandenes App-verwaltetes Binary, *wenn* ein neues installiert wird, *dann* wird es erst als temporäre Datei im selben Verzeichnis geschrieben und geprüft, dann per `rename` an den Zielnamen verschoben; Abbruch vorher → altes Binary unverändert ausführbar · Test: Integration (Abbruch simuliert)
- **AC-M5-06-8** (FR-33) – *Gegeben* ein laufender Lauf mit dem App-verwalteten restic, *wenn* ein Update bereitsteht, *dann* wird erst ersetzt, wenn kein Lauf mehr aktiv ist · Test: Integration (D-Bus-Fake für Lauf-Status)
- **AC-M5-06-9** (FR-33, NFR-05) – *Gegeben* Release-Mock mit 0.17.1, *wenn* es installiert werden soll, *dann* lehnt Kestic wegen Mindestversion 0.18.0 ab · Test: Unit
- **AC-M5-06-10** (FR-33, P21) – *Gegeben* der Release-Build (Feature `test-trust-anchor` nicht aktiv), *wenn* Release-URL oder Signatur-Fingerprint per Umgebungsvariable, Config oder Argument geändert werden sollen, *dann* wirkt das nicht; austauschbar sind beide nur mit `test-trust-anchor`, das Integrationstests unter `tests/` aktivieren (`#[cfg(test)]` reicht nicht) und das im `.deb`-Build nie gesetzt ist · Test: Unit (Build ohne Feature) + Prüfung der `.deb`-Build-Konfiguration
- **AC-M5-06-11** (FR-33, P21) – *Gegeben* der Release-Build, *wenn* Release-Informationen abgerufen werden, *dann* fragt Kestic die GitHub Releases von restic ab (`api.github.com/repos/restic/restic/releases`) und lädt Assets nur von dort · Test: Unit (Request-Aufzeichnung)
- **AC-M5-06-12** (FR-33, P23) – *Gegeben* «Automatisch», System-restic 0.16.4, kein App-verwaltetes restic, *wenn* Kestic startet, *dann* lädt es nichts selbst, sondern bietet den Download an; erst nach Klick startet er · Test: Integration (Release-Mock zählt 0 Zugriffe vor dem Klick)

### M5-S07 Einstellung «restic-Quelle»
- **AC-M5-07-1** (FR-33) – *Gegeben* «Automatisch» und System-restic 0.18.1, *wenn* ein Lauf startet, *dann* nutzt Kestic das System-restic · Test: Unit (restic-Attrappe)
- **AC-M5-07-2** (FR-33, NFR-05) – *Gegeben* «Automatisch», System 0.16.4, App-verwaltet 0.19.1, *wenn* ein Lauf startet, *dann* nutzt Kestic das App-verwaltete · Test: Unit
- **AC-M5-07-3** (FR-33, NFR-05) – *Gegeben* «Automatisch», System 0.16.4, kein App-verwaltetes, *wenn* die Einstellungen öffnen, *dann* erklärt Kestic, warum 0.18.0 nötig ist, und bietet den Download an · Test: GUI-Smoke (Screenshot)
- **AC-M5-07-4** (FR-33) – *Gegeben* System- und App-verwaltetes restic, *wenn* «restic-Quelle» angezeigt wird, *dann* stehen beide Versionen da · Test: GUI-Smoke
- **AC-M5-07-5** (FR-33) – *Gegeben* App-verwaltetes aktiv und System-restic auf ≥ 0.18.0 aktualisiert (Attrappe ersetzt), *wenn* Kestic die Versionen prüft, *dann* erscheint «System-restic ist jetzt aktuell genug – umstellen?»; ein Klick stellt auf System um, auf Wunsch wird das App-verwaltete Binary aus `~/.local/share/kestic/` gelöscht · Test: Integration + GUI-Smoke
- **AC-M5-07-6** (FR-33, NFR-05) – *Gegeben* «System» explizit gewählt, System 0.16.4, *wenn* ein Lauf starten soll, *dann* startet er nicht und Kestic meldet die zu alte Version mit Handlungsoption · Test: Unit
- **AC-M5-07-7** (FR-33, NFR-05) – *Gegeben* «Eigener Pfad», *wenn* er auf keine ausführbare Datei oder restic < 0.18.0 zeigt, *dann* wird er mit Meldung abgewiesen; bei restic ≥ 0.18.0 wird er verwendet · Test: Unit
- **AC-M5-07-8** (FR-33, P31) – *Gegeben* Attrappe wie echtes 0.16.4 (`version --json` endet mit Fehler, `restic version` gibt `restic 0.16.4 compiled with go… on linux/amd64` aus), *wenn* Kestic die Version prüft, *dann* erkennt es per Text-Fallback 0.16.4 und meldet «zu alt, mindestens 0.18.0» statt eines Aufruffehlers · Test: Unit (restic-Attrappe)
- **AC-M5-07-9** (FR-33, P31) – *Gegeben* restic 0.18.0 bzw. aktuell, *wenn* Kestic die Version prüft, *dann* nutzt es `version --json`, nicht den Text-Fallback · Test: Integration

### M5-S08 Prüfung auf neue Releases
- **AC-M5-08-1** (FR-51, F20) – *Gegeben* App-verwaltet 0.19.0, Mock meldet 0.19.1, letzte Prüfung vor mehr als 7 Tagen, *wenn* der Daemon prüft, *dann* erscheint im Tray ein Hinweis auf die neue Version, nichts wird installiert · Test: Integration (Release-Mock, simulierte Uhr)
- **AC-M5-08-2** (FR-51, F20) – *Gegeben* letzte Prüfung vor weniger als 7 Tagen, *wenn* der Daemon startet, *dann* keine Anfrage an die Release-Quelle (Mock zählt 0 Zugriffe) · Test: Unit (simulierte Uhr)
- **AC-M5-08-3** (FR-51, F20) – *Gegeben* Prüfung abgeschaltet, *wenn* eine Woche simuliert vergeht, *dann* kein einziger Netzzugriff (Mock zählt 0) · Test: Integration (Release-Mock, simulierte Uhr)
- **AC-M5-08-4** (FR-51, F20) – *Gegeben* der Hinweis auf eine neue Version, *wenn* «Installieren» geklickt wird, *dann* läuft der Download mit allen Prüfungen aus M5-S06 · Test: Integration (Release-Mock)
- **AC-M5-08-5** (FR-51) – *Gegeben* «restic-Quelle» = System, *wenn* eine Woche simuliert vergeht, *dann* prüft Kestic nicht auf neue Releases · Test: Unit

### M5-S09 Verbindung bearbeiten: Repo-ID-Abgleich
Fälle für lokale Ziele: AC-M5-05-5 bis -7.
- **AC-M5-09-1** (FR-56, P42) – *Gegeben* ein REST-Ziel, dessen `rest-server` mit demselben Repo unter neuem Port läuft, *wenn* die URL geändert und gespeichert wird, *dann* erkennt Kestic die gleiche Repo-ID und speichert ohne Rückfrage · Test: Integration (`rest-server`) + D-Bus
- **AC-M5-09-2** (FR-56, P42, P24) – *Gegeben* ein SFTP-Ziel, dessen Repo unter neuem Host erreichbar ist, *wenn* der Host geändert und gespeichert wird, *dann* erscheint zuerst der Host-Key-Dialog; nach Bestätigung erkennt Kestic die gleiche Repo-ID und speichert ohne weitere Rückfrage · Test: Integration (SSH)
- **AC-M5-09-3** (FR-56, P42) – *Gegeben* Ziel A mit 2 bekannten Snapshots, per `restic copy` erst nach B, dann von B nach C kopiert, *wenn* die Verbindung auf C geändert wird, *dann* erkennt der Abgleich über `original` (falls gesetzt, sonst `id`) «2 von 2 Snapshots vorhanden» **[verifiziert: `original` zeigt nach A→B→C auf den Snapshot in A]** · Test: Integration
- **AC-M5-09-4** (FR-56, P42) – *Gegeben* ein umgezogenes Repo mit anderem Passwort, *wenn* die Verbindung darauf geändert wird, *dann* nimmt die Prüfung das optionale Feld «Passwort des neuen Repositorys» entgegen und speichert es erst nach erfolgreicher Prüfung im Secret Service; falsches Passwort → Exit 12, Meldung, nichts gespeichert · Test: Integration + D-Bus
- **AC-M5-09-5** (FR-56, P42) – *Gegeben* das alte Repo hatte 3 Snapshots, davon 1 per `forget` entfernt, und das neue enthält die 2 übrigen, *wenn* abgeglichen wird, *dann* meldet Kestic «2 von 2 vorhanden»; ist das alte Repo nicht erreichbar, nutzt der Abgleich die zuletzt gespeicherte Liste aus `~/.local/state/kestic/` (im Test `XDG_STATE_HOME`) · Test: Integration
- **AC-M5-09-6** (FR-56, P42) – *Gegeben* Fall 2 (umgezogenes Repo) bestätigt, *wenn* gespeichert ist, *dann* startet der Kit-Ablauf direkt (ab M6; davor nur Zustand «Kit fehlt»); bricht der Benutzer ab, zeigt das Ziel «Kit fehlt» mit Emblem und Hinweis in der Übersicht, nicht ausblendbar · Test: GUI-Smoke + D-Bus
- **AC-M5-09-7** (FR-56, P42) – *Gegeben* die Prüfung scheitert nur am Repo-Passwort, *wenn* das Ergebnis angezeigt wird, *dann* erscheint zuerst das Feld «Passwort des neuen Repositorys» (Ablauf AC-M5-09-4), erst danach entscheidet sich Fall 2 oder 3 · Test: Integration + GUI-Smoke
- **AC-M5-09-8** (FR-56, P42) – *Gegeben* ein Ziel ohne Snapshots (bekannte Liste leer), *wenn* die Verbindung auf ein Repo mit anderer Repo-ID geändert wird, *dann* gilt Fall 3 (Speichern gesperrt, «Als neues Ziel hinzufügen»), nie Fall 2 · Test: Integration
- **AC-M5-09-9** (FR-56, P42) – *Gegeben* die eigene Aufbewahrung hat nach dem letzten Speichern der Liste Snapshots entfernt und das alte Repo ist nicht erreichbar, *wenn* abgeglichen wird, *dann* ist die gespeicherte Liste bereits nach dem `forget`-Lauf aktualisiert, die entfernten Snapshots zählen nicht als fehlend, und es gilt Fall 2 · Test: Integration

## Exit-Kriterien

- Auf TUXEDO ohne System-restic: Kestic lädt restic geprüft herunter und sichert.
- Ziele Lokal, REST, SFTP, S3 getestet; übrige über generisches Formular.
- Manuelle Abnahme: echtes Netzwerk-Ziel (z. B. NAS) (REST oder SFTP).
