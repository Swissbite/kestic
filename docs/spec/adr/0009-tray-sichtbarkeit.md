# ADR 0009: Tray-Sichtbarkeit (K1 = B)

- Status: angenommen
- Datum: 2026-11-15

## Kontext

Die KDE HIG sieht ein Tray-Icon nur als Ausnahme vor; NFR-11 verlangt, dass Kestic nichts versteckt.

## Entscheid

Tray-Eintrag immer registriert, solange der Daemon läuft; SNI-Status nach Gesamtstatus: `NeedsAttention` bei blockierendem Zustand, Fehler, überfällig; `Active` bei Warnung, Lauf, Pause; sonst `Passive` (ausgeklappter Bereich **[angenommen, Spike]**). Tabelle: [ui/tray-und-benachrichtigungen.md](../ui/tray-und-benachrichtigungen.md) §10.1.

## Konsequenzen

Bewusste Abweichung von der HIG zugunsten von NFR-11.
