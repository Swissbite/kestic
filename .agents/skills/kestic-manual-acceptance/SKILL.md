---
name: kestic-manual-acceptance
description: Structured step-by-step protocol for guiding the human maintainer through docs/spec/manual-acceptance.md on TUXEDO OS / Plasma at the end of a milestone.
---

# Kestic Manual Acceptance Skill

Use this skill whenever the user asks to perform manual acceptance tests or when completing a milestone in `docs/spec/manual-acceptance.md`.

## Core Rules

1. **Never self-check manual ACs:** Leave `- [ ]` open in `docs/spec/manual-acceptance.md` until the user explicitly confirms the observed behavior.
2. **Pre-flight before asking the user:** Run `just verify` and check which binary or `.deb` artifact is needed (`cargo build --bin kestic` or `just deb`), verifying exact file paths first.
3. **Never give vague summaries:** The terse bullet lines in `manual-acceptance.md` are not enough for a human tester (lesson from M0).

## Step-by-Step Test Card Format

Walk through open items sequentially or in small logical groups (e.g., all `.deb`/install checks together, all live Plasma/hardware checks together). For **each** AC, provide:

1. **AC-ID & Kurz-Ziel** (what risk or behavior is being verified).
2. **Voraussetzung / Vorbereitung** (e.g., `.deb` installed, test repo path, clean config or PowerDevil setting).
3. **Exakte Schritte** — copy-pasteable shell commands and/or exact KDE Plasma click paths (including `kcmshell6 <module>` names where relevant).
4. **Erwartetes Ergebnis** — precisely what should happen (and what should *not* happen, e.g., why daemon doesn't auto-show in tray without `config.toml`).
5. **Klare Rückfrage** — ask for the concrete confirmation or terminal output/screenshot needed to check off that AC.

## Safety & Cleanup Reminders

- When a test temporarily changes system settings (e.g., PowerDevil sleep timer, disabling a notification, stopping a local server, `apt remove`/`purge`), explicitly remind the user to revert the setting once verified.
- For destructive/hardware tests in later milestones (e.g., M3 lid-close during backup, M4/M5 real NAS target, M6 fresh-config recovery), keep instructions isolated to test backups/repos so real user backups are never risked.
