# PR #31 Review — Managed Git Vault Lifecycle Foundation

> Historical review record.

- PR: [#31 — Establish managed Git vault lifecycle foundation](https://github.com/BattermanZ/Hatchdoor/pull/31)
- Reviewed head: `9fe7f77988bbd5243e0e6920b5b3fb445f107906`
- Target branch: `development`
- Review date: 2026-07-26
- Review status: Changes recommended before merge

## Executive Summary

PR #31 introduces a useful lifecycle model for configured local and managed Git
vault sources. Its most important new capability is representing an application
that is healthy even though no validated vault path or published index exists.
It also adds source-aware capabilities and a structured `/api/vault-status`
response.

The PR does not implement managed Git acquisition or synchronization. A managed
Git configuration intentionally enters `unavailable` with
`managed_vault_not_acquired`.

Much of the always-available startup behavior predates this PR. `/health`,
`/ready`, `/api/startup-status`, the SPA outside readiness middleware, and
background model loading/indexing already exist on `development`. The PR
generalizes that implementation from “configured local vault still indexing” to
“the vault itself may not exist or be usable.”

The architecture is directionally sound, but the migration is incomplete:

1. Web and MCP treat every non-ready state as a search-model setup problem,
   including managed-vault failures.
2. The old startup representation remains as a forwarding compatibility wrapper
   around the new runtime.
3. Readiness is represented independently by both the lifecycle phase and the
   optional published vault, allowing contradictory internal states.

These issues should be addressed before merge or explicitly bounded by a
documented follow-up with regression tests.

## What `/api/vault-status` Does

`GET /api/vault-status` returns the current `VaultRuntimeSnapshot`. It is an
unauthenticated, non-cacheable operational endpoint that reports:

- lifecycle `phase`;
- configured source kind (`local` or `managed-git`);
- source mode (`local`, `pull-only`, or `bidirectional`);
- derived browse, search, mutation, pull, push, and retry capabilities;
- optional model-download progress;
- optional indexing progress; and
- an optional sanitized structured error.

Example ready local response:

```json
{
  "phase": "ready",
  "source": "local",
  "mode": "local",
  "capabilities": {
    "browse": true,
    "search": true,
    "mutate": true,
    "pull": false,
    "push": false,
    "retry": false
  }
}
```

Example managed source response in this PR:

```json
{
  "phase": "unavailable",
  "source": "managed-git",
  "mode": "pull-only",
  "capabilities": {
    "browse": false,
    "search": false,
    "mutate": false,
    "pull": false,
    "push": false,
    "retry": false
  },
  "error": {
    "code": "managed_vault_not_acquired",
    "message": "Managed Git vault acquisition is not implemented in this foundation slice.",
    "retryable": false
  }
}
```

This is more expressive than `/api/startup-status`, but both endpoints currently
project the same underlying runtime state.

## What Was Already Implemented

Before PR #31, `development` already provided:

| Behavior | Before PR #31 | PR #31 |
| --- | --- | --- |
| `/health` independent of index readiness | Yes | Preserved |
| `/ready` based on startup readiness | Yes | Preserved |
| `/api/startup-status` | Yes | Preserved as a compatibility view |
| SPA reachable while startup is incomplete | Yes | Preserved |
| Model loading and indexing in a background task | Yes | Preserved |
| Listener bound before background model/index work | Yes | Preserved |
| App state without a completed index | Partially | Formalized |
| App state without any validated vault path | No | Added |
| Source/mode-aware capabilities | No | Added |
| Structured managed-vault lifecycle status | No | Added |
| `/api/vault-status` | No | Added |

The accurate description is therefore:

> PR #31 refactors and extends the existing always-available startup system so
> it can represent an absent or failed managed vault, and exposes that richer
> state through `/api/vault-status`.

## Findings

### F1 — Non-ready MCP states are incorrectly treated as model setup

**Severity:** P2  
**Status:** Confirmed; existing automated review thread is unresolved.

`initialize` and `tools/list` use only `startup.is_ready()` to choose between the
normal MCP surface and the three Gemma/Nomic setup tools. `tools/call` similarly
allows the model setup tools for every non-ready phase.

When the phase is `Unavailable` because the managed vault was not acquired, an
MCP client is told to configure the embedding model. Accepting or declining the
model terms cannot acquire the vault and may trigger an unnecessary download.

**Recommendation**

- Branch MCP setup behavior on the specific lifecycle phase/error category.
- Expose a read-only lifecycle-status tool for vault failures.
- Offer Gemma accept/decline tools only for `TermsRequired`.
- Offer model retry only for a model setup/download failure.
- Add tests for MCP `initialize`, `tools/list`, and `tools/call` while the phase
  is `Unavailable` with `managed_vault_not_acquired`.

### F2 — The browser presents the wrong recovery action for vault failures

**Severity:** P2  
**Status:** Confirmed; not covered by the PR tests.

The unchanged frontend maps every legacy `failed` startup response to the same
screen and sends the “Retry setup” button to `/api/model/retry`.

For `managed_vault_not_acquired`, this either:

- returns a conflict if model terms are undecided; or
- loads/downloads the selected model and returns to the same unavailable vault
  state.

The button cannot repair the reported failure.

**Recommendation**

- Make the frontend consume the structured failure category, preferably from
  `/api/vault-status`.
- Show model controls only for model setup states.
- Show a vault-specific explanation for acquisition/configuration failures.
- Do not show a retry action until a real vault retry command exists.
- Add a frontend test for a managed-vault unavailable fixture.

### F3 — `StartupTracker` is now mostly a forwarding wrapper

**Severity:** Design debt  
**Status:** Confirmed.

`StartupTracker` contains a `VaultRuntime` and forwards setters and queries such
as `set_ready`, `set_indexing`, `set_downloading`, and `is_ready`. Its remaining
substantive responsibility is translating `VaultRuntimeSnapshot` into the legacy
`StartupStatusResponse`.

This is understandable as a compatibility bridge, but the PR does not identify
it as temporary or provide a consolidation path.

**Recommendation**

- Keep `/api/startup-status` as an explicitly documented compatibility
  projection if backward compatibility requires it.
- Move that projection to a standalone conversion/handler.
- Let application code depend directly on the authoritative runtime instead of
  retaining a second tracker abstraction.
- Add a removal/deprecation milestone for the compatibility response.

### F4 — Two independent readiness representations can disagree

**Severity:** P2 design risk  
**Status:** Confirmed structurally; no observed production failure in this slice.

Readiness is represented by:

1. `VaultRuntime.phase == Ready`; and
2. `AppState.ready_vault == Some(ReadyVault)`.

Middleware checks the lifecycle phase. Handlers then read `ready_vault`. Nothing
in the types prevents:

```text
phase = Ready
ready_vault = None
```

or:

```text
phase = Unavailable
ready_vault = Some(previous snapshot)
```

The latter may eventually represent degraded read-only operation, but current
capability derivation disables browsing for every phase except `Ready`. The
relationship is therefore not yet defined as an intentional state machine.

**Recommendation**

- Establish one authoritative publication transition that updates the usable
  vault and lifecycle atomically from the perspective of readers.
- Define whether a published prior snapshot remains browseable in degraded and
  conflict states.
- Derive readiness and browse/search capabilities from that authoritative
  state.
- Add invariant tests covering publication, initial failure, failed reindex,
  degraded operation, and recovery.

### F5 — Model setup phases are mixed into `VaultPhase`

**Severity:** Design debt  
**Status:** Confirmed.

`VaultPhase` includes `TermsRequired` and `Downloading`, which are embedding
model states rather than vault-source states. This coupling is the underlying
reason consumers cannot reliably distinguish model failures from vault failures.

**Recommendation**

Either:

- model vault and embedding lifecycle as separate status components; or
- retain one top-level lifecycle but use typed phase categories that force
  exhaustive, category-specific handling.

Avoid selecting user actions solely from a generic ready/non-ready boolean.

### F6 — Managed Git configuration is accepted but deliberately unusable

**Severity:** Scope/communication risk  
**Status:** Intentional in this PR.

The PR accepts managed Git source configuration but does not clone, open, index,
pull, or push the configured repository. It immediately reports
`managed_vault_not_acquired`.

This matches the PR body, but the title can be read as delivering a functioning
managed Git vault.

**Recommendation**

- Make the incomplete behavior prominent in release notes and configuration
  documentation.
- Do not present the environment variables as a usable deployment option until
  acquisition exists.
- Consider keeping the configuration behind an experimental/internal flag until
  the next slice can make it functional.

## Duplication Assessment

Not all new code is redundant. The source model, optional validated vault,
capability derivation, structured errors, and operational snapshot are useful
foundation pieces.

The current implementation does contain transitional or risky duplication:

| Area | Assessment |
| --- | --- |
| `/api/startup-status` and `/api/vault-status` | Two views of the same state; acceptable temporarily |
| `StartupStatusResponse` progress fields and `VaultRuntimeSnapshot` progress | Duplicate response representation |
| `StartupTracker` and `VaultRuntime` setters/readiness methods | Mostly forwarding duplication |
| Runtime `Ready` phase and `ready_vault: Option<_>` | Duplicate source of truth; correctness risk |
| Configured source path and validated ready-vault path | Necessary distinction, not accidental duplication |
| Lifecycle-derived web/MCP mutation capability | Useful centralization |

The PR should be treated as an incomplete migration rather than a finished
consolidation.

## Validation Performed

At reviewed head `9fe7f77988bbd5243e0e6920b5b3fb445f107906`:

- PR is open, non-draft, mergeable, and targets `development`.
- `git diff --check` passed.
- All locally executed Rust test targets passed:
  - 440 library tests;
  - 7 evaluation CLI tests; and
  - 3 main CLI tests.
- Standard Clippy completed with three warnings in untouched
  `src/model_setup.rs`.
- GitHub reported only the GitGuardian security check for the PR; the broader
  backend/frontend validation claimed in the PR description was not represented
  as enforced GitHub checks at review time.

Passing tests do not cover F1 or F2 because existing tests verify that the
managed vault becomes unavailable, but not whether MCP and the frontend offer
the correct explanation and recovery action.

## Recommended Merge Checklist

- [ ] Fix MCP phase-specific setup/status behavior (F1).
- [ ] Fix browser recovery behavior for non-model failures (F2).
- [ ] Add managed-unavailable MCP regression tests.
- [ ] Add a managed-unavailable frontend regression test.
- [ ] Define and test the invariant between lifecycle readiness and
      `ready_vault` publication (F4).
- [ ] Document whether `StartupTracker` and `/api/startup-status` are temporary
      compatibility layers (F3).
- [ ] Clarify that managed Git acquisition and synchronization are not included
      in this PR (F6).
- [ ] Decide whether the model/vault phase coupling is accepted debt or should be
      corrected in this slice (F5).

## Suggested Decision

Request changes before merge for F1 and F2. Require either a fix for F4 or a
clear invariant design plus tests demonstrating that contradictory states cannot
escape to clients. Track F3 and F5 as explicit migration work if they are not
resolved in this PR.
