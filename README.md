# Hatchdoor

Rust backend + React/Vite frontend for browsing an Obsidian vault, with optional write-capable MCP access.

## Features

- Explorer tree from vault folders/files
- Open notes via route (`/n/:slug`)
- Obsidian wikilinks (`[[Note]]`, `[[Note|Alias]]`) resolved through backend API
- Persistent SQLite cache/read model for fast API, UI, search, links, and MCP access
- SQLite FTS5 search over note title, relative path, and Markdown content
- Cached headings, tags, wikilinks, and backlinks
- Markdown rendering with:
  - GFM (tables, task lists, strikethrough)
  - Math (`remark-math` + KaTeX)
  - Mermaid fenced code blocks (lazy-loaded for mobile performance)
- Unresolved links rendered with explicit broken-link styling
- PWA build output (manifest + service worker via Vite PWA)
- Live vault updates without server restarts via periodic backend reindex
- Optional embedded MCP endpoint for OpenClaw (`/mcp`)

## Configuration

Copy `.env.example` to `.env`:

- `HOST_VAULT_PATH=./vault`
- `HOST_CACHE_PATH=./data/cache`
- `HOST=0.0.0.0`
- `PORT=42824`
- `VAULT_REFRESH_SECONDS=2`
- `HATCHDOOR_CACHE_DB=/data/cache/hatchdoor-cache.sqlite3`
- `HATCHDOOR_ARCHIVE_PREFIX=90-archive/`
- `HATCHDOOR_MCP_ENABLED=false`
- `HATCHDOOR_MCP_BEARER_TOKEN=`
- `HATCHDOOR_MCP_ALLOWED_ORIGINS=http://127.0.0.1,http://localhost`
- `RUST_LOG=hatchdoor=info,tower_http=info,axum::rejection=warn`

`VAULT_REFRESH_SECONDS` is kept for compatibility with forced refresh internals. Normal cache updates are driven by the recursive vault watcher.
`HATCHDOOR_CACHE_DB` points to Hatchdoor's generated SQLite cache. Keep it outside the Markdown vault.
`HATCHDOOR_ARCHIVE_PREFIX` controls which vault folder is treated as archived by wikilink resolution and archive-note writes.
`RUST_LOG` controls structured backend log verbosity.

## SQLite cache/read model

Markdown remains the source of truth. SQLite is a persistent, disposable cache/read model.

Hatchdoor stores this in SQLite:

- note metadata and full Markdown content
- normalised title/path lookup data
- file modification time, size, and stable content hash
- explorer tree data
- FTS5 search index
- resolved wikilinks and backlinks
- headings
- tags

Refresh behaviour:

- A recursive vault watcher refreshes the cache after Markdown or asset file changes.
- Browser clients subscribe to `/api/vault-events` and reload visible vault data when a refreshed revision is broadcast.
- Changed/new/deleted note rows are updated incrementally using file metadata plus stable content hash.
- Link relationships are rebuilt from the current vault index on refresh so backlinks stay correct after note renames, additions, or deletions.
- `/api/refresh` and MCP `refresh_index` force an immediate refresh.

If the SQLite database is deleted, Hatchdoor rebuilds it from the Markdown vault at startup. If the database cannot be opened, has no schema metadata, or has an unsupported schema version, Hatchdoor fails startup rather than silently falling back.

Manual cache rebuild:

```bash
rm ./data/cache/hatchdoor-cache.sqlite3
docker compose restart hatchdoor
```

The embedded MCP endpoint is disabled by default. Enable it only when OpenClaw should be allowed to query Hatchdoor:

```env
HATCHDOOR_MCP_ENABLED=true
```

Write-capable MCP tools are a separate opt-in and require bearer auth:

```env
HATCHDOOR_MCP_WRITE_ENABLED=true
HATCHDOOR_MCP_BEARER_TOKEN=change-me
```

Attachment import uses a staging folder outside the vault:

```env
HOST_ATTACHMENT_STAGING_PATH=/home/battermanz/coding/hatchdoor/data/attachments-inbox
HATCHDOOR_MCP_ATTACHMENT_STAGING_PATH=/data/attachments-inbox
HATCHDOOR_MCP_MAX_ATTACHMENT_BYTES=10485760
HATCHDOOR_MCP_ADVERTISE_HOST_PATHS=true
```

If Hatchdoor is reachable beyond localhost, set a bearer token and configure OpenClaw to send it:

```env
HATCHDOOR_MCP_BEARER_TOKEN=change-me
```

## Run

### 1) Build frontend

```bash
cd frontend
npm install
npm run build
cd ..
```

### 2) Run backend

```bash
cargo run
```

Open `http://localhost:42824`.

## Docker

Build image:

```bash
docker build -t hatchdoor:latest .
```

Run with Docker Compose:

```bash
docker compose up -d
```

Compose uses `.env` via `env_file` for container runtime variables.
Use `HOST_VAULT_PATH` in `.env` for the host vault directory.
Use `HOST_CACHE_PATH` in `.env` for the host SQLite cache directory.
Use `HOST_ATTACHMENT_STAGING_PATH` in `.env` for the host-side attachment import inbox.

The container uses:

```text
/data/vault               = Markdown vault, source of truth
/data/cache               = generated SQLite cache
/data/attachments-inbox   = temporary attachment import staging folder
```

## API

- `GET /api/tree` -> explorer tree JSON
- `GET /api/note/:slug` -> note JSON (`title`, `slug`, `content`, `content_hash`)
- `GET /api/resolve?target=...` -> single wikilink resolution (`slug` or `null`)
- `POST /api/resolve-batch` -> batch wikilink resolution
- `GET /api/search?q=...` -> note search results
- `POST /api/refresh` -> force SQLite cache refresh from Markdown vault
- `GET /api/vault-events` -> Server-Sent Events stream for refreshed vault revisions
- `GET /vault-assets/*path` -> image assets from vault (`png`, `jpg`, `jpeg`, `gif`, `webp`, `svg`, `avif`, `bmp`)
- `GET /health` -> `ok`

## Embedded MCP for OpenClaw

Hatchdoor exposes a vault-safe Streamable HTTP MCP endpoint at `/mcp` when enabled.

Current MCP protocol version:

```text
2025-11-25
```

Enable it:

```env
HATCHDOOR_MCP_ENABLED=true
```

Enable write tools:

```env
HATCHDOOR_MCP_WRITE_ENABLED=true
HATCHDOOR_MCP_BEARER_TOKEN=change-me
```

Register it in OpenClaw:

```bash
openclaw mcp set hatchdoor '{"url":"http://127.0.0.1:42824/mcp","transport":"streamable-http","connectionTimeoutMs":10000}'
```

With bearer auth:

```bash
openclaw mcp set hatchdoor '{"url":"http://127.0.0.1:42824/mcp","transport":"streamable-http","connectionTimeoutMs":10000,"headers":{"Authorization":"Bearer change-me"}}'
```

MCP transport behaviour:

- `POST /mcp` handles JSON-RPC MCP requests.
- `GET /mcp` returns `405 Method Not Allowed` with `Allow: POST` because server-sent events are not implemented.

Vault-safe MCP tools:

- `search_notes` -> compact SQLite search results; prefer this before fetching full note content
- `get_note` -> fetch one note by slug with Markdown content and `content_hash` from SQLite cache
- `get_note_links` -> fetch outgoing links and backlinks for a slug
- `resolve_wikilink` -> resolve an Obsidian wikilink target to a slug
- `get_tree` -> fetch the explorer tree; potentially larger response
- `refresh_index` -> force Hatchdoor to refresh its SQLite view of the vault without modifying vault content
- `get_attachment_import_config` -> report attachment staging config, allowed extensions, max size, and usage guidance
- `get_git_sync_status` -> report whether git sync is enabled, the last sync time, whether it succeeded, the last error (with a machine-readable `last_error_kind`: `conflict`/`remote`/`validation`/`other`), how many writes are pending, and how many local commits are unpushed
- `create_note` -> create a Markdown note when write mode is enabled
- `update_note` -> replace note content with `expected_content_hash`
- `append_to_note` -> append Markdown with `expected_content_hash`
- `rename_note` -> rename a note, rewrite wikilink backlinks, move referenced assets, and rewrite other asset references
- `move_note` -> move a note folder, rewrite wikilink backlinks, move referenced assets, and rewrite other asset references
- `move_rename_note` -> move and rename in one operation
- `archive_note` -> move a note to the configured archive folder, rewrite wikilink backlinks, move referenced assets, and rewrite other asset references
- `delete_note` -> move a note and referenced assets to `.hatchdoor-trash`, rewrite other asset references, and remove backlinks to the deleted note
- `import_attachment` -> import a staged attachment into the vault
- `move_attachment` / `rename_attachment` -> move or rename an attachment and rewrite note references
- `delete_attachment` -> move an attachment to `.hatchdoor-trash` and rewrite note references
- `list_note_attachments` -> list attachments referenced by one note

Write tools modify Markdown files as the source of truth, then force a SQLite cache refresh. They do not expose shell or arbitrary filesystem path tools. Tool argument structs reject unknown fields so runtime behaviour matches the advertised schemas.

MCP attachment imports allow image formats except SVG, plus PDF. Existing SVG files may still be served from a vault, but MCP cannot import or move SVG attachments.

### Git sync

Hatchdoor can automatically commit and push vault changes to a git remote so that edits made by remote MCP agents propagate to every synced device. It is opt-in and off by default:

```env
HATCHDOOR_GIT_SYNC_ENABLED=true
HATCHDOOR_GIT_REMOTE=origin
HATCHDOOR_GIT_BRANCH=main
HATCHDOOR_GIT_HTTPS_USERNAME=hatchdoor
HATCHDOOR_GIT_HTTPS_TOKEN=your-token
HATCHDOOR_GIT_DEBOUNCE_SECONDS=30
HATCHDOOR_GIT_AUTHOR_NAME=Hatchdoor
HATCHDOOR_GIT_AUTHOR_EMAIL=hatchdoor@localhost
```

Requirements and behaviour:

- The vault directory must be a git repository whose root is the vault and whose checked-out `HEAD` is the configured branch. The remote URL comes from the repo's existing remote config; Hatchdoor only references the remote by name. Misconfiguration is fatal at startup so problems surface immediately.
- Authentication is HTTPS with a username and token. The token is required when sync is enabled and is never logged or surfaced in status or error output.
- After successful MCP write tools run, affected paths are committed and pushed. Writes are debounced (default 30s) and coalesced into a single commit. Agents may pass an optional `commit_summary` argument that is added to the commit body.
- Each sync fetches and integrates the remote before pushing. A clean merge is committed and pushed; a conflicting merge is aborted, the local commit is kept (not pushed), and the conflict must be resolved by a human on the server.
- Use the `get_git_sync_status` tool to check whether your changes have been committed and pushed. It reports an `unpushed` commit count (non-zero after a conflict abort or an outage) and a `last_error_kind` so a conflict is distinguishable from a transient remote error. When the most recent sync failed, write-tool responses also include a `git_sync_warning` field. Stranded commits from a previous run are flushed immediately on startup.

## Frontend Dev Mode

```bash
# terminal 1
cargo run

# terminal 2
cd frontend
npm run dev
```

Vite dev server proxies `/api` and `/health` to `http://127.0.0.1:42824`.

## Quality checks

```bash
cargo fmt --all --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings

cd frontend
npm run lint
npm run format:check
npm run typecheck
npm run test
npm run build
```
