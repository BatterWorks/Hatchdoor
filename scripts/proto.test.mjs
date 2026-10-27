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
} from "./proto-new.mjs";
import { fileFor } from "./proto-serve.mjs";

test("a path becomes a file name, and the root is home", () => {
  assert.equal(slugOf("/"), "home");
  assert.equal(slugOf("/notes/a b?x=1"), "notes-a-b-x-1");
});

test("an image address becomes one file name under media/", () => {
  assert.equal(
    mediaName("/vault-assets/v1/Images/a b.png"),
    "vault-assets_v1_Images_a_b.png",
  );
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

    assert.equal(folderOf("583-x", prototypes), folder);
    assert.throws(() => folderOf("../583-x", prototypes), ToolError);
    assert.throws(() => folderOf("missing", prototypes), ToolError);

    assert.equal(fileFor(folder, "/"), path.join(folder, "index.html"));
    assert.equal(fileFor(folder, "/../secret"), undefined);
    assert.equal(fileFor(folder, "/%2e%2e/secret"), undefined);
    assert.equal(fileFor(folder, "/nothing.html"), undefined);
  } finally {
    fs.rmSync(prototypes, { recursive: true, force: true });
  }
});
