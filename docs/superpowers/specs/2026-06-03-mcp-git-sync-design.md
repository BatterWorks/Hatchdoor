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
  enabled. Opens the vault as a git repo and confirms: it is a repo whose root
  is the vault path, `HEAD` is on the configured branch (not detached or on a
  different branch), the configured remote exists, and a token is present. If
  git sync is enabled but any of this is wrong, **fail fast at startup** —
  consistent with how the SQLite cache refuses to start on bad config.
- After validation, if the local branch has commits the remote lacks (a push
  stranded by an earlier outage/restart), trigger one sync attempt at startup so
  those commits aren't stuck until the next MCP write.
- `sync(batch)` — the single entry point that performs
  stage → commit → fetch → integrate → push.

### Background sync task

A single tokio task, owned by `AppState`:

- Every MCP write tool, *after* its filesystem write succeeds, sends a
  `WriteRecord { op, affected_paths, summary: Option<String> }` down an
  `mpsc` channel and returns to the agent immediately. `affected_paths` is the
  **complete** set of vault paths the operation created, modified, or removed —
  including the old+new paths of a rename and any other notes whose wikilinks
  were rewritten (see `rewrites.rs`). The write layer already computes this set.
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
manual-commit workflow keeps working, and unrelated uncommitted local edits are
deliberately left untouched (see "Staging strategy").

### Vault-mutation lock

The background task serializes git operations against each other, but **not**
against incoming MCP writes. A merge or a conflict `reset --hard` rewrites files
on disk; a concurrent MCP write to the same note would race it (atomic rename
narrows but does not close the window).

A shared async lock (`Arc<Mutex>` / `RwLock`) guards vault mutation. Both the
MCP write path and `git::sync`'s tree-mutating steps (merge, reset, checkout)
acquire it, so a sync never overlaps a write. The debounce wait happens
*outside* the lock; only the actual git tree mutation holds it, keeping write
latency unaffected in the common case.

## `git::sync(batch)` semantics

1. **Stage** — stage exactly the `affected_paths` collected from the batch's
   `WriteRecord`s (see "Staging strategy"), capturing additions, modifications,
   **and** removals, then write the tree.
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
   for token auth and is env-overridable. The token is supplied only via the
   credentials callback — never written into `.git/config` and never logged.

### Secret handling

The HTTPS token must never appear in `tracing` output or in any error message,
status field, or commit metadata. All error paths that wrap git2 errors redact
credentials before surfacing or logging them.

### Staging strategy

Stage only the `affected_paths` reported by the batch — **not** a blanket
`add_all("*")`. Rationale:

- **Accurate commits.** Contents exactly match the commit message; deletions and
  renames (which `add_all` would miss without `update_all`) are captured because
  the write layer reports the removed/renamed paths explicitly.
- **No surprise commits.** Unrelated work-in-progress from the local editing
  agent is left uncommitted rather than being scooped into an MCP-labeled commit
  and pushed prematurely.
- **No tmp-file risk.** A path Hatchdoor didn't write (e.g. `*.md.hatchdoor-tmp`)
  is never staged.

Each rename/move stages its old and new paths plus any other notes whose
wikilinks were rewritten; the write layer already knows this full set.

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

- **Write/sync race** — guarded by the vault-mutation lock (see above); a sync's
  tree mutation never overlaps an MCP write.
- **Deletions / renames** — staged explicitly via the batch's `affected_paths`,
  including wikilink-rewrite side effects on other notes.
- **Stranded unpushed commits after restart** — startup runs a sync attempt when
  the local branch is ahead of the remote.
- **Post-fetch push race** — if the remote moves again between our fetch and
  push, the push is rejected non-fast-forward; this self-heals by re-fetching
  and merging on the next batch.
- **Token leakage** — redacted on every log and error path; never persisted to
  `.git/config`.
- **Branch/HEAD mismatch** — rejected at startup validation.
- Transient auth/network errors leave the commit local and retry next batch.
- Only one sync runs at a time (single background task).
- Unrelated local uncommitted edits are left untouched (precise staging).
- A merge that pulls in remote changes can invalidate an `expected_content_hash`
  an agent is holding; the next MCP write to that note correctly returns the
  existing hash-conflict error — expected behavior, not a bug.

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
- Precise staging commits a note delete and a note rename (with wikilink-rewrite
  side effects on other notes), and leaves an unrelated uncommitted file alone.
- Startup validation fails fast when enabled-but-misconfigured (not a repo,
  detached/wrong branch, missing remote, missing token).
- Startup sync pushes commits that were stranded ahead of the remote.
