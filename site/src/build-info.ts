// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Build metadata shown on the page. CI passes the commit the screenshots were
// rendered from in KESTIC_BUILD_SHA; locally the checked-out commit is used.
// CI also passes the newest final release (KESTIC_RELEASE_TAG,
// KESTIC_RELEASE_DEB_URL); without them the page says there is no release yet.

import { execFileSync } from "node:child_process";

function resolveSha(): string {
  const fromEnv = process.env["KESTIC_BUILD_SHA"];
  if (fromEnv !== undefined && fromEnv !== "") {
    return fromEnv;
  }
  try {
    return execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  } catch {
    return "unknown";
  }
}

export const buildSha: string = resolveSha();
export const buildShaShort: string = buildSha.slice(0, 7);
export const repoUrl = "https://github.com/Swissbite/kestic";
export const commitUrl = `${repoUrl}/commit/${buildSha}`;

export interface Release {
  /** Tag of the release, e.g. `v0.4.0`. */
  tag: string;
  /** Page of the release on GitHub. */
  url: string;
  /** Download URL of the amd64 package. */
  debUrl: string;
  /** File name of the amd64 package, as `apt install` gets it. */
  debFile: string;
}

// Only final versions count: pre-releases of integration branches (`-m4.21`)
// and release candidates (`-rc.1`) carry a suffix.
const FINAL_TAG = /^v[0-9]+\.[0-9]+\.[0-9]+$/;
const DEB_FILE = /^kestic_[0-9A-Za-z.+~-]+_amd64\.deb$/;

function envValue(name: string): string {
  return (process.env[name] ?? "").trim();
}

function resolveRelease(): Release | null {
  const tag = envValue("KESTIC_RELEASE_TAG");
  const debUrl = envValue("KESTIC_RELEASE_DEB_URL");
  if (tag === "" && debUrl === "") {
    return null;
  }
  if (!FINAL_TAG.test(tag)) {
    throw new Error(`KESTIC_RELEASE_TAG "${tag}" is not a final version tag like v1.2.3`);
  }
  const downloadPrefix = `${repoUrl}/releases/download/${tag}/`;
  const debFile = debUrl.startsWith(downloadPrefix) ? debUrl.slice(downloadPrefix.length) : "";
  if (!DEB_FILE.test(debFile)) {
    throw new Error(
      `KESTIC_RELEASE_DEB_URL "${debUrl}" is not an amd64 package of ${tag} (expected ${downloadPrefix}kestic_…_amd64.deb)`,
    );
  }
  return { tag, url: `${repoUrl}/releases/tag/${tag}`, debUrl, debFile };
}

export const release: Release | null = resolveRelease();
