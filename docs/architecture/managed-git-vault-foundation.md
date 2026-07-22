# Managed Git Vault Foundation — High-Level Architecture

- Status: Draft for discussion
- Scope: First increment of the vault lifecycle roadmap
- Related roadmap: [`docs/roadmap/vault-lifecycle.md`](../roadmap/vault-lifecycle.md)
- Related proposal: [Managed Git Vault Lifecycle, PR #18](https://github.com/BattermanZ/Hatchdoor/pull/18)
- Agent handoff: [`docs/plans/managed-git-vault-agent-handoff.md`](../plans/managed-git-vault-agent-handoff.md)
- Compatibility: Preserve the existing local-directory workflow

## Purpose

This document describes the runtime foundation for one environment-configured
local or managed Git vault. Hatchdoor should acquire, synchronize, index, and
expose the vault without making the application itself unavailable when the
vault or Git remote has a problem.

This is smaller than the complete Phase 1 roadmap outcome. Configuration and
secrets remain deployment-owned; managing them through the UI and supporting
multiple vaults come later.

## Product Promise

An operator can configure one Git repository, start Hatchdoor, and immediately
reach an application that shows the vault being cloned or reopened,
synchronized, and indexed.

If the first clone, authentication, branch, or indexing step fails, the process
keeps serving an actionable status and can retry. If a valid persistent checkout
already exists, a temporary remote failure leaves the last consistent vault
snapshot usable and keeps local bidirectional edits durable for later push.

## Scope

The first increment includes:

- one vault per Hatchdoor process;
- unchanged local-directory mode;
- a persistent managed Git checkout;
- pull-only and bidirectional Git modes;
- public and token-authenticated HTTPS repositories;
- optional branch and vault subdirectory;
- startup acquisition, periodic pulling, writeback, retry, and crash recovery;
- coordinated working-tree and index updates;
- one lifecycle and capability model shared by web and MCP; and
- minimal lifecycle status, retry/sync controls, and UI feedback.

It does not include:

- UI-managed configuration or secret storage;
- multiple vaults or runtime source switching;
- SSH, Git LFS, submodules, sparse, or shallow clones;
- an in-browser merge editor or Git history; or
- multiple Hatchdoor replicas sharing one checkout.

## Principles

1. Markdown remains authoritative; SQLite remains a disposable read model.
2. Application liveness is independent of vault readiness.
3. Existing local deployments keep their current behavior.
4. Git network operations never hold the vault mutation lock.
5. No operation silently discards, resets, or force-pushes local work.
6. Web and MCP observe and enforce the same runtime capabilities.
7. Managed checkout state lives on persistent storage.
8. Credentials never enter URLs, repository configuration, logs, status, or
   commits.
9. The design serves this single-vault increment without prematurely building
   the multi-vault architecture.

## System Shape

```text
  deployment configuration
            |
            v
  +----------------------+       commands       +----------------------+
  | Vault source resolver |---------------------->| Lifecycle manager    |
  +----------------------+                       +----------------------+
            |                                               |
            | resolved source                               | state + capabilities
            v                                               v
  +----------------------+                       +----------------------+
  | Local directory or   |                       | Runtime snapshot     |
  | managed Git checkout |                       +----------------------+
  +----------------------+                                  |
            |                                               |
            v                                               v
  +----------------------+    atomic index publish  +-------------------+
  | Markdown vault       |-------------------------->| SQLite read model |
  +----------------------+                           +-------------------+
            ^                                               |
            |                                               v
       web/MCP writes                                web API, MCP, SPA
```

The source and lifecycle layers resolve either source into an ordinary
filesystem vault. Search, rendering, downloads, attachments, and note operations
continue to use that filesystem boundary.

## Runtime Model

The names below are descriptive rather than a prescribed Rust API.

### Vault source

```text
VaultSource
  Local { vault_root, legacy_git_sync }
  ManagedGit {
    repository_url,
    checkout_root,
    branch,
    vault_subdirectory,
    mode,
    credentials,
    poll_interval
  }
```

`Local` preserves `VAULT_PATH`. In managed mode the persistent checkout is the
source; `VAULT_PATH` is not a second source of truth.

### Runtime snapshot

```text
VaultRuntimeSnapshot
  phase
  source and mode
  repository_root: optional path
  vault_root: optional validated path
  capabilities
  acquisition/index/sync progress
  sanitized repository status
  last error and retryability
```

Paths are optional because Hatchdoor must exist before a vault has been cloned
or validated. The snapshot replaces separate, potentially contradictory startup
and Git-sync status concepts.

### Capabilities

```text
VaultCapabilities
  browse
  search
  mutate
  pull
  push
  retry
```

Capabilities are derived from source mode and lifecycle phase, then combined
with existing web, demo, and MCP security policy. Pull-only and conflict states
always disable mutation. Route and tool guards enforce the result; the frontend
uses the same snapshot for presentation.

## Lifecycle

Application liveness and vault readiness are distinct:

- `/health` means Hatchdoor can serve its application.
- `/ready` means a usable vault index has been published.
- lifecycle status explains initialization, degraded operation, or failure.

```text
unconfigured
    |
    v
validating -> acquiring -> synchronizing -> indexing -> ready
     |            |              |             |
     +------------+--------------+-------------+-> unavailable -> retry

ready -> syncing -> ready
  |         |
  |         +-> degraded -> retry
  |
  +-> conflict -> read-only recovery -> syncing
```

`Unavailable` means there is no usable published vault snapshot. `Degraded`
means Hatchdoor can continue serving an existing snapshot while a remote or
optional operation fails.

## Startup Behavior

### Local source

The server starts, resolves `VAULT_PATH`, applies the existing starter-vault
policy, builds the index in the background, and then starts the watcher and
optional legacy Git sync. No current deployment should need to change.

### First managed Git startup

1. Start the HTTP application in `validating` state.
2. Validate source configuration and acquire the checkout ownership lock.
3. Clone into an application-owned temporary sibling directory.
4. Validate origin, branch, repository shape, and vault boundary.
5. Atomically install the checkout on persistent storage.
6. Apply the explicit managed empty/bootstrap policy.
7. Build and publish the index.
8. Enter ready state and start polling/writeback.

Failure moves the vault to `unavailable`; it does not stop the HTTP server.

### Subsequent managed startup

Hatchdoor locks and validates the existing checkout, detects dirty files and
unpushed commits, fetches when possible, and publishes the resulting index. If
the remote is unavailable, a previously validated checkout can start as
`degraded` and recover in the background.

## Repository Lifecycle Manager

The current write-triggered Git task evolves into one actor receiving:

- startup preparation or recovery;
- periodic pull ticks;
- debounced Hatchdoor write records;
- manual retry/sync commands;
- retry timers; and
- graceful shutdown.

For each synchronization cycle:

```text
fetch without vault lock
          |
          v
acquire vault lock
  preserve eligible dirty vault changes
  re-read local and remote refs
  fast-forward or merge
  revalidate vault boundary
  rebuild/publish index if content changed
  broadcast one vault revision
release vault lock
          |
          v
push without vault lock when required
```

The graph must be re-read after acquiring the lock because a web or MCP write
may finish while fetch is running. A push rejected because another writer moved
the remote enters a small bounded fetch/reconcile/push loop, then falls back to
backoff while retaining local commits.

The main graph actions are:

| State | Action |
| --- | --- |
| Equal | No working-tree change |
| Remote ahead only | Fast-forward and reindex |
| Local ahead only | Push in bidirectional mode |
| Both ahead, clean | Merge, reindex, and push in bidirectional mode |
| Both ahead, conflict | Preserve local head and enter conflict state |

Unexpected local history in pull-only mode is preserved and reported, never
reset implicitly.

## Working Tree and Index Consistency

Checkout, merge, reset, note mutation, and indexing must not race. Watcher
refreshes therefore coordinate through the same vault mutation lock as web, MCP,
and Git working-tree changes.

Readers continue using the previous SQLite snapshot while a replacement is
built. The new snapshot and vault revision become visible only after a successful
transaction. A failed later rebuild retains the previous snapshot and marks the
vault degraded.

Manager-generated filesystem events are coalesced so one working-tree transition
produces one index publication and one revision event.

## Vault Subdirectory Boundary

Repository root and vault root remain distinct. The configured subdirectory is
relative, canonicalized, and contained within the repository root.

Containment is revalidated after every checkout, merge, reset, or recovery. This
prevents a remote commit from replacing the vault directory with an escaping
symlink.

In subdirectory mode Hatchdoor stages only the validated vault subtree. Dirty
files elsewhere in the repository are never committed, reset, or deleted by
Hatchdoor; if they prevent safe integration, the lifecycle reports an actionable
condition.

## Empty Repositories and Recovery

Local mode retains automatic starter-vault seeding. Managed mode does not seed
merely because a repository has no Markdown.

An empty or unborn remote may be bootstrapped only in bidirectional mode with an
explicit option. Pull-only mode rejects bootstrap and never creates a commit it
cannot push. Source preparation therefore owns seeding; generic index
construction does not.

Startup recovery classifies repository operation state, dirty vault files,
outside-vault dirt, and unpushed commits separately. It must preserve dirty vault
content before any hard reset or operation cleanup. Bidirectional dirty work is
committed and retried; pull-only dirt becomes an actionable degraded condition.

## Conflict Policy

On a conflict Hatchdoor:

1. records the conflicting paths and both heads;
2. restores the last clean local head without discarding it;
3. creates a unique conflict-preservation branch;
4. attempts to publish that branch according to the chosen product policy;
5. enters conflict state and disables web/MCP mutation; and
6. continues serving the last consistent local snapshot read-only.

Hatchdoor never force-pushes or chooses a conflicting version. Once the remote
configured branch contains the preserved work, polling or manual sync can return
the vault to ready state.

## Configuration, API, and UI

Configuration remains deployment-owned and restart-bound. It defines source,
URL, branch, mode, optional subdirectory, persistent checkout location, polling,
bootstrap, and optional HTTPS credentials. Public repositories require no token;
private credentials are passed only through `git2` callbacks.

The first operational surface includes:

- lifecycle status and progress;
- sanitized branch, ahead/behind, dirty, conflict, and operation timestamps;
- redacted actionable errors;
- manual retry/sync;
- shared write capabilities; and
- a minimal initialization view and ready/degraded/conflict indicator.

It does not include configuration forms or secret entry.

## Deployment and Compatibility

The managed checkout and generated cache use separate persistent locations, for
example:

```text
/data/repositories/vault   managed non-bare checkout
/data/cache                generated SQLite cache
```

The runtime remains rootless and distroless and uses the existing `git2`
dependency. A process-lifetime filesystem lock enforces one process per managed
checkout.

When the source setting is absent or local, current `VAULT_PATH`, starter
seeding, watcher, writes, and optional legacy Git sync remain unchanged. Existing
host-managed checkouts are not automatically adopted.

## Decisions to Resolve

1. Is a nonempty managed repository without Markdown a valid empty vault or an
   unavailable source?
2. What polling interval should be the default, and can polling be disabled?
3. What fixed immediate retry limit applies to push races?
4. Should conflict-preservation branches be published automatically?
5. Does `/ready` stay successful whenever a previously published snapshot is
   usable, including degraded and conflict states?
6. What minimum sanitized repository identity should status expose?
