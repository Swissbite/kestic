# Frozen review and audit prompts

Used by `kestic-milestone-autonomy` §3 step 6 (story reviews), §10 (milestone review) and §8
(auditor at the halt and at closing). Do not edit them during a milestone; changes are level 3.

## How to run a review

Give each reviewer, and nothing else:

1. the prompt file (`review-a-correctness.md`, `review-b-spec-rules.md`, `review-c-quality.md`,
   `review-d-break-it.md`), with `{{BASE}}`, `{{STORIES}}`, `{{SPEC_FILES}}` filled in;
2. read access to the worktree, so it can run `git diff {{BASE}}...HEAD` and read files.

Do **not** pass the pull request body, commit messages, the author's notes or summaries. The
reviewer judges the code against the spec, not against the author's intent.

At least one of the four reviewers runs on a different model than the author. If all four
report zero findings on a change of more than 400 changed lines (`git diff --shortstat`),
run `review-d-break-it.md` again with `strict-addendum.md` appended.

## Verdict format (all reviewers)

The reply ends with exactly one fenced JSON block:

```json
{
  "reviewer": "a | b | c | d",
  "base": "<base ref>",
  "head": "<head commit>",
  "findings": [
    {
      "id": "a-1",
      "severity": "must-fix | should-fix | note",
      "file": "core/src/restore.rs",
      "line": 42,
      "ac": "AC-M4-05-6 | null",
      "claim": "one sentence: what is wrong",
      "evidence": "how you know: quoted code, a command and its output, or the spec line",
      "suggested_test": "a test that fails today, or null"
    }
  ],
  "must_fix": 0,
  "verdict": "pass | fail"
}
```

- `must-fix`: a bug, an AC or rule violation, a security or data-loss risk, a test that would
  pass with the feature removed. `verdict` is `fail` when `must_fix > 0`.
- `should-fix`: clear improvement with low risk; the author fixes it or explains in the ledger.
- `note`: everything else. Keep notes few.
- A finding without evidence is a `note` at most.
