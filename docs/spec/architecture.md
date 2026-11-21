# Architektur

Domänenmodell und Persistenz: [domain-model.md](domain-model.md). D-Bus-Schnittstelle: [dbus-api.md](dbus-api.md). restic-Fakten: [restic-integration.md](restic-integration.md). Entscheide: [decisions.md](decisions.md).

## Komponenten und Ausführung

```
┌──────────────────┐  D-Bus   ┌──────────────────────────────┐  spawn + JSON  ┌────────┐
│ gui              │◄────────►│ daemon (XDG-Autostart)       │───────────────►│ restic │
│ CXX-Qt + Kirigami│          │ Scheduler, Warteschlangen,   │                └────────┘
└────────┬─────────┘          │ Tray (ksni), Notifications,  │
         │                    │ UPower-/NM-/logind-Signale   │
         │                    └──────────────┬───────────────┘
         │      ┌──────────────────────┐     │
         └─────►│ core (reines Rust)   │◄────┘   Modell, Config (TOML/serde), restic-Runner,
                │                      │         JSON-Parser, Secrets, Scheduler,
                └──────────────────────┘         Quellen-/Bedingungs-Checks
                           ▲
         `kestic secret get <ziel-id>`  ◄── RESTIC_PASSWORD_COMMAND / Expertenmodus
```

- **Binaries:** ein Binary `kestic` mit Unterbefehlen `kestic` (GUI), `kestic daemon`, `kestic secret get`; `.desktop` für die GUI, Autostart-Eintrag für den Daemon (F19). Im MVP lädt damit auch der Daemon Qt (E17, FR-52).
- **Daemon = einziger Executor und Zustands-Owner.** Startet mit der Session über `/etc/xdg/autostart/<app-id>.daemon.desktop` aus dem Paket (wie Déjà Dup; ein Eintrag in `~/.config/autostart` bliebe nach `apt remove` verwaist) mit `Exec=kestic daemon --autostart` und `TryExec=/usr/bin/kestic` (E11, P34). Der Daemon startet damit für jeden Benutzer: **mit `--autostart` und ohne Kestic-Konfiguration beendet er sich sofort** (kein Tray, NFR-11 bleibt gewahrt). Von der GUI gestartet (ohne `--autostart`) bleibt er und nimmt die erste Konfiguration entgegen (Onboarding, F14). Pro Benutzer abschaltbar in den Systemeinstellungen unter «Autostart» (XDG-Override mit `Hidden=true` **[angenommen, Plasma-KCM-Toggle noch nicht separat manuell geprüft]**). App-ID im Reverse-DNS-Format einer eigenen Domain: `net.daester.Kestic` (`org.kde.*` nur für offizielle KDE-Projekte).
- **Nach `apt remove`:** Dateien unter `/etc` sind Conffiles und bleiben bis `purge` **[verifiziert: M0-S05, M1-S17 (dpkg-Standard)]** → Autostart-Eintrag nach `remove` vorhanden, aber dank `TryExec` inaktiv; nach `purge` entfernt (P34).
- **Paket-Update:** dpkg ersetzt das Binary per `rename`, das alte Inode bleibt offen → der Daemon prüft `/proc/self/exe` auf «(deleted)» sowie den Inode-Vergleich `(st_dev, st_ino)` ohne zusätzliche `inotify`-Abhängigkeit **[verifiziert: M0-S10]**; der automatische Neustart nach dem laufenden Lauf nutzt diese Erkennung ab der Paket-Update-Integration (E11).
- **Daemon ↔ GUI (F14):** Nur der Daemon schreibt Config und Zustand; die GUI ändert über D-Bus-Aufrufe (`core` validiert). Bus-Name als Single-Instance-Lock (Autostart + GUI starten keinen zweiten Daemon). Interface versioniert (`net.daester.Kestic1`), eigenes Artefakt [dbus-api.md](dbus-api.md). Die GUI meldet dem Daemon `WindowActive(bool, page)` (U17), damit er Hinweise im Fenster statt doppelt anzeigen kann; Systembenachrichtigungen bleiben gemäss F3.
- **[verifiziert: Referenzsystem]** Plasma nutzt `systemd-xdg-autostart-generator`: XDG-Autostart-Einträge laufen als `app-…@autostart.service`.
- **Eigener Scheduler im Daemon:** berechnet fällige Läufe aus Zeitplan + letztem erfolgreichem Lauf. **Nachholen:** beim Daemon-Start und nach dem Aufwachen aus dem Suspend (logind-Signal `PrepareForSleep`) werden überfällige Läufe gestartet, sofern die Laufbedingungen erfüllt sind. Die Uhr ist injizierbar (Clock-Trait in `core` ab M1, damit M5 nicht von M3 abhängt; P36).
- **GUI startet den Daemon bei Bedarf** (z. B. nach Absturz oder deaktiviertem Autostart) und zeigt an, wenn er nicht läuft. Ohne laufenden Daemon gibt es keine geplanten Läufe → Monitoring-Warnung (FR-12).
- **Tray = sichtbares Gesicht des Daemons** (gleicher Prozess, `ksni`): Status, Fortschritt (Prozent, verarbeitete Bytes, geschätzte Restzeit aus den `backup --json`-Status-Meldungen, P5), nächster Lauf, «Jetzt sichern», «Pausieren», «Laufendes Backup abbrechen» (FR-21), «Kestic öffnen», «Dateien wiederherstellen…» (ab M4), «Kit erzeugen…» (ab M6), «restic aktualisieren…» (ab M5), «Kestic beenden…» (NFR-11). **SNI-Status (K1 = B):** Tray-Eintrag immer registriert, solange der Daemon läuft; Status und Overlay je Gesamtstatus gemäss Tabelle in [ui/tray-und-benachrichtigungen.md](ui/tray-und-benachrichtigungen.md) §10.1: `NeedsAttention` bei blockierendem Zustand, Fehler, überfällig; `Active` bei Warnung, Lauf, Pause; sonst `Passive` (auch wartet, unterbrochen, nie gelaufen); Plasma zeigt `Passive` im ausgeklappten Bereich **[verifiziert: Referenzsystem Plasma 6.6.6 (M0-S11, M1-S11)]**.
- **Benachrichtigungen:** Der Daemon sendet über `org.freedesktop.Notifications` (`notify-rust`); Plasma zeigt sie im Benachrichtigungsbereich und in dessen Verlauf. Weil der Daemon weiterläuft, kann er auf Aktionen reagieren («Details» öffnet die GUI beim Lauf, «Erneut versuchen»).
- **Vorteil:** gleicher Mechanismus nativ und im Flatpak (Background-Portal), ohne systemd-Timer, Laufbedingungen und Signale an einem Ort.

## Packaging

| | Nativ (`.deb`, MVP) | Flatpak (Phase 2) |
| --- | --- | --- |
| restic | System-restic ≥ 0.18.0 oder App-verwaltet (FR-33) | gebündelt |
| Autostart Daemon | XDG-Autostart (`/etc/xdg/autostart/`, E11) | Background-Portal (`RequestBackground` mit `autostart`); KDE-Portal implementiert `Background` **[verifiziert: Referenzsystem]** |
| Dateizugriff | voll (Benutzer) | `--filesystem=home` bzw. `host` |
| Secrets | Secret Service (KWallet) via Rust-Crate | siehe dieses Dokument |
| `restic mount` (FUSE) | möglich | problematisch → Browsing via `ls --json` |
| Root-/Systembackup | ausser Scope | ausser Scope |

- Entwicklung nativ einfacher (keine Sandbox), Verteilung per Flatpak einfacher (ein Paket für alle Distros); Befehlsdarstellung (FR-34) packaging-abhängig.
- **Build des `.deb` nur über `just deb`:** Es setzt `--remap-path-prefix` (Rust) und `-ffile-prefix-map` (C++), damit `/usr/bin/kestic` keine Pfade des Build-Rechners enthält (NFR-01); ein direktes `cargo deb` tut das nicht. Die Dev-Fallbacks für Icons und Übersetzungen aus dem Quellbaum gibt es nur in Debug- und `test-harness`-Builds.
- **Build-Stand im Journal:** Die Startzeile des Daemons nennt Paketversion und Git-Stand des Builds (`kestic-daemon 0.1.0 (a561018[-dirty]): started (pid …)`), ermittelt von `daemon/build.rs` über `git`, ohne `git` oder ausserhalb eines Checkouts `unknown`. Nach einem Paket-Update (M3-S11) unterscheidet sich vorher und nachher nur dieser Stand (AC-M3-11-4).

## Native Pakete

**[verifiziert: Referenzsystem]** Referenzsystem TUXEDO OS 24.04.5 (Ubuntu-Basis noble): Plasma 6.6.6, KDE Frameworks 6.24.0, Kirigami 6.24.0 (`qml6-module-org-kde-kirigami`), Qt 6.10.2. Das restic aus den Ubuntu-24.04-Paketquellen ist 0.16.4 (zu alt, FR-33).

- Ein auf dem Referenzsystem TUXEDO OS gebautes `.deb` braucht Qt ≥ 6.10 und KF ≥ 6.24.
  - Bibliotheken: Abhängigkeiten berechnet `dpkg-shlibdeps`.
  - QML-Module (Kirigami, QtQuick) werden erst zur Laufzeit geladen → **manuell** als `Depends` mit Mindestversion, z. B. `qml6-module-org-kde-kirigami (>= 6.24)`, `qml6-module-org-kde-kirigamiaddons-datetime (>= 1.11)`, `qml6-module-org-kde-kirigamiaddons-tableview (>= 1.11)` (laut [ui/index.md](ui/index.md), freigegeben; Version 1.11.0 nur auf dem Referenzsystem TUXEDO OS verifiziert, für Ubuntu 26.04 und KDE neon nicht geprüft).
- `Recommends: restic (>= 0.18.0)`. **[verifiziert: Referenzsystem, Test-Paket mit `apt-get -s install`]** apt listet restic unter «Empfohlene Pakete», installiert es nicht und installiert das Paket trotzdem.
- **Passende Systeme** (laut packages.ubuntu.com / packages.debian.org / Release-News):

| System | Plasma / KF / Qt | passt |
| --- | --- | --- |
| TUXEDO OS 24.04 | 6.6.6 / 6.24 / 6.10.2 | ✅ Referenz |
| Ubuntu/Kubuntu 26.04 | 6.6.4 / 6.24 / 6.10.2 | ✅ |
| KDE neon (Ubuntu-24.04-Basis) | 6.8.1 / 6.31 / Qt-6-Minor nicht geprüft | ✅ voraussichtlich |
| Debian 13 | 6.3.6 / 6.13 / 6.8.2 | ❌ |
| Kubuntu 24.04 | 5.27.11 (Plasma 5) | ❌ |

- Mindest-Basis = TUXEDO. Breitere Basis nur durch Bauen auf dem ältesten Zielsystem (Container).
- Andere Paketformate (rpm, AUR) nicht im MVP.

## Showcase-Seite (GitHub Pages)

Entscheid vom 2026-11-20 ([decisions.md](decisions.md)). Die englische Projektseite liegt in `site/` (Astro, TypeScript, pnpm) und erscheint unter `https://swissbite.github.io/kestic/`.

- **Werkzeuge:** Versionen exakt in `site/package.json` und `site/pnpm-lock.yaml`; pnpm über `packageManager`, Node über `devEngines.runtime` (pnpm lädt die gepinnte Version selbst). `site/pnpm-workspace.yaml` verlangt ein Mindestalter neuer Paketversionen (`minimumReleaseAge`) und lässt den optionalen Bilddienst `sharp` weg. `astro check` läuft vor jedem Build.
- **Screenshots:** `site/src/screenshots.ts` listet die gezeigten Bilder mit Grösse, Bildunterschrift und Alt-Text. Die Integration `site/src/integrations/screenshots.ts` kopiert genau diese Dateien aus `target/smoke-artifacts/` bzw. `KESTIC_SCREENSHOTS_DIR` nach `site/public/screenshots/`; fehlt ein Bild oder stimmt seine Grösse nicht, bricht der Build ab. Die Bilder werden nie committet.
- **Pipeline:** `.github/workflows/pages.yml` startet nach jedem erfolgreichen Verify-Lauf eines Pushs auf `main` (`workflow_run`, nur Pushs dieses Repos), lädt dessen Artefakt `smoke-artifacts`, baut die Seite aus dem aktuellen `main` (nie aus einem Ref aus den Event-Daten; ist `main` inzwischen weiter, folgt mit dessen Verify-Lauf ein weiterer Deploy) und veröffentlicht sie mit `actions/deploy-pages`; kein `gh-pages`-Branch. PRs, die `site/` ändern, bauen nur. Die Seite zeigt den Commit der Screenshots als «Development preview». Nach dem ersten Release (Release-Workflow, Q7) kommen die Screenshots aus dem Release, und die Seite erhält einen Download-Link.
- **Lokal:** `just site` (setzt einen Smoke-Lauf voraus), `just site-dev`, `just site-preview`. `just verify` braucht weiterhin kein Node.

## Secrets

- **Secret Service API** (`org.freedesktop.secrets`) über den direkten Client `SecretServiceStore` (`daemon/src/secrets.rs`) auf Basis von `zbus 6.3.0` (statt `secret-service`/`keyring`, vgl. [ADR 0001](adr/0001-tech-stack-rust-cxx-qt-kirigami.md)) → auf Plasma KWallet, auf GNOME GNOME Keyring.
- Pro Ziel: Alltags-Repo-Passwort + Backend-Credentials (REST, S3/B2/Azure-Keys, SFTP-Passphrase falls nötig). Die Recovery-Passphrase (FR-35) wird **nicht** gespeichert.
- Übergabe an restic: NFR-02.
- **[verifiziert: Referenzsystem]** KWallet stellt `org.freedesktop.secrets` bereit; mit PAM-Entsperrung (`libpam-kwallet5`) ist die Wallet nach dem Login offen, Autostart-Dienste laufen in der Session und erreichen KWallet. `secret-tool` (`libsecret-tools`) ist keine Voraussetzung.
- Ohne PAM-Entsperrung erkennt `SecretServiceStore` gesperrte Collections (`LockedOrPromptRequired`) und löst über `org.freedesktop.Secret.Prompt` die interaktive Entsperrung (`unlock_interactive()`) aus **[verifiziert: D-Bus-Fake in M0/M1 + Live-Unlock auf TUXEDO OS; isolierte PAM-freie Session bleibt für M6]** → App erkennt und meldet das.
- Bedrohungsmodell: siehe Abschnitt «Bedrohungsmodell» (F18).

**Flatpak (Phase 2):**

- **[verifiziert: Referenzsystem]** Das Secret-Portal implementiert KWallet (`kwallet.portal` aus Paket `kwallet6`; `kde-portals.conf`: `org.freedesktop.impl.portal.Secret=kwallet`), seit KDE Frameworks 6.2 ([KDE-Bug 466197](https://bugs.kde.org/show_bug.cgi?id=466197)).
- Das Portal liefert jeder App ein eigenes Secret ([Portal-Doku](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Secret.html)); dass libsecret damit eine verschlüsselte Datei im App-Ordner führt bzw. wo libsecret im Flatpak die Secrets ablegt: **[nicht belegt, in Phase 2 prüfen]**.
- Ein restic-Pfad aus der Sandbox (`/app/bin/restic`) gilt auf dem Host nicht; Secrets im App-Ordner findet ein Host-`secret-tool` nicht → FR-34 nutzt den App-Helfer `flatpak run --command=kestic <app-id> secret get …`.

## Zeitsteuerung, Laufbedingungen und Fehlerbehandlung

- **[verifiziert: Referenzsystem]** Plasma 6.6 läuft als systemd-User-Units (`plasma-plasmashell.service`), `Linger=no` → Session-gebundene Dienste laufen genau dann, wenn eine Session besteht.
- KDE hat keine eigene Zeitplanung für Anwendungen; KCron (KCM «Geplante Aufgaben», gepflegt, Qt 6) bearbeitet crontab. Kestic plant selbst im Daemon (dieses Dokument).
- **Autostart-Daemon statt systemd-Timer:** Abwägung (Sichtbarkeit, Beenden, Benachrichtigungen, Fortschritt, Laufbedingungen, Nachholen nach Suspend, übersprungene `Condition…=`-Starts, Flatpak, Ressourcen) in [ADR 0003](adr/0003-daemon-mit-xdg-autostart.md).
- **[verifiziert: Referenzsystem]** NetworkManager liefert per D-Bus `Metered` (global und pro Gerät), Gerätetyp und `Connectivity`; UPower liefert `OnBattery`.
- «Metered» ist bei NetworkManager eine Schätzung, u. a. nach Gerätetyp und DHCP-Option `ANDROID_METERED` ([NM-Doku](https://networkmanager.dev/docs/api/latest/nm-dbus-types.html)); `connection.metered` lässt sich pro Verbindung manuell setzen. Ob ein iPhone-Hotspot erkannt wird: nicht belegt → die robuste Option ist «nur in Netzwerk X».
- **Ziel nicht erreichbar – Erkennung (P32):** Vorab-Check vor jedem Lauf je Backend: Lokal/Laufwerk = Pfad vorhanden (F10); REST/S3/Cloud = TCP-Verbindung + HTTP-Antwort mit Connect-Timeout 10 s; SFTP = SSH-Verbindung mit `ConnectTimeout=10`; übrige Backends (B2, Azure, GCS, Swift, rclone) = `restic cat config` mit Kestic-Timeout 15 s. Scheitert er → `ausstehend` (Grund «Ziel nicht erreichbar»), restic wird nicht gestartet. Exit 1 eines laufenden restic wird nach Meldung eingeteilt: nicht erreichbar / DNS / Auth / sonst (Tabelle in `restic-integration.md`). Gestoppter Server → Status spätestens nach 15 s. Gelingt der Vorab-Check und bricht restic danach mit Verbindungsfehler ab, gilt der Lauf als vorübergehender Fehler (F2: `ausstehend` + Backoff). Nach erfolgreichem Vorab-Check kann restic Backend-Fehler intern länger wiederholen **[angenommen]**; Kestic setzt kein Laufzeit-Timeout, der Benutzer kann abbrechen (FR-21).
- **Zeitplan-Anker (AC-M3-01-8):** Ein neuer oder geänderter Zeitplan (Backup, Aufräumen, Prüfen) zählt ab dem Zeitpunkt der Änderung: Termine davor werden nicht nachgeholt, die Überfällig-Zählung eines Backups beginnt neu. Aufräumen und Prüfen haben je einen eigenen Anker; ein unveränderter Zeitplan behält seinen. Fällig ist ein Termin nach dem späteren von letztem Lauf und Anker, ohne beides nach der Registrierung. Persistenz in `history.db`: beim Backup der Anker `backup_state.baseline_at_unix`, beim Ziel `destination_state.prune_schedule_set_at_unix` und `check_schedule_set_at_unix`. Ein gespeicherter Zeitplan, der dem bisherigen gleicht, ändert nichts.
- **Neuversuch nach Fehler (F2):** Vorübergehende Fehler (Netzwerk, Ziel nicht erreichbar, Lock/Exit 11): Backoff 5 / 15 / 60 Min. (gezählt ab Ende des letzten Versuchs), danach nächster regulärer Termin; Alert erst bei «überfällig». Exit 11 beim `backup` selbst → `ausstehend` + Backoff; Exit 11 beim nachfolgenden `forget` → Lauf `Warnung` + «forget offen» (F4). Dauerhafte Fehler (Exit 12 falsches Passwort, Exit 10 Repository fehlt, abgelehnte Backend-Anmeldung): sofort Alert, kein Neuversuch, geplante Läufe gesperrt, bis ein Lauf des Backups wieder ohne solchen Fehler endet (z. B. nach «Gespeichertes Passwort aktualisieren…» und «Jetzt sichern»). Übrige Fehler (z. B. Platte voll, unbekannte Exit-Codes): sofort Alert, kein Neuversuch für den fälligen Termin, der nächste reguläre Termin läuft normal. `--retry-lock` wird bewusst **nicht** gesetzt; Kestic wiederholt selbst, damit Status und Backoff sichtbar bleiben **[verifiziert: restic-Default «no retries»]** (P33).
- **Ruhezustand / Deckel zu (F9):** **[verifiziert: Referenzsystem]** `systemd-inhibit`: Modus `delay` gibt es nur für `sleep` und `shutdown`, verzögert höchstens `InhibitDelayMaxSec` (Default 5 s); logind-Default `LidSwitchIgnoreInhibited=yes` → Deckel zu schläft trotz Inhibitor. Kestic setzt während eines Laufs: (1) **Inhibit über das Desktop-Portal** (`org.freedesktop.portal.Inhibit`, Flags Idle + Suspend; KDE-Portal implementiert `Inhibit` **[verifiziert: Referenzsystem]**) gegen automatischen Ruhezustand bei Inaktivität, sichtbar in Plasma; (2) logind-Inhibitor `sleep` im Modus `delay`, um restic bei Deckel zu sauber zu beenden → Lauf `unterbrochen`, nach dem Aufwachen `ausstehend`. Dass PowerDevil das Portal-Inhibit bei Inaktivität respektiert, ist belegt **[verifiziert: M0-S10, AC-M0-10-1; `PortalInhibitGuard` muss die `zbus::Connection` dauerhaft halten]**.

## Onboarding und Ziel-Typen

Ablauf FR-38 (5 Schritte):

1. **Quelle** – Default `$HOME`, ändern über den KDE-Dateidialog (Portal `FileChooser`).
2. **Ziel-Typ** – Liste sortiert nach «bereit» vor «braucht Installation» (Erkennung zur Laufzeit über vorhandene Programme; `rclone` ist typischerweise nicht vorinstalliert):

| Ziel-Typ | restic-Backend | braucht lokal |
| --- | --- | --- |
| Externes Laufwerk / USB | local | – (Erkennung Wechseldatenträger via UDisks2) |
| Ordner / anderer Datenträger | local | – |
| NAS / Server per SSH | sftp | `ssh` |
| REST-Server (z. B. auf dem NAS) | rest | – |
| Cloud-Speicher S3-kompatibel, Backblaze B2, Azure, Google Cloud, Swift | s3, b2, azure, gs, swift | – |
| Cloud-Dienste über rclone (Google Drive, OneDrive, Dropbox …) | rclone | `rclone` |

   - Für «braucht Installation»: Erklärung, Paketname, Knopf «In Discover öffnen» (`appstream://`-URL, von Discover unterstützt) bzw. Befehl `sudo apt install rclone`; danach erneut prüfen.
   - Nicht als eigener Typ: SSHFS-Mount (SFTP-Backend direkt ist einfacher), SMB/Windows-Freigabe (restic rät von CIFS ab; nur als bereits eingehängter Ordner mit Warnhinweis). Netzwerkorte aus Dolphin (`smb://`, `sftp://`) sind KIO-Adressen, keine Dateipfade; ohne `kio-fuse` für restic nicht nutzbar → Dateidialog auf lokale Pfade beschränken.
3. **Ziel-Details** – Formular je Typ, Verbindungstest, Erkennung «Repo existiert» → verbinden, sonst `init`.
4. **Passwort + Wiederherstellungs-Kit** (FR-35).
5. **Zusammenfassung** – Zeitplan, Ausschlüsse, Aufbewahrung mit Defaults, Grössenschätzung (FR-17) → «Jetzt sichern» oder «Speichern, später sichern».

## Tech-Stack

Stand recherchiert am 2026-11-15. Entscheid: [ADR 0001](adr/0001-tech-stack-rust-cxx-qt-kirigami.md).

- **Workspace:** `core` (reines Rust, ohne Qt testbar mit `cargo test`) / `daemon` (inkl. Tray, Notifications, CLI `kestic secret get`) / `gui` (CXX-Qt + Kirigami QML) / `i18n` (gemeinsame Texte und Codes) / `test-support` (nur Dev-Abhängigkeit für Tests, nie im Paket). GUI-Schicht dünn, Logik in `core`.
- **Crates (gewählt gemäss [ADR 0001](adr/0001-tech-stack-rust-cxx-qt-kirigami.md)):** `notify-rust`, `ksni`, `zbus` (inkl. direktem `org.freedesktop.secrets`-Client), `cargo-deb`, `cxx-qt`, `cxx-kde-frameworks` (Git-Commit in `Cargo.toml`).
- **Grund der Wahl:** Der Maintainer reviewt Rust eher als C++; Compiler fängt bei agentisch geschriebenem Code viel ab.
- **Risiko:** CXX-Qt im Spike erfolgreich validiert ([milestones/m0-spike.md](milestones/m0-spike.md)).
- **Abhängigkeits-Updates:** Renovate (`.github/renovate.json`) öffnet montags PRs für Crates (Minor/Patch gebündelt; die CXX-Qt-Crates und der Commit von `cxx-kde-frameworks` als eine Gruppe, weil `cxx-kde-frameworks` eine bestimmte CXX-Qt-Version voraussetzt), Toolchain mit `rust-version`, die Werkzeuge in `scripts/bootstrap.sh` (ausser der restic-Mindestversion), das Basis-Image des CI-Images, GitHub Actions und die Showcase-Seite; die Übersicht steht im Issue «Dependency Dashboard». Es gibt keine gesperrten Versionen ausser typescript `<7`: Ob eine Kombination passt, entscheidet `just verify`. Patch-, Digest-, Lockfile- und Minor-Updates ab 1.x mergt Renovate nach grünen Pflicht-Checks selbst; Majors, `0.x`-Minors und restic, rest-server und rclone prüft ein Mensch ([decisions.md](decisions.md), Zeile «Renovate: Automerge»).

**Grundlagen:**

- KDE dokumentiert «Kirigami mit Rust» offiziell über **CXX-Qt** (KDAB). Gewählt ist `cxx-qt` 0.10 (inkl. `cxx-qt-lib-extras` für `QApplication` + `org.kde.desktop`). Quellen: [Setup Rust](https://develop.kde.org/docs/getting-started/kirigami/setup-rust/), [Rust bei KDE](https://develop.kde.org/docs/getting-started/rust/).
- KF6-Bindings `cxx-kde-frameworks` (KDE Invent, als Git-Abhängigkeit auf einem Commit): genutzt für KI18n; Bindings für KNotifications, KStatusNotifierItem, Solid, KIO, KConfig sind nicht genutzt/nicht belegt. Quelle: [Rust bindings](https://develop.kde.org/docs/getting-started/rust/rust-bindings/).
- System-Integration ohne KF-Bindings über D-Bus in reinem Rust (Versionen bei der Wahl; aktuelle in `Cargo.toml` und `Cargo.lock`):

| Zweck | Crate | Version |
| --- | --- | --- |
| Benachrichtigungen (`org.freedesktop.Notifications`) | `notify-rust` | 4.19.0 |
| Tray-Icon (StatusNotifierItem) | `ksni` | 0.3.7 |
| Secret Service (`org.freedesktop.secrets`, KWallet, GNOME Keyring) | direkter Client via `zbus` | 6.3.0 |
| D-Bus allgemein (logind, UPower, NetworkManager, UDisks2, systemd, Portal) | `zbus` | 6.3.0 |
| `.deb`-Paket | `cargo-deb` | 3.8.0 |
| Qt-Brücke | `cxx-qt` | 0.10.0 |

**Packaging-Fallstricke (`.deb`):** Gebaut wird gegen die Qt-/KF-Version des Build-Systems → das ist die Mindestanforderung; ein nicht erfüllbares `Recommends: paket (>= x)` überspringt apt ohne Fehler (geprüft mit `apt-get -s install`); App-ID im Reverse-DNS-Format einer eigenen Domain bzw. `io.github.<owner>.<App>`, `org.kde.*` ist offiziellen KDE-Projekten vorbehalten.

## Code-Struktur

Verbindliche Ablage neuer und aufgeteilter Module über alle Meilensteine:

| Art | Ablage | Hinweis |
| --- | --- | --- |
| D-Bus-Methode | `daemon/src/service/<bereich>.rs` + `daemon/src/dbus_server.rs` + `daemon/src/dbus_api.rs` | `service.rs` nur als dünner Einstieg bzw. Modul-Verteiler |
| Fachlogik | `core/src/<bereich>.rs` | in der GUI nur Aufrufe, keine Geschäftslogik |
| Gemeinsame Texte und Codes | `i18n/src/` (Crate `kestic-i18n`) | nur was Daemon und GUI beide brauchen; Regel in [dbus-api.md](dbus-api.md) §Texte und Übersetzungen |
| GUI-Modell | `gui/src/models/<modell>.rs` | pro Modell eine Datei statt monolithischem `models.rs` |
| Dialog / Seite | eigene QML-Datei unter `gui/src/qml/` | nicht inline in `Main.qml` |
| Gemeinsame Test-Fixture | `test-support/src/<bereich>.rs` (Crate `kestic-test-support`, `publish = false`) | nur als `dev-dependency`; privater Session-Bus, Headless-Keyring, GUI-Smoke-Harness, Testwerkzeuge, erzeugte Testpasswörter (`password.rs`), Vorlage-Repos (`repo_template.rs`). Keine Test-Hooks im Produktivcode: Tests ersetzen Abhängigkeiten über Konstruktoren und Konfiguration (z. B. `DaemonRunConfig.secret_store`) oder über echte Dienste auf dem isolierten Bus. Test-Harness im GUI-Binary (Smoke-Szenen, synthetische Eingaben, `--simulate-status`) nur mit dem Cargo-Feature `test-harness`, nie im `.deb` |

**Grössengrenzen ohne Ausnahmen (NFR-12, durch `just verify` erzwungen):**

- **Funktionslänge:** Clippy-Lint `too_many_lines` mit Schwelle 150 Zeilen in `clippy.toml`.
- **Dateigrösse:** Maximal 1500 Zeilen pro `.rs`- und `.qml`-Datei unter `src/` (ohne Tests und generierten Code), geprüft durch `just verify`.

## Bedrohungsmodell (F18)

Gerüst; Ausarbeitung in M6-S08.

- **Explizit akzeptiert:** Prozesse desselben Benutzers können Secrets lesen (Secret Service, `kestic secret get`, `/proc/<pid>/environ` für Backend-Credentials per Env, NFR-02).
- **Risiken mit Gegenmassnahme:**
  - Kit-Datei mit Recovery-Passphrase im Klartext → Hinweistext und Export nicht in eine Backup-Quelle (F8, FR-35).
  - Export-Script → enthält keine Secrets, fragt sie ab (`read -s`) (F16, FR-24).
  - Secrets nie in argv, Logs oder Config (NFR-02); neue Passwörter über `--new-password-file` (P37).
  - Heruntergeladenes restic → `SHA256SUMS` + PGP-Signatur mit fest eingebautem Fingerprint (FR-33).
- **Offen (M6-S08):** Angreifermodell, Grenzen (Root, physischer Zugriff), Bewertung je Risiko.
