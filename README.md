# Hatchdoor

Rust backend + React/Vite frontend for browsing an Obsidian vault in read-only mode.

## Features

- Explorer tree from vault folders/files
- Open notes via route (`/n/:slug`)
- Obsidian wikilinks (`[[Note]]`, `[[Note|Alias]]`) resolved through backend API
- Markdown rendering with:
  - GFM (tables, task lists, strikethrough)
  - Math (`remark-math` + KaTeX)
  - Mermaid fenced code blocks
- PWA build output (manifest + service worker via Vite PWA)

## Configuration

Copy `.env.example` to `.env`:

- `VAULT_PATH=./vault`
- `HOST=0.0.0.0`
- `PORT=42824`

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

## API

- `GET /api/tree` -> explorer tree JSON
- `GET /api/note/:slug` -> note JSON (`title`, `slug`, `content`)
- `GET /api/resolve?target=...` -> wikilink resolution (`slug` or `null`)
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
npm run build
```
