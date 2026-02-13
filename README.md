# Hatchdoor

Rust backend + React/Vite frontend for browsing an Obsidian vault in read-only mode.

## Features

- Explorer tree from vault folders/files
- Open notes via route (`/n/:slug`)
- Obsidian wikilinks (`[[Note]]`, `[[Note|Alias]]`) resolved through backend API
- Markdown rendering with:
  - GFM (tables, task lists, strikethrough)
  - Math (`remark-math` + KaTeX)
  - Mermaid fenced code blocks (lazy-loaded for mobile performance)
- Unresolved links rendered with explicit broken-link styling
- PWA build output (manifest + service worker via Vite PWA)
- Live vault updates without server restarts via periodic backend reindex

## Configuration

Copy `.env.example` to `.env`:

- `VAULT_PATH=./vault`
- `HOST=0.0.0.0`
- `PORT=42824`
- `VAULT_REFRESH_SECONDS=2`
- `RUST_LOG=hatchdoor=info,tower_http=info,axum::rejection=warn`

`VAULT_REFRESH_SECONDS` controls how often API requests may trigger a fresh vault scan.
`RUST_LOG` controls structured backend log verbosity.

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
The container always uses `VAULT_PATH=/data/vault`.

## API

- `GET /api/tree` -> explorer tree JSON
- `GET /api/note/:slug` -> note JSON (`title`, `slug`, `content`)
- `GET /api/resolve?target=...` -> single wikilink resolution (`slug` or `null`)
- `POST /api/resolve-batch` -> batch wikilink resolution
- `POST /api/refresh` -> force vault reindex
- `GET /vault-assets/*path` -> image assets from vault (`png`, `jpg`, `jpeg`, `gif`, `webp`, `svg`, `avif`, `bmp`)
- `GET /health` -> `ok`

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
