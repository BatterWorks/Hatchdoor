import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, test } from "node:test";
import { fileURLToPath } from "node:url";

import { CHECKLIST_ITEMS, parseChecklist } from "./release-common.mjs";

const scriptsDirectory = path.dirname(fileURLToPath(import.meta.url));
const temporaryDirectories = [];
const BRANCH = "docs/release-v1.1.0";

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map((directory) => rm(directory, { recursive: true, force: true })),
  );
});

// Stand-ins for the GitHub CLI and `just`. The `gh` stub keeps pull requests
// and releases in a JSON file, answers in the shapes the real CLI does, and
// refuses to open a pull request from a branch the remote does not have.
const GH_STUB = `#!/usr/bin/env node
const fs = require("node:fs");
const { execFileSync } = require("node:child_process");
const file = process.env.STUB_STATE;
const state = JSON.parse(fs.readFileSync(file, "utf8"));
const args = process.argv.slice(2);
state.calls.push(["gh", ...args]);
const save = () => fs.writeFileSync(file, JSON.stringify(state, null, 2));
const option = (name) => { const i = args.indexOf(name); return i === -1 ? undefined : args[i + 1]; };
const [noun, verb] = args;
if (noun === "pr" && verb === "list") {
  const wanted = option("--state");
  const pulls = state.pulls.filter((pull) =>
    pull.head === option("--head") && pull.base === option("--base") &&
    (wanted === "all" || pull.state === wanted.toUpperCase()));
  save();
  console.log(JSON.stringify(pulls.map(({ number, url, state, title }) => ({ number, url, state, title }))));
} else if (noun === "pr" && verb === "create") {
  if (process.env.STUB_FAIL_PR_CREATE) { save(); console.error("HTTP 502"); process.exit(1); }
  const head = option("--head");
  const remote = execFileSync("git", ["ls-remote", "--heads", "origin", head], { encoding: "utf8" });
  if (!remote.trim()) { save(); console.error("head branch not pushed: " + head); process.exit(1); }
  const number = state.pulls.length + 1;
  const url = "https://github.test/pull/" + number;
  state.pulls.push({ number, url, state: "OPEN", base: option("--base"), head, title: option("--title"), body: option("--body") });
  save();
  console.log(url);
} else if (noun === "release" && verb === "view") {
  const release = state.releases.find((candidate) => candidate.tag === args[2]);
  save();
  if (!release) { console.error("release not found"); process.exit(1); }
  console.log(JSON.stringify({ isDraft: release.isDraft, url: release.url }));
} else if (noun === "release" && verb === "create") {
  const url = "https://github.test/releases/untagged-" + (state.releases.length + 1);
  state.releases.push({ tag: args[2], isDraft: args.includes("--draft"), target: option("--target"), title: option("--title"), notes: option("--notes"), url });
  save();
  console.log(url);
} else {
  save();
  console.error("gh stub: unexpected " + args.join(" "));
  process.exit(3);
}
`;

const JUST_STUB = `#!/usr/bin/env node
const fs = require("node:fs");
const { execFileSync } = require("node:child_process");
const file = process.env.STUB_STATE;
const state = JSON.parse(fs.readFileSync(file, "utf8"));
const args = process.argv.slice(2);
const head = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
state.calls.push(["just", ...args, head]);
fs.writeFileSync(file, JSON.stringify(state, null, 2));
if (args[0] === "check-full") {
  console.log("stub check-full");
  process.exit(Number(process.env.STUB_CHECK_FULL_EXIT ?? 0));
}
if (args[0] === "docs-freshness" && args[1] === "main") {
  if (process.env.STUB_FRESHNESS_CHANGELOG_ONLY) {
    console.error("Documentation freshness check failed: no user-facing surface changed since main, but the changelog needs an entry.");
    process.exit(1);
  }
  console.error("Documentation freshness review required: 1 user-facing surface(s) changed since main.");
  console.error("  docs/user-vault/Some note.md  (UNTOUCHED)");
  process.exit(Number(process.env.STUB_FRESHNESS_EXIT ?? 1));
}
console.error("just stub: unexpected " + args.join(" "));
process.exit(3);
`;

const CARGO_TOML = `[package]
name = "hatchdoor"
version = "1.0.0"
edition = "2024"
keywords = ["markdown"]

[dependencies]
serde = { version = "1.0.0" }
`;

const CARGO_LOCK = `version = 4

[[package]]
name = "hatchdoor"
version = "1.0.0"
dependencies = [
 "serde",
]

[[package]]
name = "serde"
version = "1.0.0"
`;

const PACKAGE_JSON = `${JSON.stringify({ name: "frontend", private: true, version: "1.0.0", dependencies: { left: "1.0.0" } }, null, 2)}\n`;

const PACKAGE_LOCK = `${JSON.stringify(
  {
    name: "frontend",
    version: "1.0.0",
    lockfileVersion: 3,
    requires: true,
    packages: {
      "": { name: "frontend", version: "1.0.0" },
      "node_modules/left": { version: "1.0.0" },
    },
  },
  null,
  2,
)}\n`;

const CHANGELOG = `# Changelog

## Unreleased

### Added
- A new thing. [#12]

### Fixed
- An old thing. [#3]

[#12]: https://github.test/issues/12

## v1.0.0 - 2026-01-01

### Added
- The first thing. [#3]

[#3]: https://github.test/issues/3
`;

function git(root, ...args) {
  const result = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  assert.equal(
    result.status,
    0,
    `git ${args.join(" ")} failed: ${result.stderr}`,
  );
  return result.stdout.trim();
}

async function write(root, file, contents) {
  const absolute = path.join(root, file);
  await mkdir(path.dirname(absolute), { recursive: true });
  await writeFile(absolute, contents);
}

const GIT_IDENTITY = {
  GIT_AUTHOR_NAME: "Release Test",
  GIT_AUTHOR_EMAIL: "release@test.invalid",
  GIT_COMMITTER_NAME: "Release Test",
  GIT_COMMITTER_EMAIL: "release@test.invalid",
};

// A clone whose `origin` is a local bare repository holding `main` and
// `development` and a v1.0.0 tag, with the checkout on `development`.
async function fixture({ changelog = CHANGELOG, cargoLock = CARGO_LOCK } = {}) {
  const base = await mkdtemp(path.join(tmpdir(), "hatchdoor-release-prepare-"));
  temporaryDirectories.push(base);
  const origin = path.join(base, "origin.git");
  const root = path.join(base, "work");
  const bin = path.join(base, "bin");
  const state = path.join(base, "state.json");

  spawnSync("git", [
    "init",
    "--quiet",
    "--bare",
    "--initial-branch=main",
    origin,
  ]);
  spawnSync("git", ["init", "--quiet", "--initial-branch=main", root]);
  git(root, "remote", "add", "origin", origin);
  git(root, "config", "user.name", GIT_IDENTITY.GIT_AUTHOR_NAME);
  git(root, "config", "user.email", GIT_IDENTITY.GIT_AUTHOR_EMAIL);

  await write(root, "Cargo.toml", CARGO_TOML);
  await write(root, "Cargo.lock", cargoLock);
  await write(root, "frontend/package.json", PACKAGE_JSON);
  await write(root, "frontend/package-lock.json", PACKAGE_LOCK);
  await write(root, "CHANGELOG.md", changelog);
  for (const script of ["release-prepare.mjs", "release-common.mjs"]) {
    await mkdir(path.join(root, "scripts"), { recursive: true });
    await copyFile(
      path.join(scriptsDirectory, script),
      path.join(root, "scripts", script),
    );
  }
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "v1.0.0");
  git(root, "tag", "v1.0.0");
  git(root, "switch", "--quiet", "--create", "development");
  git(root, "push", "--quiet", "origin", "main", "development", "--tags");

  await write(bin, "gh", GH_STUB);
  await write(bin, "just", JUST_STUB);
  await chmod(path.join(bin, "gh"), 0o755);
  await chmod(path.join(bin, "just"), 0o755);
  await writeFile(
    state,
    JSON.stringify({ pulls: [], releases: [], calls: [] }),
  );

  return { root, origin, bin, state };
}

function prepare(context, args = ["1.1.0"], env = {}) {
  return spawnSync(process.execPath, ["scripts/release-prepare.mjs", ...args], {
    cwd: context.root,
    encoding: "utf8",
    env: {
      ...process.env,
      ...GIT_IDENTITY,
      PATH: `${context.bin}${path.delimiter}${process.env.PATH}`,
      STUB_STATE: context.state,
      ...env,
    },
  });
}

async function stubState(context) {
  return JSON.parse(await readFile(context.state, "utf8"));
}

async function calls(context, tool, verb) {
  return (await stubState(context)).calls.filter(
    (call) => call[0] === tool && call[1] === verb,
  );
}

function assertRefused(result, check) {
  assert.equal(result.status, 1, result.stdout + result.stderr);
  assert.match(
    result.stderr,
    new RegExp(`release-prepare refused \\(${check}\\)`),
  );
}

// Merges the bump the way GitHub would, as far as this script can tell: the
// branch lands on `development` and the pull request reads MERGED.
async function mergeBump(context) {
  git(context.root, "fetch", "--quiet", "origin");
  git(
    context.root,
    "push",
    "--quiet",
    "origin",
    `origin/${BRANCH}:refs/heads/development`,
  );
  const state = await stubState(context);
  state.pulls.find((pull) => pull.head === BRANCH).state = "MERGED";
  await writeFile(context.state, JSON.stringify(state));
}

function today() {
  const now = new Date();
  const pad = (value) => String(value).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

test("opens the bump, then the release pull request and draft release, without duplicating any", async () => {
  const context = await fixture();

  const first = prepare(context);
  assert.equal(first.status, 0, first.stderr);
  assert.equal(git(context.root, "branch", "--show-current"), BRANCH);
  assert.equal(
    git(context.root, "rev-parse", "HEAD"),
    git(context.root, "rev-parse", `origin/${BRANCH}`),
  );
  assert.equal(
    git(context.root, "rev-list", "--count", "origin/development..HEAD"),
    "1",
  );
  assert.equal(
    git(context.root, "log", "-1", "--format=%s"),
    "docs: prepare the v1.1.0 release",
  );

  const diff = git(
    context.root,
    "diff",
    "--unified=0",
    "origin/development",
    "HEAD",
  );
  const removed = diff.split("\n").filter((line) => /^-[^-]/.test(line));
  const added = diff.split("\n").filter((line) => /^\+[^+]/.test(line));
  assert.deepEqual(removed, [
    "-## Unreleased",
    '-version = "1.0.0"',
    '-version = "1.0.0"',
    // package-lock.json sorts first and declares the version twice.
    '-  "version": "1.0.0",',
    '-      "version": "1.0.0"',
    '-  "version": "1.0.0",',
  ]);
  assert.deepEqual(added, [
    `+## v1.1.0 - ${today()}`,
    '+version = "1.1.0"',
    '+version = "1.1.0"',
    // package-lock.json sorts first and declares the version twice.
    '+  "version": "1.1.0",',
    '+      "version": "1.1.0"',
    '+  "version": "1.1.0",',
  ]);

  let state = await stubState(context);
  assert.equal(state.pulls.length, 1);
  assert.equal(state.pulls[0].base, "development");
  assert.equal(state.pulls[0].head, BRANCH);
  assert.doesNotMatch(state.pulls[0].body, /close/i);
  assert.equal((await calls(context, "just", "check-full")).length, 1);

  const waiting = prepare(context);
  assert.equal(waiting.status, 0, waiting.stderr);
  assert.match(
    waiting.stdout,
    /waiting to be merged: https:\/\/github\.test\/pull\/1/,
  );
  assert.equal((await stubState(context)).pulls.length, 1);
  assert.equal((await calls(context, "just", "check-full")).length, 1);

  await mergeBump(context);
  // The local main falls behind origin, as it does between releases.
  git(
    context.root,
    "push",
    "--quiet",
    "origin",
    "origin/development:refs/heads/main",
  );
  assert.notEqual(
    git(context.root, "rev-parse", "main"),
    git(context.root, "rev-parse", "origin/development"),
  );
  const released = prepare(context);
  assert.equal(released.status, 0, released.stderr);
  assert.equal(
    git(context.root, "branch", "--show-current"),
    BRANCH,
    "returns to the starting branch",
  );
  assert.equal(
    git(context.root, "rev-parse", "main"),
    git(context.root, "rev-parse", "origin/main"),
    "fast-forwards the local main first",
  );

  state = await stubState(context);
  assert.equal(state.pulls.length, 2);
  const release = state.pulls[1];
  assert.equal(release.base, "main");
  assert.equal(release.head, "development");
  assert.doesNotMatch(release.body, /close/i);
  const checklist = parseChecklist(release.body);
  assert.deepEqual(checklist.missing, []);
  assert.equal(checklist.items.length, CHECKLIST_ITEMS.length);
  assert.ok(checklist.items.every((item) => !item.checked));
  assert.match(
    release.body,
    /^ {4}docs\/user-vault\/Some note\.md {2}\(UNTOUCHED\)$/m,
  );

  const freshness = await calls(context, "just", "docs-freshness");
  assert.equal(freshness.length, 1);
  assert.equal(
    freshness[0].at(-1),
    git(context.root, "rev-parse", "origin/development"),
    "reviews the tip of development",
  );

  assert.equal(state.releases.length, 1);
  assert.deepEqual(
    {
      tag: state.releases[0].tag,
      isDraft: state.releases[0].isDraft,
      title: state.releases[0].title,
      target: state.releases[0].target,
    },
    { tag: "v1.1.0", isDraft: true, title: "v1.1.0", target: "main" },
  );
  assert.equal(
    state.releases[0].notes,
    "### Added\n- A new thing. [#12]\n\n### Fixed\n- An old thing. [#3]\n\n[#12]: https://github.test/issues/12\n\n[#3]: https://github.test/issues/3\n",
  );

  const again = prepare(context);
  assert.equal(again.status, 0, again.stderr);
  assert.match(again.stdout, /already open: https:\/\/github\.test\/pull\/2/);
  assert.match(again.stdout, /draft release already exists/);
  state = await stubState(context);
  assert.equal(state.pulls.length, 2);
  assert.equal(state.releases.length, 1);
  assert.equal((await calls(context, "just", "docs-freshness")).length, 1);
  assert.equal(
    (await calls(context, "gh", "pr")).filter((call) => call[2] === "merge")
      .length,
    0,
  );
});

test("refuses a dirty working tree", async () => {
  const context = await fixture();
  await write(context.root, "stray.txt", "x\n");
  assertRefused(prepare(context), "clean working tree");
  assert.match(prepare(context).stderr, /stray\.txt/);
});

test("refuses a version that is not bare or not higher than the last tag", async () => {
  const context = await fixture();
  for (const version of ["v1.1.0", "1.1", "1.1.0-rc.1"]) {
    const result = prepare(context, [version]);
    assertRefused(result, "version");
    assert.match(result.stderr, /not a bare semantic version/);
  }
  for (const version of ["1.0.0", "0.9.9"]) {
    const result = prepare(context, [version]);
    assertRefused(result, "version");
    assert.match(
      result.stderr,
      /not higher than the latest release tag, v1\.0\.0/,
    );
  }
  assert.equal((await stubState(context)).calls.length, 0);
});

test("refuses a version that is already published", async () => {
  const context = await fixture();
  const state = await stubState(context);
  state.releases.push({
    tag: "v1.1.0",
    isDraft: false,
    url: "https://github.test/releases/tag/v1.1.0",
  });
  await writeFile(context.state, JSON.stringify(state));
  const result = prepare(context);
  assertRefused(result, "version");
  assert.match(result.stderr, /already published/);
});

test("refuses when Unreleased has no entries on development", async () => {
  const context = await fixture({
    changelog:
      "# Changelog\n\n## Unreleased\n\n### Added\n\n## v1.0.0 - 2026-01-01\n\n- First. [#3]\n\n[#3]: https://x\n",
  });
  const result = prepare(context);
  assertRefused(result, "changelog");
  assert.match(result.stderr, /has no entries, so there is nothing to release/);
  assert.equal(git(context.root, "branch", "--list", BRANCH), "");
});

test("refuses when a version file cannot be set", async () => {
  const context = await fixture({
    cargoLock: CARGO_LOCK.replace('name = "hatchdoor"', 'name = "renamed"'),
  });
  const result = prepare(context);
  assertRefused(result, "version files");
  assert.match(result.stderr, /Cargo\.lock/);
  assert.equal((await stubState(context)).pulls.length, 0);
});

test("refuses a section whose references lack a link definition, before running the tests", async () => {
  const context = await fixture({
    changelog: CHANGELOG.replace(
      "An old thing. [#3]",
      "An old thing. [#3] [#99]",
    ),
  });
  const result = prepare(context);
  assertRefused(result, "changelog links");
  assert.match(result.stderr, /references #99 with no link definition/);
  assert.equal((await calls(context, "just", "check-full")).length, 0);
  assert.equal(git(context.root, "ls-remote", "--heads", "origin", BRANCH), "");
});

test("refuses a resumed bump whose release section was emptied", async () => {
  const context = await fixture();
  assert.equal(
    prepare(context, ["1.1.0"], { STUB_CHECK_FULL_EXIT: "1" }).status,
    1,
  );
  const file = path.join(context.root, "CHANGELOG.md");
  await writeFile(
    file,
    (await readFile(file, "utf8")).replace(
      /^- A new thing.*\n|^- An old thing.*\n/gm,
      "",
    ),
  );
  git(context.root, "commit", "--quiet", "-am", "emptied by hand");
  const result = prepare(context);
  assertRefused(result, "changelog");
  assert.match(
    result.stderr,
    /"## v1\.1\.0 - " section of CHANGELOG\.md has no entries/,
  );
});

test("a failed check-full stops before the push, and the re-run resumes without a second bump commit", async () => {
  const context = await fixture();
  const failed = prepare(context, ["1.1.0"], { STUB_CHECK_FULL_EXIT: "1" });
  assertRefused(failed, "just check-full");
  assert.match(failed.stderr, /exited with status 1/);
  assert.equal(git(context.root, "ls-remote", "--heads", "origin", BRANCH), "");
  assert.equal((await stubState(context)).pulls.length, 0);

  git(context.root, "switch", "--quiet", "development");
  const resumed = prepare(context);
  assert.equal(resumed.status, 0, resumed.stderr);
  assert.match(resumed.stdout, /Resuming on docs\/release-v1\.1\.0/);
  assert.equal(
    git(context.root, "rev-list", "--count", "origin/development..HEAD"),
    "1",
  );
  assert.equal((await stubState(context)).pulls.length, 1);
  assert.equal((await calls(context, "just", "check-full")).length, 2);
});

test("a run that pushed but failed to open the pull request opens it on the re-run", async () => {
  const context = await fixture();
  const failed = prepare(context, ["1.1.0"], { STUB_FAIL_PR_CREATE: "1" });
  assert.notEqual(failed.status, 0);
  assert.notEqual(
    git(context.root, "ls-remote", "--heads", "origin", BRANCH),
    "",
  );

  // A fresh clone has no local branch, so it resumes from the remote one.
  git(context.root, "switch", "--quiet", "development");
  git(context.root, "branch", "--quiet", "-D", BRANCH);
  const resumed = prepare(context);
  assert.equal(resumed.status, 0, resumed.stderr);
  assert.match(resumed.stdout, /checked out from origin/);
  assert.match(resumed.stdout, /already pushed at this commit/);
  assert.equal(
    git(context.root, "rev-list", "--count", "origin/development..HEAD"),
    "1",
  );
  assert.equal((await stubState(context)).pulls.length, 1);
  assert.equal((await calls(context, "just", "check-full")).length, 1);
});

test("a closed bump pull request blocks until its branch is deleted", async () => {
  const context = await fixture();
  assert.equal(prepare(context).status, 0);
  const state = await stubState(context);
  state.pulls[0].state = "CLOSED";
  await writeFile(context.state, JSON.stringify(state));

  const blocked = prepare(context);
  assertRefused(blocked, "bump pull request");
  assert.match(blocked.stderr, /closed without merging/);

  git(context.root, "switch", "--quiet", "development");
  git(context.root, "branch", "--quiet", "-D", BRANCH);
  git(context.root, "push", "--quiet", "origin", "--delete", BRANCH);
  const restarted = prepare(context);
  assert.equal(restarted.status, 0, restarted.stderr);
  assert.equal((await stubState(context)).pulls.length, 2);
});

test("a failing docs-freshness run refuses and opens nothing", async () => {
  const context = await fixture();
  assert.equal(prepare(context).status, 0);
  await mergeBump(context);
  const result = prepare(context, ["1.1.0"], { STUB_FRESHNESS_EXIT: "2" });
  assertRefused(result, "docs-freshness");
  const state = await stubState(context);
  assert.equal(state.pulls.length, 1);
  assert.equal(state.releases.length, 0);
  assert.equal(git(context.root, "branch", "--show-current"), BRANCH);
});

test("refuses a resumed bump whose version files no longer agree", async () => {
  const context = await fixture();
  assert.equal(
    prepare(context, ["1.1.0"], { STUB_CHECK_FULL_EXIT: "1" }).status,
    1,
  );
  const file = path.join(context.root, "frontend/package.json");
  await writeFile(
    file,
    (await readFile(file, "utf8")).replace(
      '"version": "1.1.0"',
      '"version": "1.0.9"',
    ),
  );
  git(context.root, "commit", "--quiet", "-am", "a hand edit after the bump");

  const result = prepare(context);
  assertRefused(result, "version files");
  assert.match(
    result.stderr,
    /these do not say 1\.1\.0: frontend\/package\.json \(1\.0\.9\)/,
  );
  assert.equal((await calls(context, "just", "check-full")).length, 1);
  assert.equal((await stubState(context)).pulls.length, 0);
});

test("refuses when the open release pull request belongs to another version", async () => {
  const context = await fixture();
  assert.equal(prepare(context).status, 0);
  await mergeBump(context);
  const state = await stubState(context);
  state.pulls.push({
    number: 9,
    url: "https://github.test/pull/9",
    state: "OPEN",
    base: "main",
    head: "development",
    title: "Release v1.0.5",
    body: "",
  });
  await writeFile(context.state, JSON.stringify(state));

  const result = prepare(context);
  assertRefused(result, "release pull request");
  assert.match(result.stderr, /"Release v1\.0\.5", not "Release v1\.1\.0"/);
  assert.equal((await stubState(context)).releases.length, 0);
});

test("a docs-freshness exit 1 without a reading list is a failure, not a result", async () => {
  const context = await fixture();
  assert.equal(prepare(context).status, 0);
  await mergeBump(context);
  const result = prepare(context, ["1.1.0"], {
    STUB_FRESHNESS_CHANGELOG_ONLY: "1",
  });
  assertRefused(result, "docs-freshness");
  assert.match(result.stderr, /changelog needs an entry/);
  assert.equal((await stubState(context)).pulls.length, 1);
});
