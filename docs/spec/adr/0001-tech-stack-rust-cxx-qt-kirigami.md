# ADR 0001: Tech-Stack Rust + CXX-Qt + Kirigami

- Status: **akzeptiert (Spike M0 abgeschlossen, Versionen gepinnt)**
- Datum: 2026-11-16

## Kontext

Kestic ist eine native KDE-Plasma-Desktop-App und wird agentisch umgesetzt. Kandidaten: C++/Kirigami, Rust/CXX-Qt/Kirigami, Python/PySide6.

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

### Gepinnte Versionen (M0 Spike)

| Komponente / Werkzeug | Festgelegte Version | Quelle / Pin |
| --- | --- | --- |
| Rust-Toolchain | `1.100.0` (Edition 2024, MSRV 1.85) | `rust-toolchain.toml` |
| CXX-Qt (`cxx-qt`, `cxx-qt-lib`, `cxx-qt-lib-extras`, `cxx-qt-build`) | `=0.10.0` | `Cargo.toml` |
| `cxx` | `1.0` | `Cargo.toml` |
| `cxx-kde-frameworks` (`ki18n`, `kcoreaddons`, `kcrash`, `kiconthemes`, `kconfigwidgets`) | Git-Commit `5c51af42ad078feb89e4b7e590da9e3eab9e4d6d` (`MPL-2.0`) | `Cargo.toml`, `deny.toml` |
| `zbus` | `6.3.0` | `Cargo.toml` |
| `ksni` | `0.3.7` (`blocking`, `async-io`, `Unlicense`) | `Cargo.toml`, `deny.toml` |
| `notify-rust` | `4.19.0` | `Cargo.toml` |
| `cargo-deny` / `cargo-deb` | `0.20.2` / `3.8.0` | `scripts/bootstrap.sh` |
| Linker (Qt6/CXX-Qt) | `lld` (`qt-build-utils` wählt `lld` vor `ld.gold`) | `scripts/bootstrap.sh` |
| Test-Binaries (PGP + SHA256 geprüft) | `restic` `0.18.0` & `0.19.1`, `rest-server` `0.14.0`, `rclone` `1.76.0` | `scripts/bootstrap.sh` |

### CXX-Qt-Version (0.8.1 vs. 0.10.0, M0-S03, AC-M0-03-1)

Gepinnt auf **`cxx-qt = "=0.10.0"`** (inkl. `cxx-qt-lib = "=0.10.0"`, `cxx-qt-lib-extras = "=0.10.0"`, `cxx-qt-build = "=0.10.0"`):

- `cxx-kde-frameworks` (`invent.kde.org/libraries/cxx-kde-frameworks`, benötigt u. a. für KI18n) setzt selbst bereits `cxx-qt = "0.10.0"` voraus; `0.8.1` ist damit nicht kompatibel.
- Das Referenzsystem TUXEDO OS 24.04.5 verwendet Qt 6.10.2 und KDE Frameworks / Kirigami 6.24, die von `0.10.0` aktuell unterstützt werden.
- Das ältere `0.8.1` aus dem KDE-Tutorial bringt keinen Vorteil gegenüber `0.10.0` und würde spätere KF6-Integration blockieren.

### Secret Service und Test-Backend (M0-S08, AC-M0-08-1)

- **Produktiv-Client:** Direkte Implementierung der `org.freedesktop.secrets`-Schnittstelle (`SecretServiceStore` in `daemon/src/secrets.rs` hinter `core::secrets::SecretStore`) über `zbus = "6.3.0"` (`plain`-Session). Grund: `secret-service` 5.2.0 aus der ursprünglichen Recherche existiert auf crates.io nicht (`5.1.0` bzw. `keyring 4.2.0` ziehen `zbus 5` mit den `unmaintained`-Advisories `RUSTSEC-2026-0370/0371/0372` nach), und `secret-service 6.0.0-rc.1` ist ein Pre-Release. Der direkte `zbus 6`-Client kommt ohne zusätzliche Crates aus und erkennt gesperrte Collections (`LockedOrPromptRequired`).
- **Festgelegtes Test-Backend für automatisierte Integrationstests (AC-M0-08-1):** **Headless `gnome-keyring-daemon` in isolierter `dbus-run-session`** (`--foreground --replace --unlock --components=secrets --control-directory <tempdir>` mit leerem Passwort auf `stdin` und bereinigten Host-Umgebungsvariablen). Begründung: Es prüft den echten D-Bus-Client `SecretServiceStore` inklusive `OpenSession`, `CreateItem`, `SearchItems`, `GetSecrets` und `Delete` Ende-zu-Ende in `just verify` (~0,10 s, ohne interaktiven Dialog und ohne Zugriff auf KWallet). Für reine Unit-Tests in `core` stehen daneben `InMemorySecretStore` und `FileSecretStore` hinter demselben `SecretStore`-Trait bereit.

### i18n und String-Lint-Werkzeug (M0-S09, NFR-06, E12)

- **Laufzeit-Mechanismus:** **KI18n** über `cxx-kde-frameworks` (auf Commit-Hash gepinnt, `allow-git` + `MPL-2.0`-Clarification in `deny.toml`; `libkf6iconthemes-dev`, `libkf6configwidgets-dev`, `libkf6kcmutils-dev` in `scripts/bootstrap.sh`). Sowohl Rust (`cxx_kde_frameworks::ki18n::i18n` nach `KLocalizedString::set_application_domain("kestic")`) als auch QML (`setup_localized_context` vor `engine.load(...)`) beziehen Übersetzungen aus derselben `.mo`-Datei (`usr/share/locale/de/LC_MESSAGES/kestic.mo`, im Build über `gui/build.rs` via `msgfmt --check` erzeugt). Kein Fallback auf `gettext-rs` + `qsTr` nötig.
- **Festgelegtes String-Lint-Werkzeug (NFR-06):** **GNU gettext (`xgettext` + `msgcmp` + `msgattrib` + `msgfmt --check`)**, ausgeführt in `core/tests/m1_s16_i18n.rs` (Bestandteil von `just verify`): `xgettext --from-code=UTF-8 --keyword=i18n:1` extrahiert mit `--language=C` aus Rust und `--language=JavaScript` aus QML in eine temporäre `.pot`-Datei; `msgcmp po/de/kestic.po <.pot>`, `msgattrib --untranslated`, `msgattrib --only-fuzzy` und `msgfmt --check` erzwingen 100 % übersetzte, fehlerfreie Kataloge (in M1-S16 ergänzt durch Prüfung auf nicht gewrappte UI-Strings in QML/Rust).

## Konsequenzen

- **Systemintegration & D-Bus-Verbindungslebensdauer:** `notify-rust 4.19.0`, `ksni 0.3.7` und `zbus 6.3.0` funktionieren zuverlässig unter Plasma 6.6.6. **Wichtig:** Handles für zustandsbehaftete Plasma-/Portal-Sitzungen (`JobViewHandle` auf `org.kde.JobViewServerV2`, `PortalInhibitGuard` auf `org.freedesktop.portal.Inhibit`) müssen ihre `zbus::Connection` dauerhaft halten, da sowohl `kuiserver` als auch `xdg-desktop-portal` das Schliessen der Sender-Verbindung als Absturz/Beenden werten.
- **Benachrichtigungen:** `kestic.notifyrc` ohne `KNotifications` erlaubt in Plasma 6.6.6 keine wirksame Pro-Ereignis-Filterung (erscheint separat unter *Systemdienste*, während `desktop-entry` unter *Anwendungen* landet). Die in F3 / §9.6 vorgesehenen Kestic-eigenen Benachrichtigungsschalter sind daher verpflichtend.
- **Temporäre Advisory-Ausnahme (`ksni`):** `ksni 0.3.7` (`Unlicense`) zieht transitiv `zbus 5` mit drei `unmaintained`-Hinweisen (`RUSTSEC-2026-0370/0371/0372`) nach; bis `ksni` auf `zbus 6` wechselt, bleiben diese in `deny.toml` dokumentiert ignoriert (vgl. `questions.md`).
- **Kirigami 6.24 / Qt 6.10.2:** Gepushte `Component`-Seiten über `comp.createObject(pageStack, props)` instanziieren; bei `NavigationTabBar` im breiten Layout `maximumContentWidth` pro Tab berücksichtigen, damit längere deutsche Labels («Wiederherstellen») einzeilig bleiben.
