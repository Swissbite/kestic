# Kestic

Native KDE-Plasma-App für Backups mit [restic](https://restic.net): so einfach wie ein Einsteiger-Backup-Tool, so mächtig wie restic selbst.

**Status:** Meilensteine M0 (Spike) und M1 (Walking Skeleton) abgeschlossen und auf TUXEDO OS manuell abgenommen; als Nächstes folgt M2 (Backup-Konfiguration).

- Spezifikation: [docs/spec/](docs/spec/) – Einstieg über [backlog.md](docs/spec/backlog.md) und [requirements.md](docs/spec/requirements.md)
- Für Agenten und Mitwirkende: [AGENTS.md](AGENTS.md)
- Oberfläche: [docs/spec/ui/index.md](docs/spec/ui/index.md)

Stack: Rust + CXX-Qt + Kirigami (QML), Referenzsystem TUXEDO OS (Plasma 6.6, KDE Frameworks 6.24, Qt 6.10), Mindestversion restic 0.18.0.

Lizenz: `GPL-3.0-or-later` (REUSE 3.0).
