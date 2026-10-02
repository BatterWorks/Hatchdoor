#!/usr/bin/env node

// `just release-prepare <version>`: the first of the two release commands
// (ADR-36). It opens pull requests and a draft release, and stops. It never
// merges anything, never pushes to `development` or `main`, and never records
// the documentation review on anyone's behalf.
//
// The first run cuts `docs/release-v<version>` off `development`, sets the
// version in the four version files, dates the changelog's Unreleased
// section, runs the mechanical checks, and opens the version-bump pull
// request into `development`. Once that has merged, the next run opens the
// `development` to `main` release pull request, carrying the judgment
// checklist, and creates a draft GitHub Release from the changelog section.
//
// Every run works out where the release stands from git and GitHub rather
// than from local notes, so re-running after any step, or after a crash,
// resumes at the first step not yet done instead of repeating one.

import { spawnSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  VERSION_FILES,
  compareVersions,
  hasEntries,
  latestReleaseVersion,
  parseVersion,
  releaseHeadingPrefix,
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

const REMOTE = "origin";
const DEVELOPMENT = "development";
const MAIN = "main";

class Refusal extends Error {}

function refuse(check, detail) {
  throw new Refusal(`release-prepare refused (${check}): ${detail}`);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    encoding: "utf8",
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  return result;
}

function checked(command, args) {
  const result = run(command, args);
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed: ${result.stderr.trim() || result.stdout.trim() || `exit ${result.status}`}`,
    );
  }
  return result.stdout.trim();
}

const git = (...args) => checked("git", args);
const gh = (...args) => checked("gh", args);

function refExists(ref) {
  return run("git", ["rev-parse", "--verify", "--quiet", ref]).status === 0;
}

function remoteBranch(branch) {
  return `refs/remotes/${REMOTE}/${branch}`;
}

function developmentChangelog() {
  return git("show", `${REMOTE}/${DEVELOPMENT}:CHANGELOG.md`);
}

function today() {
  const now = new Date();
  const pad = (value) => String(value).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

async function readRepositoryFile(file) {
  return readFile(path.join(repositoryRoot, file), "utf8");
}

async function readVersionFiles() {
  const files = {};
  for (const file of VERSION_FILES) {
    files[file] = await readRepositoryFile(file);
  }
  return files;
}

// ---------------------------------------------------------------------------
// Step 1: refuse before touching anything

function requireCleanTree() {
  const status = git("status", "--porcelain", "--untracked-files=all");
  if (status !== "") {
    refuse(
      "clean working tree",
      `commit, stash or remove these first:\n${status}`,
    );
  }
}

function requireNewVersion(version) {
  const tags = git("tag", "--list", "v*").split("\n").filter(Boolean);
  const latest = latestReleaseVersion(tags);
  if (latest !== null && compareVersions(version, latest) <= 0) {
    refuse(
      "version",
      `${version} is not higher than the latest release tag, v${latest}.`,
    );
  }
}

function requireUnreleasedEntries() {
  const section = unreleasedSection(developmentChangelog());
  if (!section || !hasEntries(section.body)) {
    refuse(
      "changelog",
      `"## Unreleased" in CHANGELOG.md on ${DEVELOPMENT} has no entries, so there is nothing to release.`,
    );
  }
}

// ---------------------------------------------------------------------------
// Where the release stands on GitHub

function existingRelease(tag) {
  const result = run("gh", ["release", "view", tag, "--json", "isDraft,url"]);
  if (result.status === 0) {
    return JSON.parse(result.stdout);
  }
  if (/release not found/i.test(result.stderr)) {
    return null;
  }
  throw new Error(`gh release view ${tag} failed: ${result.stderr.trim()}`);
}

// A merged pull request wins over an open one, and an open one over a closed
// one, so a branch reused after a rejected attempt still resolves.
function bumpPullRequest(branch) {
  const pulls = JSON.parse(
    gh(
      "pr",
      "list",
      "--head",
      branch,
      "--base",
      DEVELOPMENT,
      "--state",
      "all",
      "--json",
      "number,url,state",
    ),
  );
  for (const state of ["MERGED", "OPEN", "CLOSED"]) {
    const pull = pulls.find((candidate) => candidate.state === state);
    if (pull) {
      return pull;
    }
  }
  return null;
}

// GitHub allows one open pull request per head and base, so an open one made
// for another version would block this release rather than stand in for it.
function openReleasePullRequest(version) {
  const pulls = JSON.parse(
    gh(
      "pr",
      "list",
      "--head",
      DEVELOPMENT,
      "--base",
      MAIN,
      "--state",
      "open",
      "--json",
      "number,url,title",
    ),
  );
  const pull = pulls[0] ?? null;
  if (pull && pull.title !== `Release v${version}`) {
    refuse(
      "release pull request",
      `${pull.url} is already open from ${DEVELOPMENT} into ${MAIN} as "${pull.title}", not "Release v${version}". Close it or finish that release first.`,
    );
  }
  return pull;
}

// ---------------------------------------------------------------------------
// Steps 2 to 4: the version-bump pull request

function switchToBumpBranch(branch) {
  if (refExists(`refs/heads/${branch}`)) {
    git("switch", branch);
    console.log(`Resuming on ${branch}.`);
    return;
  }
  if (refExists(remoteBranch(branch))) {
    git("switch", "--create", branch, "--track", `${REMOTE}/${branch}`);
    console.log(`Resuming on ${branch}, checked out from ${REMOTE}.`);
    return;
  }
  requireUnreleasedEntries();
  git("switch", "--create", branch, "--no-track", `${REMOTE}/${DEVELOPMENT}`);
  console.log(`Cut ${branch} from ${REMOTE}/${DEVELOPMENT}.`);
}

// The bump is made once. When the branch already has the dated section, the
// bump commit exists and nothing is rewritten: a version file edited after it
// is for checkBump to judge, not for this to overwrite. The tree was clean
// when the run started, so a crash before the commit leaves edits that the
// clean-tree check refuses on the next run.
async function applyBump(version) {
  const changelog = await readRepositoryFile("CHANGELOG.md");
  if (releaseSection(changelog, version)) {
    return;
  }
  if (!unreleasedSection(changelog)) {
    refuse(
      "changelog",
      `CHANGELOG.md has neither "## Unreleased" nor a "${releaseHeadingPrefix(version)}" section.`,
    );
  }

  const files = await readVersionFiles();
  let bumped;
  try {
    bumped = setVersions(files, version);
  } catch (error) {
    refuse("version files", error.message);
  }
  for (const file of VERSION_FILES) {
    if (bumped[file] !== files[file]) {
      await writeFile(path.join(repositoryRoot, file), bumped[file]);
    }
  }
  await writeFile(
    path.join(repositoryRoot, "CHANGELOG.md"),
    renameUnreleased(changelog, version, today()),
  );

  if (git("status", "--porcelain") === "") {
    return;
  }
  git("add", "--", ...VERSION_FILES, "CHANGELOG.md");
  git(
    "commit",
    "--quiet",
    "-m",
    `docs: prepare the v${version} release`,
    "-m",
    `Sets the version to ${version} in ${VERSION_FILES.join(", ")}, and dates the changelog's Unreleased section as v${version}. Made by \`just release-prepare ${version}\`.`,
  );
  console.log(`Committed the version bump to ${version}.`);
}

// Step 3. The cheap checks run first so a typo does not cost a full test run.
// The branch is pushed only after `just check-full` passes, so when the remote
// branch already holds this exact commit, that run is not repeated.
async function checkBump(version, branch) {
  const disagreements = versionDisagreements(await readVersionFiles(), version);
  if (disagreements.length > 0) {
    refuse(
      "version files",
      `these do not say ${version}: ${disagreements.join(", ")}.`,
    );
  }

  const changelog = await readRepositoryFile("CHANGELOG.md");
  const section = releaseSection(changelog, version);
  if (!section || !hasEntries(section.body)) {
    refuse(
      "changelog",
      `the "${releaseHeadingPrefix(version)}" section of CHANGELOG.md has no entries.`,
    );
  }
  const missing = undefinedReferences(changelog, section.body);
  if (missing.length > 0) {
    refuse(
      "changelog links",
      `the v${version} section references ${missing.join(", ")} with no link definition. Add a "[#N]: <url>" line for each above the first release heading.`,
    );
  }

  if (isPushed(branch)) {
    console.log(
      `${branch} is already pushed at this commit, which passed \`just check-full\` before the push.`,
    );
    return;
  }

  console.log(
    "\nRunning `just check-full`: every backend test in both feature configurations, the model tests and the frontend checks. A cold build takes tens of minutes; its output follows.\n",
  );
  const result = spawnSync("just", ["check-full"], {
    cwd: repositoryRoot,
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    refuse(
      "just check-full",
      `it exited with status ${result.status}. Fix the failure on ${branch}, commit it, and run this again.`,
    );
  }
}

function isPushed(branch) {
  return (
    refExists(remoteBranch(branch)) &&
    git("rev-parse", remoteBranch(branch)) === git("rev-parse", "HEAD")
  );
}

function pushBranch(branch) {
  if (isPushed(branch)) {
    return;
  }
  git("push", "--quiet", "--set-upstream", REMOTE, branch);
  console.log(`Pushed ${branch}.`);
}

async function prepareBump(version, branch) {
  switchToBumpBranch(branch);
  await applyBump(version);
  await checkBump(version, branch);
  pushBranch(branch);
  const url = gh(
    "pr",
    "create",
    "--base",
    DEVELOPMENT,
    "--head",
    branch,
    "--title",
    `Prepare the v${version} release`,
    "--body",
    `Sets the version to ${version} in ${VERSION_FILES.map((file) => `\`${file}\``).join(", ")}, and renames the changelog's \`## Unreleased\` section to \`## v${version} - <date>\`.

The version files agree, every \`[#N]\` in the section has a link definition, and \`just check-full\` passed on this branch.

Made by \`just release-prepare ${version}\`. Once this has merged, run it again to open the release pull request into \`${MAIN}\` and the draft GitHub Release.`,
  );
  console.log(`
Opened the version-bump pull request: ${url}

Merge it into ${DEVELOPMENT}, then run \`just release-prepare ${version}\` again.`);
}

// ---------------------------------------------------------------------------
// Step 5: the release pull request and the draft release

// `just docs-freshness main` judges the checkout, so it runs on the tip of
// `development` that the release pull request merges, then returns to where
// the run started. It compares against the local `main` when one exists, so
// that is brought up to date first; otherwise a stale `main` widens the
// reading list past what this release touched.
//
// The script exits 1 both when it hands over a reading list and when the
// changelog check fails, so only the first counts as a result to paste.
function docsFreshness() {
  const branch = git("rev-parse", "--abbrev-ref", "HEAD");
  const start = branch === "HEAD" ? git("rev-parse", "HEAD") : branch;
  git("switch", "--quiet", "--detach", `${REMOTE}/${DEVELOPMENT}`);
  let update;
  let result;
  try {
    update = run("git", ["fetch", "--quiet", REMOTE, `${MAIN}:${MAIN}`]);
    if (update.status === 0) {
      result = run("sh", ["-c", `just docs-freshness ${MAIN} 2>&1`]);
    }
  } finally {
    if (branch === "HEAD") {
      git("switch", "--quiet", "--detach", start);
    } else {
      git("switch", "--quiet", start);
    }
  }
  if (update.status !== 0) {
    refuse(
      "docs-freshness",
      `could not fast-forward the local ${MAIN} to ${REMOTE}/${MAIN}: ${update.stderr.trim()}`,
    );
  }
  const handedList =
    result.status === 1 &&
    result.stdout.includes("Documentation freshness review required");
  if (result.status !== 0 && !handedList) {
    refuse(
      "docs-freshness",
      `\`just docs-freshness ${MAIN}\` exited with status ${result.status}:\n${result.stdout.trim()}`,
    );
  }
  return result.stdout;
}

function releasePullRequestBody(version, freshness) {
  return `Releases v${version}: merges \`${DEVELOPMENT}\` into \`${MAIN}\`.

\`just release-publish ${version}\` merges this pull request, and refuses while any box below is unticked. Tick each box only after doing the check it names. Publishing waits for the maintainer to approve the release title and notes.

## Release checklist

${renderChecklist(freshness)}
`;
}

async function prepareRelease(version, release) {
  const changelog = developmentChangelog();
  if (!releaseSection(changelog, version)) {
    refuse(
      "changelog",
      `the version-bump pull request has merged, but CHANGELOG.md on ${DEVELOPMENT} has no "${releaseHeadingPrefix(version)}" section.`,
    );
  }

  let pullUrl;
  const existing = openReleasePullRequest(version);
  if (existing) {
    pullUrl = existing.url;
    console.log(`The release pull request is already open: ${pullUrl}`);
  } else {
    const freshness = docsFreshness();
    pullUrl = gh(
      "pr",
      "create",
      "--base",
      MAIN,
      "--head",
      DEVELOPMENT,
      "--title",
      `Release v${version}`,
      "--body",
      releasePullRequestBody(version, freshness),
    );
    console.log(`Opened the release pull request: ${pullUrl}`);
  }

  const tag = `v${version}`;
  let releaseUrl;
  if (release) {
    releaseUrl = release.url;
    console.log(`The draft release already exists: ${releaseUrl}`);
  } else {
    releaseUrl = gh(
      "release",
      "create",
      tag,
      "--draft",
      "--target",
      MAIN,
      "--title",
      tag,
      "--notes",
      releaseNotes(changelog, version),
    );
    console.log(`Created the draft release: ${releaseUrl}`);
  }

  console.log(`
Next: work through the checklist on ${pullUrl}, draft the release title and notes on ${releaseUrl}, and give them to the maintainer to approve. \`just release-publish ${version}\` runs only after that approval.`);
}

// ---------------------------------------------------------------------------

function usage(message) {
  console.error(`release-prepare: ${message}`);
  console.error("Usage: just release-prepare <version>   (for example 2.7.0)");
  process.exit(2);
}

async function main() {
  const argv = process.argv.slice(2);
  if (argv.length !== 1) {
    usage("expected exactly one argument, the version.");
  }
  const [version] = argv;
  if (!parseVersion(version)) {
    refuse(
      "version",
      `"${version}" is not a bare semantic version such as 2.7.0.`,
    );
  }
  const branch = `docs/release-v${version}`;
  const tag = `v${version}`;

  requireCleanTree();
  git("fetch", "--quiet", "--prune", "--tags", REMOTE);
  requireNewVersion(version);

  const release = existingRelease(tag);
  if (release && !release.isDraft) {
    refuse("version", `${tag} is already published: ${release.url}`);
  }

  const bump = bumpPullRequest(branch);
  if (bump?.state === "MERGED") {
    await prepareRelease(version, release);
  } else if (bump?.state === "OPEN") {
    console.log(
      `The version-bump pull request is open and waiting to be merged: ${bump.url}\nRun \`just release-prepare ${version}\` again once it has merged.`,
    );
  } else if (bump?.state === "CLOSED" && refExists(remoteBranch(branch))) {
    // Once the branch is gone the closed pull request is history, and the
    // next run starts over with a new one.
    refuse(
      "bump pull request",
      `${bump.url} was closed without merging. Reopen it, or delete the ${branch} branch on ${REMOTE} and locally to start over.`,
    );
  } else {
    await prepareBump(version, branch);
  }
}

try {
  await main();
} catch (error) {
  if (error instanceof Refusal) {
    console.error(error.message);
    process.exit(1);
  }
  throw error;
}
