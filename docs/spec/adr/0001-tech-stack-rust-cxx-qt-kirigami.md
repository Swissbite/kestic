# ADR 0001: Tech-Stack Rust + CXX-Qt + Kirigami

- Status: vorläufig, wird im Spike M0 finalisiert (Versionen pinnen, CXX-Qt 0.8.1 vs. 0.10.0)
- Datum: 2026-11-15

## Kontext

Kestic ist eine native KDE-Plasma-App und wird agentisch umgesetzt. Kandidaten: C++/Kirigami, Rust/CXX-Qt/Kirigami, Python/PySide6.

| | C++/Kirigami | Rust/CXX-Qt/Kirigami | Python/PySide6 |
| --- | --- | --- | --- |
| KDE-Look (Kirigami) | ✅ nativ | ✅ via QML | ⚠️ möglich, wenig Beispiele |
| KF6-Bibliotheken | ✅ alle | ⚠️ wenige, Rest via D-Bus-Crates | ❌ kaum |
| Reife / Beispiele | ✅ hoch | ⚠️ jung | ✅ hoch (Qt), ❌ (KDE) |
| Agentische Umsetzung | ⚠️ Speicherfehler erst zur Laufzeit | ✅ Compiler fängt viel ab; ⚠️ CXX-Qt-Makros wenig bekannt | ✅ schnell, ⚠️ Fehler erst zur Laufzeit |
| Kern ohne GUI testbar | ⚠️ mit Aufwand | ✅ sehr gut | ✅ gut |
| Review durch den Maintainer | ⚠️ | ✅ bevorzugt | – |

## Entscheid

Rust + CXX-Qt + Kirigami (QML). Workspace `core` (reines Rust) / `daemon` / `gui`. Im MVP ein Binary `kestic` mit Unterbefehlen, der Daemon lädt Qt; Trennung in `kestic-daemon` ohne Qt später (E17, FR-52, P3). Details: [architecture.md](../architecture.md), Abschnitt «Tech-Stack».

## Konsequenzen

- KF6-Bindings (`cxx-kde-frameworks`) sind jung; System-Integration über D-Bus-Crates (`notify-rust`, `ksni`, `secret-service`/`keyring`, `zbus`).
- Risiko CXX-Qt → Spike zuerst ([milestones/m0-spike.md](../milestones/m0-spike.md)).
- i18n über KI18n (E12, NFR-06).
