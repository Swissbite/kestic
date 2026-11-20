// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

import { fileURLToPath } from "node:url";
import { defineConfig } from "astro/config";
import screenshots from "./src/integrations/screenshots.ts";

const repoRoot = fileURLToPath(new URL("..", import.meta.url));

export default defineConfig({
  site: "https://swissbite.github.io",
  base: "/kestic",
  output: "static",
  trailingSlash: "ignore",
  i18n: {
    defaultLocale: "en",
    locales: ["en"],
  },
  integrations: [screenshots()],
  vite: {
    server: {
      // The app icon is imported from packaging/ (single source).
      fs: { allow: [repoRoot] },
    },
  },
});
