# ADR 0010: Lizenz und REUSE

- Status: angenommen
- Datum: 2026-11-15 (aktualisiert 2026-11-16)

## Kontext

Der Weg zu einem offiziellen KDE-Projekt soll bei Bedarf offen bleiben ([KDE Licensing Policy](https://community.kde.org/Policies/Licensing_Policy)), gleichzeitig soll die Lizenz einfach zum Rust-Ökosystem (inkl. `Apache-2.0`- und `MPL-2.0`-Crates) passen und ohne KDE-e.V.-Proxy auskommen. Die KDE Licensing Policy erlaubt für Anwendungen ausdrücklich `GPL-3.0-or-later` zur Lizenzkompatibilität.

## Entscheid

| Dateien | Lizenz |
| --- | --- |
| Quellcode (Rust, QML) | `GPL-3.0-or-later` |
| Build-Dateien (`Cargo.toml`, `build.rs`, `justfile`) | `BSD-2-Clause` (Policy nennt CMake; Übertragung auf Cargo ist Auslegung) |
| Dokumentation, Icons | `CC-BY-SA-4.0` |
| Nicht schützbare Dateien | `CC0-1.0` |

REUSE 3.0 von Anfang an, `reuse lint` in `just verify`.

## Konsequenzen

Sauber kompatibel mit `Apache-2.0`- und `MPL-2.0`-Abhängigkeiten im Rust-Ökosystem (anders als `GPL-2.0-only`), zukunftssicher für spätere GPL-Versionen ohne `LicenseRef-KDE-Accepted-GPL`-Proxy, und zugleich durch die KDE Licensing Policy für Anwendungen gedeckt. Weg zum offiziellen KDE-Projekt: Incubation → KDE Review; Akzeptanz von Rust/CXX-Qt dort nicht belegt, für ein Hobby-Projekt nicht nötig.
