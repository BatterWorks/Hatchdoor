# Domain Collaboration Boundaries

## Status

Implemented and validated. The durable results and remaining evidence gaps are
recorded in
[`docs/architecture/collaboration-pilot-assessment.md`](architecture/collaboration-pilot-assessment.md).

## Goal

Make Hatchdoor easier for a human contributor or coding agent to change
surgically: one owner, one explicit contract, and a predictable set of
integration points.

The goal is not to force every feature into one directory. Hatchdoor contains
different kinds of boundaries, and the collaboration model must represent them
honestly:

- **Product capabilities** such as search, vault mutation, and note reading.
- **Infrastructure** such as the cache, embeddings, and Git synchronization.
- **Adapters** such as HTTP handlers, MCP tools, and React UI.
- **Composition and shared context** such as `server.rs`, `AppState`, and
  `App.tsx`.

This work must preserve Hatchdoor as a lean modular monolith. It must not
introduce services, a Cargo workspace, speculative traits, a frontend state
library, or a schema framework merely to make the architecture look modular.

## Definition of a surgical work packet

Every claimable piece of work must state:

- **Owned paths:** implementation files the contributor may change freely.
- **Public contract:** interfaces consumers rely on and that remain stable
  unless the packet explicitly declares a change.
- **Coordination paths:** shared or composition files that may be changed only
  when listed in the packet.
- **Consumed dependencies:** modules the work may call but does not own.
- **Forbidden paths and invariants:** areas and behavior that must not change.
- **Validation:** focused tests and full checks required before completion.

A domain boundary is successful when internal work stays within its owned paths.
A vertical feature may legitimately touch declared coordination files; this is
not considered a boundary failure.

## Architectural constraints

The module catalog and every applicable work packet must translate the existing
ADRs into concrete invariants. In particular:

- **ADR-01:** Markdown remains authoritative; SQLite remains disposable.
- **ADR-02:** Keep one binary and one shared domain core.
- **ADR-03:** HTTP and MCP mutations continue to use `vault/write/`.
- **ADR-05:** Runtime search remains pure semantic retrieval by default.
- **ADR-06:** Keep the embedded SQLite read model and its concurrency model.
- **ADR-13:** Add abstractions only when the collaboration pilot demonstrates
  that they pay for themselves.

## Plan

### Phase 1: Define and inventory

1. Adopt the work-packet vocabulary above as the collaboration contract.
2. Inventory every production path without moving code.
3. Classify each path as a product capability, infrastructure, adapter, or
   shared/composition code.
4. For each claimable module, record:
   - purpose and classification;
   - owned paths;
   - public interface;
   - allowed or consumed dependencies;
   - coordination paths;
   - forbidden dependencies;
   - ADR-backed invariants;
   - focused and full validation commands.
5. Give every production file one documented owner or an explicit
   shared/composition classification.

The main deliverable is `docs/architecture/module-map.md`.

### Phase 2: Add collaboration guidance

Add:

- a root `AGENTS.md` with agent scope and escalation rules;
- a collaboration section in `CONTRIBUTING.md`;
- a reusable work-packet template;
- an interface-change checklist for pull requests.

Do not add `CODEOWNERS` until real maintainers and ownership assignments are
known. Do not promise automated PR checks unless the corresponding GitHub
configuration is deliberately added.

### Phase 3: Dry-run a bounded module

Use the work-packet format for a small, already bounded, low-risk area such as
the Graph UI or diagnostics.

The dry run should answer:

- Could the contributor identify all writable files without broad repository
  exploration?
- Were the public contract and invariants sufficient?
- Were any undeclared coordination files required?
- Were the focused checks adequate?
- What context did the contributor still need from unrelated modules?

Update the catalog and template from this evidence before restructuring code.

### Phase 4: Pilot a meaningful feature boundary

Use frontend Search as the first structural stress-test, unless an upcoming real
collaborator task provides a better evidence-based candidate.

The intended Search pilot:

- co-locates feature-owned state, UI, client contract, tests, and styles;
- exposes one intentional public entry point;
- treats `App.tsx` as a declared composition file for navigation and keyboard
  shortcuts;
- leaves backend retrieval behavior unchanged;
- does not change cache schemas, ranking, reranking, hybrid search, or MCP
  behavior.

Backend Search already has useful façade functions and should not be rewritten
solely for symmetry with the frontend.

### Phase 5: Add lightweight enforcement

Only add enforcement justified by issues observed during the dry run and pilot:

- Rust module privacy and narrow re-exports;
- ESLint `no-restricted-imports` rules preventing imports into feature
  internals;
- focused tests for extracted feature behavior;
- a small shared JSON contract fixture if cross-language Search types are in
  scope;
- the existing Cargo and frontend quality gates.

Avoid new dependency-analysis packages or code-generation frameworks unless a
specific, demonstrated problem cannot be solved with existing tools.

### Phase 6: Validate and reassess

Run the full backend and frontend checks. Then repeat a scoped contributor task
using the completed packet and compare:

- context required;
- number of unrelated files inspected;
- changed paths;
- undeclared integration points;
- test scope;
- blast radius.

Extract another feature only if the pilot demonstrates a meaningful improvement.
Create a new ADR only if the result establishes a lasting architectural policy
that is not already covered by the existing records.

## Acceptance criteria

- Every production file has one documented owner or an explicit
  shared/composition classification.
- A work packet states writable paths, coordination paths, contracts,
  invariants, and exact checks.
- A contributor can change module internals without examining unrelated
  implementation code.
- Interface changes are declared and identify affected consumers.
- External frontend code cannot import feature internals.
- Rust exposes only intentional module façades where a boundary benefits from
  one.
- Pilot changes remain within owned paths and predeclared coordination files.
- Existing behavior remains unchanged and all relevant backend and frontend
  checks pass.
- ADR-01, ADR-02, ADR-03, ADR-05, ADR-06, and ADR-13 appear as concrete
  invariants wherever they apply.

## Out of scope

- Splitting Hatchdoor into services or separately deployed components.
- Creating a Cargo workspace solely to represent domains.
- Hiding `AppState` behind a set of speculative per-domain service traits.
- Introducing a frontend state-management framework for module boundaries.
- Reworking backend search behavior during the frontend Search pilot.
- Assigning maintainers or code owners without their agreement.
