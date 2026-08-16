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

## Verified Development VM Snapshot

The following was checked on 2026-07-22. Treat it as a host snapshot rather than
a permanent project guarantee, and recheck it if the environment changes.

Available tooling:

- Git 2.47.3;
- Rust 1.97.1 with Cargo, rustfmt, and Clippy;
- Node 24.16.0 and npm 12.0.1 under
  `/home/alemhnan/.nvm/versions/node/v24.16.0/bin`;
- GCC/G++, make, Perl, pkg-config, linker tools, SSH, curl, and Python;
- Docker 26.1.5 with Docker Compose 2.26.1 and a reachable daemon; and
- an existing Cargo registry cache.

Preparation still required before verification:

- The repository pins Rust 1.96.0, which is not currently installed. Install
  that exact toolchain or explicitly agree to verify with the installed 1.97.1;
  do not silently ignore the pin.
- Node and npm are not on the non-interactive shell `PATH`. Prefix commands with
  `/home/alemhnan/.nvm/versions/node/v24.16.0/bin` in `PATH` or load the matching
  NVM environment before running frontend checks.
- `frontend/node_modules` is absent, so `npm ci` is required.
- GitHub CLI (`gh`) is absent. This does not block local work or ordinary Git
  push, but GitHub Actions log and review-thread workflows need `gh` or the
  available GitHub connector.
- The current `origin` points to `BattermanZ/Hatchdoor`; the contributor fork
  remote and push authentication must be configured and verified separately
  before publishing.

Host capacity at the same check:

- 1.5 GiB physical RAM, with about 988 MiB available at measurement time;
- 2.0 GiB swap, almost entirely free;
- 2 logical CPUs; and
- about 28 GiB free on the workspace filesystem.

This is sufficient for development and verification, but memory is tight for
parallel Rust, frontend, and container builds. Run the toolchains sequentially,
use `CARGO_BUILD_JOBS=1` for local Cargo compilation, and avoid running a full
container build alongside other compilation. Expect a release/container build
to use swap and complete slowly. If it is killed for memory despite constrained
parallelism, perform final container verification on a larger runner rather than
weakening the checks.

The production Hatchdoor instance reported to use about 1 GiB RAM is managed by
Komodo on another host, not this development VM. At the 2026-07-22 check, the
local Docker context had no running containers, no Hatchdoor or Komodo process
was present, and total host memory use was about 557 MiB. The live deployment
therefore does not consume this VM's build headroom.

If Hatchdoor is later deployed on this same 1.5 GiB VM at roughly 1 GiB resident
memory, do not compile alongside it: Rust or container compilation could force
heavy swapping, disrupt the service, or trigger an out-of-memory kill. Stop or
move the build, or use a larger runner in that situation.

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
