// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Copies the screenshots listed in `src/screenshots.ts` from the GUI smoke
// artifacts into `public/screenshots/` before dev or build. Only listed PNGs
// are read; nothing from the artifact directory is executed.

import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { AstroIntegration } from "astro";
import { allShots, type Screenshot } from "../screenshots.ts";

const PNG_SIGNATURE = "89504e470d0a1a0a";

function pngSize(path: string): { width: number; height: number } {
  const header = readFileSync(path).subarray(0, 24);
  if (header.subarray(0, 8).toString("hex") !== PNG_SIGNATURE) {
    throw new Error(`${path} is not a PNG file`);
  }
  return { width: header.readUInt32BE(16), height: header.readUInt32BE(20) };
}

function problemsWith(shot: Screenshot, sourceDir: string): string | null {
  const path = join(sourceDir, shot.file);
  if (!existsSync(path)) {
    return `${shot.file}: missing`;
  }
  const size = pngSize(path);
  if (size.width !== shot.width || size.height !== shot.height) {
    return `${shot.file}: ${size.width}x${size.height}, manifest says ${shot.width}x${shot.height}`;
  }
  return null;
}

export default function screenshots(): AstroIntegration {
  return {
    name: "kestic-screenshots",
    hooks: {
      "astro:config:setup": ({ command, config, logger }) => {
        if (command === "preview") {
          // Preview serves the finished `dist/`; nothing to copy.
          return;
        }
        const siteRoot = fileURLToPath(config.root);
        const fromEnv = process.env["KESTIC_SCREENSHOTS_DIR"];
        const sourceDir =
          fromEnv !== undefined && fromEnv !== ""
            ? isAbsolute(fromEnv)
              ? fromEnv
              : resolve(siteRoot, fromEnv)
            : resolve(siteRoot, "..", "target", "smoke-artifacts");

        const problems = allShots
          .map((shot) => problemsWith(shot, sourceDir))
          .filter((problem): problem is string => problem !== null);
        if (problems.length > 0) {
          throw new Error(
            [
              `Screenshots in ${sourceDir} do not match src/screenshots.ts:`,
              ...problems.map((problem) => `  - ${problem}`),
              "Run `just smoke` / `just verify`, or point KESTIC_SCREENSHOTS_DIR at the smoke-artifacts directory.",
            ].join("\n"),
          );
        }

        const targetDir = join(fileURLToPath(config.publicDir), "screenshots");
        rmSync(targetDir, { recursive: true, force: true });
        mkdirSync(targetDir, { recursive: true });
        for (const shot of allShots) {
          copyFileSync(join(sourceDir, shot.file), join(targetDir, shot.file));
        }
        logger.info(`copied ${allShots.length} screenshots from ${sourceDir}`);
      },
    },
  };
}
