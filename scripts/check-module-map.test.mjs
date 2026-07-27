import assert from "node:assert/strict";
import { copyFile, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { afterEach, test } from "node:test";
import { fileURLToPath } from "node:url";

const sourceScript = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "check-module-map.mjs",
);
const temporaryRepositories = [];

const baselineMap = `# Module map

## Backend

### Runtime

**Owned paths:** none by default.

**Paths:**

- \`src/lib.rs\`

## Frontend

### Shell

**Owned paths:** none by default.

**Paths:** \`frontend/src/App.tsx\`
`;

async function fixture(moduleMap = baselineMap) {
  const root = await mkdtemp(path.join(tmpdir(), "hatchdoor-module-map-"));
  temporaryRepositories.push(root);
  await Promise.all([
    mkdir(path.join(root, "scripts"), { recursive: true }),
    mkdir(path.join(root, "docs", "architecture"), { recursive: true }),
    mkdir(path.join(root, "src"), { recursive: true }),
    mkdir(path.join(root, "frontend", "src"), { recursive: true }),
  ]);
  await Promise.all([
    copyFile(sourceScript, path.join(root, "scripts", "check-module-map.mjs")),
    writeFile(
      path.join(root, "docs", "architecture", "module-map.md"),
      moduleMap,
    ),
    writeFile(path.join(root, "src", "lib.rs"), ""),
    writeFile(path.join(root, "frontend", "src", "App.tsx"), ""),
  ]);
  return root;
}

function run(root) {
  return spawnSync(process.execPath, ["scripts/check-module-map.mjs"], {
    cwd: root,
    encoding: "utf8",
  });
}

afterEach(async () => {
  await Promise.all(
    temporaryRepositories
      .splice(0)
      .map((root) => rm(root, { recursive: true, force: true })),
  );
});

test("accepts list and inline shared Paths assignments", async () => {
  const result = run(await fixture());
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /2 production source files/);
});

test("reports an unowned production file", async () => {
  const root = await fixture();
  await writeFile(path.join(root, "src", "new_feature.rs"), "");

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /UNOWNED PRODUCTION FILES/);
  assert.match(result.stderr, /src\/new_feature\.rs/);
});

test("reports a stale assigned path", async () => {
  const root = await fixture(
    baselineMap.replace("- `src/lib.rs`", "- `src/lib.rs`\n- `src/missing.rs`"),
  );

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /STALE ASSIGNED PATHS/);
  assert.match(result.stderr, /src\/missing\.rs/);
});

test("reports duplicate assignments with both module sections", async () => {
  const root = await fixture(
    baselineMap.replace(
      "## Frontend",
      `### Duplicate owner

**Owned paths:** \`src/lib.rs\`

## Frontend`,
    ),
  );

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /DUPLICATE ASSIGNMENTS/);
  assert.match(result.stderr, /Runtime/);
  assert.match(result.stderr, /Duplicate owner/);
});

test("rejects non-canonical and escaping ownership paths", async () => {
  const root = await fixture(
    baselineMap.replace(
      "- `src/lib.rs`",
      "- `src/lib.rs`\n- `src/../src/lib.rs`\n- `/etc/passwd`",
    ),
  );

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /INVALID OWNERSHIP MARKUP/);
  assert.match(result.stderr, /not a canonical repository-relative path/);
  assert.match(result.stderr, /\/etc\/passwd/);
});

test("only H3 headings establish module ownership", async () => {
  for (const marker of ["#", "##", "####", "#####", "######"]) {
    const root = await fixture(
      baselineMap.replace(
        "## Frontend",
        `${marker} Detail

**Owned paths:** \`src/lib.rs\`

## Frontend`,
      ),
    );

    const result = run(root);
    assert.equal(result.status, 1, `heading marker ${marker}`);
    assert.match(result.stderr, /INVALID OWNERSHIP MARKUP/);
    assert.match(result.stderr, /outside a module section/);
  }
});

test("rejects an assigned path that is not a regular file", async () => {
  const root = await fixture(
    baselineMap.replace("- `src/lib.rs`", "- `src/lib.rs`\n- `src/generated`"),
  );
  await mkdir(path.join(root, "src", "generated"));

  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /STALE ASSIGNED PATHS/);
  assert.match(result.stderr, /src\/generated/);
});

test("supports inline Owned paths and Shared path fields", async () => {
  const moduleMap = baselineMap
    .replace(
      "**Owned paths:** none by default.\n\n**Paths:**\n\n- `src/lib.rs`",
      "**Owned paths:** `src/lib.rs`.",
    )
    .replace(
      "**Owned paths:** none by default.\n\n**Paths:** `frontend/src/App.tsx`",
      "**Shared path:** `frontend/src/App.tsx`.",
    );

  const result = run(await fixture(moduleMap));
  assert.equal(result.status, 0, result.stderr);
});
