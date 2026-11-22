---
name: kestic-milestone-autonomy
description: Rules for running a whole Kestic milestone (M4–M6) largely on your own from a GitHub milestone with one issue per story, on an integration branch m<n> – decision levels, merging your own story pull request, protected paths, budgets, syncs, reports to the maintainer and the milestone Definition of Done (incl. README and site/). Load when working on an issue of such a milestone.
---

# Kestic Milestone Autonomy

A milestone agent implements a whole milestone with few check-ins of the maintainer: the
kickoff (issues, this skill and spec changes approved), questions collected at most once a day,
the halt before a risky write path (M4: before S05) and the closing pull request. Precedence:
`AGENTS.md` > this skill > the issue > the agent's own judgement. How a single story is done
(research, inner loop, `just verify`) is in [kestic-story-execution](../kestic-story-execution/SKILL.md)
and `AGENTS.md`; this skill adds what a whole milestone needs. It and `prompts/` are frozen after
the kickoff; changing them is level 3.

## 1. Milestone, issues and branches

- **GitHub milestone** `M<n> <name>` (M4: «M4 Restore») with one issue per story, titled
  `M<n>-S<NN>: …`, plus milestone issues titled `M<n>: …` (halt, milestone review, closing).
  Issues are in English. An issue is the assignment: it links the story's ACs in
  `docs/spec/milestones/` as a checklist and never copies the spec, which stays the source.
- **Order:** the issue number. Dependencies are GitHub «blocked by» links. The watcher
  (`.devcontainer/kestic-agent-watch.sh`) starts the open issue with the lowest number that has
  no open blocker and no label `needs-maintainer`, one at a time, each in a fresh session.
- **Integration branch** `m<n>` (a number only: `m4`, never `m4-x`), created from `main`; never
  rebased or force-pushed.
- **Story branch** `agent/issue-<issue>` from the current `origin/m<n>`; one pull request into
  `m<n>`, title as the issue.
- **Never** merge or push into `main`; never touch real repositories or user data.

## 2. Decision levels

**Level 3 – the maintainer decides.** When in doubt, level 3. Triggers:

- observable behaviour, a text, an error message, a default or a state transition differs from
  or goes beyond an AC, FR or NFR;
- writing, overwriting, deleting or changing permissions outside a fresh temp directory or the
  default «new folder» of a restore;
- restic flags `--overwrite`, `--delete`, `--target`, `--include`, `--exclude`, `--host`,
  `--path`, `--sparse`, `--verify` used in a way the spec does not fix;
- mapping paths or users; touching snapshots of other hosts or users;
- the D-Bus interface, the config, a persisted schema or a DTO;
- everything under «Stop & Ask» in `AGENTS.md` (spec, dependencies and tools, licences,
  destructive operations), secrets and permissions;
- a protected path (§5), this skill or its prompts;
- three failed attempts at the same problem (§4).

Post the question on the story's issue (options, recommendation, what continues), add the
label `needs-maintainer` and end the turn. The maintainer answers in a comment and removes the
label; the answer is binding: implementing it is level 1, only a deviation from it is level 3.
Restoring with `--target` into a fresh temp directory in a test is level 1.

**Level 2 – decide, record, cheap to undo.** Only if no level-3 trigger applies, no AC text,
visible text or default changes, and one commit reverts it without migration. The pull request
says `Rückbau-Kosten: …` and carries the label `level-2`. At most **5 per milestone**
(`gh pr list --base m<n> --label level-2 --state all`); entries for `decisions.md` are proposed
together at the halt or at closing.

**Level 1** – everything else inside the story's ACs.

## 3. Per story

1. Read `AGENTS.md`, this skill, the issue with its comments and the linked ACs.
2. Implement and verify per `kestic-story-execution` until `just verify` is green.
3. Two reviews with the frozen prompts (`prompts/README.md`): **correctness and spec**, and
   **break it** on a different model than the author. Reviewers get the diff, the ACs and the
   spec files, not the author's reasoning. Fix every `must-fix`, then run the complete
   `just verify` again.
4. Pull request into `m<n>`. The body maps each AC to its test, names level-2 decisions with
   `Rückbau-Kosten` and ends with the review verdicts (`must_fix: 0` each).
5. Wait for CI with one blocking `gh pr checks <pr> --watch`.
6. **Merge it yourself** with `gh pr merge <pr> --squash --match-head-commit <head>` when all of
   these hold, otherwise do not merge:
   - `verify` is green on the head commit;
   - both reviews of the last head have no open `must-fix`;
   - the branch contains the current `origin/m<n>` (if not: merge it in, `just verify`, step 5);
   - the pull request changes no protected path (§5). If it does, add `needs-maintainer` to
     the issue, say which paths and why, and end the turn; the maintainer reviews and merges.
7. Tick the AC checklist in the issue, close it with a comment naming the pull request (a
   pull request into `m<n>` does not close issues), and end the session.

## 4. Budgets and flakes

- **Three failed attempts at the same problem** (same failing test, same review finding, same CI
  failure) → level 3. Every new attempt starts with a written hypothesis of what is wrong.
- One story at a time; GUI, D-Bus, `Main.qml` and `po/` changes therefore never run in parallel.
- A flaky test is a defect: fix the cause. Retries, raised timeouts, ignored tests or loosened
  assertions are level 3. The maintainer may approve a re-run for a known infrastructure fault.

## 5. Protected paths

A change to one of these paths is level 3 and needs the maintainer's review before the merge:

- rules and specification: `AGENTS.md`, `CLAUDE.md`, `.agents/`, `.claude/`, `.gemini/`,
  `docs/spec/`;
- CI, build and tool configuration: `.github/`, `.devcontainer/`, `justfile`, `clippy.toml`,
  `deny.toml`, `rustfmt.toml`, `rust-toolchain.toml`, every `Cargo.toml`, `Cargo.lock`, every
  `build.rs` (it runs arbitrary code), `.cargo/`, `.config/`, `.reuse/`, `LICENSES/`,
  `.gitattributes`, `scripts/`, `packaging/`, `site/package.json`, `site/pnpm-lock.yaml`;
- existing test fixtures under `core/tests/fixtures/`;
- the M4 restore write path: `core/src/restore*` and `daemon/src/service/restore*`.

A story may change without review: new fixtures under `core/tests/fixtures/m<n>/` and, in
`.config/nextest.toml`, only additions of new test binaries to the `heavy-integration` or
`gui-smoke` filter. `docs/spec/backlog.md` (status line) and `docs/spec/manual-acceptance.md`
change only in the closing pull request.

## 6. Sync `main` → `m<n>`

Renovate and other sessions work on `main`; their changes reach `m<n>` only by a sync, which
the coordinating session creates outside the container (the agent's token cannot push workflow
changes): a fast-forward while `m<n>` has no commits of its own, afterwards a merge commit on
`claude/m<n>-sync-<yyyymmdd>`, never a rebase. Syncs are bundled: after the Monday Renovate
merges, when a story needs a change from `main` and before the closing. An agent that needs a
change from `main` says so on its issue with `needs-maintainer`.

## 7. Records

The milestone, its issues (state, labels, comments) and the pull requests are the record; there
is no ledger file. Dates are absolute; no secrets and no paths of real data in issues, comments
or pull requests.

## 8. Reports to the maintainer

| Moment | How |
| --- | --- |
| Kickoff | the maintainer approves the issues, this skill and the spec changes; then the watcher starts |
| Questions | issues labelled `needs-maintainer`; the maintainer looks at most once a day |
| Halt | the issue «M<n>: halt …» runs before the write path: auditor report (`prompts/auditor.md`), level-2 list and the design of the write path as a comment on that issue. The first write-path story starts labelled `needs-maintainer`; removing the label is the approval. |
| Closing | the issue «M<n>: closing …»: pull request `m<n>` → `main` with auditor report, the write-path diff for line-by-line review, level-2 and level-3 log, test deltas and manual acceptance cards (`kestic-manual-acceptance`). The maintainer merges it as a merge commit. |

## 9. Definition of Done of a milestone

- All stories done: non-manual ACs as tests green in `just verify`; manual ACs open in
  `docs/spec/manual-acceptance.md` with step-by-step instructions.
- Exit criteria of the milestone file met.
- Milestone review: `prompts/review-milestone-quality.md` plus the two story prompts over the
  whole milestone diff; findings fixed in one clean-up pull request.
- README (English) matches the milestone (`AGENTS.md` step 8).
- Showcase site `site/`: milestone done in the roadmap, its features in «What it does» (glossary
  terms), its smoke screenshots in the manifest; `just site` green.
- Closing pull request `m<n>` → `main`, which the maintainer reviews and merges.

## 10. Operating in the agent container

Only this section is specific to the tooling (`.devcontainer/README.md`).

- The watcher starts and continues sessions; the agent never polls. When it waits for the
  maintainer, it ends its turn; a new comment continues the session.
- Every comment the agent writes ends with the line `<!-- kestic-agent -->`; the watcher
  ignores those.
- Reviews run as two read-only sub-agents, one with a model override (e.g. `sonnet`).
- Commits, pull requests and comments carry no AI co-author trailers and no «Generated with»
  lines.
