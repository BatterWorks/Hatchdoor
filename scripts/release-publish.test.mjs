import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, test } from "node:test";
import { fileURLToPath } from "node:url";

import { renderChecklist } from "./release-common.mjs";

const scriptsDirectory = path.dirname(fileURLToPath(import.meta.url));
const temporaryDirectories = [];
const VERSION = "1.1.0";
const TAG = `v${VERSION}`;
const BUMP_BRANCH = `docs/release-v${VERSION}`;

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map((directory) => rm(directory, { recursive: true, force: true })),
  );
});

// A stand-in for the GitHub CLI. It keeps pull requests and releases in a
// JSON file and answers in the shapes the real CLI does. The merge endpoint
// merges in the bare origin the way GitHub would: a merge commit whose first
// parent is `main` and whose second is the pull request's head, refused when
// the head moved past the `sha` the caller expected.
const GH_STUB = `#!/usr/bin/env node
const fs = require("node:fs");
const { execFileSync } = require("node:child_process");
const file = process.env.STUB_STATE;
const state = JSON.parse(fs.readFileSync(file, "utf8"));
const args = process.argv.slice(2);
state.calls.push(["gh", ...args]);
const save = () => fs.writeFileSync(file, JSON.stringify(state, null, 2));
const option = (name) => { const i = args.indexOf(name); return i === -1 ? undefined : args[i + 1]; };
const field = (name) => { const hit = args.find((arg) => arg.startsWith(name + "=")); return hit && hit.slice(name.length + 1); };
const origin = (...rest) => execFileSync("git", ["--git-dir", process.env.STUB_ORIGIN, ...rest], { encoding: "utf8" }).trim();
const [noun, verb] = args;
if (noun === "pr" && verb === "list") {
  const wanted = option("--state");
  const pulls = state.pulls.filter((pull) =>
    pull.head === option("--head") && pull.base === option("--base") &&
    (wanted === "all" || pull.state === wanted.toUpperCase()));
  save();
  console.log(JSON.stringify(pulls.map(({ number, url, state, title, body, mergeCommit }) =>
    ({ number, url, state, title, body, mergeCommit: mergeCommit ?? null }))));
} else if (noun === "api" && option("--method") === "PUT" && /\\/pulls\\/\\d+\\/merge$/.test(args.find((arg) => arg.startsWith("repos/")))) {
  const number = Number(/pulls\\/(\\d+)\\/merge/.exec(args.find((arg) => arg.startsWith("repos/")))[1]);
  const pull = state.pulls.find((candidate) => candidate.number === number);
  if (field("merge_method") !== "merge") { save(); console.error("stub: only merge commits"); process.exit(1); }
  if (process.env.STUB_FAIL_MERGE) { save(); console.error("HTTP 405: Pull Request is not mergeable"); process.exit(1); }
  const head = origin("rev-parse", "refs/heads/" + pull.head);
  if (field("sha") !== head) { save(); console.error("HTTP 409: Head branch was modified"); process.exit(1); }
  const tree = origin("rev-parse", head + "^{tree}");
  const base = origin("rev-parse", "refs/heads/" + pull.base);
  const merge = execFileSync("git", ["--git-dir", process.env.STUB_ORIGIN, "commit-tree", tree, "-p", base, "-p", head, "-m", "Merge pull request #" + number], { encoding: "utf8", env: process.env }).trim();
  origin("update-ref", "refs/heads/" + pull.base, merge);
  pull.state = "MERGED";
  pull.mergeCommit = { oid: merge };
  save();
  console.log(JSON.stringify({ sha: merge, merged: true, message: "Pull Request successfully merged" }));
} else if (noun === "release" && verb === "view") {
  const release = state.releases.find((candidate) => candidate.tagName === args[2]);
  save();
  if (!release) { console.error("release not found"); process.exit(1); }
  console.log(JSON.stringify(release));
} else if (noun === "release" && verb === "edit") {
  if (process.env.STUB_FAIL_RELEASE_EDIT) { save(); console.error("HTTP 502"); process.exit(1); }
  const release = state.releases.find((candidate) => candidate.tagName === args[2]);
  const remote = execFileSync("git", ["ls-remote", "--tags", "origin", "refs/tags/" + args[2]], { encoding: "utf8" });
  if (args.includes("--verify-tag") && !remote.trim()) { save(); console.error("tag " + args[2] + " doesn't exist in the repo"); process.exit(1); }
  if (args.includes("--draft=false")) release.isDraft = false;
  if (args.includes("--latest")) state.latest = args[2];
  save();
  console.log(release.url);
} else {
  save();
  console.error("gh stub: unexpected " + args.join(" "));
  process.exit(3);
}
`;

// The out-of-repository release hook. It records each call and prints a line
// so the output can be traced back to it.
const HOOK_STUB = `#!/usr/bin/env node
const fs = require("node:fs");
const file = process.env.STUB_STATE;
const state = JSON.parse(fs.readFileSync(file, "utf8"));
const args = process.argv.slice(2);
state.calls.push(["hook", ...args]);
fs.writeFileSync(file, JSON.stringify(state, null, 2));
console.log("hook says: " + args.join(" "));
const exit = process.env["STUB_HOOK_" + args[0].toUpperCase() + "_EXIT"];
process.exit(Number(exit ?? 0));
`;

const CHANGELOG = `# Changelog

## v${VERSION} - 2026-10-01

### Added
- A new thing. [#12]

[#12]: https://github.test/issues/12

## v1.0.0 - 2026-01-01

### Added
- The first thing.
`;

const GIT_IDENTITY = {
  GIT_AUTHOR_NAME: "Release Test",
  GIT_AUTHOR_EMAIL: "release@test.invalid",
  GIT_COMMITTER_NAME: "Release Test",
  GIT_COMMITTER_EMAIL: "release@test.invalid",
};

function git(root, ...args) {
  const result = spawnSync("git", args, {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, ...GIT_IDENTITY },
  });
  assert.equal(
    result.status,
    0,
    `git ${args.join(" ")} failed: ${result.stderr}`,
  );
  return result.stdout.trim();
}

async function write(root, file, contents) {
  const absolute = path.join(root, file);
  await mkdir(path.dirname(absolute), { recursive: true });
  await writeFile(absolute, contents);
}

function ticked(body) {
  return body.replaceAll("- [ ] ", "- [x] ");
}

// A release where `just release-prepare` has done its part: the bump branch,
// its checklist all ticked, merged into `development` with a merge commit, the
// release pull request open from `development` into `main`, and the draft
// release written. The checkout sits on `development`. With `reviewFix`, the
// bump branch also carries a fix the review committed to it.
async function fixture({ reviewFix = false } = {}) {
  const base = await mkdtemp(path.join(tmpdir(), "hatchdoor-release-publish-"));
  temporaryDirectories.push(base);
  const origin = path.join(base, "origin.git");
  const root = path.join(base, "work");
  const bin = path.join(base, "bin");
  const hook = path.join(base, "hook");
  const state = path.join(base, "state.json");

  spawnSync("git", [
    "init",
    "--quiet",
    "--bare",
    "--initial-branch=main",
    origin,
  ]);
  spawnSync("git", ["init", "--quiet", "--initial-branch=main", root]);
  git(root, "remote", "add", "origin", origin);
  git(root, "config", "user.name", GIT_IDENTITY.GIT_AUTHOR_NAME);
  git(root, "config", "user.email", GIT_IDENTITY.GIT_AUTHOR_EMAIL);

  await write(
    root,
    "CHANGELOG.md",
    CHANGELOG.replace(`## v${VERSION}`, "## Unreleased"),
  );
  for (const script of ["release-publish.mjs", "release-common.mjs"]) {
    await mkdir(path.join(root, "scripts"), { recursive: true });
    await copyFile(
      path.join(scriptsDirectory, script),
      path.join(root, "scripts", script),
    );
  }
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "v1.0.0");
  git(root, "tag", "--annotate", "v1.0.0", "-m", "v1.0.0");
  git(root, "switch", "--quiet", "--create", "development");
  git(root, "switch", "--quiet", "--create", BUMP_BRANCH);
  await write(root, "CHANGELOG.md", CHANGELOG);
  git(
    root,
    "commit",
    "--quiet",
    "--all",
    "-m",
    `docs: prepare the ${TAG} release`,
  );
  if (reviewFix) {
    await write(root, "docs/user-vault/Some note.md", "Fixed.\n");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "docs: fix a stale note");
  }
  git(root, "switch", "--quiet", "development");
  git(
    root,
    "merge",
    "--quiet",
    "--no-ff",
    BUMP_BRANCH,
    "-m",
    "Merge pull request #1",
  );
  const bumpMerge = git(root, "rev-parse", "HEAD");
  git(
    root,
    "push",
    "--quiet",
    "origin",
    "main",
    "development",
    BUMP_BRANCH,
    "--tags",
  );

  await write(bin, "gh", GH_STUB);
  await chmod(path.join(bin, "gh"), 0o755);
  await writeFile(hook, HOOK_STUB);
  await chmod(hook, 0o755);
  await writeFile(
    state,
    JSON.stringify({
      pulls: [
        {
          number: 1,
          url: "https://github.test/pull/1",
          state: "MERGED",
          base: "development",
          head: BUMP_BRANCH,
          title: `Prepare the ${TAG} release`,
          body: `Sets the version.\n\n## Release checklist\n\n${ticked(renderChecklist("Documentation freshness review required"))}`,
          mergeCommit: { oid: bumpMerge },
        },
        {
          number: 2,
          url: "https://github.test/pull/2",
          state: "OPEN",
          base: "main",
          head: "development",
          title: `Release ${TAG}`,
          body: `Releases ${TAG}.\n`,
        },
      ],
      releases: [
        {
          tagName: TAG,
          isDraft: true,
          name: TAG,
          body: "### Added\n- A new thing.\n",
          url: "https://github.test/releases/untagged-1",
        },
      ],
      calls: [],
    }),
  );

  return { root, origin, bin, hook, state, bumpMerge };
}

function publish(context, args = [VERSION], env = {}) {
  return spawnSync(process.execPath, ["scripts/release-publish.mjs", ...args], {
    cwd: context.root,
    encoding: "utf8",
    env: {
      ...process.env,
      ...GIT_IDENTITY,
      PATH: `${context.bin}${path.delimiter}${process.env.PATH}`,
      STUB_STATE: context.state,
      STUB_ORIGIN: context.origin,
      HATCHDOOR_RELEASE_HOOK: context.hook,
      ...env,
    },
  });
}

async function stubState(context) {
  return JSON.parse(await readFile(context.state, "utf8"));
}

async function editState(context, change) {
  const state = await stubState(context);
  change(state);
  await writeFile(context.state, JSON.stringify(state));
}

// The side-effecting calls, in order, so a test can see what a run did
// and what it skipped.
async function actions(context) {
  return (await stubState(context)).calls
    .filter(
      (call) =>
        call[0] === "hook" ||
        (call[0] === "gh" && call[1] === "api") ||
        (call[0] === "gh" && call[1] === "release" && call[2] === "edit"),
    )
    .map((call) =>
      call[0] === "hook"
        ? `hook ${call[1]}`
        : call[1] === "api"
          ? "merge"
          : "publish",
    );
}

function remoteTag(context) {
  return spawnSync(
    "git",
    [
      "--git-dir",
      context.origin,
      "rev-parse",
      "--verify",
      "--quiet",
      `refs/tags/${TAG}`,
    ],
    { encoding: "utf8" },
  ).stdout.trim();
}

function assertRefused(result, check) {
  assert.equal(result.status, 1, result.stdout + result.stderr);
  assert.match(
    result.stderr,
    new RegExp(`release-publish refused \\(${check}\\)`),
  );
}

async function assertNothingDone(context) {
  assert.deepEqual(await actions(context), []);
  assert.equal(remoteTag(context), "");
}

test("merges, tags, builds images, publishes and deploys, in that order", async () => {
  const context = await fixture();

  const result = publish(context);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(await actions(context), [
    "merge",
    "hook images",
    "publish",
    "hook deploy",
  ]);

  const state = await stubState(context);
  const mergeCall = state.calls.find((call) => call[1] === "api");
  assert.ok(mergeCall.includes("repos/{owner}/{repo}/pulls/2/merge"));
  assert.ok(mergeCall.includes("merge_method=merge"));
  assert.ok(mergeCall.includes(`sha=${context.bumpMerge}`));

  const merge = spawnSync(
    "git",
    ["--git-dir", context.origin, "rev-parse", "refs/heads/main"],
    { encoding: "utf8" },
  ).stdout.trim();
  assert.equal(git(context.root, "rev-parse", `${merge}^2`), context.bumpMerge);
  assert.equal(
    spawnSync(
      "git",
      ["--git-dir", context.origin, "cat-file", "-t", `refs/tags/${TAG}`],
      { encoding: "utf8" },
    ).stdout.trim(),
    "tag",
    "the tag is annotated",
  );
  assert.equal(
    spawnSync(
      "git",
      ["--git-dir", context.origin, "rev-parse", `refs/tags/${TAG}^{commit}`],
      { encoding: "utf8" },
    ).stdout.trim(),
    merge,
  );

  const hookCalls = state.calls.filter((call) => call[0] === "hook");
  assert.deepEqual(hookCalls, [
    ["hook", "images", VERSION],
    ["hook", "deploy", VERSION],
  ]);
  const edit = state.calls.find((call) => call[2] === "edit");
  assert.ok(edit.includes("--draft=false"));
  assert.ok(edit.includes("--latest"));
  assert.ok(edit.includes("--verify-tag"));
  assert.equal(state.releases[0].isDraft, false);
  assert.equal(state.latest, TAG);
  assert.match(result.stdout, /hook says: deploy 1\.1\.0/);
});

test("a re-run after a finished release does nothing again", async () => {
  const context = await fixture();
  assert.equal(publish(context).status, 0);
  const tag = remoteTag(context);
  await editState(context, (state) => {
    state.calls = [];
  });

  const again = publish(context);
  assert.equal(again.status, 0, again.stderr);
  assert.deepEqual(await actions(context), []);
  assert.match(again.stdout, /deploy step already succeeded/);
  assert.equal(remoteTag(context), tag, "the tag is not recreated");
});

test("ships a fix the review committed to the bump branch", async () => {
  const context = await fixture({ reviewFix: true });
  const result = publish(context);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(await actions(context), [
    "merge",
    "hook images",
    "publish",
    "hook deploy",
  ]);
  assert.equal(
    spawnSync(
      "git",
      [
        "--git-dir",
        context.origin,
        "show",
        `refs/tags/${TAG}:docs/user-vault/Some note.md`,
      ],
      { encoding: "utf8" },
    ).stdout,
    "Fixed.\n",
  );
});

test("reads the checklist from the version-bump pull request, not the release pull request", async () => {
  const context = await fixture();
  await editState(context, (state) => {
    state.pulls[1].body += `\n${ticked(renderChecklist(""))}`;
    state.pulls[0].body = "Sets the version.\n";
  });
  const result = publish(context);
  assertRefused(result, "release checklist");
  assert.match(result.stderr, /https:\/\/github\.test\/pull\/1 is missing/);
  await assertNothingDone(context);
});

test("refuses without an executable release hook, on every run", async () => {
  const context = await fixture();

  const unset = publish(context, [VERSION], { HATCHDOOR_RELEASE_HOOK: "" });
  assertRefused(unset, "release hook");
  assert.match(unset.stderr, /HATCHDOOR_RELEASE_HOOK is not set/);
  await assertNothingDone(context);

  const missing = publish(context, [VERSION], {
    HATCHDOOR_RELEASE_HOOK: path.join(context.root, "no-such-hook"),
  });
  assertRefused(missing, "release hook");
  await assertNothingDone(context);

  await chmod(context.hook, 0o644);
  assertRefused(publish(context), "release hook");
  await assertNothingDone(context);

  // Still refused once the merge is behind it.
  await chmod(context.hook, 0o755);
  assert.notEqual(
    publish(context, [VERSION], { STUB_HOOK_IMAGES_EXIT: "1" }).status,
    0,
  );
  assertRefused(
    publish(context, [VERSION], { HATCHDOOR_RELEASE_HOOK: "" }),
    "release hook",
  );
});

test("refuses a version that is not bare", async () => {
  const context = await fixture();
  for (const version of ["v1.1.0", "1.1", "01.1.0"]) {
    assertRefused(publish(context, [version]), "version");
  }
  await assertNothingDone(context);
});

test("refuses without an open release pull request", async () => {
  const context = await fixture();
  await editState(context, (state) => {
    state.pulls = state.pulls.filter((pull) => pull.number !== 2);
  });
  assertRefused(publish(context), "release pull request");
  await assertNothingDone(context);
});

test("refuses while any checklist box is unticked, naming it", async () => {
  const context = await fixture();
  await editState(context, (state) => {
    state.pulls[0].body = state.pulls[0].body.replace(
      "- [x] The roadmap is current.",
      "- [ ] The roadmap is current.",
    );
  });
  const result = publish(context);
  assertRefused(result, "release checklist");
  assert.match(result.stderr, /The roadmap is current\./);
  await assertNothingDone(context);
});

test("refuses when a checklist item was edited or removed", async () => {
  const context = await fixture();
  await editState(context, (state) => {
    state.pulls[0].body = state.pulls[0].body.replace(
      "- [x] The README describes what shipped.",
      "- [x] The README is fine.",
    );
  });
  const result = publish(context);
  assertRefused(result, "release checklist");
  assert.match(result.stderr, /missing.*The README describes what shipped\./s);
  await assertNothingDone(context);
});

test("refuses without a draft release", async () => {
  const context = await fixture();
  await editState(context, (state) => {
    state.releases = [];
  });
  assertRefused(publish(context), "draft release");
  await assertNothingDone(context);
});

test("refuses a draft release with an empty title or body", async () => {
  for (const key of ["name", "body"]) {
    const context = await fixture();
    await editState(context, (state) => {
      state.releases[0][key] = "  \n";
    });
    const result = publish(context);
    assertRefused(result, "draft release");
    assert.match(result.stderr, key === "name" ? /title/ : /notes/);
    await assertNothingDone(context);
  }
});

test("refuses a release that is already published before the merge", async () => {
  const context = await fixture();
  await editState(context, (state) => {
    state.releases[0].isDraft = false;
  });
  assertRefused(publish(context), "draft release");
  await assertNothingDone(context);
});

test("refuses while Unreleased on development has entries", async () => {
  const context = await fixture();
  await write(
    context.root,
    "CHANGELOG.md",
    CHANGELOG.replace(
      `## v${VERSION}`,
      "## Unreleased\n\n### Fixed\n- Something late.\n\n## v" + VERSION,
    ),
  );
  git(context.root, "commit", "--quiet", "--all", "-m", "late fix");
  git(context.root, "push", "--quiet", "origin", "development");
  const result = publish(context);
  assertRefused(result, "changelog");
  await assertNothingDone(context);
});

test("refuses when development has moved past the bump merge", async () => {
  const context = await fixture();
  await write(context.root, "later.txt", "later\n");
  git(context.root, "add", "later.txt");
  git(context.root, "commit", "--quiet", "-m", "a later change");
  git(context.root, "push", "--quiet", "origin", "development");
  const result = publish(context);
  assertRefused(result, "development tip");
  assert.match(result.stderr, new RegExp(context.bumpMerge.slice(0, 12)));
  await assertNothingDone(context);
});

test("refuses when the version-bump pull request has not merged", async () => {
  const context = await fixture();
  await editState(context, (state) => {
    state.pulls[0].state = "OPEN";
    delete state.pulls[0].mergeCommit;
  });
  assertRefused(publish(context), "development tip");
  await assertNothingDone(context);
});

test("once merged, the pre-merge checks are skipped and the run resumes after the merge", async () => {
  const context = await fixture();
  const failed = publish(context, [VERSION], { STUB_HOOK_IMAGES_EXIT: "7" });
  assert.equal(failed.status, 1);
  assert.match(failed.stderr, /images step failed with exit status 7/);
  assert.deepEqual(await actions(context), ["merge", "hook images"]);
  const tag = remoteTag(context);
  assert.notEqual(tag, "");

  // Everything the pre-merge checks look at now fails them.
  await write(context.root, "later.txt", "later\n");
  git(context.root, "add", "later.txt");
  git(context.root, "commit", "--quiet", "-m", "a later change");
  git(context.root, "push", "--quiet", "origin", "development");
  await editState(context, (state) => {
    state.pulls[0].body = state.pulls[0].body.replaceAll("- [x] ", "- [ ] ");
    state.calls = [];
  });

  const resumed = publish(context);
  assert.equal(resumed.status, 0, resumed.stderr);
  assert.deepEqual(await actions(context), [
    "hook images",
    "publish",
    "hook deploy",
  ]);
  assert.equal(remoteTag(context), tag, "the tag is not recreated");
});

test("resumes after a failed publish without merging or tagging again", async () => {
  const context = await fixture();
  const failed = publish(context, [VERSION], { STUB_FAIL_RELEASE_EDIT: "1" });
  assert.notEqual(failed.status, 0);
  assert.deepEqual(await actions(context), ["merge", "hook images", "publish"]);
  await editState(context, (state) => {
    state.calls = [];
  });

  const resumed = publish(context);
  assert.equal(resumed.status, 0, resumed.stderr);
  assert.deepEqual(await actions(context), ["publish", "hook deploy"]);
});

test("a failed deploy is reported with its output and leaves the release published", async () => {
  const context = await fixture();
  const result = publish(context, [VERSION], { STUB_HOOK_DEPLOY_EXIT: "4" });
  assert.equal(result.status, 1);
  assert.match(result.stdout, /hook says: deploy 1\.1\.0/);
  assert.match(result.stderr, /deploy step failed with exit status 4/);
  assert.match(result.stderr, /stays published/);
  const state = await stubState(context);
  assert.equal(state.releases[0].isDraft, false);

  await editState(context, (next) => {
    next.calls = [];
  });
  const retried = publish(context);
  assert.equal(retried.status, 0, retried.stderr);
  assert.deepEqual(await actions(context), ["hook deploy"]);
});

test("pushes a tag made locally before an interrupted push, without remaking it", async () => {
  const context = await fixture();
  // Merge only: the merge goes through, then the run stops at the hook.
  const failed = publish(context, [VERSION], { STUB_HOOK_IMAGES_EXIT: "1" });
  assert.equal(failed.status, 1);
  const tag = remoteTag(context);
  // Take the tag back off the remote, as if its push had never landed.
  spawnSync("git", ["--git-dir", context.origin, "tag", "--delete", TAG]);
  assert.equal(remoteTag(context), "");

  const resumed = publish(context);
  assert.equal(resumed.status, 0, resumed.stderr);
  assert.equal(remoteTag(context), tag, "the same tag object is pushed");
});

test("never moves a tag that points somewhere other than the merge", async () => {
  const context = await fixture();
  git(context.root, "tag", "--annotate", TAG, "main", "-m", "wrong place");
  git(context.root, "push", "--quiet", "origin", TAG);
  const before = remoteTag(context);

  const result = publish(context);
  assertRefused(result, "tag");
  assert.equal(remoteTag(context), before);
  assert.deepEqual(await actions(context), [], "refused before the merge");
});

test("refuses a merge whose second parent is not the bump merge", async () => {
  const context = await fixture();
  // GitHub merged something else under this pull request.
  await write(context.root, "other.txt", "other\n");
  git(context.root, "switch", "--quiet", "--create", "other", "main");
  git(context.root, "add", "other.txt");
  git(context.root, "commit", "--quiet", "-m", "other");
  git(context.root, "switch", "--quiet", "main");
  git(
    context.root,
    "merge",
    "--quiet",
    "--no-ff",
    "other",
    "-m",
    "Merge pull request #2",
  );
  git(context.root, "push", "--quiet", "origin", "main");
  const merge = git(context.root, "rev-parse", "main");
  await editState(context, (state) => {
    state.pulls[1].state = "MERGED";
    state.pulls[1].mergeCommit = { oid: merge };
  });

  const result = publish(context);
  assertRefused(result, "merge commit");
  assert.equal(remoteTag(context), "");
  assert.deepEqual(await actions(context), []);
});
