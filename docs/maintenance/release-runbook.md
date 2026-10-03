# Release runbook

A release is cut by two commands, decided in [ADR-36](../adr/README.md#adr-36--a-release-is-cut-by-two-commands-and-only-the-maintainer-approves-its-notes). [ADR-37](../adr/README.md#adr-37--a-release-is-reviewed-on-its-version-bump-pull-request) moved the review onto the version-bump pull request:

- `just release-prepare <version>` opens the version-bump pull request with the review checklist, creates a draft GitHub Release, and once the bump has merged, opens the release pull request into `main`. It never merges anything.
- `just release-publish <version>` merges that pull request, tags the merge, has the release hook build and push the images, publishes the GitHub Release, and has the hook deploy.

An agent carries out the release. The maintainer's only task is to approve the release title and notes, and `release-publish` runs only after that approval (see [Approval](#approval)).

Both commands work out where the release stands from git and GitHub on every run. Running one again after a failure, a crash or a closed terminal resumes at the first step not yet done. Neither command force-pushes, moves a tag, or replaces the image behind a published version.

## Before you start

- Run the commands from the repository root on the maintainer's build machine. `release-publish` needs the release hook and the registry credentials, which exist only there.
- The working tree must be clean. `release-prepare` refuses otherwise.
- `gh` must be logged in to an account that can push branches and tags, open and merge pull requests, and create releases on the repository.
- `HATCHDOOR_RELEASE_HOOK` must hold the path to the release hook (see [The release hook](#the-release-hook)). Only `release-publish` reads it.
- `## Unreleased` in `CHANGELOG.md` on `development` must have entries. With nothing under it there is nothing to release.
- The version is a bare semantic version such as `2.7.0`: no `v`, no suffix, no leading zeros, and higher than the latest `v*` tag. The git tag and the GitHub Release add the `v`; the image tags do not.

The review happens on the version-bump pull request, before it merges. A stale note, README section or roadmap entry it finds is fixed on the bump branch and ships in this release (see [Judgment checklist](#judgment-checklist)). From the moment `release-prepare` cuts the bump branch until `release-publish` has merged the release, nothing else may merge into `development`. A pull request merged while the bump is open rides into the release with the bump merge, and `release-publish` cannot tell.

## The release, start to finish

### 1. Open the version-bump pull request and the draft release

```bash
just release-prepare 2.7.0
```

The first run:

1. Fetches `origin` with its tags and refuses if the version is not higher than the latest release tag or is already published.
2. Cuts `docs/release-v2.7.0` from `origin/development`.
3. Sets the version in `Cargo.toml`, `Cargo.lock`, `frontend/package.json` and `frontend/package-lock.json`, renames `## Unreleased` in `CHANGELOG.md` to `## v2.7.0 - <today>`, drafts the release's highlights in `docs/user-vault/What's new.md` (see [Highlights](#highlights)), and commits.
4. Refuses unless the four files agree on the version, the dated section has entries, every `[#N]` in it has a `[#N]: <url>` definition somewhere in the file, and the What's new section has 3 to 6 lines with action-needed lines first.
5. Runs `just check-full`. A cold build takes tens of minutes, and its output streams to the terminal.
6. Pushes the branch.
7. Runs `just docs-freshness main` on the bump branch, after fast-forwarding the local `main` to `origin/main` so the list covers this release and no more.
8. Opens **Prepare the v2.7.0 release** into `development`, carrying the judgment checklist with the docs-freshness output pasted under its item.
9. Creates a draft GitHub Release for `v2.7.0` targeting `main`, titled `v2.7.0`, with the changelog section as its notes. Link definitions the section uses but does not hold are copied in, so `[#N]` markers render as links on the release page.

It prints both URLs and stops.

### 2. Work through the judgment checklist

Do each check named in [Judgment checklist](#judgment-checklist) and tick its box on the version-bump pull request on GitHub. `release-publish` refuses while any box is unticked, and it reads an item only in its exact wording, so edit a box's mark, never its text.

When a check finds something to fix, such as a stale note, a README section or a roadmap entry, fix it on the bump branch:

```bash
git switch docs/release-v2.7.0
# edit, then
git add <the files you changed or added>
git commit -m 'docs: <what the fix does>'
just release-prepare 2.7.0
```

With the bump open, `release-prepare` opens nothing new. It checks the branch again, runs `just check-full` on the new commit, and pushes it, so the fix merges with the bump and ships in this release. Push the bump branch only this way, never by hand: `release-prepare` skips `just check-full` when the remote branch already holds the commit, because it pushes only after a passing run. A commit pushed any other way makes it refuse with `bump branch`.

A fix that adds a changelog entry goes under the dated `## v2.7.0` section, not under a new `## Unreleased`.

### 3. Draft the release title and notes

Before the notes, rewrite the highlights if the first run did not already make you (see [Highlights](#highlights)). They are part of what the maintainer approves.

Edit the draft GitHub Release until its title and notes are what should go public:

```bash
gh release edit v2.7.0 --title 'v2.7.0' --notes-file <file>
```

The notes start as the changelog section; rewrite them for readers of the release page where that helps, including any entry a review fix added. Then tick **The release title and notes are drafted.**

### 4. Merge the version-bump pull request

Once every box is ticked, merge it with a merge commit, like any other pull request into `development`. Nothing else may merge into `development` until `release-publish` has merged the release. Publish checks that the tip of `development` is exactly this merge, so that `main` receives what `release-prepare` tested.

### 5. Open the release pull request

```bash
just release-prepare 2.7.0
```

The same command, run again, sees that the bump has merged and opens **Release v2.7.0** from `development` into `main`. Its body links the version-bump pull request and carries no checklist; there is nothing to review on it. If the draft release has gone missing, it creates it again from the changelog on `development`. It prints both URLs and stops.

### 6. Get the maintainer's approval

Give the maintainer the exact title and notes now on the draft, together with the release's section of `docs/user-vault/What's new.md`, and stop. Wait for the maintainer to reply "approved". The rule and the cases where it must be asked again are in [Approval](#approval).

### 7. Publish

```bash
HATCHDOOR_RELEASE_HOOK=/path/to/hook just release-publish 2.7.0
```

Before merging, it refuses unless all of these hold:

- The release hook is set and is an executable file.
- **Release v2.7.0** is open from `development` into `main` with that exact title.
- The version-bump pull request from `docs/release-v2.7.0` has merged, and every checklist item on it is present in its exact wording, and ticked.
- The draft release for `v2.7.0` exists, is still a draft, and has a title and notes.
- `## Unreleased` on `development` has no entries.
- The tip of `development` is the merge commit of the version-bump pull request.
- No `v2.7.0` tag exists yet.

Then it:

1. Merges the release pull request with a merge commit, pinned to the `development` tip it checked, so GitHub refuses the merge if `development` moved in between.
2. Checks that the merge commit is on `origin/main` and that its second parent is the version-bump merge.
3. Creates the annotated tag `v2.7.0` on the merge commit and pushes it.
4. Runs `<hook> images 2.7.0`.
5. Publishes the draft release and marks it latest.
6. Runs `<hook> deploy 2.7.0`.

The release goes public at step 5, once the images exist. A failed deploy does not hold it back.

## Judgment checklist

These are the checks only a reader can make. `release-prepare` puts them on the version-bump pull request as tick boxes, and the agent does each one and ticks it before the bump merges.

- **The changelog section lists everything that shipped since the previous release, and nothing that did not.** Compare the `## v<version>` section against the pull requests merged into `development` since the last release tag (`git log --merges --first-parent v<previous>..origin/development`). Nothing listed may have been reverted or superseded, and every user-facing change needs an entry.
- **The README describes what shipped.** New features, endpoints, settings and MCP tools are documented, nothing it describes was removed or changed without an update, and image-tag examples name this version.
- **The roadmap is current.** Work this release finished is marked done or removed in [`docs/roadmap/`](../roadmap/product-roadmap.md), and version hints that no longer match are corrected.
- **The notes named by `just docs-freshness main` were read, and the review was recorded with `just docs-freshness-ack main`.** The command's output sits under this box on the pull request. Open every note it names, compare it with what the release changed, then run `just docs-freshness-ack main` on the bump branch. The pasted list is taken when the pull request opens; after a review fix, run `just docs-freshness main` on the bump branch again and read anything new it names before acknowledging.
- **No ADR is contradicted by what shipped.** An ADR the release went against must be amended or superseded, not left stale. The same goes for the architecture records and the module map.
- **An MCP conformance run is recorded, if MCP behavior changed.** If the release touches `/mcp`, the MCP tools or MCP-facing security, record a clean run per [`mcp-conformance-run.md`](./mcp-conformance-run.md). Otherwise tick the box.
- **The release title and notes are drafted.** Step 3.

A check that fails is fixed on the bump branch, as in step 2, and its box is ticked once the fix is pushed. Never tick a box that is not true. A fix merged into `development` by another pull request after the bump makes `release-publish` refuse, so the bump branch is the only way in.

## Approval

Only the maintainer approves a release's title and notes, and the approval is given in the agent's session. This is a rule the agent follows; no script checks it.

1. After the judgment checklist on the version-bump pull request, the agent gives the maintainer the release title and notes as they stand on the draft GitHub Release, and the release's What's new highlights as they stand on the bump branch, and stops.
2. The agent runs `just release-publish` only after the maintainer replies "approved" to that draft in the current session. The approval covers the highlights too ([ADR-42](../adr/README.md#adr-42--every-release-ships-plain-highlights)).
3. A resumed or new session that cannot see that reply in its own conversation asks again.
4. If the title, the notes or the highlights changed after the reply, the agent asks again.

`release-publish` publishes whatever is on the draft when it runs, so the draft on GitHub must be the text that was approved. Approval also authorizes the hook's deploy step, which pushes to the deployment configuration of the servers that run releases.

## Highlights

Every release adds a section to the manual's What's new page, `docs/user-vault/What's new.md` ([ADR-42](../adr/README.md#adr-42--every-release-ships-plain-highlights)). The What's new pop-up, Help and agents all read it, so it is written for someone who does not read changelogs:

```markdown
## v2.8.0 - 2026-10-20

- **Action needed:** Installs on 2.4.x or earlier must upgrade to a 2.5.0 to 2.7.x release first. [[Install Hatchdoor with Docker Compose]]
- A plain line about something you can now do.
```

- 3 to 6 lines in total, action-needed lines first, each starting with `**Action needed:**`.
- One sentence per line, about what a person can now do or must do, not how it was built.
- A line may end with one wikilink to the manual page that explains it. Links elsewhere in the line must point outside the manual.
- The heading is the same `## v<version> - <date>` the changelog uses. Newest release first.

`release-prepare` drafts the section from the changelog's Unreleased entries, one line per entry, with entries under a breaking-changes heading first as action needed. A section already on the page for the version is kept as written. Most releases have more than six entries, so the first run usually refuses with `highlights`: rewrite the section on the bump branch, commit, and run again. `cargo test whats_new`, which `just check-full` runs, checks the format the binary reads.

### For 2.8.0

The 2.8.0 section must include these three lines:

- **Action needed:** an install on 2.4.x or earlier must upgrade to a 2.5.0 to 2.7.x release before 2.8.0.
- The setup checklist, which Help can reopen.
- The opt-in daily check for a newer release.

## The release hook

The build machines, the internal registry and the servers that run releases are not described in this repository. One executable outside it handles all of them. It lives in the maintainer's private repository and is not documented here. `release-publish` knows only this contract:

- `HATCHDOOR_RELEASE_HOOK` holds its path. A bare command name is not looked up on `PATH`.
- It is called from the repository root as `<hook> images <version>` and `<hook> deploy <version>`, with the bare version, such as `2.7.0`. Its output streams to the terminal.
- Exit status 0 means the step succeeded. Anything else fails the run.
- Each step must be safe to run again. `release-publish` records a successful step in a marker under the git directory, `.git/hatchdoor-release/v<version>/<step>-done`, and skips it on the next run. A run on another machine, or after the markers are removed, repeats the step.

`images` must:

- Build and push four tags: `<version>` and `latest` from a Docker BuildKit build, and `podman-<version>` and `podman-latest` from a Podman build, each for amd64 and arm64. The two pairs are separate builds, not copies of each other.
- Copy them to Docker Hub from the fixed version tags, never from a moving tag such as `latest`.
- Skip a tag that already points at the image it would push, and refuse if a different image already sits under the version. A published version is never overwritten.

`deploy` must:

- Pin the servers that run releases to the new version in their deployment configuration, run that configuration's own validation, and push it.
- Refuse if the deployment checkout holds any change other than its own pin for this version.
- Wait until each server reports the new version.
- On failure, print the exact revert that rolls the pin back.

## When a step fails

Every refusal names its check in brackets, as in `release-prepare refused (changelog links): ...`. Fix the cause and run the same command again; it resumes where it stopped.

### `release-prepare`

| Refusal | What happened | What to do |
| --- | --- | --- |
| `clean working tree` | Local changes would leak into the bump commit. | Commit, stash or remove them. |
| `version` | Not a bare version, not higher than the latest `v*` tag, or already published. | Pick the next version. A published one is never reused. |
| `changelog` | `## Unreleased` on `development` is empty, or neither it nor the dated section exists. | Nothing is ready to release, or the changelog was hand-edited. Fix it through an ordinary pull request first. |
| `changelog` | The dated `## v<version>` section on the bump branch has no entries. | Add the entries on `docs/release-v<version>`, commit, and run again. |
| `changelog` | The bump has merged, but `CHANGELOG.md` on `development` has no `## v<version>` section. | Something changed the changelog after the bump. Stop and ask the maintainer. |
| `version files` | The four files do not agree on the version after the bump. | Fix the disagreeing file on `docs/release-v<version>`, commit, and run again. |
| `changelog links` | A `[#N]` in the section has no `[#N]: <url>` definition. | Add the definition above the first release heading on the bump branch, commit, and run again. |
| `highlights` | `docs/user-vault/What's new.md` is missing, has no `## v<version>` section, or its section is not 3 to 6 one-line items with action-needed lines first. | Rewrite the section on the bump branch as in [Highlights](#highlights), commit, and run again. |
| `just check-full` | A test or check failed, on the bump or on a review fix committed after it. | Fix it on the bump branch, commit, and run again. Nothing is pushed until it passes. |
| `bump branch` | `origin/docs/release-v<version>` holds commits the local branch lacks, pushed by hand, from another clone or through GitHub. | Bring them in with `git pull --ff-only`, run `just check-full` yourself, since `release-prepare` does not test a commit already on `origin`, and run again. |
| `bump pull request` | The version-bump pull request was closed without merging. | Reopen it, or delete `docs/release-v<version>` on `origin` and locally to start the version over. |
| `release pull request` | A pull request from `development` into `main` is already open under another title. | Finish or close that release first. GitHub allows only one. |
| `docs-freshness` | The local `main` could not be fast-forwarded to `origin/main`, or `just docs-freshness main` failed for a reason other than handing over its reading list. The bump branch is pushed but no pull request is open. | Make the local `main` match `origin/main` (it should never carry its own commits), or fix what the output names, and run again. |

While the version-bump pull request is open, the command opens nothing. It tests and pushes any commit on the local bump branch that the remote does not have, creates the draft release if a crash lost it, and prints both URLs.

### `release-publish`

| Refusal | What happened | What to do |
| --- | --- | --- |
| `version` | The argument is not a bare version such as `2.7.0`. | Pass the version without a `v` or a suffix. |
| `release hook` | `HATCHDOOR_RELEASE_HOOK` is unset or does not name an executable file. | Set it to the hook's path. |
| `release pull request` | No open or merged pull request from `development` into `main` is titled exactly `Release v<version>`. | Restore the title if it was edited, or run `just release-prepare <version>`. |
| `release checklist` | An item on the merged version-bump pull request is unticked, or its wording was changed or deleted. | Do the check and tick the box, or restore the item's exact text. The bump has merged, so a check that finds something to fix can no longer ship it in this release: stop and ask the maintainer. |
| `draft release` | Before the merge: the draft has no title or notes. | Finish drafting it, and get the maintainer's approval again, since the text changed. |
| `draft release` | Before the merge: there is no draft. | Run `just release-prepare <version>` to create it, redraft the title and notes, and get the maintainer's approval again. |
| `draft release` | The release was published before its pull request merged, or the draft went missing after the merge. | `release-prepare` cannot repair either, because the version then counts as published or tagged. Stop and ask the maintainer. |
| `changelog` | `## Unreleased` on `development` has entries merged after the bump. | Those changes are not in this release's notes. Stop and ask the maintainer: they belong to the next version. |
| `development tip` | The bump has not merged, or something merged into `development` after it. | Merge the bump, or stop and ask the maintainer, as above. |
| `tag` | A `v<version>` tag exists before the merge, the local and remote tags differ, the tag is not annotated, or it points away from the release merge. | A tag is never moved. Delete it by hand only if the version was never published; otherwise work out which is right before going further. |
| `merge commit` | The release merge is not on `origin/main`, its second parent is not the version-bump merge, or the release pull request merged but no merged version-bump pull request from `docs/release-v<version>` exists. | This check runs before tagging, so a first run tagged nothing. Look at what landed on `main` before going further, and stop and ask the maintainer. |
| `images` | The hook's `images` step failed. The tag is pushed and the release is still a draft. | Fix the cause from the hook's output and run again. Publishing waits for the images. |
| `deploy` | The hook's `deploy` step failed. The release is published. | Fix the cause, or apply the revert the hook printed, and run again to retry the deploy. |

### Errors without a check name

A failed `git` or `gh` call, such as a push, a fetch, `gh pr create` or the merge request to GitHub, is not a refusal. It stops the run with the command that failed and its error output, often with a stack trace. Fix the cause, whether authentication, the network, a conflict or branch protection, and run the same command again; it resumes from where git and GitHub say the release stands.

When GitHub refuses the merge, the usual cause is that `development` moved after the checks, because the merge is pinned to the tip that was checked. Running again then reports a `development tip` refusal. A merge conflict or branch protection produces the same kind of error and needs fixing on GitHub first.

A release that turns out broken after publishing is never fixed by republishing its version. Release the next patch version.
