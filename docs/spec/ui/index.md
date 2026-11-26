# GUI Design

Anforderungen an die Oberfläche von Kestic. Begriffe gemäss Glossar ([glossary.md](../glossary.md)). Allgemeine KDE-Regeln mit Quellen: KDE HIG ([develop.kde.org/hig](https://develop.kde.org/hig/)).

**Stand:** 2026-11-15. Festlegungen in §14 sind verbindlich; offene Punkte in §14.1.

**Grundlage:** [decisions.md](../decisions.md), [requirements.md](../requirements.md), [domain-model.md](../domain-model.md), [architecture.md](../architecture.md) und die mit ID markierten Festlegungen (E, F, Q, P, K, U – in den Spec-Dateien an der fachlichen Stelle); Meilensteine M1–M6; KDE HIG (Stand 2026-11-15). **[verifiziert: Referenzsystem]** = auf dem Referenzsystem geprüft (TUXEDO OS, Plasma 6.6.6, KF/Kirigami 6.24, Kirigami Addons 1.11).

## Teile

Kapitelnummern gelten über alle Teile; «§x» verweist auf das Kapitel in der Datei laut Tabelle, Verweise auf `../*.md` auf die übrigen Spec-Dateien. Dieses Index-Dokument (§0–§3, §14–§16) gilt für jede Aufgabe; dazu den Teil laden, um den es geht, und die in «Braucht zusätzlich» genannten Abschnitte (aus den tatsächlichen Verweisen je Teil ermittelt). Übrige §-Verweise, v. a. von §4 und §7 auf §8, sind Navigationsziele zu anderen Seiten; nur nachladen, wenn diese Seite mitgebaut wird.

| Kapitel | Datei | Laden für | Braucht zusätzlich |
| --- | --- | --- | --- |
| §0–§3, §14–§16 | dieses Dokument (dieses) | jede GUI-Aufgabe: Verbindlichkeit, Datensatz, Defaults, Leitlinien, HIG-Abweichungen, Festlegungen, Rückverfolgung | – |
| §4–§5 | [struktur.md](struktur.md) | Navigation, Seitenhierarchie, Fenstergrössen, Kirigami-Bausteine | §6.1, §6.5 |
| §6 | [zustaende.md](zustaende.md) | Status-Anzeigen, Wartet-Gründe, Ziel-Zustände, leer/lädt/offline/Fehler | §9.1, §9.4, §10.1, §12.2 |
| §7 | [hauptbereiche.md](hauptbereiche.md) | Übersicht, Backups, Ausschluss-Regeln, Zeitplan, Aufbewahrung, Wiederherstellen, Ziele, Verlauf | §4.1, §5, §6.1–§6.5, §9.1, §9.4, §12.2, §12.3 |
| §8 | [nebenseiten.md](nebenseiten.md) | Wartung, Kit, Übernehmen, Dialoge, Ziel zuweisen, Verbindung, Quelle, Konfiguration, Snapshots, Log, M1-Formulare | §4.1, §5, §6.1, §6.3–§6.5, §7.3, §7.6, §9.1, §9.3, §12.2, §12.3 |
| §9 | [ablaeufe.md](ablaeufe.md) | Assistenten, Fehlermuster, Beenden, Einstellungen | §5, §6.1, §8.3, §8.7, §8.9, §12.2 |
| §10 | [tray-und-benachrichtigungen.md](tray-und-benachrichtigungen.md) | Tray-Icon, SNI-Status, Benachrichtigungen, Fortschrittsanzeige | §6.1–§6.3, §8.2, §9.4–§9.6, §12.2 |
| §11–§13 | [stil-und-texte.md](stil-und-texte.md) | Theming, Icons, Systemintegration, Texte, destruktive Aktionen, Tastatur, Barrierefreiheit | §6.2, §6.3, §10.2 |

## 0. Mockups, Notation, Referenz-Datensatz

Gerenderte Entwürfe im Breeze-Stil (Farben aus den Plasma-Farbschemata Breeze/Breeze Dunkel, Icons aus dem Breeze-Thema), eingebettet in den Teilen. Zum Weitergeben: die PNGs in `assets/`; eine Gesamtansicht als eine HTML-Datei (`kestic-mockups.html`, alle Screens) erzeugt `python3 build_mockups.py` im Ordner `assets/`. PNGs neu erzeugen: `python3 build_mockups.py --png` im Ordner `assets/`.

### 0.1 Verbindlichkeit

- **Mockups sind nicht normativ.** Verbindlich sind Text, Komponenten (§5), Zustände (§6) und Texte (§12). Abnahme (M1-S13, M6-S09) prüft diese, nicht Pixel, Abstände oder Farbwerte.
- App-Icon und Tray-Symbol sind Platzhalter; Namen, Pfade und Adressen sind Beispiele aus dem Referenz-Datensatz (§0.2).
- Bewusste Abweichungen der Mockups von der echten App:
  - Farben und Schrift sind im Generator feste Breeze-Werte zur Illustration; die App nimmt sie ausschliesslich aus `Kirigami.Theme` und der System-Schrift (§11).
  - Steuerelemente sind nachgezeichnet, nicht Qt-gerendert; Grössen, Fokusrahmen und Animationen der echten Komponenten (`qqc2-desktop-style`) können abweichen.
  - Die Tab-Leiste ist vereinfacht; das echte Aussehen von `Kirigami.NavigationTabBar` gilt.
  - Hover-, Fokus- und Tastaturzustände sind nicht dargestellt.
  - Dunkle Varianten nur für einige Screens; U16 gilt für Screenshots der echten App, nicht für Mockups.
- Schmales Fenster: Bei 26 `gridUnit` passt «Wiederherstellen» im Mockup knapp in die Tab-Leiste; ob das mit Kirigami und längeren Übersetzungen hält, prüft der Spike (§15).

### 0.2 Referenz-Datensatz

Alle Beispiele in Text und Mockups beziehen sich auf diesen Stand: **Sonntag, 15.11.2026, 14:32**, Benutzer `/home/user`.

| Backup | Quelle → Ziel | Zeitplan | Letzter Lauf | Status jetzt |
| --- | --- | --- | --- | --- |
| Persönlicher Ordner | `~` → NAS | täglich 12:00 · am Strom, mit Netzwerk | heute 12:00, ok, 312 MB neu (gestern 12:00: Warnung, 2 Dateien nicht lesbar, 45 MB) | Gesichert · vor 2 Stunden; nächstes morgen 12:00 |
| Fotos | `/media/fotos` (Laufwerk) → USB-Disk | sonntags 10:00 | 08.11.2026 10:00, ok | Wartet: Quelle nicht verfügbar (seit heute 10:00) |
| Projekte | `~/Projekte` → S3 | Mo, Mi, Fr 20:00 | Fr 13.11.2026 20:05, Fehler: S3 hat das gespeicherte Passwort abgelehnt (letzter Erfolg Mi 11.11.2026 20:04) | Fehlgeschlagen, nicht überfällig (nur ein Termin verpasst); nächstes Mo 16.11. 20:00 |

| Ziel | Typ | Zustand | Kit | Wartung |
| --- | --- | --- | --- | --- |
| NAS | REST-Server | erreichbar · 1 Backup | erzeugt am 01.11.2026 | Aufräumen sonntags 12:00 (zuletzt heute 12:30), Prüfen am ersten Sonntag im Monat (zuletzt 01.11.2026) |
| USB-Disk | Laufwerk | nicht eingesteckt · 1 Backup | erzeugt am 25.10.2026 | Standard |
| S3 | S3-kompatibel | gespeichertes Passwort passt nicht · 1 Backup | **fehlt** | Standard |

- **Gesamtstatus:** «1 Backup fehlgeschlagen» (Rangfolge §6.2). Der Fehler von «Projekte» steht auf seiner Karte (§7.1); als Hinweis oben in der Übersicht nur: Kit fehlt für «S3». Variante «Lauf» (dunkle Übersicht, Tray): Persönlicher Ordner wird um 14:30 manuell gesichert (42 %, 7 MB/s, noch etwa 4 Minuten), Zeitpläne pausiert seit 13:00 bis 15:00.
- **Ausnahmen:** Willkommen und Assistent «Erstes Backup anlegen» (§9.1) zeigen den Zustand **vor** dem ersten Backup (gleicher Rechner, USB-Disk eingesteckt). Dialoge, die im Datensatz nicht vorkommen können, verwenden «Alte-Disk» (Ziel ohne Backups) bzw. «backup.example.org» (SFTP-Server). «Backups übernehmen» zeigt Backups eines anderen Rechners («notebook-alt»: Dokumente, Musik) auf dem NAS. Dieser Rechner heisst «notebook» (Host in nicht zugeordneten Snapshots aus einem früheren Script).

### 0.3 Notation in Aufbau-Listen

`[Knopf]`, `[✓]`/`[ ]` Checkbox bzw. Switch, `(•)`/`( )` Radio, `[… ▾]` ComboBox oder Knopf mit Menü, `⋮` Überlaufmenü, `›` öffnet Unterseite, `→` Folge bzw. Navigation. Struktur- und Ablaufdiagramme sind Mermaid (rendert u. a. auf GitHub). Verbindlich sind die Breeze-Icon-Namen (§6, §11.3), nicht Symbolzeichen. Beispielnamen sind Platzhalter aus §0.2.

### 0.4 Defaults und IDs

| Default | Wert | Quelle |
| --- | --- | --- |
| Zeitplan neues Backup | täglich 12:00, nur am Strom und mit Netzwerk | [requirements.md](../requirements.md) FR-03, FR-13 (P35) |
| Aufbewahrung | 7 tägliche, 4 wöchentliche, 12 monatliche (Vorlage «Standard») | [requirements.md](../requirements.md) FR-05 |
| Ausschlüsse neues Backup | `$HOME/.cache`, Papierkorb `$HOME/.local/share/Trash`, Download-Ordner (`xdg-user-dir DOWNLOAD`), `--exclude-caches` | [requirements.md](../requirements.md) FR-38 (P29) |
| Wartung | Aufräumen sonntags 12:00, Prüfen am ersten Sonntag im Monat 12:00; nur am Strom, nicht getaktet | [requirements.md](../requirements.md) FR-45, FR-11 (P15, F11) |
| Überfällig | nach 2 verpassten Terminen, Anzahl einstellbar (§7.4) | [glossary.md](../glossary.md) «überfällig» |
| Undo-Frist (harmlose Fälle) | 10 Sekunden; nie für «Aus Kestic entfernen…» oder «Ziel entfernen» (dort Bestätigungsdialog) | §12.3 |

IDs: FR/NFR = Anforderungen, AC = Akzeptanzkriterien, M…-S = Meilenstein-Stories ([backlog.md](../backlog.md)), E/F/Q/P = bestätigte Festlegungen in den Spec-Dateien (an der fachlichen Stelle erklärt), K/U = Festlegungen dieses Dokuments (§14).

## 1. Leitlinien

1. **Einfach als Standard, mächtig bei Bedarf** (HIG-Leitsatz; «so einfach wie Déjà Dup, so mächtig wie restic»). Der Endnutzer sieht Status, «Jetzt sichern», «Wiederherstellen». Ausschluss-Syntax, Befehle und Wartungsdetails liegen eine Ebene tiefer oder im Expertenmodus (E16).
2. **Status zuerst:** Fenster, Tray und Benachrichtigung beantworten zuerst «Sind meine Daten gesichert?», dann «Was kann ich tun?».
3. **Wiederherstellen ist eine Hauptaufgabe:** eigenes Navigationsziel, in höchstens zwei Klicks erreichbar.
4. **Nichts Verstecktes:** Was der Daemon tut, ist sichtbar (NFR-11). Jeder restic-Aufruf ist über «Befehl anzeigen» sichtbar und kopierbar (FR-34, K2).
5. **Nativ statt eigen:** Kirigami- und Qt-Quick-Controls-Komponenten im System-Stil (`qqc2-desktop-style`); Kirigami Addons nur, wo Kirigami nichts bietet (U2). Keine eigenen Farben, Schriften, Abstände, Icon-Sätze (§11).
6. **Sicher bei Destruktivem:** Undo nur, wo es wirklich alles zurückholt; sonst Vorschau + Bestätigung mit klarer Folge; nie stilles Überschreiben (NFR-08, §12.3).
7. **Gleiche Begriffe überall:** UI-Texte aus dem Glossar, ein Begriff pro Ding in GUI, Tray, Benachrichtigung, Fehlertext; keine doppelte Beschriftung im selben Fenster (HIG).

## 2. HIG-Konformität und bewusste Abweichungen

Kestic folgt der KDE HIG. Wo Kestic bewusst abweicht oder eine HIG-Regel auslegt, steht es hier.

| # | HIG sagt | Festlegung für Kestic |
| --- | --- | --- |
| **K1** | System-Tray nur als letztes Mittel; **opt-in**; **nur sichtbar bei abweichendem Status und ohne offenes Fenster**; nicht als Tray weiterlaufen, wenn das Fenster schliesst ([Status changes](https://develop.kde.org/hig/status_changes/), [Simple by default](https://develop.kde.org/hig/simple_by_default/)) | Tray-Eintrag immer registriert, solange der Daemon läuft; SNI-Status und Overlay nach §10.1. **Bewusste Abweichung:** kein opt-in, weil Kestic keine versteckten Hintergrundaktivitäten haben darf (NFR-11). Ergänzend die von der HIG bevorzugten Wege: Fortschrittsanzeige im Benachrichtigungsbereich (FR-53), Benachrichtigungen, Dienstzustand im Hauptfenster (§6.1), Selbsttest beim GUI-Start. KDE-Präzedenz für einen Backup-Tray-Daemon: Kup **[verifiziert: Referenzsystem: `kupdaemon.notifyrc`]**. |
| **K2** | Einstellungen als echte Verhaltens-Alternativen, keine «Advanced»-Bereiche, stattdessen Progressive Disclosure ([Powerful when needed](https://develop.kde.org/hig/powerful_when_needed/)) | «Befehl anzeigen» (FR-34) für alle im ⋮-Menü und als Tab «Befehle» im Lauf-Detail. Der Schalter «Expertenmodus» (E16) ist nur eine Darstellungsart für Ausschluss-Syntax und Expertenoptionen, in-place (zusätzlicher Abschnitt), keine eigene Experten-Seite. **Keine Funktion dauerhaft versperrt:** Wo der einfache Modus etwas nur zeigt (F12), bietet ein Link an, den Expertenmodus einzuschalten – mit `PromptDialog` «Expertenmodus einschalten?», nie stillschweigend. |
| **K3** | ≤ 5 Navigationsziele → `Kirigami.NavigationTabBar`, nicht Seitenleiste ([Layout and navigation](https://develop.kde.org/hig/layout_and_nav/)) | Tab-Leiste mit 5 festen Bereichen (§4.1). Spike: Labels bei Mindestbreite nicht abgeschnitten (§15). |

Weitere übernommene HIG-Folgen: Formulare mit `Kirigami.FormLayout` (U2), app-weite Einstellungen sofort wirksam (U3), Radio nur bis 3 Optionen, Baumansichten vermeiden (U8), Benachrichtigungen nicht doppelt bei sichtbarer Seite (§10.2), Kürzel nie nur Funktionstaste (§13.1), Fehler aus Drittsoftware als solche kennzeichnen (§9.4), Wizard nur für zwingende Schritte (§9.1), keine gleichen Beschriftungen in einem Fenster.

## 3. Oberflächen von Kestic

| Oberfläche | Prozess | Technik | Zweck |
| --- | --- | --- | --- |
| Hauptfenster | `kestic` (GUI) | `Kirigami.ApplicationWindow` | Status, Wiederherstellen, Konfiguration, Verlauf |
| Assistenten | GUI | lineare Seitenfolge im Hauptfenster | erstes Backup (FR-38), neues Backup, Ziel hinzufügen, neuer Rechner (FR-39) |
| Einstellungen | GUI | eigenes Fenster, eine `FormLayout`-Seite (U3) | app-weite Optionen |
| Tray-Icon + Menü | `kestic daemon` | StatusNotifierItem (`ksni`) | Status, Schnellaktionen (NFR-11, K1) |
| Benachrichtigungen | Daemon | `org.freedesktop.Notifications` (`notify-rust`) + `kestic.notifyrc` | Fehler, Warnungen, überfällig (F3) |
| Fortschrittsanzeige im Benachrichtigungsbereich | Daemon | D-Bus `org.kde.JobViewServerV2` (U5) | laufender Lauf wie eine Dateikopie in Dolphin |
| Fortschritt im Task-Manager | GUI | `QGuiApplication::setBadgeNumber()` | Prozent bei manuell gestartetem Lauf, solange die GUI läuft (FR-54, P2) |

Das Hauptfenster ist nie Voraussetzung für einen Lauf (NFR-04). Fenster schliessen beendet nur die GUI (§13.2).

## 14. Festlegungen

| # | Thema | Festlegung | Betrifft |
| --- | --- | --- | --- |
| K1 | Tray vs. HIG | Tray-Eintrag immer registriert; SNI-Status und Overlay nach §10.1 | [decisions.md](../decisions.md), NFR-11, [architecture.md](../architecture.md) («Komponenten und Ausführung»), [architecture.md](../architecture.md) («Zeitsteuerung, Laufbedingungen und Fehlerbehandlung»), M1-S11, AC-M1-11-1, AC-M3-07-4 |
| K2 | Expertenmodus | «Befehl anzeigen» für alle; Expertenmodus nur Darstellungsart (Ausschluss-Syntax, Expertenoptionen), in-place, Einschalten per Bestätigung | [decisions.md](../decisions.md), FR-34, Glossar |
| K3 | Navigation | `NavigationTabBar` mit 5 festen Bereichen (Übersicht, Wiederherstellen, Backups, Ziele, Verlauf), unabhängig von der Anzahl Backups oder Ziele | M1-S13, AC-M1-13-1 |
| U1 | Navigationsziele | Übersicht (Start), Wiederherstellen, Backups, Ziele, Verlauf; Hamburger-Menü; Zustandsleiste | M1-S13 (M1 ohne «Wiederherstellen») |
| U2 | Formulare + Abhängigkeit | `Kirigami.FormLayout`; Kirigami Addons nur `dateandtime`, `tableview`; `Depends` `qml6-module-org-kde-kirigamiaddons-datetime` und `-tableview` (>= 1.11) | [architecture.md](../architecture.md) («Native Pakete»), AC-M1-17-1 bzw. M2/M4 |
| U3 | Einstellungen | eigenes Fenster, eine FormLayout-Seite, sofort wirksam ausser Aktionen mit Folgen (Knopf) | – |
| U4 | Breadcrumbs | `pageStack.globalToolBar` im Breadcrumb-Stil | Spike |
| U5 | Fortschrittsanzeige | FR-53 «Fortschrittsanzeige in Plasma» (MVP, M3-S12); Taskleisten-Fortschritt FR-54 (P2) | [requirements.md](../requirements.md), M3 |
| U6 | notifyrc | `kestic.notifyrc` + Hint `desktop-entry`; Installation nach `/usr/share/knotifications6/` | M1-S11, M1-S17, AC-M1-17-1, M3-S08 |
| U7 | Dringlichkeit | Fehler/überfällig `Normal` + persistent; nie `Critical`/`Low` | F3 |
| U8 | Snapshot-Browser | Liste mit Breadcrumb statt Baum (HIG) | M4-S03, AC-M4-03-3 |
| U9 | Fenster schliessen | Ctrl+Q/Kreuz = nur GUI; Hinweis einmalig + dauerhaft in Einstellungen/Handbuch | NFR-11, M1-S11 |
| U10 | Anrede | neutral, sonst Sie-Form | M1-S16 |
| U11 | Übersetzung | msgid Englisch, DE in `.po` (de_DE nach KDE-Praxis), Spec DE-CH | NFR-06, M1-S16 |
| U12 | Kürzel | Ctrl+Shift+B «Jetzt sichern» | M0-Spike |
| U13 | Glossar | UI-Begriffe: «Übersicht», «Verlauf» (= Lauf-Historie), «Aufräumen» (`prune`), «Prüfen» (`check`), «Nicht zugeordnet» (FR-36), «Verwaist», «Wartet» (UI für `ausstehend`), «Ausnahme» (Negation), «Sperre» (Lock), «Fortschrittsanzeige», «Passwortspeicher», «Systemabschnitt» | [glossary.md](../glossary.md) |
| U14 | Icons Zustände | `chronometer` = wartet, `state-pause` = pausiert, `alarm-symbolic` = überfällig, `state-warning` = Warnung | §6.2 |
| U15 | Assistenten | linear, nur zwingende Schritte; Repo/Keys erst beim Abschluss; wiederverwendet für «Neues Backup», «Ziel hinzufügen», «Neuer Rechner» | M6-S01, M6-S06, FR-38 |
| U16 | Screenshots | je Screen in Breeze und Breeze Dunkel, DE + EN (automatisch, `just verify`); Hoher Kontrast und RTL prüft ein Mensch (AC-M6-09-7) | Q3, `just verify`, M6-S09 |
| U17 | Benachrichtigungen | MVP: Daemon sendet immer; GUI meldet `WindowActive` (F14). **FR-55 (P2):** Unterdrückung nur, wenn Fenster aktiv und betroffene Seite sichtbar; `WindowActive(bool, page)` | F3, F14, M1-S09, `dbus-api.md` |
| U18 | Speichern in Editoren | Backup-Editoren mit `[Speichern]` = `UpdateBackup`; Zwischenstand nur in der GUI | M2-S02, M2-S06 (AC) |
| U19 | Zustandsleiste | blockierende Zustände Daemon → Passwortspeicher → restic im Fensterkopf, Aktionen deaktiviert; Selbsttest beim GUI-Start | M1-S13, M5-S07, M6-S11 |

### 14.1 Offen

Keine offenen Punkte.

## 15. Folgen für Spike und Meilensteine

Prüfungen im Spike ([m0-spike.md](../milestones/m0-spike.md), Kriterium 7 «GUI-Grundlagen»; M0-S11) – **alle abgeschlossen**:

| Prüfung | Ergebnis (Spike, Referenzsystem Plasma 6.6.6 / Kirigami 6.24 / Qt 6.10.2) |
| --- | --- |
| `NavigationTabBar` + `pageStack`-Spalten + Breadcrumb in Kirigami 6.24 | **belegt (Offscreen breit/schmal):** lädt ohne QML-Warnungen. Finding für Qt 6.10: `Component`-Seiten beim `pageStack.push`/`replace` über `comp.createObject(pageStack, props)` instanziieren (sonst QML-Warnung `Created graphical object was not placed in the graphics scene`) |
| Tab-Bar bei Mindestbreite ohne abgeschnittene Labels («Wiederherstellen»), DE + EN | **belegt (Offscreen bei 26 `gridUnit`):** kein Label abgeschnitten (`truncated == false`, alle 5 EN- und DE-Tabs lesbar). Finding: `Kirigami.NavigationTabBar` setzt `maximumContentWidth = gridUnit * 5 * count` (`~90 px`/Tab), sodass «Wiederherstellen» in zwei Zeilen umbricht (`Wiederherstel` / `len`); in M1-S13 im breiten Fenster `maximumContentWidth` pro Tab entsprechend erweitern |
| SNI-Status `Passive` in Plasma 6.6: wird das Icon in den Überlauf geklappt, wechselt es bei `Active` sichtbar zurück | **belegt (manuelle Abnahme):** `Active` sichtbar im Panel, `Passive` wandert ins `^`-Überlauf-Menü, `NeedsAttention` mit Overlay + `Active` wieder sichtbar im Panel |
| Tray-Icon-Umfärbung per `IconThemePath` (hell/dunkel) | **belegt (manuelle Abnahme):** SVG mit `.ColorScheme-Text` färbt sich beim Wechsel Breeze ↔ Breeze Dunkel automatisch um |
| `JobViewServerV2` aus Rust (zbus) | **belegt (manuelle Abnahme + D-Bus):** zeigt Fortschrittsbalken, Text, Geschwindigkeit und «Abbrechen» (`cancelRequested` kommt an). Finding: `JobViewHandle` muss die `zbus::Connection` dauerhaft halten (sonst wertet `kuiserver` das Beenden der Verbindung sofort als Abbruch/Fehlschlag) |
| `notifyrc`-Ereignisse ohne KNotifications | **belegt (manuelle Abnahme): wirkt nicht** – `kestic.notifyrc` erscheint in `kcm_notifications` separat unter *Systemdienste → Kestic*, `desktop-entry` unter *Anwendungen → Kestic*; Deaktivieren eines Ereignisses unterdrückt `notify-rust`-Popups nicht. Eigene F3-Schalter in Kestic (§9.6) sind erforderlich |
| Modulnamen Systemeinstellungen (Autostart, Benachrichtigungen) | **belegt (manuelle Abnahme + `kcmshell6 --list`):** `kcm_autostart`, `kcm_notifications` |
| Ctrl+Shift+B ohne Konflikt | **belegt (GUI-Smoke + manuelle Abnahme):** löst «Jetzt sichern» in Kestic aus; kein Konflikt mit globalen Plasma-Kürzeln |

Weitere Folgen: M1-S13 nach §7 (Tab-Bar, Hamburger-Menü, Zustandsleiste, Placeholder; `maximumContentWidth` im breiten Layout anpassen); M2-S02/S06, M4-S03/S05, M6-S01 mit §7.3, §7.5, §7.6, §9.1 als Abnahmegrundlage; M6-S09-Checkliste = §12.3 + §13.

## 16. Rückverfolgung FR → Screen

| FR | Abschnitt |
| --- | --- |
| FR-01, FR-15 | §7.7, §8.9, §9.1 Schritt 2–3, §9.3 |
| FR-02, FR-27 | §7.3 |
| FR-03, FR-13 | §7.4 |
| FR-04 | §7.2, §8.11 |
| FR-05 | §7.5 |
| FR-06, FR-07, FR-10, FR-36 | §7.6, §8.12, §8.14 |
| FR-11, FR-45 | §7.7, §8.1 |
| FR-12 | §6, §7.1, §10 |
| FR-13a, FR-13b | §7.2, §10.1 |
| FR-17 | §7.3 Vorschau, §9.1 Schritt 5 |
| FR-18, FR-34, F4 | §7.8, §8.13 |
| FR-19 | §6.4, §8.5 |
| FR-21 | §7.1, §10.1, §10.3 |
| FR-24 | §4.1, §8.6 |
| FR-28, FR-31 | §7.2, §8.10, §9.2 |
| FR-29, FR-30 | §6.3, §7.2, §10.2 |
| FR-33, FR-51, NFR-05 | §6.1, §9.6 |
| FR-35 | §8.2, §9.1 Schritt 4b |
| FR-37 | §8.4, §9.4 |
| FR-38, FR-39 | §9.1 |
| FR-48 | §7.2 Gefahrenbereich, §12.3 |
| FR-49 | §8.3, §8.14 |
| FR-53 | §3, §10.3 |
| FR-54 | §3, §11.4 |
| FR-55 | §10.2 |
| FR-56 | §7.2 ⋮, §7.7 ⋮, §8.8 (Ziel zuweisen), §8.9 (Ziel bearbeiten), §12.3 |
| FR-57 | §8.4 |
| NFR-08 | §12.3 |
| NFR-09 | §13 |
| NFR-11 | §2 K1, §9.5, §10.1, §13.2 |
