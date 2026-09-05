---
tags: [type/reference, topic/http-api]
---

# HTTP API reference

Every HTTP endpoint Hatchdoor exposes, grouped by area. This is a dictionary, not a walkthrough — see [[How to deploy Hatchdoor with an agent]] or [[Install Hatchdoor with Docker Compose]] for task-oriented setup steps.

All request and response bodies are JSON unless noted. Errors from the `/api/v1/vaults/...` group share one shape:

```json
{ "code": "vault_not_found", "message": "...", "vault_id": "...", "retryable": false }
```

`vault_id` is omitted when an error is not about one specific Vault.

## Auth model

| Surface | Auth |
| --- | --- |
| `/health`, `/ready`, `/api/startup-status` | None, always |
| `/api/model/*` | Web bearer token (if configured); **absent entirely (`404`) in demo mode** |
| `/api/settings*` | Web bearer token (if configured); **absent entirely (`404`) in demo mode** |
| `/mcp` | Its own MCP bearer token — see [[Connect your agent]] |
| `/api/v1/vaults/...` reads (`GET`) | Web bearer token if configured, **unauthenticated in demo mode** |
| `/api/v1/vaults/...` writes and Vault control | Web bearer token if configured; **refused with `403 demo_read_only` in demo mode** (not `404` — the route exists, it just declines) |
| `/api/v1/vaults/{vault_id}/attachments` (upload) | Web bearer token **or** a live MCP bearer token; same demo-mode refusal as other writes |

> [!warning]
> Demo mode treats settings and model setup as operator-only surfaces that don't exist (`404`), but treats every Vault-scoped route as present — reads are public, writes/control answer `403 demo_read_only`. Don't infer "not implemented" from a `404` on a `/api/v1/vaults/...` path; check the method and current mode first.

The web bearer token is sent as `Authorization: Bearer <token>`, or as an `access_token` query parameter.

## Health & startup

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/health` | Liveness probe; also used by the container's own `--healthcheck`. Returns `200 ok` plaintext. |
| GET | `/ready` | `200 ready` once legacy single-Vault startup (model + first index) is complete, else `503 not ready`. |
| GET | `/api/startup-status` | JSON legacy startup-progress snapshot (model download/index progress). `Cache-Control: no-store`. |

## Model setup

First-run embedding model selection. Not present in demo mode.

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/api/model/accept-gemma` | Accept Gemma terms and start downloading/indexing with it. `202` on success, `409` if a model is already active. |
| POST | `/api/model/decline-gemma` | Decline Gemma and use Nomic Embed Text v1.5 instead. Same status codes. |
| POST | `/api/model/retry` | Retry startup with the already-selected model. `409` if terms are still pending, `500` on an internal persist failure. |

## Settings

Server-wide instance configuration. Not present in demo mode (routes don't exist, rather than existing and refusing).

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/settings` | Every known setting: `key`, current `value` (secrets redacted), `configured`, `source`, `locked`, `class` (`instant`/`reindex`), `kind` (text/switch/secret/number/mode). |
| PATCH | `/api/settings` | Update one or more settings. Body: `{"updates": {"KEY": "value", ...}, "confirm": ["reindex", ...]}`. |
| POST | `/api/settings/web-token/reveal` | Returns the current web bearer token, `{"value": "..."}`. `404` if none is set. |
| POST | `/api/settings/mcp-token/generate` | Returns a freshly generated candidate token, `{"value": "..."}`. Not saved or made live by this call alone. |
| POST | `/api/settings/mcp-token/reveal` | Returns the live MCP bearer token — only if it equals the caller's own web token (seeing it grants no new access). `404` otherwise. |

**`PATCH /api/settings` consequences.** A save that would reindex returns `409` with the machine-readable consequence `reindex` instead of applying. Resend the same request with that value added to `confirm` to proceed. `reindex` is the only consequence: the instance-wide `git_init` and `git_downgrade` consequences were retired along with the routes that reported on them, and per-Vault Git changes carry their own consequences on `/api/v1/vaults/{vault_id}`.

**Setting keys**, with change class (`instant` applies immediately; `reindex` triggers a background reindex) and kind:

| Key | Class | Kind |
| --- | --- | --- |
| `HATCHDOOR_ARCHIVE_PREFIX` | instant | text |
| `HATCHDOOR_EXCLUDE` | reindex | text |
| `HATCHDOOR_EMBED_LAYERS` | reindex | switch |
| `HATCHDOOR_MCP_ENABLED` | instant | switch |
| `HATCHDOOR_MCP_WRITE_ENABLED` | instant | switch |
| `HATCHDOOR_MCP_RATE_LIMITS_ENABLED` | instant | switch |
| `HATCHDOOR_MCP_BEARER_TOKEN` | instant | secret |
| `HATCHDOOR_MCP_ALLOWED_ORIGINS` | instant | text |
| `HATCHDOOR_MAX_ATTACHMENT_BYTES` | instant | number |
| `HATCHDOOR_MCP_MAX_BASE64_BYTES` | instant | number |
| `HATCHDOOR_GIT_SYNC_ENABLED` | instant | mode |
| `HATCHDOOR_GIT_HTTPS_USERNAME` | instant | text |
| `HATCHDOOR_GIT_HTTPS_TOKEN` | instant | secret |
| `HATCHDOOR_GIT_DEBOUNCE_SECONDS` | instant | number |
| `HATCHDOOR_GIT_AUTHOR_NAME` | instant | text |
| `HATCHDOOR_GIT_AUTHOR_EMAIL` | instant | text |
| `HATCHDOOR_GIT_BRANCH` | instant | text |

> [!note]
> The six `HATCHDOOR_GIT_*`/`HATCHDOOR_EXCLUDE` keys and `HATCHDOOR_ARCHIVE_PREFIX` are legacy: they exist to import a pre-registry single-Vault deployment's `.env` once. For a Vault created directly in the registry (via `POST /api/v1/vaults` or `create_vault`), the equivalent per-Vault fields — `source` (branch/mode/poll interval), `https_credentials`, `commit_identity`, `archive_folder`, `exclude_patterns` — are the only place that setting lives; nothing here overrides them.

## MCP transport

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/mcp` | Always `405`; MCP is POST-only. |
| POST | `/mcp` | JSON-RPC 2.0 Streamable HTTP MCP endpoint. See [[MCP tools reference]] for every tool, and [[Connect your agent]] for client setup. |

## Vault collection management

`/api/v1/vaults` and Vault-control routes. Every mutation here uses optimistic concurrency: read `registry_revision` from `GET /api/v1/vaults` first, and pass it back as `expected_registry_revision` — a stale value is rejected rather than silently overwriting a concurrent change.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/v1/vaults` | List every Vault definition plus `registry_revision`/`collection_revision`. In demo mode, only enabled Vaults, each `source` is omitted (never exposes host paths or remote URLs to a public visitor), and the `capabilities` block is rewritten for a visitor (see below). |
| POST | `/api/v1/vaults` | Create a Vault. Body: `CreateVaultRequest` (below). `201` with the new definition. |
| GET | `/api/v1/vaults/events` | Server-Sent Events stream of collection-revision changes (`vault-collection-revision` events carrying `collection_revision`, affected `vault_ids`, and a change `category`). Carries no Note content. |
| POST | `/api/v1/vaults/start-with-no-vaults` | One-shot recovery action, reachable only when a failed legacy `.env` import left the instance pending recovery. Body: `{"confirm": true}`. |
| PATCH | `/api/v1/vaults/{vault_id}` | Replace a Vault's definition wholesale (not a partial patch — resend every field you want to keep). Body: `EditVaultRequest`. |
| DELETE | `/api/v1/vaults/{vault_id}` | Disconnect a Vault from the registry. Deletes no files, checkout, Git history, or credentials outside the registry record itself. |
| POST | `/api/v1/vaults/{vault_id}/enable` | Enable a disabled Vault. Query: `expected_registry_revision`. |
| POST | `/api/v1/vaults/{vault_id}/disable` | Disable a Vault. Same query param. |
| POST | `/api/v1/vaults/{vault_id}/sync` | Request an immediate managed-Git turn, bypassing the poll schedule. `202` with `{"schedule": "queued"\|"coalesced"}`, or `409`/`503` if the Vault has no remote or isn't accepting work. |
| POST | `/api/v1/vaults/{vault_id}/retry` | Same as `sync`, distinct name for a post-failure retry. |
| POST | `/api/v1/vaults/{vault_id}/refresh` | Request the Vault's next Index turn (re-scan local Markdown). `202`/`409`/`503` as above. |

**`CreateVaultRequest` body:**

```json
{
  "expected_registry_revision": 0,
  "name": "Primary",
  "enabled": true,
  "source": { "type": "local", "path": "/data/vault" },
  "exclude_patterns": [],
  "https_credentials": { "username": "x-access-token", "token": "..." },
  "archive_folder": "90-archive/",
  "commit_identity": { "name": "Hatchdoor", "email": "hatchdoor@example.com" }
}
```

`source` is one of three shapes (`type` discriminates):

- **`local`** — `{ "type": "local", "path": "<absolute container path>" }`. Hatchdoor never runs Git for it.
- **`existing_git`** — `{ "type": "existing_git", "repository_path": "...", "repository_url": "...|null", "branch": "...|null", "vault_subdirectory": "...|null", "mode": "local_history"|"pull_only"|"two_way", "poll_interval_secs": 86400 }`. A Git working copy that already exists on disk; Hatchdoor uses it in place and never clones it. `repository_url` is required for `pull_only`/`two_way`, may be null only for `local_history`.
- **`managed_git`** — `{ "type": "managed_git", "repository_url": "...", "branch": "...|null", "vault_subdirectory": "...|null", "mode": "pull_only"|"two_way", "poll_interval_secs": 900 }`. Hatchdoor clones and owns the checkout; no `local_history` mode (there is always a remote to track). `poll_interval_secs` minimum 60, default 86400. There is no maximum, but the scheduler treats anything beyond ten years as ten years, so a very large value is stored as sent and read back unchanged while still producing a `next_attempt_at` you can read.

**Git schedule fields on a listed Vault.** Alongside the status fields, a Vault whose source has a remote to poll (`managed_git`, or `existing_git` in `pull_only`/`two_way`) carries two optional RFC 3339 UTC timestamps: `last_checked_at`, when its last completed Git turn finished, and `next_attempt_at`, when the next scheduled one is due. Both are absent for a source with no remote and in demo mode. They are the supported way to tell a Vault that checked and found nothing from one that has stopped checking — a fetch that brings nothing new leaves no trace in the repository itself.

`last_checked_at` reports the last check whether it succeeded or failed, so read it together with `git` and `git_error` rather than as a successful sync: a Vault that cannot authenticate still reports the time it last tried. It is absent until the first check completes, and it survives a restart. `next_attempt_at` is present for every Vault with a remote — one that has never checked is due immediately, not unscheduled — and reflects the live countdown, so it also accounts for a manual sync, a retry backoff, or an edit to the Vault's `poll_interval_secs`. Shortening the interval moves `next_attempt_at` back to one new interval after `last_checked_at`, which may be immediately; lengthening it leaves the pending attempt where it is. A Vault mid-backoff after a failed check keeps the backoff's own timing, so its `next_attempt_at` does not move until a check succeeds.

**The `capabilities` block on a listed Vault.** Six booleans: `browse`, `search`, `mutate`, `pull`, `push`, `retry`. On a normal instance they are derived from the Vault itself, whether its directory is readable and writable, whether its source has a remote to pull or push, whether a failure is worth retrying. They describe the Vault, not your request. To find out whether this caller may write, read `GET /api/v1/vaults/{vault_id}/write-capabilities`, which answers for the caller and is what the Web UI gates its write controls on.

In demo mode the block answers the visitor's question instead. `mutate`, `pull`, `push` and `retry` are always `false`, because every route behind them refuses with `403 demo_read_only`. `browse` and `search` keep their derived values, since those reads do work on a demo, so a Vault that is unavailable still reports both as `false`. `local_content` is unchanged and still describes the directory: a demo Vault on a writable directory reports `read_write` next to `mutate: false`, the same pairing a pull-only Git Vault has on any instance.

`https_credentials`, `archive_folder`, and `commit_identity` are all optional; omitted, the server-wide defaults apply (`HATCHDOOR_GIT_HTTPS_*`, `HATCHDOOR_ARCHIVE_PREFIX`, `HATCHDOOR_GIT_AUTHOR_*`). Embedded credentials in `repository_url` are rejected — supply them via `https_credentials` instead.

`EditVaultRequest` is the same shape, plus `vault_id` in the path and `confirm_identity_change: bool`. It replaces the definition wholesale: `name` and `source` are required on every edit, and omitting `exclude_patterns`, `archive_folder`, or `commit_identity` clears the stored value rather than preserving it. The one exception is `https_credentials`, which takes an explicit `{"action": "keep"}` / `{"action": "remove"}` / `{"action": "replace", "username": "...", "token": "..."}` so a secret never has to be resent just to survive an edit.

## Vault-scoped content — one Vault

Every route below is a read and stays reachable unauthenticated in demo mode (subject to the collection-wide token gate above). Exact reads always inspect the Vault's live Markdown directory, never the disposable cache, so indexing lag never applies to them.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/v1/vaults/{vault_id}/notes/{slug}` | Full note by slug: content, frontmatter, content hash. `404` if absent. |
| GET | `/api/v1/vaults/{vault_id}/notes/{slug}/links` | Outgoing/incoming links for one note. |
| GET | `/api/v1/vaults/{vault_id}/notes/{slug}/download` | Download the note as a file (`Content-Disposition: attachment`). `413` if the export exceeds the server's download size limit. |
| GET | `/api/v1/vaults/{vault_id}/resolve?target=...` | Resolve one wikilink target to a slug. `{"vault_id": "...", "slug": "...|null"}`. |
| POST | `/api/v1/vaults/{vault_id}/resolve-batch` | Resolve many targets at once. Body: `{"targets": [...], "asset_targets": [...], "note_path": "...|null"}` (targets + asset_targets capped at 200 combined). `note_path` anchors asset resolution to that note's folder. |
| GET | `/api/v1/vaults/{vault_id}/assets/{*path}` | Serve one contained asset or attachment file, with extension allowlisting and traversal containment. |
| GET | `/api/v1/vaults/{vault_id}/write-capabilities` | `{"vault_id", "enabled", "warnings": [...]}` — whether the Web UI's write controls should be shown, and why not if disabled (unwritable path, non-mutable source, or missing web auth on a mutable one). |
| GET | `/api/v1/vaults/{vault_id}/stats/detail` | Rich exact statistics for this one Vault (richer than the collection projection below), including layer diagnostics. |

## Vault-scoped content — one-or-all

`{scope}` is either one canonical Vault ID or the literal `all`. Also reads, also demo-safe.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/v1/vaults/{scope}/tree` | Folder/note tree, grouped per Vault. Always the whole tree — `get_tree`'s `folder`, `max_depth` and `include_notes` narrowing is on the MCP surface only. Each folder carries `note_count`, the notes held directly inside it; the notes themselves carry `title` and `slug` but no `vault_id`, because the tree around them already names its Vault. |
| GET | `/api/v1/vaults/{scope}/recent?limit=` | Recently modified notes, flattened across Vaults. `limit` clamped 1–25, default 5. |
| GET | `/api/v1/vaults/{scope}/stats` | Lean per-Vault statistics projection (for the exact/rich version, see `stats/detail` above). |
| GET | `/api/v1/vaults/{scope}/graph` | Note-link graph, grouped per Vault; edges never cross a Vault boundary. |
| GET | `/api/v1/vaults/{scope}/search?q=&mode=&limit=&per_note_cap=&layers=` | One global ranking flattened across every usable participant. `mode` is `semantic` (default) or `keyword`. `limit` clamped 1–50 (default 10), `per_note_cap` clamped 1–10 (default 2). `layers` is a comma-separated list of layer names, or `all`/`default`; a demo instance always sees the default surface regardless of this parameter. |

## Vault-scoped mutations

Content-changing routes. Web bearer token (if configured); refused with `403 demo_read_only` in demo mode rather than a bare `401`. Every mutation except create takes `expected_content_hash`, read from a prior `GET .../notes/{slug}` — a stale hash is rejected rather than overwriting a concurrent edit.

| Method | Path | Body | Purpose |
| --- | --- | --- | --- |
| POST | `/api/v1/vaults/{vault_id}/notes` | `{"relative_path", "content"}` | Create a note. |
| PUT | `/api/v1/vaults/{vault_id}/notes/{slug}` | `{"content", "expected_content_hash"}` | Replace a note's content. |
| PATCH | `/api/v1/vaults/{vault_id}/notes/{slug}/rename` | `{"new_title", "expected_content_hash"}` | Rename a note (keeps its folder). |
| PATCH | `/api/v1/vaults/{vault_id}/notes/{slug}/move` | `{"target_folder", "expected_content_hash"}` | Move a note to another folder (keeps its title). |
| PATCH | `/api/v1/vaults/{vault_id}/notes/{slug}/move-rename` | `{"target_relative_path", "expected_content_hash"}` | Move and rename in one step. |
| PATCH | `/api/v1/vaults/{vault_id}/notes/{slug}/archive` | `{"expected_content_hash"}` | Move a note under the Vault's archive folder. |
| DELETE | `/api/v1/vaults/{vault_id}/notes/{slug}` | `{"expected_content_hash"}` | Delete a note. |
| POST | `/api/v1/vaults/{vault_id}/attachments` | `multipart/form-data`: `target_relative_path`, `file` | Import an attachment file. Accepts the web token **or** a live MCP bearer token, unlike other mutations — an MCP agent can use it directly without provisioning a separate web token. |

All of the above (except attachment upload) return `VaultWriteOutcomeResponse`: `{"vault_id", "ok", "slug", "relative_path", "content_hash", "quality_warnings": [...], "rewritten_notes", "moved_assets", "trashed_path", "layer"}`. `rewritten_notes` counts other notes whose wikilinks were rewritten to follow a rename/move; `quality_warnings` flags things like a missing heading, not hard failures. Attachment upload returns `VaultAttachmentOutcomeResponse`: `{"vault_id", "ok", "attachment", "rewritten_notes", "trashed_path", "cleanup_warning"}`.

---

Related: [[MCP tools reference]] · [[Connect your agent]] · [[Understand where your data lives]]
