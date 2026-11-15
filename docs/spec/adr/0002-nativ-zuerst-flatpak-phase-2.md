# ADR 0002: Packaging: nativ zuerst, Flatpak in Phase 2

- Status: angenommen
- Datum: 2026-11-15

## Kontext

Entwicklung ist nativ einfacher (keine Sandbox), Verteilung per Flatpak einfacher (ein Paket für alle Distros). Flatpak hat keinen Zugriff auf Host-systemd und eigene Regeln für Secrets.

## Entscheid

MVP als `.deb` für TUXEDO OS (Referenz) und Debian/Ubuntu-basierte Systeme mit gleicher oder neuerer KDE-/Qt-Basis (NFR-01). Flatpak in Phase 2. Browsing über `restic ls --json` statt FUSE (`restic mount`), Befehlsdarstellung packaging-abhängig (FR-34).

## Konsequenzen

Abhängigkeiten und passende Systeme: [architecture.md](../architecture.md), Abschnitte «Packaging» und «Native Pakete». Andere Paketformate (rpm, AUR) nicht im MVP.
