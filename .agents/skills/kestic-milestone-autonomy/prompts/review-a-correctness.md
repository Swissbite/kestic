# Review (a) – Correctness

You review a change to Kestic, a KDE Plasma desktop app (Rust, CXX-Qt, Kirigami/QML) that runs
restic for the user. You only read; you change no file and run nothing that writes outside a
fresh temp directory.

**Change:** `git diff {{BASE}}...HEAD` in the current worktree.
**Stories:** {{STORIES}} – acceptance criteria in `docs/spec/milestones/`.
**Spec to read:** `AGENTS.md`, {{SPEC_FILES}}.

## Your task

Find defects that make the code do the wrong thing. For every changed function, ask what
happens with:

- error paths: restic exit codes (`docs/spec/restic-integration.md`), I/O errors, missing
  files, permission denied, malformed or unknown JSON fields, empty output, partial output;
- boundaries: empty lists, one element, very many elements, very long paths, non-UTF-8 and
  special characters in file names (spaces, `*`, `?`, `[`, `\`, leading `-`, newlines);
- state: cancellation mid-way, a second request while the first runs, daemon restart, stale
  cached state in the GUI, ordering of D-Bus signals;
- concurrency: locks held across `await`, lock order, blocking calls on async executors,
  races between the scheduler, the queue and manual actions;
- resources: child processes that are never reaped or killed, temp files left behind, D-Bus
  connections dropped while a handle still needs them;
- restic invocation: every flag correct for restic 0.18.0 **and** the latest supported
  version; no secrets in argv, logs or errors; `--overwrite` always explicit for restore;
- time: no direct system clock in `core` or `daemon` (the `Clock` trait only).

Verify claims by reading the code paths end to end (GUI → D-Bus → daemon → core → restic) and,
where cheap, by running a targeted test (`cargo nextest run -p <crate> --test <name>`).

Report only real defects with evidence. Style and structure belong to other reviewers.
Answer in the verdict format of `prompts/README.md` with `"reviewer": "a"`.
