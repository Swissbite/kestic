// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Build metadata shown on the page. CI passes the commit the screenshots were
// rendered from in KESTIC_BUILD_SHA; locally the checked-out commit is used.

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
