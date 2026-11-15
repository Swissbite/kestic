# ADR 0010: Lizenz und REUSE

- Status: angenommen
- Datum: 2026-11-15

## Kontext

Der Weg zu einem offiziellen KDE-Projekt soll offen bleiben ([KDE Licensing Policy](https://community.kde.org/Policies/Licensing_Policy)).

## Entscheid

| Dateien | Lizenz |
| --- | --- |
| Quellcode (Rust, QML) | `GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL` |
| Build-Dateien (`Cargo.toml`, `build.rs`, `justfile`) | `BSD-2-Clause` (Policy nennt CMake; Übertragung auf Cargo ist Auslegung) |
| Dokumentation, Icons | `CC-BY-SA-4.0` |
| Nicht schützbare Dateien | `CC0-1.0` |

REUSE 3.0 von Anfang an, `reuse lint` in `just verify`.

## Konsequenzen

Weg zum offiziellen KDE-Projekt: Incubation → KDE Review; Akzeptanz von Rust/CXX-Qt dort nicht belegt, für ein Hobby-Projekt nicht nötig.
