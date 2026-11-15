# ADR 0005: Konfiguration als TOML

- Status: angenommen
- Datum: 2026-11-15

## Kontext

KDE-Standard wäre KConfig/INI; für Rust sind KConfig-Bindings nicht belegt, TOML + `serde` ist naheliegend.

## Entscheid

`~/.config/kestic/config.toml` mit `config_version` + Migration, ohne Secrets (NFR-03, F13). Nur der Daemon schreibt (F14).

## Konsequenzen

Schema und Persistenz: [domain-model.md](../domain-model.md).
