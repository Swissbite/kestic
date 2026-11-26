---
name: kestic-story-execution
description: Disciplined, parallelized workflow for implementing Kestic stories and milestones using subagents, targeted inner-loop checks, and full just verify completion gates.
---

# Kestic Story & Milestone Execution Skill

Use this skill when starting a story, a batch of stories, or a new milestone.

## 1. Before Writing Code: Parallel Spec Research

- Follow the "Spec – was wann laden" table in `AGENTS.md` and check `docs/spec/questions.md` for open Stop & Ask blockers.
- **Delegate spec extraction to a read-only `research` subagent** whenever a story touches multiple spec files (e.g., milestone ACs + `ui/*.md` + `glossary.md` + `restic-integration.md` + `dbus-api.md`). Ask the subagent for a compact checklist of:
  - exact AC IDs and test levels (`Unit`, `Integration`, `D-Bus`, `GUI-Smoke`, `manuell`),
  - mandatory glossary terms and prohibited words,
  - UI copy/layout constraints,
  - Stop & Ask triggers (new crates, spec ambiguities).
- If a Stop & Ask condition applies, log it in `docs/spec/questions.md`, mark the story blocked, and move to the next independent story. In a milestone run from GitHub issues, ask on the story's issue instead (`kestic-milestone-autonomy` §2).

## 2. Parallelize Independent Work

- Look for natural concurrency across crates and files (without letting two writers edit the same file simultaneously):
  - e.g., `core/` model/logic + unit tests in parallel with `docs/spec/` lookup or `daemon/` trait stubs;
  - independent test suites or independent milestones (`M4` and `M5` only depend on `M1`).
- **Writers share one checkout** (on different files). A sub-agent in a worktree of its own
  builds every dependency from scratch (minutes, about 5 GB); give one only for a large part in
  a separate crate. Research and review sub-agents need none. Only the main agent stages and
  commits. Parallel `cargo` runs wait for the lock on `target/`; that is not a hang.
- Keep the main context clean by offloading large searches, crate/spec surveys, and multi-file audits (like M6-S09 accessibility/destructive-action audits) to subagents.

## 3. Fast Inner Loop, Strict Outer Gate

- **Tests first:** for Rust code, write the AC tests and see each fail before writing the code that makes
  them pass.
- **During implementation:** Run targeted checks first so iteration stays fast:
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test -p kestic-core --test <test_name>`
- **Before declaring any story complete:**
  1. Name every non-manual AC ID in test function names or comments (`AC-M<n>-<story>-<nr>`).
  2. Update `po/de/kestic.po` if any `i18n(...)` string changed.
  3. Add SPDX headers (or `.reuse/dep5` entry) to every new file.
  4. Visually inspect any generated GUI screenshots in `target/smoke-artifacts/`.
  5. Confirm zero compiler/clippy/C++/QML warnings and no new `#[allow(...)]` or lint suppressions where a clean refactor or `-isystem` solves the root cause (`NFR-12`).
  6. Run full `just verify` and confirm exit code `0`. In a milestone run (`kestic-milestone-autonomy` §3) this full run happens once before the first push; after review fixes CI runs it on every push.

## 4. Background Task Discipline

- When running long commands asynchronously, wait for system completion notifications rather than polling in a loop or attempting to kill already-finished tasks.
