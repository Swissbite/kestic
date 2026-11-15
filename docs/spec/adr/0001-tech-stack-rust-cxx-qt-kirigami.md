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

Rust + CXX-Qt (`=0.10.0`) + Kirigami (QML). Workspace `core` (reines Rust) / `daemon` / `gui`. Im MVP ein Binary `kestic` mit Unterbefehlen, der Daemon lädt Qt; Trennung in `kestic-daemon` ohne Qt später (E17, FR-52, P3). Details: [architecture.md](../architecture.md), Abschnitt «Tech-Stack».

### CXX-Qt-Version (0.8.1 vs. 0.10.0, M0-S03)

Gepinnt auf **`cxx-qt = "=0.10.0"`** (inkl. `cxx-qt-lib = "=0.10.0"`, `cxx-qt-lib-extras = "=0.10.0"`, `cxx-qt-build = "=0.10.0"`):

- `cxx-kde-frameworks` (`master` auf KDE Invent, benötigt u. a. für KI18n) setzt selbst bereits `cxx-qt = "0.10.0"` voraus; `0.8.1` ist damit nicht kompatibel.
- Das Referenzsystem TUXEDO OS 24.04.5 verwendet Qt 6.10.2, das von `0.10.0` aktuell unterstützt wird.
- Das ältere `0.8.1` aus dem KDE-Tutorial bringt keinen Vorteil gegenüber `0.10.0` und würde spätere KF6-Integration blockieren.

## Konsequenzen

- KF6-Bindings (`cxx-kde-frameworks`) sind jung; System-Integration über D-Bus-Crates (`notify-rust`, `ksni`, `secret-service`/`keyring`, `zbus`).
- Risiko CXX-Qt → Spike zuerst ([milestones/m0-spike.md](../milestones/m0-spike.md)).
- i18n über KI18n (E12, NFR-06).
