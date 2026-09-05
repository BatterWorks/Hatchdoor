---
tags: [type/explanation, topic/vaults]
---

# Vault lifecycle states

`list_vaults` and the Settings screen report a Vault's condition, but not as one word like "healthy" or "ready" — that single-word framing doesn't actually exist. A Vault's condition is five independent signals plus one operator switch, and conflating them is the most common way to misread what's actually happening.

> [!note]
> "Healthy" and "degraded" aren't states Hatchdoor reports anywhere — they're not in the API, not in the Settings UI copy, not in the source. The closest the UI comes is describing a Git-backed Vault's sync as "healthy" in one sentence, which just means its Git status isn't `unavailable`. Don't look for a `degraded` value; it doesn't exist.

## `enabled` is the operator's switch, nothing else

Every Vault definition carries one boolean, `enabled`, set by the operator via **enable_vault**/**disable_vault** (or the equivalent Settings toggle). It answers exactly one question: should this Vault be running at all? It says nothing about whether the Vault is currently working — a Vault can be enabled and still be unavailable (bad path, broken remote, mid-index), or disabled while its last-known runtime state is simply frozen in place.

## The five status axes

Once a Vault is enabled, Hatchdoor tracks its condition on five separate axes rather than one combined phase. They can (and routinely do) disagree with each other — a Vault can be browsable while still indexing, or have a broken Git remote while its Markdown is perfectly readable:

| Axis | Values | What it answers |
| --- | --- | --- |
| **Activation** | `active`, `disabled`, `unavailable` | Is this Vault's runtime slot up at all? |
| **Local content** | `read_write`, `read_only`, `unavailable` | Can the authoritative Markdown on disk currently be read/written? |
| **Search** | `unavailable`, `indexing`, `browsable`, `ready`, `stale` | What state is the search index in? |
| **Git** | `disabled`, `pending`, `ready`, `unavailable` | Is Git sync (if configured) working? |
| **Watcher** | `running`, `disabled`, `unavailable` | Is the file-change watcher keeping the index current? |

**Git** survives a restart for a Vault Hatchdoor polls on a schedule — one with a remote to check. A Vault that last checked cleanly comes back as `ready`, and one whose last check failed comes back as `unavailable` with the same reason it showed before, so `pending` means a Vault that has genuinely never completed a check rather than one Hatchdoor has simply forgotten about. A Vault with no remote to poll — an `existing_git` Vault in `local_history` mode, which only records your own edits — has no schedule to resume, so it starts as `pending` and settles on its first turn after startup. Alongside these, `GET /api/v1/vaults` reports `last_checked_at` and `next_attempt_at` for any Vault with a remote.

**Search** deserves the closest look, because its middle values are easy to misread:

- `indexing` — actively building; nothing usable yet for this axis.
- `browsable` — a real, load-bearing state, not a typo for "ready." The Vault's structure (notes, links, headings) is published and current, but this generation has no vectors yet. You can open and read every note; semantic search returns nothing. This is reached once, on a Vault's very first successful index, between the structure pass and the embedding pass — a later rebuild of an already-searchable Vault never regresses through it, it just keeps serving the prior generation while the rebuild runs.
- `ready` — fully current, structure and vectors both.
- `stale` — search still works, but what it answers from is a build behind. Three ways to get here: a newer build is in progress, the last build failed, or a note was written *during* the build that just finished, so the generation it published was already behind the moment it landed. That last one is normal during a bulk edit or migration — every write arms the next reindex, and the Vault settles on `ready` once the writing stops. Not an error by itself, just "what you're seeing might be a build behind."

## What capabilities actually come from

`browse`, `search`, `mutate`, `pull`, `push`, and `retry` — the six flags that decide what the UI shows and what an MCP/API write is allowed to do — are derived from combinations of the axes above, not from any single one:

- **`browse`** — true whenever local content is `read_write` or `read_only`. Notably independent of the search axis: a Vault mid-index (or even stuck at `browsable`) is still fully browsable.
- **`search`** — true only for `ready` or `stale`. `browsable` and `indexing` both grant `browse` but not `search`.
- **`mutate`** — true only when local content is `read_write` *and* the Vault isn't a `pull_only` Git Vault. A `pull_only` Vault never allows local edits, regardless of how healthy everything else looks, since edits would just conflict with the next pull.
- **`pull`** / **`push`** — true only when Git status is `ready`, gated further by the configured Git mode (`pull_only` or `two_way` for pull; `two_way` only for push).
- **`retry`** — true if *any* of the four per-axis error fields (activation, search, git, watcher) is marked retryable. This is what puts a **Try again** button in front of an operator instead of leaving a Vault silently stuck.

One instance-wide exception: on a public read-only demo (`HATCHDOOR_DEMO_MODE=true`), `GET /api/v1/vaults` reports `mutate`, `pull`, `push` and `retry` as `false` for every Vault, whatever the axes say. Nothing about the Vault changed. The demo refuses every write and every Vault-control request with `403 demo_read_only`, so publishing the derived value would advertise a button that cannot work to a visitor who has no way to tell. `browse` and `search` are still derived normally, because those reads do work, and the axes themselves are untouched: a demo Vault on a writable folder still reports local content `read_write`.

## Two different things both called "Recovery"

There are two unrelated recovery mechanisms, and they don't overlap:

1. **Registry recovery** — the registry file itself (`vaults.json`) fails to load: it's corrupt, or its schema version is unsupported or from a newer Hatchdoor than this one. Every registry-mutating request answers `503 vault_registry_recovery_required` until the file is fixed on disk; there's no in-app action that resolves this, because the thing that's broken is the very store every other recovery path would need to write to.
2. **Legacy-migration recovery** — narrower and specific to the one-time import of a pre-registry, single-folder `.env` deployment. If that import fails, Hatchdoor exposes **start_with_no_vaults** as an explicit escape hatch (`POST /api/v1/vaults/start-with-no-vaults`, requiring `{"confirm": true}`) to start from an empty, working registry rather than staying stuck. This has nothing to do with any individual Vault's own health.

Neither of these is a per-Vault state — both describe the registry as a whole being unable to tell you about any Vault at all, which is a different kind of problem than one Vault having a bad Git remote or a locked file.

> [!warning]
> There's also an older, single-Vault-only `TermsRequired → Downloading → Validating → Scanning → Indexing → Ready → Unavailable` phase sequence still present in the code, from before the multi-vault registry existed. It only governs the legacy first-run embedding-model setup (accepting Gemma's terms, downloading a model) and has no bearing on how a Vault created through the registry is described — don't confuse it with the five-axis model above if you come across it.

---

Related: [[Connect your first Vault]] · [[How to set up a Git-backed Vault]] · [[HTTP API reference]]
