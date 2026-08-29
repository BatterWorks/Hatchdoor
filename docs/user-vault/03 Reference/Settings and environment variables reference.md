---
tags: [type/reference, topic/configuration]
---

# Settings and environment variables reference

Every environment variable and live setting Hatchdoor reads, what each one does, and — the distinction that matters most — whether it's deploy-time-only (set in `.env`, needs a restart to change) or a live setting (changeable from **Settings** or `PATCH /api/settings`, no restart). [[HTTP API reference]] documents the `/api/settings` wire format itself; this page explains what each key is for.

## Docker Compose host mounts

Read by Compose on the host, not by the Hatchdoor binary — these decide what gets mounted where, per `docker-compose.yml`.

| Variable | Default | Mounted as | Contents |
| --- | --- | --- | --- |
| `HOST_VAULT_PATH` | `./vault` | `/data/vault` | Markdown notes and attachments |
| `HOST_CACHE_PATH` | `./data/cache` | `/data/cache` | SQLite search cache and `settings.json` |
| `HOST_STATE_PATH` | `./data/state` | `/data/state` | The Vault registry (`vaults.json`) and any stored Git credentials |
| `HOST_MODELS_PATH` | `./models` | `/models` | Downloaded embedding model and the Gemma-terms acceptance record |

See [[Understand where your data lives]] for what to back up.

## Server and storage (environment-only)

Read once at process startup via `AppConfig::from_env`. Docker Compose fixes most of these inside the container already — they matter mainly when running Hatchdoor directly, not through Compose.

| Variable | Default | Purpose |
| --- | --- | --- |
| `VAULT_PATH` | `./vault` | The folder Hatchdoor recognizes as its first local Vault on a first start. Legacy: read once to seed the Vault registry, then ignored — later Vaults are managed through the registry, not this variable. |
| `HATCHDOOR_CACHE_DB` | `./data/cache/hatchdoor-cache.sqlite3` | Where the disposable SQLite search cache lives. |
| `HOST` | `127.0.0.1` | The interface the process binds to. The standard Compose file fixes this at `0.0.0.0` inside the container so Docker's port publishing can reach it — see [[The security model]] for why that makes a web token mandatory. |
| `PORT` | `42824` | The port the process listens on. |
| `HATCHDOOR_SETTINGS_FILE` | next to `HATCHDOOR_CACHE_DB`, named `settings.json` | Relocates the live-settings file outside the default cache directory. |
| `HATCHDOOR_VAULT_REGISTRY_PATH` | `/data/state/vaults.json` | Relocates the Vault registry. `just dev-start` points this at `.dev/state/vaults.json` automatically for local development. |

## Web access (environment-only)

| Variable | Default | Purpose |
| --- | --- | --- |
| `HATCHDOOR_WEB_BEARER_TOKEN` | unset | Protects the browser and the HTTP API. Mandatory the moment `HOST` isn't loopback — a Docker first run without it prints a freshly generated token to the logs and refuses to start unauthenticated. See [[The security model]] and [[How to troubleshoot common problems]]. |
| `HATCHDOOR_DEMO_MODE` | `false` | Turns the instance into a public, read-only demo: Settings and model setup disappear entirely, Vault reads become public, and writes are refused. Incompatible with `HATCHDOOR_MCP_ENABLED=true` — Hatchdoor won't start with both set. |

## Live settings

These live in `settings.json`, not `.env` — leave them unset in `.env` to manage them from **Settings** with no restart. A non-empty `.env` value pins that one setting until it's removed from `.env` and the instance restarted. Each entry's **Class** says what a change costs: `instant` applies immediately, `reindex` rebuilds each Vault's search index in the background. A `reindex` save asks you to confirm first, then queues one rebuild per Vault that's turned on: each Vault shows as **indexing** while its own rebuild runs, and searching or browsing it keeps working the whole time — it just answers from the previous index until the new one is ready. Vaults you've turned off aren't touched.

**Note handling**

| Key | Default | Class | Purpose |
| --- | --- | --- | --- |
| `HATCHDOOR_ARCHIVE_PREFIX` | `90-archive/` | instant | The instance-wide default archive folder `archive_note` moves a note into, when a Vault doesn't override it with its own `archive_folder`. |
| `HATCHDOOR_EMBED_LAYERS` | `true` | reindex | Whether notes on a demoted [[The layer system\|layer]] get semantic embeddings at all, not just structural indexing. Off trades semantic search over demoted content for a smaller, faster index. Changing it rebuilds every Vault that's turned on, one at a time. |

**Agent access (MCP)**

| Key | Default | Class | Purpose |
| --- | --- | --- | --- |
| `HATCHDOOR_MCP_ENABLED` | `false` | instant | Turns `/mcp` on or off. Off, the endpoint returns `404` rather than refusing — it isn't advertised as existing. |
| `HATCHDOOR_MCP_WRITE_ENABLED` | `false` | instant | Separately gates every content- and Vault-mutating MCP tool. An agent can read with MCP enabled and this still off. Toggling it changes which tools Hatchdoor advertises, so connected agents are told to refresh their tool list — no reconnection needed. |
| `HATCHDOOR_MCP_RATE_LIMITS_ENABLED` | `true` | instant | Layered resource protection on `/mcp`: at most 120 tool calls per minute per token, eight tool calls running at once (two of them expensive searches), with over-limit requests answered `429 Retry-After`. Protocol, discovery, and list handling are always exempt. Off removes the caps entirely. |
| `HATCHDOOR_MCP_BEARER_TOKEN` | unset | instant | The MCP password, required even for read-only access — see [[The security model]]. Enabling `HATCHDOOR_MCP_ENABLED` without this set is a startup validation error if pinned in `.env`. |
| `HATCHDOOR_MCP_ALLOWED_ORIGINS` | `http://127.0.0.1,http://localhost` | instant | Origin allow-list checked on every MCP request, as a defense against DNS-rebinding attacks. Mainly relevant to a browser-based MCP client, not a CLI agent. |

**Uploads**

| Key | Default | Class | Purpose |
| --- | --- | --- | --- |
| `HATCHDOOR_MAX_ATTACHMENT_BYTES` | `10485760` (10 MiB) | instant | Size limit for an attachment uploaded through the Web UI or `POST /api/v1/vaults/{vault_id}/attachments`. |
| `HATCHDOOR_MCP_MAX_BASE64_BYTES` | `5242880` (5 MiB, decoded) | instant | Size limit for MCP's base64 fallback path, in both directions — `import_attachment` on the way in and `get_attachment` with `encoding: "base64"` on the way out — for clients that can't make an out-of-band HTTP request. |

**Legacy — single-Vault import only**

The following exist solely to import a pre-registry, single-Vault `.env` deployment once; see the fuller explanation already on [[HTTP API reference#Settings|the HTTP API reference's Settings section]]. For any Vault created directly in the registry, the equivalent per-Vault field (`source`, `https_credentials`, `commit_identity`) is the only place the setting lives — none of these override it.

| Key | Legacy default | Stands in for |
| --- | --- | --- |
| `HATCHDOOR_EXCLUDE` | empty | A Vault's `exclude_patterns` |
| `HATCHDOOR_GIT_SYNC_ENABLED` | `false` | A Vault's Git `mode` |
| `HATCHDOOR_GIT_HTTPS_USERNAME` | `hatchdoor` | A Vault's `https_credentials` username |
| `HATCHDOOR_GIT_HTTPS_TOKEN` | empty | A Vault's `https_credentials` token |
| `HATCHDOOR_GIT_REMOTE` | `origin` | Which remote in the legacy repository to read: its URL becomes the imported Vault's `source` repository URL |
| `HATCHDOOR_GIT_BRANCH` | `main` | A Vault's `branch` |
| `HATCHDOOR_GIT_AUTHOR_NAME` / `HATCHDOOR_GIT_AUTHOR_EMAIL` | `Hatchdoor` / `hatchdoor@localhost` | A Vault's `commit_identity` — see the note below |
| `HATCHDOOR_GIT_DEBOUNCE_SECONDS` | `30` | No registry equivalent — retired once imported |

`HATCHDOOR_GIT_AUTHOR_NAME` and `HATCHDOOR_GIT_AUTHOR_EMAIL` are the one pair that keeps a live job after the import: they're the name and address Hatchdoor signs commits with for any Vault that hasn't been given a `commit_identity` of its own. Changing either in **Settings** applies to the next commit Hatchdoor makes in such a Vault, with no restart. A Vault that has its own `commit_identity` ignores them entirely.

> [!warning]
> Leave these exactly as they were in an upgraded single-Vault `.env` for one start so Hatchdoor can import them, then delete them — it refuses to start again while they're still set, since they aren't valid configuration for a registry Vault.

## Logging (environment-only)

| Variable | Default | Purpose |
| --- | --- | --- |
| `RUST_LOG` | `hatchdoor=info,tower_http=info,axum::rejection=warn` | Standard `tracing`/`EnvFilter` syntax; controls log verbosity per module. |

---

Related: [[HTTP API reference]] · [[Install Hatchdoor with Docker Compose]] · [[How to troubleshoot common problems]]
