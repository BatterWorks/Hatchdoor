# MCP Git Sync Design

**Date:** 2026-06-03
**Status:** Approved (pending spec review)

## Problem

Hatchdoor serves an Obsidian vault that is kept in sync across devices through a
git repository. Until now, an agent running on the same machine as Hatchdoor
committed and pushed vault changes by hand. The user is now driving Hatchdoor
from agents on *other* machines via the write-capable MCP tools.

Those remote agents cannot run git on the host. Their writes land on disk
(through the existing atomic-write path) but are never committed or pushed, so
the user's other devices never receive them. This feature closes that gap:
successful MCP writes should be committed and pushed automatically.

## Goals

- When MCP write tools modify the vault, Hatchdoor commits and pushes the
  changes to the configured git remote so they propagate everywhere.
- Pull/fetch before pushing so the server clone stays current.
- Never silently lose either the agent's edit or an incoming remote change.
- Don't add network latency to MCP tool responses.

## Non-goals

- Replacing the user's local manual-commit workflow (it keeps working).
- Triggering git on non-MCP file changes (e.g. the local editing agent).
- SSH-based push (HTTPS token only, for now).
- A merge/conflict resolution UI. Conflicts are surfaced for a human to resolve
  on the server.

## Decisions (from brainstorming)

| Question | Decision |
|----------|----------|
| Git scope | Pull/fetch → commit → push (full sync). |
| Timing | Debounced background sync; failures surfaced, not blocking. |
| Conflicts | Abort cleanly, keep the local commit, surface the error. Never auto-discard. |
| Push auth | HTTPS token supplied via env var. |
| Commit message | Agent-supplied optional summary per write; auto-generated fallback. |
| Debounce default | 30s (env-configurable). |
| Git mechanism | `git2` crate (libgit2), OpenSSL vendored — runtime image stays distroless. |

## Architecture

A new opt-in subsystem, **off by default**, that turns successful MCP writes
into git commits + pushes without blocking the agent.

### New module `src/git/`

Wraps the `git2` crate. Two responsibilities:

- `open_and_validate(vault_path, config)` — called at startup when git sync is
  enabled. Opens the vault as a git repo and confirms the configured remote and
  branch exist. If git sync is enabled but the repo/remote/token is
  misconfigured, **fail fast at startup** — consistent with how the SQLite
  cache refuses to start on bad config.
- `sync(batch)` — the single entry point that performs
  stage → commit → fetch → integrate → push.

### Background sync task

A single tokio task, owned by `AppState`:

- Every MCP write tool, *after* its filesystem write succeeds, sends a
  lightweight `WriteRecord { op, paths, summary: Option<String> }` down an
  `mpsc` channel and returns to the agent immediately.
- The task collects records and waits for a quiet window
  (`HATCHDOOR_GIT_DEBOUNCE_SECONDS`, default 30s), then drains **all** pending
  records into one batch and calls `git::sync`.
- Because it is a single task, only one sync ever runs at a time — no
  overlapping git operations.
- The outcome is written to a shared `Arc<RwLock<GitSyncStatus>>`
  (last sync time, ok/err, last error text, pending count, unpushed-commit
  count).

This shape decouples agent latency from network sync, coalesces bursts into few
commits, and serializes git access. Only MCP writes trigger it; the local
manual-commit workflow keeps working, and any stray uncommitted local edits are
simply swept into the next `add -A`.

## `git::sync(batch)` semantics

1. **Stage** — `index.add_all(["*"])`, write tree. `.gitignore` excludes
   Hatchdoor's tmp files (`*.md.hatchdoor-tmp`); the cache DB already lives
   outside the vault, so neither is committed.
2. **Nothing to do?** — if the tree matches HEAD *and* there are no unpushed
   commits, return a no-op. Never create empty commits.
3. **Commit** — author/committer from `HATCHDOOR_GIT_AUTHOR_NAME` /
   `HATCHDOOR_GIT_AUTHOR_EMAIL`, falling back to repo config, then
   `Hatchdoor <hatchdoor@localhost>`. Message:
   - Title line auto-generated from the file ops, e.g.
     `hatchdoor: update "Project X", create "Meeting notes" (2 files)`.
   - Body lists any agent-supplied summaries from the batch.
4. **Fetch** the configured remote branch.
5. **Integrate:**
   - If our branch is up-to-date or strictly ahead (the push will
     fast-forward) → go to push.
   - Else (remote moved) → attempt an **in-memory merge** of the remote into
     our branch:
     - **Clean** → create a merge commit, continue to push.
     - **Conflict** → `reset --hard` back to our just-made commit (abort,
       working tree restored), **do not push**, return a `Conflict` error
       naming the conflicted file(s).
6. **Push** to the remote branch using HTTPS-token credentials (`git2`
   `RemoteCallbacks` + `Cred::userpass_plaintext`). Username defaults sensibly
   for token auth and is env-overridable.

### Failure behavior (all non-fatal to the server)

- **Auth failure / network down** on fetch or push → the commit stays local;
  status records the error; the *next* batch retries the push automatically.
  Transient failures self-heal.
- **Conflict** → committed locally but unpushed; surfaced for a human to
  resolve on the server; the next sync pushes the resolution.

## Error surfacing

Implements the "debounced, but surface errors" choice with two channels:

- A new read-only MCP tool **`get_git_sync_status`** returns the current
  `GitSyncStatus` (last sync time, ok/error, last error text, pending count,
  unpushed-commit count).
- When the most recent background sync **failed**, write-tool responses carry a
  short `git_sync_warning` field so an agent notices without polling.
- Everything is also logged via `tracing`.

## Configuration

All git sync is opt-in.

| Env var | Default | Purpose |
|---------|---------|---------|
| `HATCHDOOR_GIT_SYNC_ENABLED` | `false` | Master switch. |
| `HATCHDOOR_GIT_REMOTE` | `origin` | Remote name; URL comes from repo config. |
| `HATCHDOOR_GIT_BRANCH` | current branch | Branch to commit/push. |
| `HATCHDOOR_GIT_HTTPS_TOKEN` | — | Push token (required when enabled). |
| `HATCHDOOR_GIT_HTTPS_USERNAME` | sensible token default | Override for providers needing a specific value. |
| `HATCHDOOR_GIT_DEBOUNCE_SECONDS` | `30` | Quiet window before a batch syncs. |
| `HATCHDOOR_GIT_AUTHOR_NAME` | repo config / fallback | Commit author name. |
| `HATCHDOOR_GIT_AUTHOR_EMAIL` | repo config / fallback | Commit author email. |

Documented in `.env.example` and `README.md`.

## Build / Docker

Add `git2` with `vendored-openssl` (and bundled libgit2) so OpenSSL links
statically and the **runtime image stays distroless**. The builder stage
already provides `libssl-dev`, `g++`, and `pkg-config`, so the runtime
Dockerfile needs no change.

## Watcher interaction

- Confirm the vault watcher ignores `.git/` so git's own churn and incoming
  merges don't cause reindex storms.
- Incoming remote notes from a merge flow through the existing reindex path so
  the SQLite cache and UI update normally.

## Edge cases

- Transient auth/network errors leave the commit local and retry next batch.
- Only one sync runs at a time (single background task).
- Stray local uncommitted edits get swept into the next commit.

## Testing

`git2` supports the local `file://` transport, so all git tests run against a
bare "remote" repo on disk — no network required.

- Stage → commit → push, fast-forward to the bare remote.
- Remote moved, **clean** merge → merge commit created and pushed.
- Remote moved, **conflicting** → sync aborts, working tree returns to our
  commit, nothing pushed, `Conflict` error names the file.
- No-op when tree unchanged and nothing unpushed.
- Commit-message assembly (auto title + agent-supplied body lines).
- Debouncer coalesces multiple `WriteRecord`s into a single sync.
- Startup validation fails fast when enabled-but-misconfigured.
