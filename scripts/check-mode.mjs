#!/usr/bin/env node

// How much `just check` has to run, and whether it has to run at all (#710).
//
// Mode, from what the branch changed since its merge base on `development`:
//
// - docs-only: every change is documentation, which nothing the build
//   compiles or a Rust or frontend test reads. Only the script checks run.
// - manual: the rest of the changes are text edits to existing pages of the
//   bundled manual (`docs/user-vault`). The binary compiles those pages in and
//   Rust tests read their text, search ranking included, so the
//   tests of every module that reads the manual run, and the frontend's
//   help-link test. Lints, the other tests and the frontend build do not.
// - full: anything else, including adding, deleting or renaming a manual
//   page, and a branch with no change at all, since then there is nothing to
//   narrow it to.
//
// Pass record. After a pass, the recipe records the tree it passed on: the
// content of the worktree, committed or not, untracked files included and
// ignored ones left out. When `just check` runs again on that same tree, it
// reports the earlier pass and stops. Any edit gives a different tree, so it runs again.
// The record sits in `.dev/`, which Git ignores; deleting it costs one rerun.
//
//   check-mode.mjs start           exit 0 and print "<mode> <tree>" to run,
//                                  exit 10 when this tree already passed
//   check-mode.mjs record <mode> <tree>
//                                  remember a pass, unless the tree moved
//                                  while the checks ran
//   check-mode.mjs manual-tests rust|frontend
//                                  the tests manual mode runs, one per line

import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { git, gitFields, mergeBaseOf, resolveBase } from "./git-changes.mjs";

export const FULL = "full";
export const MANUAL_MODE = "manual";
export const DOCS_ONLY = "docs-only";

// The modules whose tests read the manual's text, as `cargo test` filters,
// and the frontend tests that do. `check-mode.test.mjs` fails when a file that
// names the manual is missing here.
export const MANUAL_RUST_TESTS = [
  "docs_bundle::",
  "usage_report::",
  "server::",
  "handlers::docs::",
  "handlers::spa::",
  "handlers::whats_new::",
  "mcp::config::",
  "mcp::docs_pointers::",
  "mcp::results::",
  "mcp::routes::",
  "mcp::tools::",
];
export const MANUAL_FRONTEND_TESTS = [
  "src/features/help/contextualLinks.test.ts",
];

const BASE = "development";
const MANUAL = "docs/user-vault/";
const RECORD = ".dev/check-pass.json";
// What `start` prints in place of a tree that cannot be hashed.
const UNHASHABLE = "none";

// The documentation, by path. The bundled manual is not on this list: the
// binary compiles it in and Rust and frontend tests read its text (see
// `decide`).
const DOCUMENTATION = [
  { prefix: "docs/", why: "documentation under docs/" },
  { prefix: "demo-vaults/", why: "demo Vault content" },
];

function isRootMarkdown(file) {
  return !file.includes("/") && file.toLowerCase().endsWith(".md");
}

// The mode one changed path needs, with the reason. `status` is Git's
// name-status letter against the merge base: M for an edit, A for a new file,
// D for a deleted one.
export function decide(file, status) {
  if (file.startsWith(MANUAL)) {
    return status === "M"
      ? {
          mode: MANUAL_MODE,
          reason: `${file} is a text edit to the bundled manual`,
        }
      : {
          mode: FULL,
          reason: `${file} adds or removes a page of the bundled manual`,
        };
  }
  if (isRootMarkdown(file)) {
    return { mode: DOCS_ONLY, reason: null };
  }
  if (DOCUMENTATION.some(({ prefix }) => file.startsWith(prefix))) {
    return { mode: DOCS_ONLY, reason: null };
  }
  return { mode: FULL, reason: `${file} is not documentation` };
}

const RANK = [DOCS_ONLY, MANUAL_MODE, FULL];

// The mode for a set of changes, `[{ file, status }]`: the widest any one of
// them needs, with the reason to print.
export function classify(changes) {
  if (changes.length === 0) {
    return { mode: FULL, reason: "nothing changed against the base" };
  }
  let widest = { mode: DOCS_ONLY, reason: null };
  for (const { file, status } of changes) {
    const decision = decide(file, status);
    if (RANK.indexOf(decision.mode) > RANK.indexOf(widest.mode)) {
      widest = decision;
    }
  }
  return {
    mode: widest.mode,
    reason:
      widest.reason ??
      `all ${changes.length} changed path(s) are documentation`,
  };
}

// Every path that differs between the merge base and the worktree, both
// sides of a rename included, plus untracked files. Renames are off so that a
// moved page counts where it left as well as where it arrived.
export function branchChanges(root, mergeBase) {
  const fields = gitFields(root, [
    "diff",
    "--name-status",
    "--no-renames",
    "-z",
    mergeBase,
  ]);
  const changes = [];
  for (let index = 0; index + 1 < fields.length; index += 2) {
    changes.push({ status: fields[index], file: fields[index + 1] });
  }
  for (const file of gitFields(root, [
    "ls-files",
    "--others",
    "--exclude-standard",
    "-z",
  ])) {
    changes.push({ status: "A", file });
  }
  return changes.sort((left, right) => left.file.localeCompare(right.file));
}

// The worktree's content as one tree hash, built in a throwaway index so the
// real one is never touched. The index starts empty rather than as a copy of
// the real one: a copy would carry `assume-unchanged` and `skip-worktree`
// flags, and Git does not rehash flagged files, so an edit to one would keep
// the old hash. Hashing from scratch takes a fraction of a second here.
//
// Returns null when the hash cannot stand for the content: an untracked
// folder holding its own Git repository is hashed by its commit, not its
// files, so edits inside it would go unseen. Git lists such a folder with a
// trailing slash, and only such a folder. A tree holding one is never
// remembered.
export function worktreeTree(root) {
  const nested = gitFields(root, [
    "ls-files",
    "--others",
    "--exclude-standard",
    "-z",
  ]).some((entry) => entry.endsWith("/"));
  if (nested) {
    return null;
  }
  const scratch = mkdtempSync(path.join(tmpdir(), "hatchdoor-check-tree-"));
  try {
    const env = { ...process.env, GIT_INDEX_FILE: path.join(scratch, "index") };
    gitOrThrow(root, ["add", "-A"], env);
    return gitOrThrow(root, ["write-tree"], env).stdout.trim();
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

function gitOrThrow(root, args, env) {
  const result = git(root, args, { env });
  if (result.status !== 0) {
    throw new Error(`git ${args.join(" ")} failed: ${result.stderr.trim()}`);
  }
  return result;
}

function readRecord(root) {
  try {
    return JSON.parse(readFileSync(path.join(root, RECORD), "utf8"));
  } catch {
    return null;
  }
}

// A pass covers its own mode and every narrower one.
export function covers(record, tree, mode) {
  return (
    record !== null &&
    typeof record === "object" &&
    record.tree === tree &&
    RANK.indexOf(record.mode) >= RANK.indexOf(mode) &&
    RANK.includes(mode)
  );
}

export function plan(root, base = BASE) {
  const resolved = resolveBase(root, base);
  const mergeBase = resolved === null ? null : mergeBaseOf(root, resolved);
  const decision =
    mergeBase === null
      ? { mode: FULL, reason: `no merge base with ${base}` }
      : classify(branchChanges(root, mergeBase));
  return { ...decision, tree: worktreeTree(root) };
}

function start(root) {
  const { mode, reason, tree } = plan(root);
  console.error(`check: ${mode} mode (${reason})`);
  const record = readRecord(root);
  if (tree !== null && covers(record, tree, mode)) {
    console.error(
      `check: this tree already passed in ${record.mode} mode at ${record.at}; nothing to run`,
    );
    return 10;
  }
  console.log(`${mode} ${tree ?? UNHASHABLE}`);
  return 0;
}

function record(root, mode, tree) {
  if (!RANK.includes(mode)) {
    console.error(`check-mode: unknown mode ${mode}`);
    return 2;
  }
  if (tree === UNHASHABLE) {
    console.error(
      "check: the worktree holds an untracked Git repository; this pass is not remembered",
    );
    return 0;
  }
  const now = worktreeTree(root);
  if (now !== tree) {
    console.error(
      "check: the worktree changed while the checks ran; this pass is not remembered",
    );
    return 0;
  }
  mkdirSync(path.join(root, path.dirname(RECORD)), { recursive: true });
  writeFileSync(
    path.join(root, RECORD),
    `${JSON.stringify({ tree, mode, at: new Date().toISOString() })}\n`,
  );
  console.error(
    `check: passed in ${mode} mode; a rerun on this tree returns at once`,
  );
  return 0;
}

function main(argv) {
  const root = git(process.cwd(), [
    "rev-parse",
    "--show-toplevel",
  ]).stdout.trim();
  const [command, ...rest] = argv;
  if (command === "start" && rest.length === 0) {
    return start(root);
  }
  if (command === "record" && rest.length === 2) {
    return record(root, rest[0], rest[1]);
  }
  if (command === "manual-tests" && rest.length === 1) {
    const list = { rust: MANUAL_RUST_TESTS, frontend: MANUAL_FRONTEND_TESTS }[
      rest[0]
    ];
    if (list !== undefined) {
      console.log(list.join("\n"));
      return 0;
    }
  }
  console.error(
    "Usage: check-mode.mjs start | record <mode> <tree> | manual-tests rust|frontend",
  );
  return 2;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exitCode = main(process.argv.slice(2));
}
