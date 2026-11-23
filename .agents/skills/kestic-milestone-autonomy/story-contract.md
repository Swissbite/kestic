# Story contract, tests first, prevention

Required at step 2 of §3 in [SKILL.md](SKILL.md). For a story with one AC or without code, one
line per AC is enough.

## 1. Contract

Before any code, post one comment titled `Story contract` on the issue:

| AC | Observable behaviour | Test: name, level | Where: file, function |
| --- | --- | --- | --- |

- One row per AC; its sub-statements (a), (b), … go into the behaviour cell.
- A story that starts, cancels or queues runs, or changes daemon state, adds a table
  state × event → next state, with cancel, a second start, a daemon restart and a run on the
  same destination.
- **Open questions** end the comment: only level-3 points (§2), each with options and a
  recommendation; decide level 1 and 2 yourself. With open questions, add `needs-maintainer`
  and end the turn; the next session reads the answer and goes on here. Without, go on.

## 2. Contract review (maintainer-reviewed stories, §5)

Run `prompts/review-break-it.md` once in contract mode, on a different model than the author.
Add each `must-fix` as a test row; one that needs a decision becomes an open question (§1). Its
verdict does not count toward the two review rounds of §3 step 4 and does not go into the pull
request body.

## 3. Tests first

Write the contract's tests against stubs with the final signatures and `todo!()` bodies. Run
only those tests (`cargo test -p …`): each must fail on an assertion or a `todo!()`, never on a
compile error. A test that passes before the feature exists does not check it: fix the test.
Then replace the stubs with the real code; clippy and `just verify` run only after that.

## 4. Prevention and numbers

- Pull request body, section **Prevention**: each `must-fix` with the lint, test or skill line
  that would have caught it earlier, or «none».
- The comment that closes the issue: minutes from the story contract comment to the draft pull
  request, review rounds, `must-fix` findings per reviewer, red CI runs.
