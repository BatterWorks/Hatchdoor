# Phase 1 — Semantic Foundation (Layers 4 → 5 → 6)

**Status:** Design approved, plan pending.
**Date:** 2026-05-18.
**Scope:** Implement the chunking, embedding, and semantic-index layers from `docs/hatchdoor-architecture-layers.md`. No new public HTTP routes, no MCP tool changes, no hybrid retrieval — those belong to Phase 2.

---

## 1. Goal

Turn the vault from a text-searchable cache into a semantically-searchable one. After Phase 1 ships, every note is chunked, every chunk has a 384-dim embedding, and both live in the existing SQLite cache. Phase 2 will then consume this to build a hybrid-retrieval MCP tool.

## 2. Locked technical decisions

| Area | Decision | Rationale |
|---|---|---|
| Embedding location | Local, in-process via `fastembed-rs` | Keeps single-binary deployment, no API cost, no network dependency. |
| Embedding model | Snowflake Arctic Embed M (`SnowflakeArcticEmbedM` in fastembed-rs), 768-dim `f32` | Highest English retrieval score among fastembed-rs models under 500M params. Trained specifically for retrieval, not general-purpose embedding. ~700 MB resident RAM, comfortable on the 4 vCPU / 4 GB VM. |
| Vector storage | `sqlite-vec` extension (`vec0` virtual table) inside existing `hatchdoor-cache.sqlite3` | One source of truth, atomic backup, makes Phase 2 hybrid retrieval a single SQL JOIN. No Chroma / Qdrant / second container. |
| Chunker | `text-splitter` crate, configured with the Arctic-M tokenizer | Markdown-aware, tokenizer-accurate splitting (same tokenizer as the embedder, so "800 tokens" means the same thing in both stages), recursive fallback through headings → paragraphs → sentences → words → chars. Avoids reimplementing a well-tested library. |
| Chunking strategy | Hybrid structural — split on H1/H2/H3 headings; sub-split sections over ~800 tokens on paragraph boundaries with ~50-token overlap; fall back to a fixed-size token window if a single paragraph itself exceeds the cap. Heading path preserved as metadata. | Respects author intent for small notes; bounds chunk size for long ones; guarantees no chunk exceeds the cap regardless of input shape. |
| Pre-embed normalization | Strip YAML frontmatter; strip code-block fences (keep code contents); keep wikilinks (`[[X]]`) literal; lift frontmatter `tags`/`aliases` to a separate metadata column. | Frontmatter degrades embeddings (structured-data noise). Code identifiers carry semantic value. Wikilinks left literal so Phase 2 context assembly can resolve them. Tags/aliases are first-class filter dimensions. |
| Deployment shape | Unchanged single Rust binary in Docker; image grows ~500 MB (ONNX runtime + Arctic-M weights). | No new operational moving parts. |
| Public API surface in Phase 1 | Unchanged | Phase 1 is additive infrastructure; consumers arrive in Phase 2. |

## 3. Module layout

Two new modules; three existing files touched.

| Path | Kind | Purpose |
|---|---|---|
| `src/chunk/mod.rs`, `src/chunk/chunker.rs` | NEW | Thin wrapper around `text-splitter::MarkdownSplitter` configured with the embedder's tokenizer. Handles normalization (frontmatter strip, code-fence strip, tags/aliases lift), then delegates the actual splitting. Returns ordered chunks with heading paths, byte ranges, and content hashes. No IO, no state. |
| `src/embed/mod.rs`, `src/embed/embedder.rs` | NEW | Owns the loaded `fastembed::TextEmbedding` (Arctic-M). Exposes an `Embedder` trait + concrete `ArcticEmbedder` and a test-only `StubEmbedder`. Held by `AppState` as `Arc<dyn Embedder>`. Also exposes the tokenizer so the chunker can borrow it. |
| `src/cache/schema.rs` | TOUCH | Add `chunks` table, indexes, and `chunk_vectors` virtual table. |
| `src/cache/populate.rs` | TOUCH | After inserting a note, call chunker, then embedder (in `spawn_blocking`), then write chunks + vectors in the same transaction. |
| `src/cache/queries.rs` | TOUCH | Add `chunks_for_note(slug)` and `chunk_count()` — minimal surface for Phase 1 tests; Phase 2 will add the hybrid retrieval queries. |
| `src/app_state.rs` | TOUCH | Hold the `Embedder` instance; create it once at startup. |
| `src/cache/mod.rs` | TOUCH | Load the `sqlite-vec` extension when opening the connection. |
| `Cargo.toml` | TOUCH | Add `fastembed`, `sqlite-vec`, `text-splitter`, `blake3` dependencies. |

The `Embedder` trait exists from day one (not introduced lazily) for one reason only: tests need a deterministic stub. It is not a hedge against backend swaps — that's YAGNI.

## 4. Data model

Two tables added to `hatchdoor-cache.sqlite3`. No changes to existing tables.

```sql
CREATE TABLE IF NOT EXISTS chunks (
  id           INTEGER PRIMARY KEY,
  note_slug    TEXT    NOT NULL,
  ordinal      INTEGER NOT NULL,
  heading_path TEXT,
  content      TEXT    NOT NULL,
  byte_start   INTEGER NOT NULL,
  byte_end     INTEGER NOT NULL,
  content_hash TEXT    NOT NULL,
  tags         TEXT,
  aliases      TEXT,
  FOREIGN KEY (note_slug) REFERENCES notes(slug) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_chunks_note_slug ON chunks(note_slug);
CREATE INDEX IF NOT EXISTS idx_chunks_content_hash ON chunks(content_hash);

CREATE VIRTUAL TABLE IF NOT EXISTS chunk_vectors USING vec0(
  chunk_id  INTEGER PRIMARY KEY,
  embedding FLOAT[768]
);
```

Design notes:

- `vec0` virtual tables cannot host arbitrary `TEXT` columns; the split into `chunks` + `chunk_vectors` joined on `chunk_id` is the sqlite-vec idiomatic pattern.
- SQLite `ON DELETE CASCADE` does not reach virtual tables. Before deleting a note's chunks, `populate.rs` explicitly deletes their vectors in the same transaction.
- `content_hash` (BLAKE3 of normalized chunk content) lets the indexer skip re-embedding unchanged chunks. This is the load-bearing column for incremental indexing — present from Phase 1 even though Phase 3 generalizes it further.
- `byte_start` / `byte_end` are not used in Phase 1; they avoid a schema migration when Phase 2 needs to highlight matching snippets.
- `tags` / `aliases` as JSON columns: idiomatic in SQLite, filterable via `json_each()`, fast enough at our scale (~1,500 chunks).

Estimated storage cost for the current vault (286 notes, ~1,500 chunks at 768-dim): ~7 MB added to the cache file.

## 5. Indexing flow

Three triggers, one code path. None are new.

1. **Cold start.** `main.rs` already calls `build_cache_with_sqlite`; this now also chunks + embeds every note. The HTTP server binds only after this completes.
2. **Watcher event.** `vault_watcher.rs` already triggers `refresh_if_needed`; the existing path now also re-chunks and re-embeds affected notes. Unchanged chunks skip the embedder via `content_hash`.
3. **Manual refresh.** `POST /api/refresh` behaves identically to the watcher path.

Per-note transaction:

```
BEGIN
  DELETE FROM chunk_vectors WHERE chunk_id IN (SELECT id FROM chunks WHERE note_slug = ?);
  DELETE FROM chunks WHERE note_slug = ?;
  -- chunker produces N chunks
  INSERT INTO chunks (...) RETURNING id;   -- N rows
  -- embedder (in spawn_blocking) produces N vectors in batches of up to 32
  INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?, ?);  -- N rows
COMMIT
```

All-or-nothing per note. A failed embed leaves the prior chunks intact.

Cold-start cost on the target VM (4 vCPU, Arctic Embed M): ~3 min for 1,500 chunks (one-time per fresh cache). Steady-state per-edit cost: ~100 ms for typical single-paragraph edits.

The decision to delay HTTP-server start until cold indexing completes is deliberate: Phase 2 will make `/api/search` route through the semantic index, and serving a half-built index would silently degrade quality. After first boot the cache file persists, so subsequent restarts are immediate.

## 6. Failure modes

| Failure | Behaviour |
|---|---|
| Embedding model fails to load at startup | Log error, exit non-zero. Hatchdoor will not run without an embedder. |
| Embedding call fails mid-batch | Per-note transaction rolls back, prior chunks retained, error logged. Watcher retries on next file event. |
| SQLite write fails | Existing `populate.rs` error path; surfaced as `500` from `/api/refresh`. |
| Note deleted from disk | Watcher fires; explicit pre-delete in the transaction removes its chunks and vectors. |
| `sqlite-vec` extension fails to load | Log error, exit non-zero. Caught at startup, not at first query. |

## 7. Testing strategy

Unit tests (pure, no IO):

- `chunk/chunker.rs` fixtures cover: small note, large note, heading-heavy, frontmatter-heavy, code-block-heavy, no-headings, single oversized paragraph. Assertions cover chunk count, boundaries, heading paths, byte ranges, hash determinism, and that frontmatter / code fences are stripped while wikilinks survive.
- `embed/embedder.rs`: a real-model test gated behind `cfg(feature = "embedder-tests")` asserting dim = 768, finite values, deterministic for identical input. Default `cargo test` uses the stub.

Integration tests (extend the existing `app_for_tests` pattern):

- In-memory SQLite + `StubEmbedder` (deterministic hash-based fake vectors). Exercises the full populate → chunks → vectors flow without the real model.
- Assertions: indexing a temp vault yields expected `chunks` and `chunk_vectors` row counts; deleting a note removes both; re-indexing an unchanged note triggers zero new embedding calls (the regression test for the `content_hash` skip path, per AGENTS.md §3).

Commands (per AGENTS.md §4):

- `cargo fmt && cargo check && cargo clippy && cargo test` — default, stub embedder, fast.
- `cargo test --features embedder-tests` — manual, loads real model, run before merging.

## 8. Out of scope for Phase 1

- Any new HTTP endpoint or MCP tool. The chunk + embed data is internal until Phase 2.
- Hybrid retrieval (Layer 8) and context assembly (Layer 9) — Phase 2.
- Note-level hash skipping in the watcher path — Phase 3.
- Multiple embedding backends, embedding versioning, model swaps — YAGNI.
- Permission gating on chunk content — Phase 5.

## 9. Open questions

None blocking. Two minor calls deferred to implementation:

- Exact batch size for `fastembed` (default 32, may tune by measurement).
- Whether `chunks_for_note` returns ordered by `ordinal` ASC at the SQL level or lets callers sort. Default: SQL.
