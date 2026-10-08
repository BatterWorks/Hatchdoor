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
| `HOST_STATE_PATH` | `./data/state` | `/data/state` | The Vault registry (`vaults.json`), any stored Git credentials, each Vault's Git poll schedule (`vault-runtime.json`), which Hatchdoor version ran before this one, which agent last connected and, with the usage report on, its install ID and when the last report was sent (`instance.json`) |
| `HOST_MODELS_PATH` | `./models` | `/models` | Downloaded embedding model and the Gemma-terms acceptance record |

See [[Understand where your data lives]] for what to back up.

## Server and storage (environment-only)

Read once at process startup via `AppConfig::from_env`. Docker Compose fixes most of these inside the container already — they matter mainly when running Hatchdoor directly, not through Compose.

| Variable | Default | Purpose |
| --- | --- | --- |
| `VAULT_PATH` | `./vault` | The folder Hatchdoor can see Vaults in: `GET /api/v1/folders` lists its subfolders, so a Vault can be picked from what Hatchdoor can see. Nothing in it becomes a Vault by itself; a fresh install starts with no Vaults, and every Vault is one you added. |
| `HATCHDOOR_CACHE_DB` | `./data/cache/hatchdoor-cache.sqlite3` | Where the disposable SQLite search cache lives. |
| `HOST` | `127.0.0.1` | The interface the process binds to. The standard Compose file fixes this at `0.0.0.0` inside the container so Docker's port publishing can reach it — see [[The security model]] for why that makes a web token mandatory. |
| `PORT` | `42824` | The port the process listens on. |
| `HATCHDOOR_SETTINGS_FILE` | next to `HATCHDOOR_CACHE_DB`, named `settings.json` | Relocates the live-settings file outside the default cache directory. |
| `HATCHDOOR_VAULT_REGISTRY_PATH` | `/data/state/vaults.json` | Relocates the Vault registry. `just dev-start` points this at `.dev/state/vaults.json` automatically for local development. `vault-runtime.json`, which remembers when each Git-backed Vault last checked its remote, is written beside it. That file is bookkeeping, not configuration: deleting it costs one extra check per Vault at the next start and nothing else. `instance.json`, beside it too, remembers the version that ran before the current one and the last agent that connected over MCP, with when. Deleting it makes the next start count as an upgrade from 2.7.0, or as a fresh install when there is no Vault list and no stored settings, and Settings says no agent has connected until one calls again. |

## Web access (environment-only)

| Variable | Default | Purpose |
| --- | --- | --- |
| `HATCHDOOR_WEB_BEARER_TOKEN` | unset | Protects the browser and the HTTP API. Mandatory the moment `HOST` isn't loopback — a Docker first run without it prints a freshly generated token to the logs and refuses to start unauthenticated. See [[The security model]] and [[How to troubleshoot common problems]]. |
| `HATCHDOOR_DEMO_MODE` | `false` | Turns the instance into a public, read-only demo: Settings and model setup disappear entirely, Vault reads become public, and writes are refused. A link to a demo page shared in a chat app shows a preview with the page's title and a short description; set `HATCHDOOR_PUBLIC_URL` for the preview to include a picture. Incompatible with `HATCHDOOR_MCP_ENABLED=true` — Hatchdoor won't start with both set. |

## Live settings

These live in `settings.json`, not `.env` — leave them unset in `.env` to manage them from **Settings** with no restart. A non-empty `.env` value pins that one setting until it's removed from `.env` and the instance restarted. Each entry's **Class** says what a change costs: `instant` applies immediately, `reindex` rebuilds each Vault's search index in the background. A `reindex` save asks you to confirm first, then queues one rebuild per Vault that's turned on: each Vault's `search` status reads `stale` while its own rebuild runs, and searching or browsing it keeps working the whole time — it just answers from the previous index until the new one is ready. Vaults you've turned off aren't touched.

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
| `HATCHDOOR_MCP_RATE_LIMITS_ENABLED` | `true` | instant | Layered resource protection on `/mcp`: at most 120 tool calls per minute per token, eight tool calls running at once (two of them expensive searches), with over-limit requests answered `429 Retry-After`. Each `search_notes` item inside a `batch` counts as one call. Protocol, discovery, and list handling are always exempt. Off removes the caps entirely. |
| `HATCHDOOR_MCP_BEARER_TOKEN` | unset | instant | The MCP password, required even for read-only access — see [[The security model]]. Enabling `HATCHDOOR_MCP_ENABLED` without this set is a startup validation error if pinned in `.env`. |
| `HATCHDOOR_MCP_ALLOWED_ORIGINS` | `http://127.0.0.1,http://localhost` | instant | Origin allow-list checked on every MCP request, as a defense against DNS-rebinding attacks. Mainly relevant to a browser-based MCP client, not a CLI agent. |
| `HATCHDOOR_PUBLIC_URL` | unset | instant | The address people and agents reach this server at, such as `https://notes.example.com`, shown in Settings as **Public address**. Transfer links, which let an agent download or upload a file with no token, are built on it (see [[The security model#Transfer links]]). When set it always wins. Unset, links use the address the agent reached the server on, as a proxy reports it in `Forwarded` or `X-Forwarded-Proto`/`X-Forwarded-Host`, else the request's own `http://` and `Host`. Set it when a proxy sends none of those headers or serves Hatchdoor under a path. On a public demo (`HATCHDOOR_DEMO_MODE=true`) it is also what makes the link preview picture appear: a shared link gets its picture and its own address only when this is set, because Hatchdoor never takes that address from the request (see [[The security model#Demo mode's narrower rule]]). Must be an absolute `http://` or `https://` address without a query; a trailing slash is dropped. |

Above these settings, **Agent access (MCP)** names the last agent that used a tool and how long ago, or says "No agent has connected yet". It is not a setting: Hatchdoor records it on its own and keeps only the name the agent gives itself and the time.

**Uploads**

| Key | Default | Class | Purpose |
| --- | --- | --- | --- |
| `HATCHDOOR_MAX_ATTACHMENT_BYTES` | `10485760` (10 MiB) | instant | Size limit for an attachment uploaded through the Web UI, an agent's upload link, or `POST /api/v1/vaults/{vault_id}/attachments`. |
| `HATCHDOOR_MCP_MAX_BASE64_BYTES` | `5242880` (5 MiB, decoded) | instant | Size limit for what an agent moves through MCP's own allowance: `import_attachment` on the way in, and every download on the way out, whether `get_attachment` returns the bytes as base64 or as a download link. |

**Updates**

| Key | Default | Class | Purpose |
| --- | --- | --- | --- |
| `HATCHDOOR_UPDATE_CHECK_ENABLED` | `false` | instant | Shown in Settings as **Tell me about new releases**. On, Hatchdoor sends one `GET` a day to `https://api.github.com/repos/BatterWorks/Hatchdoor/releases/latest` with the user-agent `Hatchdoor` and no version, so GitHub sees the server's IP address and nothing else about the instance. A newer release shows a banner with links to its release notes and to [[How to upgrade Hatchdoor]]. A failed request is logged and tried again the next day. Turning it on checks within a minute; turning it off stops the check at once, both without a restart. Never runs in demo mode. Hatchdoor never upgrades itself. |

**Usage report**

| Key | Default | Class | Purpose |
| --- | --- | --- | --- |
| `HATCHDOOR_USAGE_REPORT_ENABLED` | `false` | instant | Shown in Settings as **Send a usage report**. This is telemetry. On, Hatchdoor makes a random install ID and sends one small report a day to `https://telemetry-hatchdoor.battercloud.cc`, saying what the install runs on and which parts of Hatchdoor it uses, and nothing about your notes. Settings shows the exact report, on or off. Turning it off deletes the install ID and everything kept for the report; turning it on again makes a new ID. Both take effect without a restart. Never active in demo mode. [[Usage report reference]] lists every field. |

**Legacy — single-Vault keys**

These configured a single-Vault deployment before the Vault registry existed. Releases 2.5.0 to 2.7.x imported them into a Vault once; this version no longer does. Each Vault's own field (`source`, `https_credentials`, `commit_identity`) is the only place the setting lives, and none of these override it. See [[HTTP API reference#Settings|the HTTP API reference's Settings section]].

| Key | Legacy default | Was imported as |
| --- | --- | --- |
| `HATCHDOOR_EXCLUDE` | empty | A Vault's `exclude_patterns` |
| `HATCHDOOR_GIT_SYNC_ENABLED` | `false` | A Vault's Git `mode` |
| `HATCHDOOR_GIT_HTTPS_USERNAME` | `hatchdoor` | A Vault's `https_credentials` username |
| `HATCHDOOR_GIT_HTTPS_TOKEN` | empty | A Vault's `https_credentials` token |
| `HATCHDOOR_GIT_REMOTE` | `origin` | Which remote in the legacy repository to read: its URL became the imported Vault's `source` repository URL |
| `HATCHDOOR_GIT_BRANCH` | `main` | A Vault's `branch` |
| `HATCHDOOR_GIT_AUTHOR_NAME` / `HATCHDOOR_GIT_AUTHOR_EMAIL` | `Hatchdoor` / `hatchdoor@localhost` | A Vault's `commit_identity` — see the note below |
| `HATCHDOOR_GIT_DEBOUNCE_SECONDS` | `30` | No registry equivalent — retired once imported |

`HATCHDOOR_GIT_AUTHOR_NAME` and `HATCHDOOR_GIT_AUTHOR_EMAIL` are the one pair that keeps a live job: they're the name and address Hatchdoor signs commits with for any Vault that hasn't been given a `commit_identity` of its own. Changing either in **Settings** applies to the next commit Hatchdoor makes in such a Vault, with no restart. A Vault that has its own `commit_identity` ignores them entirely.

> [!warning]
> Remove these from `.env`. Hatchdoor logs a warning at startup naming any that are still set, because they do nothing there. An install from 2.4.x or earlier that still stores the retired Git settings (every key above except `HATCHDOOR_EXCLUDE` and the two author keys) in its settings file, with no Vault registry, refuses to start: upgrade it to a 2.5.0 to 2.7.x release first, which imports them, then to this version. See `docs/migrations/legacy-single-vault.md` in the repository.

## Logging (environment-only)

| Variable | Default | Purpose |
| --- | --- | --- |
| `RUST_LOG` | `hatchdoor=info,tower_http=info,axum::rejection=warn` | Standard `tracing`/`EnvFilter` syntax; controls log verbosity per module. |

---

Related: [[HTTP API reference]] · [[Install Hatchdoor with Docker Compose]] · [[How to troubleshoot common problems]]
