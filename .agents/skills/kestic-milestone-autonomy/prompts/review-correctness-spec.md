# Story review (1) – Correctness, spec and rules

You review a change to Kestic, a KDE Plasma desktop app (Rust, CXX-Qt, Kirigami/QML) that runs
restic for the user. You only read; you change no file.

**Change:** `git diff {{BASE}}...HEAD` in the current worktree.
**Stories:** {{STORIES}} – acceptance criteria in `docs/spec/milestones/`.
**Spec to read:** `AGENTS.md`, `docs/spec/glossary.md`, `docs/spec/test-strategy.md`,
`docs/spec/restic-integration.md`, `.agents/skills/kestic-milestone-autonomy/SKILL.md` §2,
{{SPEC_FILES}}.

Do not read commit messages (`git log`), the pull request or notes of the author; judge the
code against the spec.

## Your task

1. **AC matrix.** For every AC of the stories (skip those tested only «manuell»): the test that
   names it, and for every sub-statement `(a)`, `(b)`, … the assertion that checks it. Missing
   assertion, `||` fallback, `let _ =` on a measured value, or a test that would pass with the
   feature removed: `must-fix`. Put the matrix before the JSON.
2. **Correctness.** Follow each changed path end to end (GUI → D-Bus → daemon → core → restic)
   and ask what happens with: restic exit codes and malformed or unknown JSON; empty, single and
   very large inputs; special characters and non-UTF-8 in paths; cancellation, a second request,
   daemon restart; locks held across `await`, lock order, blocking calls in async code; child
   processes and temp files left behind; flags valid for restic 0.18.0 and the latest version.
3. **Spec fidelity.** Texts, defaults, order, states, commands and flags exactly as the ACs say.
   Anything that matches a level-3 trigger in SKILL.md §2 without the maintainer's answer on
   the issue is `must-fix`.
4. **Hard rules of `AGENTS.md`:** test isolation, `Clock` instead of the system clock, no
   warning suppressions, size limits, no secrets in argv/logs/config/test output, glossary terms
   only, comment rules, SPDX headers, new strings in `po/de/kestic.po`, code layout per
   `docs/spec/architecture.md` §Code-Struktur. Changed or removed assertions in existing tests
   must be justified by an AC.

Report only real problems with evidence. Answer in the verdict format of `prompts/README.md`
with `"reviewer": "correctness-spec"`.
