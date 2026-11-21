---
name: kestic-milestone-autonomy
description: Rules for running a whole Kestic milestone (M4–M6) largely on your own on an integration branch m<n> – decision levels, per-story flow, budgets, ledger, reports to the maintainer, milestone Definition of Done (incl. README and site/), infra-flake and sync rules. Load when starting, continuing or closing a milestone run.
---

# Kestic Milestone Autonomy

A milestone agent implements a whole milestone with few check-ins: kickoff, one halt before
the restore write path (M4) or an equivalent risky step, collected questions at most once a
day, and the closing review. This skill is the agent's rulebook. Precedence when things
conflict: `AGENTS.md` > this skill > the agent's own judgement.

This skill and the prompts in `prompts/` are **frozen** once the maintainer approved them at
kickoff. Changing them is a level-3 decision (they are protected paths, see §9).

Related skills: `kestic-story-execution` (inner loop per story), plus the domain skills listed
in `AGENTS.md`.

## 1. Branches and guards

| Item | Rule |
| --- | --- |
| Integration branch | `m<n>`, created from `main` at milestone start. Never rebased, never force-pushed. |
| Story branch | `claude/m<n>-s<NN>-<slug>` from the current `origin/m<n>`; one pull request into `m<n>`. |
| Sync branch | `claude/m<n>-sync-<yyyymmdd>` from `origin/m<n>`, merges `origin/main` (§7). |
| Closing branch | `m<n>` itself, or `claude/m<n>-closing` (= `m<n>` + status line in `backlog.md` and `manual-acceptance.md`) → pull request into `main`. The maintainer merges it. |
| Merge into `m<n>` | Only `scripts/merge-into-milestone.sh <pr>`: base must be `m<n>`, `verify` and `tripwire` green on the head commit, head contains the current base; squash merge, title becomes the commit subject. |
| Never | Merge or push into `main`; touch the main checkout, the installed Kestic, `~/.config/kestic` or real repositories. |

Guards that do not depend on this text:

- **Ruleset** for `m[0-9]*` (maintainer): pull request required, required checks `verify` and
  `tripwire`, branch must be up to date, no force-push, no deletion.
- **Tripwire** (`.github/workflows/tripwire.yml`, `.github/scripts/tripwire.mjs`): runs on
  `pull_request_target`, so it always uses the version on the base branch and never runs code
  of the pull request. It fails on: protected paths (§9); new lint/test suppressions
  (`#[allow]`, `#[expect]`, `#[ignore]`, `#pragma`, `-Wno-`, `qmllint disable`, …); removed or
  changed assertion statements in test code (formatting and `use` paths do not count); fewer
  test functions in the changed files; raised timeouts or retries in tests; nextest config
  changes other than adding `binary(…)`; an AC that had a test before and has none now; a story
  named in the title (`M4-S01: …`) whose non-manual ACs are not all named by a test. A sync
  merge from `main` skips the story rules for files taken unchanged from `main`.
  A tripwire finding is never "fixed" by working around the check: either undo the change or
  record a level-3 entry and leave the merge to the maintainer.
- **Local permissions** (maintainer, per agent tool): allow the build and test tools, pushes to
  `m<n>` story branches, `gh pr create|view|checks|edit`, read-only `gh api`, the merge script;
  deny `gh pr merge`, pushes to `main`, edits of protected paths.

## 2. Decision levels

Classify every decision that the ACs do not settle. **When in doubt: level 3.**

**Level 3 – the maintainer decides.** Any of:

- changes observable behaviour, a text, an error message, a default or a state transition of an
  AC, FR or NFR, or goes beyond what an AC says;
- writes, overwrites, deletes or changes permissions outside a fresh temp directory or outside
  the default «new folder» of a restore;
- uses restic flags `--overwrite`, `--delete`, `--target`, `--include`, `--exclude`, `--host`,
  `--path`, `--sparse`, `--verify` in a way not fixed by the spec;
- maps paths or users; touches snapshots of other hosts or users;
- changes the D-Bus interface, the config, a persisted schema or a DTO;
- adds a dependency (crate, system package, tool), touches licences, secrets or permissions;
- everything listed under «Stop & Ask» in `AGENTS.md`;
- changing this skill, its prompts, the tripwire, the merge script or a protected path;
- a budget is exhausted (§4) or a sync conflict is more than trivial (§7).

How: entry in the ledger (`level3[]`) and a row in `docs/spec/questions.md` (date, story,
question, options, recommendation) in the story's pull request or a small question-only pull
request; mark the story `blocked`; continue with the next independent story. Questions go to the
maintainer collected, at most once a day (§8). A new spec text the maintainer approves (e.g.
DTOs in `dbus-api.md`) comes as its own small pull request that the maintainer merges.

**Level 2 – decide, record, make it cheap to undo.** Only if all hold:

- no AC text, visible text or default changes, nothing from level 3 applies;
- one commit reverts it without migration;
- the pull request body has a line `Rückbau-Kosten: …` (what reverting costs);
- at most **5 per milestone**; the maintainer samples them at the halt and at closing.

How: entry in the ledger (`level2[]`: id, story, decision, alternatives, revert cost, PR).
Entries meant for `decisions.md` are collected there and proposed as one spec pull request at
the halt or at closing (spec files are protected).

**Level 1 – just do it.** Everything else inside the story's ACs: internal structure, names of
private items, test layout, refactorings without behaviour change.

## 3. Flow per story

1. **Pick** the next story in the agreed order; skip stories that are `blocked` or wait for a
   dependency. Read `docs/spec/questions.md` and the ledger first.
2. **Research** (read-only, e.g. a sub-agent): compact checklist of the story's AC IDs with
   sub-statements and test level, glossary terms and forbidden words, UI texts, restic flags,
   level-3 triggers. Record the AC list in the ledger.
3. **Branch** from `origin/m<n>` in its own worktree with its own `target/` (no shared
   `CARGO_TARGET_DIR` while two worktrees run). At most **two** worktrees at a time; GUI, D-Bus,
   `Main.qml` and `po/` work strictly one after another.
4. **Implement** in small steps per `AGENTS.md` (§Code-Struktur, thin GUI, logic in `core`),
   with targeted tests (`cargo nextest run -p … --test …`). Every non-manual AC gets a test
   whose name or comment carries the AC ID; every sub-statement `(a)`, `(b)`, … its own
   assertion. New integration test binaries go into the `heavy-integration` group of
   `.config/nextest.toml`, new GUI smoke binaries into `gui-smoke` (only `binary(…)` additions).
   New UI strings go into `po/de/kestic.po` in the same story.
5. **Verify**: `just verify` green on a clean worktree. It writes `target/verify-green` with
   `VERIFY GREEN <commit>`; the commit must be `HEAD` and the line must not say `dirty`.
   Inspect the screenshots in `target/smoke-artifacts/` for GUI stories.
6. **Review** with the four frozen prompts in `prompts/` – (a) correctness, (b) spec and rules,
   (c) quality, (d) break it. Each reviewer gets the diff (`git diff origin/m<n>...HEAD`), the
   story's ACs and the spec files the story names – **not** the author's reasoning or summary.
   At least one reviewer runs on a different model than the author. Each answers with the JSON
   verdict defined in `prompts/README.md`. Fix every `must-fix`, then re-run `just verify`.
   Zero findings from all four on a change of more than 400 changed lines → run (d) again with
   the strict addendum. Record the verdicts (counts) in the ledger.
7. **Pull request** into `m<n>`, title `M<n>-S<NN>: <summary>` (several stories: all IDs), body
   from §3.1. Push only after step 6; every push that triggers `verify` counts as a CI run.
8. **CI**: wait for the checks by a blocking watch or a completion notification
   (`gh pr checks <pr> --watch`), never by a polling loop. Red: new written hypothesis in the
   ledger, fix, local check, push (budget §4).
9. **Merge** with `scripts/merge-into-milestone.sh <pr>`. Merges into `m<n>` strictly one after
   another; the script refuses a branch that is behind `m<n>`.
10. **Ledger**: the story's entry is updated in its own pull request before the last push
    (status `merging`, PR number, CI runs); the merge commit and status `done` are recorded in
    the next pull request into `m<n>`.
11. **Clean up**: remove the worktree and its `target/`, delete the local branch.

### 3.1 Pull request body

```markdown
## Story
M4-S01 – <story title>

## ACs → Tests
| AC | Test (file::function) | Sub-statements asserted |
| --- | --- | --- |

## Changes
<what changed, per crate; no reasoning about alternatives>

## Decisions
- Level 2: <none | id: decision> — Rückbau-Kosten: <…>
- Level 3 open: <none | ledger id + questions.md row>

## Test delta
<tests added / changed / removed; assertion changes must be explained here>

## Verification
- `VERIFY GREEN <sha>`
- Reviews: a <n must-fix, fixed>, b …, c …, d …
- Screenshots checked: <list | n/a>
```

## 4. Budgets

| Budget | Limit | When reached |
| --- | --- | --- |
| Fix rounds per error class (same failing test or same review finding) | 3 | level-3 entry, story `blocked`, next story |
| CI runs per story (every push that triggers `verify`, re-runs included) | 4 | same |
| Review rounds per story (plus one strict re-run of (d)) | 2 | remaining must-fix → level 3 |
| Turns per story goal | 25 | stop, ledger entry with state and next step |
| Level-2 decisions per milestone | 5 | further ones are level 3 |
| Parallel worktrees | 2 | wait |

Every retry needs a **new written hypothesis** in the ledger (`attempts[]`): what failed, why
the agent thinks so, what changes. Repeating a step without a new hypothesis is not allowed.

Not counted as CI runs: the `infra-flake` re-run (§5) and the first run after a sync that
rebuilds the CI image (§7).

## 5. Flakes and `infra-flake`

- A flaky test is a defect: find and fix the cause. Never add retries, raise timeouts, mark
  tests ignored or slow, or loosen assertions to get green – each of these is level 3 and the
  tripwire reports it.
- **`infra-flake`**: the test keyring crashes (SIGABRT or SIGTRAP with `OpenSession` or
  `plain_negotiate` in the log) while the keyring fix is not yet on `m<n>`. Then re-run the
  failed jobs **once** (`gh run rerun <run> --failed`), ledger entry `ci_runs[]` with kind
  `rerun-infra` and the log line; test and timeouts stay as they are. A second crash in the
  same story is a normal CI run. The exception ends once the fix is synced into `m<n>`.

## 6. Ledger

`.agents/milestones/m<n>-progress.json` is the single source of progress; the maintainer's
coordinating session reads it to detect a stall (no progress in ledger or pull requests for
3 hours). Shape:

```json
{
  "milestone": "M4",
  "branch": "m4",
  "started": "2026-11-22",
  "rulebook": ".agents/skills/kestic-milestone-autonomy/SKILL.md",
  "order": ["M4-S00", "M4-S01", "…"],
  "stories": [
    {
      "id": "M4-S01",
      "status": "todo | in-progress | review | merging | done | blocked",
      "branch": "claude/m4-s01-snapshot-list",
      "prs": [41],
      "merge_commit": null,
      "acs": { "AC-M4-01-1": { "tests": ["core/tests/m4_s01_snapshots.rs::ac_m4_01_1_…"], "status": "green | open | manual" } },
      "ci_runs": [{ "pr": 41, "run": 123, "result": "success | failure | rerun-infra", "hypothesis": "…" }],
      "attempts": [{ "class": "…", "hypothesis": "…", "result": "…" }],
      "reviews": { "a": { "must_fix": 0, "findings": 2 }, "b": {}, "c": {}, "d": {} },
      "notes": "…"
    }
  ],
  "level2": [{ "id": "L2-1", "story": "…", "decision": "…", "alternatives": "…", "revert_cost": "…", "pr": 0 }],
  "level3": [{ "id": "L3-1", "story": "…", "question": "…", "options": ["…"], "recommendation": "…", "status": "open | answered", "answer": null }],
  "syncs": [{ "date": "…", "main": "<sha>", "pr": 0, "reason": "…" }],
  "reports": [{ "date": "…", "kind": "kickoff | questions | halt | closing", "note": "<path>" }]
}
```

Dates are absolute (`YYYY-MM-DD`). The ledger never contains secrets or paths of the
maintainer's real data.

## 7. Sync `main` → `m<n>`

- Only by **merge commit** on a sync branch (§1) with a pull request into `m<n>`; never rebase
  `m<n>`. Only when no story pull request is open.
- When: after the keyring fix lands on `main`; after a restic update on `main` (M4: PR 29,
  restic 0.20.0 – S05 starts only after it is synced); after each Monday batch of Renovate
  merges; after S04; before the closing pull request.
- The tripwire treats files that equal `main`'s version and were not changed on `m<n>` since the
  merge base as synced and skips the story rules for them. Everything else is judged like a
  story change. A conflict beyond trivial (e.g. lock file regenerated, both sides touched
  unrelated lines) is level 3.
- Renovate pull requests target `main`, never `m<n>`. Work of other sessions on `main` (restic
  updates, `prune --json`, the keyring fix) reaches `m<n>` only through this rule.

## 8. Reports to the maintainer

Reports go to the coordinating session (address given at milestone start), which relays them.
Large content goes into the shared notes folder; the message carries the summary and the path.

| Moment | Content |
| --- | --- |
| **Kickoff** | link to the step-0 pull request; story order and pull-request cut with estimated size; budgets; level-3 pre-decisions with options and recommendation; tool permissions; required check names for the ruleset; draft spec changes (e.g. D-Bus methods and DTOs). Then wait for approval. |
| **Collected questions** | at most once a day: open level-3 entries with options and recommendation; which stories are blocked; what continues meanwhile. If everything is blocked: stop and send this report. |
| **Halt** (before a write path, M4: before S05) | auditor report (`prompts/auditor.md`); level-2 list with a sample; design of the write path (flows, flags, failure modes, tests). Wait for approval. |
| **Closing** | auditor report; write-path diff (M4: S05–S07) for line-by-line review; level-2/3 log; test deltas; tripwire in closing mode; manual acceptance cards (`kestic-manual-acceptance`). Not the whole diff. |

## 9. Paths in story pull requests

Protected (tripwire): `AGENTS.md`, `CLAUDE.md`, `.agents/skills/**` (this skill and its
prompts included), `.claude/**`, `.gemini/**`, `docs/spec/**`, `.github/**`, `justfile`,
`clippy.toml`, `deny.toml`, `rustfmt.toml`, `rust-toolchain.toml`, every `Cargo.toml`,
`Cargo.lock`, `.cargo/**`, `.config/**`, `.reuse/**`, `REUSE.toml`, `LICENSES/**`,
`scripts/merge-into-milestone.sh`, `site/package.json`, `site/pnpm-lock.yaml`.

Allowed despite protection:

- `docs/spec/questions.md` – level-3 questions, any pull request.
- `.config/nextest.toml` – only adding `binary(…)` of new test binaries to existing filters.
- `docs/spec/backlog.md` (status line) and `docs/spec/manual-acceptance.md` – only on the
  closing branch, which the maintainer merges.

Not protected, but rules apply: `.agents/milestones/**` (ledger, only facts), `po/**` (one
catalogue entry per new string), `site/src/**` (only the closing site update), `README.md`
(closing).

## 10. Definition of Done of a milestone

- Every story `done`: all non-manual ACs as tests green in `just verify`; manual ACs stay open
  in `docs/spec/manual-acceptance.md` with step-by-step instructions (`kestic-manual-acceptance`).
- Exit criteria of the milestone file met.
- **Milestone review**: the four review prompts over the whole milestone diff
  (`git diff <start>...m<n>`), findings fixed in one clean-up pull request.
- **README** (English) matches the milestone (`AGENTS.md` step 8); `docs/spec/` stays German.
- **Showcase site** (`site/`): the milestone is marked done in the roadmap, its features are in
  «What it does» (glossary terms only), its screenshots from the smoke tests are in the
  screenshot manifest; `just site` (incl. `astro check`) is green. This is the last story pull
  request into `m<n>`.
- Auditor report at closing; tripwire in closing mode without findings (run locally:
  `GITHUB_REPOSITORY=… PR_NUMBER=<closing pr> GITHUB_TOKEN=… node .github/scripts/tripwire.mjs`).
- Closing pull request `m<n>` → `main` opened; the maintainer reviews and merges it.

This applies to M4, M5 and M6.

## 11. Texts

Glossary terms only; never «Profil», «Job», «Ignore», «Advanced-Modus». «einrichten» only for
Kestic's own settings; backups and destinations are «angelegt». New strings: English `msgid`,
German in `po/de/kestic.po` in the same story.

## 12. Operating with Claude Code

Only this section is specific to one agent tool; the rules above hold for any agent.

- **Story goal**: one `/goal` per story with a measurable condition, e.g. «`target/verify-green`
  names `HEAD` without `dirty`, and PR #n is merged into `m4` by the merge script – or stop
  after 25 turns with a ledger entry». `/clear` between stories; the ledger and this skill
  restore the context.
- **Worktrees**: a worktree created by the tool's own worktree feature starts from `main`
  (`worktree.baseRef` «fresh»). Create story worktrees explicitly:
  `git worktree add <dir> -b claude/m4-s<NN>-<slug> origin/m4`.
- **Waiting for CI**: run `gh pr checks <pr> --watch` as a background command or through
  `Monitor`, and continue when notified. No `sleep` loops, no scheduled wake-ups for polling.
- **Stop hook** (maintainer's settings): blocks ending a story turn while `target/verify-green`
  does not name `HEAD` or says `dirty`.
- **Reviews**: four sub-agents (or one workflow) with the prompts from `prompts/`, the diff and
  the spec paths; at least one with a model override (e.g. `sonnet`) different from the
  author's model. Sub-agents only read.
- **Permissions** (`.claude/settings.local.json`, set by the maintainer): allow `just *`,
  `cargo *`, `git push origin claude/*`, `gh pr create|view|checks|edit`, `gh run view|rerun`,
  read-only `gh api`, `scripts/merge-into-milestone.sh *`; deny `gh pr merge`,
  `git push origin main`, edits of protected paths except the allowed ones in §9. Direct
  pushes to `m4` are blocked by the ruleset once it is active.
- **Messages** to the coordinating session with `SendMessage`, at most the moments in §8.
- **Commits and pull requests** carry no AI co-author trailers and no «Generated with» lines.
