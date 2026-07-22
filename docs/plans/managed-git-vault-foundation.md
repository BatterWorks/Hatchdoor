# Managed Git Vault Foundation — Implementation Plan

- Status: Draft for discussion
- Architecture: [`docs/architecture/managed-git-vault-foundation.md`](../architecture/managed-git-vault-foundation.md)
- Product roadmap: [`docs/roadmap/vault-lifecycle.md`](../roadmap/vault-lifecycle.md)
- Design proposal: [Managed Git Vault Lifecycle, PR #18](https://github.com/BattermanZ/Hatchdoor/pull/18)
- Fresh-chat entry point: [`docs/plans/managed-git-vault-agent-handoff.md`](managed-git-vault-agent-handoff.md)
- Target: One environment-configured local or managed Git vault with an
  observable background lifecycle

## Completion Definition

Existing local deployments behave unchanged. A managed Git deployment can
acquire, reuse, synchronize, index, expose, and safely recover one persistent
vault while the application remains reachable during vault failures.

The items are ordered implementation milestones, not completed work.

## 0. Resolve the Open Product Decisions

- [ ] Decide how to treat a nonempty managed repository without Markdown.
- [ ] Select the polling default and whether polling can be disabled.
- [ ] Select the immediate retry limit for non-fast-forward push races.
- [ ] Decide whether conflict-preservation branches are pushed automatically.
- [ ] Define readiness behavior for degraded and conflict states.
- [ ] Define the sanitized repository identity exposed to clients.
- [ ] Propose an ADR amendment if the accepted direction changes ADR-10.

Exit: Observable behavior is decided and any required ADR proposal is ready.

## 1. Establish the Runtime Foundation

- [ ] Add explicit local and managed Git source configuration while keeping local
  as the default.
- [ ] Represent pull-only and bidirectional managed modes.
- [ ] Separate configured source, repository root, and validated vault root.
- [ ] Introduce one lifecycle snapshot for phase, progress, capabilities,
  repository/index status, and redacted errors.
- [ ] Allow `AppState` to exist before a vault path or index is ready.
- [ ] Derive and enforce one mutation capability across browser and MCP.
- [ ] Keep existing demo, web-auth, and MCP-auth posture checks intact.

Verification:

- [ ] Local mode resolves and behaves as before.
- [ ] Pull-only rejects every browser and MCP mutation.
- [ ] Invalid combinations fail clearly without leaking secrets.

Exit: The HTTP application can run without a ready vault, and all surfaces share
one capability decision.

## 2. Make Vault Startup a Background Lifecycle

- [ ] Start the HTTP listener before acquisition and indexing.
- [ ] Keep the SPA, `/health`, `/ready`, and lifecycle status reachable throughout
  initialization and failure.
- [ ] Return stable structured errors from vault-dependent surfaces until a
  snapshot is ready.
- [ ] Preserve an existing published SQLite snapshot during later sync/reindex.
- [ ] Add an idempotent manual retry command.
- [ ] Continue to fail fast for unsafe application security configuration.

Verification:

- [ ] The UI remains reachable during a blocked or failed first acquisition.
- [ ] Readiness changes only after the first index publication.
- [ ] A transient failure or changed token file can be retried without restart;
  changed environment settings take effect after restart.

Exit: Operational vault failure is runtime state, not process failure.

## 3. Separate Source Preparation From Indexing

- [ ] Move starter-vault seeding out of generic cache construction.
- [ ] Preserve the current local empty-vault behavior.
- [ ] Apply the selected no-Markdown policy to managed repositories.
- [ ] Reject bootstrap in pull-only mode.
- [ ] Support explicit bidirectional bootstrap of an empty/unborn remote.

Verification:

- [ ] Local seeding tests remain unchanged.
- [ ] A managed repository is never seeded implicitly.
- [ ] Pull-only creates no bootstrap commit.
- [ ] Explicit bidirectional bootstrap creates and pushes starter notes once.

Exit: Each source prepares content according to its own mutation policy before
generic indexing begins.

## 4. Acquire and Reuse a Managed Checkout

- [ ] Validate HTTPS URLs and reject embedded credentials.
- [ ] Support public repositories without credentials and private repositories
  through `git2` callbacks using a token or token file.
- [ ] Acquire a process-lifetime checkout ownership lock.
- [ ] Clone into an owned temporary sibling, validate it, and atomically install
  it on persistent storage.
- [ ] Reopen and validate a matching checkout on restart.
- [ ] Refuse to overwrite or adopt unknown destinations.
- [ ] Validate branch, origin, repository shape, and vault boundary.

Verification:

- [ ] Fresh public and authenticated fixture clones succeed.
- [ ] Interrupted clone does not occupy the final destination.
- [ ] Restart reuses the checkout.
- [ ] Wrong origin/branch/subdirectory and concurrent ownership fail safely.
- [ ] Credentials do not appear in logs, status, errors, or `.git/config`.

Exit: Managed configuration reliably produces a validated persistent vault
without a Git executable.

## 5. Enforce the Vault Boundary

- [ ] Reject absolute, traversing, and escaping subdirectory configuration.
- [ ] Revalidate canonical containment after every checkout, merge, reset, and
  recovery operation.
- [ ] Stage only the validated vault subtree in subdirectory mode.
- [ ] Leave unrelated repository dirt untouched and report it if it blocks safe
  integration.
- [ ] Keep every note, asset, attachment, and download operation inside the
  validated vault root.

Verification:

- [ ] Startup and post-checkout symlink escapes are rejected.
- [ ] A Hatchdoor write never commits an outside-subtree file.
- [ ] Outside-subtree dirt is never reset or deleted implicitly.

Exit: Remote or local repository content cannot redirect Hatchdoor outside its
vault or make it publish unrelated files.

## 6. Build the Repository Lifecycle Actor

- [ ] Evolve the write-triggered task into one actor handling startup, polling,
  write batches, manual commands, retry timers, and shutdown.
- [ ] Fetch and push without the vault mutation lock.
- [ ] Under the lock, preserve dirty vault work and re-read the commit graph.
- [ ] Implement no-op, fast-forward, ahead, clean-divergence, and conflict paths.
- [ ] Revalidate the vault boundary after working-tree changes.
- [ ] Poll for inbound changes independently of Hatchdoor writes.
- [ ] Add bounded immediate reconciliation for non-fast-forward push races,
  followed by backoff with local commits retained.

Verification:

- [ ] Network phases never hold the vault lock.
- [ ] A write completed during fetch is preserved and included in the post-lock
  graph decision.
- [ ] Remote-ahead uses fast-forward without a merge commit.
- [ ] Clean divergence preserves both histories.
- [ ] Push races reconcile or stop at the bound without losing local commits.
- [ ] Polling ingests a change made from another clone.

Exit: One actor safely maintains a shared-branch checkout with independent
remote writers.

## 7. Publish a Consistent Index

- [ ] Define one lock order for working-tree mutation and index refresh.
- [ ] Make watcher refreshes coordinate through the vault mutation lock.
- [ ] Explicitly rebuild after manager-originated working-tree changes.
- [ ] Coalesce manager-generated watcher events.
- [ ] Publish SQLite changes transactionally and broadcast one revision.
- [ ] Retain the prior snapshot when a later rebuild fails.

Verification:

- [ ] Checkout/indexing cannot race browser, MCP, or watcher work.
- [ ] Readers see the previous or replacement snapshot, never a mixed tree.
- [ ] One remote transition produces one publication and revision event.
- [ ] A failed rebuild leaves the prior snapshot usable in degraded state.

Exit: Every published SQLite snapshot corresponds to one validated working-tree
state.

## 8. Add Recovery and Conflict Safety

- [ ] Classify repository operation state, dirty vault files, outside-vault dirt,
  and ahead commits separately at startup.
- [ ] Preserve dirty vault content before any hard reset or cleanup.
- [ ] Commit and retry recovered dirt in bidirectional mode.
- [ ] Preserve and report unexpected dirt in pull-only mode.
- [ ] On conflict, retain the local head on a unique preservation branch.
- [ ] Publish the branch according to the selected policy and report whether it
  exists remotely or only locally.
- [ ] Disable browser/MCP mutation in conflict state while continuing read-only
  browsing and search.
- [ ] Recover automatically after the configured remote branch contains the
  preserved work.

Verification:

- [ ] A process killed during debounce recovers and pushes the dirty edit.
- [ ] Interrupted Git operation recovery does not discard unrelated vault dirt.
- [ ] Remote outage with an existing checkout starts degraded and later heals.
- [ ] A conflict never force-pushes and blocks all mutation surfaces.
- [ ] Failed preservation-branch publication remains durable locally.

Exit: Crashes, outages, and conflicts cannot silently lose vault work.

## 9. Add the Operational Surface

- [ ] Expose lifecycle phase, progress, mode, branch, ahead/behind, dirty state,
  operation timestamps, conflicts, and redacted errors.
- [ ] Add authenticated retry/sync behavior.
- [ ] Extend web capabilities and MCP Git status from the same snapshot.
- [ ] Show first acquisition/indexing progress in the existing startup surface.
- [ ] Add a compact ready/syncing/degraded/conflict indicator and details view.
- [ ] Clearly explain pull-only and conflict read-only states.
- [ ] Do not add configuration forms or secret entry.

Verification:

- [ ] UI states map mechanically to lifecycle fixtures.
- [ ] Web and MCP report consistent mode, readiness, and capabilities.
- [ ] Status and errors contain no secrets or internal filesystem paths.

Exit: An operator can understand and retry lifecycle work without container
access or log inspection.

## 10. Finish Shutdown, Deployment, and Compatibility

- [ ] On graceful shutdown, stop mutations, drain writes, commit locally, and
  attempt final remote reconciliation within a bounded timeout.
- [ ] Document separate persistent repository and cache storage.
- [ ] Verify rootless/distroless clone and reuse permissions.
- [ ] Keep local mode and legacy `HATCHDOOR_GIT_SYNC_ENABLED` compatible.
- [ ] Document public pull-only and private bidirectional deployments, outages,
  conflicts, and host-managed checkout migration.
- [ ] Add container-level end-to-end scenarios for the completion definition.

Release scenarios:

- [ ] Existing local deployment upgrades without changes.
- [ ] Fresh public pull-only and private bidirectional sources become ready.
- [ ] Container recreation reuses the checkout and retained local state.
- [ ] First-clone failure leaves the application reachable and actionable.
- [ ] Remote changes, network outage/recovery, push races, conflicts, dirty crash
  recovery, and subdirectory escapes behave as documented.

Exit: The complete behavior is covered by automated tests or documented
container verification and is ready for release review.

## Suggested Pull Request Slices

1. Runtime source, lifecycle, and capability model.
2. Always-available startup and source-specific seeding.
3. Managed checkout acquisition, reuse, and vault boundary.
4. Polling, reconciliation, and coordinated index publication.
5. Crash/conflict recovery and graceful shutdown.
6. Operational API, minimal UI, deployment docs, and end-to-end tests.

Each slice should preserve local behavior and add characterization tests before
changing an existing seam.

## Deferred Work

- UI-managed vault configuration and Git secrets.
- Configuration precedence between environment and product state.
- Runtime add/edit/remove/switch behavior.
- Multi-vault identity, navigation, search, indexes, and MCP scoping.
- Provider-specific conflict workflows and additional Git transports.
