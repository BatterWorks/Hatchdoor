// `just dev-note <vault> <file or -> [path in the Vault]` (#616): place a test
// note in a dev fixture Vault and wait until the index has it.
//
// `<vault>` is a fixture's folder under `.dev/vaults/` (`healthy`,
// `readonly`), its name or its id. The note comes from a file, or from
// standard input with `-`. It lands at the path given, or under the file's
// own name at the Vault's root. It prints the note's slug and its address on
// the dev frontend.
//
// It only ever adds a file:
//
// - A path that already holds anything is refused, so no fixture is
//   overwritten.
// - A read-only Vault gets its write permission for the one folder the note
//   goes in, for as long as the write takes, and has it taken away again.
// - Each placed note is listed in `.dev/vaults/placed-notes.txt`. That file
//   sits beside the fixtures, so `just dev-vaults-reset`, which rebuilds
//   `.dev/vaults/` whole, removes the notes and their list together.

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import {
  BACKEND_PORT,
  DEV_DIR,
  DEV_HOST,
  FRONTEND_PORT,
  WEB_TOKEN_SETTING,
  devSetting,
  notAnswering,
  portFrom,
  runScrubbed,
  sleep,
  standardInput,
  takeOption,
} from "./dev-common.mjs";
import { ToolError } from "./proto-common.mjs";

const DEFAULT_SECONDS = 120;
const USAGE =
  "usage: just dev-note <vault> <file or -> [path in the Vault]\n" +
  "   as in: just dev-note readonly draft.md '20-projects/Acceptance note.md'";

export const PLACED_LIST = "placed-notes.txt";

// A note's extension, in any case, as the index reads it (#637, #766).
const NOTE_EXTENSION = /\.md$/i;

function inside(parent, child) {
  const relative = path.relative(parent, child);
  return relative !== "" && !relative.startsWith("..") && !path.isAbsolute(relative);
}

// The fixture Vault `wanted` names: its id, its folder under `vaultsDir`, or
// its name. Only an enabled Vault whose folder exists there will do, since
// nothing else can ever show the note.
export function vaultFrom(registry, wanted, vaultsDir) {
  const local = Object.entries(registry.vaults ?? {})
    .filter(([, vault]) => vault.source?.type === "local")
    .map(([id, vault]) => ({
      id,
      name: vault.name,
      enabled: vault.enabled,
      folder: vault.source.path,
    }))
    .filter((vault) => inside(vaultsDir, vault.folder));
  const found = local.find(
    (vault) =>
      vault.id === wanted ||
      path.basename(vault.folder) === wanted ||
      vault.name.toLowerCase() === (wanted ?? "").toLowerCase(),
  );
  const known = local.map((vault) => path.basename(vault.folder)).join(", ");
  if (!found) {
    throw new ToolError(
      `there is no dev fixture Vault named ${wanted}; this profile has: ${known || "none"}`,
    );
  }
  if (!found.enabled) {
    throw new ToolError(
      `the ${found.name} Vault is disabled, so a note placed there would never be indexed`,
    );
  }
  if (!fs.existsSync(found.folder) || !fs.statSync(found.folder).isDirectory()) {
    throw new ToolError(`the ${found.name} Vault has no folder on disk`);
  }
  return found;
}

// Where in the Vault the note goes, as `folder/Name.md`. `.MD` and `.Md` are
// notes too, and keep the case they were given.
export function placeFrom(asPath, source, content) {
  let place = asPath;
  if (!place) {
    place =
      source === "-"
        ? `dev-note-${createHash("sha256").update(content).digest("hex").slice(0, 8)}.md`
        : path.basename(source);
  }
  const parts = place.split("/");
  if (
    path.isAbsolute(place) ||
    parts.some((part) => part === "" || part === "." || part === "..") ||
    !NOTE_EXTENSION.test(place) ||
    // A file name that is the extension alone has no stem.
    parts.at(-1).replace(NOTE_EXTENSION, "") === ""
  ) {
    throw new ToolError(
      `a note's path is relative to the Vault and ends in .md, as in "Inbox/Test note.md": ${place}`,
    );
  }
  return place;
}

function writable(target) {
  return (fs.statSync(target).mode & 0o200) !== 0;
}

// Write the note, refusing a path that holds anything. Returns nothing; the
// Vault's permissions are as they were when it returns or throws.
export function placeNote(vaultFolder, place, content) {
  const root = fs.realpathSync(vaultFolder);
  const target = path.join(root, place);
  // `anchor` is the deepest folder that exists. It must be the Vault's own: a fixture
  // holds links that lead out of it.
  let existing = path.dirname(target);
  while (!fs.existsSync(existing)) {
    existing = path.dirname(existing);
  }
  const anchor = fs.realpathSync(existing);
  if (anchor !== root && !inside(root, anchor)) {
    throw new ToolError(`${place} leads out of the Vault, so nothing was written`);
  }
  if (!fs.statSync(anchor).isDirectory()) {
    throw new ToolError(`${place} goes through a file, not a folder`);
  }
  // `lstat`, so a dangling link counts as taken too.
  if (fs.lstatSync(target, { throwIfNoEntry: false })) {
    throw new ToolError(
      `${place} is already there, and dev-note never overwrites a note; choose another path`,
    );
  }
  // `Name.MD` beside `Name.md` is one note to the index, which keeps one file
  // and hides the other (#637), so a case twin is as much in the way (#766).
  const name = path.basename(target);
  const stem = name.replace(NOTE_EXTENSION, "");
  const twin =
    fs.existsSync(path.dirname(target)) &&
    fs
      .readdirSync(path.dirname(target))
      .find(
        (other) =>
          other !== name &&
          NOTE_EXTENSION.test(other) &&
          other.replace(NOTE_EXTENSION, "") === stem,
      );
  if (twin) {
    throw new ToolError(
      `${path.posix.join(path.posix.dirname(place), twin)} is already there, and the index takes ${place} for the same note; choose another path`,
    );
  }

  const readOnly = !writable(root);
  const unlocked = !writable(anchor);
  const mode = fs.statSync(anchor).mode & 0o7777;
  const made = [];
  try {
    if (unlocked) {
      fs.chmodSync(anchor, mode | 0o200);
    }
    let folder = anchor;
    for (const part of path.relative(anchor, path.dirname(target)).split(path.sep)) {
      if (!part) {
        continue;
      }
      folder = path.join(folder, part);
      fs.mkdirSync(folder);
      made.push(folder);
    }
    // `wx` fails when the path appeared since the check above.
    fs.writeFileSync(target, content, { flag: "wx" });
    if (readOnly) {
      fs.chmodSync(target, 0o444);
    }
  } finally {
    if (readOnly) {
      for (const folder of made.reverse()) {
        fs.chmodSync(folder, 0o555);
      }
    }
    if (unlocked) {
      fs.chmodSync(anchor, mode);
    }
  }
}

export function recordPlaced(vaultsDir, vaultFolder, place) {
  fs.appendFileSync(
    path.join(vaultsDir, PLACED_LIST),
    `${path.basename(vaultFolder)}/${place}\n`,
  );
}

// Every `{ slug }` in a tree answer, folders included.
export function notesIn(tree) {
  return [
    ...(tree.notes ?? []),
    ...(tree.folders ?? []).flatMap((folder) => notesIn(folder)),
  ];
}

// The slug the index gave the note at `place`. A slug comes from a note's
// file name and from what else shares it, so it is asked for, not worked
// out: the note is the one whose `relative_path` is the path it was placed
// at. `before` is the slugs the Vault had first, which are looked at last.
export async function slugOnceIndexed({
  api,
  vaultId,
  place,
  before,
  seconds,
  intervalMs = 500,
}) {
  const wanted = place.replace(NOTE_EXTENSION, "");
  const base = `/api/v1/vaults/${encodeURIComponent(vaultId)}`;
  const deadline = Date.now() + seconds * 1000;
  let scanned;
  for (;;) {
    const answer = await api(`${base}/tree`);
    const tree = answer?.data?.[0]?.tree;
    if (tree) {
      const slugs = notesIn(tree).map((note) => note.slug);
      const fresh = slugs.filter((slug) => !before?.has(slug));
      const key = slugs.join("\n");
      // Every slug once for each change to the tree, the new ones each time.
      const candidates =
        key === scanned
          ? fresh
          : [...fresh, ...slugs.filter((slug) => before?.has(slug))];
      scanned = key;
      for (const slug of candidates) {
        const read = await api(`${base}/notes/${encodeURIComponent(slug)}`);
        if (read?.note?.relative_path === wanted) {
          return slug;
        }
      }
    }
    if (Date.now() >= deadline) {
      return undefined;
    }
    await sleep(intervalMs);
  }
}

async function main() {
  const args = process.argv.slice(2);
  const backendPort = portFrom(takeOption(args, "--backend-port"), BACKEND_PORT);
  const frontendPort = portFrom(
    takeOption(args, "--frontend-port"),
    FRONTEND_PORT,
  );
  const devDir = takeOption(args, "--dev-dir") ?? DEV_DIR;
  // The registry the backend reads: the recipe passes the path it exports.
  const registryPath =
    takeOption(args, "--registry") || path.join(devDir, "state", "vaults.json");
  const limit = takeOption(args, "--timeout");
  const seconds = limit === undefined ? DEFAULT_SECONDS : Number(limit);
  const [wanted, source, asPath, ...extra] = args.filter((arg) => arg !== "");
  if (!wanted || !source || extra.length || !(seconds > 0)) {
    throw new ToolError(USAGE);
  }

  const vaultsDir = path.join(devDir, "vaults");
  let registry;
  try {
    registry = JSON.parse(fs.readFileSync(registryPath, "utf8"));
  } catch {
    throw new ToolError(
      "there are no dev fixture Vaults yet; `just dev-start` makes them",
    );
  }
  const vault = vaultFrom(registry, wanted, vaultsDir);

  let content;
  try {
    content = source === "-" ? await standardInput() : fs.readFileSync(source);
  } catch {
    throw new ToolError(`${source} could not be read`);
  }
  const place = placeFrom(asPath, source, content);

  const token = devSetting(WEB_TOKEN_SETTING);
  const origin = `http://${DEV_HOST}:${backendPort}`;
  // One reading of the dev server: its status and its JSON, if it sent any.
  const ask = async (route) => {
    let response;
    try {
      response = await fetch(origin + route, {
        headers: token ? { Authorization: `Bearer ${token}` } : {},
      });
    } catch {
      throw new ToolError(notAnswering("dev server", origin));
    }
    const text = await response.text();
    let body;
    try {
      body = JSON.parse(text);
    } catch {
      body = undefined;
    }
    return { ok: response.ok, status: response.status, body, text };
  };
  // The JSON, or nothing while the server has none to give.
  const api = async (route) => {
    const answer = await ask(route);
    return answer.ok ? answer.body : undefined;
  };

  // Asked before anything is written, so nothing is placed when the server
  // is down, refuses the token or does not know the Vault. A Vault still
  // waiting for its first index answers with a refusal worth retrying, and
  // that one is waited out below.
  const treeRoute = `/api/v1/vaults/${encodeURIComponent(vault.id)}/tree`;
  const asked = await ask(treeRoute);
  if (!asked.ok && asked.body?.retryable !== true) {
    const said = asked.body?.message ?? asked.text.trim();
    const hint =
      asked.status === 404
        ? "; a backend started before the fixtures were rebuilt does not know them, and `just dev-restart-backend` puts that right"
        : "";
    throw new ToolError(
      `the dev server refused to list the ${vault.name} Vault (${asked.status}${said ? `: ${said}` : ""}), so nothing was written${hint}`,
    );
  }
  const treeBefore = asked.ok ? asked.body?.data?.[0]?.tree : undefined;
  const before = treeBefore
    ? new Set(notesIn(treeBefore).map((note) => note.slug))
    : undefined;

  placeNote(vault.folder, place, content);
  recordPlaced(vaultsDir, vault.folder, place);

  const slug = await slugOnceIndexed({
    api,
    vaultId: vault.id,
    place,
    before,
    seconds,
  });
  if (!slug) {
    const listed = await api("/api/v1/vaults");
    const now = listed?.vaults?.find((entry) => entry.vault_id === vault.id);
    const state = now
      ? ` (the Vault's search is ${now.search}${now.index_turn ? `, its index turn ${now.index_turn}` : ""})`
      : "";
    throw new ToolError(
      `${place} is in the ${vault.name} Vault, but the index did not have it within ${seconds} seconds${state}; ` +
        "a Vault still building its first index takes longer, so pass a longer `--timeout`",
    );
  }
  console.log(`placed ${path.basename(vault.folder)}/${place}`);
  console.log(`slug   ${slug}`);
  console.log(
    `url    http://${DEV_HOST}:${frontendPort}/v/${vault.id}/n/${encodeURIComponent(slug)}`,
  );
}

if (process.argv[1] === import.meta.filename) {
  await runScrubbed(main, () => [devSetting(WEB_TOKEN_SETTING)]);
}
