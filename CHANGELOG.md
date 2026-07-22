# Changelog

## Unreleased

### ⚠️ Breaking changes — action required on upgrade
- **The MCP attachment staging folder is removed.** Agents no longer import
  attachments by dropping a file into a shared, mounted inbox and calling
  `import_attachment` with a `staged_filename`. Instead, `import_attachment` now
  takes the file bytes directly as base64 (`content` + `target_relative_path`),
  and larger files use the existing multipart `POST /api/attachment`.
  **Action:** remove the `HATCHDOOR_MCP_ATTACHMENT_STAGING_PATH`,
  `HOST_ATTACHMENT_STAGING_PATH`, and `HATCHDOOR_MCP_ADVERTISE_HOST_PATHS`
  variables from your `.env`, and delete the attachments-inbox volume mount from
  your Docker Compose file. Any agent workflow that placed files in the inbox
  must switch to sending base64 via `import_attachment` (call
  `get_attachment_import_config` to see the methods and limits).
- **`HATCHDOOR_MCP_MAX_ATTACHMENT_BYTES` is renamed to
  `HATCHDOOR_MAX_ATTACHMENT_BYTES`** (it caps the web UI and HTTP uploads, not
  just MCP). **Action:** rename it in your `.env` if you set it; otherwise the
  old name is ignored and the default (10 MiB) applies.

### Added
- Direct attachment upload for agents: the `import_attachment` MCP tool accepts
  base64 file bytes inline (universal fallback for any MCP client), capped by the
  new `HATCHDOOR_MCP_MAX_BASE64_BYTES` (default 5 MiB, measured on the decoded
  file). `get_attachment_import_config` now enumerates both upload methods, their
  size limits, and which to use.

### Changed
- The `/mcp` request-body limit is raised to fit base64 attachment inflation so a
  legitimately sized upload is not rejected before the tool's own size check.
- `POST /api/attachment` now accepts the MCP bearer token as an alternative to
  the web bearer token, so an agent uploading larger files over HTTP can reuse
  its existing MCP credential instead of needing `HATCHDOOR_WEB_BEARER_TOKEN`
  provisioned separately. The rest of the web API is unaffected — this route
  was pulled out of the shared protected-routes group so the MCP token is not
  granted any broader access.

## v2.3.0 - 2026-07-19

### Added
- The UI is now available during initial indexing and shows live token-weighted progress, note/chunk counts, and a measured ETA while vault and MCP data remain unavailable until the index commits.
- Indexing logs a human-readable heartbeat every minute and detailed performance diagnostics at debug level.
- MCP `search_notes` now supports exact tag, path-prefix, property-existence, and typed property-equality filters with explicit property projection.
- Added the metadata-only `query_notes` MCP tool and structured tags, aliases, and frontmatter properties to `get_note`.

### Changed
- Chunks are embedded individually to avoid batch-longest padding and reduce peak memory pressure.
- The cache schema is upgraded to version 6 for note-level frontmatter metadata. The first 2.3.0 startup automatically rebuilds the generated SQLite cache from the Markdown vault.
- Rust and frontend dependencies are refreshed within their current compatibility lines, including git2 0.21, resolving the current RustSec and npm audit findings.

## v2.1.1 - 2026-06-13

Security, performance, and operational hardening from the 2026-06-11 codebase audit, plus an iOS PWA download fix.

### Fixed
- Markdown downloads no longer arrive as HTML on iOS standalone PWAs. The service worker's SPA navigation fallback was intercepting `/api/*`, `/vault-assets/*`, and `/health` navigations (iOS ignores the `<a download>` attribute and treats the click as a navigation) and serving the cached `index.html`. Added a `navigateFallbackDenylist` so those requests reach the network.

### Security
- Added optional web API authentication: when `HATCHDOOR_WEB_BEARER_TOKEN` is set, all `/api/*` routes, `/vault-assets/*`, and note downloads require the token (via `Authorization: Bearer` header or `access_token` query parameter). The PWA prompts for the token on a 401. (F-01)
- Changed the default bind host to `127.0.0.1`; exposing on `0.0.0.0` is now an explicit opt-in documented to require auth or a reverse proxy. (F-01)
- Coalesced concurrent `/api/refresh` requests so a request loop can no longer trigger overlapping full reindexes. (F-02)
- Compared bearer tokens (MCP and web) in constant time. (F-06)
- Served SVG vault assets with `Content-Security-Policy: sandbox` and `Content-Disposition: attachment` to neutralize script execution on direct navigation. (F-09)
- Stopped leaking absolute filesystem paths and raw internal error strings in HTTP error bodies; details now go to logs only. (F-10)
- Capped `POST /api/resolve-batch` at 200 targets. (F-11)

### Performance
- Moved vault reindexing, embedding, and query embedding off the async runtime via `spawn_blocking`, holding the cache write lock only for the final swap so reads no longer freeze during a refresh. (F-03)
- MCP write tools now resolve the target note from the SQLite cache instead of rebuilding the full vault index from disk on every write. (F-04)
- Enabled SQLite WAL mode and added a pooled set of read connections so concurrent reads run in parallel instead of serializing on a single mutex. (F-05)

### Correctness & operations
- Validated `McpConfig` once at startup (failing fast when write mode is enabled without a bearer token) instead of re-parsing the environment on every MCP request. (F-07)
- Git sync now refuses to force-checkout over uncommitted manual edits to tracked vault files, surfacing them as an error instead of silently discarding them. (F-08)
- Moved the hard-coded `90-archive/` prefix to `HATCHDOOR_ARCHIVE_PREFIX`. (F-12)
- Added a Forgejo Actions CI workflow (fmt, clippy, test for the backend; lint, typecheck, test, build for the frontend) and a Docker Compose `healthcheck`. (F-13)
- The SSE vault-events stream now emits the current revision on broadcast lag instead of silently dropping it, so a slow client always resyncs. (F-16)
- `/health` now runs a `SELECT 1` against the cache so it reports unhealthy if the database is unreachable, and the binary gained a `--healthcheck` mode for the container probe. (F-17)

## v2.1.0 - 2026-06-xx

### Added
- Optional automatic git sync of the vault: successful MCP write tools commit and push changes to the configured remote with debounced batching, conflict-abort semantics, and an immediate flush of stranded commits on startup.
- `get_git_sync_status` MCP tool and a git-sync warning on write-tool responses.

### Changed
- Richer git sync status reporting with plural-aware commit messages.

### Fixed
- Enabled the git2 `https` feature for TLS remote transport.
- The vault watcher now ignores `.git/` so sync churn does not trigger reindexing.

## v2.0.0 - 2026-xx-xx

### Added
- SQLite read model (FTS5 + sqlite-vec embeddings) backing the vault index, with chunking, embedding, and hybrid/semantic/keyword retrieval.
- Streamable-HTTP MCP endpoint exposing read tools always and write tools (create/update/edit/replace-section/append/move/rename/delete notes and attachments) gated by env flag and bearer token.

### Changed
- Pinned the Rust toolchain to 1.96.0 and reformatted the tree.

## v1.1.0 - 2026-02-20

Compared with `v1.0.0`.

### Added
- Added a `Download .md` action in the note actions menu.
- Added a server download endpoint: `GET /api/note/{slug}/download`.

### Changed
- Switched markdown download flow to server-driven delivery for native mobile handoff.
- Updated frontend download trigger to use an anchor `download` flow instead of popup navigation.
- Added UTF-8-aware filename handling in `Content-Disposition` for markdown downloads.

### Fixed
- Improved iOS/Safari file handoff behavior for `.md` downloads by using attachment headers.
- Added and updated frontend/backend tests for the download path and response headers.
