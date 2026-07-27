# Release Merge Runbook

Use this procedure after a release pull request from `development` to `main`
has passed review and the user has explicitly authorized its merge.

`development` is the long-lived integration branch. `main` is the public
release branch. Merge `development` into `main` with a squash merge unless the
user explicitly selects a different strategy.

## Merge and realignment

1. Confirm the release PR targets `main`, has the expected `development` head,
   and has passed its required checks.
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
