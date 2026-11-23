# M5 Ziele und restic-Bereitstellung

Teil von [backlog.md](../backlog.md).

**Ziel:** Alle restic-Backends sind nutzbar, und Kestic funktioniert auch dort, wo das System-restic zu alt ist (Ubuntu-24.04-Basis).

**Voraussetzung:** M1; M4-S00 für M5-S01 (Aufteilung gemäss §Code-Struktur).

Ablage gemäss [architecture.md](../architecture.md) §Code-Struktur.

## Anforderungen

FR-01 (alle Ziel-Typen), FR-33 (Download, Umstellen), FR-56 · NFR-05 · F20, P21–P24, P31, P32, P42, U18 · [architecture.md](../architecture.md) «Onboarding und Ziel-Typen» (Ziel-Typen)

## Stories

| ID | Story |
| --- | --- |
| M5-S01 | Ziel-Typ SFTP: Formular, Verbindungstest, Hinweis auf SSH-Schlüssel |
| M5-S02 | Ziel-Typ S3-kompatibel: Formular, Credentials im Secret Service |
| M5-S03 | Generisches Formular (URL + Umgebungsvariablen) für B2, Azure, GCS, Swift, rclone |
| M5-S04 | Erkennung «bereit» / «braucht Installation» (z. B. `ssh`, `rclone`), Hinweis mit Paketname und `apt install` (Discover-Link nur mit AppStream-Komponente) |
| M5-S05 | Externes Laufwerk / USB: Wechseldatenträger erkennen (UDisks2), Hinweis bei SMB/CIFS-Ordner |
| M5-S06 | restic herunterladen: Architektur (amd64/arm64), `SHA256SUMS` + `.asc` prüfen (Fingerprint im Code), atomar ersetzen, nie während Lauf (FR-33) |
| M5-S07 | Einstellung «restic-Quelle» (Automatisch / System / Von Kestic verwaltet / Eigener Pfad), Anzeige beider Versionen, Umstellen auf «Automatisch», App-Binary löschen, Migration der Config (FR-33) |
| M5-S09 | Verbindung bearbeiten mit Vergleich der Repo-ID für alle Ziel-Typen: gleiche ID → speichern, andere ID → «als neues Ziel hinzufügen» (FR-56, P42, U18); ACs AC-M5-01-6, -02-5, -03-6, -05-4, -05-5, -05-7, -09-1 bis -09-3 |

## Akzeptanzkriterien

Test-Isolation gemäss [test-strategy.md](../test-strategy.md). Zusätzlich: kein echter Netzzugriff (Release-Mock), simulierte Uhr aus `core` (M1, P36); restic-Attrappen unter 0.17 verhalten sich wie das echte Binary (kein `version --json`, P31); SSH-Server im Test-Setup (`sshd` oder Container) mit `known_hosts`-Pfad im Tempdir.

### M5-S01 Ziel-Typ SFTP
- **AC-M5-01-1** (FR-01, P24) – *Gegeben* SSH-Server mit Schlüssel-Login, `ssh` im `PATH`, *wenn* im SFTP-Formular Host, Benutzer, Pfad angegeben und «Verbindung testen» gewählt wird, *dann* (a) meldet Kestic Erfolg, (b) erkennt es «kein Repo vorhanden», (c) legt `AddDestination` das Repository erst beim Abschluss des Assistenten per `init` an (der Test speichert nichts), und (d) enthält jeder ssh-Aufruf von restic `-o UserKnownHostsFile=<Kestic-Datei> <~/.ssh/known_hosts>` (über `sftp.args`, im Test beide im Tempdir) · Test: Integration (SSH)
- **AC-M5-01-2** (FR-01) – *Gegeben* derselbe Server mit bestehendem Repo, *wenn* der Test läuft und das Passwort stimmt, *dann* (a) wird das bestehende Repo verbunden statt initialisiert, und (b) nutzt ssh nur die übergebenen `known_hosts`-Pfade · Test: Integration (SSH)
- **AC-M5-01-3** (FR-01) – *Gegeben* kein passender Schlüssel, *wenn* der Test läuft, *dann* (a) erscheint eine verständliche Meldung mit Hinweis auf SSH-Schlüssel und ssh-agent, und (b) wird kein Ziel angelegt · Test: Integration (SSH)
- **AC-M5-01-4** (FR-01, P24) – *Gegeben* unbekannter Host-Key, *wenn* der Test läuft, *dann* (a) zeigt Kestic Schlüsseltyp und SHA256-Fingerprint und fragt, (b) steht genau dieser Key erst nach Bestätigung in Kestics eigener Datei `~/.local/share/kestic/known_hosts` (im Test unter `XDG_DATA_HOME`), (c) bleibt die Datei bei Ablehnung unverändert und es entsteht kein Ziel, und (d) bleibt `~/.ssh/known_hosts` immer unverändert, Kestic liest es nur · Test: Integration (SSH)
- **AC-M5-01-5** (FR-30, P32) – *Gegeben* SFTP-Ziel mit gestopptem `sshd`, das der Verfügbarkeitsstatus (M1-S23) noch als erreichbar führt, *wenn* «Jetzt sichern» geklickt wird, *dann* (a) scheitert der Vorab-Check (`restic cat config` mit `ConnectTimeout=10` über `sftp.args`, Kestic-Timeout 15 s), (b) startet kein `restic backup`, (c) kommt spätestens nach 15 s `ManualRunBlocked` mit «Ziel nicht erreichbar», (d) wird der Status (M1-S23) «nicht erreichbar», und (e) ist «Jetzt sichern» danach mit diesem Grund deaktiviert (geplante Läufe deckt M3 ab; hier nur der manuelle Lauf) · Test: Integration (SSH)
- **AC-M5-01-6** (FR-56, U18) – *Gegeben* ein SFTP-Ziel, *wenn* Host oder Pfad geändert und `[Speichern]` gewählt wird, *dann* (a) laufen zuerst Verbindungstest und Repo-ID-Abgleich (M5-S09), (b) ruft die GUI nur bei Erfolg `UpdateDestination` auf und enthält die Konfiguration die neuen Werte, und (c) bleibt bei Scheitern die Konfiguration unverändert und nennt die Meldung den Grund · Test: Integration (SSH) + D-Bus
- **AC-M5-01-7** (FR-01, P24) – *Gegeben* ein Host, für den Kestics Datei oder `~/.ssh/known_hosts` einen anderen Key desselben Typs enthält, *wenn* der Test läuft oder ein Lauf startet, *dann* (a) bricht Kestic mit «Der Server meldet sich anders als bisher» ab und nennt die Datei mit dem alten Eintrag, (b) bietet es kein Übernehmen an, (c) bleiben beide Dateien unverändert, und (d) startet restic nicht · Test: Integration (SSH)
- **AC-M5-01-8** (FR-01, FR-56) – *Gegeben* zwei SFTP-Ziele auf demselben Host und Port, *wenn* das erste entfernt wird, *dann* (a) bleibt der Eintrag in Kestics `known_hosts`, und (b) verschwindet er erst, wenn auch das zweite entfernt ist; Einträge in `~/.ssh/known_hosts` bleiben immer · Test: Integration

### M5-S02 Ziel-Typ S3-kompatibel
- **AC-M5-02-1** (FR-01) – *Gegeben* S3-Testserver (`rclone serve s3` mit `--auth-key`) mit Bucket und Schlüssel, *wenn* Endpoint, Bucket, Access-Key, Secret-Key eingegeben und das Ziel angelegt wird, *dann* (a) gelingt `init`, und (b) erzeugt ein anschliessendes Backup einen Snapshot im Bucket · Test: Integration (`rclone serve s3`)
- **AC-M5-02-2** (FR-01, FR-08) – *Gegeben* das S3-Ziel, *wenn* die Config gelesen wird, *dann* (a) enthält sie Endpoint und Bucket, (b) enthält sie weder Access- noch Secret-Key, (c) liegen beide im Secret Service, und (d) gehen sie an restic nur als Umgebungsvariablen, nie in argv · Test: Integration (`rclone serve s3`) + Unit (Runner)
- **AC-M5-02-3** (FR-01) – *Gegeben* falscher Secret-Key, *wenn* der Test läuft, *dann* (a) erscheint eine Fehlermeldung mit Handlungsoption, und (b) wird kein Ziel angelegt · Test: Integration (`rclone serve s3`)
- **AC-M5-02-4** (FR-30, P32) – *Gegeben* S3-Ziel mit gestopptem Testserver, das der Status (M1-S23) noch als erreichbar führt, *wenn* «Jetzt sichern» geklickt wird, *dann* (a) scheitert der Vorab-Check (TCP/HTTP, Connect-Timeout 10 s), (b) startet restic nicht, (c) kommt spätestens nach 15 s `ManualRunBlocked` mit «Ziel nicht erreichbar», (d) wird der Status «nicht erreichbar», und (e) ist «Jetzt sichern» danach deaktiviert (geplante Läufe deckt M3 ab; hier nur der manuelle Lauf) · Test: Integration (`rclone serve s3`)
- **AC-M5-02-5** (FR-56, U18) – *Gegeben* ein S3-Ziel, *wenn* Endpoint, Bucket oder Schlüssel geändert und gespeichert werden, *dann* (a) laufen wie AC-M5-01-6 Verbindungstest gegen `rclone serve s3` und Repo-ID-Abgleich (M5-S09), (b) folgt `UpdateDestination` nur bei Erfolg, (c) stehen geänderte Schlüssel nur im Secret Service, und (d) wird bei falschem Schlüssel nichts gespeichert · Test: Integration (`rclone serve s3`) + D-Bus

### M5-S03 Generisches Formular
- **AC-M5-03-1** (FR-01) – *Gegeben* das generische Formular, *wenn* eine Repo-URL mit `b2:`, `azure:`, `gs:`, `swift:` oder `rclone:` eingegeben wird, *dann* (a) werden sie akzeptiert, (b) ergibt ein unbekanntes Schema einen Validierungsfehler, und (c) bleibt «Anlegen» bei ungültigem Schema gesperrt · Test: Unit
- **AC-M5-03-2** (FR-01, FR-08) – *Gegeben* erfasste Umgebungsvariablen, eine davon als Secret markiert, *wenn* gespeichert und ein Lauf gestartet wird, *dann* (a) steht der Secret-Wert nur im Secret Service, (b) steht er nicht in der Config, und (c) erhält restic ihn als Umgebungsvariable · Test: Unit (Runner) + Integration
- **AC-M5-03-3** (FR-01) – *Gegeben* rclone-Ziel ohne `rclone` im `PATH`, *wenn* das Ziel angelegt werden soll, *dann* (a) verweist Kestic auf die fehlende Installation (M5-S04), und (b) startet es restic nicht mit unverständlichem Fehler · Test: Unit (PATH-Fake)
- **AC-M5-03-4** (FR-01, P22) – *Gegeben* die Variablen `B2_ACCOUNT_KEY`, `AWS_SECRET_ACCESS_KEY`, `MY_TOKEN`, `RCLONE_PASSWORD`, `RESTIC_HOST`, *wenn* sie erfasst werden, *dann* (a) ist «geheim» bei den ersten vier vorausgewählt (Name enthält `KEY`, `SECRET`, `TOKEN` bzw. `PASSWORD`), (b) ist es bei `RESTIC_HOST` nicht vorausgewählt, und (c) ist es je Variable umschaltbar · Test: Unit
- **AC-M5-03-5** (FR-30, P32) – *Gegeben* Ziel aus dem generischen Formular (B2, Azure, GCS, Swift oder rclone; im Test `rclone:` auf einen Remote bzw. ein Mock-Kommando, das nicht antwortet), *wenn* vor einem manuellen Lauf der Vorab-Check läuft, *dann* (a) führt Kestic `restic cat config` aus, (b) beendet es einen nicht antwortenden Prozess nach 15 s, (c) startet kein `restic backup`, (d) setzt es den Status (M1-S23) auf «nicht erreichbar», (e) meldet es «Ziel nicht erreichbar» (`ManualRunBlocked`), und (f) startet der Lauf normal, wenn das Backend innerhalb von 15 s antwortet · Test: Integration (Mock)
- **AC-M5-03-6** (FR-56, U18) – *Gegeben* ein generisches Ziel, *wenn* URL oder Variablen geändert und gespeichert werden, *dann* (a) laufen Vorab-Check (`restic cat config`, 15 s) und Repo-ID-Abgleich (M5-S09), (b) folgt `UpdateDestination` nur bei Erfolg, und (c) bleibt sonst alles unverändert · Test: Integration (Mock) + D-Bus
- **AC-M5-03-7** (FR-01, NFR-02) – *Gegeben* das generische Formular, *wenn* eine Variable `RESTIC_PASSWORD_COMMAND`, `RESTIC_REPOSITORY_FILE`, `PATH`, `HOME`, `LD_PRELOAD` oder ein Name ausserhalb `[A-Z_][A-Z0-9_]*` erfasst wird, *dann* (a) weist `core` sie mit Meldung ab (auch über D-Bus), (b) bleibt «Anlegen» gesperrt, und (c) übergibt Kestic sie nie an restic · Test: Unit

### M5-S04 Erkennung «bereit» / «braucht Installation»
- **AC-M5-04-1** (§9.1) – *Gegeben* `ssh` im `PATH`, `rclone` nicht, *wenn* die Ziel-Typ-Liste entsteht, *dann* (a) sind Externes Laufwerk, Ordner, NAS per SSH, REST-Server und Cloud-Speicher (S3, B2, Azure, Google Cloud, Swift) «bereit», (b) ist rclone «braucht Installation», und (c) stehen alle «bereit» davor · Test: Unit (PATH-Fake) + GUI-Smoke
- **AC-M5-04-2** (§9.1) – *Gegeben* `ssh` fehlt im `PATH`, *wenn* die Liste entsteht, *dann* (a) ist «NAS / Server per SSH» «braucht Installation» · Test: Unit (PATH-Fake)
- **AC-M5-04-3** (§9.1) – *Gegeben* rclone «braucht Installation», *wenn* der Eintrag geöffnet wird, *dann* (a) zeigt er Paketname `rclone`, (b) zeigt er keinen Knopf «In Discover öffnen» (`rclone` hat keine AppStream-Komponente), und (c) zeigt er `sudo apt install rclone` · Test: GUI-Smoke (Screenshot)
- **AC-M5-04-4** (§9.1) – *Gegeben* rclone wurde nachträglich in den `PATH` gelegt, *wenn* «erneut prüfen» gewählt wird, *dann* (a) wechselt der Eintrag auf «bereit» · Test: Unit (PATH-Fake)

### M5-S05 Externes Laufwerk / USB
- **AC-M5-05-1** (§9.1) – *Gegeben* UDisks2-Fake meldet einen eingehängten Wechseldatenträger, *wenn* «Externes Laufwerk / USB» gewählt wird, *dann* (a) erscheint er mit Name und Mountpoint zur Auswahl · Test: D-Bus (Fake)
- **AC-M5-05-2** (§9.1) – *Gegeben* ein Ordner auf einem CIFS/SMB-Mount (Mountinfo-Fake), *wenn* das Ziel angelegt wird, *dann* (a) warnt Kestic, dass restic von Repos auf CIFS/SMB abrät, und (b) bleibt Anlegen nach Bestätigung möglich · Test: Unit (Mountinfo-Fake)
- **AC-M5-05-3** (§9.1) – *Gegeben* der Dateidialog liefert eine KIO-Adresse (z. B. `smb://…`) statt eines lokalen Pfads, *wenn* sie als Ziel übernommen werden soll, *dann* (a) lehnt Kestic sie mit Hinweis ab · Test: Unit
- **AC-M5-05-4** (FR-56, U18) – *Gegeben* ein lokales Ziel (Laufwerk/Ordner), *wenn* der Pfad geändert und gespeichert wird, *dann* (a) prüft Kestic, dass dort ein restic-Repo mit dem gespeicherten Passwort lesbar ist, (b) gleicht es die Repo-ID ab (M5-S09), (c) folgt `UpdateDestination` nur dann, und (d) bleibt die Konfiguration sonst unverändert · Test: Integration (Temp-Repo) + D-Bus
- **AC-M5-05-5** (FR-56, P42; Story M5-S09) – *Gegeben* ein lokales Ziel, dessen Repo-Verzeichnis per `cp -r` an einen neuen Pfad kopiert wurde, *wenn* die Verbindung auf den neuen Pfad geändert und gespeichert wird, *dann* (a) erkennt Kestic die gleiche Repo-ID (`restic cat config`), (b) speichert es ohne Rückfrage, und (c) findet das nächste Backup seinen Parent · Test: Integration
- **AC-M5-05-7** (FR-56, P42, negativ; Story M5-S09) – *Gegeben* ein lokales Ziel und am neuen Pfad ein Repo mit anderer Repo-ID (eigenes `init`, auch wenn per `restic copy` dieselben Snapshots darin sind), *wenn* die Verbindung darauf geändert wird, *dann* (a) bleibt Speichern gesperrt, (b) nennt Kestic «anderes Backup-Ziel», und (c) bietet es «Als neues Ziel hinzufügen» mit den eingegebenen Werten an · Test: Integration + GUI-Smoke

### M5-S06 restic herunterladen
- **AC-M5-06-1** (FR-33) – *Gegeben* Release-Mock mit `restic_0.19.1_linux_amd64.bz2`, gültigen `SHA256SUMS` und `SHA256SUMS.asc` (signiert mit Test-Key, Fingerprint über das Cargo-Feature `test-trust-anchor` gesetzt), CPU amd64, *wenn* der Download läuft, *dann* (a) liegt ein ausführbares Binary unter `~/.local/share/kestic/` (im Test `XDG_DATA_HOME` auf Tempdir), und (b) meldet `version --json` 0.19.1 · Test: Integration (Release-Mock)
- **AC-M5-06-2** (FR-33) – *Gegeben* CPU arm64, *wenn* der Download-Plan entsteht, *dann* (a) wird `restic_<v>_linux_arm64.bz2` gewählt · Test: Unit
- **AC-M5-06-3** (FR-33) – *Gegeben* der Produktions-Build, *wenn* der eingebaute Signatur-Fingerprint gelesen wird, *dann* (a) lautet er `CF8F 18F2 8445 7597 3F79 D4E1 91A6 868B D3F7 A907` · Test: Unit
- **AC-M5-06-4** (FR-33) – *Gegeben* `SHA256SUMS.asc` mit anderem Key signiert, *wenn* geprüft wird, *dann* (a) bricht Kestic mit Meldung ab, (b) legt es kein neues Binary ab, und (c) bleibt ein vorhandenes App-verwaltetes unverändert · Test: Integration (Release-Mock)
- **AC-M5-06-5** (FR-33) – *Gegeben* ein Archiv, dessen SHA256 nicht zu `SHA256SUMS` passt, *wenn* geprüft wird, *dann* (a) bricht Kestic mit Meldung ab (wie AC-M5-06-4), (b) legt es kein neues Binary ab, und (c) bleibt ein vorhandenes App-verwaltetes unverändert · Test: Integration (Release-Mock)
- **AC-M5-06-6** (FR-33) – *Gegeben* kein Netz (Mock nicht erreichbar), *wenn* der Download startet, *dann* (a) erscheint eine verständliche Fehlermeldung, (b) bleibt die Einstellung unverändert, und (c) bleibt das vorhandene Binary unverändert · Test: Integration
- **AC-M5-06-7** (FR-33) – *Gegeben* ein vorhandenes App-verwaltetes Binary, *wenn* ein neues installiert wird, *dann* (a) wird es erst als temporäre Datei im selben Verzeichnis geschrieben und geprüft, (b) wird es dann per `rename` an den Zielnamen verschoben, und (c) bleibt das alte Binary bei Abbruch vorher unverändert ausführbar · Test: Integration (Abbruch simuliert)
- **AC-M5-06-8** (FR-33) – *Gegeben* ein laufender Lauf mit dem App-verwalteten restic, *wenn* ein Update bereitsteht, *dann* (a) lädt und prüft Kestic die neue Version sofort, (b) ersetzt es das Binary erst, wenn kein Lauf und keine Vorschau mit restic mehr aktiv ist (Backup, Wartung, Restore, Metadaten, Vorschauen, Probeläufe), und (c) wartet ein Lauf, der während des Ersetzens startet, bis es fertig ist · Test: Integration (D-Bus-Fake für Lauf-Status)
- **AC-M5-06-9** (FR-33, NFR-05) – *Gegeben* Release-Mock mit 0.17.1, *wenn* es installiert werden soll, *dann* (a) lehnt Kestic wegen Mindestversion 0.18.0 ab · Test: Unit
- **AC-M5-06-10** (FR-33, P21) – *Gegeben* der Release-Build (Feature `test-trust-anchor` nicht aktiv), *wenn* Release-URL oder Signatur-Fingerprint per Umgebungsvariable, Config oder Argument geändert werden sollen, *dann* (a) wirkt das nicht, (b) sind beide (samt eingebettetem Schlüssel) nur mit `test-trust-anchor` austauschbar (das Integrationstests unter `tests/` aktivieren; `#[cfg(test)]` reicht nicht), und (c) ist es im `.deb`-Build nie gesetzt · Test: Unit (Build ohne Feature) + Prüfung der `.deb`-Build-Konfiguration
- **AC-M5-06-11** (FR-33, P21) – *Gegeben* der Release-Build, *wenn* Release-Informationen abgerufen werden, *dann* (a) fragt Kestic die GitHub Releases von restic ab (`api.github.com/repos/restic/restic/releases/latest`), (b) lädt es Assets nur von `github.com/restic/restic/releases/download/` und folgt Weiterleitungen nur per HTTPS auf echte Unterdomains von `githubusercontent.com`, und (c) bricht bei jedem anderen Host ab · Test: Unit (Request-Aufzeichnung)
- **AC-M5-06-12** (FR-33, P23) – *Gegeben* «Automatisch», System-restic 0.16.4, kein App-verwaltetes restic, *wenn* Kestic startet, *dann* (a) lädt es nichts selbst (0 Zugriffe auf den Release-Mock vor dem Klick), (b) bietet es den Download an, und (c) startet der Download erst nach Klick · Test: Integration (Release-Mock zählt 0 Zugriffe vor dem Klick)
- **AC-M5-06-13** (FR-33) – *Gegeben* das `.deb`, *wenn* seine Abhängigkeiten gelesen werden, *dann* (a) steht `gpgv` unter `Depends`, und (b) bricht ein Download ohne `gpgv` im `PATH` mit Meldung ab, ohne ein Binary abzulegen und ohne andere Prüfung · Test: Unit (`.deb`-Metadaten) + Integration (PATH-Fake)

### M5-S07 Einstellung «restic-Quelle»
- **AC-M5-07-1** (FR-33) – *Gegeben* «Automatisch» und System-restic 0.18.1, *wenn* ein Lauf startet, *dann* (a) nutzt Kestic das System-restic · Test: Unit (restic-Attrappe)
- **AC-M5-07-2** (FR-33, NFR-05) – *Gegeben* «Automatisch», System 0.16.4, App-verwaltet 0.19.1, *wenn* ein Lauf startet, *dann* (a) nutzt Kestic das App-verwaltete · Test: Unit
- **AC-M5-07-3** (FR-33, NFR-05) – *Gegeben* «Automatisch», System 0.16.4, kein App-verwaltetes, *wenn* die Einstellungen öffnen, *dann* (a) erklärt Kestic, warum 0.18.0 nötig ist, und (b) bietet es den Download an · Test: GUI-Smoke (Screenshot)
- **AC-M5-07-4** (FR-33) – *Gegeben* System- und App-verwaltetes restic, *wenn* «restic-Quelle» angezeigt wird, *dann* (a) stehen beide Versionen da, (b) stehen Quelle, genutzte Version und Pfad des genutzten Programms je in einer eigenen Zeile, und (c) steht bei einer genutzten Version ab 0.18.0, aber unter der getesteten aktuellen, «empfohlen: <getestete aktuelle Version>» · Test: GUI-Smoke
- **AC-M5-07-5** (FR-33) – *Gegeben* «Von Kestic verwaltet» gewählt und System-restic auf ≥ 0.18.0 aktualisiert (Attrappe ersetzt), *wenn* Kestic die Versionen prüft, *dann* (a) erscheint «System-restic ist jetzt aktuell genug – umstellen?», (b) stellt ein Klick auf «Automatisch» um (damit gilt das System-restic), (c) wird auf Wunsch das App-verwaltete Binary aus `~/.local/share/kestic/` gelöscht, und (d) nutzt Kestic bei «Automatisch» das System-restic ohne Frage und bietet nur (c) an · Test: Integration + GUI-Smoke
- **AC-M5-07-6** (FR-33, NFR-05) – *Gegeben* «System» explizit gewählt, System 0.16.4, *wenn* ein Lauf starten soll, *dann* (a) startet er nicht, und (b) meldet Kestic die zu alte Version mit Handlungsoption · Test: Unit
- **AC-M5-07-7** (FR-33, NFR-05) – *Gegeben* «Eigener Pfad», *wenn* er auf keine ausführbare Datei oder restic < 0.18.0 zeigt, *dann* (a) wird er mit Meldung abgewiesen, und (b) wird er bei restic ≥ 0.18.0 verwendet · Test: Unit
- **AC-M5-07-8** (FR-33, P31) – *Gegeben* Attrappe wie echtes 0.16.4 (`version --json` endet mit Fehler, `restic version` gibt `restic 0.16.4 compiled with go… on linux/amd64` aus), *wenn* Kestic die Version prüft, *dann* (a) erkennt es per Text-Fallback 0.16.4, und (b) meldet es «zu alt, mindestens 0.18.0» statt eines Aufruffehlers · Test: Unit (restic-Attrappe)
- **AC-M5-07-9** (FR-33, P31) – *Gegeben* restic 0.18.0 bzw. aktuell, *wenn* Kestic die Version prüft, *dann* (a) nutzt es `version --json`, und (b) nicht den Text-Fallback · Test: Integration
- **AC-M5-07-10** (FR-33, F13) – *Gegeben* eine Config mit `config_version = 3`, *wenn* sie geladen wird, *dann* (a) wird `restic_provider.kind = "system"` zu `"automatic"`, (b) bleibt `custom` mit seinem Pfad unverändert, und (c) schreibt Kestic die Datei erst beim nächsten Speichern mit Version 4 · Test: Unit

### M5-S08 (nach dem MVP)
Die Prüfung auf neue Releases (FR-51) ist seit 2026-11-23 nicht mehr Teil des MVP ([nach-mvp.md](nach-mvp.md)). Ein Update des App-verwalteten restic gibt es im MVP nur über `[Jetzt aktualisieren]` (M5-S06).

### M5-S09 Verbindung bearbeiten: Repo-ID-Abgleich
Fälle für lokale Ziele: AC-M5-05-5 bis -7.
- **AC-M5-09-1** (FR-56, P42) – *Gegeben* ein REST-Ziel, dessen `rest-server` mit demselben Repo unter neuem Port läuft, *wenn* die URL geändert und gespeichert wird, *dann* (a) erkennt Kestic die gleiche Repo-ID, und (b) speichert es ohne Rückfrage · Test: Integration (`rest-server`) + D-Bus
- **AC-M5-09-2** (FR-56, P42, P24) – *Gegeben* ein SFTP-Ziel, dessen Repo unter neuem Host erreichbar ist, *wenn* der Host geändert und gespeichert wird, *dann* (a) erscheint zuerst der Host-Key-Dialog, (b) erkennt Kestic nach Bestätigung die gleiche Repo-ID, und (c) speichert es ohne weitere Rückfrage · Test: Integration (SSH)
- **AC-M5-09-3** (FR-56, P42) – *Gegeben* ein Ziel, dessen alte Adresse nicht mehr antwortet, *wenn* die Verbindung auf eine Adresse mit demselben Repo geändert wird, *dann* (a) vergleicht Kestic mit der gespeicherten Repo-ID, und (b) speichert ohne Rückfrage; (c) bei einem Ziel aus der Zeit vor M5 ohne gespeicherte ID liest Kestic sie von der alten Adresse, wenn sie antwortet, sonst gilt jede neue ID als anderes Backup-Ziel · Test: Integration

Ein umgezogenes Repo mit neuer Repo-ID (Abgleich über Snapshots, früher AC-M5-05-6, AC-M5-09-3 bis -9) kommt nach dem MVP mit FR-14 ([nach-mvp.md](nach-mvp.md)).

## Exit-Kriterien

- Auf TUXEDO ohne System-restic: Kestic lädt restic geprüft herunter und sichert.
- Ziele Lokal, REST, SFTP, S3 getestet; übrige über generisches Formular.
- Manuelle Abnahme: echtes Netzwerk-Ziel (z. B. NAS) (REST oder SFTP).
