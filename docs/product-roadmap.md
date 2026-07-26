# Hatchdoor Product Roadmap

- Status: Draft for discussion
- Audience: Hatchdoor maintainers, contributors, and product collaborators
- Horizon: Product direction. Version hints below (v2.4, v3, …) indicate rough
  ordering and intent, not committed release dates.
- Scope: The overall product direction and the workstreams it breaks into. Each
  workstream is a problem to solve; some are detailed in their own document under
  [`docs/roadmap/`](roadmap/), the rest are stated here to set direction.

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

### Polished, publishable UI/UX

**Problem:** the web experience needs to be good enough to show publicly.

Direction: address the open UI/UX issues and raise the overall quality of the web
experience to a publishable bar. Tracked issues:

- [#7 — Improve attachment UX](https://github.com/BattermanZ/Hatchdoor/issues/7)
- [#8 — Improve presentation of submenu](https://github.com/BattermanZ/Hatchdoor/issues/8)
- [#9 — Add an onboarding experience](https://github.com/BattermanZ/Hatchdoor/issues/9)
- [#10 — Messy UX hierarchy](https://github.com/BattermanZ/Hatchdoor/issues/10)
- [#11 — Create-note interaction is odd](https://github.com/BattermanZ/Hatchdoor/issues/11)
- [#12 — Sidebar layout is odd](https://github.com/BattermanZ/Hatchdoor/issues/12)

Related but broader than pure UI polish: [#13 — Global Settings](https://github.com/BattermanZ/Hatchdoor/issues/13)
and [#14 — Live editing content](https://github.com/BattermanZ/Hatchdoor/issues/14).

_Horizon: **v2.5.0**._

### Agent-driven ingestion

**Problem:** agents can edit notes but cannot bring new material into the vault.

Direction: let agents **send files through MCP for ingestion** into the vault, so
content can be captured by agents, not only authored by hand. _To be detailed._

_Horizon: **v2.4**._

### Vault lifecycle & multi-vault

**Problem:** vaults are configured before startup via environment variables and a
single instance serves only one folder. Users need to connect, configure, sync,
and manage vaults from the product, and eventually run several at once.

Direction: detailed in the
[vault lifecycle & multi-vault roadmap](roadmap/vault-lifecycle.md). Also includes
**excluding sub-folders from indexing**
([#22](https://github.com/BattermanZ/Hatchdoor/issues/22)) when Hatchdoor points at
a directory, so parts of a vault can be kept out of the index.

_Horizon: ongoing; managed Git vaults are proposed in
[PR #18](https://github.com/BattermanZ/Hatchdoor/pull/18)._

### Multi-user, network-exposed deployment

**Problem:** Hatchdoor assumes a single trusted user on a private deployment.

Direction: support **several users on one instance with scoped permissions**, with
the app **exposed online**. This is a multi-tenancy initiative — authentication,
accounts, authorization, isolation, and audit must be designed explicitly, since
it means serving mutually untrusted users.

_Horizon: **v3** (long-term)._

## How to Use This Roadmap

1. Agree on the vision, objectives, and the relative priority of the workstreams.
2. For a prioritized workstream, review or write its detailed roadmap document.
3. Turn agreed outcomes into architecture decisions, functional specifications,
   and implementation plans — one increment at a time.

This keeps contributors and agents working from an agreed product direction while
leaving technical design to be reviewed separately.
