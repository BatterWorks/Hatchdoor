import assert from "node:assert/strict";
import fs from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";

import { ToolError, folderOf } from "./proto-common.mjs";
import {
  indexOf,
  mediaName,
  pageOf,
  slugOf,
  tokenFrom,
  withoutToken,
} from "./proto-new.mjs";
import { fileFor } from "./proto-serve.mjs";

test("a path becomes a file name, and the root is home", () => {
  assert.equal(slugOf("/"), "home");
  assert.equal(slugOf("/notes/a b?x=1"), "notes-a-b-x-1");
});

test("an image keeps its extension and loses its query in media/", () => {
  const name = mediaName(
    "/api/v1/vaults/v1/assets/Media/a b.png?access_token=secret",
  );
  assert.match(name, /^api_v1_vaults_v1_assets_Media_a_b-[0-9a-f]{8}\.png$/);
  assert.doesNotMatch(name, /secret|access_token/);
  assert.equal(name, mediaName("/api/v1/vaults/v1/assets/Media/a b.png"));
});

test("two images that make the same safe name stay two files", () => {
  assert.notEqual(mediaName("/a b.png"), mediaName("/a_b.png"));
});

test("a page loses every access_token, and one still holding it is refused", () => {
  assert.equal(
    withoutToken('<img src="/x.png?access_token=t0k&w=2">', "t0k"),
    '<img src="/x.png?w=2">',
  );
  assert.equal(
    withoutToken('<img src="/x.png?w=2&access_token=t0k">', "t0k"),
    '<img src="/x.png?w=2">',
  );
  assert.equal(
    withoutToken('<img src="/x.png?access_token=t0k">', "t0k"),
    '<img src="/x.png">',
  );
  assert.throws(() => withoutToken("<p>t0k</p>", "t0k"), ToolError);
  assert.equal(withoutToken("<p>plain</p>", ""), "<p>plain</p>");
});

test("the token comes from the environment first, then .env", () => {
  const env = "OTHER=1\nHATCHDOOR_WEB_BEARER_TOKEN='from-file'\n";
  assert.equal(
    tokenFrom({ HATCHDOOR_WEB_BEARER_TOKEN: " exported " }, env),
    "exported",
  );
  assert.equal(tokenFrom({}, env), "from-file");
  assert.equal(tokenFrom({}, ""), "");
});

test("a captured page links the shipped stylesheet and keeps its theme", () => {
  const page = pageOf({
    title: "Hatchdoor <dev>",
    htmlAttributes: ' lang="en" data-theme="dark"',
    styles: "",
    body: "<body><p>hi</p></body>",
  });
  assert.match(page, /<html lang="en" data-theme="dark">/);
  assert.match(page, /<link rel="stylesheet" href="\/assets\/app.css">/);
  assert.match(page, /<title>Hatchdoor &lt;dev>/);
  assert.doesNotMatch(page, /<script/);
});

test("the index lists every page by path, size then theme", () => {
  const index = indexOf("583-x", [
    "home.desktop.dark.html",
    "home.phone.dark.html",
    "home.phone.light.html",
  ]);
  const order = [...index.matchAll(/href="captured\/([^"]+)"/g)].map(
    (match) => match[1],
  );
  assert.deepEqual(order, [
    "home.phone.light.html",
    "home.phone.dark.html",
    "home.desktop.dark.html",
  ]);
  assert.match(index, /390 × 844/);
});

test("only a prototype folder is served, and nothing outside it", () => {
  const prototypes = fs.mkdtempSync(path.join(tmpdir(), "proto-test-"));
  try {
    const folder = path.join(prototypes, "583-x");
    fs.mkdirSync(path.join(folder, "captured"), { recursive: true });
    fs.writeFileSync(path.join(folder, "index.html"), "x");
    fs.writeFileSync(path.join(prototypes, "secret"), "x");
    fs.symlinkSync(path.join(prototypes, "secret"), path.join(folder, "link"));

    assert.equal(folderOf("583-x", prototypes), folder);
    assert.throws(() => folderOf("../583-x", prototypes), ToolError);
    assert.throws(() => folderOf("missing", prototypes), ToolError);
    assert.throws(() => folderOf("secret", prototypes), ToolError);

    assert.equal(
      fileFor(folder, "/"),
      fs.realpathSync(path.join(folder, "index.html")),
    );
    // `new URL` leaves an encoded slash alone, so this one reaches the
    // folder guard as `/../secret`, a file that exists.
    assert.equal(fileFor(folder, "/%2e%2e%2fsecret"), undefined);
    assert.equal(fileFor(folder, "/link"), undefined);
    assert.equal(fileFor(folder, "/nothing.html"), undefined);
  } finally {
    fs.rmSync(prototypes, { recursive: true, force: true });
  }
});
