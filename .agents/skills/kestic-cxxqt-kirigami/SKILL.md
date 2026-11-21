---
name: kestic-cxxqt-kirigami
description: Guidelines and M0-verified patterns for Kestic GUI work using Rust, CXX-Qt 0.10, cxx-kde-frameworks (KI18n), Kirigami 6.24, QML offscreen smoke tests, and keyboard accessibility (NFR-09). Load for all GUI stories in M1–M6.
---

# Kestic CXX-Qt & Kirigami Skill

Use this skill whenever implementing or modifying anything in `gui/`, QML files, `QAbstractListModel` bridges, offscreen smoke tests, or keyboard/accessibility checks.

## Verified Stack & Build Rules (ADR-0001)

- **Pinned versions:** `cxx-qt`, `cxx-qt-lib`, `cxx-qt-lib-extras`, `cxx-qt-build` are pinned to `=0.10.0`. `cxx-kde-frameworks` is pinned to a git revision in `Cargo.toml`. Never bump these without Stop & Ask (`docs/spec/questions.md`).
- **Linker:** `lld` is required (`scripts/bootstrap.sh`) to avoid `ld.gold` bugs/warnings with Qt6/CXX-Qt.
- **Build registration:** Every new `.qml` file and every Rust module containing `#[cxx_qt::bridge]` must be registered in `gui/build.rs` inside `CxxQtBuilder::new_qml_module(...)` (module URI `net.daester.kestic`).
- **Thin GUI layer:** Keep domain logic, restic calls, config persistence, and scheduling out of `gui/` (`core/` + `daemon/` own logic; `gui/` talks to `daemon` over D-Bus `…Kestic1`).

## CXX-Qt 0.10 Patterns

- Follow `gui/src/models/backups_model.rs` for exposing `QAbstractListModel` to QML:
  - `#[qobject] #[base = QAbstractListModel] #[qml_element]` inside `extern "RustQt"`.
  - Override `data`, `roleNames`, and `rowCount` with `#[qinvokable] #[cxx_override]` and explicit `#[cxx_name = "..."]` where camelCase is expected by Qt.
- Initialize `QApplication` (`cxx-qt-lib-extras`) with desktop style (`org.kde.desktop`) and `KLocalizedString::set_application_domain("kestic")` + `setup_localized_context` before loading QML (see `gui/src/main.rs`).

## Kirigami 6.24 / Qt 6.10 Gotchas (from M0)

- **Pushing/replacing pages:** Instantiate `Component` pages via `comp.createObject(pageStack, props || {})` before `pageStack.push(...)` / `pageStack.replace(...)` (see `gui/src/qml/Main.qml`).
- **`NavigationTabBar` width:** In wide layout, `maximumContentWidth` must be wide enough per tab so longer German labels like `"Wiederherstellen"` (added in M4) do not wrap onto two lines.
- **Window title:** Main window title always stays `"Kestic"` regardless of active subpage (`AC-M1-13-6`); settings window title is `"Kestic einrichten"`.
- **No silent QML warnings:** Treat any QML/Qt warning on `stderr` as a test failure (same assertion as `core/tests/m1_gui_smoke.rs`).

## Accessibility & Keyboard Navigation (NFR-09, M6-S09)

- Every interactive control must be reachable in visual order via `Tab` / `Shift+Tab` and triggerable via `Enter` / `Space` (`Esc` closes dialogs/modals).
- Every interactive element must set a non-empty `Accessible.name` (localized via `i18n(...)`).
- Never encode status solely via color — always pair color with text or icon.
- Follow `docs/spec/ui/` strictly (no invented terms; verify against `docs/spec/glossary.md`).

## Offscreen Smoke Tests & Visual Inspection

- Run GUI smoke tests with `QT_QPA_PLATFORM=offscreen`, isolated XDG tempdirs, `LC_ALL=` cleared, `LANG=en_US.UTF-8`, and `LANGUAGE=en` / `LANGUAGE=de`.
- Check both wide and narrow (`26 gridUnit` minimum width) layouts for truncation or overlap.
- Always inspect generated screenshots in `target/smoke-artifacts/` directly with your file-viewing tool to verify layout, readability, and alignment against `docs/spec/ui/`.
- The smoke scenes (`gui/src/smoke*.cpp`, `gui/src/smoke.rs`), their synthetic input (`qt_handleKeyEvent`/`qt_handleMouseEvent`) and `--simulate-status` exist only with the Cargo feature `test-harness`: build test binaries with `cargo build --bin kestic --features kestic-gui/test-harness` (the justfile does).
- The `.deb` is built without `test-harness`; the test `shipped_binary_runs_and_carries_no_test_harness` (`core/tests/m0_s05_deb_packaging.rs`) fails if the shipped binary contains any of it.
- Product Qt helpers (clipboard, KI18n substitution, keyboard handling) live in `gui/src/qt_bridge.{rs,cpp,h}`, never in the smoke files.
- Smoke scenes never contain secrets: the Rust test generates the repository password (`strong_password()`) and passes it to the scene via `KESTIC_SMOKE_REPO_PASSWORD`.

## External Reference Lookup

- Do **not** call workspace-restricted file-read tools (`view_file`) on `/usr/lib/...` or `~/.cargo/...` (blocked by pre-tool hook).
- If you need to inspect installed Kirigami QML components or crate headers, inspect them via `run_command` (e.g., `rg`, `sed -n`, `qtpaths6`) instead of `view_file`.
