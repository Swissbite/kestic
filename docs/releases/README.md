# Release highlights

Every release `v<major>.<minor>.<patch>` needs a highlights file `docs/releases/<tag>.md`, for example `docs/releases/v0.4.0.md`. It is the only hand-written part of the release notes; [`.github/workflows/release.yml`](../../.github/workflows/release.yml) fails without it.

## Who writes it, and when

The agent of a milestone writes the file in the closing pull request of the milestone into `main`, together with the version bump in the workspace `Cargo.toml` (`[workspace.package] version`, then `cargo update --workspace` for `Cargo.lock`). After the merge, the maintainer tags the merge commit on `main`:

```bash
git tag -a v0.4.0 -m "Kestic 0.4.0" && git push origin v0.4.0
```

The tag starts the release workflow. It fails if the tag is not on `main`, does not match the version in `Cargo.toml`, or the highlights file is missing or invalid. It then waits for the Verify run of the tagged commit (up to 90 minutes) and publishes only if that run is green, so tag a commit that had its own Verify run, normally the merge commit. If a later step fails, re-running the workflow replaces the draft it left; a published release is never changed. Agents never tag and never push to `main`.

| Version | After milestone |
| --- | --- |
| `v0.4.0` | M4 |
| `v0.5.0` | M5 |
| `v1.0.0` | M6 (MVP) |

Every `0.x` release is a pre-release. A fix release (`v0.4.1`) takes the same milestone as its minor version.

## Format

```markdown
---
milestones:
  - M4 Restore
---

Kestic 0.4.0 brings your data back: …

- **Browse snapshots** …
```

- **Front matter**, strictly in this form:
  - `milestones:` lists GitHub milestones by their exact title, one `  - <title>` line each; a title without a matching milestone is an error. A `0.x` release names exactly one, and it must be milestone `M<minor>` (`v0.4.0` → `M4 …`). From `1.0.0` on, the file may name several, for example all MVP milestones without an earlier release.
  - `title:` (optional) is the release title after the tag. Without it the title is the milestone title: `v0.4.0 – M4 Restore`. It is required when the file names several milestones.
  - Any other line in the front matter is an error.
- **Body** (required): the highlights in Markdown, written for users, in English. Say what users can do now; leave out story IDs, internal names and anything the generated parts already list.

## What the release notes contain

1. The body of this file.
2. **Stories:** every closed story issue (title `M<n>-S<nn>: …`, closed as completed) of the named milestones, each with a link, and a link to the milestone. Open stories cause a warning in the workflow.
3. GitHub's generated notes: all pull requests since the previous version tag, grouped by label as configured in [`.github/release.yml`](../../.github/release.yml).

The release also carries the `.deb`, the GUI smoke screenshots of the tagged commit (`kestic_<version>_screenshots.zip`), `SHA256SUMS` and a build provenance attestation of the `.deb` (`gh attestation verify kestic_<version>-1_amd64.deb --repo Swissbite/kestic`).

## Checking a release before tagging

A pull request from this repository into `main` or a milestone branch (`m4`, `m5`, …) that adds or changes a highlights file runs the release workflow as a dry run, for the tag `v<version in Cargo.toml>`: put the version bump and the highlights file in the same pull request. A manual run (`gh workflow run release.yml --ref <branch>`, optional input `tag`) is a dry run too. A dry run does everything except the attestation and the release and shows notes and assets in the job summary of `publish`. Without a Verify run of its commit it takes the screenshots of the latest green Verify run on `main`; a red Verify run of the commit fails the dry run.
