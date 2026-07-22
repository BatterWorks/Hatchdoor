# Managed Git Vault Foundation — Agent Handoff

Use this document to start a fresh implementation chat. It provides the entry
point, required context, and expected debrief before code changes begin.

## Objective

Implement the managed Git vault foundation described in this repository, one
reviewable pull-request slice at a time.

The intended end result is one environment-configured local or managed Git vault
whose acquisition, synchronization, indexing, capabilities, and failures are
observable while the Hatchdoor application remains available.

Existing local `VAULT_PATH` deployments must remain backward compatible.

## Required Reading

Read these documents in order:

1. [`docs/architecture/managed-git-vault-foundation.md`](../architecture/managed-git-vault-foundation.md)
2. [`docs/plans/managed-git-vault-foundation.md`](managed-git-vault-foundation.md)
3. [`docs/roadmap/vault-lifecycle.md`](../roadmap/vault-lifecycle.md)
4. [`docs/adr/README.md`](../adr/README.md), especially ADR-01, ADR-03, ADR-07,
   ADR-10, ADR-12, and ADR-13
5. [Managed Git Vault Lifecycle, PR #18](https://github.com/BattermanZ/Hatchdoor/pull/18),
   including its review discussion

Treat the architecture and plan as draft working documents. Do not silently
resolve an open product decision or change an accepted ADR.

## Current-Code Orientation

Before proposing implementation, inspect at least:

- `src/server.rs` — composition, startup, health, readiness, and task startup;
- `src/app_state.rs` — vault path, cache publication, locks, and revisions;
- `src/startup.rs` — current startup state model;
- `src/config.rs` — application environment configuration;
- `src/git/config.rs` — legacy Git-sync configuration;
- `src/git/sync.rs` — commit, fetch, integrate, push, and recovery behavior;
- `src/git/task.rs` — debouncing, retry, and lock boundaries;
- `src/git/status.rs` — current Git status model;
- `src/vault_watcher.rs` — filesystem refresh behavior;
- `src/handlers/write_api.rs` — browser capability and mutation guards;
- `src/mcp/config.rs` and `src/mcp/tools/` — MCP capability and mutation guards;
  and
- `src/vault/seed.rs` and cache construction — current starter-vault policy.

Also inspect the current branch, working-tree status, recent commits, and any
repository instructions before editing. Preserve unrelated user changes.

## Required Debrief

After reading and inspection, provide a concise debrief before writing code. It
must include:

1. The current behavior and the specific gap being addressed.
2. The first proposed pull-request slice and why it is independently useful.
3. The files and runtime seams likely to change.
4. The compatibility and data-loss risks.
5. The characterization and new tests needed.
6. Any open product decision that blocks or materially changes the slice.
7. What is explicitly deferred.

Wait for confirmation after this debrief if the user has asked to discuss or
approve scope before implementation. Otherwise, proceed only when the requested
slice is unambiguous and no open product decision changes its behavior.

## Recommended First Slice

Begin with the runtime foundation, not cloning:

- explicit local versus managed source configuration types;
- a lifecycle snapshot that can represent no ready vault;
- shared runtime capabilities for web and MCP;
- application startup that does not require a ready vault path or index; and
- characterization tests proving local mode remains unchanged.

This slice should establish the seam needed by later acquisition and lifecycle
work without implementing Git clone, polling, conflict branches, or UI-managed
configuration yet.

If this is too large for one review, split it into:

1. source/lifecycle/capability types with local-mode adapters; then
2. always-available startup and structured unavailable/initializing responses.

## Open Decisions

Do not assume answers to these without user agreement:

1. Whether a nonempty managed repository without Markdown is a valid empty
   vault or an unavailable source.
2. The default polling interval and whether polling can be disabled.
3. The immediate retry limit for non-fast-forward push races.
4. Whether conflict-preservation branches are pushed automatically.
5. Readiness semantics for degraded and conflict states.
6. The sanitized repository identity exposed through status.

Most of these do not block the recommended first slice. If a chosen slice
depends on one, surface it in the debrief rather than selecting a default.

## Non-Negotiable Constraints

- Markdown remains the source of truth; SQLite remains disposable.
- Local mode is the default and preserves current behavior.
- Git network operations must not hold the vault mutation lock.
- No dirty work or local commit may be silently reset, discarded, or
  force-pushed.
- Web and MCP must enforce the same lifecycle-derived mutation capability.
- Pull-only and conflict states disable every mutation surface.
- Repository and vault roots remain distinct in subdirectory mode.
- Vault containment must be revalidated after every working-tree transition.
- Subdirectory mode must never stage unrelated repository files.
- Credentials must not appear in URLs, repository configuration, logs, status,
  errors, or commits.
- The runtime remains rootless, distroless, and independent of a Git executable.
- Do not introduce speculative multi-vault abstractions into this increment.

## Verification Expectations

For every slice:

- add characterization tests before changing an existing behavior;
- add regression tests for concurrency, containment, or recovery invariants
  touched by the slice;
- use temporary local repositories for Git behavior where possible;
- run formatting, linting, backend tests, and relevant frontend checks;
- verify `git diff --check`; and
- clearly separate local verification from container or end-to-end verification.

Do not mark a plan item complete merely because its types or interfaces exist;
its documented behavior and verification must be present.

## Copyable Fresh-Chat Prompt

```text
Work on the Hatchdoor managed Git vault foundation.

Start by reading:
- docs/plans/managed-git-vault-agent-handoff.md
- docs/architecture/managed-git-vault-foundation.md
- docs/plans/managed-git-vault-foundation.md

Then inspect the linked roadmap, ADRs, PR #18, and the current implementation
seams named in the handoff. Do not edit code yet.

Give me the required concise debrief for the recommended first slice: current
behavior, proposed boundary, affected seams, risks, tests, blockers, and deferred
work. After we agree on that debrief, implement only the approved slice and
verify backward compatibility with local mode.
```
