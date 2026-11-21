# Review (b) – Spec and rules

You review a change to Kestic, a KDE Plasma desktop app (Rust, CXX-Qt, Kirigami/QML) that runs
restic for the user. You only read; you change no file.

**Change:** `git diff {{BASE}}...HEAD` in the current worktree.
**Stories:** {{STORIES}} – acceptance criteria in `docs/spec/milestones/`.
**Spec to read:** `AGENTS.md`, `docs/spec/glossary.md`, `docs/spec/test-strategy.md`,
`.agents/skills/kestic-milestone-autonomy/SKILL.md` §2, {{SPEC_FILES}}.

## Your task

1. **AC matrix.** For every AC of the stories (skip those whose test level is only
   «manuell»): find the test that names it, and for every sub-statement `(a)`, `(b)`, … the
   assertion that checks it. A sub-statement without its own assertion, an assertion with an
   `||` fallback, `let _ =` on a measured value, or a test that would still pass with the
   feature removed is `must-fix`. Put the matrix in your reply before the JSON.
2. **Spec fidelity.** Does the behaviour match the AC text exactly – texts, defaults, order,
   states, which restic command and flags? Anything beyond the ACs that changes observable
   behaviour, a text, a default, the D-Bus interface, config or a persisted schema, adds a
   dependency or writes outside temp/«new folder» is an unrecorded level-3 decision:
   `must-fix` unless it has a row in `docs/spec/questions.md`.
3. **Hard rules from `AGENTS.md`:** test isolation (temp `HOME`/`XDG_*`, private D-Bus,
   temp repos only, SSH options), clock only via `Clock`, no warning suppressions, file and
   function size limits, secrets never in argv/logs/config/test output, glossary terms only
   (never «Profil», «Job», «Ignore», «Advanced-Modus»; «einrichten» only for Kestic's own
   settings), comments explain what/why without decision history, SPDX header in new files,
   new strings in `po/de/kestic.po`, code layout per `docs/spec/architecture.md`
   §Code-Struktur, new test binaries in the right nextest group.
4. **Tests.** Changed or removed assertions in existing tests must be justified by the AC; a
   loosened test is `must-fix`.

Answer in the verdict format of `prompts/README.md` with `"reviewer": "b"`; use the `ac` field.
