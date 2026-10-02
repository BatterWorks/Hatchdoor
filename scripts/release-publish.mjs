#!/usr/bin/env node

// `just release-publish <version>`: the second of the two release commands
// (ADR-36, ADR-37). It merges the release pull request that
// `just release-prepare` opened, once every box in the checklist on the merged
// version-bump pull request is ticked, tags the merge, has the release hook
// build and push the images, publishes the draft GitHub Release, and has the
// hook deploy.
//
// The agent runs it only after the maintainer approved the release title and
// notes in the agent's own session (ADR-36 decision 4). Nothing here checks
// that approval; it is a rule the agent follows.
//
// Everything the public repository cannot know about, the build machines,
// the registries and the servers, sits behind the hook named by
// `HATCHDOOR_RELEASE_HOOK`, called as `<hook> images <version>` and
// `<hook> deploy <version>` (decision 7). Each hook step is safe to re-run.
//
// Every run works out where the release stands from git and GitHub, so a
// re-run after a failure resumes at the first step not yet done. The two hook
// steps leave no trace there, so each records its success in a marker under
// the git directory; a re-run on another machine repeats them, which the hook
// contract allows. Once the
// release merge commit exists on `main`, the pre-merge checks are skipped:
// `development` moving on after the merge must not block finishing the
// release. Nothing is ever force-pushed, and an existing tag is never moved.

import { spawnSync } from "node:child_process";
import {
  accessSync,
  constants,
  existsSync,
  mkdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  hasEntries,
  parseChecklist,
  parseVersion,
  unreleasedSection,
} from "./release-common.mjs";

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

const REMOTE = "origin";
const DEVELOPMENT = "development";
const MAIN = "main";
const HOOK_VARIABLE = "HATCHDOOR_RELEASE_HOOK";

class Refusal extends Error {}

function refuse(check, detail) {
  throw new Refusal(`release-publish refused (${check}): ${detail}`);
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

function short(sha) {
  return sha.slice(0, 12);
}

// ---------------------------------------------------------------------------
// Step 1: checks

// Checked on every run, since every run after the merge still calls the hook.
// A bare name is not looked up on PATH: the variable must name the file.
function requireHook() {
  const value = process.env[HOOK_VARIABLE] ?? "";
  if (value === "") {
    refuse(
      "release hook",
      `${HOOK_VARIABLE} is not set. Point it at the executable that runs \`<hook> images <version>\` and \`<hook> deploy <version>\`.`,
    );
  }
  const hook = path.resolve(value);
  try {
    if (!statSync(hook).isFile()) {
      throw new Error("not a file");
    }
    accessSync(hook, constants.X_OK);
  } catch {
    refuse(
      "release hook",
      `${HOOK_VARIABLE} names ${hook}, which is not an executable file.`,
    );
  }
  return hook;
}

// The merged version-bump pull request, which carries the release checklist
// (ADR-37), with its merge commit as `bumpMerge`. Prepare names the branch,
// and a re-used branch can carry closed attempts beside the merged one.
function mergedBumpPullRequest(version) {
  const pulls = JSON.parse(
    gh(
      "pr",
      "list",
      "--head",
      `docs/release-v${version}`,
      "--base",
      DEVELOPMENT,
      "--state",
      "merged",
      "--json",
      "number,url,body,mergeCommit",
    ),
  );
  const pull = pulls.find((candidate) => candidate.mergeCommit?.oid);
  return pull ? { ...pull, bumpMerge: pull.mergeCommit.oid } : null;
}

// The `development` to `main` pull request titled for this version. A merged
// one wins over an open one, so a run after the merge finds the merge.
function releasePullRequest(version) {
  const pulls = JSON.parse(
    gh(
      "pr",
      "list",
      "--head",
      DEVELOPMENT,
      "--base",
      MAIN,
      "--state",
      "all",
      "--limit",
      "100",
      "--json",
      "number,url,state,title,mergeCommit",
    ),
  ).filter((pull) => pull.title === `Release v${version}`);
  for (const state of ["MERGED", "OPEN"]) {
    const pull = pulls.find((candidate) => candidate.state === state);
    if (pull) {
      return pull;
    }
  }
  return null;
}

function githubRelease(tag) {
  const result = run("gh", [
    "release",
    "view",
    tag,
    "--json",
    "isDraft,name,body,url",
  ]);
  if (result.status === 0) {
    return JSON.parse(result.stdout);
  }
  if (/release not found/i.test(result.stderr)) {
    return null;
  }
  throw new Error(`gh release view ${tag} failed: ${result.stderr.trim()}`);
}

function requireChecklist(pull) {
  const { items, missing } = parseChecklist(pull.body ?? "");
  if (missing.length > 0) {
    refuse(
      "release checklist",
      `${pull.url} is missing these items in their exact wording:\n${missing.map((item) => `  - ${item}`).join("\n")}`,
    );
  }
  const unticked = items.filter((item) => !item.checked);
  if (unticked.length > 0) {
    refuse(
      "release checklist",
      `these boxes on ${pull.url} are not ticked:\n${unticked.map((item) => `  - ${item.text}`).join("\n")}`,
    );
  }
}

function requireDraftRelease(tag) {
  const draft = githubRelease(tag);
  if (!draft) {
    refuse(
      "draft release",
      `there is no GitHub Release for ${tag}. Run \`just release-prepare\` to create the draft.`,
    );
  }
  if (!draft.isDraft) {
    refuse(
      "draft release",
      `${tag} is already published (${draft.url}), but its release pull request has not merged.`,
    );
  }
  if (!draft.name?.trim()) {
    refuse("draft release", `the draft ${draft.url} has no title.`);
  }
  if (!draft.body?.trim()) {
    refuse("draft release", `the draft ${draft.url} has no notes.`);
  }
}

function requireEmptyUnreleased() {
  const changelog = git("show", `${REMOTE}/${DEVELOPMENT}:CHANGELOG.md`);
  const section = unreleasedSection(changelog);
  if (section && hasEntries(section.body)) {
    refuse(
      "changelog",
      `"## Unreleased" in CHANGELOG.md on ${DEVELOPMENT} has entries. They were merged after the version bump and are not in this release's notes. Release them in the next version.`,
    );
  }
}

function requireMergedBump(version) {
  const bump = mergedBumpPullRequest(version);
  if (!bump) {
    refuse(
      "development tip",
      `the version-bump pull request from docs/release-v${version} has not merged into ${DEVELOPMENT}.`,
    );
  }
  return bump;
}

// Nothing may follow the bump on `development`, so the merge brings into
// `main` exactly what prepare tested (decision 5).
function requireBumpTip({ bumpMerge: bump }) {
  const tip = git("rev-parse", `${REMOTE}/${DEVELOPMENT}`);
  if (tip !== bump) {
    refuse(
      "development tip",
      `${DEVELOPMENT} is at ${short(tip)}, not at the version-bump merge ${short(bump)}. Something merged after the bump would ship unlisted; release it in the next version.`,
    );
  }
  return bump;
}

function requireNoTag(tag) {
  if (localTag(tag) || remoteTag(tag)) {
    refuse(
      "tag",
      `${tag} already exists, but its release pull request has not merged. A tag is never moved; delete it by hand only if it was never published.`,
    );
  }
}

// ---------------------------------------------------------------------------
// Step 2: the merge

// The `sha` parameter makes GitHub refuse the merge if `development` moved
// after the checks above.
function merge(pull, tip) {
  const response = JSON.parse(
    gh(
      "api",
      "--method",
      "PUT",
      `repos/{owner}/{repo}/pulls/${pull.number}/merge`,
      "-f",
      "merge_method=merge",
      "-f",
      `sha=${tip}`,
    ),
  );
  console.log(`Merged ${pull.url} as ${short(response.sha)}.`);
  return response.sha;
}

// Run after the merge on every run, not only the one that merged: it pins
// what the tag will name.
function requireMergeCommit(mergeCommit, bumpMerge) {
  git("fetch", "--quiet", REMOTE, MAIN);
  const onMain =
    run("git", [
      "merge-base",
      "--is-ancestor",
      mergeCommit,
      `${REMOTE}/${MAIN}`,
    ]).status === 0;
  if (!onMain) {
    refuse(
      "merge commit",
      `${short(mergeCommit)} is not on ${REMOTE}/${MAIN}.`,
    );
  }
  const parents = git("rev-list", "--parents", "--max-count=1", mergeCommit)
    .split(" ")
    .slice(1);
  if (parents.length !== 2 || parents[1] !== bumpMerge) {
    refuse(
      "merge commit",
      `${short(mergeCommit)} does not merge the version-bump merge ${short(bumpMerge)} into ${MAIN}. Nothing was tagged; look at what landed on ${MAIN} before going further.`,
    );
  }
}

// ---------------------------------------------------------------------------
// Step 3: the tag

function localTag(tag) {
  const result = run("git", [
    "rev-parse",
    "--verify",
    "--quiet",
    `refs/tags/${tag}`,
  ]);
  return result.status === 0 ? result.stdout.trim() : null;
}

function remoteTag(tag) {
  const match = git("ls-remote", "--tags", REMOTE, `refs/tags/${tag}`)
    .split("\n")
    .map((line) => line.split(/\s+/))
    .find(([, ref]) => ref === `refs/tags/${tag}`);
  return match ? match[0] : null;
}

function tagCommit(tag) {
  return git("rev-parse", `refs/tags/${tag}^{commit}`);
}

function ensureTagPushed(tag, mergeCommit) {
  const remote = remoteTag(tag);
  let local = localTag(tag);
  if (remote && local && remote !== local) {
    refuse(
      "tag",
      `the local ${tag} and the one on ${REMOTE} differ. A tag is never moved; work out which is right by hand.`,
    );
  }
  if (remote && !local) {
    git("fetch", "--quiet", REMOTE, `refs/tags/${tag}:refs/tags/${tag}`);
    local = remote;
  }
  if (local) {
    if (git("cat-file", "-t", local) !== "tag") {
      refuse("tag", `${tag} exists but is not an annotated tag.`);
    }
    const tagged = tagCommit(tag);
    if (tagged !== mergeCommit) {
      refuse(
        "tag",
        `${tag} points at ${short(tagged)}, not at the release merge ${short(mergeCommit)}. A tag is never moved.`,
      );
    }
  } else {
    git("tag", "--annotate", tag, mergeCommit, "-m", `Hatchdoor ${tag}`);
    console.log(`Tagged ${short(mergeCommit)} as ${tag}.`);
  }
  if (!remote) {
    git("push", "--quiet", REMOTE, `refs/tags/${tag}`);
    console.log(`Pushed ${tag}.`);
  }
}

// ---------------------------------------------------------------------------
// Steps 4 to 6: the hook and the release

function stepMarker(tag, step) {
  return path.resolve(
    repositoryRoot,
    git("rev-parse", "--git-path", `hatchdoor-release/${tag}/${step}-done`),
  );
}

function markDone(tag, step) {
  const marker = stepMarker(tag, step);
  mkdirSync(path.dirname(marker), { recursive: true });
  writeFileSync(marker, `${new Date().toISOString()}\n`);
}

function runHook(hook, step, version) {
  console.log(
    `\nRunning the release hook's ${step} step; its output follows.\n`,
  );
  const result = spawnSync(hook, [step, version], {
    cwd: repositoryRoot,
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  return result.status;
}

function publishRelease(tag) {
  gh("release", "edit", tag, "--draft=false", "--latest", "--verify-tag");
  console.log(`Published ${tag} and marked it latest.`);
}

// ---------------------------------------------------------------------------

function usage(message) {
  console.error(`release-publish: ${message}`);
  console.error("Usage: just release-publish <version>   (for example 2.7.0)");
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
  const tag = `v${version}`;

  const hook = requireHook();
  git("fetch", "--quiet", REMOTE);

  const pull = releasePullRequest(version);
  let mergeCommit;
  let bumpMerge;
  if (pull?.state === "MERGED") {
    mergeCommit = pull.mergeCommit.oid;
    bumpMerge = mergedBumpPullRequest(version)?.bumpMerge;
    if (!bumpMerge) {
      refuse(
        "merge commit",
        `${pull.url} has merged, but no merged version-bump pull request from docs/release-v${version} was found to check it against.`,
      );
    }
    console.log(
      `${pull.url} has already merged as ${short(mergeCommit)}; resuming after the merge.`,
    );
  } else {
    if (!pull) {
      refuse(
        "release pull request",
        `there is no open or merged pull request from ${DEVELOPMENT} into ${MAIN} titled exactly "Release ${tag}". If it was retitled, restore that title; otherwise run \`just release-prepare ${version}\` to open it.`,
      );
    }
    const bump = requireMergedBump(version);
    requireChecklist(bump);
    requireDraftRelease(tag);
    requireEmptyUnreleased();
    bumpMerge = requireBumpTip(bump);
    requireNoTag(tag);
    mergeCommit = merge(pull, bumpMerge);
  }

  requireMergeCommit(mergeCommit, bumpMerge);
  ensureTagPushed(tag, mergeCommit);

  const current = githubRelease(tag);
  if (!current) {
    refuse("draft release", `the GitHub Release for ${tag} has gone missing.`);
  }
  if (current.isDraft) {
    if (existsSync(stepMarker(tag, "images"))) {
      console.log(`The hook's images step already succeeded for ${tag}.`);
    } else {
      const images = runHook(hook, "images", version);
      if (images !== 0) {
        refuse(
          "images",
          `the hook's images step failed with exit status ${images}; its output is above. ${tag} is tagged and the release is still a draft. Fix the cause and run \`just release-publish ${version}\` again.`,
        );
      }
      markDone(tag, "images");
    }
    publishRelease(tag);
  } else {
    console.log(`${tag} is already published: ${current.url}`);
  }

  if (existsSync(stepMarker(tag, "deploy"))) {
    console.log(`The hook's deploy step already succeeded for ${tag}.`);
  } else {
    const deploy = runHook(hook, "deploy", version);
    if (deploy !== 0) {
      refuse(
        "deploy",
        `the hook's deploy step failed with exit status ${deploy}; its output is above. ${tag} stays published. Fix the cause and run \`just release-publish ${version}\` again to retry the deploy.`,
      );
    }
    markDone(tag, "deploy");
  }
  console.log(`\n${tag} is released and deployed: ${githubRelease(tag).url}`);
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
