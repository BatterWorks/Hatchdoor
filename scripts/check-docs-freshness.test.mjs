import assert from "node:assert/strict";
import { copyFile, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { afterEach, test } from "node:test";
import { fileURLToPath } from "node:url";

const sourceScript = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "check-docs-freshness.mjs",
);
const temporaryRepositories = [];

const MCP_NOTE = "docs/user-vault/03 Reference/MCP tools reference.md";
const UI_NOTE =
  "docs/user-vault/01 Get started/Browse and review through the Web UI.md";
const ATTACHMENTS_NOTE =
  "docs/user-vault/02 Guides/How to import and work with attachments.md";

// Every note the mcp-tools, web-ui, attachments, and write-mutations surfaces
// name. The script refuses to print a reading list containing a file it cannot
// open, so a fixture that triggers a surface must carry that surface's whole
// note set.
const FIXTURE_NOTES = [
  MCP_NOTE,
  UI_NOTE,
  ATTACHMENTS_NOTE,
  "docs/user-vault/01 Get started/Connect your agent.md",
  "docs/user-vault/01 Get started/Search and change notes with your agent.md",
  "docs/user-vault/02 Guides/How to edit notes with the live editor.md",
];

function run(root, args = []) {
  return spawnSync(
    process.execPath,
    ["scripts/check-docs-freshness.mjs", ...args],
    { cwd: root, encoding: "utf8" },
  );
}

function git(root, args) {
  const result = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  assert.equal(
    result.status,
    0,
    `git ${args.join(" ")} failed: ${result.stderr}`,
  );
  return result;
}

async function write(root, file, contents = "placeholder\n") {
  const absolute = path.join(root, file);
  await mkdir(path.dirname(absolute), { recursive: true });
  await writeFile(absolute, contents);
}

// A repository whose `development` branch already holds the vault notes and a
// source tree, with HEAD on a feature branch that has changed nothing yet.
async function fixture() {
  const root = await mkdtemp(path.join(tmpdir(), "hatchdoor-docs-freshness-"));
  temporaryRepositories.push(root);

  await mkdir(path.join(root, "scripts"), { recursive: true });
  await copyFile(
    sourceScript,
    path.join(root, "scripts", "check-docs-freshness.mjs"),
  );
  await write(root, "src/mcp/tools/read.rs");
  await write(root, "src/lib.rs");
  await write(root, "frontend/src/App.tsx");
  for (const note of FIXTURE_NOTES) {
    await write(root, note, `# ${path.basename(note, ".md")}\n`);
  }

  // Isolate from the ambient git config. Without this the suite inherits the
  // developer's global settings: commit signing fails the fixture commits on
  // any machine whose key is unavailable, and a global core.hooksPath can run
  // arbitrary hooks inside the temporary repository.
  git(root, ["init", "--initial-branch=development", "--quiet", "--template="]);
  git(root, ["config", "user.email", "test@example.com"]);
  git(root, ["config", "user.name", "Test"]);
  git(root, ["config", "commit.gpgsign", "false"]);
  git(root, ["config", "tag.gpgsign", "false"]);
  git(root, ["config", "core.hooksPath", ""]);
  git(root, ["add", "."]);
  git(root, ["commit", "--quiet", "-m", "base"]);
  git(root, ["switch", "--quiet", "-c", "feature"]);
  return root;
}

async function commit(root, message) {
  git(root, ["add", "."]);
  git(root, ["commit", "--quiet", "-m", message]);
}

afterEach(async () => {
  await Promise.all(
    temporaryRepositories
      .splice(0)
      .map((root) => rm(root, { recursive: true, force: true })),
  );
});

test("passes when no user-facing surface changed", async () => {
  const root = await fixture();
  await write(root, "src/lib.rs", "// internal refactor\n");
  await commit(root, "refactor");

  const result = run(root);
  assert.equal(result.status, 0);
  assert.match(result.stdout, /no user-facing surface changed/);
});

test("fails and names the notes that document a changed surface", async () => {
  const root = await fixture();
  await write(root, "src/mcp/tools/read.rs", "// a new tool\n");
  await commit(root, "add a tool");

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /MCP tool catalogue and protocol/);
  assert.match(result.stderr, /MCP tools reference\.md {2}\(UNTOUCHED\)/);
  assert.doesNotMatch(result.stderr, /Browse and review through the Web UI/);
});

test("reports a note edited on the branch without treating it as proof", async () => {
  const root = await fixture();
  await write(root, "frontend/src/App.tsx", "// new panel\n");
  await write(
    root,
    UI_NOTE,
    "# Browse and review through the Web UI\n\nNew.\n",
  );
  await commit(root, "ui change plus docs");

  const result = run(root);
  assert.equal(result.status, 1, "an edited note is not an acknowledgement");
  assert.match(
    result.stderr,
    /Browse and review through the Web UI\.md {2}\(edited on this branch\)/,
  );
});

test("counts uncommitted working-tree changes", async () => {
  const root = await fixture();
  await write(root, "src/mcp/adapter.rs", "// uncommitted\n");

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /src\/mcp\/adapter\.rs/);
});

test("--acknowledge records the review and exits zero", async () => {
  const root = await fixture();
  await write(root, "src/mcp/tools/read.rs", "// a new tool\n");
  await commit(root, "add a tool");

  const result = run(root, ["--acknowledge"]);
  assert.equal(result.status, 0);
  assert.match(result.stderr, /Acknowledged: \d+ note\(s\) reviewed/);
});

test("one change can require several surfaces' notes", async () => {
  const root = await fixture();
  await write(root, "src/mcp/tools/read.rs", "// tool\n");
  await write(root, "frontend/src/App.tsx", "// panel\n");
  await commit(root, "both");

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /MCP tools reference\.md/);
  assert.match(result.stderr, /Browse and review through the Web UI\.md/);
});

// Attachment mutation lives under src/vault/write/, so it already triggered
// the note-mutation surface. What it did not do was summon the guide that
// actually documents move_attachment, rename_attachment, and delete_attachment
// (#220), which left that guide free to drift.
test("an attachment mutation change names the attachments guide too", async () => {
  const root = await fixture();
  await write(root, "src/vault/write/attachments.rs", "// move an asset\n");
  await commit(root, "attachment mutation");

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /How to import and work with attachments\.md/);
  assert.match(result.stderr, /MCP tools reference\.md/);
});

// The regression that motivated `-z`: `git status --porcelain` c-quotes any
// path containing a space, and every real note name has spaces. Parsing that
// as a bare line left the quotes attached, so a note edited but not yet
// committed — exactly the state during an acknowledgement — read as UNTOUCHED.
test("sees an uncommitted edit to a note whose path contains spaces", async () => {
  const root = await fixture();
  await write(root, "src/mcp/tools/read.rs", "// a new tool\n");
  await write(root, MCP_NOTE, "# MCP tools reference\n\nDocumented.\n");

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(
    result.stderr,
    /MCP tools reference\.md {2}\(edited on this branch\)/,
  );
  assert.doesNotMatch(result.stderr, /"docs\/user-vault/);
});

test("matches a surface path containing spaces", async () => {
  const root = await fixture();
  await write(root, "frontend/src/panel one.tsx", "// spaced source\n");

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /frontend\/src\/panel one\.tsx/);
});

test("reports a deleted source file as a changed surface", async () => {
  const root = await fixture();
  git(root, ["rm", "--quiet", "src/mcp/tools/read.rs"]);
  await commit(root, "drop a tool module");

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /src\/mcp\/tools\/read\.rs/);
});

test("follows a rename to its new path and ignores the original", async () => {
  const root = await fixture();
  git(root, ["mv", "src/mcp/tools/read.rs", "src/mcp/tools/renamed.rs"]);

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /src\/mcp\/tools\/renamed\.rs/);
});

test("expands a newly untracked directory instead of collapsing it", async () => {
  const root = await fixture();
  await write(root, "src/mcp/fresh/handler.rs", "// new module\n");

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /src\/mcp\/fresh\/handler\.rs/);
});

test("--base selects the ref it was given", async () => {
  const root = await fixture();
  await write(root, "src/mcp/tools/read.rs", "// a new tool\n");
  await commit(root, "add a tool");
  git(root, ["tag", "after-tool"]);
  await write(root, "src/lib.rs", "// internal only\n");
  await commit(root, "internal");

  const result = run(root, ["--base", "after-tool"]);
  assert.equal(result.status, 0, "only internal changes since the tag");
  assert.match(result.stdout, /since after-tool/);
});

test("rejects --base without a ref instead of silently using the default", async () => {
  const root = await fixture();

  const bare = run(root, ["--base"]);
  assert.equal(bare.status, 2);
  assert.match(bare.stderr, /--base needs a ref/);

  const swallowed = run(root, ["--base", "--acknowledge"]);
  assert.equal(swallowed.status, 2);
  assert.match(swallowed.stderr, /--base needs a ref/);
});

test("rejects an unknown argument", async () => {
  const root = await fixture();

  const result = run(root, ["--wat"]);
  assert.equal(result.status, 2);
  assert.match(result.stderr, /unknown argument --wat/);
});

test("names its base in the acknowledge hint", async () => {
  const root = await fixture();
  await write(root, "src/mcp/tools/read.rs", "// a new tool\n");
  await commit(root, "add a tool");
  git(root, ["tag", "start"]);
  await write(root, "src/mcp/tools/read.rs", "// more\n");
  await commit(root, "more");

  const result = run(root, ["--base", "start"]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /just docs-freshness-ack 'start'/);
});

test("refuses to hand over a reading list naming a note it cannot open", async () => {
  const root = await fixture();
  await write(root, "src/mcp/tools/read.rs", "// a new tool\n");
  await rm(path.join(root, MCP_NOTE));
  await commit(root, "delete a documented note");

  const result = run(root);
  assert.equal(result.status, 2);
  assert.match(result.stderr, /DEAD TABLE ENTRIES/);
  assert.match(result.stderr, /MCP tools reference\.md \(missing note\)/);
});

// Guards the table against the real repository: a renamed note or a moved
// module turns a rule into one that silently guards nothing.
test("every surface path and note in the table exists in this repository", () => {
  const result = spawnSync(
    process.execPath,
    [sourceScript, "--validate-table"],
    { cwd: path.dirname(path.dirname(sourceScript)), encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Surface table OK/);
});

test("exits 2 when the base ref does not exist", async () => {
  const root = await fixture();

  const result = run(root, ["--base", "no-such-branch"]);
  assert.equal(result.status, 2);
  assert.match(result.stderr, /no ref named no-such-branch/);
});
