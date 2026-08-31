---
tags: [type/how-to, topic/troubleshooting]
---

# How to troubleshoot common problems

Find your symptom below. Each entry gives the real cause, grounded in what Hatchdoor actually checks, and the fix.

## Hatchdoor won't start

The container exits immediately and the logs show a line like `HOST=0.0.0.0 is non-loopback but HATCHDOOR_WEB_BEARER_TOKEN is unset: refusing to start unauthenticated on a public interface`, followed by a freshly generated token and the exact line to paste into `.env`. This is deliberate: the container's own listener is never loopback (`HOST: 0.0.0.0` is fixed in `compose.yaml` so Docker's port publishing can reach it at all), so Hatchdoor refuses to serve mutating routes unauthenticated the moment it isn't bound to `127.0.0.1`/`localhost`.

Fix: `docker compose logs hatchdoor | grep HATCHDOOR_WEB_BEARER_TOKEN | tail -1` — grab the **most recent** one, since `restart: unless-stopped` means a crash loop generates a new token on every attempt. Paste the assignment into `.env`, then `docker compose up -d` again.

> [!warning]
> If you don't catch this within a few restarts, older tokens in the log are stale and won't match what a later attempt actually needs — always take the last one, not the first you see.

A second, rarer startup failure: `HATCHDOOR_MCP_ENABLED is set but HATCHDOOR_MCP_BEARER_TOKEN is missing`. This only happens if you set `HATCHDOOR_MCP_ENABLED=true` directly in `.env` before ever starting — the normal path is to enable MCP live from **Settings** after the container is already running (see [[Connect your agent]]), which doesn't hit this check at all. Fix: either also set `HATCHDOOR_MCP_BEARER_TOKEN` in `.env`, or remove `HATCHDOOR_MCP_ENABLED` from `.env` and enable MCP from Settings instead.

## Browser asks for a token you don't have

You bound Hatchdoor to a non-loopback host and it generated one on first start (see above) — it isn't something you chose, it's printed once to the container logs. Fix: `docker compose logs hatchdoor | grep HATCHDOOR_WEB_BEARER_TOKEN | tail -1`, or if you already saved it to `.env` and just forgot it, read the value straight out of that file — Hatchdoor never displays it back to you once set, by design (revealing an already-known credential grants no new access, but Hatchdoor still won't show a browser session a token it hasn't already proven it holds).

## An agent can't connect over MCP

Five distinct failures, distinguishable by the response:

- **`404 Not Found` on `/mcp`, with an empty body** — MCP is disabled. Turn on **Let assistants connect (MCP)** in **Settings** → **Agent access (MCP)**. Read the body before acting on a `404`: an empty one means this, and a `404` that *says* something means the next entry instead.
- **`404 Not Found` whose body reads `Not Found: Session not found`** — MCP is on and the client got in, but the session it is quoting doesn't exist. Either it was never issued, or it was issued before Hatchdoor last restarted. The remedy belongs to the client: it has to run `initialize` again and use the session that comes back.
- **`422 Unprocessable Entity`, "Unexpected message, expect initialize request"** — the same problem from the other side: the client sent an ordinary call with no session at all. The wording describes what the server was expecting to receive rather than what you should do about it; the remedy is the one above, re-initialize.
- **`401`, JSON-RPC error `-32001`, "Missing or invalid MCP bearer token"** — the client's `Authorization: Bearer <token>` header doesn't match the current MCP password. Regenerate or re-copy it from Settings; the client and Settings must hold the exact same value.
- **A write tool (`create_note`, `edit_vault`, etc.) returns "MCP write tools are disabled by HATCHDOOR_MCP_WRITE_ENABLED"** — MCP is connected and reading fine, but **Let assistants change notes** is off. This is a separate toggle from connecting at all; see [[Search and change notes with your agent]] for why that separation exists.

A sixth, less common one: **`403 Forbidden`, "Forbidden MCP origin"** — an `Origin` header was sent that isn't on the allow-list (`HATCHDOOR_MCP_ALLOWED_ORIGINS`). This normally only matters for a browser-based MCP client, not a CLI agent.

> [!note]
> MCP sessions are held in memory only, so restarting Hatchdoor ends every one of them. A client holding a session finds out on its next call — as the `Session not found` or the `422` above — and has to re-initialize. No Hatchdoor setting keeps sessions across a restart, so a client that keeps retrying the same failing call needs restarting or reconnecting; that lever is yours, not Hatchdoor's. Clients that don't use a session at all are unaffected.

## Model download is stuck or failed

Check `get_model_setup_status` (MCP) or `GET /api/startup-status` — a failed download reports `"failed"` with a message describing exactly what broke, e.g. a Hugging Face fetch error for a specific model file. Fix: `POST /api/model/retry` (or ask the agent to call `get_model_setup_status` again and retry) once whatever blocked the download — usually network access to Hugging Face — is resolved.

Two related, more specific errors:

- **`409`, "Gemma terms must be accepted or declined first."** — you tried to retry before choosing a model at all. Accept or decline Gemma first.
- **`409`, "The running search model cannot be changed until Hatchdoor restarts."`** — a model is already active; Hatchdoor doesn't support switching models live. This is expected, not a bug — restart the instance if you genuinely need a different model.

## A Vault won't index or stays in a bad state

`list_vaults` (or the Vault's own Settings page) reports five independent status axes, not one health flag — a Vault can be broken in one dimension while working fine in every other:

| Axis | Values | What it means |
| --- | --- | --- |
| `activation` | `active`, `disabled`, `unavailable` | Whether the Vault definition itself is usable |
| `local_content` | `read_write`, `read_only`, `unavailable` | Whether the authoritative Markdown folder can be read/written |
| `search` | `unavailable`, `indexing`, `browsable`, `ready`, `stale` | Whether search/browse actually work right now |
| `git` | `disabled`, `pending`, `ready`, `unavailable` | Only meaningful for a Git-backed Vault |
| `watcher` | `running`, `disabled`, `unavailable` | Whether file-change detection is active |

`browsable` (not `ready`) right after connecting a Vault isn't broken — it means structural indexing finished but the semantic embedding pass hasn't yet, which happens once per Vault's first successful index. `stale` means content changed and a reindex hasn't completed yet; give it a moment. Anything reporting `unavailable` carries a matching `*_error` field (`activation_error`, `search_error`, `git_error`, `watcher_error`) with a real error code and message — read that field before guessing at a cause.

## Permission denied reading or writing the Vault

`local_content` reports `unavailable` with an error code of `vault_path_unreadable` or `vault_path_unavailable`, and the message includes the underlying OS error (e.g. `Permission denied (os error 13)`). This is almost always the container's UID: the image runs as the numeric `nonroot` user (UID `65532`), and the host folder mounted as the Vault needs to be readable — and writable, if agents or the Web UI should change notes — by that UID specifically, not just by your own host user. See [[Install Hatchdoor with Docker Compose]] for the exact `chown`/`chmod` commands.

A softer variant: `local_content` reports `read_only` rather than `unavailable` — the folder is readable but not writable by UID `65532`. This isn't an error state; it just means Hatchdoor can browse and search the Vault but not write to it. Fix the same way if write access is what you actually wanted.

## Git sync is failing

Check the Vault's Sync console for the specific failure rather than assuming — these need different fixes:

- **Authentication failed** — the stored HTTPS token was rejected by the remote. Re-enter it under **Sign-in** on the Vault's own page; see [[How to set up a Git-backed Vault]].
- **Clone/fetch failed, or the remote is unreachable** — a network or DNS problem, or the repository URL itself is wrong. Confirm the URL resolves from wherever the container runs, not just from your own machine.
- **Local commits ahead on a Pull-only Vault** — Hatchdoor made local commits (from note edits) that a Pull-only Vault is configured never to push. This isn't a failure exactly — it's Hatchdoor accurately reporting that local history and the remote have diverged and staying pull-only rather than silently discarding your local commits. Switch the Vault to **Two-way** if you want those commits pushed, or accept that Pull-only Vaults are meant to be read-mostly.

> [!note]
> A Vault reporting `git: pending` isn't stuck by default — that's the normal state while a clone or fetch is in flight. Only treat it as a problem if it stays `pending` well past the configured sync interval.

## "Is this Vault still syncing on schedule?"

`GET /api/v1/vaults` reports `last_checked_at` and `next_attempt_at` for every Vault with a remote — answer the question from those rather than from the repository's Git history. `last_checked_at` is when Hatchdoor last *tried*, not when it last succeeded, so read it next to the Vault's Git status: a Vault that is checking on schedule but failing every time shows a recent `last_checked_at` and an `unavailable` status with the reason. A check that finds nothing new leaves no trace in `git log` or `git reflog`, so an unchanged remote-tracking branch is not evidence that Hatchdoor stopped checking; it usually means there was nothing to fetch.

If `next_attempt_at` is in the past by more than a minute or so, something is genuinely wrong. If it's in the future, the Vault is simply waiting out its interval — **Sync now** on the Vault's page overrides it, and shortening the Vault's sync schedule brings `next_attempt_at` forward to one new interval after `last_checked_at` — unless the Vault is mid-retry after a failed check, where the retry's own timing wins and `next_attempt_at` deliberately does not move. Read it next to the Git status: on a `ready` Vault a shortened schedule moves it, on an `unavailable` one it may not until a check succeeds. Note that restarting Hatchdoor no longer forces a sync: a Vault inside its interval resumes the countdown across a restart, so restarting is no longer a way to prod a Vault into checking.

## Search returns nothing, or not what you expected

Before assuming something's broken: search only considers the **default surface** unless you explicitly ask for more. If the note you expected lives under a [[The layer system|layer]], it won't appear in an ordinary search — see [[How to organize a Vault with layers]] for how to search across layers deliberately. If a Vault's `search` status is `browsable` rather than `ready` (see above), semantic search over it isn't available yet, but keyword search and browsing already work.

---

Related: [[Connect your agent]] · [[How to set up a Git-backed Vault]] · [[Install Hatchdoor with Docker Compose]] · [[HTTP API reference]]
