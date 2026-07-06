<p align="center">
  <img src="assets/hatchdoor-wordmark.svg" alt="Hatchdoor" width="340">
</p>

# Hatchdoor

Hatchdoor is a self-hosted web app for browsing, searching, and editing an
Obsidian-style Markdown vault.

Your Markdown files stay the source of truth. Hatchdoor builds a disposable
SQLite read model for fast browsing, links, backlinks, keyword search, semantic
search, graph data, and metadata. If the cache is deleted, Hatchdoor rebuilds it
from the vault.

Hatchdoor was fully vibecoded with help from Claude Code and Codex.

## What You Get

- A web UI for browsing folders and Markdown notes.
- Clean note URLs at `/n/:slug`.
- Obsidian-style wikilinks for `[[Note]]`, `[[Folder/Note]]`, and
  `[[Note|Alias]]`.
- Markdown rendering with GitHub-flavored Markdown, math, Mermaid diagrams,
  frontmatter, images, attachments, and broken-link styling.
- Keyword search and semantic search.
- Recent notes, backlinks, outbound links, stats, and graph views.
- Browser write support when the vault mount is writable.
- Attachment uploads and local asset serving.
- Optional MCP endpoint for agent access.
- Optional automatic git commits and pushes for Hatchdoor writes.
- PWA assets and service worker caching for common read paths.

## Who It Is For

Hatchdoor is useful if you have a folder of Markdown notes and want a private
web interface for them.

It is beginner-friendly enough to run with Docker Compose, but it also includes
advanced features for people who want agent access, git-backed vault sync,
semantic search, and local development.

Hatchdoor is not a hosted sync service, not a multi-user collaboration platform,
and not a replacement for Obsidian. It is a self-hosted companion for a Markdown
vault you control.

## Quick Start With Docker

### 1. Requirements

You need:

- Docker
- Docker Compose
- A Markdown vault folder, or an empty folder if you want Hatchdoor to create a
  starter vault

### 2. Create Your Config

Copy the example environment file:

```bash
cp .env.example .env
```

Edit `.env` and set at least these values:

```env
HOST_VAULT_PATH=/absolute/path/to/your/markdown-vault
HOST_CACHE_PATH=./data/cache
HATCHDOOR_WEB_BEARER_TOKEN=choose-a-long-random-token
```

What these mean:

- `HOST_VAULT_PATH` is your Markdown vault on the host machine.
- `HOST_CACHE_PATH` stores Hatchdoor's generated SQLite cache.
- `HATCHDOOR_WEB_BEARER_TOKEN` protects your notes in the browser.

Docker Compose binds Hatchdoor to `0.0.0.0` inside the container so the
published port works. Hatchdoor refuses to start on a non-loopback bind unless
`HATCHDOOR_WEB_BEARER_TOKEN` is set, except in explicit read-only demo mode.

### 3. Start Hatchdoor

```bash
docker compose up -d
```

Open:

```text
http://localhost:42824
```

Enter the web bearer token when prompted.

### 4. Container Paths

The Docker image is:

```text
battermanz/hatchdoor:latest
```

Docker Compose mounts:

| Container path | Purpose |
| --- | --- |
| `/data/vault` | Markdown vault, source of truth |
| `/data/cache` | Generated SQLite cache |
| `/data/attachments-inbox` | Temporary staging folder for MCP attachment import |

## Data And Safety Model

Hatchdoor is designed around a simple rule: your Markdown vault is the source of
truth.

- Markdown files live in `VAULT_PATH`.
- SQLite is a generated cache and can be rebuilt.
- The SQLite cache should live outside the vault.
- Hatchdoor scans `.md` files under the vault, excluding `.hatchdoor-trash`.
- Delete actions move notes and referenced assets into `.hatchdoor-trash`.
- Archive actions move notes under `HATCHDOOR_ARCHIVE_PREFIX`.
- Browser write actions are available only when the vault is writable.
- MCP is disabled by default.
- MCP requires its own bearer token whenever it is enabled.
- Git sync is disabled by default.

If `VAULT_PATH` contains no Markdown files, Hatchdoor creates a small starter
vault before the first index build. Existing vaults are not seeded or modified
by this startup step.

## Permissions

The Docker image runs as a non-root user.

For read-only browsing:

- Mount the vault read-only if you want.
- Keep the cache directory writable.

For browser writes, MCP writes, attachment uploads, or git sync:

- The vault mount must be writable by the container runtime user.
- The cache directory must be writable.
- The attachment staging directory must be writable when MCP attachment import is
  enabled.

If Hatchdoor starts but write features are disabled, check the permissions on
your vault mount and call `/api/write-capabilities` from an authenticated
browser session.

## Configuration

Copy `.env.example` to `.env` and adjust values.

### Basic Server And Storage

| Variable | Default | Purpose |
| --- | --- | --- |
| `HOST_VAULT_PATH` | `./vault` | Host-side vault path for Docker Compose |
| `HOST_CACHE_PATH` | `./data/cache` | Host-side cache directory for Docker Compose |
| `VAULT_PATH` | `/data/vault` | Runtime vault path read by Hatchdoor |
| `HATCHDOOR_CACHE_DB` | `/data/cache/hatchdoor-cache.sqlite3` | Runtime SQLite cache file |
| `HOST` | `127.0.0.1` | Bind host for local `cargo run` |
| `PORT` | `42824` | HTTP port |
| `RUST_LOG` | `hatchdoor=info,tower_http=info,axum::rejection=warn` | Backend logging filter |

### Web Authentication

| Variable | Default | Purpose |
| --- | --- | --- |
| `HATCHDOOR_WEB_BEARER_TOKEN` | empty | Protects `/api/*`, `/vault-assets/*`, and note downloads |
| `HATCHDOOR_DEMO_MODE` | `false` | Allows unauthenticated public browsing while disabling Hatchdoor write features |

When this token is set, protected requests must send:

```text
Authorization: Bearer <token>
```

The bundled PWA stores the token locally after a `401` response and attaches it
to API calls. For image, download, and server-sent-event URLs where headers
cannot be set, the frontend appends an `access_token` query parameter.

Hatchdoor refuses to start with `HOST=0.0.0.0` or another non-loopback bind
unless `HATCHDOOR_WEB_BEARER_TOKEN` is set.

For a public test instance that people can browse without credentials, use demo
mode:

```env
HOST=0.0.0.0
HATCHDOOR_DEMO_MODE=true
HATCHDOOR_WEB_BEARER_TOKEN=
HATCHDOOR_MCP_ENABLED=false
HATCHDOOR_GIT_SYNC_ENABLED=false
```

Demo mode is intentionally read-only. It disables browser write operations,
reports writes as unavailable through `/api/write-capabilities`, and refuses to
start if MCP or automatic git sync are enabled.

### Vault Behavior

| Variable | Default | Purpose |
| --- | --- | --- |
| `HATCHDOOR_ARCHIVE_PREFIX` | `90-archive/` | Vault-relative folder prefix used by archive actions and archived-link styling |

## Using Hatchdoor

### Browsing

Hatchdoor builds a folder explorer from your vault folders and Markdown files.
The UI root is named `Vault`. Folder names come directly from your filesystem;
Hatchdoor does not require a PARA, Zettelkasten, or numbered folder scheme.

### Note URLs And Links

Note slugs are generated from Markdown filenames. Duplicate filenames receive
unique suffixes so routes remain stable.

Supported wikilinks include:

```text
[[Note]]
[[Folder/Note]]
[[Note|Alias]]
```

Hatchdoor resolves links, backlinks, headings, tags, graph data, and broken-link
state into the SQLite cache.

### Search

Hatchdoor stores:

- Full Markdown content
- File metadata
- Tags and headings
- Wikilinks and backlinks
- FTS5 keyword search data
- sqlite-vec semantic vectors

A recursive vault watcher refreshes the cache after Markdown or asset changes.
Browser clients subscribe to `/api/vault-events` and reload visible data after a
refreshed revision is broadcast.

### Manual Cache Rebuild

If you want to rebuild the cache from scratch:

```bash
rm ./data/cache/hatchdoor-cache.sqlite3
docker compose restart hatchdoor
```

Adjust the path if you changed `HOST_CACHE_PATH`.

## MCP Agent Access

The embedded MCP endpoint is disabled by default. Enable it only for trusted
clients.

MCP requires a bearer token even in read-only mode because `/mcp` bypasses the
web auth layer and can expose the full vault.

```env
HATCHDOOR_MCP_ENABLED=true
HATCHDOOR_MCP_BEARER_TOKEN=change-me
```

Enable write tools separately:

```env
HATCHDOOR_MCP_WRITE_ENABLED=true
```

Register the endpoint with a Streamable HTTP MCP client:

```text
http://127.0.0.1:42824/mcp
```

Send:

```text
Authorization: Bearer <token>
```

### MCP Attachment Import

Attachment import uses a staging folder outside the vault:

```env
HOST_ATTACHMENT_STAGING_PATH=./data/attachments-inbox
HATCHDOOR_MCP_ATTACHMENT_STAGING_PATH=/data/attachments-inbox
HATCHDOOR_MCP_MAX_ATTACHMENT_BYTES=10485760
HATCHDOOR_MCP_ADVERTISE_HOST_PATHS=false
```

Set `HATCHDOOR_MCP_ADVERTISE_HOST_PATHS=true` only for local/dev agents that
need to see the host staging path. Keep it false for shared or remote
deployments.

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

## Running Without Docker

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
Point Hatchdoor at a real vault with:

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

Optional: prefetch the embedder model used by semantic search:

```bash
cargo run -- --prefetch-embedder
```

## Troubleshooting

### Hatchdoor refuses to start on `0.0.0.0`

Set `HATCHDOOR_WEB_BEARER_TOKEN`, bind to `127.0.0.1`, or enable
`HATCHDOOR_DEMO_MODE=true` for a read-only public demo.

This is intentional. A non-loopback bind can expose your vault to the network,
so Hatchdoor requires web authentication unless demo mode has disabled the app's
write surfaces.

### Docker starts, but the UI cannot write

Check that the mounted vault directory is writable by the container runtime
user. Browser write support depends on filesystem permissions.

### Cache errors or stale data

Delete the generated SQLite cache and restart:

```bash
rm ./data/cache/hatchdoor-cache.sqlite3
docker compose restart hatchdoor
```

### The app starts with a starter vault

Hatchdoor seeds starter notes only when `VAULT_PATH` contains no Markdown files.
If you expected an existing vault, check `HOST_VAULT_PATH` in `.env`.

### MCP returns `401` or `403`

Check:

- `HATCHDOOR_MCP_ENABLED=true`
- `HATCHDOOR_MCP_BEARER_TOKEN` is set
- The client sends `Authorization: Bearer <token>`
- Browser-originated MCP requests come from an allowed origin in
  `HATCHDOOR_MCP_ALLOWED_ORIGINS`

### Git sync does not push

Check:

- The vault is a git repository root.
- The current branch matches `HATCHDOOR_GIT_BRANCH`.
- The remote exists in the repo config.
- The HTTPS token can push.
- There are no merge conflicts waiting for manual resolution.

## API Reference

Common routes:

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/health` | Health check |
| `GET` | `/api/tree` | Folder and note tree |
| `GET` | `/api/recently-modified` | Recently modified notes |
| `GET` | `/api/note/:slug` | Read a note |
| `GET` | `/api/note/:slug/links` | Outbound links and backlinks |
| `GET` | `/api/note/:slug/download` | Download a Markdown export |
| `GET` | `/api/resolve?target=...` | Resolve one wikilink target |
| `POST` | `/api/resolve-batch` | Resolve multiple wikilink targets |
| `GET` | `/api/search?q=...` | Search notes |
| `GET` | `/api/stats` | Vault stats |
| `GET` | `/api/graph` | Graph data |
| `POST` | `/api/refresh` | Trigger cache refresh |
| `GET` | `/api/vault-events` | Server-sent vault revision events |
| `GET` | `/api/write-capabilities` | Check write availability |
| `POST` | `/api/note` | Create a note |
| `PUT` | `/api/note/:slug` | Update a note |
| `PATCH` | `/api/note/:slug/rename` | Rename a note |
| `PATCH` | `/api/note/:slug/move` | Move a note |
| `PATCH` | `/api/note/:slug/archive` | Archive a note |
| `PATCH` | `/api/note/:slug/move-rename` | Move and rename a note |
| `DELETE` | `/api/note/:slug` | Move a note to trash |
| `POST` | `/api/attachment` | Upload an attachment |
| `GET` | `/vault-assets/*path` | Serve vault assets |
| `POST` | `/mcp` | Streamable HTTP MCP endpoint |

## Security Notes

- Use a long random `HATCHDOOR_WEB_BEARER_TOKEN`.
- Do not expose Hatchdoor publicly without HTTPS in front of it.
- Use `HATCHDOOR_DEMO_MODE=true` only for browse-only public test instances.
- Keep MCP disabled unless you need it.
- Treat MCP write mode as powerful: it can create, edit, move, delete, and
  import content.
- Keep the SQLite cache outside the vault.
- Keep `.env` out of git.
- Review Docker volume paths before starting the container.

## Development

Backend checks:

```bash
cargo fmt --check
CARGO_BUILD_JOBS=1 cargo clippy --all-targets -- -D warnings
CARGO_BUILD_JOBS=1 cargo test
```

Frontend checks:

```bash
cd frontend
npm run format:check
npm run typecheck
npm run lint
npm test
npm run build
```

Build and publish the Docker image:

```bash
docker build -t battermanz/hatchdoor:latest .
docker tag battermanz/hatchdoor:latest battermanz/hatchdoor:2.2.0
docker push battermanz/hatchdoor:2.2.0
docker push battermanz/hatchdoor:latest
```

## Project Docs

- [Design system](docs/design-system.html): visual tokens, component patterns,
  layout rules, and interaction states used by the frontend.
- [Semantic search strategy](docs/adr/semantic-search-strategy.md): decision
  record for shipping pure semantic search instead of hybrid retrieval or a
  cross-encoder reranker in the runtime path.

## License

Hatchdoor is licensed under the GNU Affero General Public License v3.0 only.
See [LICENSE](LICENSE).
