# Release Merge Runbook

Use this procedure after a release pull request from `development` to `main`
has passed review and the user has explicitly authorized its merge.

`development` is the long-lived integration branch. `main` is the public
release branch. Merge `development` into `main` with a **merge commit**, the
same way feature branches land on `development`. Squash merging is retired at
every boundary in this repository: it flattened a release's history into one
commit, and it is what forced the branch realignment this runbook used to
carry.

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
- If this release touches `/mcp` or MCP-facing behavior: a clean manual MCP
  conformance run per [`mcp-conformance-run.md`](./mcp-conformance-run.md) is
  recorded as release evidence (manual pre-release step, not a CI gate).
- Other affected docs under `docs/` — architecture records, ADRs, the module
  map — are consistent with what was actually built. If an ADR was
  contradicted by this release's changes, it was amended or superseded, not
  silently left stale.

## Merge

1. Complete the pre-merge release checklist. Confirm the release PR targets
   `main`, has the expected `development` head, and has passed its required
   checks.
2. Merge the PR — with a merge commit — only after explicit user authorization.
3. Fetch the remotes and confirm the merge commit is present on remote `main`,
   and that its second parent is the `development` tip you expected.
4. Verify the `main` and `development` refs on every remote.

No realignment step follows, and none is needed. The merge commit's second
parent *is* the `development` tip, so every commit on `development` is now an
ancestor of `main`: the two branches share history rather than diverging, and
`development` simply carries on from where it was. Nothing is rewritten, so
there is nothing to back up first.

This is the whole reason the procedure used to be longer. A squash merge put a
brand-new commit on `main` that `development` had never seen, which left the
branches permanently divergent and forced a hard reset and force-push to
reconcile them — with a backup branch on every remote to make that survivable.
Merging instead removes the divergence, and with it the rewrite, the backup, and
the force-push. **Never force-push `development` or `main` as part of a
release.**

Never use a bare `git push`: this repository may configure additional push
refspecs. Push each intended branch explicitly.

## Command outline

Run these from the repository root after the merge, to verify it landed as
intended.

```bash
git fetch --prune origin
git log -1 --oneline origin/main
git log -1 --format='%h parents: %p' origin/main   # two parents; the second is development
git merge-base --is-ancestor origin/development origin/main && echo "development is contained in main"
git ls-remote --heads origin main development
```

Repeat the ref verification for every additional configured remote.

## When `main` moves on its own

A hotfix committed directly to `main` is the one case where the branches can
diverge. Bring it back with an ordinary merge — `git switch development && git
merge origin/main` — and push. Do not reset or force-push to reconcile them.
