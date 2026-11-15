# ADR 0004: Ein Secret-Speicher: Secret Service

- Status: angenommen
- Datum: 2026-11-15

## Kontext

Repo-Passwörter und Backend-Credentials dürfen nie im Klartext in der Config liegen (FR-08).

## Entscheid

Ein Speicher: Secret Service API (`org.freedesktop.secrets`) → auf Plasma KWallet, auf GNOME GNOME Keyring. Bestehende Secrets werden in den Secret Service übernommen; andere Passwortmanager werden nicht angebunden. Übergabe an restic gemäss NFR-02 (`RESTIC_PASSWORD_COMMAND` → `kestic secret get`).

## Konsequenzen

Bedrohungsmodell und Flatpak-Fragen: [architecture.md](../architecture.md), Abschnitte «Secrets» und «Bedrohungsmodell».
