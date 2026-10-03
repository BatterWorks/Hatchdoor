#!/usr/bin/env node

// Documentation freshness trigger for merges into `development`.
//
// This script does not judge whether the user documentation is correct: only a
// reader can tell whether a note still reads true. What it does is refuse to
// let a branch that moved a user-facing surface reach `development` without
// someone having looked. It reports which surfaces the branch touched, which
// notes in `docs/user-vault` claim to document them, and whether those notes
// moved in the same range.
//
// Run it, read the notes it names, fix what drifted, then re-run with
// `--acknowledge` to record that the review happened. Acknowledging without
// reading defeats the only thing this gate does.
//
// It also refuses a branch that changes code Hatchdoor ships without editing
// CHANGELOG.md. About forty merges after v2.6.1 went in without an entry and
// the gap only surfaced when someone asked. `--acknowledge` does not waive
// this half: a branch whose change nobody outside the code can notice says so
// with a `Changelog: none, <reason>` trailer on one of its commits, so the
// reason stays in history.

import { spawnSync } from "node:child_process";
import { lstat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

const VAULT_ROOT = "docs/user-vault";

const GET_STARTED = `${VAULT_ROOT}/01 Get started`;
const GUIDES = `${VAULT_ROOT}/02 Guides`;
const REFERENCE = `${VAULT_ROOT}/03 Reference`;
const CONCEPTS = `${VAULT_ROOT}/04 Concepts`;

// Each surface names the source paths that produce a user-facing behaviour and
// the notes that describe it. A changed file may match several surfaces; that
// is intended, because one change can invalidate several notes.
const SURFACES = [
  {
    id: "mcp-tools",
    label: "MCP tool catalogue and protocol",
    paths: ["src/mcp/"],
    notes: [
      `${REFERENCE}/MCP tools reference.md`,
      `${GET_STARTED}/Connect your agent.md`,
      `${GET_STARTED}/Search and change notes with your agent.md`,
    ],
  },
  {
    id: "http-api",
    label: "HTTP endpoints and API payloads",
    paths: ["src/handlers/", "src/server.rs", "src/api_types.rs"],
    notes: [`${REFERENCE}/HTTP API reference.md`],
  },
  {
    id: "settings",
    label: "Settings and environment variables",
    paths: [
      "src/runtime_config.rs",
      "src/config.rs",
      "src/handlers/settings.rs",
      ".env.example",
    ],
    notes: [
      `${REFERENCE}/Settings and environment variables reference.md`,
      `${GET_STARTED}/Install Hatchdoor with Docker Compose.md`,
    ],
  },
  {
    id: "git-vaults",
    label: "Git-backed Vault behaviour",
    paths: ["src/git/"],
    notes: [
      `${GUIDES}/How to set up a Git-backed Vault.md`,
      `${CONCEPTS}/Vault lifecycle states.md`,
      `${GUIDES}/How to troubleshoot common problems.md`,
    ],
  },
  {
    id: "vault-lifecycle",
    label: "Vault registry, lifecycle, and watching",
    paths: [
      "src/vault_registry.rs",
      "src/vault_registry/",
      "src/vault_management.rs",
      "src/vault_migration.rs",
      "src/vault_runtime.rs",
      "src/vault_runtime/",
      "src/vault_watcher.rs",
    ],
    notes: [
      `${GUIDES}/How to manage multiple Vaults.md`,
      `${GET_STARTED}/Connect your first Vault.md`,
      `${CONCEPTS}/Vault lifecycle states.md`,
    ],
  },
  {
    id: "search-indexing",
    label: "Search, embedding, indexing, and model setup",
    paths: [
      "src/search/",
      "src/embed/",
      "src/chunk/",
      "src/cache/",
      "src/rerank/",
      "src/model_setup.rs",
      "src/vault/exclude.rs",
    ],
    notes: [
      `${CONCEPTS}/How indexing and search work.md`,
      `${GET_STARTED}/Search and change notes with your agent.md`,
      `${GUIDES}/How to troubleshoot common problems.md`,
    ],
  },
  {
    id: "layers",
    label: "The layer system",
    paths: ["src/vault/layers.rs", "src/search/layer_selection.rs"],
    notes: [
      `${CONCEPTS}/The layer system.md`,
      `${GUIDES}/How to organize a Vault with layers.md`,
      `${GUIDES}/How to run an LLM wiki in Hatchdoor.md`,
    ],
  },
  {
    id: "attachments",
    label: "Attachment import, serving, and mutation",
    // The mutation half matters as much as import and serving: the guide
    // documents move_attachment, rename_attachment, and delete_attachment,
    // and without this path a change to them summoned only the note-mutation
    // reading list (#220).
    paths: [
      "src/vault_read/assets.rs",
      "src/handlers/assets.rs",
      "src/vault/write/attachments.rs",
    ],
    notes: [`${GUIDES}/How to import and work with attachments.md`],
  },
  {
    id: "markdown",
    label: "Markdown parsing, links, and note paths",
    paths: [
      "src/vault/links.rs",
      "src/vault/markdown_links.rs",
      "src/vault/paths.rs",
      "src/cache/parse.rs",
    ],
    notes: [`${REFERENCE}/Supported Markdown reference.md`],
  },
  {
    id: "link-inserts",
    label: "Links and embeds Hatchdoor inserts, in the Vault's link style",
    // ADR-33: the style is read on the server and followed by the editor's
    // autocomplete and attachment inserts, which three notes describe.
    paths: [
      "src/vault/link_style.rs",
      "frontend/src/components/note-page/autocomplete.ts",
      "frontend/src/components/note-page/attachmentDrop.ts",
      "frontend/src/components/note-page/linkStyle.ts",
    ],
    notes: [
      `${REFERENCE}/Supported Markdown reference.md`,
      `${GUIDES}/How to import and work with attachments.md`,
      `${GUIDES}/How to edit notes with the live editor.md`,
    ],
  },
  {
    id: "vault-shape",
    label: "The Vault shape list_vaults and GET /api/v1/vaults return",
    paths: ["src/vault_management.rs"],
    notes: [
      `${REFERENCE}/HTTP API reference.md`,
      `${REFERENCE}/MCP tools reference.md`,
    ],
  },
  {
    id: "write-mutations",
    label: "Note mutation behaviour",
    paths: ["src/vault_mutation.rs", "src/vault/write.rs", "src/vault/write/"],
    notes: [
      `${GET_STARTED}/Search and change notes with your agent.md`,
      `${REFERENCE}/MCP tools reference.md`,
    ],
  },
  {
    id: "security",
    label: "Authentication and the security model",
    paths: ["src/auth.rs", "src/mcp/auth.rs"],
    notes: [
      `${CONCEPTS}/The security model.md`,
      `${GET_STARTED}/Understand where your data lives.md`,
    ],
  },
  {
    id: "web-ui",
    label: "Web UI",
    paths: ["frontend/src/"],
    notes: [
      `${GET_STARTED}/Browse and review through the Web UI.md`,
      `${GUIDES}/How to edit notes with the live editor.md`,
    ],
  },
  {
    id: "startup",
    label: "Startup, readiness, and note reads",
    paths: [
      "src/startup.rs",
      "src/vault_read.rs",
      "src/vault_work.rs",
      "src/vault_executor.rs",
    ],
    notes: [
      `${GUIDES}/How to troubleshoot common problems.md`,
      `${CONCEPTS}/Vault lifecycle states.md`,
    ],
  },
  {
    id: "deployment",
    label: "Deployment and packaging",
    paths: ["Dockerfile", "docker-compose.yml", ".env.example"],
    notes: [
      `${GET_STARTED}/Install Hatchdoor with Docker Compose.md`,
      `${GUIDES}/How to deploy Hatchdoor with an agent.md`,
      `${GET_STARTED}/Understand where your data lives.md`,
    ],
  },
];

const CHANGELOG = "CHANGELOG.md";

// What ships in the image or reaches an operator's deployment. A change here
// needs a changelog entry unless a trailer says why not.
const SHIPPED_PATHS = [
  "src/",
  "frontend/src/",
  "frontend/index.html",
  "frontend/public/",
  "frontend/package.json",
  "Cargo.toml",
  "Dockerfile",
  "docker-compose.yml",
  ".env.example",
];

// Test code sits inside the shipped trees but never ships.
function isTestFile(file) {
  return (
    /\.test\.[cm]?[jt]sx?$/.test(file) ||
    file.startsWith("frontend/src/test/") ||
    /(^|\/)tests\.rs$/.test(file) ||
    /(^|\/)tests\//.test(file)
  );
}

function shippedChanges(files) {
  return files.filter(
    (file) =>
      SHIPPED_PATHS.some((prefix) => matches(file, prefix)) &&
      !isTestFile(file),
  );
}

// The `Changelog:` trailers on the branch's own commits. Only committed work
// can carry one, so a waiver lands with the commit it explains.
function changelogWaivers(mergeBase) {
  const result = git([
    "log",
    "--format=%(trailers:key=Changelog,valueonly,separator=%x00)%x00",
    `${mergeBase}..HEAD`,
  ]);
  if (result.status !== 0) {
    return [];
  }
  return result.stdout
    .split("\0")
    .map((value) => value.trim())
    .filter((value) => value !== "");
}

function git(args) {
  const result = spawnSync("git", args, {
    cwd: repositoryRoot,
    encoding: "utf8",
  });
  if (result.error) {
    throw result.error;
  }
  return result;
}

// Every path this script reads comes from a NUL-separated `-z` listing. The
// default line-based output is not usable here: `git status --porcelain`
// c-quotes any path containing a space, and nearly every note under
// `docs/user-vault` has spaces in its name, while `git diff --name-only`
// leaves those same paths bare and octal-escapes non-ASCII instead. Parsing
// both as lines made one file arrive under two spellings, and made a note
// edited but not yet committed look untouched.
function gitFields(args) {
  const result = git(args);
  if (result.status !== 0) {
    throw new Error(
      `git ${args.join(" ")} failed: ${result.stderr.trim() || "unknown error"}`,
    );
  }
  return result.stdout.split("\0").filter((field) => field !== "");
}

function resolveBase(requested) {
  for (const candidate of [requested, `origin/${requested}`]) {
    const result = git([
      "rev-parse",
      "--verify",
      "--quiet",
      `${candidate}^{commit}`,
    ]);
    if (result.status === 0) {
      return candidate;
    }
  }
  return null;
}

// Each `-z` status record is `XY <path>`. A rename or copy adds the original
// path as its own following field, which describes where the content came
// from rather than what changed now, so it is skipped. `-uall` expands
// untracked directories, which the default output collapses to a bare
// `newdir/` that would match no surface prefix.
function workingTreePaths() {
  const fields = gitFields(["status", "--porcelain", "-z", "-uall"]);
  const paths = [];

  for (let index = 0; index < fields.length; index += 1) {
    const record = fields[index];
    const status = record.slice(0, 2);
    paths.push(record.slice(3));
    if (status.includes("R") || status.includes("C")) {
      index += 1;
    }
  }

  return paths;
}

function changedPaths(mergeBase) {
  return [
    ...new Set([
      ...gitFields(["diff", "--name-only", "-z", mergeBase, "HEAD"]),
      ...workingTreePaths(),
    ]),
  ].sort();
}

function matches(file, prefix) {
  return prefix.endsWith("/") ? file.startsWith(prefix) : file === prefix;
}

async function missing(paths) {
  const absent = [];
  await Promise.all(
    paths.map(async (candidate) => {
      try {
        const stat = await lstat(path.join(repositoryRoot, candidate));
        if (!stat.isFile()) {
          absent.push(candidate);
        }
      } catch {
        absent.push(candidate);
      }
    }),
  );
  return absent.sort();
}

// A rule pointing at something that no longer exists is worse than no rule:
// the gate still exits non-zero, so it looks like it worked while naming a
// note nobody can open. Source prefixes rot the other way — a renamed module
// silently stops matching anything. Checking every entry needs the whole
// repository present, so it runs under --validate-table rather than on every
// invocation; the reading list a run actually prints is checked inline.
async function deadTableEntries() {
  const expectations = [];
  for (const surface of SURFACES) {
    for (const prefix of surface.paths) {
      expectations.push({
        surface: surface.id,
        kind: prefix.endsWith("/") ? "directory" : "file",
        value: prefix,
      });
    }
    for (const note of surface.notes) {
      expectations.push({ surface: surface.id, kind: "note", value: note });
    }
  }

  const dead = [];
  await Promise.all(
    expectations.map(async ({ surface, kind, value }) => {
      const target = path.join(repositoryRoot, value.replace(/\/$/, ""));
      try {
        const stat = await lstat(target);
        const wrongType =
          kind === "directory" ? !stat.isDirectory() : !stat.isFile();
        if (wrongType) {
          dead.push(`${value} (${surface}: not a ${kind})`);
        }
      } catch {
        dead.push(`${value} (${surface}: missing ${kind})`);
      }
    }),
  );
  return [...new Set(dead)].sort();
}

function touchedSurfaces(files) {
  return SURFACES.map((surface) => ({
    ...surface,
    changedFiles: files.filter((file) =>
      surface.paths.some((prefix) => matches(file, prefix)),
    ),
  })).filter((surface) => surface.changedFiles.length > 0);
}

const DEFAULT_BASE = "development";

function usage(message) {
  console.error(`Documentation freshness check failed: ${message}`);
  console.error(
    "Usage: check-docs-freshness.mjs [--base <ref>] [--acknowledge]",
  );
  process.exit(2);
}

const argv = process.argv.slice(2);
const acknowledged = argv.includes("--acknowledge");
const validateTable = argv.includes("--validate-table");
const baseIndex = argv.indexOf("--base");

// `--base` with nothing after it, or with the next flag as its value, used to
// fall through to the default and check a ref the caller never asked for.
if (baseIndex !== -1) {
  const value = argv[baseIndex + 1];
  if (value === undefined || value.startsWith("--")) {
    usage("--base needs a ref.");
  }
}
const requestedBase = baseIndex === -1 ? DEFAULT_BASE : argv[baseIndex + 1];

for (const argument of argv) {
  if (
    argument !== "--acknowledge" &&
    argument !== "--validate-table" &&
    argument !== "--base" &&
    argument !== requestedBase
  ) {
    usage(`unknown argument ${argument}.`);
  }
}

function reportDeadEntries(entries) {
  console.error(
    "Documentation freshness check failed: the surface table points at paths that do not exist.",
  );
  console.error("\nDEAD TABLE ENTRIES");
  for (const entry of entries) {
    console.error(`  ${entry}`);
  }
  console.error(
    "\nFix the table in scripts/check-docs-freshness.mjs. A rule that matches\nnothing silently stops guarding the surface it names.",
  );
}

if (validateTable) {
  const dead = await deadTableEntries();
  if (dead.length > 0) {
    reportDeadEntries(dead);
    process.exit(2);
  }
  const surfaceCount = SURFACES.length;
  const noteCount = new Set(SURFACES.flatMap((surface) => surface.notes)).size;
  console.log(
    `Surface table OK: ${surfaceCount} surfaces and ${noteCount} notes all resolve.`,
  );
  process.exit(0);
}

const base = resolveBase(requestedBase);
if (base === null) {
  console.error(
    `Documentation freshness check failed: no ref named ${requestedBase} or origin/${requestedBase}.`,
  );
  console.error("Pass an existing ref with --base <ref>.");
  process.exit(2);
}

const mergeBaseResult = git(["merge-base", base, "HEAD"]);
if (mergeBaseResult.status !== 0) {
  console.error(
    `Documentation freshness check failed: no merge base between ${base} and HEAD.`,
  );
  process.exit(2);
}
const mergeBase = mergeBaseResult.stdout.trim();

const files = changedPaths(mergeBase);
const surfaces = touchedSurfaces(files);
const changedNotes = new Set(
  files.filter((file) => file.startsWith(`${VAULT_ROOT}/`)),
);

const shipped = shippedChanges(files);
const changelogEdited = files.includes(CHANGELOG);
const waivers = changelogWaivers(mergeBase);
const changelogMissing =
  shipped.length > 0 && !changelogEdited && waivers.length === 0;

const SHIPPED_LISTED = 10;

function reportChangelog() {
  if (shipped.length === 0) {
    return;
  }
  if (changelogEdited) {
    console.error(`\nCHANGELOG\n  ${CHANGELOG} is edited on this branch.`);
    return;
  }
  if (waivers.length > 0) {
    console.error(`\nCHANGELOG\n  No entry, by trailer: ${waivers.join("; ")}`);
    return;
  }
  console.error(
    `\nCHANGELOG ENTRY MISSING\n  This branch changes code Hatchdoor ships and does not edit ${CHANGELOG}:`,
  );
  for (const file of shipped.slice(0, SHIPPED_LISTED)) {
    console.error(`    ${file}`);
  }
  if (shipped.length > SHIPPED_LISTED) {
    console.error(`    and ${shipped.length - SHIPPED_LISTED} more`);
  }
  console.error(`
  Add an entry under "## Unreleased" for anything a user, operator or agent
  can notice. If nothing here is noticeable (a test-only change, an internal
  refactor), say so in a commit on this branch with a trailer:

    Changelog: none, <reason>`);
}

if (surfaces.length === 0) {
  if (changelogMissing) {
    console.error(
      `Documentation freshness check failed: no user-facing surface changed since ${base} (${mergeBase.slice(0, 9)}), but the changelog needs an entry.`,
    );
    reportChangelog();
    process.exit(1);
  }
  console.log(
    `Documentation freshness OK: no user-facing surface changed since ${base} (${mergeBase.slice(0, 9)}).`,
  );
  process.exit(0);
}

const notesToReview = new Map();
for (const surface of surfaces) {
  for (const note of surface.notes) {
    const reasons = notesToReview.get(note) ?? [];
    reasons.push(surface.label);
    notesToReview.set(note, reasons);
  }
}

// Never hand over a reading list containing a file that cannot be opened.
const absentNotes = await missing([...notesToReview.keys()]);
if (absentNotes.length > 0) {
  reportDeadEntries(absentNotes.map((note) => `${note} (missing note)`));
  process.exit(2);
}

console.error(
  `Documentation freshness review required: ${surfaces.length} user-facing surface(s) changed since ${base} (${mergeBase.slice(0, 9)}).`,
);

console.error("\nCHANGED SURFACES");
for (const surface of surfaces) {
  console.error(`  ${surface.label} [${surface.id}]`);
  for (const file of surface.changedFiles) {
    console.error(`    ${file}`);
  }
}

console.error("\nNOTES THAT DOCUMENT THEM");
for (const [note, reasons] of [...notesToReview].sort(([left], [right]) =>
  left.localeCompare(right),
)) {
  const state = changedNotes.has(note) ? "edited on this branch" : "UNTOUCHED";
  console.error(`  ${note}  (${state})`);
  console.error(`    covers: ${[...new Set(reasons)].join("; ")}`);
}

reportChangelog();

if (acknowledged) {
  console.error(
    `\nAcknowledged: ${notesToReview.size} note(s) reviewed against ${surfaces.length} changed surface(s).`,
  );
  if (changelogMissing) {
    console.error(
      "The changelog entry is still missing. Acknowledging the notes does not waive it.",
    );
    process.exit(1);
  }
  process.exit(0);
}

// The hint has to carry the base it was actually run with, or following it
// verbatim acknowledges a different range than the one just reported.
const ackCommand =
  requestedBase === DEFAULT_BASE
    ? "just docs-freshness-ack"
    : `just docs-freshness-ack '${requestedBase}'`;

console.error(
  `
Read each note above and confirm it still describes what this branch does.
"Edited on this branch" means the file moved, not that it is correct.
Update whatever drifted, then re-run with --acknowledge to record the review:

  ${ackCommand}
`,
);
process.exitCode = 1;
