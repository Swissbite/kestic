# Frozen review and audit prompts

Used by `kestic-milestone-autonomy` (story reviews, milestone review, auditor). Do not edit them
during a milestone; changes are level 3.

| Prompt | When |
| --- | --- |
| `review-correctness-spec.md` | every story pull request |
| `review-break-it.md` | every story pull request, on a different model than the author; in contract mode before the code of a maintainer-reviewed story |
| `review-ui.md` | every story pull request that changes `gui/` or `po/`, and once over all smoke screenshots in the milestone review |
| `review-milestone-quality.md` | once, over the whole milestone diff |
| `auditor.md` | at the halt and at the closing |

Give each reviewer the prompt with `{{BASE}}`, `{{STORIES}}` and `{{SPEC_FILES}}` (the UI review also
`{{SCREENSHOTS}}`: the list of PNG paths) filled in, and
read access to the worktree – nothing else. No pull request body, commit messages or summaries
of the author. The auditor takes `{{MILESTONE}}` (`M4`), `{{BRANCH}}` (`m4`), `{{START}}` (commit
the milestone started from: the tag `m4-start`) and `{{GH_MILESTONE}}` (`M4 Restore`). In
contract mode the break-it review gets `{{CONTRACT}}` (the text of the story contract comment),
`{{STORIES}}` and `{{SPEC_FILES}}`, no `{{BASE}}`, and answers with `"head": null`.

## Verdict format

The reply ends with exactly one fenced JSON block (`|` separates alternatives; `ac` and `line`
may be `null`):

```json
{
  "reviewer": "correctness-spec | break-it | ui | quality",
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
