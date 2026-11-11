import assert from "node:assert/strict";
import fs from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";

import {
  chromePath,
  contextOptions,
  playwright,
  signedInBrowser,
  storedState,
} from "./dev-browser.mjs";
import { FAKE_TOKEN, freePort, runNode, serve } from "./dev-test-support.mjs";
import { ToolError } from "./proto-common.mjs";

// Launching needs a Chrome and `npm ci` in frontend/; without either the
// launching tests are skipped and say why.
function cannotLaunch() {
  try {
    chromePath();
    playwright();
    return false;
  } catch (error) {
    return error.message;
  }
}

// A stand-in for the dev frontend: a page that draws what the browser had
// stored when it loaded, a moment late as the app does, and then remembers a
// Reading view the way a real run leaves one behind.
const PAGE = `<!doctype html><html><head><title>fake app</title>
<meta name="viewport" content="width=device-width, initial-scale=1"></head><body>
<div id="root"></div>
<script>
  const had = JSON.stringify(Object.fromEntries(Object.entries(localStorage)));
  localStorage.setItem("hatchdoor.noteReadingView", "1");
  setTimeout(() => {
    const main = document.createElement("main");
    main.textContent = had + " width=" + innerWidth +
      " dark=" + matchMedia("(prefers-color-scheme: dark)").matches;
    document.getElementById("root").append(main);
  }, 150);
</script></body></html>`;

function fakeFrontend() {
  const bearers = new Set();
  return serve((request, response) => {
    bearers.add(request.headers.authorization);
    response.writeHead(200, { "content-type": "text/html" }).end(PAGE);
  }).then((server) => ({ ...server, bearers }));
}

test("a fresh browser holds the sign-in, the theme and What's new as seen, and no view", () => {
  const stored = Object.fromEntries(storedState({ token: "t", theme: "dark" }));
  assert.deepEqual(stored, {
    hatchdoor_web_token: "t",
    "hatchdoor.theme": "dark",
    hatchdoor_whats_new_seen: "999999.0.0",
  });
  assert.equal("hatchdoor_web_token" in Object.fromEntries(storedState({ token: "", theme: "light" })), false);
});

test("a size is one proto-new captures at, touch on the small ones", () => {
  const phone = contextOptions({ size: "phone", theme: "dark", token: "t" });
  assert.deepEqual(phone.viewport, { width: 390, height: 844 });
  assert.equal(phone.hasTouch, true);
  assert.equal(phone.serviceWorkers, "block");
  assert.equal(phone.colorScheme, "dark");
  const desktop = contextOptions({ size: "desktop", theme: "light", token: "" });
  assert.deepEqual(desktop.viewport, { width: 1440, height: 900 });
  assert.equal(desktop.hasTouch, false);
  assert.deepEqual(desktop.extraHTTPHeaders, {});
});

test("an unknown size or theme, or a frontend that is down, is refused", async () => {
  await assert.rejects(signedInBrowser({ size: "watch" }), ToolError);
  await assert.rejects(signedInBrowser({ theme: "sepia" }), ToolError);
  await assert.rejects(
    signedInBrowser({ frontendPort: await freePort(), token: FAKE_TOKEN }),
    (error) =>
      error instanceof ToolError &&
      /did not answer.*just dev-start/.test(error.message) &&
      !error.message.includes(FAKE_TOKEN),
  );
});

test(
  "the page comes back drawn and signed in, with nothing left from the run before",
  { skip: cannotLaunch() },
  async () => {
    const frontend = await fakeFrontend();
    try {
      for (const run of ["first", "second"]) {
        const app = await signedInBrowser({
          size: "phone",
          theme: "dark",
          frontendPort: frontend.port,
          token: FAKE_TOKEN,
        });
        try {
          const page = await app.open("/v/some-vault/n/some-note");
          // Read at once: `open` returned only after the late draw.
          const text = await page.textContent("#root main");
          const had = JSON.parse(text.slice(0, text.indexOf(" width=")));
          assert.deepEqual(
            had,
            {
              "hatchdoor-dev-browser": "1",
              hatchdoor_web_token: FAKE_TOKEN,
              "hatchdoor.theme": "dark",
              hatchdoor_whats_new_seen: "999999.0.0",
            },
            `${run} run`,
          );
          assert.match(text, / width=390 dark=true$/);
          assert.doesNotMatch(app.clean(text), new RegExp(FAKE_TOKEN));

          // What the check itself changes outlives a reload in the same run.
          await page.evaluate(() => localStorage.setItem("hatchdoor.theme", "light"));
          await app.open("/");
          assert.match(await page.textContent("#root main"), /"hatchdoor.theme":"light"/);
          assert.match(await page.textContent("#root main"), /"hatchdoor.noteReadingView":"1"/);
          // In a second tab of the same browser as well.
          const tab = await app.context.newPage();
          await tab.goto(app.origin);
          await tab.waitForSelector("#root main");
          assert.match(await tab.textContent("#root main"), /"hatchdoor.theme":"light"/);
        } finally {
          await app.close();
        }
      }
      assert.ok(frontend.bearers.has(`Bearer ${FAKE_TOKEN}`));
    } finally {
      await frontend.close();
    }
  },
);

test(
  "a script using the helper prints no token",
  { skip: cannotLaunch() },
  async () => {
    const frontend = await fakeFrontend();
    const folder = fs.mkdtempSync(path.join(tmpdir(), "dev-browser-test-"));
    const script = path.join(folder, "check.mjs");
    fs.writeFileSync(
      script,
      `import { signedInBrowser } from ${JSON.stringify(pathToFileURL(path.join(import.meta.dirname, "dev-browser.mjs")).href)};
const app = await signedInBrowser({ frontendPort: ${frontend.port} });
try {
  const page = await app.open("/");
  console.log(app.clean(await page.textContent("#root main")));
} finally {
  await app.close();
}
`,
    );
    try {
      const run = await runNode([script], {
        env: { HATCHDOOR_WEB_BEARER_TOKEN: FAKE_TOKEN },
      });
      assert.equal(run.status, 0, run.stderr);
      // The page did hold the token, or this would prove nothing.
      assert.match(run.stdout, /"hatchdoor_web_token":"\[token\]"/);
      assert.doesNotMatch(run.stdout + run.stderr, new RegExp(FAKE_TOKEN));
    } finally {
      await frontend.close();
      fs.rmSync(folder, { recursive: true, force: true });
    }
  },
);
