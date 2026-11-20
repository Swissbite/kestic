---
name: kestic-i18n-packaging
description: KI18n workflow, po/de/kestic.po catalog rules, NFR-06 GNU gettext lint checks, and cargo-deb packaging invariants (NFR-01, P34). Load when adding/changing user-visible strings or packaging files.
---

# Kestic i18n & `.deb` Packaging Skill

Use this skill whenever adding or changing user-visible text in Rust or QML, editing `po/de/kestic.po`, modifying `packaging/`, or touching `Cargo.toml` package metadata for `cargo-deb`.

## i18n Rules (KI18n + GNU gettext, `NFR-06`)

- **Language convention:**
  - Spec documents (`docs/spec/`): German (Swiss orthography without `ß`, using `ss`).
  - Code, identifiers, and `i18n(...)` source `msgid`s: **English**.
  - German translations: `po/de/kestic.po`.
- **Every user-visible string** in Rust (`cxx_kde_frameworks::ki18n::i18n(...)`) and QML (`i18n(...)` / `i18nc(...)` / `i18np(...)`) must be wrapped so `xgettext` extracts it cleanly (`AC-M1-16-1`).
- **One owner per text** (`docs/spec/dbus-api.md` §Texte und Übersetzungen): texts that daemon **and** GUI need (code texts, shared status lines, tray + window labels) live in the crate `kestic-i18n` (`i18n/src/messages.rs`, marked with `i18n_noop("…")`, translated by each side via `tr(&Message)`; QML gets shared labels from the `Texts` singleton). Texts only one side needs stay there. **No msgid in more than one production file**, UI labels included: a GUI text used in several places goes once into `gui/src/ui_texts.rs` (`GuiText`; Rust `GuiText::Close.text()`, QML `Texts.label("close")` / `Texts.labelArgs("code", [a, b])`), and repeated form parts become one QML component (e.g. `DestinationDetailsForm.qml`). Same meaning, same wording. `i18n/tests/text_ownership.rs` fails on a text written in two files, an unknown `Texts` code or an unused `GuiText`.
- Keep `i18n("literal")` calls with string literals (avoid passing opaque variables directly to `i18n(...)`, as `xgettext` cannot extract dynamic strings — see `gui/src/ui_texts.rs`).
- Whenever you add, change, or remove `i18n(...)` strings:
  1. Update `po/de/kestic.po` immediately (do not wait until the end of the milestone).
  2. Ensure 0 untranslated entries, 0 fuzzy entries, and valid format strings (`msgfmt --check`).
  3. Verify `core/tests/m0_s09_i18n.rs` passes (it runs `xgettext --language=C` on Rust + `xgettext --language=JavaScript` on QML + `msgcmp` + `msgattrib` + `msgfmt --check`).
- **Locale env var gotcha (from M0):** `glibc`/`gettext` ignores `LANGUAGE` when `LANG=C.UTF-8` is set. In all tests checking `LANGUAGE=de` vs `LANGUAGE=en`, set `LANG=en_US.UTF-8` and clear `LC_ALL` / `LC_MESSAGES`.

## `.deb` Packaging (`cargo-deb`, `NFR-01`, `P34`)

- Built via `cargo deb -p kestic-gui --fast` (part of `just verify`).
- Verify package contents and metadata (`dpkg-deb -I` / `dpkg-deb -c`) whenever `gui/Cargo.toml` or `packaging/` changes:
  - `Depends`: `qml6-module-org-kde-kirigami (>= 6.24)`, `qml6-module-org-kde-kirigamiaddons-datetime (>= 1.11)`, `qml6-module-org-kde-kirigamiaddons-tableview (>= 1.11)`.
  - `Recommends`: `restic (>= 0.18.0)`.
  - `/etc/xdg/autostart/net.daester.Kestic.daemon.desktop` installed as a Debian conffile containing `Exec=kestic daemon --autostart` and `TryExec=/usr/bin/kestic` (so `apt remove` leaves it inert and `apt purge` removes it).
  - `/usr/share/knotifications6/kestic.notifyrc`, GUI `.desktop` file, compiled `kestic.mo`, and icons (`kestic-symbolic.svg` + application icon).
- **Feature safety (M5-S06):** Ensure the Cargo test feature `test-trust-anchor` is **never** enabled in `cargo-deb` builds.
- **License / REUSE:** Every new file in the repo must pass `reuse lint` (either valid SPDX header on top or covered in `.reuse/dep5`).
