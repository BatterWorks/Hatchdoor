// `just proto-new <issue>-<slug> <path>...` (#602): the start of a prototype,
// as a static copy of the screens the dev instance draws today.
//
// It makes `.dev/prototypes/<issue>-<slug>/` holding:
//
// - `captured/<path>.<size>.<theme>.html`, each path at every size in
//   `SIZES` and in each theme. A page is the screen's `<body>` without its
//   scripts, so nothing in it runs, saves or goes anywhere.
// - `assets/app.css`, the stylesheet a production build ships, with its fonts
//   beside it. The dev server injects its styles with JavaScript, so a page
//   links this instead.
// - `media/`, every same-origin image the pages show.
// - `index.html`, a link to every page.
//
// Pages load what they need from the folder's root (`/assets/app.css`), so a
// build script can write option pages anywhere in the folder and they still
// find it. `just proto-serve` serves the folder as that root.
//
// The browser signs in with the dev web token, read the way the backend reads
// it, and never prints it. A canvas, such as the graph, is captured empty.
// Running it again with more paths adds to the folder and redraws the index.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import {
  PROTOTYPES_DIR,
  ROOT,
  SIZES,
  THEMES,
  ToolError,
  run,
} from "./proto-common.mjs";

const TOKEN_SETTING = "HATCHDOOR_WEB_BEARER_TOKEN";
// The keys the app reads its token and its theme from (`frontend/src/api/api.ts`
// and `frontend/src/app/constants.ts`).
const TOKEN_KEY = "hatchdoor_web_token";
const THEME_KEY = "hatchdoor.theme";
// The query parameter `withAccessToken` (`frontend/src/api/api.ts`) puts the
// web token in, on note images among others. A prototype is served to the
// whole LAN, so it is cut from every address before anything is written.
const ACCESS_TOKEN_PARAM = /([?&])access_token=[^&#"'\s]*(&?)/g;
// What's new (`frontend/src/features/whats-new/whatsNew.ts`) opens over every
// screen in a browser that has never seen a release. It belongs to the fresh
// capture browser, not to the screen, so the capture says it saw them all.
const SEEN_VERSION_KEY = "hatchdoor_whats_new_seen";
const SEEN_EVERYTHING = "999999.0.0";

const CHROME_NAMES = [
  "google-chrome",
  "google-chrome-stable",
  "chromium",
  "chromium-browser",
];

// The web token: the environment first, then `.env`, as the backend and
// `scripts/dev-browser-token.py` find it.
export function tokenFrom(environment, envFileText) {
  const exported = (environment[TOKEN_SETTING] ?? "").trim();
  if (exported) {
    return exported;
  }
  for (const line of (envFileText ?? "").split("\n")) {
    const [name, ...rest] = line.trim().split("=");
    if (name === TOKEN_SETTING) {
      return rest.join("=").trim().replace(/^(["'])(.*)\1$/, "$2");
    }
  }
  return "";
}

// A path as part of a file name: `/notes/a b?x=1` is `notes-a-b-x-1`.
export function slugOf(route) {
  const slug = route.replace(/[^A-Za-z0-9_]+/g, "-").replace(/^-+|-+$/g, "");
  return slug || "home";
}

// The file an image is kept in under `media/`: its path without the query,
// made safe for a file name, with a short hash of the path so two addresses
// that make the same safe name stay two files.
export function mediaName(src) {
  const pathname = src.split(/[?#]/)[0];
  const safe = pathname.replace(/^\/+/, "").replace(/[^A-Za-z0-9._-]+/g, "_");
  const dot = safe.lastIndexOf(".");
  const [stem, extension] =
    dot > 0 ? [safe.slice(0, dot), safe.slice(dot)] : [safe, ""];
  const hash = createHash("sha256").update(pathname).digest("hex").slice(0, 8);
  return `${stem}-${hash}${extension}`;
}

// `text` with every `access_token` parameter cut out. Refuses outright when
// the token is still there in any other form, so a page that carries it is
// never written.
export function withoutToken(text, token) {
  // `?access_token=x&y` keeps `?y`, `&access_token=x&y` keeps `&y`, and a
  // last parameter goes with its separator.
  const cut = text.replace(ACCESS_TOKEN_PARAM, (_, lead, trail) =>
    trail ? lead : "",
  );
  if (token && (cut.includes(token) || cut.includes(encodeURIComponent(token)))) {
    throw new ToolError(
      "a captured page still holds the web token, so it was not written",
    );
  }
  return cut;
}

function escaped(text) {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/"/g, "&quot;");
}

export function pageOf({ title, htmlAttributes, styles, body }) {
  return (
    `<!doctype html>\n<html${htmlAttributes}>\n<head>\n<meta charset="utf-8">\n` +
    '<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">\n' +
    `<title>${escaped(title)}</title>\n` +
    // No icon is asked for, so no missing one is logged.
    '<link rel="icon" href="data:,">\n' +
    '<link rel="stylesheet" href="/assets/app.css">\n' +
    `${styles}</head>\n${body}\n</html>\n`
  );
}

// The index: every captured page, by path, then size, then theme.
export function indexOf(name, pages) {
  const byRoute = new Map();
  for (const page of pages) {
    const [route, size, theme] = page.replace(/\.html$/, "").split(".");
    byRoute.set(route, [...(byRoute.get(route) ?? []), { page, size, theme }]);
  }
  const sizes = Object.keys(SIZES);
  let list = "";
  for (const [route, entries] of [...byRoute].sort(([a], [b]) =>
    a.localeCompare(b),
  )) {
    const links = entries
      .sort(
        (a, b) =>
          sizes.indexOf(a.size) - sizes.indexOf(b.size) ||
          THEMES.indexOf(a.theme) - THEMES.indexOf(b.theme),
      )
      .map(({ page, size, theme }) => {
        const dimensions = SIZES[size];
        const measure = dimensions ? ` ${dimensions[0]} × ${dimensions[1]}` : "";
        return `<li><a href="captured/${page}">${size}, ${theme}</a>${measure}</li>`;
      })
      .join("\n");
    list += `<h2>${escaped(route)}</h2>\n<ul>\n${links}\n</ul>\n`;
  }
  return (
    '<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n' +
    '<meta name="viewport" content="width=device-width, initial-scale=1">\n' +
    `<title>${escaped(name)}</title>\n<link rel="icon" href="data:,">\n` +
    "<style>body{font:16px/1.5 system-ui,sans-serif;margin:24px;max-width:40rem}li{margin:4px 0}</style>\n" +
    `</head>\n<body>\n<h1>${escaped(name)}</h1>\n` +
    "<p>The screens as the dev instance drew them when this was captured. " +
    "Open each at the size it names.</p>\n" +
    `${list}</body>\n</html>\n`
  );
}

// Read the screen a page shows into something that stands alone. Runs in the
// browser, where nothing of this file but this function exists.
function capture(theme) {
  // What was typed is a property; a copy of the markup carries attributes.
  for (const input of document.querySelectorAll("input")) {
    input.setAttribute("value", input.value);
  }
  for (const area of document.querySelectorAll("textarea")) {
    area.textContent = area.value;
  }

  const body = document.body.cloneNode(true);
  for (const script of body.querySelectorAll("script")) {
    script.remove();
  }
  // Nothing behind a link was captured, so a link goes nowhere.
  for (const link of body.querySelectorAll("a[href]")) {
    link.setAttribute("href", "#");
  }

  const media = [];
  for (const img of body.querySelectorAll("img")) {
    img.removeAttribute("srcset");
    img.removeAttribute("loading");
    const src = img.getAttribute("src") ?? "";
    if (src.startsWith("/") && !src.startsWith("//")) {
      media.push(src);
      img.setAttribute("src", "hatchdoor-media:" + encodeURIComponent(src));
    }
  }

  // Vite's own `<style data-vite-dev-id>` elements are the app's stylesheet,
  // which the copy links as built. What a library injected at run time, such
  // as the editor's styles, is kept.
  const styles = [...document.head.querySelectorAll("style")]
    .filter((style) => !style.hasAttribute("data-vite-dev-id"))
    .map((style) => `<style>${style.textContent}</style>\n`)
    .join("");

  // Pinned, so the page shows this theme whatever the viewer's device prefers.
  const attributes = [...document.documentElement.attributes]
    .filter((attribute) => attribute.name !== "data-theme")
    .map((attribute) => [attribute.name, attribute.value]);
  attributes.push(["data-theme", theme]);

  return {
    title: document.title,
    attributes,
    styles,
    body: body.outerHTML,
    media,
  };
}

const MEDIA_MARK = /hatchdoor-media:([A-Za-z0-9%._~!*'()-]+)/g;

function chromePath() {
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

function playwright() {
  const require = createRequire(path.join(ROOT, "frontend", "package.json"));
  try {
    return require("playwright-core");
  } catch {
    throw new ToolError(
      "playwright-core is not installed; run `npm ci` in frontend/",
    );
  }
}

// The stylesheet a production build ships, and its fonts, into `assets/`.
// Built into a scratch folder so nothing of the real build output moves.
function buildStylesheet(folder) {
  const scratch = fs.mkdtempSync(
    path.join(process.env.TMPDIR ?? os.tmpdir(), "hatchdoor-proto-"),
  );
  try {
    const built = spawnSync(
      "npx",
      ["vite", "build", "--outDir", scratch, "--emptyOutDir", "--logLevel", "error"],
      { cwd: path.join(ROOT, "frontend"), encoding: "utf8" },
    );
    if (built.status !== 0) {
      throw new ToolError(`the frontend did not build:\n${built.stderr}`);
    }
    const assets = path.join(folder, "assets");
    fs.rmSync(assets, { recursive: true, force: true });
    fs.mkdirSync(assets, { recursive: true });
    const files = fs.readdirSync(path.join(scratch, "assets"));
    // The entry stylesheet first, then the ones lazy chunks bring.
    const sheets = files
      .filter((file) => file.endsWith(".css"))
      .sort((a, b) => Number(!a.startsWith("index-")) - Number(!b.startsWith("index-")));
    fs.writeFileSync(
      path.join(assets, "app.css"),
      sheets
        .map((file) =>
          fs.readFileSync(path.join(scratch, "assets", file), "utf8"),
        )
        .join("\n"),
    );
    for (const file of files) {
      if (/\.(woff2?|ttf|svg|png)$/.test(file)) {
        fs.copyFileSync(
          path.join(scratch, "assets", file),
          path.join(assets, file),
        );
      }
    }
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }
}

async function settle(page, url) {
  await page.goto(url);
  // The network can fall quiet before React has drawn anything, which would
  // capture an empty page; the data the first draw asks for comes after.
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

async function main() {
  const usage =
    "just proto-new <issue>-<slug> <path>...   as in: just proto-new 583-vaults-unavailable /";
  const args = process.argv.slice(2);
  const at = args.indexOf("--origin");
  const origin = at >= 0 ? args.splice(at, 2)[1] : undefined;
  const [name, ...routes] = args;
  if (
    !origin ||
    !/^\d+-[a-z0-9][a-z0-9-]*$/.test(name ?? "") ||
    routes.length === 0
  ) {
    throw new ToolError(`usage: ${usage}`);
  }
  const strays = routes.filter((route) => !route.startsWith("/"));
  if (strays.length) {
    throw new ToolError(
      `a path starts with a slash, as in /graph: ${strays.join(" ")}`,
    );
  }
  let envFile = "";
  try {
    envFile = fs.readFileSync(path.join(ROOT, ".env"), "utf8");
  } catch {
    // The environment may carry the token alone.
  }
  const token = tokenFrom(process.env, envFile);
  try {
    await fetch(origin);
  } catch {
    throw new ToolError(
      `the dev frontend did not answer at ${origin}; \`just dev-start\` starts it`,
    );
  }

  const folder = path.join(PROTOTYPES_DIR, name);
  const captured = path.join(folder, "captured");
  const media = path.join(folder, "media");
  fs.mkdirSync(captured, { recursive: true });
  fs.mkdirSync(media, { recursive: true });
  console.log("building the stylesheet the app ships...");
  buildStylesheet(folder);

  const browser = await playwright().chromium.launch({
    executablePath: chromePath(),
  });
  const errors = [];
  // Fetched afresh on every run, so a changed image is not left stale, and
  // once per run, however many pages show it.
  const fetched = new Set();
  try {
    for (const [size, [width, height]] of Object.entries(SIZES)) {
      for (const theme of THEMES) {
        // The service worker is kept out: it answers from its own cache, and
        // this exists to show what the code on disk draws today.
        // Phone and tablet are touch screens, which the app reads as
        // `(pointer: coarse)` to choose, for one, the editor's toolbar.
        const touch = size !== "desktop";
        const context = await browser.newContext({
          viewport: { width, height },
          hasTouch: touch,
          isMobile: touch,
          serviceWorkers: "block",
          reducedMotion: "reduce",
          colorScheme: theme,
          extraHTTPHeaders: token ? { Authorization: `Bearer ${token}` } : {},
        });
        await context.addInitScript(
          (stored) => {
            for (const [key, value] of stored) {
              localStorage.setItem(key, value);
            }
          },
          [
            ...(token ? [[TOKEN_KEY, token]] : []),
            [THEME_KEY, theme],
            [SEEN_VERSION_KEY, SEEN_EVERYTHING],
          ],
        );
        const page = await context.newPage();
        page.on("pageerror", (error) =>
          errors.push(`${page.url()} at ${size}, ${theme}: ${error}`),
        );
        for (const route of routes) {
          await settle(page, origin + route);
          const got = await page.evaluate(capture, theme);
          for (const src of got.media) {
            const file = path.join(media, mediaName(src));
            if (fetched.has(file)) {
              continue;
            }
            fetched.add(file);
            const answer = await context.request.get(origin + src);
            if (!answer.ok()) {
              errors.push(
                `${route} at ${size}: an image did not come (${answer.status()})`,
              );
              continue;
            }
            fs.writeFileSync(file, await answer.body());
          }
          const body = got.body.replaceAll(
            MEDIA_MARK,
            (_, src) => `/media/${mediaName(decodeURIComponent(src))}`,
          );
          const htmlAttributes = got.attributes
            .map(([key, value]) => ` ${key}="${escaped(value)}"`)
            .join("");
          const file = path.join(
            captured,
            `${slugOf(route)}.${size}.${theme}.html`,
          );
          fs.writeFileSync(
            file,
            withoutToken(pageOf({ ...got, htmlAttributes, body }), token),
          );
          console.log(path.relative(ROOT, file));
        }
        await context.close();
      }
    }
  } finally {
    await browser.close();
  }

  const pages = fs
    .readdirSync(captured)
    .filter((file) => file.endsWith(".html"));
  fs.writeFileSync(path.join(folder, "index.html"), indexOf(name, pages));
  console.log(
    `${path.relative(ROOT, folder)}: ${pages.length} pages, index.html lists them`,
  );
  if (errors.length) {
    throw new ToolError(`the capture is not whole:\n${errors.join("\n")}`);
  }
}

if (process.argv[1] === import.meta.filename) {
  await run(main);
}
