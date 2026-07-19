# Managed Git Vault Lifecycle

- Status: Proposal
- Audience: Hatchdoor maintainers and contributors
- Scope: Vault acquisition, persistence, synchronization, and operational lifecycle
- Compatibility: Preserve the existing local-directory workflow

## Intent and Rationale

The intent is to make a Git repository URL a first-class Hatchdoor vault source.
An operator should be able to provide the URL and credentials, give Hatchdoor a
persistent storage location, and let Hatchdoor acquire the repository and manage
its operational lifecycle without a separately maintained host checkout.

Managed lifecycle does not mean that Hatchdoor becomes the exclusive repository
writer. The configured branch remains a shared collaboration boundary. People
using Obsidian or another Git client, automation, and other services may all push
independently from separate clones. Hatchdoor must ingest those changes and
reconcile its own writes using normal Git concurrency semantics.

This belongs inside Hatchdoor rather than only in an entrypoint or deployment
script because repository changes affect more than the files on disk. Pulls and
merges must be coordinated with Hatchdoor writes, SQLite reindexing, filesystem
watching, cache revision events, health, and user-visible error status. Keeping
that coordination in one process gives Hatchdoor a consistent view of the
working tree without replacing Git as the source of collaboration history.

The managed checkout also serves as a durable local buffer. A temporary network
failure must not lose a Hatchdoor edit, and an independently pushed remote change
must not silently overwrite one. Persistent storage, explicit reconciliation,
and observable conflict state provide that durability while the existing local
directory workflow remains available to operators who prefer to own the checkout
themselves.

## Summary

Hatchdoor currently expects `VAULT_PATH` to point to a local directory. Optional
Git synchronization can commit and push Hatchdoor writes, but the directory must
already be a correctly configured Git checkout.

This proposal adds a managed Git vault source. An operator supplies a Git URL,
and Hatchdoor clones the repository into persistent application-managed storage,
selects the vault directory, keeps the checkout synchronized, and exposes its
lifecycle state. The existing local-directory mode remains supported.

The proposed model is:

```text
Git URL -> managed persistent checkout -> vault path -> index/cache -> Hatchdoor
                         ^                                   |
                         |---------- pull/push --------------|
```

The main architectural recommendation is to keep the vault/indexing layer
filesystem-based. A repository manager resolves either a local source or a
managed Git source into an ordinary local `vault_path`; search, rendering,
attachments, downloads, and note operations continue to use that path.

## Motivation

The current deployment contract requires operators to perform part of the vault
lifecycle outside Hatchdoor:

1. Clone a repository on the host.
2. Check out the expected branch.
3. Configure its remote.
4. Mount that checkout into the container.
5. Ensure the checkout and mount have compatible permissions.
6. Keep the checkout available across container replacement.

This works, but it makes Git-backed deployment more complicated than necessary
and divides ownership between Hatchdoor and an external process. It also does
not provide continuous ingestion of remote changes: the current Git sync task
fetches the remote as part of a write-triggered push, rather than polling for
inbound changes independently.

A managed source would make a Git repository a first-class vault input while
retaining Markdown files as the source of truth.

## Current Behaviour

Today:

- `VAULT_PATH` selects one filesystem directory.
- The directory is seeded with starter notes if it contains no Markdown files.
- Hatchdoor indexes it into the generated SQLite cache.
- A filesystem watcher reindexes relevant changes.
- Optional Git sync requires the vault directory to already be a non-bare Git
  repository root.
- The checkout must already be on `HATCHDOOR_GIT_BRANCH` and contain the
  configured remote.
- Git sync batches successful Hatchdoor writes, commits the whole working tree,
  fetches, integrates the remote, and pushes.
- Remote/network failures are retried with backoff.
- Merge conflicts abort without discarding the local commit.
- Startup flushes commits that are ahead of the remote.

This is a strong base for writeback, but it does not clone repositories, manage
checkout storage, poll independently for inbound changes, or fully recover an
uncommitted dirty tree after a crash.

## Goals

- Allow an operator to configure a vault using an HTTPS Git URL.
- Clone the repository without requiring a Git executable in the runtime image.
- Persist and reuse the checkout across process and container restarts.
- Pull remote changes at startup and on a configurable interval.
- Reindex after a working-tree update so clients see a consistent vault state.
- Optionally commit and push Hatchdoor writes using the existing writeback
  behaviour.
- Preserve local changes across network outages, process crashes, and merge
  conflicts.
- Expose enough status to diagnose acquisition, authentication, divergence, and
  conflict problems.
- Keep the existing `VAULT_PATH` deployment mode backward compatible.
- Support a vault located in an optional subdirectory of the repository.

## Non-goals for the Initial Version

- Multiple vaults in one Hatchdoor process.
- Switching repository URLs without restarting Hatchdoor.
- Multiple Hatchdoor replicas sharing one checkout.
- SSH authentication.
- Git LFS materialization.
- Recursive submodule management.
- Sparse or shallow clones.
- Provider-specific pull request creation or conflict resolution.
- An in-browser Git history or merge editor.

These can be considered independently after the basic lifecycle is reliable.

## Proposed Source Model

Introduce an explicit vault source configuration with three operational modes:

| Mode | Checkout owner | Remote ingestion | Hatchdoor writes | Git push |
| --- | --- | --- | --- | --- |
| `local` | Operator | Filesystem watcher | Existing behaviour | Optional legacy sync |
| `git-pull-only` | Hatchdoor | Startup and polling | Disabled | No |
| `git-bidirectional` | Hatchdoor | Startup and polling | Enabled | Yes |

Pull-only mode must disable application-level writes rather than merely
disabling push. Otherwise local mutations could drift from the configured
remote and later be overwritten or left permanently unpushed.

Internally, configuration should resolve to an enum similar to:

```text
VaultSource
  LocalPath { vault_path }
  ManagedGit {
    url,
    checkout_path,
    branch,
    vault_subdirectory,
    mode,
    credentials,
    poll_interval
  }
```

Both variants produce a resolved runtime object containing at least:

```text
ResolvedVault
  repository_root: optional path
  vault_root: path
  source_mode
  write_capabilities
```

The rest of the application continues to consume `vault_root` as it consumes
`VAULT_PATH` today.

## Configuration Sketch

The exact names can be adjusted during implementation. A possible environment
contract is:

```env
HATCHDOOR_VAULT_SOURCE=git
HATCHDOOR_VAULT_GIT_URL=https://github.com/example/my-vault.git
HATCHDOOR_VAULT_GIT_BRANCH=main
HATCHDOOR_VAULT_GIT_CHECKOUT_PATH=/data/repositories/vault
HATCHDOOR_VAULT_GIT_SUBDIR=
HATCHDOOR_VAULT_GIT_MODE=bidirectional
HATCHDOOR_VAULT_GIT_POLL_SECONDS=60

HATCHDOOR_GIT_HTTPS_USERNAME=hatchdoor
HATCHDOOR_GIT_HTTPS_TOKEN=
```

Recommended semantics:

- `HATCHDOOR_VAULT_SOURCE` defaults to `local` for backward compatibility.
- In local mode, `VAULT_PATH` behaves exactly as it does today.
- In Git mode, Hatchdoor derives the runtime vault path from the checkout path
  and optional vault subdirectory; `VAULT_PATH` is not a second source of truth.
- If the branch is omitted, the remote default branch is selected and the
  resolved branch is retained in status.
- Credentials are optional for public repositories.
- Credentials must not be embedded in the URL, logged, returned through status,
  or stored in repository configuration.
- Token-file support should be considered alongside the environment variable
  for Docker and orchestrator secrets.
- Only HTTPS should be accepted initially. Plain HTTP should require an explicit
  insecure opt-in, if supported at all.

The existing `HATCHDOOR_GIT_SYNC_ENABLED` configuration can remain available to
local-mode users during a compatibility period. Managed mode should use the
source mode as the authority for whether pull and writeback are enabled.

## Persistent Checkout Storage

The managed checkout must live on persistent writable storage, not in the
container's ephemeral layer. It can contain:

- Unpushed local commits.
- Dirty files written immediately before a crash.
- Objects needed to recover from a remote outage.
- State needed to diagnose or preserve a conflict.

A Compose deployment could use:

```yaml
volumes:
  - ${HOST_REPOSITORY_DATA_PATH:-./data/repositories}:/data/repositories
  - ${HOST_CACHE_PATH:-./data/cache}:/data/cache
```

A named volume is also suitable. The generated SQLite cache remains outside the
repository.

Hatchdoor should hold a process-lifetime ownership lock outside the working tree
or within Git metadata. This prevents two local Hatchdoor processes from
managing the same checkout. Deployments must use one replica per managed
checkout; a filesystem lock does not coordinate replicas on different hosts.

## Startup Lifecycle

Repository preparation must occur before starter-vault seeding and the initial
index build.

### First Startup

1. Validate source configuration, URL scheme, branch, subdirectory, and mode.
2. Acquire the checkout ownership lock.
3. Confirm the final checkout path is absent.
4. Clone into an application-owned temporary sibling directory using `git2` and
   authenticated fetch callbacks.
5. Validate the resulting non-bare working tree, origin URL, branch, and vault
   subdirectory.
6. Atomically rename the temporary checkout into the configured final path.
7. Resolve the vault root.
8. Build the initial SQLite index.
9. Start the watcher, repository lifecycle task, and HTTP server.

Cloning into a temporary location prevents an interrupted clone from appearing
to be a valid managed checkout. Hatchdoor may clean up only temporary paths it
can prove it created; it must not delete an unknown destination.

### Subsequent Startup

1. Acquire the checkout ownership lock.
2. Open and validate the existing checkout.
3. Verify that its origin matches the configured repository URL.
4. Verify or restore the configured branch without discarding local work.
5. Detect interrupted Git state, dirty files, and commits ahead of the remote.
6. Fetch and integrate the configured branch.
7. Resolve the vault root and build the index from the resulting checkout.
8. Start background polling and writeback.

If the remote is unavailable but a previously validated checkout exists,
Hatchdoor should start from the last-known checkout in a visible degraded state
and retry in the background. If no successful checkout exists, clone,
authentication, or branch failure is fatal.

If an existing destination is not a repository, has a different origin, or
cannot be proven to belong to this managed source, Hatchdoor must refuse to
start rather than overwrite or adopt it silently.

## Runtime Repository Manager

The current write-triggered sync task should evolve into a repository lifecycle
actor. It receives:

- A periodic pull tick.
- A debounced batch of Hatchdoor write records.
- A manual sync request.
- A retry timer after transient failures.
- A graceful shutdown request.

Repository work should retain the current split between network and working-tree
phases:

```text
fetch without the vault lock
          |
          v
compare local and remote commit graph
          |
          v
acquire vault lock
  commit pending local files when appropriate
  fast-forward or merge
  refresh the SQLite index
  broadcast a vault revision
release vault lock
          |
          v
push without the vault lock when needed
```

Slow fetches and pushes must not prevent vault writes. Checkout, merge, reset,
note mutation, and any index build that reads the working tree must not race one
another.

The filesystem watcher should also coordinate its refresh through the vault
lock. Repository-manager checkouts should explicitly refresh after a changed
HEAD; relying only on eventual filesystem events would create a period where the
working tree and cache disagree. Self-generated watcher events may be coalesced
or skipped using the indexed commit ID.

## Integration Rules

After fetch, the manager should distinguish the commit graph explicitly:

| State | Action |
| --- | --- |
| Local equals remote | No operation |
| Remote ahead, local not ahead | Fast-forward and checkout |
| Local ahead, remote not ahead | Push in bidirectional mode |
| Both ahead, merge is clean | Create merge commit, refresh, and push |
| Both ahead, merge conflicts | Abort merge safely and enter conflict state |

Fast-forwarding the common inbound-update case avoids unnecessary merge commits.

The working tree is Hatchdoor's durable local source of truth in bidirectional
mode. Before an operation that could replace files, dirty changes must be
committed or otherwise preserved. They must never be silently discarded.

### Concurrent Remote Writers and Push Races

Managed Git mode must not assume that Hatchdoor is the only entity pushing to
the configured branch. Each writeback therefore fetches and integrates the
remote immediately before pushing, even when periodic polling recently reported
that the checkout was current.

There is still an unavoidable race: another writer can push after Hatchdoor's
fetch but before Hatchdoor's push. Git will reject the push as a non-fast-forward
update. Hatchdoor should treat that rejection as a normal concurrency event and
run a bounded reconciliation loop:

```text
commit Hatchdoor changes locally
          |
          v
fetch -> compare -> fast-forward/merge -> push
                                      success | non-fast-forward
                                              v
                         fetch latest remote and repeat
```

For each attempt:

1. Fetch the latest configured branch without holding the vault lock.
2. Compare the latest local and remote heads.
3. Under the vault lock, fast-forward or merge and refresh the index if the
   working tree changes.
4. Release the lock and attempt the push.
5. If the push is rejected because the remote advanced, repeat from fetch using
   the new remote head.

The immediate loop should have a small fixed limit, for example three attempts,
so a very active remote cannot monopolize the repository task. If the limit is
exhausted, Hatchdoor keeps its local commits, reports the checkout as degraded
with unpushed work, and retries later using backoff. Network failures follow the
same durability rule. Authentication, permissions, and branch-protection errors
should remain visible until configuration or remote policy changes.

If reconciliation produces a content conflict, the conflict policy below takes
over; Hatchdoor must not force-push or choose one writer's content implicitly.
Multiple entities using separate clones are therefore supported. Multiple
Hatchdoor processes sharing the same physical checkout remain unsupported.

## Conflict Policy

Conflict handling requires special attention because a managed-checkout user may
not have shell access to the local repository.

On a merge conflict, Hatchdoor should:

1. Collect the conflicting paths.
2. Abort the conflicted merge and restore the last clean local commit.
3. Preserve the local head in a branch such as
   `hatchdoor-conflict/<instance>/<timestamp>`.
4. Attempt to push that preservation branch without changing the configured
   branch.
5. Mark the lifecycle state as `conflict`.
6. Disable further Hatchdoor mutations to avoid compounding the divergence.
7. Continue serving the local snapshot read-only.
8. Expose the conflict paths, local and remote commit IDs, and preservation
   branch through status.

The operator can merge the preservation branch into the configured branch using
their normal Git provider workflow. Once the remote contains the local history,
Hatchdoor can fetch and fast-forward automatically.

If publishing the preservation branch fails, the local commits remain protected
in persistent checkout storage and status must make clear that the recovery
history exists only locally.

## Starter Vault and Empty Repositories

Managed Git mode should not automatically seed every repository that contains no
Markdown. A nonempty repository may intentionally contain no Markdown, and
automatic seeding would unexpectedly mutate it.

Recommended policy:

- A nonempty repository with no Markdown produces an empty vault, or a clear
  validation error if a stricter policy is selected.
- A truly empty/unborn remote may be bootstrapped only when an explicit option
  such as `HATCHDOOR_BOOTSTRAP_EMPTY_REPOSITORY=true` is set.
- Bootstrap creates starter notes, an initial commit, and—when bidirectional
  mode is configured—pushes the initial branch.
- The `.git` directory and Hatchdoor management metadata must be excluded from
  vault indexing and empty-vault detection.

## Crash Recovery and Shutdown

Startup recovery must inspect both:

- A dirty working tree.
- Local commits ahead of the remote.

Checking only for ahead commits misses a crash after a file write but before the
debounced commit. In bidirectional mode, Hatchdoor should commit recovered dirty
vault changes and resume synchronization without waiting for another user write.

On graceful shutdown:

1. Stop accepting new mutations.
2. Drain queued write records.
3. Commit dirty working-tree changes locally.
4. Attempt fetch/integration/push within a bounded timeout.
5. Exit even if the remote is unavailable, relying on persistent storage and
   startup recovery for the next attempt.

Local durability is mandatory; remote availability during shutdown is not.

## Status, Health, and Operability

Repository status should cover lifecycle state rather than only writeback. It
should include:

- Source and synchronization mode.
- `initializing`, `ready`, `syncing`, `degraded`, or `conflict` phase.
- A sanitized repository identity and the resolved branch.
- Local and remote commit IDs.
- Ahead and behind counts.
- Dirty working-tree state.
- Pending Hatchdoor write count.
- Last successful clone, fetch, integration, and push timestamps.
- Last error kind and redacted message.
- Conflict paths and preservation branch, when applicable.

The existing MCP Git status tool can be extended, and an authenticated web API
can expose the same snapshot. A small frontend indicator can follow separately.

`/health` should remain a liveness/cache check. A separate `/ready` endpoint
should report whether the configured vault source has been prepared and indexed.
For an existing checkout running in degraded network state, readiness may remain
successful while repository status reports staleness. A first startup with no
usable checkout is not ready.

Logs must never include credentials or a URL containing user information. Clone
progress, retry state, branch, sanitized remote identity, and commit IDs are safe
and useful operational fields.

## Security Considerations

- Accept HTTPS only in the first version.
- Reject embedded URL credentials and avoid storing tokens in `.git/config`.
- Keep tokens out of logs, status payloads, errors, and commit metadata.
- Support public repositories without requiring a token.
- Recommend least-privilege, repository-scoped credentials for bidirectional
  mode.
- Canonicalize the configured vault subdirectory and ensure it remains inside
  the managed checkout.
- Do not automatically delete, replace, or hard-reset an unrecognized checkout.
- Do not follow repository symlinks outside the vault for indexing or assets.
- Disable all mutation surfaces in pull-only mode.
- Continue requiring web and MCP authentication independently of Git
  credentials.
- Document that the configured remote is trusted input from the deployer; vault
  Markdown remains untrusted content.

## Compatibility and Migration

The first release should be additive:

- With `HATCHDOOR_VAULT_SOURCE=local` or when the variable is unset, current
  `VAULT_PATH`, Docker bind mount, seeding, watcher, and optional Git sync
  behaviour remain unchanged.
- Existing users with a host-managed Git checkout can keep that model.
- New managed-source users mount repository storage instead of a host vault.
- Automatic adoption of an existing checkout is not required. If supported, it
  should require an explicit flag and exact origin/branch validation.
- The legacy Git sync variables can be deprecated only after managed mode has
  equivalent operational coverage.

## Implementation Plan

### Phase 1: Repository Acquisition

- Add the vault source configuration and validation model.
- Implement clone/open/validate/resolve using the existing `git2` dependency.
- Add atomic first clone and safe reuse after restart.
- Support public and token-authenticated HTTPS repositories.
- Support optional branch selection and vault subdirectory.
- Add persistent repository storage to Compose examples.
- Preserve local-path behaviour.

### Phase 2: Inbound Lifecycle

- Fetch and integrate before the initial index build.
- Add periodic pull polling.
- Implement equal, fast-forward, ahead, and diverged graph paths.
- Refresh the index explicitly after working-tree changes.
- Coordinate watcher refreshes with repository and note mutations.
- Recover dirty files and unpushed commits on startup.

### Phase 3: Operational Safety

- Expand repository lifecycle status.
- Add readiness and manual sync endpoints.
- Enforce pull-only write capabilities.
- Add graceful shutdown flushing.
- Add conflict preservation branches and conflict-state write blocking.
- Allow pull-only managed sources in demo mode while continuing to reject
  bidirectional sync there.

### Phase 4: Documentation and UX

- Document public pull-only and private bidirectional deployments.
- Document persistent volumes, credential scopes, and outage behaviour.
- Add migration guidance for host-managed checkouts.
- Add a minimal repository status indicator to the web application if desired.

## Verification Plan

### Unit and Repository Tests

- Configuration rejects ambiguous local/Git sources and redacts secrets.
- Fresh public clone selects the requested or default branch.
- Private clone receives credentials through callbacks without persisting them.
- An interrupted clone never occupies the final path.
- Restart reuses a matching checkout.
- Wrong origin, branch, subdirectory, and unknown destination fail safely.
- Equal repositories are no-ops.
- Behind-only repositories fast-forward without a merge commit.
- Ahead-only repositories push only in bidirectional mode.
- Clean divergence merges and pushes.
- A remote update pushed after Hatchdoor fetches but before it pushes triggers a
  refetch/integrate/push retry and succeeds without losing either change.
- Repeated non-fast-forward races stop after the bounded attempt limit, preserve
  the unpushed local commits, and transition to retry backoff.
- Conflicting divergence preserves the local head and reports conflict state.
- Dirty working-tree changes survive restart and are committed.
- Fetch and push do not hold the vault mutation lock.
- Checkout and reindex cannot race vault writes.
- A changed checkout refreshes the cache once and broadcasts a revision.
- Pull-only mode rejects browser and MCP writes.

Tests should primarily use temporary local bare repositories so the lifecycle
matrix does not depend on an external Git provider.

### End-to-End Scenarios

1. Start with an empty repository volume and only a public Git URL; remote notes
   become available through the API.
2. Restart or recreate the container; the checkout is reused instead of cloned
   again.
3. Advance the remote from another clone; Hatchdoor pulls, reindexes, and emits
   a vault revision within the polling interval.
4. Create, update, move, and delete notes in bidirectional mode; commits reach
   the configured remote.
5. Kill Hatchdoor during the debounce window; restart detects and commits the
   dirty change.
6. Start with the network unavailable and a valid checkout; Hatchdoor serves the
   last snapshot as degraded and later recovers.
7. Start with invalid credentials and no checkout; readiness fails clearly
   without leaking credentials.
8. Change the configured URL over an existing checkout; Hatchdoor refuses to
   overwrite it.
9. Create a clean divergence; Hatchdoor merges and pushes.
10. Advance the remote after Hatchdoor fetches but before it pushes; Hatchdoor
    refetches, reconciles, and completes the push.
11. Keep advancing the remote during every immediate attempt; Hatchdoor stops at
    the retry limit and preserves its unpushed local commit for later backoff.
12. Create a conflicting divergence; local work is preserved and status provides
    a recovery path.
13. Run pull-only mode; all mutation endpoints and tools are unavailable.
14. Use a vault subdirectory; Markdown outside it and `.git` metadata are not
    indexed.

### Container Verification

- Rootless/distroless execution can create and reuse the checkout volume.
- Container recreation retains dirty files, commits, and conflict state.
- No host vault bind mount is required in managed mode.
- Health and readiness behave correctly during first clone, degraded startup,
  and normal operation.
- Credentials do not appear in logs or repository configuration.

## Alternatives Considered

### Clone in an Entrypoint Script

This would require adding a shell and Git executable to the distroless runtime or
changing the base image. It would handle first clone but not runtime polling,
locking, cache refresh, status, shutdown, or conflict recovery. It also splits
repository ownership from the application.

### Clone in an Init Container

An init container is viable for environments that already use orchestration,
but it still does not manage ongoing pull/writeback lifecycle. It could remain
an optional deployment pattern for local-source mode, not the primary managed
source design.

### Bare Repository Plus Separate Worktree

This can separate objects from the checked-out vault but adds recovery and
worktree-management complexity without a clear benefit for a single-vault
process. A normal non-bare checkout is simpler and matches the existing Git
implementation.

## Open Questions

- Should a nonempty repository with no Markdown be a valid empty vault or a
  configuration error?
- Should remote polling default to 60 seconds, another interval, or be disabled
  until explicitly configured?
- Should the immediate non-fast-forward reconciliation limit be fixed or
  configurable?
- Is pushing a generic conflict-preservation branch acceptable as the default,
  or should the first version only preserve conflicts locally?
- Should bidirectional managed mode require an explicit global write-enable flag
  in addition to web/MCP authentication settings?
- Should readiness stay successful when a persisted checkout is usable but the
  remote is temporarily unavailable?
- Is vault-subdirectory support required in the first release or appropriate for
  a follow-up?

## Decision Requested

Approve the direction of an application-managed, persistent Git checkout with:

1. Backward-compatible local and managed Git source modes.
2. Startup clone/reuse before indexing.
3. Independent inbound polling.
4. Optional bidirectional writeback.
5. Shared working-tree/index locking.
6. Durable crash and conflict recovery.
7. Explicit lifecycle status and readiness.

Detailed configuration names and UI presentation can be finalized during
implementation after these lifecycle semantics are agreed.
