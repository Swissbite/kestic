# Frozen review and audit prompts

Used by `kestic-milestone-autonomy` (story reviews, milestone review, auditor). Do not edit them
during a milestone; changes are level 3.

| Prompt | When |
| --- | --- |
| `review-correctness-spec.md` | every story pull request |
| `review-break-it.md` | every story pull request, on a different model than the author |
| `review-milestone-quality.md` | once, over the whole milestone diff |
| `auditor.md` | at the halt and at the closing |

Give each reviewer the prompt with `{{BASE}}`, `{{STORIES}}` and `{{SPEC_FILES}}` filled in, and
read access to the worktree – nothing else. No pull request body, commit messages or summaries
of the author. The auditor takes `{{MILESTONE}}` (`M4`), `{{BRANCH}}` (`m4`), `{{START}}` (commit
the milestone started from) and `{{GH_MILESTONE}}` (`M4 Restore`).

## Verdict format

The reply ends with exactly one fenced JSON block (`|` separates alternatives; `ac` and `line`
may be `null`):

```json
{
  "reviewer": "correctness-spec | break-it | quality",
  "head": "<head commit>",
  "findings": [
    {
      "severity": "must-fix | should-fix | note",
      "file": "core/src/restore.rs",
      "line": 42,
      "ac": "AC-M4-05-6",
      "claim": "one sentence: what is wrong",
      "evidence": "quoted code, a command and its output, or the spec line",
      "suggested_test": "a test that fails today, or null"
    }
  ],
  "must_fix": 0
}
```

`must-fix`: a bug, an AC or rule violation, a security or data-loss risk, a test that would pass
with the feature removed. `should-fix`: clear, low-risk improvement. `note`: everything else. A
finding without evidence is a `note` at most.
