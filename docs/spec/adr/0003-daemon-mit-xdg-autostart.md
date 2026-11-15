# ADR 0003: Ausführung: Hintergrunddienst per XDG-Autostart statt systemd-Timer

- Status: angenommen
- Datum: 2026-11-15

## Kontext

Backups sollen nach Zeitplan laufen, sichtbar für den Benutzer (NFR-11), mit Laufbedingungen und Signalen (Strom, Netzwerk, Suspend).

| | Autostart-Daemon mit Tray (gewählt) | systemd-Timer + Einmal-Prozess |
| --- | --- | --- |
| Sichtbarkeit für den Benutzer | ✅ Tray-Eintrag immer registriert; im Panel bei Lauf, Pause oder Problem, sonst im ausgeklappten Bereich | ❌ Backups laufen unsichtbar im Hintergrund |
| Läuft, wenn der Benutzer Kestic beendet | ❌ nein, mit Warnung (NFR-11) | ✅ ja |
| Benachrichtigungen | ✅ mit Aktionen («Details», «Erneut versuchen») | ⚠️ Prozess endet nach dem Lauf, Aktionen kaum nutzbar |
| Fortschritt live | ✅ im Tray | ⚠️ nur, wenn GUI offen ist und den Prozess abfragt |
| Laufbedingungen, Neuversuch bei Strom-/Netzwechsel | ✅ direkt per D-Bus-Signal | ⚠️ nur per periodischem Wecken |
| Nachholen nach Suspend | selbst gebaut (logind-Signal) | ✅ eingebaut: Kalender-Timer holen nach dem Aufwachen nach (systemd.timer) |
| Bedingung nicht erfüllt | App entscheidet, «ausstehend» | ❌ mit `Condition…=` übersprungener Start verfällt still (systemd.unit) |
| Flatpak | ✅ gleich (Background-Portal) | ❌ kein Host-systemd → zweiter Mechanismus nötig |
| Ressourcen im Leerlauf | ein Prozess (MVP mit Qt, später ohne, E17) | keine |

## Entscheid

Der Daemon startet per XDG-Autostart mit der Session (`/etc/xdg/autostart/<app-id>.daemon.desktop`, `Exec=kestic daemon --autostart`, `TryExec=/usr/bin/kestic`; E11, P34), ist einziger Executor und Zustands-Owner mit eigenem Scheduler. Keine systemd-Timer.

## Konsequenzen

Details: [architecture.md](../architecture.md), Abschnitte «Komponenten und Ausführung» und «Zeitsteuerung, Laufbedingungen und Fehlerbehandlung». Nachholen nach Suspend und Laufbedingungen sind Eigenbau (M3).
