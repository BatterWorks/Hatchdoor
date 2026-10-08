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
| `/docs/*`, `/llms.txt` | None, always, demo mode included. A private manual page needs the web token (if configured) |
| `/api/model/*` | Web bearer token (if configured); **absent entirely (`404`) in demo mode** |
| `/api/settings*` | Web bearer token (if configured); **absent entirely (`404`) in demo mode** |
| `/mcp` | Its own MCP bearer token — see [[Connect your agent]] |
| `/api/v1/vaults/...` reads (`GET`) | Web bearer token if configured, **unauthenticated in demo mode** |
| `/api/v1/vaults/...` writes and Vault control | Web bearer token if configured; **refused with `403 demo_read_only` in demo mode** (not `404` — the route exists, it just declines) |
| `/api/v1/vaults/{vault_id}/attachments` (upload) | Web bearer token **or** a live MCP bearer token; same demo-mode refusal as other writes |
| `/api/v1/folders` (`GET` and `POST`) | Web bearer token (if configured); **refused with `403 demo_read_only` in demo mode** |
| `/api/v1/whats-new` | Web bearer token (if configured); **refused with `403 demo_read_only` in demo mode** |
| `/api/v1/vaults/{vault_id}/transfers/{*path}` | No token: the transfer link's own signed query string is the credential, and only while MCP is enabled — see [[#Transfer links]] |

> [!warning]
> Demo mode treats settings and model setup as operator-only surfaces that don't exist (`404`), but treats every Vault-scoped route as present — reads are public, writes/control answer `403 demo_read_only`. Don't infer "not implemented" from a `404` on a `/api/v1/vaults/...` path; check the method and current mode first.

The web bearer token is sent as `Authorization: Bearer <token>`, or as an `access_token` query parameter.

## Health & startup

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/health` | Liveness probe; also used by the container's own `--healthcheck`. Returns `200 ok` plaintext. |
| GET | `/ready` | `200 ready` once the search model is set up and every active Vault's first index has settled, else `503 not ready`. A Vault that failed to index, or has no folder, counts as settled: its problem shows on that Vault (`GET /api/v1/vaults`), not here. An instance with no active Vaults is ready as soon as the model is set up. The exception is a Vault list that needs recovery (see [[Vault lifecycle states#Registry recovery]]): it stays `503` until the file is fixed. Once `200`, it stays `200` through later reindexing. |
| GET | `/api/startup-status` | JSON legacy startup-progress snapshot (model download/index progress). While first-run indexing runs, `percent` and `eta_seconds` cover every active Vault, including ones still waiting their turn, and `percent` never goes down; the `notes_*`, `chunks_*` and `tokens_*` counters describe only the Vault indexing right now. `Cache-Control: no-store`. |

## Manual

The manual built into the running version, as plain Markdown. These routes never read a Vault, the settings or a token. Wikilinks in a page become relative links to other `/docs/<page>.md` addresses. A page's name is its path in the manual, each folder's number dropped and every part lowercased with dashes, so this page is `reference/http-api-reference`.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/docs/<page>.md` | One page, `text/markdown; charset=utf-8`. An unknown page is a plain-text `404`. |
| GET | `/docs/index.md` | The Home page, then a list of every page. |
| GET | `/docs/deploy.md` | [[How to deploy Hatchdoor with an agent]], at a short address to hand an agent. |
| GET | `/docs/search?q=<words>` | JSON `{ "results": [{ "name", "title", "excerpt" }] }`, at most 5 pages, best first. The same word search as the MCP `search_docs` tool. Only the first 200 characters of `q` count. |
| GET | `/llms.txt` | A plain-text list of every page in the [llms.txt](https://llmstxt.org) format, the deploy page first. |

A page marked `private: true` in its frontmatter answers `/docs/<page>.md` only with the web token, `401` without it, and appears in the index and search only for a caller who sends the token. It never appears in `/llms.txt`. On an instance with no web token there is no token to send, so these routes never serve a private page. Signed-in Help and the MCP docs tools still serve them.

## Browser app

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/`, `/stats`, `/graph`, `/settings`, `/v/{vault_id}/n/{slug}` | The browser app, `200`. |
| GET | any other path, outside `/api/`, `/vault-assets/`, `/docs/`, `/llms.txt` and paths starting `/health` | The browser app with a `404` status, when no built file matches. The app shows a not-found page with a way back to your notes. |

Paths under `/api/`, `/vault-assets/` and `/docs/`, `/llms.txt`, and any path starting `/health`, never get the app. An unknown one there is a plain `404`, so a script calling a mistyped API path gets an error, not a page of HTML.

The page is sent before any token is checked. Outside demo mode it is the same file for every address, so it names no note and no Vault.

### Link previews in demo mode

In demo mode (`HATCHDOOR_DEMO_MODE=true`) the page carries a link preview, the title, description and picture a chat app or search engine shows for a shared address. The tags are written into the page's `<head>` when it is sent; the status code does not change.

| Address | `<title>` | `og:title` | Description | `og:type` |
| --- | --- | --- | --- | --- |
| `/v/{vault_id}/n/{slug}`, when the demo serves that note | `<note title> · Hatchdoor` | The note title | The note's `description` property when it is a non-empty string, otherwise its first paragraph of prose. Either is reduced to plain text, with Markdown, wikilink and HTML syntax removed. At most 200 characters, cut on a word with `…`. A note with neither gets the general description | `article` |
| `/`, `/graph`, `/stats`, `/settings` | `Hatchdoor`, `Graph · Hatchdoor`, `Stats · Hatchdoor`, `Settings · Hatchdoor` | `Hatchdoor` | `Self-host your Obsidian vaults: a web UI for you, MCP for your AI agents. No Obsidian, no plugins required.` | `website` |
| Any other address the app answers, and a note address the demo does not serve | `Hatchdoor` | `Hatchdoor` | The same general description | `website` |

Every response carries `<title>`, `<meta name="description">`, `og:title`, `og:description`, `og:type`, `og:site_name` (`Hatchdoor`) and `twitter:card`. When **Public address** (`HATCHDOOR_PUBLIC_URL`) is set it also carries `og:url` (the public address plus the path asked for), `og:image` (the public address plus `/link-preview.png`, one fixed 1200 by 630 picture), `og:image:width`, `og:image:height` and `og:image:alt`, and `twitter:card` is `summary_large_image`. With no public address those are left out and `twitter:card` is `summary`. Hatchdoor never builds an address from the `Host` header or a forwarded header here.

A note address previews the note only when the unauthenticated read of that note would succeed. A missing note, an unknown or disabled Vault, or a note on a demoted layer gets the general wording, so a preview tells a fetcher nothing the demo's API would not. Every value is HTML-escaped. `/link-preview.png` is served without a token on every instance.

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
| GET | `/api/settings` | Every known setting: `key`, current `value` (secrets redacted), `configured`, `source`, `locked`, `class` (`instant`/`reindex`), `kind` (text/switch/secret/number/mode). Beside the list, read-only `last_agent`: the last MCP client that called a tool, as `{"name": "Claude Code", "connected_at": "2026-10-03T09:00:00Z"}`, or `null` when no agent has connected yet. Also read-only `update_check`: `{"enabled": true, "checked_at": "2026-10-03T09:00:00Z", "update_available": {"version": "2.9.0", "release_url": "https://github.com/BatterWorks/Hatchdoor/releases/tag/v2.9.0"}}`, where `checked_at` is the last daily check (`null` before the first) and `update_available` is `null` unless the check is on and its last answer is newer than the running version (see [[How to upgrade Hatchdoor#Hear about new releases]]). Also read-only `usage_report`: `{"enabled": false, "install_id": null, "report": "{ ... }", "last_sent_at": null}`, where `report` is the exact usage report the next send would carry, as indented JSON text, `install_id` is `null` while the report is off, and `last_sent_at` is the time the last report got through, such as `"2026-10-07T09:00:00Z"`, or `null` while the report is off or before the first one (see [[Usage report reference]]). |
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
| `HATCHDOOR_PUBLIC_URL` | instant | text |
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
> The six `HATCHDOOR_GIT_*`/`HATCHDOOR_EXCLUDE` keys and `HATCHDOOR_ARCHIVE_PREFIX` are legacy: they configured a single-Vault deployment before the Vault registry existed, and releases 2.5.0 to 2.7.x imported them once. This version no longer imports them. For a Vault created directly in the registry (via `POST /api/v1/vaults` or `create_vault`), the equivalent per-Vault fields — `source` (branch/mode/poll interval), `https_credentials`, `commit_identity`, `archive_folder`, `exclude_patterns` — are the only place that setting lives; nothing here overrides them.

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
| PATCH | `/api/v1/vaults/{vault_id}` | Replace a Vault's definition wholesale (not a partial patch — resend every field you want to keep). Body: `EditVaultRequest`. |
| DELETE | `/api/v1/vaults/{vault_id}` | Disconnect a Vault from the registry. Deletes no files, checkout, Git history, or credentials outside the registry record itself. |
| POST | `/api/v1/vaults/{vault_id}/enable` | Enable a disabled Vault. Query: `expected_registry_revision`. |
| POST | `/api/v1/vaults/{vault_id}/disable` | Disable a Vault. Same query param. |
| POST | `/api/v1/vaults/{vault_id}/sync` | Request an immediate Git turn, bypassing the poll schedule: a remote sync on a Vault with a remote, a local commit on one that only keeps history. `202` with `{"schedule": "queued"\|"coalesced"}`, or `409`/`503` if the Vault has no Git at all or isn't accepting work. |
| POST | `/api/v1/vaults/{vault_id}/retry` | Same as `sync`, distinct name for a post-failure retry. |
| POST | `/api/v1/vaults/{vault_id}/recovery-branch` | Publish a Two-way Vault's side of a sync conflict to its recovery branch on the remote, so the conflict can be resolved on the Git host. Only while `capabilities.publish_recovery` is true, otherwise `409 capability_unavailable`. `202` with `{"schedule": "queued"\|"coalesced"}`; the outcome appears on the Vault's `recovery_branch` (below). |
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

**The `index_turn` field on a listed Vault.** Where the Vault's indexing stands in the instance-wide queue: `running` while its Index turn runs, `waiting` while it is queued behind another Vault's or paused part-way to let one through, and absent when nothing is queued for it. Vaults index one at a time, and a turn that has embedded for about five minutes pauses if another Vault is waiting ([[How indexing and search work#Vaults take turns indexing]]). It is independent of `search`, which keeps saying what the Vault can answer while it waits or runs, and it is present in demo mode too. A Vault that was already searchable reads `search: "stale"` with `capabilities.search: true` for the whole reindex. `search: "indexing"` means a Vault with nothing to search yet.

**Link style fields on a listed Vault.** `GET /api/v1/vaults` also reports the form the Vault writes new links in, so a client adding links can match it: `link_style` is `wikilink` (`[[Note]]`, `![[file.png]]`) or `markdown` (`[Note](Note.md)`, `![](file.png)`), and `link_path_form` is `relative`, `absolute` or `shortest`, the path form a Markdown link takes. `link_path_form` is present whenever `link_style` is, whichever style that is. Both are read from the Vault on every request, from Obsidian's `.obsidian/app.json` when it exists and otherwise from the links the Vault already has; [[Supported Markdown reference#Which link style Hatchdoor writes]] gives the rule. Both are absent for a Vault Hatchdoor cannot read, which is still listed, in demo mode, and in the `vault` a create or edit returns. Hatchdoor never rewrites a client's links into this style. Without an Obsidian settings file, working out the style means checking every note in the Vault and reading the ones that changed since the last listing, so this listing costs more on a large Vault that has none, most of all the first time after a restart.

**The `capabilities` block on a listed Vault.** Nine booleans: `browse`, `search`, `mutate`, `pull`, `push`, `retry`, `commit`, `sync`, `publish_recovery`. On a normal instance they are derived from the Vault itself, whether its directory is readable and writable, whether its source has a remote to pull or push, whether a failure is worth retrying. They describe the Vault, not your request. `commit` (this Vault keeps Git history of its own: `local_history` or `two_way`) and `sync` (this Vault has a remote: `pull_only` or `two_way`) come from the Vault's definition rather than its current status, unlike `pull` and `push`, so they keep their answer while the Vault is failing. Between them they say whether `POST .../sync` will do anything and which of the two things it will do. `publish_recovery` is true for a Two-way Vault whose `git_error` is `managed_git_conflict`, and is the one state `POST .../recovery-branch` accepts. To find out whether this caller may write, read `GET /api/v1/vaults/{vault_id}/write-capabilities`, which answers for the caller and is what the Web UI gates its write controls on.

**The `recovery_branch` field on a listed Vault.** Present once a recovery branch has been requested for the Vault's current conflict, and cleared by the first sync that goes through. `branch` is `hatchdoor-recovery/<configured branch>/<vault_id>`; `published_commit`, `conflicting_commit` (the remote side of the conflict) and `published_at` describe the last publish; `error` says why the latest request published nothing, with the earlier publication's fields left in place. Its codes are `managed_git_recovery_diverged` (someone added commits to the branch, so Hatchdoor left it alone), `managed_git_recovery_push_rejected` (the remote refused the branch, with its reason), `managed_git_authentication_failed`, `managed_git_remote_unreachable`, and `capability_unavailable` (the conflict had already cleared). Hatchdoor only ever fast-forwards this branch, never touches the configured branch while publishing, and never deletes a branch. The field is held in memory, so a restart clears it until the next publish. Absent in demo mode.

In demo mode the block answers the visitor's question instead. `mutate`, `pull`, `push`, `retry`, `commit`, `sync` and `publish_recovery` are always `false`, because every route behind them refuses with `403 demo_read_only`. `browse` and `search` keep their derived values, since those reads do work on a demo, so a Vault that is unavailable still reports both as `false`. `local_content` is unchanged and still describes the directory: a demo Vault on a writable directory reports `read_write` next to `mutate: false`, the same pairing a pull-only Git Vault has on any instance.

`https_credentials`, `archive_folder`, and `commit_identity` are all optional; omitted, the server-wide defaults apply (`HATCHDOOR_GIT_HTTPS_*`, `HATCHDOOR_ARCHIVE_PREFIX`, `HATCHDOOR_GIT_AUTHOR_*`). Embedded credentials in `repository_url` are rejected — supply them via `https_credentials` instead.

`EditVaultRequest` is the same shape, plus `vault_id` in the path and `confirm_identity_change: bool`. It replaces the definition wholesale: `name` and `source` are required on every edit, and omitting `exclude_patterns`, `archive_folder`, or `commit_identity` clears the stored value rather than preserving it. The one exception is `https_credentials`, which takes an explicit `{"action": "keep"}` / `{"action": "remove"}` / `{"action": "replace", "username": "...", "token": "..."}` so a secret never has to be resent just to survive an edit.

## Folders under the Vault mount

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/v1/folders?path=<relative>` | List the folders directly inside one folder under the Vault mount, so a Vault can be picked instead of typed. Without `path` it lists the mount itself. |
| POST | `/api/v1/folders` | Make one new, empty folder inside a folder under the Vault mount. See [[#Make a new folder]]. |

The mount is the folder `VAULT_PATH` names: `/data/vault` in the stock Compose file, `./vault` when unset. `path` is relative to it, with `/` between folder names.

```json
{
  "root": "/data/vault",
  "root_found": true,
  "path": "",
  "markdown": { "count": 214, "at_least": false },
  "vault": null,
  "folders": [
    {
      "name": "Work",
      "path": "Work",
      "markdown": { "count": 180, "at_least": false },
      "vault": { "vault_id": "4f1c...", "name": "Work" },
      "has_subfolders": true
    }
  ],
  "skipped_invalid_names": 0
}
```

- `root` is the mount as an absolute path inside the container, as `VAULT_PATH` names it, made absolute but not resolved through symlinks. Join it and a folder's `path` with `/` to get the `path` that `POST /api/v1/vaults` takes for a `local` source.
- `markdown` counts the `.md` notes in a folder and every folder below it. Each listed subfolder's count stops at 10,000 notes, and a listing stops counting after about two seconds, so a folder late in the list can come back as `at least 0`. When either limit cut a count short, `at_least` is `true` and the real number is higher. The top-level `markdown` is the listed folder's own notes plus its subfolders' counts, so it can pass 10,000, and a mount that is itself one notes folder shows its notes there.
- `vault` names the registered Vault whose folder is exactly this one, or is `null`.
- `has_subfolders` says whether opening the folder would list anything. It is `true` when counting stopped before Hatchdoor could tell.
- `skipped_invalid_names` counts folders left out because their names are not valid UTF-8.
- When the mount folder does not exist, the answer is `200` with `root_found: false` and no folders.

The listing never returns file names or file contents and changes nothing on disk. It never follows a symlink, so a symlinked folder is left out and a symlink cannot lead outside the mount. Hidden folders (`.git`, `.obsidian`, `.trash` and other names starting with a dot) are left out, as is any folder that holds Hatchdoor's own state, which the registry would refuse as a Vault anyway.

| Status | `code` | When |
| --- | --- | --- |
| `400` | `folder_outside_root` | `path` starts with `/` or contains `..`. |
| `404` | `folder_not_found` | Nothing the listing would show is at `path`: it is missing, a file, a symlink, hidden, or holds Hatchdoor's state. |
| `422` | `folder_unreadable` | The folder exists but Hatchdoor has no permission to read it. |

### Make a new folder

`POST /api/v1/folders` makes one new, empty folder, so a Vault can be started without a shell. It is the only write Hatchdoor makes outside a Vault and its own state.

```json
{ "parent": "Work", "name": "Journal" }
```

- `parent` is the folder to make it in, relative to the mount like `path` above. Leave it out, or send `""`, for the mount itself. It must be a folder the listing shows.
- `name` is the new folder's name: one path segment, kept as sent. It cannot be empty, be longer than 255 bytes, start with a dot, start or end with a space, or contain `/`, `\` or a control character.
- Unknown fields are refused, so there is no way to ask for a chain of folders.

The answer is `201` with the new folder in the shape the listing uses:

```json
{
  "name": "Journal",
  "path": "Work/Journal",
  "markdown": { "count": 0, "at_least": false },
  "vault": null,
  "has_subfolders": false
}
```

Join the listing's `root` and this `path` to get the `path` for `POST /api/v1/vaults`. Creating the Vault is a separate request and is unchanged: it still refuses a folder that does not exist.

Exactly one directory is made and nothing is written into it. The parent is reached without following a symlink. A folder is never made inside a registered Vault, and an existing folder is never reused. Hatchdoor does not delete or rename folders, so a folder made and then not used stays on disk. This route is not available over MCP.

| Status | `code` | When |
| --- | --- | --- |
| `400` | `folder_name_invalid` | `name` breaks a rule above, or names a folder that holds Hatchdoor's own state. |
| `400` | `folder_outside_root` | `parent` starts with `/` or contains `..`, or resolves outside the mount. |
| `404` | `folder_mount_not_found` | The mount folder does not exist. It is not created. |
| `404` | `folder_parent_not_found` | Nothing the listing would show is at `parent`: it is missing, a file, a symlink, hidden, or holds Hatchdoor's state. No parent folder is created. |
| `409` | `folder_name_taken` | A folder, file or link with that name is already there. |
| `409` | `folder_inside_vault` | `parent` is a registered Vault or sits inside one, or the new folder would be a registered Vault's own missing folder. |
| `422` | `folder_not_writable` | Hatchdoor could not write to `parent`: it is read-only, or the process lacks permission. |
| `422` | `invalid_request_body` | The body is not the JSON object above. |
| `503` | `folder_vaults_unknown` | Hatchdoor cannot read its Vault registry, so it cannot tell whether `parent` belongs to a Vault. Nothing is made. |

## What's new

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/v1/whats-new` | Which version runs, which ran before it, and the release highlights in between, from [[What's new]]. `Cache-Control: no-store`. |

```json
{
  "version": "2.8.0",
  "previous_version": "2.7.0",
  "fresh_install": false,
  "releases": [
    {
      "version": "2.8.0",
      "date": "2026-10-20",
      "highlights": [
        {
          "text": "Installs on 2.4.x or earlier must upgrade to a 2.5.0 to 2.7.x release first.",
          "action_needed": true,
          "link": {
            "label": "Install Hatchdoor with Docker Compose",
            "page": "get-started/install-hatchdoor-with-docker-compose",
            "heading": null
          }
        }
      ]
    }
  ]
}
```

- `version` is the running version as the binary reports it. A development build that was built with its commit adds ` (dev <commit>)`; a release never does.
- `previous_version` is the version this instance ran before the current one, or `null` when it has run no other. An install that was already set up before it started keeping this record counts as coming from `2.7.0`, unless it runs 2.7.0 itself.
- `fresh_install` is `true` when the instance was first started with no Vault list and no stored settings and still runs the version it started on. It turns `false` at its first upgrade.
- `releases` lists the releases after `previous_version` up to the running version, newest first. It is empty on a fresh install that has not been upgraded yet. Each highlight's `text` is Markdown. `action_needed` lines come first, and `link`, when present, names the manual page (`read_docs` and `/docs/<page>.md` take the same name) and heading that explain it.

Hatchdoor keeps the version record in `instance.json`, beside the Vault list in its state folder. It updates the record once at startup, and only when the version changes. Deleting the file makes the next start count as an upgrade from `2.7.0` when a Vault list or settings exist, and as a fresh install otherwise.

## Vault-scoped content — one Vault

Every route below is a read and stays reachable unauthenticated in demo mode (subject to the collection-wide token gate above). Exact reads always inspect the Vault's live Markdown directory, never the disposable cache, so indexing lag never applies to them.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/v1/vaults/{vault_id}/notes/{slug}` | Full note by slug: content, content hash, `metadata` (the frontmatter's `tags`, `aliases` and remaining keys under `properties`, or `null` when the frontmatter does not parse, in which case the note is still returned), and `saved_queries`, the note's saved queries by name (`null` for an unnamed one), never evaluated. `404` if absent. |
| GET | `/api/v1/vaults/{vault_id}/notes/{slug}/links` | Outgoing/incoming links for one note. |
| GET | `/api/v1/vaults/{vault_id}/notes/{slug}/download` | Download the note as a file (`Content-Disposition: attachment`). A note that embeds local images or PDFs comes back as a `.zip` holding the Markdown and those files, with the links rewritten to point inside it; otherwise it is plain Markdown. In demo mode the zip carries only assets the asset route below would serve, so an image under a demoted layer or an excluded folder is left out and its link kept as written, the same as for a file that does not exist. `413` if the export exceeds the server's download size limit. |
| GET | `/api/v1/vaults/{vault_id}/notes/{slug}/saved-queries` | Every saved query in the note (each fenced `base` block), evaluated against that note's own Vault now. Wrapped in the same `{scope, collection_revision, partial, participants, data}` envelope as the collection reads, because the definitions come from the live Markdown but the rows come from the Vault's index; `participants` says whether that index is current. `data` is `{"vault_id", "slug", "queries": [...], "marker_problems": [...]}`, queries in document order. Each query carries `source` (the block's text), `name` when a usable `<!-- hatchdoor-query: name -->` marker precedes it (a name two blocks share appears on both and is listed under `duplicate_name` below, which means it addresses neither), and a `status`: `populated` with `columns` (`{id, label}`), `rows` (`{vault_id, title, slug, relative_path, cells}`, at least one, one cell per column, `null` where the note lacks the property) and `truncated` (`{reason: "definition_limit" or "ceiling", shown}`) when rows were held back; `empty` with `columns` when the definition was read and evaluated and no note qualified; `refused` with a `construct` (the part it could not use, such as `daysUntil()` or `YAML`) and a `message` naming it; or `stopped` with a `message` when evaluating would pass Hatchdoor's limits (the note's saved queries together scan at most 20,000 notes, and a note holds at most 10). `populated` and `empty` may carry `ignored`, a list of `{instruction, message}` for presentation instructions that were not carried out (`groupBy`, `summaries`, a view type other than `table`); the rows are complete without them. Each marker problem has a `problem` and a `message`: `orphaned` (`name`, `line`) for a marker with no `base` block after it, `unusable_name` (`name`, `query`) for a name that is not a slug, whose block is then unnamed, and `duplicate_name` (`name`, `queries`) when several blocks claim one name, which then names none of them. `query` and `queries` are positions in `queries`. No marker problem changes a row. Nothing computed here is written to the note or indexed. `404` if the note is absent. |
| GET | `/api/v1/vaults/{vault_id}/resolve?target=...` | Resolve one wikilink target to a slug. `{"vault_id": "...", "slug": "...|null"}`. |
| POST | `/api/v1/vaults/{vault_id}/resolve-batch` | Resolve many targets at once. Body: `{"targets": [...], "asset_targets": [...], "note_link_targets": [...], "note_path": "...|null"}` (the three lists capped at 200 combined). `targets` are wikilink targets, resolved by title. `note_link_targets` are Markdown note-link destinations as written, such as `../20-projects/Beacon%20Launch.md`, resolved by path and answered in `note_link_results`, shaped like `results`. `note_path` anchors asset and Markdown note-link resolution to that note's folder. |
| GET | `/api/v1/vaults/{vault_id}/assets/{*path}` | Serve one contained asset or attachment file, with extension allowlisting and traversal containment. |
| GET | `/api/v1/vaults/{vault_id}/write-capabilities` | `{"vault_id", "enabled", "atomic_compare_and_swap", "warnings": [...]}` — whether the Web UI's write controls should be shown, and why not if disabled (unwritable path, non-mutable source, or missing web auth on a mutable one). `atomic_compare_and_swap` is `true` when this Vault's filesystem can commit a save as one atomic swap, `false` when saves work through the weaker check-then-rename path, and `null` when the filesystem could not be asked. It answers for the filesystem, not for whether the Vault is writable, so a read-only Vault is never `false` on that account. When it is `false` and `enabled` is `true`, `warnings` gains one sentence saying a change made in another editor mid-save is overwritten rather than refused. |
| GET | `/api/v1/vaults/{vault_id}/stats/detail` | Rich exact statistics for this one Vault (richer than the collection projection below), including layer diagnostics. `activity_by_month` is six calendar months, oldest first, each with a `created_count` of the notes whose created date falls in it: a `created` property, else the commit that first added the note in a Git-backed Vault, else the file's modification time. `created_date_status` is `complete`, `estimated` (some Git dates were unavailable) or `reading` (history is still being read; ask again shortly). |

## Vault-scoped content — one-or-all

`{scope}` is either one canonical Vault ID or the literal `all`. Also reads, also demo-safe.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/v1/vaults/{scope}/tree` | Folder/note tree, grouped per Vault. Always the whole tree — `get_tree`'s `folder`, `max_depth` and `include_notes` narrowing is on the MCP surface only. Each folder carries `note_count`, the notes held directly inside it; the notes themselves carry `title` and `slug` but no `vault_id`, because the tree around them already names its Vault. |
| GET | `/api/v1/vaults/{scope}/recent?limit=` | Recently modified notes, flattened across Vaults. `limit` clamped 1–25, default 5. |
| GET | `/api/v1/vaults/{scope}/stats` | Lean per-Vault statistics projection (for the exact/rich version, see `stats/detail` above). |
| GET | `/api/v1/vaults/{scope}/graph` | Note-link graph, grouped per Vault; edges never cross a Vault boundary. |
| GET | `/api/v1/vaults/{scope}/search?q=&mode=&limit=&per_note_cap=&layers=` | One global ranking flattened across every usable participant. `mode` is `semantic` (default) or `keyword`. `limit` clamped 1–50 (default 10), `per_note_cap` clamped 1–10 (default 2). `layers` is a comma-separated list of layer names, or `all`/`default`; a demo instance always sees the default surface regardless of this parameter. Each result's `score` runs 0 to 1: in semantic mode it is the cosine similarity between the query and the chunk, so it can be compared across searches; in keyword mode it is relative to the best hit of that search, which scores 1. A query that is a single `#tag` (letters, digits, `-`, `_` and `/` after the `#`) runs as a tag match whatever `mode` was requested: every hit scores 1 and the response reports `"mode": "tag"`. Every other query reports the mode it asked for. `tag` is never accepted as a requested mode. |

## Vault-scoped mutations

Content-changing routes. Web bearer token (if configured); refused with `403 demo_read_only` in demo mode rather than a bare `401`. Every mutation except create takes `expected_content_hash`, read from a prior `GET .../notes/{slug}` — a stale hash is rejected rather than overwriting a concurrent edit.

That rejection is unconditional. What depends on the filesystem is the narrower race: on a Vault reporting `atomic_compare_and_swap: false`, a change landing between the hash check and the replacement is overwritten instead of reported, because the save is two steps there rather than one. Read `GET .../write-capabilities` to know which kind of Vault you are writing to.

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

All of the above (except attachment upload) return `VaultWriteOutcomeResponse`: `{"vault_id", "ok", "slug", "relative_path", "content_hash", "quality_warnings": [...], "rewritten_notes", "moved_assets", "trashed_path", "layer"}`. `rewritten_notes` counts other notes whose links, wikilinks or Markdown note links, were rewritten to follow a rename, move or delete; `quality_warnings` flags things like a missing heading, not hard failures. Attachment upload returns `VaultAttachmentOutcomeResponse`: `{"vault_id", "ok", "attachment", "rewritten_notes", "trashed_path", "cleanup_warning"}`. A rename, move, archive or delete that would have to rewrite a link in a note that is not valid UTF-8 text answers `409` with the code `link_rewrite_unsupported`, writes nothing, and names every such note in its `message`.

## Transfer links

Transfer links are minted over MCP, never by these routes: `get_attachment` returns a download link and `create_upload_link` an upload link (see [[MCP tools reference]]). Both address one path on this route and carry `expires`, a `signature`, for uploads `overwrite` and `nonce`, and for a link that replaces a note `expected_content_hash`, in the query string. Send them exactly as given, with no `Authorization` header.

| Method | Path | Body | Purpose |
| --- | --- | --- | --- |
| GET | `/api/v1/vaults/{vault_id}/transfers/{*path}` | — | Download the attachment a download link names. Answers like the asset route, held to `HATCHDOOR_MCP_MAX_BASE64_BYTES` and the MCP rate quota (`429` with `Retry-After`). Usable any number of times until it expires. |
| POST | `/api/v1/vaults/{vault_id}/transfers/{*path}` | `multipart/form-data`: `file`, and optionally `target_relative_path`, which must match the link | Upload one file to the path an upload link names, under `HATCHDOOR_MAX_ATTACHMENT_BYTES`. Returns `VaultAttachmentOutcomeResponse`, or `VaultWriteOutcomeResponse` when the path ends in `.md` and the file becomes a note. Usable once. |

Refusals are `403` with a stable `code`: `transfer_link_invalid` (any other path, Vault, or target; a tampered link; a link from before a restart or an MCP password change), `transfer_link_expired` (five minutes after minting), `transfer_link_spent` (an upload link's second use), `mcp_disabled` (MCP is off), and `mcp_write_disabled` (an upload while MCP writes are off). An upload whose target appeared after the link was minted, when replacing was not allowed, is `409 write_conflict`, and so is a note upload whose note no longer has the hash the link was minted with. A note upload that is not UTF-8 or contains a NUL byte is `400 invalid_write_input`.

---

Related: [[MCP tools reference]] · [[Connect your agent]] · [[Understand where your data lives]]
