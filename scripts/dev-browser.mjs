// A signed-in browser on the dev app for Playwright scripts (#616), and the
// sign-in `just proto-new` shares with it.
//
// A live acceptance script keeps to `.playwright-mcp/`, which Git ignores:
//
//     // .playwright-mcp/check.mjs, run with `node .playwright-mcp/check.mjs`
//     import { signedInBrowser } from "../scripts/dev-browser.mjs";
//
//     const app = await signedInBrowser({ size: "phone", theme: "dark" });
//     try {
//       const page = await app.open("/v/<vault id>/n/<slug>");
//       await page.screenshot({ path: ".playwright-mcp/note.png" });
//     } finally {
//       await app.close();
//     }
//
// `signedInBrowser` starts a headless browser with nothing remembered from an
// earlier run: no stored view, theme, draft or last note. It signs in with
// the dev web token, marks What's new as seen so its dialog never covers the
// screen, and sets the theme asked for. `open` returns the page only once the
// app has drawn. The sizes are the ones `just proto-new` captures at.
//
// The token is read here, from the environment or `.env`, and put straight
// into the browser, so it passes through no tool call and no transcript.
// `app.clean(text)` cuts it from page text a script is about to print.

import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import {
  DEV_HOST,
  FRONTEND_PORT,
  WEB_TOKEN_SETTING,
  devSetting,
  scrubbed,
} from "./dev-common.mjs";
import { ROOT, SIZES, THEMES, ToolError } from "./proto-common.mjs";

// The keys the app reads its token and its theme from (`frontend/src/api/api.ts`
// and `frontend/src/app/constants.ts`).
const TOKEN_KEY = "hatchdoor_web_token";
const THEME_KEY = "hatchdoor.theme";
// What's new (`frontend/src/features/whats-new/whatsNew.ts`) opens over every
// screen in a browser that has never seen a release. It belongs to the fresh
// browser, not to the screen, so the browser says it saw them all.
const SEEN_VERSION_KEY = "hatchdoor_whats_new_seen";
const SEEN_EVERYTHING = "999999.0.0";

const CHROME_NAMES = [
  "google-chrome",
  "google-chrome-stable",
  "chromium",
  "chromium-browser",
];

export function chromePath() {
  if (process.env.HATCHDOOR_DEV_BROWSER) {
    return process.env.HATCHDOOR_DEV_BROWSER;
  }
  for (const dir of (process.env.PATH ?? "").split(path.delimiter)) {
    for (const name of CHROME_NAMES) {
      const candidate = path.join(dir, name);
      if (dir && fs.existsSync(candidate)) {
        return candidate;
      }
    }
  }
  throw new ToolError(
    "no Chrome or Chromium is on the path; name one in HATCHDOOR_DEV_BROWSER",
  );
}

export function playwright() {
  const require = createRequire(path.join(ROOT, "frontend", "package.json"));
  try {
    return require("playwright-core");
  } catch {
    throw new ToolError(
      "playwright-core is not installed; run `npm ci` in frontend/",
    );
  }
}

// What a fresh browser holds before the app first loads, as `[key, value]`
// pairs for `localStorage`: the sign-in, the theme, and What's new as seen.
// Nothing else, so no view or note is remembered.
export function storedState({ token, theme }) {
  return [
    ...(token ? [[TOKEN_KEY, token]] : []),
    [THEME_KEY, theme],
    [SEEN_VERSION_KEY, SEEN_EVERYTHING],
  ];
}

// The browser context for one size and theme.
export function contextOptions({ size, theme, token }) {
  const [width, height] = SIZES[size];
  // Phone and tablet are touch screens, which the app reads as
  // `(pointer: coarse)` to choose, for one, the editor's toolbar.
  const touch = size !== "desktop";
  return {
    viewport: { width, height },
    hasTouch: touch,
    isMobile: touch,
    // The service worker is kept out: it answers from its own cache, and a
    // check is about what the code on disk draws today.
    serviceWorkers: "block",
    reducedMotion: "reduce",
    colorScheme: theme,
    extraHTTPHeaders: token ? { Authorization: `Bearer ${token}` } : {},
  };
}

// Go to `url` and come back once the app has drawn it.
export async function settle(page, url) {
  await page.goto(url);
  // The network can fall quiet before React has drawn anything, which would
  // read an empty page; the data the first draw asks for comes after.
  try {
    await page.waitForFunction(
      () => (document.getElementById("root")?.children.length ?? 0) > 0,
      undefined,
      { timeout: 30_000 },
    );
  } catch {
    throw new ToolError(`${url} drew nothing within 30 seconds`);
  }
  await page.waitForLoadState("networkidle");
  await page.evaluate(async () => {
    for (const img of document.querySelectorAll("img[loading=lazy]")) {
      img.loading = "eager";
    }
    await document.fonts.ready;
  });
  await page.waitForLoadState("networkidle");
  await page.evaluate(() =>
    Promise.all([...document.images].map((img) => img.decode().catch(() => {}))),
  );
}

// A fresh, signed-in browser on the dev frontend. See the top of this file.
export async function signedInBrowser({
  size = "desktop",
  theme = "light",
  frontendPort = FRONTEND_PORT,
  token = devSetting(WEB_TOKEN_SETTING),
} = {}) {
  if (!(size in SIZES) || !THEMES.includes(theme)) {
    throw new ToolError(
      `a size is one of ${Object.keys(SIZES).join(", ")} and a theme one of ${THEMES.join(", ")}`,
    );
  }
  const origin = `http://${DEV_HOST}:${frontendPort}`;
  try {
    await fetch(origin);
  } catch {
    throw new ToolError(
      `the dev frontend did not answer at ${origin}; \`just dev-start\` starts it and \`just dev-wait\` waits for it`,
    );
  }
  const browser = await playwright().chromium.launch({
    executablePath: chromePath(),
  });
  try {
    const context = await browser.newContext(
      contextOptions({ size, theme, token }),
    );
    // Runs before the app's own scripts on every load, and writes on the
    // first one only (the tab remembers), so what a check changes, such as
    // the theme or signing out, outlives a reload.
    await context.addInitScript((stored) => {
      if (sessionStorage.getItem("hatchdoor-dev-browser") === null) {
        sessionStorage.setItem("hatchdoor-dev-browser", "1");
        for (const [key, value] of stored) {
          localStorage.setItem(key, value);
        }
      }
    }, storedState({ token, theme }));
    const page = await context.newPage();
    return {
      browser,
      context,
      page,
      origin,
      // `route` starts with a slash, as in `/graph`.
      async open(route = "/") {
        await settle(page, origin + route);
        return page;
      },
      clean: (text) => scrubbed(text, [token]),
      close: () => browser.close(),
    };
  } catch (error) {
    await browser.close();
    throw error;
  }
}
