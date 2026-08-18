---
tags: [type/explanation, topic/security]
---

# The security model

Hatchdoor has three separate secrets, not one. Each protects a different boundary, and none of them substitutes for another. Knowing which one gates what matters before you decide who gets which credential.

## The three secrets

| Secret | Protects | Configured as |
| --- | --- | --- |
| **Web bearer token** | The browser and the HTTP API — `/api/v1/vaults/...`, Settings, model setup | `HATCHDOOR_WEB_BEARER_TOKEN` |
| **MCP bearer token** | Agent access via `/mcp`, further split into read and write | `HATCHDOOR_MCP_BEARER_TOKEN` |
| **Git HTTPS credentials** | Hatchdoor's own outbound connection to a Git remote | Per-Vault, entered in Settings or via `create_vault`/`edit_vault` |

The first two answer "who can reach Hatchdoor and do what." The third answers a completely different question — "how does Hatchdoor authenticate itself to somewhere else." A leaked Git token lets someone push to your repository as Hatchdoor; it grants nothing against Hatchdoor itself. Don't conflate the two kinds.

## Web bearer token

`require_web_token` middleware guards the browser and the HTTP API — Settings, model setup, and every `/api/v1/vaults/...` route when a token is configured. It's checked as `Authorization: Bearer <token>`, or as an `access_token` query parameter for contexts that can't set headers, like an `<img>` tag pointing at a downloaded attachment. Comparison is constant-time, so response timing can't leak how much of a guessed token was correct.

> [!warning]
> A non-loopback `HOST` with no web token configured refuses to start at all — Hatchdoor generates a fresh token, prints it, and asks you to add it to `.env` before it will run unauthenticated on a public interface. This is a hard startup check, not a warning you can ignore. See [[Install Hatchdoor with Docker Compose]] for what this looks like in practice.

## MCP bearer token

MCP has its own middleware and its own secret, checked independently of the web token. Two things make this look stricter than the web token, and they are:

- **MCP requires a token even to enable it at all.** Turning on `HATCHDOOR_MCP_ENABLED` without also setting `HATCHDOOR_MCP_BEARER_TOKEN` is a startup validation error — MCP simply refuses to come up. There's no equivalent of "loopback-only, no token needed" for MCP the way there sometimes is for the web token.
- **Read access needs the token too.** Read-only MCP still exposes the entire Vault — `search_notes`, `get_note`, `get_tree`, and the rest — through a transport that bypasses the web auth layer entirely. So the same bearer-token requirement applies whether or not write mode is on.

On top of the token, two more gates apply, independently:

- **`HATCHDOOR_MCP_ENABLED`** — off by default. When it's off, `/mcp` doesn't just refuse requests, it returns `404` — the endpoint isn't advertised as existing at all.
- **`HATCHDOOR_MCP_WRITE_ENABLED`** — off by default even once MCP itself is on. This is what separates an agent that can only look around from one that can also create, edit, move, or delete notes. See [[MCP tools reference]] for exactly which tools each gate unlocks.

MCP also checks the request's `Origin` header against an allow-list (`HATCHDOOR_MCP_ALLOWED_ORIGINS`) as a defense against DNS-rebinding attacks, and validates the `MCP-Protocol-Version` header — neither of those is a secret, but both run before the bearer-token check on every request.

> [!note]
> The web token and the MCP token are unrelated on purpose. An agent's MCP token leaking doesn't hand out Settings or Web UI access, and revoking one never requires rotating the other. Give an agent the MCP token, never the web token — it should never need Settings access to do its job.

## Git HTTPS credentials

A Vault's own remote — configured per-Vault, not instance-wide — can carry an HTTPS token for Hatchdoor to authenticate itself when it fetches or pushes. This is stored write-only: once saved, it's never echoed back in any read, and even internal debug output redacts it. Editing a Vault's credentials is a `keep` / `remove` / `replace` choice specifically so a stored secret never has to round-trip back to you just to survive an edit — see [[HTTP API reference]] for the exact request shape.

This secret authenticates Hatchdoor *to the remote*, not a caller *to Hatchdoor*. It doesn't appear anywhere in the web-token or MCP-token checks above, and holding it grants no access to Hatchdoor itself — only to whatever the remote lets that token do.

## What's open regardless of any token

`/health`, `/ready`, `/api/startup-status`, and `/api/vault-status` are never gated by any token — they're liveness/readiness probes, meant to be checked by infrastructure (a container orchestrator, a load balancer) that has no credential to present. They report process and indexing state, never Vault content.

## Demo mode's narrower rule

A public, read-only demo (`HATCHDOOR_DEMO_MODE=true`) doesn't just relax these rules — it restructures them, and the result isn't uniform across surfaces:

- Settings and model setup stop existing entirely — `404`, not a refusal.
- Vault reads become public and unauthenticated, by design — that's the point of a demo.
- Vault writes and Vault control still exist as routes, but answer `403 demo_read_only` rather than performing the action.

Demo mode and MCP are mutually exclusive at startup: Hatchdoor refuses to run with both `HATCHDOOR_DEMO_MODE=true` and `HATCHDOOR_MCP_ENABLED=true` set together. A demo has no operator to hold an MCP token in the first place, so the two postures don't compose.

---

Related: [[Connect your agent]] · [[HTTP API reference]] · [[MCP tools reference]]
