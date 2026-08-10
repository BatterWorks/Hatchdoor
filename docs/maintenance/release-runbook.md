# Release Merge Runbook

Use this procedure after a release pull request from `development` to `main`
has passed review and the user has explicitly authorized its merge.

`development` is the long-lived integration branch. `main` is the public
release branch. Merge `development` into `main` with a squash merge unless the
user explicitly selects a different strategy.

## Pre-merge release checklist

Before approving or merging the release PR, verify:

- The intended release version is identical in `Cargo.toml`, `Cargo.lock`,
  `frontend/package.json`, and `frontend/package-lock.json`.
- `README.md` reflects the release: Docker image-tag examples use the intended
  version and its documented capabilities, configuration, and API references
  do not contradict the release changes.
- `CHANGELOG.md` has an entry for the intended version and records any required
  upgrade action, including a cache rebuild or reindex when applicable.
- `CHANGELOG.md`'s entry for the intended version is complete and accurate:
  every user-facing change actually merged into `development` since the
  previous release is listed, and nothing listed was reverted, superseded, or
  not actually shipped.
- `README.md` describes every user-facing capability actually shipped in this
  release — new features, endpoints, config options, and MCP tools are
  documented, and nothing it describes was removed or changed behavior
  without a corresponding update.
- [`docs/roadmap/`](../roadmap/product-roadmap.md) reflects reality: any
  workstream or item this release completed is marked done or removed from
  the roadmap rather than left as still-pending; horizon/version hints
  (`v2.x`, `v3`) that no longer match are corrected.
- Other affected docs under `docs/` — architecture records, ADRs, the module
  map — are consistent with what was actually built. If an ADR was
  contradicted by this release's changes, it was amended or superseded, not
  silently left stale.

## Merge and realignment

1. Complete the pre-merge release checklist. Confirm the release PR targets
   `main`, has the expected `development` head, and has passed its required
   checks.
2. Squash-merge the PR only after explicit user authorization.
3. Fetch the remotes and confirm the squash commit is present on remote `main`.
4. Warn and stop if the local working tree is dirty.
5. Create a timestamped backup branch from the current `development` tip.
6. Push that backup branch to every configured remote.
7. Reset local `development` hard to `origin/main`.
8. Force-push `development` with `--force-with-lease` to every configured
   remote.
9. Verify the `main` and `development` refs on every remote.

The backup must exist on every remote before resetting or force-pushing
`development`. Never use a bare `git push`: this repository may configure
additional push refspecs. Push each intended branch explicitly.

## Command outline

Run these commands from the repository root after the squash merge. Replace
`<timestamp>` with a UTC timestamp such as `20260727-180000Z`.

```bash
git fetch --prune origin
git log -1 --oneline origin/main
git status --short
git switch development
git branch backup/development-before-main-<timestamp> development
git remote
git push origin backup/development-before-main-<timestamp>
git reset --hard origin/main
git push --force-with-lease origin development
git ls-remote --heads origin main development backup/development-before-main-<timestamp>
```

Repeat the explicit backup push, `--force-with-lease` push, and ref verification
for every additional configured remote. Do not continue if the backup push or
lease check fails; resolve the remote state first.
