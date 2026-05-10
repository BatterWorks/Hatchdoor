# Hatchdoor

Rust backend + React/Vite frontend for browsing an Obsidian vault in read-only mode.

## Features

- Explorer tree from vault folders/files
- Open notes via route (`/n/:slug`)
- Obsidian wikilinks (`[[Note]]`, `[[Note|Alias]]`) resolved through backend API
- Persistent SQLite cache/read model for fast API, UI, search, links, and MCP reads
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
- `HATCHDOOR_MCP_ENABLED=false`
- `HATCHDOOR_MCP_BEARER_TOKEN=`
- `HATCHDOOR_MCP_ALLOWED_ORIGINS=http://127.0.0.1,http://localhost`
- `RUST_LOG=hatchdoor=info,tower_http=info,axum::rejection=warn`

`VAULT_REFRESH_SECONDS` controls how often API/UI/MCP requests may trigger a fresh vault scan.
`HATCHDOOR_CACHE_DB` points to Hatchdoor's generated SQLite cache. Keep it outside the Markdown vault.
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

- API/UI/MCP requests trigger refresh only when `VAULT_REFRESH_SECONDS` has elapsed.
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

The container uses:

```text
/data/vault  = Markdown vault, source of truth
/data/cache  = generated SQLite cache
```

## API

- `GET /api/tree` -> explorer tree JSON
- `GET /api/note/:slug` -> note JSON (`title`, `slug`, `content`)
- `GET /api/resolve?target=...` -> single wikilink resolution (`slug` or `null`)
- `POST /api/resolve-batch` -> batch wikilink resolution
- `GET /api/search?q=...` -> note search results
- `POST /api/refresh` -> force SQLite cache refresh from Markdown vault
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
- `get_note` -> fetch one note by slug with Markdown content from SQLite cache
- `get_note_links` -> fetch outgoing links and backlinks for a slug
- `resolve_wikilink` -> resolve an Obsidian wikilink target to a slug
- `get_tree` -> fetch the explorer tree; potentially larger response
- `refresh_index` -> force Hatchdoor to refresh its SQLite view of the vault without modifying vault content

The MCP endpoint does not expose write, delete, shell, or arbitrary filesystem path tools. Tool argument structs reject unknown fields so runtime behaviour matches the advertised schemas.

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
