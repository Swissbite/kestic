---
name: kestic-milestone-autonomy
description: Rules for running a whole Kestic milestone (M4–M6) largely on your own from a GitHub milestone with one issue per story, on an integration branch m<n> – decision levels, merging your own story pull request, protected paths, budgets, syncs, reports to the maintainer and the milestone Definition of Done (incl. README and site/). Load when working on an issue of such a milestone.
---

# Kestic Milestone Autonomy

A milestone agent implements a whole milestone with few check-ins of the maintainer: the
kickoff (issues, this skill and spec changes approved), questions collected at most once a day,
the halt before a risky write path (M4: before S05), reviews of the write-path stories and the
closing pull request. Precedence: `AGENTS.md` > this skill > the issue > the start prompt of the
agent container > the agent's own judgement. How a single story is done (research, inner loop,
`just verify`) is in [kestic-story-execution](../kestic-story-execution/SKILL.md) and
`AGENTS.md`; this skill adds what a whole milestone needs. It and `prompts/` are frozen after
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
  rebased or force-pushed. The tag `m<n>-start` marks its state after the kickoff. No ruleset
  protects `m<n>`: the merge rule (§3) holds only through the agent itself and the oversight of
  the coordinating session. `main` keeps the hard protection (pull request, `verify`, no update
  by the agent's App).
- **Story branch** `agent/issue-<issue>` from the current `origin/m<n>`; one pull request into
  `m<n>`, title as the issue. Milestone issues do only what their text says; a step without
  code needs no branch, no `just verify` and no pull request.
- **Never** merge or push into `main`; never touch real repositories or user data.

## 2. Decision levels

**Level 3 – the maintainer decides.** When in doubt, level 3. Triggers:

- observable behaviour, a default or a state transition differs from or goes beyond an AC, FR
  or NFR; a visible text that states behaviour or a promise the spec does not give;
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
label `needs-maintainer` and end the turn; this replaces the row in `docs/spec/questions.md`.
The maintainer answers in a comment and removes the label; only the maintainer removes it and
closes the halt and closing issues. The answer is binding: implementing it is level 1, only a
deviation from it is level 3. Restoring with `--target` into a fresh temp directory in a test
is level 1.

**Level 2 – decide, record, cheap to undo.** Only if no level-3 trigger applies, no AC text or
default changes, and one commit reverts it without migration. The pull request says
`Rückbau-Kosten: …` and carries the label `level-2`. At most **5 per milestone**
(`gh pr list --base m<n> --label level-2 --state all`); entries for `decisions.md` are proposed
together at the halt or at closing.

**Level 1** – everything else inside the story's ACs, including texts the spec gives verbatim
and new visible texts that only name what the spec defines (a column header, an error detail).

## 3. Per story

1. `git fetch`, a clean working tree, then read `AGENTS.md`, this skill (from `origin/m<n>`),
   the issue with its comments and the linked ACs. If the branch `agent/issue-<issue>` or its
   pull request exists, continue there.
2. Implement and verify per `kestic-story-execution` until `just verify` is green.
3. Two reviews with the frozen prompts (`prompts/README.md`): **correctness and spec**, and
   **break it** on a different model than the author (`sonnet` when the author runs on `opus`,
   otherwise `opus`). Reviewers get the diff, the ACs and the spec files, not the author's
   reasoning. Fix every `must-fix`, then run the complete `just verify` and both reviews again.
   Merging `origin/m<n>` into the branch needs `just verify` again, not new reviews.
4. Pull request into `m<n>`. The body maps each AC to its test, names level-2 decisions with
   `Rückbau-Kosten` and ends with both reviewers' verdict JSON blocks as they returned them.
5. Wait for CI with `gh pr checks <pr> --watch` (Bash timeout 40 minutes). The result counts
   only from `gh pr view <pr> --json headRefOid,statusCheckRollup`: `verify` is `SUCCESS` for
   exactly that head. No `verify` check at all means CI does not run for `m<n>`: do not merge,
   ask (§2).
6. **Merge it yourself** with `gh pr merge <pr> --squash --match-head-commit <head>
   --delete-branch` only when all of these hold:
   - `verify` is `SUCCESS` on the head (step 5);
   - both reviews report `must_fix: 0`, and their `head` is the pull request's head or a commit
     after which only merges of `origin/m<n>` followed (step 3);
   - the branch contains the current `origin/m<n>` (if not: merge it in, `just verify`, push,
     step 5);
   - the issue is not a maintainer-reviewed story (§5), and `gh pr diff <pr> --name-only`
     lists no protected path (§5).

   Otherwise add `needs-maintainer` to the issue, say what the maintainer has to review, and
   end the turn. The maintainer merges or comments; either continues the session.
7. After the merge, by you or the maintainer: tick the AC checklist in the issue, close it with
   a comment naming the pull request (a pull request into `m<n>` does not close issues), and
   end the session.

## 4. Budgets and flakes

- **Three failed attempts at the same problem** (same failing test, same review finding, same CI
  failure) → level 3. Each failed attempt gets an issue comment `Attempt <n>/3: <problem> –
  <hypothesis> – <result>`; a new session counts them before it tries again.
- One story at a time; GUI, D-Bus, `Main.qml` and `po/` changes therefore never run in parallel.
- A flaky test is a defect: fix the cause. Retries, raised timeouts, ignored tests or loosened
  assertions are level 3. The maintainer may approve a re-run for a known infrastructure fault.

## 5. Protected paths and reviewed stories

A change to one of these paths is level 3 and needs the maintainer's review before the merge:

- rules and specification: `AGENTS.md`, `CLAUDE.md`, `.agents/`, `.claude/`, `.gemini/`,
  `docs/spec/`;
- CI, build and tool configuration: `.github/`, `.devcontainer/`, `justfile`, `clippy.toml`,
  `deny.toml`, `rustfmt.toml`, `rust-toolchain.toml`, every `Cargo.toml`, `Cargo.lock`, every
  `build.rs` (it runs arbitrary code), `.cargo/`, `.config/`, `.reuse/`, `LICENSES/`,
  `.gitattributes`, `scripts/`, `packaging/`, `site/package.json`, `site/pnpm-lock.yaml`;
- existing test fixtures under `core/tests/fixtures/`.

A story may change without review: new fixtures under `core/tests/fixtures/m<n>/` and, in
`.config/nextest.toml`, only additions of new test binaries to the `heavy-integration` or
`gui-smoke` filter. `docs/spec/backlog.md` (status line) and `docs/spec/manual-acceptance.md`
change only through the closing issue.

**Maintainer-reviewed stories:** every pull request of a story that writes to user data (M4:
S05, S06, S07) and of the milestone review's clean-up. The write path spreads over restic
arguments, path mapping, glob escaping, the preview check and the dialog, so a path list cannot
catch it.

## 6. Sync `main` → `m<n>`

Renovate and other sessions work on `main`; their changes reach `m<n>` only by a sync, which
the coordinating session creates outside the container (the agent's token cannot push workflow
changes): a fast-forward while `m<n>` has no commits of its own, afterwards a merge commit on
`claude/m<n>-sync-<yyyymmdd>`, never a rebase or a force-push. Syncs are bundled: after the Monday Renovate
merges, when a story needs a change from `main` and before the closing. An agent that needs a
change from `main` says so on its issue with `needs-maintainer`.

## 7. Records

The milestone, its issues (state, labels, comments) and the pull requests are the record. Dates
are absolute; no secrets and no paths of real data in issues, comments or pull requests.

## 8. Reports to the maintainer

| Moment | How |
| --- | --- |
| Kickoff | the maintainer approves the issues, this skill and the spec changes, sets the tag `m<n>-start` and removes `needs-maintainer` from the first story |
| Questions | issues labelled `needs-maintainer`; the maintainer looks at most once a day |
| Halt | the issue «M<n>: halt …» runs before the write path: auditor report (`prompts/auditor.md`), level-2 list and the design of the write path as comments on that issue, then `needs-maintainer`. The first write-path story starts labelled `needs-maintainer`; the maintainer approves by removing that label and closing the halt issue. |
| Closing | the issue «M<n>: closing …»: the closing pull request `gh pr create --base main --head m<n>` with auditor report, the write-path diff for line-by-line review, level-2 and level-3 log, test deltas and manual acceptance cards (`kestic-manual-acceptance`), then `needs-maintainer`. The maintainer merges it as a merge commit. |

## 9. Definition of Done of a milestone

- All stories done: non-manual ACs as tests green in `just verify`; manual ACs open in
  `docs/spec/manual-acceptance.md` with step-by-step instructions.
- Exit criteria of the milestone file met.
- Milestone review: `prompts/review-milestone-quality.md` plus the two story prompts over
  `git diff m<n>-start...origin/m<n>`; findings fixed in one clean-up pull request.
- README (English) matches the milestone (`AGENTS.md` step 8).
- Showcase site `site/`: milestone done in the roadmap, its features in «What it does» (glossary
  terms), its smoke screenshots in the manifest; `just site` green.
- Closing pull request `m<n>` → `main`, which the maintainer reviews and merges.

## 10. Operating in the agent container

Only this section is specific to the tooling (`.devcontainer/README.md`).

- The agent acts as the GitHub App `kestic-agent[bot]`; commits carry the maintainer as author.
- The watcher starts and continues sessions; the agent never polls for comments. When it waits
  for the maintainer, it ends its turn; a new comment, review, merge or label removal by the
  maintainer continues the session.
- Reviews run as two sub-agents that change nothing in the worktree (temp directories and temp
  repositories are fine); the model is set per review (§3).
- Commits, pull requests and comments carry no AI co-author trailers and no «Generated with»
  lines.
