#!/usr/bin/env node

// `just release-prepare <version>`: the first of the two release commands
// (ADR-36, ADR-37). It opens pull requests and a draft release, and stops. It
// never merges anything, never pushes to `development` or `main`, and never
// records the documentation review on anyone's behalf.
//
// The first run cuts `docs/release-v<version>` off `development`, sets the
// version in the four version files, dates the changelog's Unreleased
// section, runs the mechanical checks, and opens the version-bump pull
// request into `development`, carrying the judgment checklist, beside a draft
// GitHub Release made from the changelog section. The review happens on that
// pull request, so a fix it needs is committed to the bump branch; a run while
// the pull request is open tests and pushes such a commit. Once the bump has
// merged, the next run opens the `development` to `main` release pull
// request, which carries no checklist.
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

// The remote branch must not hold a commit the local one lacks. Such a commit
// was pushed some other way, so no run of this command tested it, and pushing
// over it would fail only after a full test run.
function requireRemoteNotAhead(branch) {
  if (
    refExists(remoteBranch(branch)) &&
    run("git", ["merge-base", "--is-ancestor", remoteBranch(branch), "HEAD"])
      .status !== 0
  ) {
    refuse(
      "bump branch",
      `${REMOTE}/${branch} has commits the local ${branch} lacks, so they were pushed without this command testing them. Bring them in with \`git pull --ff-only\`, run \`just check-full\` yourself, since this command does not test a commit already on ${REMOTE}, and run this again.`,
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

function bumpPullRequestBody(version, branch, freshness) {
  return `Sets the version to ${version} in ${VERSION_FILES.map((file) => `\`${file}\``).join(", ")}, and renames the changelog's \`## Unreleased\` section to \`## v${version} - <date>\`.

The version files agree, every \`[#N]\` in the section has a link definition, and \`just check-full\` passed on this branch.

The release is reviewed here, before this merges. Commit any fix the review needs to \`${branch}\` and run \`just release-prepare ${version}\` again, which tests and pushes it, so the fix ships in this release. \`just release-publish ${version}\` refuses while any box below is unticked. Tick each box only after doing the check it names. Publishing waits for the maintainer to approve the release title and notes.

Made by \`just release-prepare ${version}\`. Once this has merged, run it again to open the release pull request into \`${MAIN}\`.

## Release checklist

${renderChecklist(freshness)}
`;
}

// The notes come from the changelog passed in: the bump branch's before the
// bump merges, and `development`'s after.
function ensureDraftRelease(version, release, changelog) {
  if (release) {
    console.log(`The draft release already exists: ${release.url}`);
    return release.url;
  }
  const tag = `v${version}`;
  const url = gh(
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
  console.log(`Created the draft release: ${url}`);
  return url;
}

// A run while the bump is open brings it up to date instead of opening a
// second one: a commit made on the branch for the review is checked,
// including `just check-full`, and pushed, and a draft release lost to a crash
// is created.
async function prepareBump(version, branch, bump, release) {
  switchToBumpBranch(branch);
  requireRemoteNotAhead(branch);
  await applyBump(version);
  await checkBump(version, branch);
  pushBranch(branch);
  let pullUrl;
  if (bump) {
    pullUrl = bump.url;
    console.log(`The version-bump pull request is open: ${pullUrl}`);
  } else {
    pullUrl = gh(
      "pr",
      "create",
      "--base",
      DEVELOPMENT,
      "--head",
      branch,
      "--title",
      `Prepare the v${version} release`,
      "--body",
      bumpPullRequestBody(version, branch, docsFreshness()),
    );
    console.log(`Opened the version-bump pull request: ${pullUrl}`);
  }
  const releaseUrl = ensureDraftRelease(
    version,
    release,
    await readRepositoryFile("CHANGELOG.md"),
  );
  console.log(`
Next: work through the checklist on ${pullUrl}. Commit any fix the review needs to ${branch} and run \`just release-prepare ${version}\` again to test and push it. Draft the release title and notes on ${releaseUrl}. Once every box is ticked, merge the pull request into ${DEVELOPMENT} and run \`just release-prepare ${version}\` again to open the release pull request.`);
}

// `just docs-freshness main` judges the checkout, so it runs on the bump
// branch just pushed, which is what the release will merge into `main`. It
// compares against the local `main` when one exists, so that is brought up to
// date first; otherwise a stale `main` widens the reading list past what this
// release touched.
//
// The script exits 1 both when it hands over a reading list and when the
// changelog check fails, so only the first counts as a result to paste.
function docsFreshness() {
  const update = run("git", ["fetch", "--quiet", REMOTE, `${MAIN}:${MAIN}`]);
  if (update.status !== 0) {
    refuse(
      "docs-freshness",
      `could not fast-forward the local ${MAIN} to ${REMOTE}/${MAIN}: ${update.stderr.trim()}`,
    );
  }
  const result = run("sh", ["-c", `just docs-freshness ${MAIN} 2>&1`]);
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

// ---------------------------------------------------------------------------
// Step 5: the release pull request

function releasePullRequestBody(version, bump) {
  return `Releases v${version}: merges \`${DEVELOPMENT}\` into \`${MAIN}\`.

The release was reviewed on the version-bump pull request, ${bump.url}. \`just release-publish ${version}\` merges this pull request, and refuses while any box in that pull request's checklist is unticked. Publishing waits for the maintainer to approve the release title and notes.
`;
}

async function prepareRelease(version, bump, release) {
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
      releasePullRequestBody(version, bump),
    );
    console.log(`Opened the release pull request: ${pullUrl}`);
  }

  const releaseUrl = ensureDraftRelease(version, release, changelog);

  console.log(`
Next: give the maintainer the release title and notes on ${releaseUrl} to approve. \`just release-publish ${version}\` merges ${pullUrl}, and runs only after that approval.`);
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
    await prepareRelease(version, bump, release);
  } else if (bump?.state === "OPEN") {
    await prepareBump(version, branch, bump, release);
  } else if (bump?.state === "CLOSED" && refExists(remoteBranch(branch))) {
    // Once the branch is gone the closed pull request is history, and the
    // next run starts over with a new one.
    refuse(
      "bump pull request",
      `${bump.url} was closed without merging. Reopen it, or delete the ${branch} branch on ${REMOTE} and locally to start over.`,
    );
  } else {
    await prepareBump(version, branch, null, release);
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
