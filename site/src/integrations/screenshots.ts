// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Copies the screenshots listed in `src/screenshots.ts` from the GUI smoke
// artifacts into `public/screenshots/` before dev or build. Only listed PNGs
// are read; nothing from the artifact directory is executed.
//
// A screenshot that is missing or does not match the manifest fails the build.
// With KESTIC_SCREENSHOTS_ALLOW_MISSING=true (builds of pull requests, which
// take the screenshots of main while the manifest may already name new ones)
// the problems are only warnings and the matching screenshots are copied.

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
  const { x, y, width, height } = shot.crop;
  if (x < 0 || y < 0 || width <= 0 || height <= 0 || x + width > shot.width || y + height > shot.height) {
    return `${shot.file}: crop ${x},${y} ${width}x${height} lies outside the image`;
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

        const checked = allShots.map((shot) => ({ shot, problem: problemsWith(shot, sourceDir) }));
        const problems = checked
          .map(({ problem }) => problem)
          .filter((problem): problem is string => problem !== null);
        if (problems.length > 0) {
          const report = [
            `Screenshots in ${sourceDir} do not match src/screenshots.ts:`,
            ...problems.map((problem) => `  - ${problem}`),
          ];
          if (process.env["KESTIC_SCREENSHOTS_ALLOW_MISSING"] !== "true") {
            throw new Error(
              [
                ...report,
                "Run `just smoke` / `just verify`, or point KESTIC_SCREENSHOTS_DIR at the smoke-artifacts directory.",
              ].join("\n"),
            );
          }
          logger.warn([...report, "KESTIC_SCREENSHOTS_ALLOW_MISSING=true: building without them."].join("\n"));
        }

        const targetDir = join(fileURLToPath(config.publicDir), "screenshots");
        rmSync(targetDir, { recursive: true, force: true });
        mkdirSync(targetDir, { recursive: true });
        const usable = checked.filter(({ problem }) => problem === null).map(({ shot }) => shot);
        for (const shot of usable) {
          copyFileSync(join(sourceDir, shot.file), join(targetDir, shot.file));
        }
        logger.info(`copied ${usable.length} of ${allShots.length} screenshots from ${sourceDir}`);
      },
    },
  };
}
