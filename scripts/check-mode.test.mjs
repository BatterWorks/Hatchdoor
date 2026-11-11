import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { existsSync } from "node:fs";
import { afterEach, test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  DOCS_ONLY,
  FULL,
  MANUAL_FRONTEND_TESTS,
  MANUAL_MODE,
  MANUAL_RUST_TESTS,
  classify,
  covers,
} from "./check-mode.mjs";

const scripts = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(scripts, "..");
const repositories = [];

const ADR = "docs/adr/0001-something.md";
const PAGE = "docs/user-vault/02 Guides/How to do a thing.md";

function git(root, args) {
  const result = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  assert.equal(result.status, 0, `git ${args.join(" ")}: ${result.stderr}`);
  return result.stdout;
}

function checkMode(root, args) {
  return spawnSync(
    process.execPath,
    [path.join(root, "scripts", "check-mode.mjs"), ...args],
    { cwd: root, encoding: "utf8" },
  );
}

async function write(root, file, contents = "placeholder\n") {
  const absolute = path.join(root, file);
  await mkdir(path.dirname(absolute), { recursive: true });
  await writeFile(absolute, contents);
}

// `development` holds code, docs and one manual page; HEAD is a feature
// branch that has changed nothing yet.
async function fixture() {
  const root = await mkdtemp(path.join(tmpdir(), "hatchdoor-check-mode-"));
  repositories.push(root);
  for (const script of ["check-mode.mjs", "git-changes.mjs"]) {
    await mkdir(path.join(root, "scripts"), { recursive: true });
    await copyFile(
      path.join(scripts, script),
      path.join(root, "scripts", script),
    );
  }
  await write(root, ".gitignore", "/.dev/\n");
  await write(root, "src/lib.rs");
  await write(root, ADR);
  await write(root, PAGE);
  await write(root, "README.md");
  git(root, ["init", "--quiet", "--initial-branch=development"]);
  git(root, ["config", "user.email", "test@example.invalid"]);
  git(root, ["config", "user.name", "Test"]);
  git(root, ["config", "commit.gpgsign", "false"]);
  git(root, ["add", "-A"]);
  git(root, ["commit", "--quiet", "-m", "base"]);
  git(root, ["switch", "--quiet", "-c", "feature/x"]);
  return root;
}

function start(root) {
  const result = checkMode(root, ["start"]);
  const [mode, tree] = result.stdout.trim().split(" ");
  return { status: result.status, mode, tree, stderr: result.stderr };
}

// What the recipe does after its checks pass.
function pass(root) {
  const { status, mode, tree } = start(root);
  assert.equal(status, 0);
  const recorded = checkMode(root, ["record", mode, tree]);
  assert.equal(recorded.status, 0, recorded.stderr);
  return mode;
}

afterEach(async () => {
  await Promise.all(
    repositories
      .splice(0)
      .map((root) => rm(root, { recursive: true, force: true })),
  );
});

const edits = (...files) => files.map((file) => ({ file, status: "M" }));

test("ADRs, docs, demo Vaults and root Markdown are documentation", () => {
  assert.equal(
    classify([
      ...edits(ADR, "docs/maintenance/x.md", "CHANGELOG.md", "AGENTS.md"),
      { file: "demo-vaults/a/b.md", status: "A" },
      { file: "docs/old.md", status: "D" },
    ]).mode,
    DOCS_ONLY,
  );
});

test("code, scripts and nested Markdown run in full", () => {
  for (const file of [
    "src/lib.rs",
    "frontend/src/App.tsx",
    "scripts/check-mode.mjs",
    "justfile",
    "frontend/README.md",
  ]) {
    const decision = classify(edits(ADR, PAGE, file));
    assert.equal(decision.mode, FULL, file);
    assert.ok(decision.reason.includes(file), decision.reason);
  }
});

test("a text edit to a manual page runs the manual tests", () => {
  const decision = classify(edits(ADR, PAGE));
  assert.equal(decision.mode, MANUAL_MODE);
  assert.ok(decision.reason.includes(PAGE), decision.reason);
});

test("adding or removing a manual page runs in full", () => {
  for (const status of ["A", "D", "T"]) {
    const decision = classify([{ file: PAGE, status }]);
    assert.equal(decision.mode, FULL, status);
    assert.match(decision.reason, /adds or removes a page/);
  }
});

test("no change at all runs in full", () => {
  assert.equal(classify([]).mode, FULL);
});

test("a branch changing only an ADR names the docs-only mode", async () => {
  const root = await fixture();
  await write(root, ADR, "changed\n");
  git(root, ["commit", "--quiet", "-am", "adr"]);
  const { status, mode, stderr } = start(root);
  assert.equal(status, 0);
  assert.equal(mode, DOCS_ONLY);
  assert.match(stderr, /^check: docs-only mode/);
});

test("uncommitted and untracked changes count", async () => {
  const root = await fixture();
  await write(root, ADR, "changed\n");
  assert.equal(start(root).mode, DOCS_ONLY);
  await write(root, "src/new.rs");
  assert.equal(start(root).mode, FULL);
});

test("touching src/ runs in full", async () => {
  const root = await fixture();
  await write(root, ADR, "changed\n");
  await write(root, "src/lib.rs", "changed\n");
  git(root, ["commit", "--quiet", "-am", "code"]);
  const { mode, stderr } = start(root);
  assert.equal(mode, FULL);
  assert.match(stderr, /src\/lib\.rs is not documentation/);
});

test("adding a manual page runs in full", async () => {
  const root = await fixture();
  await write(root, "docs/user-vault/02 Guides/A new page.md");
  assert.equal(start(root).mode, FULL);
});

test("a committed edit to a manual page names the manual mode", async () => {
  const root = await fixture();
  await write(root, PAGE, "changed\n");
  git(root, ["commit", "--quiet", "-am", "page"]);
  const { mode, stderr } = start(root);
  assert.equal(mode, MANUAL_MODE);
  assert.match(stderr, /^check: manual mode/);
});

test("renaming a manual page runs in full", async () => {
  const root = await fixture();
  git(root, ["mv", PAGE, "docs/user-vault/02 Guides/Renamed.md"]);
  git(root, ["commit", "--quiet", "-m", "rename"]);
  assert.equal(start(root).mode, FULL);
});

test("a branch with no change runs in full", async () => {
  const root = await fixture();
  assert.equal(start(root).mode, FULL);
});

test("a second run on a passed tree returns at once", async () => {
  const root = await fixture();
  await write(root, ADR, "changed\n");
  assert.equal(pass(root), DOCS_ONLY);
  const again = start(root);
  assert.equal(again.status, 10);
  assert.match(again.stderr, /already passed in docs-only mode/);
  assert.equal(again.mode, "");
});

test("committing a passed tree keeps the pass", async () => {
  const root = await fixture();
  await write(root, ADR, "changed\n");
  pass(root);
  git(root, ["commit", "--quiet", "-am", "adr"]);
  assert.equal(start(root).status, 10);
});

test("any edit after a pass runs again, committed or not", async () => {
  const root = await fixture();
  await write(root, ADR, "changed\n");
  pass(root);
  await write(root, ADR, "changed again\n");
  assert.equal(start(root).status, 0);
  pass(root);
  await write(root, "docs/new.md");
  assert.equal(start(root).status, 0, "an untracked file is an edit");
});

// The real `check` recipe, in a docs-only fixture whose script checks are
// stubs: the module map stub fails while `.dev/fail` exists and says when it
// ran. `.dev/` is ignored, so the flag does not change the tree.
async function recipeFixture() {
  const root = await fixture();
  await copyFile(
    path.join(repository, "justfile"),
    path.join(root, "justfile"),
  );
  await write(
    root,
    "scripts/check-module-map.mjs",
    'import { existsSync } from "node:fs";\n' +
      'console.log("module map ran");\n' +
      'process.exit(existsSync(".dev/fail") ? 1 : 0);\n',
  );
  await write(root, "scripts/check-docs-freshness.mjs", "");
  await write(
    root,
    "scripts/stub.test.mjs",
    'import { test } from "node:test";\ntest("stub", () => {});\n',
  );
  git(root, ["add", "-A"]);
  git(root, ["commit", "--quiet", "-m", "recipe"]);
  git(root, ["branch", "--quiet", "-f", "development"]);
  await write(root, ADR, "changed\n");
  return root;
}

function just(root) {
  const scratch = path.join(root, ".dev", "target");
  return spawnSync("just", ["check"], {
    cwd: root,
    encoding: "utf8",
    env: {
      ...process.env,
      CARGO_TARGET_DIR: scratch,
      HATCHDOOR_TMPDIR: scratch,
    },
  });
}

test("the recipe records nothing after a failed run, and checks again", async () => {
  const root = await recipeFixture();
  await write(root, ".dev/fail", "");
  const failed = just(root);
  assert.notEqual(failed.status, 0);
  assert.match(failed.stderr, /^check: docs-only mode/);
  assert.ok(!existsSync(path.join(root, ".dev", "check-pass.json")));

  await rm(path.join(root, ".dev", "fail"));
  const passed = just(root);
  assert.equal(passed.status, 0, passed.stderr);
  assert.match(passed.stdout, /module map ran/);
  assert.match(passed.stderr, /passed in docs-only mode/);

  const again = just(root);
  assert.equal(again.status, 0, again.stderr);
  assert.doesNotMatch(again.stdout, /module map ran/);
  assert.match(again.stderr, /already passed/);
});

test("a tree that moved during the run is not remembered", async () => {
  const root = await fixture();
  await write(root, ADR, "changed\n");
  const { mode, tree } = start(root);
  await write(root, ADR, "edited mid-run\n");
  const recorded = checkMode(root, ["record", mode, tree]);
  assert.equal(recorded.status, 0);
  assert.match(recorded.stderr, /not remembered/);
  assert.equal(start(root).status, 0);
});

test("a pass covers its own mode and narrower ones only", () => {
  assert.ok(covers({ tree: "t", mode: FULL }, "t", DOCS_ONLY));
  assert.ok(covers({ tree: "t", mode: FULL }, "t", MANUAL_MODE));
  assert.ok(covers({ tree: "t", mode: MANUAL_MODE }, "t", DOCS_ONLY));
  assert.ok(!covers({ tree: "t", mode: DOCS_ONLY }, "t", FULL));
  assert.ok(!covers({ tree: "t", mode: DOCS_ONLY }, "t", MANUAL_MODE));
  assert.ok(!covers({ tree: "t", mode: MANUAL_MODE }, "t", FULL));
  assert.ok(!covers({ tree: "t", mode: "bogus" }, "t", "bogus"));
  assert.ok(!covers({ tree: "t", mode: FULL }, "u", FULL));
  assert.ok(!covers(null, "t", FULL));
});

test("an edit to a file Git is told to assume unchanged still runs again", async () => {
  const root = await fixture();
  await write(root, ADR, "changed\n");
  pass(root);
  git(root, ["update-index", "--assume-unchanged", ADR]);
  await write(root, ADR, "changed behind the flag\n");
  assert.equal(start(root).status, 0);
});

test("a tree holding an untracked Git repository is never remembered", async () => {
  const root = await fixture();
  await write(root, "docs/nested/note.md");
  git(path.join(root, "docs", "nested"), ["init", "--quiet"]);
  const { status, mode, tree } = start(root);
  assert.equal(status, 0);
  assert.equal(tree, "none");
  const recorded = checkMode(root, ["record", mode, tree]);
  assert.match(recorded.stderr, /untracked Git repository/);
  assert.equal(start(root).status, 0);
});

test("computing the tree leaves the real index alone", async () => {
  const root = await fixture();
  await write(root, "docs/untracked.md");
  start(root);
  assert.match(
    git(root, ["status", "--porcelain"]),
    /^\?\? docs\/untracked\.md$/m,
  );
});

async function filesUnder(directory, pattern) {
  const found = [];
  for (const entry of await readdir(directory, {
    withFileTypes: true,
    recursive: true,
  })) {
    if (entry.isFile() && pattern.test(entry.name)) {
      found.push(path.join(entry.parentPath, entry.name));
    }
  }
  return found;
}

// Whatever names the manual, its folder, its routes or its tools, may hold a
// test that reads the manual's text, and manual mode must run it.
const NAMES_THE_MANUAL =
  /docs_bundle|user-vault|read_docs|search_docs|whats-new|whats_new|llms\.txt|"\/docs/;

test("manual mode runs the tests of every Rust module that names the manual", async () => {
  const missing = [];
  for (const file of await filesUnder(path.join(repository, "src"), /\.rs$/)) {
    const source = await readFile(file, "utf8");
    if (!NAMES_THE_MANUAL.test(source)) {
      continue;
    }
    const relative = path.relative(path.join(repository, "src"), file);
    const isDeclaration = /(^|\/)(lib|main|mod)\.rs$/.test(relative);
    if (
      isDeclaration &&
      !source.includes("#[test]") &&
      !source.includes("#[tokio::test]")
    ) {
      continue;
    }
    const module = relative
      .replace(/\.rs$/, "")
      .replace(/(^|\/)(lib|main|mod)$/, "")
      .split(path.sep)
      .filter(Boolean)
      .join("::");
    if (!MANUAL_RUST_TESTS.some((filter) => `${module}::`.includes(filter))) {
      missing.push(`${relative} (${module}::)`);
    }
  }
  assert.deepEqual(
    missing,
    [],
    "add these to MANUAL_RUST_TESTS in scripts/check-mode.mjs",
  );
});

test("manual mode runs every frontend test that reads the manual", async () => {
  const frontend = path.join(repository, "frontend");
  const readers = [];
  for (const file of await filesUnder(
    path.join(frontend, "src"),
    /\.test\.tsx?$/,
  )) {
    if (/user-vault/.test(await readFile(file, "utf8"))) {
      readers.push(path.relative(frontend, file).split(path.sep).join("/"));
    }
  }
  assert.deepEqual(readers.sort(), [...MANUAL_FRONTEND_TESTS].sort());
});
