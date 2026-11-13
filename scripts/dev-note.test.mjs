import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";

import {
  PLACED_LIST,
  notesIn,
  placeFrom,
  placeNote,
  slugOnceIndexed,
  vaultFrom,
} from "./dev-note.mjs";
import { FAKE_TOKEN, freePort, runNode, serve } from "./dev-test-support.mjs";
import { ToolError } from "./proto-common.mjs";

const SCRIPT = path.join(import.meta.dirname, "dev-note.mjs");
const HEALTHY = "11111111-1111-4111-8111-111111111111";
const READONLY = "33333333-3333-4333-8333-333333333333";
const DISABLED = "66666666-6666-4666-8666-666666666666";

// A dev directory shaped like the one `scripts/dev-vaults.sh` makes: a
// healthy Vault, a read-only one (0555 folders, 0444 files) and a registry.
function fixture() {
  const devDir = fs.realpathSync(
    fs.mkdtempSync(path.join(tmpdir(), "dev-note-test-")),
  );
  const vaults = path.join(devDir, "vaults");
  for (const name of ["healthy", "readonly"]) {
    fs.mkdirSync(path.join(vaults, name, "30-areas"), { recursive: true });
    fs.writeFileSync(path.join(vaults, name, "30-areas", "Homelab.md"), "kept\n");
  }
  fs.writeFileSync(path.join(devDir, "outside.md"), "outside\n");
  fs.symlinkSync(devDir, path.join(vaults, "healthy", "escape"));
  for (const entry of ["30-areas/Homelab.md", "30-areas", "."]) {
    const target = path.join(vaults, "readonly", entry);
    fs.chmodSync(target, fs.statSync(target).isDirectory() ? 0o555 : 0o444);
  }
  const local = (name, folder, enabled = true) => ({
    name,
    enabled,
    source: { type: "local", path: path.join(vaults, folder) },
  });
  const registry = {
    vaults: {
      [HEALTHY]: local("Healthy", "healthy"),
      [READONLY]: local("Read-only", "readonly"),
      [DISABLED]: local("Disabled", "disabled", false),
      "99999999-9999-4999-8999-999999999999": {
        name: "Git",
        enabled: true,
        source: { type: "managed_git" },
      },
      "55555555-5555-4555-8555-555555555555": local("Missing path", "gone"),
      "00000000-0000-4000-8000-000000000000": {
        name: "Elsewhere",
        enabled: true,
        source: { type: "local", path: devDir },
      },
    },
  };
  fs.mkdirSync(path.join(devDir, "state"));
  fs.writeFileSync(
    path.join(devDir, "state", "vaults.json"),
    JSON.stringify(registry),
  );
  return {
    devDir,
    vaults,
    registry,
    remove() {
      // A read-only folder defeats a plain removal.
      spawnSync("chmod", ["-R", "u+w", devDir]);
      fs.rmSync(devDir, { recursive: true, force: true });
    },
  };
}

function modeOf(target) {
  return fs.statSync(target).mode & 0o777;
}

// A stand-in for the dev backend: the tree and the notes of whatever is on
// disk under a Vault, once `indexed()` says so.
function fakeBackend(vaults, indexed = () => true) {
  const folders = { [HEALTHY]: "healthy", [READONLY]: "readonly" };
  const slugOf = (file) => path.basename(file).replace(/\.md$/i, "").toLowerCase().replace(/\W+/g, "-");
  const notes = (folder) =>
    indexed()
      ? fs
          .readdirSync(folder, { recursive: true })
          .filter((file) => /\.md$/i.test(file))
          .sort()
      : [];
  const seen = { bearers: new Set() };
  return serve((request, response) => {
    seen.bearers.add(request.headers.authorization);
    if (request.headers.authorization !== `Bearer ${FAKE_TOKEN}`) {
      response.writeHead(401).end(`no such token: ${request.headers.authorization}`);
      return;
    }
    const json = (status, body) =>
      response
        .writeHead(status, { "content-type": "application/json" })
        .end(JSON.stringify(body));
    const [, id, rest] =
      /^\/api\/v1\/vaults\/([^/]+)\/(.*)$/.exec(request.url) ?? [];
    if (!id) {
      json(200, { vaults: [{ vault_id: READONLY, search: "unavailable", index_turn: "waiting" }] });
      return;
    }
    if (!folders[id]) {
      json(404, { code: "vault_not_found", message: "Vault definition was not found", retryable: false });
      return;
    }
    const folder = path.join(vaults, folders[id]);
    if (rest === "tree") {
      if (!indexed()) {
        json(503, { code: "vault_unavailable", message: `no snapshot for ${request.headers.authorization}`, retryable: true });
        return;
      }
      json(200, {
        data: [
          {
            tree: {
              notes: [],
              folders: [{ notes: notes(folder).map((file) => ({ slug: slugOf(file) })) }],
            },
          },
        ],
      });
      return;
    }
    const slug = decodeURIComponent(rest.replace(/^notes\//, ""));
    const file = notes(folder).find((candidate) => slugOf(candidate) === slug);
    if (file) {
      json(200, { note: { slug, relative_path: file.replace(/\.md$/i, "") } });
    } else {
      json(404, { code: "note_not_found" });
    }
  }).then((server) => ({ ...server, seen }));
}

test("a Vault is named by its folder, its name or its id", () => {
  const { registry, vaults, remove } = fixture();
  try {
    for (const wanted of ["readonly", "read-only", READONLY]) {
      assert.equal(vaultFrom(registry, wanted, vaults).id, READONLY);
    }
    assert.throws(
      () => vaultFrom(registry, "nope", vaults),
      /no dev fixture Vault named nope; this profile has: healthy, readonly/,
    );
    assert.throws(() => vaultFrom(registry, "Disabled", vaults), /disabled/);
    assert.throws(() => vaultFrom(registry, "Missing path", vaults), /no folder/);
    // Neither a Git Vault nor a folder outside `.dev/vaults/` is a fixture.
    assert.throws(() => vaultFrom(registry, "Git", vaults), ToolError);
    assert.throws(() => vaultFrom(registry, "Elsewhere", vaults), ToolError);
  } finally {
    remove();
  }
});

test("a note lands at the path given, or under its file's name", () => {
  assert.equal(placeFrom("Inbox/A b.md", "x.md", ""), "Inbox/A b.md");
  assert.equal(placeFrom(undefined, "/tmp/Draft note.md", ""), "Draft note.md");
  assert.match(placeFrom(undefined, "-", "text"), /^dev-note-[0-9a-f]{8}\.md$/);
  assert.equal(placeFrom(undefined, "-", "text"), placeFrom(undefined, "-", "text"));
  // Any case of the extension is a note, as it is to the index (#637, #766).
  assert.equal(placeFrom("Folder/Upper.MD", "-", ""), "Folder/Upper.MD");
  assert.equal(placeFrom("Folder/Mixed.Md", "-", ""), "Folder/Mixed.Md");
  assert.equal(placeFrom(undefined, "/tmp/Draft.MD", ""), "Draft.MD");
  for (const bad of [
    "../x.md",
    "/abs/x.md",
    "a//x.md",
    "x.txt",
    "a/.md",
    "./x.md",
    "Folder/.MD",
    "Folder/Name.txt",
    "Folder/Name.mdx",
  ]) {
    assert.throws(() => placeFrom(bad, "-", ""), ToolError, bad);
  }
});

test("a note goes into a healthy Vault, new folders and all", () => {
  const { vaults, remove } = fixture();
  try {
    const vault = path.join(vaults, "healthy");
    placeNote(vault, "New/Deep/Test.md", "hello\n");
    assert.equal(fs.readFileSync(path.join(vault, "New/Deep/Test.md"), "utf8"), "hello\n");
    placeNote(vault, "30-areas/Second.md", "x");
    assert.ok(fs.existsSync(path.join(vault, "30-areas/Second.md")));
  } finally {
    remove();
  }
});

test("a read-only Vault takes a note and is read-only again afterwards", () => {
  const { vaults, remove } = fixture();
  try {
    const vault = path.join(vaults, "readonly");
    placeNote(vault, "30-areas/Placed.md", "one");
    placeNote(vault, "Root.md", "two");
    placeNote(vault, "New/Deep/Placed.md", "three");
    assert.equal(fs.readFileSync(path.join(vault, "New/Deep/Placed.md"), "utf8"), "three");
    for (const folder of [".", "30-areas", "New", "New/Deep"]) {
      assert.equal(modeOf(path.join(vault, folder)), 0o555, folder);
    }
    for (const file of ["30-areas/Placed.md", "Root.md", "New/Deep/Placed.md"]) {
      assert.equal(modeOf(path.join(vault, file)), 0o444, file);
    }
    assert.equal(fs.readFileSync(path.join(vault, "30-areas/Homelab.md"), "utf8"), "kept\n");
  } finally {
    remove();
  }
});

test("an existing path is refused and left as it was, permissions too", () => {
  const { vaults, remove } = fixture();
  try {
    for (const name of ["healthy", "readonly"]) {
      const vault = path.join(vaults, name);
      const before = modeOf(path.join(vault, "30-areas"));
      assert.throws(
        () => placeNote(vault, "30-areas/Homelab.md", "overwritten"),
        /already there, and dev-note never overwrites/,
      );
      assert.equal(fs.readFileSync(path.join(vault, "30-areas/Homelab.md"), "utf8"), "kept\n");
      assert.equal(modeOf(path.join(vault, "30-areas")), before);
    }
    // A name that differs from a note's only in the extension's case is
    // refused too: the index keeps one of the two and hides the other (#766).
    for (const twin of ["30-areas/Homelab.MD", "30-areas/Homelab.Md"]) {
      assert.throws(
        () => placeNote(path.join(vaults, "healthy"), twin, "twin"),
        /30-areas\/Homelab\.md is already there/,
      );
      assert.equal(fs.existsSync(path.join(vaults, "healthy", twin)), false);
    }
    // A folder is as much in the way as a note.
    assert.throws(() => placeNote(path.join(vaults, "healthy"), "30-areas", "x"), ToolError);
  } finally {
    remove();
  }
});

test("a path that leaves the Vault through a link writes nothing", () => {
  const { devDir, vaults, remove } = fixture();
  try {
    assert.throws(
      () => placeNote(path.join(vaults, "healthy"), "escape/Stray.md", "x"),
      /leads out of the Vault/,
    );
    assert.equal(fs.existsSync(path.join(devDir, "Stray.md")), false);
    assert.throws(
      () => placeNote(path.join(vaults, "healthy"), "30-areas/Homelab.md/In.md", "x"),
      ToolError,
    );
  } finally {
    remove();
  }
});

test("the slug is the indexed note's own, found by the path it was placed at", async () => {
  const tree = (slugs) => ({ data: [{ tree: { notes: [], folders: [{ notes: slugs.map((slug) => ({ slug })) }] } }] });
  assert.deepEqual(
    notesIn({ notes: [{ slug: "a" }], folders: [{ notes: [{ slug: "b" }], folders: [{ notes: [{ slug: "c" }] }] }] }).map((note) => note.slug),
    ["a", "b", "c"],
  );
  // The new note took the slug an older note had, which now carries `-2`.
  const paths = { test: "Inbox/Test", "test-2": "Old/Test", other: "Other" };
  let polls = 0;
  const read = [];
  const api = async (route) => {
    if (route.endsWith("/tree")) {
      polls += 1;
      return polls < 3 ? undefined : tree(polls < 5 ? ["other"] : ["other", "test", "test-2"]);
    }
    const slug = route.split("/").at(-1);
    read.push(slug);
    return { note: { relative_path: paths[slug] } };
  };
  const slug = await slugOnceIndexed({
    api,
    vaultId: HEALTHY,
    place: "Inbox/Test.md",
    before: new Set(["other", "test"]),
    seconds: 5,
    intervalMs: 5,
  });
  assert.equal(slug, "test");
  // The new slug is read first, and an unchanged tree is not read again.
  assert.deepEqual(read, ["other", "test-2", "other", "test"]);

  const never = await slugOnceIndexed({
    api: async () => undefined,
    vaultId: HEALTHY,
    place: "Inbox/Test.md",
    before: undefined,
    seconds: 0.05,
    intervalMs: 5,
  });
  assert.equal(never, undefined);
});

test("dev-note places, waits for the index, and prints the slug and address", async () => {
  const { devDir, vaults, remove } = fixture();
  let indexed = true;
  const backend = await fakeBackend(vaults, () => indexed);
  const options = (extra = []) => [
    SCRIPT,
    "--backend-port",
    String(backend.port),
    "--frontend-port",
    "5173",
    "--dev-dir",
    devDir,
    ...extra,
  ];
  const env = { HATCHDOOR_WEB_BEARER_TOKEN: FAKE_TOKEN };
  try {
    const source = path.join(devDir, "outside.md");
    const healthy = await runNode(options(["healthy", source, "Inbox/Acceptance note.md"]), { env });
    assert.equal(healthy.status, 0, healthy.stderr);
    assert.match(healthy.stdout, /^placed healthy\/Inbox\/Acceptance note\.md$/m);
    assert.match(healthy.stdout, /^slug   acceptance-note$/m);
    assert.match(
      healthy.stdout,
      new RegExp(`^url    http://127\\.0\\.0\\.1:5173/v/${HEALTHY}/n/acceptance-note$`, "m"),
    );
    assert.deepEqual([...backend.seen.bearers], [`Bearer ${FAKE_TOKEN}`]);

    // An upper-case extension is placed as given and found by the wait (#766).
    const upper = await runNode(options(["healthy", "-", "Folder/Upper.MD"]), {
      env,
      input: "# Upper\n",
    });
    assert.equal(upper.status, 0, upper.stderr);
    assert.match(upper.stdout, /^placed healthy\/Folder\/Upper\.MD$/m);
    assert.match(upper.stdout, /^slug   upper$/m);
    assert.ok(fs.existsSync(path.join(vaults, "healthy", "Folder", "Upper.MD")));

    const readonly = await runNode(options(["readonly", "-"]), { env, input: "from stdin\n" });
    assert.equal(readonly.status, 0, readonly.stderr);
    assert.match(readonly.stdout, /^slug   dev-note-[0-9a-f]{8}$/m);
    assert.equal(modeOf(path.join(vaults, "readonly")), 0o555);

    // The list of what was placed sits where a reset removes it.
    assert.equal(
      fs.readFileSync(path.join(vaults, PLACED_LIST), "utf8").split("\n")[0],
      "healthy/Inbox/Acceptance note.md",
    );

    const again = await runNode(options(["healthy", source, "Inbox/Acceptance note.md"]), { env });
    assert.equal(again.status, 1);
    assert.match(again.stderr, /already there/);
    const existing = await runNode(options(["healthy", source, "30-areas/Homelab.md"]), { env });
    assert.equal(existing.status, 1);
    assert.equal(
      fs.readFileSync(path.join(vaults, "healthy/30-areas/Homelab.md"), "utf8"),
      "kept\n",
    );

    // An index that never has the note: placed, and said so, within the limit.
    indexed = false;
    const slow = await runNode(options(["--timeout", "1", "readonly", source, "Slow.md"]), { env });
    assert.equal(slow.status, 1);
    assert.match(slow.stderr, /Slow\.md is in the Read-only Vault, but the index did not have it within 1 seconds/);
    assert.match(slow.stderr, /search is unavailable, its index turn waiting/);

    for (const run of [healthy, readonly, again, existing, slow]) {
      assert.doesNotMatch(run.stdout + run.stderr, new RegExp(FAKE_TOKEN));
    }
  } finally {
    await backend.close();
    remove();
  }
});

test("dev-note writes nothing when the dev server refuses to list the Vault", async () => {
  const { devDir, vaults, registry, remove } = fixture();
  const backend = await fakeBackend(vaults);
  const run = (env, ...rest) =>
    runNode(
      [SCRIPT, "--backend-port", String(backend.port), "--dev-dir", devDir, ...rest],
      { env, input: "x" },
    );
  try {
    // A token the server does not hold.
    const wrong = "wrong-token-77aa";
    const refused = await run({ HATCHDOOR_WEB_BEARER_TOKEN: wrong }, "healthy", "-", "Refused.md");
    assert.equal(refused.status, 1);
    assert.match(refused.stderr, /refused to list the Healthy Vault \(401.*nothing was written/);
    assert.doesNotMatch(refused.stdout + refused.stderr, new RegExp(wrong));
    assert.equal(fs.existsSync(path.join(vaults, "healthy", "Refused.md")), false);

    // A backend started before the fixtures were rebuilt: it has no such Vault.
    const stale = path.join(devDir, "stale.json");
    const moved = structuredClone(registry);
    moved.vaults["12121212-1212-4121-8121-121212121212"] = moved.vaults[HEALTHY];
    delete moved.vaults[HEALTHY];
    fs.writeFileSync(stale, JSON.stringify(moved));
    const unknown = await run(
      { HATCHDOOR_WEB_BEARER_TOKEN: FAKE_TOKEN },
      "--registry",
      stale,
      "healthy",
      "-",
      "Unknown.md",
    );
    assert.equal(unknown.status, 1);
    assert.match(unknown.stderr, /\(404: Vault definition was not found\), so nothing was written.*just dev-restart-backend/);
    assert.equal(fs.existsSync(path.join(vaults, "healthy", "Unknown.md")), false);
    assert.equal(fs.existsSync(path.join(vaults, PLACED_LIST)), false);
  } finally {
    await backend.close();
    remove();
  }
});

test("dev-note writes nothing when the dev server is down or the Vault unknown", async () => {
  const { devDir, vaults, remove } = fixture();
  const env = { HATCHDOOR_WEB_BEARER_TOKEN: FAKE_TOKEN };
  try {
    const down = await runNode(
      [SCRIPT, "--backend-port", String(await freePort()), "--dev-dir", devDir, "healthy", "-", "Down.md"],
      { env, input: "x" },
    );
    assert.equal(down.status, 1);
    assert.match(down.stderr, /did not answer.*just dev-start/);
    assert.equal(fs.existsSync(path.join(vaults, "healthy", "Down.md")), false);
    assert.equal(fs.existsSync(path.join(vaults, PLACED_LIST)), false);

    const unknown = await runNode([SCRIPT, "--dev-dir", devDir, "nope", "-"], { env });
    assert.equal(unknown.status, 1);
    assert.match(unknown.stderr, /no dev fixture Vault named nope/);

    const usage = await runNode([SCRIPT, "--dev-dir", devDir, "healthy"], { env });
    assert.equal(usage.status, 1);
    assert.match(usage.stderr, /usage: just dev-note/);
    for (const run of [down, unknown, usage]) {
      assert.doesNotMatch(run.stdout + run.stderr, new RegExp(FAKE_TOKEN));
    }
  } finally {
    remove();
  }
});
