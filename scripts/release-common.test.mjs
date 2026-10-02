import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  CHECKLIST_ITEMS,
  DOCS_FRESHNESS_ITEM,
  compareVersions,
  hasEntries,
  latestReleaseVersion,
  parseChecklist,
  parseVersion,
  readVersions,
  releaseNotes,
  releaseSection,
  renameUnreleased,
  renderChecklist,
  setVersions,
  undefinedReferences,
  unreleasedSection,
  versionDisagreements,
} from "./release-common.mjs";

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

test("accepts only bare semantic versions", () => {
  assert.deepEqual(parseVersion("2.7.0"), [2, 7, 0]);
  assert.deepEqual(parseVersion("10.0.12"), [10, 0, 12]);
  for (const invalid of [
    "v2.7.0",
    "2.7",
    "2.7.0-rc.1",
    "2.07.0",
    "2.7.0+b",
    "",
  ]) {
    assert.equal(parseVersion(invalid), null, invalid);
  }
});

test("compares versions numerically, not as text", () => {
  assert.equal(compareVersions("2.10.0", "2.9.9"), 1);
  assert.equal(compareVersions("2.6.1", "2.6.1"), 0);
  assert.equal(compareVersions("1.99.99", "2.0.0"), -1);
});

test("the latest release ignores tags that are not v<version>", () => {
  assert.equal(
    latestReleaseVersion([
      "v2.6.0",
      "v2.10.0",
      "v2.9.1",
      "podman-v3.0.0",
      "3.0.0",
      "v4.0",
    ]),
    "2.10.0",
  );
  assert.equal(latestReleaseVersion([]), null);
});

const CHANGELOG = `# Changelog

## Unreleased

### Added
- A new thing. [#12]

### Fixed
- An old thing, see [#3] and [#9]. [#14]

[#12]: https://example.test/issues/12
[#14]: https://example.test/issues/14

## v1.0.0 - 2026-01-01

### Added
- The first thing. [#3]

[#3]: https://example.test/issues/3
`;

test("finds the Unreleased section up to the next release heading", () => {
  const section = unreleasedSection(CHANGELOG);
  assert.match(section.body, /A new thing/);
  assert.match(section.body, /### Fixed/);
  assert.doesNotMatch(section.body, /The first thing/);
  assert.equal(unreleasedSection("# Changelog\n\n## v1.0.0 - x\n"), null);
});

test("a section needs a list entry, not just headings or prose", () => {
  assert.equal(hasEntries(unreleasedSection(CHANGELOG).body), true);
  assert.equal(
    hasEntries("\n### Added\n\nSome intro prose.\n\n[#1]: https://x\n"),
    false,
  );
});

test("renames Unreleased to the dated release heading", () => {
  const renamed = renameUnreleased(CHANGELOG, "1.1.0", "2026-10-02");
  assert.match(renamed, /^## v1\.1\.0 - 2026-10-02$/m);
  assert.doesNotMatch(renamed, /## Unreleased/);
  assert.equal(
    renamed.length,
    CHANGELOG.length + "v1.1.0 - 2026-10-02".length - "Unreleased".length,
  );
  assert.match(releaseSection(renamed, "1.1.0").body, /A new thing/);
  assert.throws(
    () => renameUnreleased(renamed, "1.2.0", "2026-10-02"),
    /no "## Unreleased"/,
  );
});

test("names references with no link definition anywhere in the file", () => {
  const body = unreleasedSection(CHANGELOG).body;
  // #3 is defined under v1.0.0, which still counts; #9 is defined nowhere.
  assert.deepEqual(undefinedReferences(CHANGELOG, body), ["#9"]);
});

test("release notes drop the heading and carry definitions from elsewhere", () => {
  const renamed = renameUnreleased(CHANGELOG, "1.1.0", "2026-10-02");
  const notes = releaseNotes(renamed, "1.1.0");
  assert.doesNotMatch(notes, /^## /m);
  assert.match(notes, /^### Added\n- A new thing\. \[#12\]/);
  assert.match(
    notes,
    /\[#14\]: https:\/\/example\.test\/issues\/14\n\n\[#3\]: https:\/\/example\.test\/issues\/3\n$/,
  );
  assert.equal((notes.match(/^\[#12\]:/gm) ?? []).length, 1);
  assert.throws(() => releaseNotes(renamed, "9.9.9"), /no "## v9\.9\.9 - "/);
});

async function realVersionFiles() {
  const files = {};
  for (const file of [
    "Cargo.toml",
    "Cargo.lock",
    "frontend/package.json",
    "frontend/package-lock.json",
  ]) {
    files[file] = await readFile(path.join(repositoryRoot, file), "utf8");
  }
  return files;
}

test("sets the version in the real files without moving anything else", async () => {
  const files = await realVersionFiles();
  const current = readVersions(files)["Cargo.toml"];
  assert.deepEqual(versionDisagreements(files, current), []);

  const bumped = setVersions(files, "99.0.0");
  assert.deepEqual(versionDisagreements(bumped, "99.0.0"), []);
  for (const [file, before] of Object.entries(files)) {
    const after = bumped[file].split("\n");
    const changed = before
      .split("\n")
      .filter((line, index) => line !== after[index]);
    const expected = file === "frontend/package-lock.json" ? 2 : 1;
    assert.equal(
      changed.length,
      expected,
      `${file} changed ${changed.length} lines`,
    );
    assert.equal(
      bumped[file].split("\n").length,
      before.split("\n").length,
      file,
    );
  }
  // A dependency that happens to share the old version is left alone.
  assert.equal(setVersions(bumped, current)["Cargo.lock"], files["Cargo.lock"]);
});

test("reports each file whose version disagrees", async () => {
  const files = setVersions(await realVersionFiles(), "3.0.0");
  files["frontend/package.json"] = files["frontend/package.json"].replace(
    '"version": "3.0.0"',
    '"version": "2.9.0"',
  );
  const lock = JSON.parse(files["frontend/package-lock.json"]);
  lock.packages[""].version = "2.9.1";
  files["frontend/package-lock.json"] = JSON.stringify(lock, null, 2);
  assert.deepEqual(versionDisagreements(files, "3.0.0"), [
    "frontend/package.json (2.9.0)",
    'frontend/package-lock.json packages[""] (2.9.1)',
  ]);
});

test("a version field inside another Cargo.toml table is not the package version", () => {
  const files = {
    "Cargo.toml":
      '[package]\nname = "app"\nkeywords = ["a", "b"]\nversion = "1.0.0"\n\n[dependencies]\nversion = "7"\n',
    "Cargo.lock": '[[package]]\nname = "app"\nversion = "1.0.0"\n',
    "frontend/package.json": '{\n  "version": "1.0.0"\n}\n',
    "frontend/package-lock.json":
      '{\n  "version": "1.0.0",\n  "packages": {\n    "": {\n      "version": "1.0.0"\n    }\n  }\n}\n',
  };
  const bumped = setVersions(files, "1.1.0");
  assert.match(bumped["Cargo.toml"], /^version = "1\.1\.0"$/m);
  assert.match(bumped["Cargo.toml"], /^version = "7"$/m);
});

test("renders every item unticked, with the pasted output under the docs item", () => {
  const output =
    "Documentation freshness review required.\n```\nodd\n```\n- [ ] not a box";
  const body = renderChecklist(output);
  for (const item of CHECKLIST_ITEMS) {
    assert.ok(body.includes(`- [ ] ${item}`), item);
  }
  const docsLine = body.indexOf(DOCS_FRESHNESS_ITEM);
  const pasted = body.indexOf("  Documentation freshness review required.");
  assert.ok(pasted > docsLine);
  assert.ok(
    pasted <
      body.indexOf(
        CHECKLIST_ITEMS[CHECKLIST_ITEMS.indexOf(DOCS_FRESHNESS_ITEM) + 1],
      ),
  );
  assert.match(body, /^ {2}````text$/m);
});

test("parses a freshly rendered checklist as all unticked", () => {
  const parsed = parseChecklist(`Intro.\n\n${renderChecklist("- [x] fake")}\n`);
  assert.deepEqual(parsed.missing, []);
  assert.equal(parsed.items.length, CHECKLIST_ITEMS.length);
  assert.ok(parsed.items.every((item) => item.checked === false));
});

test("reads ticks, and reports edited or deleted items as missing", () => {
  let body = renderChecklist("output").replace(
    `- [ ] ${CHECKLIST_ITEMS[0]}`,
    `- [x] ${CHECKLIST_ITEMS[0]}`,
  );
  body = body.replace(
    `- [ ] ${CHECKLIST_ITEMS[1]}`,
    `- [X] ${CHECKLIST_ITEMS[1]}`,
  );
  body = body.replace(
    `- [ ] ${CHECKLIST_ITEMS[2]}`,
    "- [x] The roadmap is fine, probably.",
  );
  body = body.replace(`- [ ] ${CHECKLIST_ITEMS[6]}`, "");
  const parsed = parseChecklist(body.replace(/\n/g, "\r\n"));
  assert.deepEqual(parsed.missing, [CHECKLIST_ITEMS[2], CHECKLIST_ITEMS[6]]);
  assert.deepEqual(
    parsed.items.map((item) => item.checked),
    [true, true, false, false, false],
  );
});

test("a ticked box inside pasted output does not tick the real item", () => {
  // The pasted output sits above the last item, so a fake box in it would be
  // the first match if fenced lines were read.
  const last = CHECKLIST_ITEMS.at(-1);
  const parsed = parseChecklist(renderChecklist(`- [x] ${last}`));
  assert.equal(parsed.items.find((item) => item.text === last).checked, false);
});
