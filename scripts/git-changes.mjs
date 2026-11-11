// Git helpers for a branch's range: resolving its base, finding the merge
// base, and the paths that differ from it, committed or not. The
// docs-freshness gate and `just check`'s mode detection share the base and
// merge-base lookup, so both measure from the same commit. Their path
// listings differ on purpose: the gate wants each path once, under its new
// name (`changedPaths`), while the mode needs Git's status letter for both
// sides of a rename (`branchChanges` in check-mode.mjs).

import { spawnSync } from "node:child_process";

export function git(root, args, options = {}) {
  const result = spawnSync("git", args, {
    cwd: root,
    encoding: "utf8",
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  return result;
}

// Every path this module reads comes from a NUL-separated `-z` listing. The
// default line-based output is not usable here: `git status --porcelain`
// c-quotes any path containing a space, and nearly every note under
// `docs/user-vault` has spaces in its name, while `git diff --name-only`
// leaves those same paths bare and octal-escapes non-ASCII instead. Parsing
// both as lines made one file arrive under two spellings, and made a note
// edited but not yet committed look untouched.
export function gitFields(root, args) {
  const result = git(root, args);
  if (result.status !== 0) {
    throw new Error(
      `git ${args.join(" ")} failed: ${result.stderr.trim() || "unknown error"}`,
    );
  }
  return result.stdout.split("\0").filter((field) => field !== "");
}

// The requested ref, or its `origin/` copy when there is no local branch.
export function resolveBase(root, requested) {
  for (const candidate of [requested, `origin/${requested}`]) {
    const result = git(root, [
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

export function mergeBaseOf(root, base) {
  const result = git(root, ["merge-base", base, "HEAD"]);
  return result.status === 0 ? result.stdout.trim() : null;
}

// Each `-z` status record is `XY <path>`. A rename or copy adds the original
// path as its own following field, which describes where the content came
// from rather than what changed now, so it is skipped. `-uall` expands
// untracked directories, which the default output collapses to a bare
// `newdir/` that would match no prefix.
export function workingTreePaths(root) {
  const fields = gitFields(root, ["status", "--porcelain", "-z", "-uall"]);
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

export function changedPaths(root, mergeBase) {
  return [
    ...new Set([
      ...gitFields(root, ["diff", "--name-only", "-z", mergeBase, "HEAD"]),
      ...workingTreePaths(root),
    ]),
  ].sort();
}
