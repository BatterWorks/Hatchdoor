# Phase 2 — Pure Semantic Retrieval + Context Assembly

> Status: design. Supersedes the Phase 2 framing in `2026-05-18-semantic-foundation-design.md` (which assumed hybrid retrieval). Implements the scope declared in `2026-05-19-phase-1.6-outcome.md` ("What this changes about Phase 2").

## Goal

Expose pure semantic retrieval (Layer 6) as the runtime path of the MCP `search_notes` tool and the HTTP `/search` route, and bundle each hit with the assembled context an agent needs (parent headings, note metadata, outbound wikilinks). No FTS5 fusion. No cross-encoder rerank.

## Scope

In:

- Replace the `search_notes` MCP tool with a semantic-first, chunk-level retriever, with an explicit `mode = "keyword"` fallback that uses FTS5 over chunk content.
- Mirror the new shape on HTTP `GET /search`.
- A new `src/search/` module that orchestrates retrieve → per-note cap → assemble.
- A new `chunk_fts` FTS5 virtual table for keyword-mode chunk search.
- Frontend update if the web UI breaks on the new shape (same PR).

Out:

- Incremental reindexing (Phase 3).
- Permission layer (Phase 4).
- Bringing back hybrid retrieval or cross-encoder rerank at runtime (Phase 1.6 outcome).
- A `get_chunk(chunk_id)` tool — full chunk content is returned inline.

## Architecture

```text
                ┌─────────────────────────────────────┐
                │  MCP tools.rs (search_notes)        │
                │  HTTP handlers/search.rs (/search)  │
                └─────────────────┬───────────────────┘
                                  │   SearchRequest { query, mode, limit, per_note_cap }
                                  ▼
                  ┌─────────────────────────────────┐
                  │  src/search/mod.rs              │
                  │  pub fn run(...) -> Vec<Result> │
                  └───────┬─────────────────┬───────┘
                          │ retrieve        │ assemble
                          ▼                 ▼
            ┌─────────────────────┐  ┌──────────────────────────┐
            │ src/search/         │  │ src/search/              │
            │   retrieve.rs       │  │   assemble.rs            │
            │ (semantic/keyword + │  │ (batched link hydration) │
            │  per-note cap)      │  └─────────┬────────────────┘
            └─────────┬───────────┘            │
                      ▼                        ▼
        SqliteCache::semantic_search   SqliteCache::notes_with_outbound_links_batch (new)
        SqliteCache::fts_search_chunks (new — FTS5 against chunk content)
```

Three units, each with a single purpose:

- `search::retrieve` — pick a retriever, fetch raw chunk hits, apply the per-note cap. Knows nothing about link expansion.
- `search::assemble` — given a `Vec<ChunkHit>`, attach note metadata and outbound wikilinks in a single batched SQL pass. Knows nothing about retrieval.
- `search::run` — the orchestrator. Validates the request, calls retrieve, calls assemble, returns the response. The only thing MCP and HTTP need to call.

Touched files outside `src/search/`:

- `src/cache/queries.rs` — add `fts_search_chunks`, add `notes_with_outbound_links_batch`. `semantic_search` already exists and is unchanged.
- `src/cache/schema.rs` — add a migration creating `chunk_fts` (FTS5 virtual table content-synced to `chunks.content`) plus the standard insert/delete/update triggers.
- `src/mcp/tools.rs` — rewrite `search_notes_tool` to call `search::run`. Update tool schema (`mode`, `per_note_cap`; drop `include_content`).
- `src/handlers/search.rs` — switch to `search::run`; new response type.
- `src/api_types.rs` — `SearchRequest`, `SearchResult`, `SearchResponse` shared by MCP and HTTP.
- Frontend (`static/` / web UI): adjust the `/search` results renderer to the new chunk-level shape.

## Public API contract

### MCP `search_notes`

`inputSchema`:

```jsonc
{
  "query":        { "type": "string", "minLength": 1 },
  "mode":         { "type": "string", "enum": ["semantic", "keyword"], "default": "semantic" },
  "limit":        { "type": "integer", "minimum": 1, "maximum": 50, "default": 10 },
  "per_note_cap": { "type": "integer", "minimum": 1, "maximum": 10, "default": 2 }
}
// required: ["query"]
// additionalProperties: false
```

`include_content` is removed. Keyword mode now always runs FTS5 over chunk content.

### HTTP `GET /search`

Query params: `q` (required), `mode`, `limit`, `per_note_cap`. Same response body as MCP, wrapped in the existing JSON envelope. `limit` and `per_note_cap` are clamped to the declared ranges (matches the existing HTTP handler pattern of clamping rather than 400-ing on out-of-range values).

### Response (both modes, both transports)

```jsonc
{
  "mode": "semantic",
  "results": [
    {
      "chunk_id":     1234,
      "note_slug":    "projects/hatchdoor",
      "note_title":   "Hatchdoor",
      "note_path":    "Projects/Hatchdoor.md",
      "heading_path": "Architecture > Retrieval",
      "content":      "…full chunk text…",
      "score":        0.812,
      "outbound_links": [
        { "slug": "projects/fastembed", "title": "fastembed-rs" },
        { "slug": "ops/sqlite-vec",     "title": "sqlite-vec" }
      ]
    }
  ]
}
```

Contract:

- `score` is always "higher is better" regardless of mode, so callers don't branch on `mode` to interpret it.
  - Semantic: `1.0 - cosine_distance`, clamped to `[0.0, 1.0]`.
  - Keyword: BM25 inverted and normalized against the worst score in the result set, mapped into `(0.0, 1.0]`. Exact value is not a probability — just an ordering aid.
- `heading_path` uses ` > ` as separator, matching how the chunker writes it into `chunks.heading_path`. `null` if the chunk is pre-H1.
- `outbound_links` contains only wikilinks whose target resolves to a known slug. Dangling wikilinks are omitted (no nulls).
- `outbound_links` is `[]` if the note has none.
- `mode` is echoed in the response so callers know which retriever ran.
- Per-note cap is applied before `limit` truncation: `limit=10, per_note_cap=2` returns up to 10 chunks drawn from up to 10 distinct notes.

## Retrieval stage (`src/search/retrieve.rs`)

```rust
pub struct ChunkHit {
    pub chunk_id:     i64,
    pub note_slug:    String,
    pub heading_path: Option<String>,
    pub content:      String,
    pub score:        f32, // normalized, higher = better
}

pub fn retrieve(
    cache:    &SqliteCache,
    embedder: &dyn Embedder,
    req:      &SearchRequest,
) -> Result<Vec<ChunkHit>, String>;
```

Flow:

1. Compute `raw_k = (req.limit * req.per_note_cap).min(200)`. The min-cap of 200 is a defensive ceiling against pathological queries; the eval harness can tell us later whether it ever matters.
2. Dispatch by `mode`:
   - `semantic` → `cache.semantic_search(embedder, &req.query, raw_k)`. Score = `(1.0 - hit.distance).clamp(0.0, 1.0)`.
   - `keyword`  → `cache.fts_search_chunks(&req.query, raw_k)`. If `crate::cache::parse::build_fts_query(&req.query)` returns `None` (no usable tokens), return `Ok(vec![])` immediately. Score = BM25 inverted and normalized: for the worst (largest) BM25 `b_max` in the result set, `score = 1.0 - bm25 / b_max` then clamped to `(0.0, 1.0]`. If the result set has only one row, `score = 1.0`.
3. Apply the per-note cap in a single pass, preserving rank order:

   ```rust
   let mut seen: HashMap<String, usize> = HashMap::new();
   raw_hits
       .into_iter()
       .filter(|h| {
           let n = seen.entry(h.note_slug.clone()).or_insert(0);
           if *n < req.per_note_cap { *n += 1; true } else { false }
       })
       .take(req.limit)
       .collect()
   ```

4. Return the capped, ordered `Vec<ChunkHit>`.

### Why over-fetch

With `per_note_cap = 2` and `limit = 10`, if the top-20 chunks all came from one note we'd return 2 results despite a `limit` of 10. Over-fetching to `limit * per_note_cap` is the simplest hedge that still preserves rank order. The 200 ceiling prevents an attacker-or-bug-driven `limit=50, per_note_cap=10` from sweeping 500 chunks through the cap.

## Assembly stage (`src/search/assemble.rs`)

```rust
pub fn assemble(
    cache: &SqliteCache,
    hits:  Vec<ChunkHit>,
) -> Result<Vec<SearchResult>, String>;
```

Flow:

1. Collect distinct note slugs from `hits`, preserving first-seen order.
2. Single batched call: `cache.notes_with_outbound_links_batch(&slugs) -> HashMap<String, NoteWithLinks>`. Implementation under the hood:
   - Statement A: `SELECT slug, title, relative_path FROM notes WHERE slug IN (?, ?, …)`.
   - Statement B: `SELECT l.source_slug, l.target_slug, t.title FROM links l JOIN notes t ON t.slug = l.target_slug WHERE l.source_slug IN (?, ?, …)`.
   - Group statement B rows by `source_slug` in Rust. Drop any link rows whose target slug is missing from `notes` (statement B's `JOIN` already does this).
3. Stitch each hit with its note metadata and outbound links, preserving `hits` order. If a hit's slug is missing from the map (race with deletion), drop the hit and `tracing::warn!` once for that response.
4. Return `Vec<SearchResult>`.

Why eager and batched: with `limit ≤ 50` and `per_note_cap ≥ 1`, the response touches at most 50 distinct notes. Two prepared statements run in microseconds; per-row lookups would be the only way to lose. Eager assembly also means the caller never has to make a second round trip for the link context promised by the contract.

## Error handling

| Failure | Where | Caller-visible result |
|---|---|---|
| `query` empty after trim | MCP/HTTP arg validation | `invalid_params` JSON-RPC / HTTP 400 |
| `mode` outside enum | MCP/HTTP arg validation | `invalid_params` / HTTP 400 |
| Keyword query yields no FTS tokens (only stopwords / punctuation) | `retrieve.rs` | `{ "mode": "keyword", "results": [] }`. Not an error — same as "no matches". |
| Embedder failure (semantic) | `retrieve.rs` | Bubble as `internal` / HTTP 500. No silent fallback to keyword. |
| `chunk_vectors` empty (vault not yet embedded) | `retrieve.rs` | `results: []`. Legit pre-embed state. |
| sqlite-vec extension missing | startup | `internal` error at startup; not Phase 2's concern. |
| Race: note deleted between retrieve and assemble | `assemble.rs` | Drop the affected hit, `warn!` once, continue. Response may return `limit - 1` rows. |
| `chunk_fts` table missing | `fts_search_chunks` | `internal` error. The migration is part of Phase 2; if it didn't run, that's a bug, not a fallback condition. |
| `per_note_cap` / `limit` out of range | MCP: JSON-schema rejects (`invalid_params`). HTTP: clamp to declared range. | Matches each transport's existing convention. |

Logging follows existing `tracing` patterns: `info!` for each search request (query, mode, result count, elapsed ms), `warn!` for assembly skips, `error!` only for SQL or embedder failures that bubble out.

No silent retriever fallback. If semantic fails, the caller learns and can retry with `mode=keyword`. This matches the Phase 1.6 conclusion that hybrid logic is net-negative; we don't want failure-driven hybridization sneaking in.

## Schema migration

Add to `src/cache/schema.rs`:

```sql
CREATE VIRTUAL TABLE chunk_fts USING fts5(
    content,
    content='chunks',
    content_rowid='id',
    tokenize='unicode61'
);

-- Standard FTS5 sync triggers
CREATE TRIGGER chunk_fts_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO chunk_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER chunk_fts_ad AFTER DELETE ON chunks BEGIN
    INSERT INTO chunk_fts(chunk_fts, rowid, content) VALUES ('delete', old.id, old.content);
END;

CREATE TRIGGER chunk_fts_au AFTER UPDATE ON chunks BEGIN
    INSERT INTO chunk_fts(chunk_fts, rowid, content) VALUES ('delete', old.id, old.content);
    INSERT INTO chunk_fts(rowid, content) VALUES (new.id, new.content);
END;
```

Backfill: the migration is run by the existing schema runner; since `chunks` is rebuilt during the full reindex pipeline, the next reindex populates `chunk_fts` via the insert trigger. No one-shot backfill SQL needed beyond ensuring the migration runs before chunk inserts.

## Testing

Unit tests live with the modules they cover, following the existing `#[cfg(test)] mod` pattern in `cache/queries.rs`.

`cache::queries`:

- `fts_search_chunks_returns_hits_ordered_by_bm25`
- `fts_search_chunks_returns_empty_on_stopword_only_query`
- `notes_with_outbound_links_batch_groups_correctly` (multi-note, mix of resolved and unresolved targets)
- `notes_with_outbound_links_batch_handles_missing_slug` (returns partial map, not error)

`search::retrieve`:

- `per_note_cap_keeps_rank_order`
- `per_note_cap_zero_results_when_raw_empty`
- `semantic_score_is_one_minus_distance_clamped`
- `keyword_score_is_normalized_higher_better`

`search::assemble`:

- `assemble_preserves_hit_order`
- `assemble_drops_hits_whose_note_vanished` (race)
- `assemble_omits_dangling_wikilinks`

`search::run` (in-memory SQLite + test embedder):

- `semantic_path_end_to_end`
- `keyword_path_end_to_end`
- `over_fetch_compensates_for_single_note_flooding`

MCP-layer (`src/mcp/tools.rs` or `tests/`): one test per mode confirming the JSON shape, plus `invalid_params` cases for missing `query`, bad `mode`, and (MCP only) out-of-range `limit` / `per_note_cap`.

Eval harness: `cargo run --bin eval -- semantic` must show no regression versus the Phase 1.6 baseline (Recall@5 ≥ 0.968, MRR ≥ 0.92) before merge. Phase 2 doesn't change the retriever's quality, only its exposure — but running eval is the cheap check that confirms it.

Manual smoke before declaring done:

1. Refresh the index, then call `search_notes` with `mode=semantic` and `mode=keyword` via MCP.
2. Hit `GET /search?q=…&mode=semantic` in the browser and confirm the UI renders.
3. If the frontend renderer breaks on the new shape, fix it in the same PR — half-broken UI is not a Phase 2 outcome.

## Rollout

This is a breaking change to `search_notes`: removed `include_content`, new fields, chunk-level shape. Hatchdoor is single-user and the MCP consumer is under the same operator's control, so no deprecation window. Phase 1's semantic-foundation design explicitly said the API surface would be unchanged in Phase 1 and would change in Phase 2; this is that change.

Single PR, single merge. No feature flag — the semantic foundation has been in tree since Phase 1, and there's no consumer to gate.
