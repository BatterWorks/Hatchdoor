# Hatchdoor Architecture Layers

## Goal

Hatchdoor should act as a reliable knowledge/context layer for agents.

It should not include autonomous workflow orchestration for now.

---

## Layers

| Layer | Role | Status |
|---|---|---|
| **1. Source layer** | Holds the original content. | Implemented — markdown vault on disk. |
| **2. Parsing layer** | Extracts structure from the source. | Implemented — `src/cache/parse.rs`, `src/vault/`. |
| **3. Structured cache layer** | Stores metadata, relationships, state, and change tracking. | Implemented — SQLite at `data/cache/hatchdoor-cache.sqlite3`. |
| **4. Chunking layer** | Splits content into retrieval units. | Planned — Phase 1. `text-splitter` crate with the embedder's tokenizer, hybrid structural + size-cap (see Phase 1 decisions). |
| **5. Embedding layer** | Converts chunks into semantic representations. | Planned — Phase 1. Local in-process via `fastembed` + Snowflake Arctic Embed S (384-dim). Phase 1.5 eval drives any later upgrade. |
| **6. Semantic index layer** | Stores and searches semantic representations. | Planned — Phase 1. `sqlite-vec` extension on the existing SQLite file. |
| **7. Exact search layer** | Searches precise terms and metadata. | Implemented — SQLite FTS5 via `cache/queries.rs::search`. |
| **8. Hybrid retrieval layer** | Combines exact and semantic retrieval. | **Dropped from runtime path** — Phase 1.6 eval (see `docs/superpowers/specs/2026-05-19-phase-1.6-outcome.md`) found hybrid retrieval is net-negative vs pure semantic on the current corpus. Code remains in tree (`src/eval/hybrid_runner.rs`, `SqliteCache::fts_search_notes`, `eval hybrid` subcommand) for offline benchmarking only. |
| **9. Context assembly layer** | Prepares useful context for agents. | Planned — Phase 2. Bundles chunks + parent headings + linked notes. |
| **10. Tool interface layer** | Exposes Hatchdoor capabilities to agents. | Implemented — MCP server at `/mcp` (`src/mcp/`). |
| **11. Permission layer** | Controls read/write/modification rights. | Planned — Phase 5. Deferred until multi-agent or multi-tenant use is real. |
| **12. Reindexing layer** | Keeps cache and indexes synchronised. | Partially implemented — watcher does full reindex; Phase 3 makes it incremental and embedding-aware. |
| **13. Evaluation layer** | Measures retrieval quality. | Planned — Phase 1.5. Small harness with a labelled query set built from the real vault. |

---

## Core Split

| Category | Layers |
|---|---|
| **Source of truth** | Source layer, structured cache layer |
| **Derived indexes** | Chunking layer, embedding layer, semantic index layer, exact search layer |
| **Retrieval logic** | Semantic retrieval (Layer 6), context assembly layer |
| **Agent access and control** | Tool interface layer, permission layer |
| **Maintenance and quality** | Reindexing layer, evaluation layer |

---

## Out of Scope for Now

| Layer | Reason |
|---|---|
| **Workflow / orchestration layer** | This is the LangGraph-style layer. It coordinates multi-step agent processes, state, retries, approvals, branching, and long-running tasks. Not needed for the current Hatchdoor scope. |

---

## Current Scope

Hatchdoor should focus on:

```text
source content
→ structured cache
→ chunks
→ embeddings
→ semantic search (exact search remains a separate keyword path)
→ assembled agent context
→ MCP/tool interface
```

The goal is:

```text
Hatchdoor as a reliable knowledge/context MCP for agents.
```

---

## Implementation Phasing

The missing layers ship as a sequence of sub-projects, each with its own spec and plan. Ordering reflects dependency and value-per-unit-of-work.

| Phase | Layers | Outcome |
|---|---|---|
| **1. Semantic foundation** | 4, 5, 6 | Vault notes are chunked, embedded, and stored alongside existing FTS5. No new public API surface yet. |
| **1.5. Evaluation harness** | 13 | Labelled query set + scoring script so Phase 2 tuning is measurable. |
| **1.6. Retrieval-strategy bake-off** | 13 (extended) | Eval-only: compared pure semantic vs cross-encoder rerank vs hybrid RRF. Outcome: pure semantic wins; reranker and hybrid dropped from runtime. See `docs/superpowers/specs/2026-05-19-phase-1.6-outcome.md`. |
| **2. Semantic retrieval + context assembly** | 6 (runtime exposure), 9 | New MCP tool returns ranked, assembled context for an agent query. Pure semantic only — no FTS5 fusion, no cross-encoder rerank. |
| **3. Incremental reindexing** | 12 (full) | Watcher embeds only changed chunks instead of full reindex. |
| **4. Permission layer** | 11 | Read/write gating per agent or path. Deferred until needed. |

---

## Phase 1 Locked Decisions

Recorded here for downstream specs; full design lives in `docs/superpowers/specs/`.

- **Embedding model:** `fastembed-rs` with Snowflake Arctic Embed S, 384-dim `f32`. Runs in-process on CPU via ONNX Runtime, ~300 MB resident, no network or API dependency. Smallest retrieval-tuned model in fastembed-rs; chosen as a cheap starting point with Phase 1.5 eval driving any later upgrade (Arctic-M, mxbai-large) via a one-line `Embedder` swap plus a one-time reindex.
- **Vector storage:** `sqlite-vec` extension on the existing `hatchdoor-cache.sqlite3` file. Adds a `vec0` virtual table for chunk embeddings. No second datastore, no Chroma/Qdrant container.
- **Chunker implementation:** `text-splitter` crate configured with the embedder's tokenizer, so token counts in the chunker match token counts in the embedder.
- **Chunking strategy:** hybrid structural — split on H1/H2/H3 headings, sub-split sections over ~800 tokens with ~50-token overlap, fixed-size token-window fallback for oversized single paragraphs. Heading path stored as chunk metadata.
- **Pre-embed normalization:** strip YAML frontmatter and code-block fences (keep code contents). Keep wikilinks (`[[X]]`) literal. Lift frontmatter `tags`/`aliases` into a separate metadata column for SQL filtering rather than embedding them as text.
- **Deployment shape unchanged:** still a single Rust binary in Docker. Image grows ~200 MB for ONNX runtime + Arctic-S model weights.
- **Public API surface unchanged in Phase 1:** chunk/embed work is internal; consumers arrive in Phase 2.
