# Vault Lifecycle & Multi-Vault Roadmap

- Status: Draft for discussion. Phase 1 (guided setup, managed Git vaults,
  product-managed vault collection) and the core of Phase 2 (multi-vault
  configuration, vault-aware navigation and search, MCP vault scoping) shipped
  in v2.5.0; see the completion checklists below for what each phase covers.
- Audience: Hatchdoor maintainers, contributors, and product collaborators
- Horizon: Product direction; no delivery dates are committed
- Scope: One workstream of the [Hatchdoor product roadmap](product-roadmap.md) —
  how vaults are connected, configured, synchronized, indexed, and searched.
  User-facing outcomes and capabilities, not implementation design.

## Purpose

This document details the **vault lifecycle and multi-vault** workstream: how
Hatchdoor can grow from a web interface connected to one preconfigured Markdown
folder into an always-available workspace that manages several independent vaults
from the product itself. It sits under the broader
[product roadmap](product-roadmap.md), which frames the other workstreams
(editing experience, search and discovery, graph and linking, agent/MCP
capabilities) alongside this one.

It is intended to help the team agree on product intent and priorities for vault
connectivity before agents or contributors turn the roadmap into architecture
decisions, implementation plans, and pull requests.

## Product Direction

Hatchdoor should start once and then let people connect, manage, synchronize,
index, search, and use their knowledge vaults from the product itself.

A vault remains a portable collection of Markdown files. It may live in a local
directory or be shared through Git. Hatchdoor provides the stable interface
around those files: a web experience for people, an MCP interface for agents,
and reliable background services for synchronization, indexing, and search.

The longer-term product is a private knowledge workspace in which several
independent vaults can enrich one personal or team knowledge system without
losing their identity, ownership, or portability.

## Product Principles

The roadmap should preserve these principles across every phase:

1. **Markdown remains the source of truth.** Hatchdoor-generated databases and
   indexes are disposable representations of the files, not replacements for
   them.
2. **Hatchdoor remains available while background work happens.** Indexing,
   embedding, cloning, pulling, and synchronization should not prevent the
   product from starting or the UI from loading.
3. **Local-first behavior remains the default.** Existing users can continue to
   use a local vault and local embeddings without adopting Git or an external
   service.
4. **Existing deployments keep working.** Environment-based configuration and
   the current single-vault workflow remain compatible as product-managed
   configuration is introduced.
5. **Optional capabilities fail independently.** A Git host, embedding service,
   or individual vault can be unavailable without making all of Hatchdoor
   unavailable.
6. **Vault boundaries stay visible.** The product must not hide which vault a
   note or search result belongs to, especially when several vaults are used
   together.
7. **People and agents share the same knowledge safely.** Web and MCP operations
   should observe the same content, capabilities, and operational state.

## Phase 1: Product-Managed Single-Vault Foundation

### Outcome

A new user can start Hatchdoor immediately, configure one vault through the UI,
and reach useful content without editing deployment files. Existing users can
upgrade without changing their current configuration.

### 1.1 Always-Available Application

Hatchdoor should start and present a usable UI even when no vault is configured
or the configured vault is not ready.

The product should:

- report the Hatchdoor service as healthy when it can serve the UI and API;
- separate application health from vault and index status;
- perform initial indexing and reindexing as background work;
- continue serving available content while an index is being refreshed;
- show progress and useful status for long-running work;
- expose recoverable errors without requiring the process to restart; and
- allow failed background operations to be retried.

Users should be able to distinguish states such as:

- no vault configured;
- vault available but not yet indexed;
- indexing or reindexing;
- ready;
- available with a degraded optional capability; and
- unavailable or requiring attention.

### 1.2 Guided Setup and UI Configuration

When no vault is configured, Hatchdoor should guide the user through connecting
one. After setup, the same configuration should remain accessible from a
settings area.

The product should allow a user to:

- choose a local-directory or Git-backed vault source;
- give the vault a recognizable display name;
- review whether Hatchdoor can read and write the source;
- configure indexing and embedding behavior;
- test the configuration before relying on it;
- see which settings are controlled by the deployment environment; and
- update product-managed settings without rebuilding the container.

Environment variables remain supported for backward compatibility, automation,
and deployments whose configuration must be controlled outside the UI. The UI
must make externally controlled settings understandable rather than silently
overriding them.

### 1.3 Managed Git Vault

A Git repository should become a first-class way to supply and share a vault.
The initial capability is described in more detail in
[the managed Git vault proposal](https://github.com/BattermanZ/Hatchdoor/pull/18).

From a product perspective, a user should be able to:

- connect a Git repository as the vault source;
- use a public repository or authenticate to a private repository;
- choose a branch and, when needed, a vault subdirectory;
- select read-only synchronization or bidirectional synchronization;
- see cloning, pulling, local-change, push, and conflict status;
- keep working through temporary network or Git-host failures; and
- recover from errors without losing local edits.

Git is a transport and collaboration mechanism around the Markdown vault. A
Git failure should affect synchronization, not the availability of the entire
Hatchdoor application.

The proposed first increment is detailed in the
[managed Git vault foundation architecture](../architecture/managed-git-vault-foundation.md)
and its [implementation plan](../plans/managed-git-vault-foundation.md). These
are draft working documents for turning this roadmap outcome into reviewable
delivery slices; they do not expand the product-managed configuration scope of
Phase 1.

### 1.4 Configurable Embedding Provider

> **Status: exploratory, low priority, not accepted.** External inference
> contradicts
> [ADR-04](../adr/README.md#adr-04--local-cpu-only-embeddings-no-external-inference-api)
> and [ADR-05](../adr/semantic-search-strategy.md), which commit Hatchdoor to
> local, CPU-only embeddings. This section records a *possible future direction*
> only; local embeddings remain the committed approach, and adopting an external
> provider would first require amending those ADRs through the documented change
> process. It is not part of the Phase 1 commitment below.

Should Hatchdoor ever support it, semantic search would work with either the
current local embedding behavior or a separately operated embedding service. In
that case the product should:

- keep local embeddings as the default with no configuration changes required;
- allow an external embedding endpoint to be configured;
- support Ollama as an initial remote-provider candidate;
- leave room for additional providers and compatible APIs;
- let the user test provider connectivity and model availability;
- show which provider and model a vault is using;
- treat remote-provider outages as a degraded search capability rather than an
  application-health failure; and
- make re-embedding status and failures visible.

Hatchdoor must retain enough information about generated embeddings to know
whether they are compatible with the currently selected provider and model.
Changing to an incompatible configuration should never silently mix vectors.
The affected semantic index should be identified as stale and rebuilt in the
background while non-semantic product capabilities remain available.

### Phase 1 Completion Experience

Phase 1 is functionally complete when a user can:

1. Start Hatchdoor and immediately reach a healthy application.
2. Add or inspect a vault through the UI.
3. Continue using an existing environment-configured local vault unchanged.
4. Connect a local directory or managed Git repository.
5. Observe vault, Git, indexing, and embedding status independently.
6. Browse and use available notes while background work continues.

(Configurable external embedding providers are out of scope for Phase 1 — see the
status note in §1.4.)

## Phase 2: Multi-Vault Knowledge Workspace

### Outcome

One Hatchdoor installation can manage and use several independent vaults. A user
can work inside a selected vault or deliberately search across vaults to enrich
their broader knowledge base.

### 2.1 Vault Collection

The single-vault configuration becomes a collection of named vaults. Each vault
retains its own identity and lifecycle.

The product should allow a user to:

- add, edit, disable, and remove vault configurations;
- connect a mix of local and Git-backed vaults;
- see the source, availability, synchronization, index, and embedding status of
  every vault;
- open a vault-specific workspace;
- understand which vault is active at all times; and
- retry or repair one vault without disrupting the others.

Removing a vault from Hatchdoor should not imply deleting the source files or
remote repository. Any destructive option must be separate and explicit.

### 2.2 Vault Selection and Navigation

The first multi-vault experience should prioritize clarity. A vault selector can
switch the active workspace, and navigation, recent notes, graph views, and
search should initially respect that selection.

Notes should have stable, vault-aware identities and URLs. Notes with the same
name in different vaults must remain distinct.

### 2.3 Cross-Vault Discovery

After vault-specific use is reliable, Hatchdoor should allow users to search
across a selected set of vaults or across all available vaults.

Cross-vault results should:

- identify the source vault clearly;
- respect unavailable, disabled, or unindexed vaults;
- communicate when results are partial;
- support keyword and semantic discovery where each vault's index permits it;
- avoid implying that equally named notes are the same item; and
- let the user return to a vault-specific context.

Cross-vault search does not require combining every vault into one physical
index. The product promise is a coherent search experience, not a particular
storage design.

### 2.4 Shared Knowledge Through Git

Multiple vaults make Git-based knowledge sharing a core workflow. A user should
be able to subscribe to knowledge maintained by other people or teams while
keeping personal and shared vaults separate.

Examples include:

- a personal knowledge vault with bidirectional synchronization;
- a team handbook that is pulled from Git and treated as read-only;
- a project-specific vault shared among contributors; and
- reference vaults added to improve cross-vault discovery and agent context.

The origin of information should always remain visible so users and agents can
judge ownership and authority.

### 2.5 Agent Use Across Vaults

MCP clients should be able to discover available vaults and operate within an
explicit vault scope.

The product should eventually allow agents to:

- list the vaults they are permitted to see;
- search one vault, a selected set, or all permitted vaults;
- identify the vault for every returned note;
- target create, edit, move, and delete operations at one explicit vault; and
- understand when a vault is read-only, unavailable, or only partially indexed.

Write operations should never choose a destination vault implicitly when more
than one writable vault is available.

### Phase 2 Completion Experience

Phase 2 is functionally complete when a user can:

1. Configure multiple local and Git-backed vaults in one Hatchdoor instance.
2. Switch between vaults without restarting Hatchdoor.
3. See and manage each vault's state independently.
4. Search within one vault and intentionally across several vaults.
5. Identify the origin of every note and search result.
6. Use MCP with explicit, safe vault scoping.
7. Keep using healthy vaults when another vault fails or is being rebuilt.

## Later Opportunities

The following capabilities fit the direction but are not committed parts of
Phase 1 or Phase 2:

- cross-vault links, backlinks, and graph relationships;
- curated collections that span selected vaults;
- source priority or authority indicators for overlapping knowledge;
- notifications for Git conflicts, stale indexes, or failed background jobs;
- export, backup, and migration workflows managed from the UI;
- per-vault retention, archive, and trash policies;
- additional remote embedding providers;
- remote content sources other than local directories and Git repositories;
- multiple Hatchdoor users with vault-level access control; and
- collaborative editing or real-time presence.

Multi-vault support should not be described as multi-tenancy by itself.
Authentication, user accounts, authorization, isolation, and audit policy form
a separate product initiative that should be designed explicitly if Hatchdoor
is later intended to serve mutually untrusted users.

Vault-level access scoping for multiple users/agents is tracked as its own
initiative — see [Multi-user, network-exposed deployment](product-roadmap.md#multi-user-network-exposed-deployment)
in the product roadmap — but this document's multi-vault work is a
prerequisite for it, met as of v2.5.0.

## Decisions to Make Together

These questions should be resolved during roadmap review or the later design
stage. They do not need to block agreement on the overall direction.

### Configuration Ownership

- When both environment and UI configuration exist, which values are editable,
  inherited, or locked?
- Should an environment-configured vault appear as a normal vault that can be
  supplemented with UI-managed vaults in Phase 2?
- Where should product-managed secrets be stored, and how should backup and
  migration treat them?

### First-Run Experience

- Should Hatchdoor create a starter vault, invite the user to connect one, or
  offer both choices?
- What is the minimum setup that demonstrates value without exposing too many
  advanced options?

### Git Collaboration

- Which authentication methods are essential for the first managed-Git release?
- What conflict-recovery actions belong in the UI, and which should initially be
  handled outside Hatchdoor?
- How should read-only shared vaults communicate that their remote repository is
  authoritative?

### Embeddings

- Is Ollama the preferred first remote provider, or should Hatchdoor begin with
  a more general compatibility contract?
- Should embedding configuration be global by default with optional per-vault
  overrides?
- What should users be able to do while a vault is being re-embedded?

### Multi-Vault Experience

- Should the first release provide only a vault selector, or also cross-vault
  search?
- Which screens are naturally vault-specific, and which should offer an
  all-vault view?
- Should a shared Git vault default to pull-only mode?
- How should agents select their default read scope and write destination?

## Explicit Non-Goals for This Roadmap

This roadmap does not yet define:

- implementation architecture, internal service boundaries, or database schema;
- API, environment-variable, or configuration-field names;
- an exact embedding compatibility fingerprint;
- task breakdowns, estimates, release dates, or contributor assignments;
- detailed security and threat models;
- migration mechanics;
- Git conflict-resolution algorithms; or
- a multi-user permission model.

Once the product roadmap is agreed, these topics should be captured in focused
[architecture decision records](../adr/README.md), functional specifications, and
implementation plans.

## Moving From Roadmap to Implementation

Before implementation begins, the team should:

1. Agree on the Phase 1 and Phase 2 outcomes and their ordering.
2. Resolve the product decisions that materially affect user behavior.
3. Define measurable acceptance criteria for the first increment of each phase.
4. Record new — or amend existing — [architecture decision records](../adr/README.md)
   to preserve Hatchdoor's safety and compatibility constraints. Several accepted
   ADRs already bear on this workstream:
   [ADR-01](../adr/README.md#adr-01--markdown-is-the-source-of-truth-sqlite-is-a-disposable-read-model)
   (Markdown stays authoritative),
   [ADR-10](../adr/README.md#adr-10--optional-git-sync-as-a-debounced-background-task)
   (git sync is a debounced background task), and
   [ADR-08](../adr/README.md#adr-08--bearer-token-auth-with-a-query-param-fallback)/[ADR-09](../adr/README.md#adr-09--mcp-off-by-default-own-token-origin-allowlist)
   (auth). Note that the **configurable external embedding provider** direction in
   §1.4 currently conflicts with
   [ADR-04](../adr/README.md#adr-04--local-cpu-only-embeddings-no-external-inference-api)
   (local, CPU-only embeddings; no external inference API) and
   [ADR-05](../adr/semantic-search-strategy.md); adopting it requires amending
   those ADRs through the documented change process, not working around them.
5. Break one approved increment at a time into agent-ready implementation work.

This keeps agents working from an agreed product direction while leaving room
for technical design to be reviewed separately.
