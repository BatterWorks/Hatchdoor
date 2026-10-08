// The release hook publishes these two files to Docker Hub as they are
// (release-runbook.md, "The release hook"). Docker Hub renders the overview
// away from this repository, so a relative address in it leads nowhere, and
// it rejects text over its size limits. This fails before a release does.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

const OVERVIEW = "docs/maintenance/docker-hub/overview.md";
const SHORT_DESCRIPTION = "docs/maintenance/docker-hub/short-description.txt";
const INSTALL_GUIDE =
  "docs/user-vault/01 Get started/Install Hatchdoor with Docker Compose.md";

// Docker Hub's limits for the two fields.
const SHORT_DESCRIPTION_MAX_CHARACTERS = 100;
const OVERVIEW_MAX_BYTES = 25_000;

async function read(file) {
  return readFile(path.join(repositoryRoot, file), "utf8");
}

// Prose only: a path inside a code block or a code span is not an address
// Docker Hub turns into a link.
function prose(markdown) {
  return markdown
    .replace(/^```.*\n[\s\S]*?^```$/gm, "")
    .replace(/`[^`\n]*`/g, "");
}

// The target of every Markdown link and image, every link reference
// definition, and every HTML `href` or `src`.
function addresses(markdown) {
  const text = prose(markdown);
  return [
    ...text.matchAll(/\]\(\s*<?([^)\s>]*)/g),
    ...text.matchAll(/^\s*\[[^\]]+\]:\s*<?(\S*?)>?\s*$/gm),
    ...text.matchAll(/\b(?:href|src)\s*=\s*["']?([^"'\s>]*)/gi),
  ].map(([, address]) => address);
}

// The host folders the quick-start Compose file mounts, in order.
function mounts(markdown) {
  const compose = /^```yaml\n([\s\S]*?)^```$/m.exec(markdown);
  assert.ok(compose, "a Compose file in a yaml code block");
  const volumes = /^ +volumes:\n((?: +- .*\n)+)/m.exec(compose[1]);
  assert.ok(volumes, "the Compose file has a volumes list");
  return volumes[1]
    .trim()
    .split("\n")
    .map((line) => line.trim());
}

test("the short description is one line within Docker Hub's limit", async () => {
  const description = await read(SHORT_DESCRIPTION);
  const lines = description.replace(/\n$/, "").split("\n");

  assert.equal(lines.length, 1, "one line, with at most a final newline");
  assert.notEqual(lines[0].trim(), "");
  assert.equal(lines[0], lines[0].trim(), "no leading or trailing space");
  assert.ok(
    [...lines[0]].length <= SHORT_DESCRIPTION_MAX_CHARACTERS,
    `${[...lines[0]].length} characters, limit ${SHORT_DESCRIPTION_MAX_CHARACTERS}`,
  );
});

test("the overview fits Docker Hub's size limit", async () => {
  const bytes = Buffer.byteLength(await read(OVERVIEW), "utf8");
  assert.ok(
    bytes <= OVERVIEW_MAX_BYTES,
    `${bytes} bytes, limit ${OVERVIEW_MAX_BYTES}`,
  );
});

test("every link and image in the overview is an absolute address", async () => {
  const found = addresses(await read(OVERVIEW));
  assert.notEqual(found.length, 0, "the overview links somewhere");
  assert.deepEqual(
    found.filter((address) => !/^https:\/\/[^/]/.test(address)),
    [],
    "relative addresses do not resolve on Docker Hub",
  );
});

test("the address finder sees every way Markdown can point somewhere", () => {
  assert.deepEqual(
    addresses(
      [
        "[guide](docs/guide.md) and ![shot](assets/shot.png)",
        "[ref]: ../README.md",
        '<a href="#tags">Tags</a> <img src=assets/logo.svg>',
        "`[not](a-link)` in a code span",
        "```yaml",
        "- [nor](this)",
        "```",
      ].join("\n"),
    ),
    [
      "docs/guide.md",
      "assets/shot.png",
      "../README.md",
      "#tags",
      "assets/logo.svg",
    ],
  );
});

test("the overview does not name the repository's old address", async () => {
  const overview = await read(OVERVIEW);
  // The repository moved to BatterWorks. The image kept its lower-case name,
  // battermanz/hatchdoor, which is why the first check minds the case.
  assert.doesNotMatch(overview, /BattermanZ\/Hatchdoor/);
  assert.doesNotMatch(overview, /github\.com\/battermanz\//i);
});

test("the overview's quick start mounts what the install guide mounts", async () => {
  assert.deepEqual(
    mounts(await read(OVERVIEW)),
    mounts(await read(INSTALL_GUIDE)),
  );
});

test("the overview sets no placeholder web token", async () => {
  const overview = await read(OVERVIEW);
  // The first start prints the token. A quick start that assigns one gets
  // copied as it stands.
  assert.doesNotMatch(overview, /change-?me/i);
  assert.doesNotMatch(overview, /HATCHDOOR_WEB_BEARER_TOKEN=[\w-]/);
});
