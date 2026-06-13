# Hatchdoor — Codebase Audit Report

**Date:** 2026-06-11 · **Commit:** `815f784` (branch `development`, clean tree) · **Scope:** full repository, read-only.

---

## Executive summary

**Overall health score: 7.5 / 10**

Hatchdoor is a well-engineered personal-scale system: a Rust/axum backend that indexes an Obsidian vault into SQLite (FTS5 + sqlite-vec embeddings), serves a React/Vite PWA, and exposes an optional write-capable MCP endpoint with debounced git sync. Code quality is consistently high — small modules, careful path sanitization, optimistic concurrency on writes, a real test suite (~223 test functions), and thoughtful touches like vendored libgit2, distroless non-root Docker, and unknown-field rejection on MCP tool args. The weaknesses are concentrated in two areas: **the web/API surface has no authentication at all**, and **heavy CPU/IO work (embedding, full reindex) runs synchronously on the async runtime while holding the global cache lock**.

### Top 5 highest-priority issues

1. **F-01 — The entire read API is unauthenticated and binds to `0.0.0.0` by default.** Anyone who can reach port 42824 can read every note, the full graph, and download notes with assets. Only `/mcp` has bearer auth.
2. **F-02 — `POST /api/refresh` is unauthenticated and triggers a full vault re-read + re-embed.** A trivial request loop pins a CPU core and stalls all reads (cheap DoS).
3. **F-03 — Reindex/embedding runs synchronously in async context while holding the cache write lock.** Every vault change blocks a tokio worker thread and freezes all API/MCP reads for the duration of the reindex.
4. **F-08 — Git sync's force checkout after a clean merge can silently discard uncommitted manual edits** made directly on the server's vault files (data loss edge; needs verification).
5. **F-04 — Each MCP write performs two full vault walks plus a full reindex pass** (`current_index()` + `refresh_after_write()`), making write latency O(vault size).

### Production-ready?

**Yes, for its actual deployment context** (single-user homelab behind a LAN, per `docker-compose.yml` pointing at a private registry) — with the caveat that it must never be exposed beyond a trusted network until F-01/F-02 are addressed. **No, for any internet-facing or multi-user use.**

### Biggest risk area

**The unauthenticated HTTP surface combined with the default `HOST=0.0.0.0` bind.** Everything else is hardening; this is the one place where a misconfigured router/port-forward turns a private vault into a public website.

---

## Detailed findings

| ID | Category | Severity | File / path | Issue |
|----|----------|----------|-------------|-------|
| F-01 | Security | **High** (deployment-dependent) | `src/main.rs:38-69`, `src/app_state.rs:31`, `.env.example` | No authentication on any `/api/*`, `/vault-assets/*`, or download route; default bind `0.0.0.0` |
| F-02 | Security / DoS | **High** | `src/main.rs:53`, `src/handlers/api.rs:121-126` | Unauthenticated `POST /api/refresh` forces a full reindex + re-embed per call |
| F-03 | Performance / Correctness | **High** | `src/app_state.rs:118-145`, `src/cache/populate.rs:25-89` | Blocking CPU work (vault walk, chunking, ONNX embedding) inside async fn, holding the `cache` `RwLock` write guard; no `spawn_blocking` |
| F-04 | Performance | Medium | `src/mcp/tools.rs:699-706` (`current_index`), `refresh_after_write` | Every MCP write rebuilds `VaultIndex` from disk twice (once for `note_entry`, once via refresh) — O(vault) per write |
| F-05 | Performance | Medium | `src/cache/mod.rs:16-18`, all handlers | Single `std::sync::Mutex<Connection>` serializes all DB access; SQLite + embedder calls are synchronous inside async handlers |
| F-06 | Security | Low | `src/mcp/config.rs:99-104` | Bearer token compared with `==` (non-constant-time); timing side channel is theoretical here |
| F-07 | Code quality | Low | `src/mcp/routes.rs:16,25` | `McpConfig::from_env()` re-parses environment on **every** MCP request; misconfiguration (write mode without token) surfaces per-request instead of failing startup |
| F-08 | Bug / Data loss | **Medium** (needs verification) | `src/git/sync.rs:313` (`checkout_head(...force())`) | After a clean merge, force checkout makes the working tree match HEAD — uncommitted *manual* edits to tracked vault files (e.g. via Obsidian sync on the server) are overwritten. Only MCP-written paths are ever staged, so manual edits are always "uncommitted" |
| F-09 | Security / XSS | Low (theoretical) | `src/handlers/assets.rs:105-121` | SVG served inline as `image/svg+xml` from the same origin; a malicious SVG entering the vault via git sync/other devices executes script when navigated directly. MCP blocks SVG import, but other entry paths don't |
| F-10 | Security / Info leak | Low | `src/handlers/assets.rs:28`, `internal_error_response` everywhere | Error bodies leak absolute filesystem paths and raw internal error strings to clients |
| F-11 | DoS | Low | `src/handlers/api.rs:89-119` | `POST /api/resolve-batch` has no cap on `targets` length — unbounded per-request work |
| F-12 | Code quality / Coupling | Low | `src/handlers/api.rs:106` | `"90-archive/"` archive convention hard-coded in the server; personal vault layout baked into product code |
| F-13 | Operations | Medium | repo root (no `.github/`), `docker-compose.yml` | No CI at all — fmt/clippy/test/build run only by convention; compose has no `healthcheck:` despite a `/health` endpoint existing |
| F-14 | Security / Hygiene | Low | `.claude/settings.local.json:5` | A real-looking MCP bearer token is embedded in an allowed-command string. Gitignored (`.claude/` is in `.gitignore`), so not in history — but rotate it if it's the production token |
| F-15 | Docs / Hygiene | Low | `CHANGELOG.md` | Changelog stops at v1.1.0 (2026-02-20) while the crate is at 2.1.0; README/changelog drift |
| F-16 | Correctness | Low | `src/handlers/api.rs:128-141` | SSE `BroadcastStream` lag errors are silently dropped (`filter_map` → `None`); a slow client can miss a revision event with no resync signal. Self-healing on the *next* event, but a stale UI can persist indefinitely on a quiet vault |
| F-17 | Operations | Low | `src/handlers/api.rs:20-22` | `/health` returns static `ok` without touching SQLite — it reports healthy even if the cache DB is locked/corrupt |
| F-18 | Code quality | Info | `src/cache/parse.rs:50-61` | `content_hash` is FNV-1a-64, used as the optimistic-concurrency token. Fine for accidental-conflict detection; just don't ever treat it as integrity. (Verify whether the `blake3` Cargo dependency is still used anywhere — if chunk hashing moved to FNV, it may be removable) |

### Evidence, impact, and fixes (per finding)

**F-01 — Unauthenticated read surface.**
*Evidence:* `build_router` (`src/main.rs:38`) wires `/api/tree`, `/api/note/{slug}`, `/api/search`, `/api/graph`, `/api/note/{slug}/download`, `/vault-assets/*` with no auth layer; `AppConfig::from_env` defaults `HOST` to `0.0.0.0` (`src/app_state.rs:31`), and `.env.example` ships the same.
*Impact:* full vault disclosure to anyone with network reach. The bearer token guards only `/mcp`, which creates a false sense of security.
*Fix:* add an optional `HATCHDOOR_WEB_BEARER_TOKEN` (or basic auth / reverse-proxy guidance) as a middleware layer on the whole router; flip the `.env.example` default to `127.0.0.1` and document `0.0.0.0` as the opt-in.
*Test:* router-level integration test asserting 401 on `/api/note/...` when the web token is set and absent from the request.

**F-02 — Unauthenticated `/api/refresh`.**
*Evidence:* `refresh_handler` calls `refresh_if_needed` directly; combined with F-03 this means each call does a full `VaultIndex::build` + chunk/embed pass under the write lock.
*Impact:* one `while true; curl -X POST /api/refresh` from any LAN device makes the server unusable.
*Fix:* require auth (see F-01) and/or coalesce concurrent refreshes (drop the request if a refresh is already running — the watcher already debounces; the HTTP path doesn't).
*Test:* spawn N concurrent refresh requests, assert at most one reindex executes.

**F-03 — Blocking reindex/embedding on the async runtime.**
*Evidence:* `refresh_if_needed` (`src/app_state.rs:118`) takes `cache.write().await` then synchronously runs `VaultIndex::build` (walks + reads every file, `src/vault/index.rs:19,148`) and `replace_from_index_with_embedder` (ONNX embedding inside a single SQLite transaction, `src/cache/populate.rs:56-69`). The same pattern: `semantic_search` embeds the query synchronously in handlers, and every handler does sync SQLite I/O. Contrast: git sync *does* use `spawn_blocking` correctly (`src/git/task.rs:154`).
*Impact:* during any reindex, (a) one tokio worker thread is hogged, and (b) all readers block on the `RwLock`. On a large vault or cold start with many new chunks, the whole API freezes for seconds-to-minutes. With the embedding work inside the open transaction, the SQLite write lock is also held the whole time.
*Fix:* wrap the index build + populate in `tokio::task::spawn_blocking`; only take the `cache` write lock to swap in the result (the rebuild doesn't need the lock — it writes to the same `Arc<SqliteCache>`, so restructure so embedding happens before acquiring the lock, or stage into the existing connection in a blocking task). Same for query embedding in search handlers.
*Test:* a test that starts a slow refresh (stub embedder with a delay) and asserts `/api/note/...` still responds within a bound.

**F-04 — Double full-vault walk per MCP write.**
*Evidence:* `update_note_tool` et al. call `current_index(&state)` → `VaultIndex::build` (reads every note from disk), then `refresh_after_write` → `refresh_if_needed` → another full `VaultIndex::build`.
*Impact:* MCP write latency and IO scale with vault size, not change size. At hundreds/thousands of notes this becomes the dominant cost of every edit.
*Fix:* resolve the target `NoteEntry` from the SQLite cache (slug → path is already stored) instead of rebuilding the index; longer-term, make refresh incremental at the file level (the DB layer already skips unchanged notes by mtime/size/hash — it's the *walk + read* that's O(N)). Use the snapshot check before `fs::read_to_string` in `VaultIndex::build`.
*Test:* none needed for behavior; add a perf-guard test only if it regresses again.

**F-05 — Global connection mutex + sync DB in async handlers.**
*Evidence:* `SqliteCache { conn: Mutex<Connection> }` (`src/cache/mod.rs:17`); all query methods lock it; handlers call them inline.
*Impact:* all reads serialize; a slow query (e.g. `graph_data` on a big vault) stalls everything, and the blocked tasks each pin a runtime thread. For a single user this is invisible; it caps any multi-client future.
*Fix (when it matters):* WAL mode + a small read-connection pool, or route DB work through `spawn_blocking`. Not urgent at current scale.

**F-08 — Force checkout vs. uncommitted manual edits (needs verification).**
*Evidence:* `merge_remote` ends with `repo.checkout_head(CheckoutBuilder::new().force())` (`src/git/sync.rs:313`). `stage_and_commit` stages only the batch's `affected_paths`, so files edited by hand on the server are never committed by Hatchdoor.
*Scenario:* human edits `Home.md` directly in the server vault → an MCP write to another note triggers sync → remote happens to be ahead → clean merge → force checkout resets `Home.md` to HEAD, discarding the manual edit. The vault watcher will then reindex the *reverted* content, so the loss is silent.
*Fix:* before force checkout, refuse (or stash/commit-all) when `repo.statuses()` shows dirty tracked files outside the batch; or use a non-force checkout and surface conflicts as a `GitError`. At minimum document "do not hand-edit the server vault while git sync is enabled."
*Test:* yes — extend the existing excellent `sync.rs` test suite with: dirty untracked-by-batch file + remote ahead + clean merge → assert the manual edit survives (this test will fail today if the analysis is right; that's the verification).

**F-09 — SVG XSS (theoretical).**
*Fix:* serve `/vault-assets/*.svg` with `Content-Security-Policy: sandbox` or `Content-Disposition: attachment`, keeping `<img>` usage intact (images don't execute scripts; only direct navigation does).

**F-10 / F-11 / F-12 / F-16 / F-17:** small, mechanical fixes — generic 500 bodies with details only in logs; `targets.len()` cap (e.g. 200); move the archive prefix to config/env; log or surface `Lagged` by emitting the current revision; make `/health` do a `SELECT 1`.

---

## Architecture and structure

**Purpose:** self-hosted Obsidian-vault browser + retrieval backend, with agent (MCP) write access and git-based propagation of agent edits.

**Components and data flow:**
- `src/vault/` — filesystem source of truth: `VaultIndex::build` walks the vault; `vault/write/` implements all mutations (atomic writes, trash, backlink/asset rewrite plans, hardened path normalization in `write/paths.rs`).
- `src/cache/` — SQLite read model: notes, FTS5, headings, tags, links, chunks, sqlite-vec embeddings. Markdown stays canonical; the DB is disposable (schema-version-checked at startup, fail-fast).
- `src/chunk/`, `src/embed/`, `src/rerank/`, `src/search/` — retrieval pipeline (fastembed Nomic v1.5; semantic and keyword modes; per-note capping).
- `src/handlers/` — HTTP API + SPA + asset/download serving. `src/mcp/` — hand-rolled Streamable-HTTP MCP (POST only), read tools always, write tools gated by env + bearer.
- `src/git/` — debounced commit/push of MCP writes with fetch-merge-push, conflict-abort semantics, and a clean status surface. Cleanly layered (`config`/`sync`/`task`/`status`/`message`) with an injected runner for tests — the best module in the repo.
- `src/vault_watcher.rs` — recursive notify watcher with debounce, correctly ignoring `.git/` and SQLite sidecars.
- Concurrency model: `cache: Arc<RwLock<VaultCache>>` for swap-on-refresh, `vault_write_lock: Mutex<()>` shared between MCP write tools and git sync (held across whole tool calls — correct), `broadcast` channel → SSE for UI refresh.

**Architectural concerns:**
- The hand-rolled MCP protocol layer is a deliberate, defensible choice (no SDK dependency; strict schemas), but it means protocol-version churn is on you (`PROTOCOL_VERSION` is a single hardcoded const; clients sending the older `2025-06-18` get a hard 400 — consider accepting a compatible set).
- Per-request `McpConfig::from_env()` (F-07) means config has no single validated lifecycle; `AppConfig`, `McpConfig`, and `GitConfig` are three separate env-parsing idioms.
- `eval/` + `src/eval/` + `src/bin/eval.rs` (~2,200 lines) are research tooling compiled into the main crate behind `#[allow(dead_code)]` in places; fine, but consider a feature gate so the production binary doesn't carry it.

## Dependencies and tooling

- Backend deps are modern and lean (axum 0.8, rusqlite bundled, git2 0.20 vendored with TLS, notify 8, zip 2). No known-risky picks. **Verify** whether `blake3` and `ahash` are still actually used (F-18) — if not, drop them.
- Frontend: React 19, Vite 7, TS 5.9 — current. `mermaid` is the heavy one and is already lazy-loaded per the README.
- Toolchain pinned (1.96.0) and matched in the Dockerfile — good.
- **No CI** (F-13) is the biggest tooling gap given the quality checks are already scripted in the README. A Forgejo Actions workflow running `cargo fmt --check`, `clippy -D warnings`, `cargo test`, and the frontend `lint/typecheck/test/build` would be ~30 lines.
- No `LICENSE` file (fine for private, blocks any sharing).

## Testing

- **Strong:** 223 test functions; git sync has genuinely good behavioral tests (conflict-abort, stranded-commit, coalescing with paused time); MCP routes are tested end-to-end at the handler level including auth, origin, unknown-field rejection; path sanitization has direct traversal tests; vault write ops are covered.
- **Gaps:**
  1. The production env-driven config path is untested — tests inject `McpConfig` structs, so `from_env` parsing (truthy values, origin list splitting) has only partial coverage and the route handlers' env read is never exercised.
  2. No test covers F-08 (dirty working tree + merge).
  3. No concurrency tests: MCP write racing watcher refresh; concurrent `/api/refresh`.
  4. Real-embedder tests exist behind `embedder-tests` feature but nothing runs them (no CI).
  5. No load/limit tests (resolve-batch size, large note download zip).
- **Highest-value tests to add first:** the F-08 git test, an auth test on the *full router* once F-01 lands, and a "refresh doesn't block reads" test for F-03.

## Deployment and operations

- Docker is well done: cargo-chef layer caching, embedder weights prefetched at build, distroless `nonroot` runtime, pinned toolchain. `.env` is gitignored and not in history; `.env.example` is thorough and honest about security trade-offs.
- Missing: compose `healthcheck`, a meaningful `/health` (F-17), log rotation guidance (stdout is fine under Docker), and any backup story for the vault beyond git sync (the SQLite cache is explicitly disposable — good).
- `RUST_LOG` defaults are sensible; the git token is verifiably never logged (checked `sync.rs`/`task.rs` paths).

---

## Quick wins (small, low-risk)

1. Default `HOST=127.0.0.1` in `.env.example`; document `0.0.0.0` as opt-in (half of F-01).
2. Cap `resolve-batch` targets (F-11); add compose `healthcheck` + `SELECT 1` in `/health` (F-17).
3. Constant-time token compare (`subtle` crate or length+XOR fold) (F-06).
4. Parse `McpConfig` once at startup, store in `AppState`; fail startup on write-mode-without-token (F-07).
5. Generic 500 bodies; details to logs only (F-10).
6. Move `"90-archive/"` to an env var (F-12). Update `CHANGELOG.md` (F-15).
7. Rotate the bearer token sitting in `.claude/settings.local.json` if it's live (F-14).

## Larger refactors (do when the pain is felt)

1. **Async hygiene pass (F-03/F-05):** `spawn_blocking` around index build + embedding; restructure refresh so the write lock is held only for the swap; embed search queries off the runtime threads.
2. **Incremental reindex (F-04):** snapshot-check before reading file contents in `VaultIndex::build`; let MCP writes resolve entries from the cache instead of a fresh walk.
3. **Unified config module:** one validated, startup-time `Config` covering app/MCP/git.
4. **Git sync dirty-tree safety (F-08).**

## Security priorities (in order)

1. F-01 web auth (or documented reverse-proxy requirement) — everything else is secondary to this.
2. F-02 refresh auth/coalescing.
3. F-08 (it's integrity rather than confidentiality, but it's silent data loss).
4. F-09 SVG headers, F-06 constant-time compare, F-10 error hygiene.

## Testing priorities

1. Failing-first test for F-08 (dirty tree + clean merge).
2. Full-router auth tests once F-01 exists.
3. Non-blocking-refresh regression test for F-03.
4. `McpConfig::from_env` unit tests (truthy parsing, origins, token trimming).

## Questions / assumptions

- **Assumed deployment:** single user, LAN-only, behind the homelab network (private registry IP in compose). Severity of F-01/F-02 is "High if ever exposed"; if a VPN/reverse-proxy with auth already fronts this, downgrade both to Medium.
- **F-08 is analysis, not a reproduced bug** — the suggested test is the verification step.
- **Needs verification:** whether `blake3`/`ahash` are live dependencies; whether any MCP client you use sends protocol version `2025-06-18` (it would be rejected today).
- The token in `.claude/settings.local.json` was assumed possibly live; if it's a dev throwaway, ignore F-14.
