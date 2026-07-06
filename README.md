# Hatchdoor

Hatchdoor is a self-hosted web app for browsing and editing an
Obsidian-style Markdown vault. It combines a Rust/Axum backend, a React/Vite
PWA, a disposable SQLite read model, semantic search, and an optional
Streamable HTTP MCP endpoint for agent access.

The Markdown vault remains the source of truth. SQLite can be deleted and
rebuilt from the vault at any time.

## Features

- Folder explorer generated from vault folders and Markdown files.
- Note routes at `/n/:slug`.
- Obsidian wikilink resolution for `[[Note]]`, `[[Folder/Note]]`, and
  `[[Note|Alias]]`.
- SQLite cache for note metadata, content, tags, headings, links, backlinks,
  keyword search, semantic search, stats, and graph data.
- Markdown rendering with GFM, math, Mermaid diagrams, images, frontmatter, and
  broken-link styling.
- Optional browser write mode for creating, editing, moving, archiving, deleting,
  and uploading attachments.
- Optional MCP endpoint for read and write tools.
- Optional git sync for automatically committing and pushing Hatchdoor writes.
- PWA build output with service worker caching for common read paths.

## Quick Start With Docker

1. Copy the example environment file:

   ```bash
   cp .env.example .env
   ```

2. Edit `.env`:

   ```env
   HOST_VAULT_PATH=/absolute/path/to/your/markdown-vault
   HOST_CACHE_PATH=./data/cache
   HATCHDOOR_WEB_BEARER_TOKEN=choose-a-long-random-token
   ```

   Docker Compose binds Hatchdoor to `0.0.0.0` inside the container so Docker
   port publishing works. Hatchdoor refuses to start on a non-loopback bind
   unless `HATCHDOOR_WEB_BEARER_TOKEN` is set.

3. Start the app:

   ```bash
   docker compose up -d
   ```

4. Open `http://localhost:42824` and enter the web bearer token when prompted.

Compose uses the published image:

```text
battermanz/hatchdoor:latest
```

The container paths are:

```text
/data/vault               Markdown vault, source of truth
/data/cache               generated SQLite cache
/data/attachments-inbox   temporary attachment import staging folder
```

## Docker Permissions

The runtime image runs as a non-root user. The mounted cache directory must be
writable by the container user, and browser/MCP write mode also requires the
vault mount to be writable.

For read-only browsing, mount the vault read-only and keep only the cache path
writable. For write features, make sure the host vault directory allows writes
from the container runtime user or use a Docker volume with suitable ownership.

## Local Development

Build the frontend once:

```bash
cd frontend
npm ci
npm run build
cd ..
```

Run the backend:

```bash
cargo run
```

By default, local source runs bind to `127.0.0.1:42824` and read `./vault`.
Override `VAULT_PATH` to point at a real local vault:

```bash
VAULT_PATH=/path/to/notes cargo run
```

For frontend dev mode:

```bash
# terminal 1
cargo run

# terminal 2
cd frontend
npm run dev
```

## Configuration

Copy `.env.example` to `.env` and adjust values.

Important settings:

- `HOST_VAULT_PATH`: host-side Markdown vault path for Docker Compose.
- `VAULT_PATH`: runtime vault path read by Hatchdoor. In Docker this should
  usually stay `/data/vault`.
- `HOST_CACHE_PATH`: host-side directory for the generated SQLite cache.
- `HATCHDOOR_CACHE_DB`: runtime SQLite cache file path.
- `HOST`: bind host for local runs. Docker Compose overrides this to
  `0.0.0.0` inside the container.
- `PORT`: HTTP port, default `42824`.
- `HATCHDOOR_WEB_BEARER_TOKEN`: protects `/api/*`, `/vault-assets/*`, and note
  downloads. Required for non-loopback binds.
- `HATCHDOOR_ARCHIVE_PREFIX`: vault-relative folder prefix used by archive
  actions and archived-link styling. Default: `90-archive/`.
- `RUST_LOG`: backend log filter.

## Vault Layout

Hatchdoor scans every `.md` file under `VAULT_PATH`, except files under
`.hatchdoor-trash`. Folder names come directly from your vault; Hatchdoor does
not require a numbered PARA-style folder scheme.

Current conventions:

- The UI root is named `Vault`.
- Note slugs are generated from Markdown filenames.
- Duplicate filenames receive unique slug suffixes.
- Archive actions move notes under `HATCHDOOR_ARCHIVE_PREFIX`.
- Delete actions move notes and referenced assets under `.hatchdoor-trash`.
- The SQLite cache should live outside the vault.

## Project Docs

- [Design system](docs/design-system.html): visual tokens, component patterns,
  layout rules, and interaction states used by the frontend.
- [Semantic search strategy](docs/adr/semantic-search-strategy.md): decision
  record for shipping pure semantic search instead of hybrid retrieval or a
  cross-encoder reranker in the runtime path.

## SQLite Cache

Hatchdoor stores this generated read model in SQLite:

- note metadata and full Markdown content
- normalized title/path lookup data
- file modification time, size, and stable content hash
- explorer tree data
- FTS5 keyword search index
- sqlite-vec semantic vectors
- resolved wikilinks and backlinks
- headings and tags

A recursive vault watcher refreshes the cache after Markdown or asset changes.
Browser clients subscribe to `/api/vault-events` and reload visible data when a
refreshed revision is broadcast.

Manual rebuild:

```bash
rm ./data/cache/hatchdoor-cache.sqlite3
docker compose restart hatchdoor
```

## Authentication

When `HATCHDOOR_WEB_BEARER_TOKEN` is set, protected web requests must send:

```text
Authorization: Bearer <token>
```

The bundled PWA stores the token locally after a `401` response and attaches it
to API calls. For image, download, and SSE URLs where headers cannot be set, the
frontend appends an `access_token` query parameter.

Hatchdoor refuses to start with `HOST=0.0.0.0` or any other non-loopback bind
unless the web bearer token is set.

## MCP

The embedded MCP endpoint is disabled by default. Enabling it requires a bearer
token even in read-only mode, because `/mcp` bypasses the web auth layer and can
expose the full vault.

```env
HATCHDOOR_MCP_ENABLED=true
HATCHDOOR_MCP_BEARER_TOKEN=change-me
```

Enable write tools separately:

```env
HATCHDOOR_MCP_WRITE_ENABLED=true
```

Attachment import uses a staging folder outside the vault:

```env
HOST_ATTACHMENT_STAGING_PATH=./data/attachments-inbox
HATCHDOOR_MCP_ATTACHMENT_STAGING_PATH=/data/attachments-inbox
HATCHDOOR_MCP_MAX_ATTACHMENT_BYTES=10485760
HATCHDOOR_MCP_ADVERTISE_HOST_PATHS=false
```

Register the MCP endpoint with a Streamable HTTP client at:

```text
http://127.0.0.1:42824/mcp
```

Send the MCP bearer token as `Authorization: Bearer <token>`.

## Git Sync

Git sync is optional and disabled by default. When enabled, successful Hatchdoor
write tools commit and push vault changes to the configured remote.

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

Requirements:

- The vault directory must be a git repository root.
- The checked-out branch must match `HATCHDOOR_GIT_BRANCH`.
- The remote URL comes from the repository's existing remote config.
- Authentication uses HTTPS username/token credentials.
- Merge conflicts are kept for human resolution on the server.

Use the `get_git_sync_status` MCP tool to check whether recent writes were
committed and pushed.

## API

- `GET /health`
- `GET /api/tree`
- `GET /api/recently-modified`
- `GET /api/note/:slug`
- `GET /api/note/:slug/links`
- `GET /api/note/:slug/download`
- `GET /api/resolve?target=...`
- `POST /api/resolve-batch`
- `GET /api/search?q=...`
- `GET /api/stats`
- `GET /api/graph`
- `POST /api/refresh`
- `GET /api/vault-events`
- `GET /api/write-capabilities`
- `POST /api/note`
- `PUT /api/note/:slug`
- `PATCH /api/note/:slug/rename`
- `PATCH /api/note/:slug/move`
- `PATCH /api/note/:slug/archive`
- `PATCH /api/note/:slug/move-rename`
- `DELETE /api/note/:slug`
- `POST /api/attachment`
- `GET /vault-assets/*path`
- `POST /mcp`

## Build And Publish The Docker Image

```bash
docker build -t battermanz/hatchdoor:latest .
docker tag battermanz/hatchdoor:latest battermanz/hatchdoor:2.2.0
docker push battermanz/hatchdoor:2.2.0
docker push battermanz/hatchdoor:latest
```

## Checks

Backend:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

Frontend:

```bash
cd frontend
npm run lint
npm run typecheck
npm test
npm run build
```

## License

Hatchdoor is licensed under the GNU Affero General Public License v3.0 only.
See [LICENSE](LICENSE).
