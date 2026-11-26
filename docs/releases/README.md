# Releases

Kestic's version numbers come only from Git tags `v<major>.<minor>.<patch>[-<pre-release>]`. No commit changes a version: `Cargo.toml` stays at `0.0.0`, and a build takes its version from the nearest tag (details in [architecture.md §Release](../spec/architecture.md#release), German). Tags are created by [`.github/workflows/release.yml`](../../.github/workflows/release.yml) only.

| Version | When |
| --- | --- |
| `v0.4.0-m4.<k>` | automatically, on every merge into the integration branch `m4` (pre-release) |
| `v0.4.0` | after milestone M4, started by the maintainer |
| `v0.5.0` | after M5 |
| `v0.6.0` | after M6 (end of the MVP scope; `v1.0.0` is decided by the maintainer later) |

A version without a suffix (`v0.6.0`, later `v1.x`) is a normal GitHub release marked «Latest». Only the branch builds `v0.<n>.0-m<n>.<k>` and the release candidates `-rc.<k>` are pre-releases, never «Latest». In apt, `0.4.0` replaces every `0.4.0~m4.<k>`.

## Starting a release (maintainer only)

1. The closing pull request of the milestone, with the highlights file (below), is merged into `main` with a merge commit (no squash, no rebase), so the branch's pre-release tags are on `main` and `minor` continues from them.
2. In GitHub: Actions → **Release** → **Run workflow** on `main`. Pick the bump. `minor` turns the milestone's pre-releases (`v0.4.0-m4.12`) into `v0.4.0`. `major`, `minor`, `patch`, `premajor`, `preminor`, `prepatch` and `prerelease` work like npm's `semver inc`; the `pre…` bumps give release candidates (`v0.6.0-rc.1`).
3. Leave **dry run** ticked first. That run shows the version, the release notes and the assets in the job summary of `publish` and creates nothing. Without a Verify run of the commit it takes the screenshots of the latest green Verify run on `main`, or goes on without screenshots.
4. Run it again with **dry run** unticked. The workflow waits for the green Verify run of the commit on `main` (it fails if the run does not appear within 5 minutes; then it waits up to 90 minutes), builds the `.deb` and creates the tag through the API. Then it publishes the release with the `.deb`, the GUI smoke screenshots (`kestic_<version>_screenshots.zip`), `SHA256SUMS` and a build provenance attestation of the `.deb` (`gh attestation verify kestic_<version>-1_amd64.deb --repo Swissbite/kestic`).

If a step fails, use **Re-run failed jobs** on the same run: it reuses its tag and replaces its own draft. **Re-run all jobs** or a new run refuses a commit that already carries a release tag. A published release is never changed.

Agents cannot start the workflow: the agents' GitHub App only has `actions: read`. A ruleset on `refs/tags/v*` lets only the admin role and the GitHub App «Kestic Release» create or delete version tags; the workflow uses a short-lived token of that app (variable `RELEASE_APP_ID`, secret `RELEASE_APP_PRIVATE_KEY`).

## Pre-releases of an integration branch

Every push to `m<n>` (normally a merged story pull request) publishes `v0.<n>.0-m<n>.<k>`, where `k` is the number of first-parent commits since the tag `m<n>-start`. The maintainer sets that tag on the start commit when creating the branch; without it the run fails. The branch needs this workflow, so merge `main` into it once this workflow is on `main`. Its notes list:
- the closed story issues of milestone M`<n>` that the merged pull request refers to (`#<n>` in its title or body);
- the pull requests since the previous pre-release of the branch.

No highlights file is needed, and the workflow does not wait for Verify: the pull request was green before the merge. Only the newest three pre-releases per branch stay; older ones are deleted together with their tags.

## The highlights file

Every release (not the pre-releases) needs `docs/releases/<tag>.md`, for example `docs/releases/v0.4.0.md`. It is the only hand-written part of the release notes. The agent of the milestone writes it in the closing pull request into `main`. A release candidate (`v0.5.0-rc.1`) uses `docs/releases/v0.5.0.md` if it exists.

```markdown
---
milestones:
  - M4 Restore
---

Kestic 0.4.0 brings your data back: …

- **Browse snapshots** …
```

- **Front matter**, strictly in this form:
  - `milestones:` lists GitHub milestones by their exact title, one `  - <title>` line each; a title without a matching milestone is an error. A `0.x` release names exactly one, and it must be milestone `M<minor>` (`v0.4.0` → `M4 …`). From `1.0.0` on, the file may name several.
  - `title:` (optional) is the release title after the tag. Without it the title is the milestone title: `v0.4.0 – M4 Restore`. It is required when the file names several milestones.
  - Any other line in the front matter is an error.
- **Body** (required): the highlights in Markdown, written for users, in English. Say what users can do now; leave out story IDs, internal names and anything the generated parts already list.

## What the release notes contain

1. The body of the highlights file.
2. **Stories:** every closed story issue (title `M<n>-S<nn>: …`, closed as completed) of the named milestones, each with a link, and a link to the milestone. The workflow warns about open stories.
3. GitHub's generated notes: all pull requests since the previous release without a pre-release suffix (for a branch pre-release: since the branch's previous pre-release), grouped by label as configured in [`.github/release.yml`](../../.github/release.yml).
