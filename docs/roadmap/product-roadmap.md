# Hatchdoor Product Roadmap

- Status: Draft for discussion
- Audience: Hatchdoor maintainers, contributors, and product collaborators
- Horizon: Product direction. Version hints below (v2.4, v3, …) indicate rough
  ordering and intent, not committed release dates.
- Scope: The overall product direction and the workstreams it breaks into. Each
  workstream is a problem to solve; some are detailed in their own document under
  [`docs/roadmap/`](./), the rest are stated here to set direction.

## Vision

Hatchdoor is a self-hosted, Markdown-first knowledge workspace where **people and
agents work on the same notes safely**. A web experience serves people, an MCP
interface serves agents, and portable Markdown files stay the source of truth —
never locked behind a database or a service.

## Strategic Objectives

Everything on this roadmap should advance at least one of these:

1. **Markdown stays portable and authoritative.** Generated indexes and databases
   are disposable representations of the files, not replacements.
2. **Humans and agents collaborate without fear.** Every change is visible,
   attributable, and reversible; mistakes are contained.
3. **Always available and easy to self-host.** The app starts and stays usable
   even while background work runs or optional capabilities fail.
4. **Grows without losing itself.** From single-user/single-vault today toward
   many vaults and, eventually, many users — without compromising the objectives
   above.

## Workstreams

Each workstream states the problem it solves. Version hints are direction, not
commitments; unversioned items are wanted but not yet slotted.

### Trustworthy human + agent collaboration

**Problem:** an agent can make a perfectly atomic change that is still a clean
disaster — e.g. "archive 400 notes because I misunderstood the folder." Users
need to see and undo what agents do, and humans and agents must not silently
clobber each other.

Direction:

- Make **agent provenance impossible to miss** in the web UI — which agent/token
  changed a note, shown alongside the change.
- Show a **before/after diff** and the **linked commit** for every agent edit.
- Offer **one-click revert** of an agent change.
- Give destructive operations (move/delete/archive) a **dry-run preview** and
  **per-agent scopes** so an agent can only touch what it is permitted to.
- **Surface conflicts** when a human and an agent edit the same note between
  index cycles, rather than losing one side.

_Horizon: unversioned ("at some point"), but high-value for trust and safety.
Some conflict-diff scaffolding already exists in the frontend._

### Vault integrity checks for agent-maintained wikis (lint)

**Problem:** the layer system was built for the agent-wiki pattern — raw
material demoted into its own layer, an agent ingesting into it and querying
the curated surface (see the "LLM wiki" research note under
[`docs/research/karpathy-llm-wiki/`](../research/karpathy-llm-wiki/)) — but
that pattern's third leg, **lint**, has no home in Hatchdoor today. Nothing
surfaces orphaned notes, broken wikilinks, or a layer whose marker vanished
while its notes are still tagged with it; a Vault an agent maintains
unattended can silently drift out of shape. The building blocks partially
exist (`build_layer_diagnostics` in `src/handlers/diagnostics.rs`), but the
route and MCP tool that once exposed it (`/api/diagnostics`,
`layer_diagnostics`) were both retired in the multi-vault rewrite with no
Vault-scoped replacement.

Direction: design and ship a Vault-scoped integrity surface — an MCP tool and
matching HTTP endpoint — covering at minimum: orphaned notes (no backlinks in
or out), broken wikilinks, layer markers with vanished declarations (notes
still tagged with a layer no marker claims), and disagreeing layer
descriptions. Read-only to start; whether it becomes agent-callable as
routine wiki maintenance, a Web UI diagnostics panel, or both is still open.

_Horizon: unversioned ("at some point"). No implementation started._

### Onboarding experience

**Problem:** a first-time user lands in Hatchdoor with no onboarding.

Direction: build a first-run onboarding flow, tracked as
[#9](https://github.com/BattermanZ/Hatchdoor/issues/9).

_Horizon: **v2.6.0**._

### Vault lifecycle & multi-vault

**Problem:** vaults are configured before startup via environment variables and a
single instance serves only one folder. Users need to connect, configure, sync,
and manage vaults from the product, and eventually run several at once.

Direction: detailed in the
[vault lifecycle & multi-vault roadmap](vault-lifecycle.md).

_Horizon: ongoing; multi-vault management and managed/Git-backed vaults shipped
in v2.5.0 (see the [vault lifecycle & multi-vault roadmap](vault-lifecycle.md)
for what remains)._

### Adopt the 2026-07-28 MCP specification

**Problem:** Hatchdoor's MCP server implements protocol version `2025-11-25`
(`src/mcp/config.rs`); the current spec, `2026-07-28`, is a substantial rewrite,
not an incremental bump.

Direction: migrate to `2026-07-28`. Headline changes to account for: the
protocol becomes **stateless** (handshake and session header removed, replaced
by a mandatory `server/discover` RPC and per-request `_meta`); live updates
move to a single opt-in `subscriptions/listen` stream, replacing SSE
subscribe/unsubscribe; `ping`, `logging/setLevel`, and stream resumability are
removed; **Roots, Sampling, and Logging are deprecated**; server-initiated
requests (elicitation, sampling) move to a request/retry pattern instead of
being pushed mid-call. HTTP+SSE transport is now formally deprecated too.

_Horizon: **v2.6.0** — landed on `development`. The boundary now runs on rmcp
(ADR-17) and advertises exactly `2026-07-28` and `2025-11-25`: stateless
discovery with per-request `_meta`, a single opt-in `subscriptions/listen`
stream, typed results with an `outputSchema` on every tool, and per-tool quotas
with concurrency caps and `429 Retry-After`. The catalogue then grew from 35 to
39 tools (`get_frontmatter`, `update_frontmatter`, `get_attachment`, `batch`),
purely additively._

### User documentation, in-app and answerable by the agent

**Problem:** the only documentation a new user has today is `README.md` and
contributor-facing material under `docs/` — there's no proper end-user
documentation, and no way to ask a question and get an answer without leaving
the app.

Direction: write proper **end-user documentation** as its own Hatchdoor
vault, hosted as a **separate demo instance** — Hatchdoor is already a
knowledge base, so the docs live in the product rather than a separate docs
toolchain. Make that instance queryable from inside the app **through the
existing MCP interface**: the in-app agent adds the docs instance as an
MCP-reachable source, so a user can ask a question ("how do I connect a Git
vault?") and get an answer grounded in the docs without leaving the app.

_Horizon: **v2.6.0**._

### Multi-user, network-exposed deployment

**Problem:** Hatchdoor assumes a single trusted user on a private deployment.

Direction: support **several users and agents on one shared instance**, each
scoped to the vaults they're permitted to see. A user or agent should only be
able to list, search, read, and write the vaults granted to them — e.g. a
personal vault, a shared/common vault, and nothing else — while an admin role
sees and manages everything. This turns vault access into a first-class
permission boundary, not just a UI convenience, and is what makes Hatchdoor
usable by more than one person/agent per instance (a household, a small team)
rather than only self-hosted single-tenant. Authentication, accounts,
authorization, isolation, and audit must be designed explicitly, since it means
serving mutually untrusted users and agents on shared infrastructure.

_Horizon: **v3** (long-term). Depends on the multi-vault work in
[vault lifecycle & multi-vault](vault-lifecycle.md) existing first — you can't
scope access to vaults that aren't yet independent, addressable entities._

## How to Use This Roadmap

1. Agree on the vision, objectives, and the relative priority of the workstreams.
2. For a prioritized workstream, review or write its detailed roadmap document.
3. Turn agreed outcomes into architecture decisions, functional specifications,
   and implementation plans — one increment at a time.

This keeps contributors and agents working from an agreed product direction while
leaving technical design to be reviewed separately.
