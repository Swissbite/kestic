# Review (d) – Break it

You attack a change to Kestic, a KDE Plasma desktop app (Rust, CXX-Qt, Kirigami/QML) that runs
restic for the user. You only read the repository; you may create and use fresh temp
directories and temp restic repositories (password only via environment), nothing else.

**Change:** `git diff {{BASE}}...HEAD` in the current worktree.
**Stories:** {{STORIES}} – acceptance criteria in `docs/spec/milestones/`.
**Spec to read:** `AGENTS.md`, `docs/spec/restic-integration.md`, {{SPEC_FILES}}.

## Your task

For **every** AC of the stories, try to find one concrete input or state for which the
implementation does the wrong thing while the existing tests stay green. Think like a user
with unusual data and like an attacker controlling file names inside a snapshot:

- paths: `..`, absolute paths, `/` as snapshot root, symlinks pointing outside the target,
  hardlinks, very deep or very long paths, names with newlines, `*`, `?`, `[`, `\`, leading `-`,
  non-UTF-8 bytes, Unicode normalisation (NFC vs NFD), case differences;
- data: empty snapshot, 100 000 files, files changed between dry-run and restore, target
  folder appearing or filling between check and write, read-only target, full disk;
- users: snapshot of another user or host, different user name, missing home directory;
- timing: cancel during dry-run, during restore, daemon restart, a backup or maintenance run
  on the same destination;
- restic versions: 0.18.0 vs latest – fields that are missing or differ in the JSON.

For each AC give: the attack, what happens (by reading the code or by trying it in a temp
directory), and whether a test covers it. An attack that succeeds and is not covered is
`must-fix` with a `suggested_test` that fails today. If you are convinced an AC cannot be
broken, say why in one line.

Answer in the verdict format of `prompts/README.md` with `"reviewer": "d"`.
