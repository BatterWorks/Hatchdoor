# Phase 2 — Semantic Retrieval + Context Assembly Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `search_notes` (MCP) and `GET /api/search` (HTTP) with a semantic-first, chunk-level retriever that returns assembled context (heading path + note metadata + outbound wikilinks); keep an explicit `mode=keyword` fallback that runs FTS5 over a new `chunk_fts` table.

**Architecture:** New `src/search/` module with three units — `retrieve.rs` (semantic via `sqlite-vec`, keyword via new `chunk_fts`, applies a per-note cap), `assemble.rs` (batched lookup of note metadata + outbound resolved wikilinks), `mod.rs` (orchestrator `run()` consumed by both MCP and HTTP). New SQLite migration to schema version 4 adds `chunk_fts` and its triggers.

**Tech Stack:** Rust, `rusqlite`, `sqlite-vec` (already vendored, used by `chunk_vectors`), FTS5, `axum`, `tokio`, `tracing`, MCP JSON-RPC. Test embedder is `crate::embed::StubEmbedder`.

**Spec:** `docs/superpowers/specs/2026-05-19-phase-2-design.md`

---

## File Map

**Create**
- `src/search/mod.rs` — orchestrator + public types (`SearchRequest`, `SearchMode`, `SearchResult`, `SearchResponse`, `OutboundLink`).
- `src/search/retrieve.rs` — `ChunkHit`, `retrieve()`.
- `src/search/assemble.rs` — `assemble()`.

**Modify**
- `src/cache/schema.rs` — bump `SCHEMA_VERSION` to `"4"`, add `chunk_fts` virtual table + triggers.
- `src/cache/queries.rs` — add `fts_search_chunks`, add `notes_with_outbound_links_batch` + `NoteWithLinks`.
- `src/lib.rs` — add `pub mod search;`.
- `src/api_types.rs` — replace `SearchQuery` and `SearchResponse` with the Phase 2 shapes.
- `src/handlers/api.rs` — rewrite `search_handler` to call `search::run`.
- `src/mcp/tools.rs` — rewrite `search_notes_tool` and its entry in `tools_list`.
- `frontend/src/types.ts` — replace `SearchHit` definition.
- `frontend/src/App.tsx` — update fetch params + result rendering hooks.
- `frontend/src/components/SearchDialog.tsx` — render the new shape.

**Not touched in this plan**
- `src/eval/*` — eval uses its own paths and benefits from the unchanged `semantic_search`.
- `src/cache/populate.rs` — chunk inserts hit the new `chunk_fts` trigger automatically; no code change.

---

## Conventions

- **TDD.** Every code-adding step writes the failing test first, runs it red, writes code, runs it green. Existing tests must continue to pass after each task.
- **Commits.** End every task with a single commit. Branch is `development` (per session decision; the saved "frontend bugs" memory was scoped to a prior session and has been removed).
- **Run tests via:** `cargo test -p hatchdoor` (workspace if relevant). For a single test: `cargo test -p hatchdoor <test_name> -- --nocapture`.
- **Frontend tests:** `cd frontend && npm test -- --run`.
- **Lints:** `cargo clippy --all-targets -- -D warnings` before each commit if you modified Rust.
- **Schema reset:** because Task 1 bumps the schema version, any local `data/cache/hatchdoor-cache.sqlite3` must be deleted before running the binary. Tests use `in_memory(...)` which builds fresh schema, so they are not affected.

---

## Task 1: Schema migration — add `chunk_fts` and bump to version 4

**Files**
- Modify: `src/cache/schema.rs`
- Test: `src/cache/schema.rs` (existing `mod tests`)

- [ ] **Step 1: Write the failing test**

Add at the bottom of `mod tests` in `src/cache/schema.rs`:

```rust
    #[test]
    fn fresh_cache_creates_chunk_fts_virtual_table() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let conn = cache.connection().expect("conn");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'chunk_fts'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(count, 1, "chunk_fts virtual table must exist");
    }

    #[test]
    fn fresh_cache_records_schema_version_4() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let conn = cache.connection().expect("conn");
        let version: String = conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(version, "4");
    }

    #[test]
    fn chunk_fts_insert_trigger_syncs_new_chunk_rows() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let conn = cache.connection().expect("conn");
        conn.execute(
            "INSERT INTO notes(slug, title, normalized_title, relative_path, normalized_relative_path, absolute_path, content, content_hash, mtime_ns, size_bytes, indexed_at) \
             VALUES ('n1','N1','n1','n1.md','n1.md','/tmp/n1.md','','h',0,0,0)",
            [],
        ).expect("insert note");
        conn.execute(
            "INSERT INTO chunks(note_slug, ordinal, heading_path, content, byte_start, byte_end, content_hash) \
             VALUES ('n1', 0, NULL, 'hello world', 0, 11, 'h0')",
            [],
        ).expect("insert chunk");
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunk_fts WHERE chunk_fts MATCH 'hello'",
                [],
                |row| row.get(0),
            )
            .expect("fts query");
        assert_eq!(hits, 1);
    }

    // Existing v3 test renames update — also update `fresh_cache_records_schema_version_3`:
    // delete it (replaced by `fresh_cache_records_schema_version_4`).
```

Also delete the existing test `fresh_cache_records_schema_version_3` (lines 198-210 in the current file). It is replaced by `fresh_cache_records_schema_version_4` above.

- [ ] **Step 2: Run tests, expect failure**

```bash
cargo test -p hatchdoor --lib cache::schema::tests
```

Expected: 2 failures (`fresh_cache_creates_chunk_fts_virtual_table`, `chunk_fts_insert_trigger_syncs_new_chunk_rows`) and 1 fail on the new `fresh_cache_records_schema_version_4`.

- [ ] **Step 3: Bump `SCHEMA_VERSION`**

In `src/cache/schema.rs` line 5:

```rust
const SCHEMA_VERSION: &str = "4";
```

And in `create_schema` at the bottom of the SQL block (line 159-161), change the literal:

```rust
        INSERT INTO metadata(key, value)
        VALUES ('schema_version', '4')
        ON CONFLICT(key) DO NOTHING;
```

- [ ] **Step 4: Add `chunk_fts` table + triggers to `create_schema`**

In `src/cache/schema.rs`, append the following SQL to the format string in `create_schema` (after the `CREATE VIRTUAL TABLE IF NOT EXISTS chunk_vectors ...` block, before the `INSERT INTO metadata` line):

```sql
        CREATE VIRTUAL TABLE IF NOT EXISTS chunk_fts USING fts5(
            content,
            content='chunks',
            content_rowid='id',
            tokenize='unicode61 remove_diacritics 2'
        );

        CREATE TRIGGER IF NOT EXISTS chunk_fts_ai AFTER INSERT ON chunks BEGIN
            INSERT INTO chunk_fts(rowid, content) VALUES (new.id, new.content);
        END;

        CREATE TRIGGER IF NOT EXISTS chunk_fts_ad AFTER DELETE ON chunks BEGIN
            INSERT INTO chunk_fts(chunk_fts, rowid, content) VALUES ('delete', old.id, old.content);
        END;

        CREATE TRIGGER IF NOT EXISTS chunk_fts_au AFTER UPDATE ON chunks BEGIN
            INSERT INTO chunk_fts(chunk_fts, rowid, content) VALUES ('delete', old.id, old.content);
            INSERT INTO chunk_fts(rowid, content) VALUES (new.id, new.content);
        END;
```

The `tokenize='unicode61 remove_diacritics 2'` matches `note_fts` to keep the user-facing tokenization consistent.

- [ ] **Step 5: Run tests, expect pass**

```bash
cargo test -p hatchdoor --lib cache::schema::tests
```

Expected: all schema tests pass.

- [ ] **Step 6: Run the full test suite**

```bash
cargo test -p hatchdoor
```

Expected: PASS. Some existing semantic-search / FTS tests may have relied on absence of `chunk_fts`; this is purely additive so they should still pass.

- [ ] **Step 7: Delete any local cache before running the binary again**

Document for the reviewer (no code change). The binary's `ensure_schema` will refuse to start against a v3 cache. The error message it prints already tells the operator to delete the cache.

- [ ] **Step 8: Commit**

```bash
git add src/cache/schema.rs
git commit -m "feat(cache): add chunk_fts FTS5 table and bump schema to v4

Adds a chunk-level FTS5 virtual table with content/delete/update
triggers that keep it in sync with the chunks table. Required for
Phase 2 keyword-mode chunk search.

Schema version bumped to 4: any existing on-disk cache must be
deleted and rebuilt (the existing migration runner already prints
that instruction)."
```

---

## Task 2: `fts_search_chunks` SQL function

**Files**
- Modify: `src/cache/queries.rs`
- Test: same file, in a new `#[cfg(test)] mod fts_search_chunks_tests`

- [ ] **Step 1: Write the failing tests**

Append at the bottom of `src/cache/queries.rs`:

```rust
#[cfg(test)]
mod fts_search_chunks_tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;

    fn vault_with(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        dir
    }

    fn build_cache(files: &[(&str, &str)]) -> SqliteCache {
        let dir = vault_with(files);
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        cache
    }

    #[test]
    fn fts_search_chunks_returns_hits_ordered_by_bm25() {
        let cache = build_cache(&[
            ("a.md", "# Apples\n\napples and oranges grow on trees"),
            ("b.md", "# Bicycles\n\nspokes and wheels"),
        ]);
        let hits = cache.fts_search_chunks("apples", 10).expect("search");
        assert!(!hits.is_empty(), "expected at least one hit");
        // BM25 ascending: best match first; first hit should come from a.md.
        assert!(hits[0].note_slug.contains('a') || hits[0].content.contains("apples"));
        for w in hits.windows(2) {
            assert!(w[0].bm25 <= w[1].bm25, "bm25 must be non-decreasing");
        }
    }

    #[test]
    fn fts_search_chunks_returns_empty_on_stopword_only_query() {
        let cache = build_cache(&[("a.md", "# A\n\nbody text")]);
        let hits = cache.fts_search_chunks("   .  ", 10).expect("search");
        assert!(hits.is_empty());
    }

    #[test]
    fn fts_search_chunks_respects_limit() {
        let cache = build_cache(&[
            ("a.md", "# A\n\napples"),
            ("b.md", "# B\n\napples"),
            ("c.md", "# C\n\napples"),
        ]);
        let hits = cache.fts_search_chunks("apples", 2).expect("search");
        assert_eq!(hits.len(), 2);
    }
}
```

- [ ] **Step 2: Run tests, expect failure**

```bash
cargo test -p hatchdoor --lib cache::queries::fts_search_chunks_tests
```

Expected: compilation error — `fts_search_chunks` not found.

- [ ] **Step 3: Add the public struct and function**

Add to `src/cache/queries.rs` right after the existing `SemanticHit` struct definition (around line 386):

```rust
#[derive(Debug, Clone)]
pub struct ChunkFtsHit {
    pub chunk_id: i64,
    pub note_slug: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub bm25: f32,
}
```

Then in the `impl SqliteCache` block (next to `fts_search_notes`, after `semantic_search`), add:

```rust
    pub fn fts_search_chunks(
        &self,
        query: &str,
        k: usize,
    ) -> Result<Vec<ChunkFtsHit>, String> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let Some(fts_q) = build_fts_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT c.id, c.note_slug, c.heading_path, c.content, bm25(chunk_fts)
                FROM chunk_fts
                JOIN chunks c ON c.id = chunk_fts.rowid
                WHERE chunk_fts MATCH ?1
                ORDER BY bm25(chunk_fts)
                LIMIT ?2
                "#,
            )
            .map_err(|e| format!("prepare fts_search_chunks: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params![fts_q, k as i64], |row| {
                Ok(ChunkFtsHit {
                    chunk_id: row.get(0)?,
                    note_slug: row.get(1)?,
                    heading_path: row.get(2)?,
                    content: row.get(3)?,
                    bm25: row.get::<_, f64>(4)? as f32,
                })
            })
            .map_err(|e| format!("query fts_search_chunks: {e}"))?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row.map_err(|e| format!("read fts_search_chunks row: {e}"))?);
        }
        Ok(hits)
    }
```

- [ ] **Step 4: Run tests, expect pass**

```bash
cargo test -p hatchdoor --lib cache::queries::fts_search_chunks_tests
```

Expected: PASS.

- [ ] **Step 5: Run the full test suite + clippy**

```bash
cargo test -p hatchdoor
cargo clippy --all-targets -- -D warnings
```

Expected: PASS, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/cache/queries.rs
git commit -m "feat(cache): add fts_search_chunks for chunk-level BM25 search

Chunk-level analogue of fts_search_notes. Returns ChunkFtsHit rows
ordered by bm25 ascending, ready for score normalization in the
search retrieve stage."
```

---

## Task 3: `notes_with_outbound_links_batch` SQL function

**Files**
- Modify: `src/cache/queries.rs`
- Test: same file, new `#[cfg(test)] mod notes_with_outbound_links_batch_tests`

- [ ] **Step 1: Write the failing tests**

Append to `src/cache/queries.rs`:

```rust
#[cfg(test)]
mod notes_with_outbound_links_batch_tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;

    fn vault_with(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        dir
    }

    fn build_cache(files: &[(&str, &str)]) -> SqliteCache {
        let dir = vault_with(files);
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        cache
    }

    #[test]
    fn batch_returns_note_metadata_for_each_slug() {
        let cache = build_cache(&[
            ("Alpha.md", "# Alpha\n\nbody"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let map = cache
            .notes_with_outbound_links_batch(&["alpha".to_string(), "bravo".to_string()])
            .expect("batch");
        assert_eq!(map.len(), 2);
        let a = map.get("alpha").expect("alpha");
        assert_eq!(a.title, "Alpha");
        assert_eq!(a.relative_path, "Alpha.md");
        assert!(a.outbound_links.is_empty());
    }

    #[test]
    fn batch_returns_resolved_outbound_links_only() {
        let cache = build_cache(&[
            ("Alpha.md", "# Alpha\n\nlinks to [[Bravo]] and [[Ghost]]"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let map = cache
            .notes_with_outbound_links_batch(&["alpha".to_string()])
            .expect("batch");
        let a = map.get("alpha").expect("alpha");
        assert_eq!(a.outbound_links.len(), 1);
        assert_eq!(a.outbound_links[0].slug, "bravo");
        assert_eq!(a.outbound_links[0].title, "Bravo");
    }

    #[test]
    fn batch_omits_missing_slugs() {
        let cache = build_cache(&[("Alpha.md", "# Alpha\n\nbody")]);
        let map = cache
            .notes_with_outbound_links_batch(&["alpha".to_string(), "ghost".to_string()])
            .expect("batch");
        assert!(map.contains_key("alpha"));
        assert!(!map.contains_key("ghost"));
    }

    #[test]
    fn batch_empty_input_returns_empty_map() {
        let cache = build_cache(&[("Alpha.md", "# Alpha\n\nbody")]);
        let map = cache
            .notes_with_outbound_links_batch(&[])
            .expect("batch");
        assert!(map.is_empty());
    }
}
```

- [ ] **Step 2: Run tests, expect failure**

```bash
cargo test -p hatchdoor --lib cache::queries::notes_with_outbound_links_batch_tests
```

Expected: compilation error.

- [ ] **Step 3: Add types and function**

Add to `src/cache/queries.rs` after the `ChunkFtsHit` struct:

```rust
#[derive(Debug, Clone)]
pub struct OutboundLinkRow {
    pub slug: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct NoteWithLinks {
    pub slug: String,
    pub title: String,
    pub relative_path: String,
    pub outbound_links: Vec<OutboundLinkRow>,
}
```

Make sure `HashMap` is imported (`std::collections::HashMap`). It is not already in this file's imports — add it to the top-of-file `use` block:

```rust
use std::collections::{BTreeMap, HashMap, HashSet};
```

Then in the `impl SqliteCache` block, add:

```rust
    pub fn notes_with_outbound_links_batch(
        &self,
        slugs: &[String],
    ) -> Result<HashMap<String, NoteWithLinks>, String> {
        if slugs.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.connection()?;

        let placeholders = std::iter::repeat("?")
            .take(slugs.len())
            .collect::<Vec<_>>()
            .join(",");

        let mut map: HashMap<String, NoteWithLinks> = HashMap::new();

        // Note metadata
        let sql_a = format!(
            "SELECT slug, title, relative_path FROM notes WHERE slug IN ({placeholders})"
        );
        let mut stmt_a = conn
            .prepare(&sql_a)
            .map_err(|e| format!("prepare notes batch: {e}"))?;
        let rows_a = stmt_a
            .query_map(rusqlite::params_from_iter(slugs.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("query notes batch: {e}"))?;
        for row in rows_a {
            let (slug, title, relative_path) =
                row.map_err(|e| format!("read notes batch row: {e}"))?;
            map.insert(
                slug.clone(),
                NoteWithLinks {
                    slug,
                    title,
                    relative_path,
                    outbound_links: Vec::new(),
                },
            );
        }

        // Outbound links (only resolved targets — JOIN drops danglers)
        let sql_b = format!(
            "SELECT l.source_slug, t.slug, t.title \
             FROM note_links l \
             JOIN notes t ON t.slug = l.target_slug \
             WHERE l.source_slug IN ({placeholders}) \
             ORDER BY l.source_slug, t.relative_path"
        );
        let mut stmt_b = conn
            .prepare(&sql_b)
            .map_err(|e| format!("prepare links batch: {e}"))?;
        let rows_b = stmt_b
            .query_map(rusqlite::params_from_iter(slugs.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("query links batch: {e}"))?;
        for row in rows_b {
            let (source_slug, target_slug, target_title) =
                row.map_err(|e| format!("read links batch row: {e}"))?;
            if let Some(entry) = map.get_mut(&source_slug) {
                entry.outbound_links.push(OutboundLinkRow {
                    slug: target_slug,
                    title: target_title,
                });
            }
        }

        Ok(map)
    }
```

- [ ] **Step 4: Run tests, expect pass**

```bash
cargo test -p hatchdoor --lib cache::queries::notes_with_outbound_links_batch_tests
```

Expected: PASS.

- [ ] **Step 5: Run the full test suite + clippy**

```bash
cargo test -p hatchdoor
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/cache/queries.rs
git commit -m "feat(cache): add notes_with_outbound_links_batch

Two prepared SQL statements (notes metadata + outbound links via
JOIN that drops dangling targets) grouped in Rust into a slug-keyed
HashMap. Used by Phase 2 context assembly to hydrate a search
response in a single batched pass."
```

---

## Task 4: Scaffold `src/search/` module + register in `lib.rs`

**Files**
- Create: `src/search/mod.rs`, `src/search/retrieve.rs`, `src/search/assemble.rs`
- Modify: `src/lib.rs`

This task just lays out the module skeleton with stub types. No tests yet — those come task-by-task as functions are implemented.

- [ ] **Step 1: Create `src/search/retrieve.rs` with the `ChunkHit` type**

```rust
//! Phase 2 retrieval stage. Dispatches by mode, applies the per-note cap.

use crate::cache::SqliteCache;
use crate::embed::Embedder;

use super::SearchRequest;

#[derive(Debug, Clone)]
pub struct ChunkHit {
    pub chunk_id: i64,
    pub note_slug: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub score: f32, // normalized: higher = better
}

pub fn retrieve(
    _cache: &SqliteCache,
    _embedder: &dyn Embedder,
    _req: &SearchRequest,
) -> Result<Vec<ChunkHit>, String> {
    unimplemented!("filled in by later tasks")
}
```

- [ ] **Step 2: Create `src/search/assemble.rs` with stub**

```rust
//! Phase 2 context assembly stage.

use crate::cache::SqliteCache;

use super::{ChunkHit, SearchResult};

pub fn assemble(
    _cache: &SqliteCache,
    _hits: Vec<ChunkHit>,
) -> Result<Vec<SearchResult>, String> {
    unimplemented!("filled in by later tasks")
}
```

- [ ] **Step 3: Create `src/search/mod.rs` with types + orchestrator stub**

```rust
//! Phase 2 search orchestrator. Consumed by both MCP and HTTP.

use serde::{Deserialize, Serialize};

use crate::cache::SqliteCache;
use crate::embed::Embedder;

pub mod assemble;
pub mod retrieve;

pub use retrieve::ChunkHit;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    Semantic,
    Keyword,
}

impl Default for SearchMode {
    fn default() -> Self {
        SearchMode::Semantic
    }
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: String,
    pub mode: SearchMode,
    pub limit: usize,
    pub per_note_cap: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutboundLink {
    pub slug: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub chunk_id: i64,
    pub note_slug: String,
    pub note_title: String,
    pub note_path: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub score: f32,
    pub outbound_links: Vec<OutboundLink>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub mode: SearchMode,
    pub results: Vec<SearchResult>,
}

pub fn run(
    _cache: &SqliteCache,
    _embedder: &dyn Embedder,
    _req: SearchRequest,
) -> Result<SearchResponse, String> {
    unimplemented!("filled in by later tasks")
}
```

- [ ] **Step 4: Register the module in `src/lib.rs`**

Open `src/lib.rs` and add (alongside the other `pub mod` lines, alphabetical neighbours):

```rust
pub mod search;
```

- [ ] **Step 5: Run `cargo build`**

```bash
cargo build -p hatchdoor
```

Expected: builds cleanly. The `unimplemented!()` stubs are never called yet.

- [ ] **Step 6: Run tests + clippy**

```bash
cargo test -p hatchdoor
cargo clippy --all-targets -- -D warnings
```

Expected: PASS. `unimplemented!()` does not trip clippy.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/search/
git commit -m "feat(search): scaffold Phase 2 search module

Lays out SearchRequest/SearchMode/SearchResult/SearchResponse types
and three submodules (mod.rs orchestrator, retrieve.rs, assemble.rs)
as stubs. Filled in by subsequent tasks."
```

---

## Task 5: Implement `retrieve` — semantic mode + score normalization

**Files**
- Modify: `src/search/retrieve.rs`
- Test: `src/search/retrieve.rs`

- [ ] **Step 1: Write the failing tests**

Replace the body of `src/search/retrieve.rs` so the file ends with:

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::search::{SearchMode, SearchRequest};
    use crate::vault::VaultIndex;

    use super::retrieve;

    fn build_cache(files: &[(&str, &str)]) -> (SqliteCache, Arc<dyn Embedder>) {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        (cache, embedder)
    }

    #[test]
    fn semantic_mode_returns_hits_ordered_by_score_desc() {
        let (cache, embedder) = build_cache(&[
            ("a.md", "# A\n\napples and oranges"),
            ("b.md", "# B\n\nspokes and wheels"),
        ]);
        let req = SearchRequest {
            query: "apples".to_string(),
            mode: SearchMode::Semantic,
            limit: 10,
            per_note_cap: 2,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert!(!hits.is_empty());
        for w in hits.windows(2) {
            assert!(w[0].score >= w[1].score, "scores must be non-increasing");
        }
        for h in &hits {
            assert!(h.score >= 0.0 && h.score <= 1.0, "score out of range: {}", h.score);
        }
    }

    #[test]
    fn semantic_mode_returns_empty_when_cache_has_no_chunks() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let req = SearchRequest {
            query: "anything".to_string(),
            mode: SearchMode::Semantic,
            limit: 10,
            per_note_cap: 2,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert!(hits.is_empty());
    }
}
```

- [ ] **Step 2: Run tests, expect failure**

```bash
cargo test -p hatchdoor --lib search::retrieve::tests
```

Expected: panic from `unimplemented!()`.

- [ ] **Step 3: Implement `retrieve` for semantic mode**

Replace `src/search/retrieve.rs` (keep the test module at the bottom) with:

```rust
//! Phase 2 retrieval stage. Dispatches by mode, applies the per-note cap.

use std::collections::HashMap;

use crate::cache::SqliteCache;
use crate::embed::Embedder;

use super::{SearchMode, SearchRequest};

const RAW_K_CEILING: usize = 200;

#[derive(Debug, Clone)]
pub struct ChunkHit {
    pub chunk_id: i64,
    pub note_slug: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub score: f32, // normalized: higher = better
}

pub fn retrieve(
    cache: &SqliteCache,
    embedder: &dyn Embedder,
    req: &SearchRequest,
) -> Result<Vec<ChunkHit>, String> {
    let raw_k = (req.limit.saturating_mul(req.per_note_cap)).min(RAW_K_CEILING);
    if raw_k == 0 {
        return Ok(Vec::new());
    }

    let raw_hits: Vec<ChunkHit> = match req.mode {
        SearchMode::Semantic => semantic(cache, embedder, &req.query, raw_k)?,
        SearchMode::Keyword => return Err(
            "keyword mode not implemented yet".to_string(),
        ),
    };

    Ok(apply_per_note_cap(raw_hits, req.per_note_cap, req.limit))
}

fn semantic(
    cache: &SqliteCache,
    embedder: &dyn Embedder,
    query: &str,
    k: usize,
) -> Result<Vec<ChunkHit>, String> {
    let hits = cache.semantic_search(embedder, query, k)?;
    Ok(hits
        .into_iter()
        .map(|h| ChunkHit {
            chunk_id: h.chunk_id,
            note_slug: h.note_slug,
            heading_path: h.heading_path,
            content: h.content,
            score: (1.0 - h.distance).clamp(0.0, 1.0),
        })
        .collect())
}

fn apply_per_note_cap(
    raw: Vec<ChunkHit>,
    per_note_cap: usize,
    limit: usize,
) -> Vec<ChunkHit> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(limit.min(raw.len()));
    for h in raw {
        let n = seen.entry(h.note_slug.clone()).or_insert(0);
        if *n < per_note_cap {
            *n += 1;
            out.push(h);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    // (keep tests from Step 1 here)
}
```

(Paste the Step 1 test module verbatim under the `#[cfg(test)] mod tests` placeholder.)

- [ ] **Step 4: Run tests, expect pass**

```bash
cargo test -p hatchdoor --lib search::retrieve::tests
```

Expected: PASS for both semantic tests.

- [ ] **Step 5: Run full test suite + clippy**

```bash
cargo test -p hatchdoor
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/search/retrieve.rs
git commit -m "feat(search): implement semantic retrieve + per-note cap

Wraps SqliteCache::semantic_search, normalizes cosine distance to
'higher is better' score in [0,1], and applies the per-note cap.
Keyword mode still stubbed."
```

---

## Task 6: Implement `retrieve` — keyword mode

**Files**
- Modify: `src/search/retrieve.rs`
- Test: `src/search/retrieve.rs`

- [ ] **Step 1: Add failing tests for keyword mode**

In the `#[cfg(test)] mod tests` block of `src/search/retrieve.rs`, add:

```rust
    #[test]
    fn keyword_mode_returns_hits_with_normalized_scores() {
        let (cache, embedder) = build_cache(&[
            ("a.md", "# A\n\napples and oranges"),
            ("b.md", "# B\n\noranges only"),
            ("c.md", "# C\n\nbananas"),
        ]);
        let req = SearchRequest {
            query: "oranges".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 2,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert!(!hits.is_empty());
        for w in hits.windows(2) {
            assert!(w[0].score >= w[1].score, "scores must be non-increasing");
        }
        for h in &hits {
            assert!(h.score > 0.0 && h.score <= 1.0, "score out of range: {}", h.score);
        }
        // bananas chunk should NOT be in keyword results for "oranges"
        assert!(!hits.iter().any(|h| h.content.contains("bananas")));
    }

    #[test]
    fn keyword_mode_returns_empty_when_query_has_no_tokens() {
        let (cache, embedder) = build_cache(&[("a.md", "# A\n\nbody")]);
        let req = SearchRequest {
            query: "   ".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 2,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert!(hits.is_empty());
    }

    #[test]
    fn keyword_mode_single_result_gets_max_score() {
        let (cache, embedder) = build_cache(&[
            ("a.md", "# A\n\nuniquetoken-xyzzy"),
            ("b.md", "# B\n\nirrelevant"),
        ]);
        let req = SearchRequest {
            query: "uniquetoken-xyzzy".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 2,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert_eq!(hits.len(), 1);
        assert!((hits[0].score - 1.0).abs() < f32::EPSILON);
    }
```

- [ ] **Step 2: Run, expect failure**

```bash
cargo test -p hatchdoor --lib search::retrieve::tests::keyword
```

Expected: failure (still returns `Err("keyword mode not implemented yet")`).

- [ ] **Step 3: Implement keyword mode**

In `src/search/retrieve.rs`, replace the `match req.mode { ... }` arm and add a `keyword` helper:

```rust
    let raw_hits: Vec<ChunkHit> = match req.mode {
        SearchMode::Semantic => semantic(cache, embedder, &req.query, raw_k)?,
        SearchMode::Keyword => keyword(cache, &req.query, raw_k)?,
    };
```

And below the existing `semantic` helper:

```rust
fn keyword(
    cache: &SqliteCache,
    query: &str,
    k: usize,
) -> Result<Vec<ChunkHit>, String> {
    let hits = cache.fts_search_chunks(query, k)?;
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    // BM25 ascending (lower = better). Normalize to (0.0, 1.0] where higher is better.
    // Single-row case: assign 1.0 to avoid division-by-zero and to give the lone hit the
    // strongest possible score.
    let b_max = hits
        .iter()
        .map(|h| h.bm25.abs())
        .fold(f32::MIN, f32::max);
    Ok(hits
        .into_iter()
        .map(|h| {
            let raw = if b_max <= f32::EPSILON {
                1.0
            } else {
                (1.0 - (h.bm25.abs() / b_max)).clamp(0.0, 1.0)
            };
            // Single-row sets, or sets where all BM25 values are identical, collapse to 0.
            // Bump those to 1.0 so the caller still gets a positive score.
            let score = if raw <= f32::EPSILON { 1.0 } else { raw };
            ChunkHit {
                chunk_id: h.chunk_id,
                note_slug: h.note_slug,
                heading_path: h.heading_path,
                content: h.content,
                score,
            }
        })
        .collect())
}
```

Note: BM25 from FTS5 is typically negative (more negative = better). The `.abs()` normalizes the sign so the formula works for both conventions.

- [ ] **Step 4: Run keyword tests, expect pass**

```bash
cargo test -p hatchdoor --lib search::retrieve::tests
```

Expected: PASS for all keyword tests as well as the prior semantic ones.

- [ ] **Step 5: Run full suite + clippy**

```bash
cargo test -p hatchdoor
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/search/retrieve.rs
git commit -m "feat(search): implement keyword retrieve via chunk FTS5

Wraps fts_search_chunks and normalizes BM25 into a 'higher is
better' score. Single-row and flat-BM25 result sets collapse to
1.0 to keep the contract that every returned hit has a positive
score."
```

---

## Task 7: Implement `assemble`

**Files**
- Modify: `src/search/assemble.rs`
- Test: `src/search/assemble.rs`

- [ ] **Step 1: Write the failing tests**

Replace `src/search/assemble.rs` with:

```rust
//! Phase 2 context assembly stage.

use crate::cache::SqliteCache;

use super::{ChunkHit, OutboundLink, SearchResult};

pub fn assemble(
    cache: &SqliteCache,
    hits: Vec<ChunkHit>,
) -> Result<Vec<SearchResult>, String> {
    if hits.is_empty() {
        return Ok(Vec::new());
    }

    // Preserve first-seen order so we re-attach in stable order later.
    let mut distinct_slugs: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for h in &hits {
        if seen.insert(h.note_slug.clone()) {
            distinct_slugs.push(h.note_slug.clone());
        }
    }

    let metadata = cache.notes_with_outbound_links_batch(&distinct_slugs)?;

    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        let Some(note) = metadata.get(&h.note_slug) else {
            tracing::warn!(
                slug = %h.note_slug,
                "search.assemble: dropping hit whose note vanished between retrieve and assemble"
            );
            continue;
        };
        out.push(SearchResult {
            chunk_id: h.chunk_id,
            note_slug: h.note_slug,
            note_title: note.title.clone(),
            note_path: note.relative_path.clone(),
            heading_path: h.heading_path,
            content: h.content,
            score: h.score,
            outbound_links: note
                .outbound_links
                .iter()
                .map(|l| OutboundLink {
                    slug: l.slug.clone(),
                    title: l.title.clone(),
                })
                .collect(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::search::ChunkHit;
    use crate::vault::VaultIndex;

    use super::assemble;

    fn build_cache(files: &[(&str, &str)]) -> SqliteCache {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        cache
    }

    #[test]
    fn preserves_hit_order() {
        let cache = build_cache(&[
            ("Alpha.md", "# Alpha\n\nbody"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let hits = vec![
            ChunkHit {
                chunk_id: 1,
                note_slug: "bravo".to_string(),
                heading_path: None,
                content: "b body".to_string(),
                score: 0.9,
            },
            ChunkHit {
                chunk_id: 2,
                note_slug: "alpha".to_string(),
                heading_path: None,
                content: "a body".to_string(),
                score: 0.8,
            },
        ];
        let out = assemble(&cache, hits).expect("assemble");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].note_slug, "bravo");
        assert_eq!(out[1].note_slug, "alpha");
    }

    #[test]
    fn drops_hits_whose_note_vanished() {
        let cache = build_cache(&[("Alpha.md", "# Alpha\n\nbody")]);
        let hits = vec![
            ChunkHit {
                chunk_id: 1,
                note_slug: "alpha".to_string(),
                heading_path: None,
                content: "a".to_string(),
                score: 0.9,
            },
            ChunkHit {
                chunk_id: 2,
                note_slug: "ghost".to_string(),
                heading_path: None,
                content: "g".to_string(),
                score: 0.8,
            },
        ];
        let out = assemble(&cache, hits).expect("assemble");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].note_slug, "alpha");
    }

    #[test]
    fn attaches_resolved_outbound_links() {
        let cache = build_cache(&[
            ("Alpha.md", "# Alpha\n\nlinks to [[Bravo]] and [[Ghost]]"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let hits = vec![ChunkHit {
            chunk_id: 1,
            note_slug: "alpha".to_string(),
            heading_path: None,
            content: "a body".to_string(),
            score: 0.9,
        }];
        let out = assemble(&cache, hits).expect("assemble");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].outbound_links.len(), 1);
        assert_eq!(out[0].outbound_links[0].slug, "bravo");
    }
}
```

- [ ] **Step 2: Run, expect pass on the implementation (tests were written alongside)**

```bash
cargo test -p hatchdoor --lib search::assemble::tests
```

Expected: PASS.

- [ ] **Step 3: Run full suite + clippy**

```bash
cargo test -p hatchdoor
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 4: Commit**

```bash
git add src/search/assemble.rs
git commit -m "feat(search): implement context assembly

Single batched lookup attaches note title, path, and resolved
outbound wikilinks to each ChunkHit, preserves hit order, and
drops hits whose note vanished mid-flight (logged as a warn)."
```

---

## Task 8: Implement `search::run` orchestrator

**Files**
- Modify: `src/search/mod.rs`
- Test: `src/search/mod.rs`

- [ ] **Step 1: Write the failing tests**

In `src/search/mod.rs`, replace the `pub fn run(...) { unimplemented!(...) }` stub with the real signature and append a test module:

```rust
pub fn run(
    cache: &SqliteCache,
    embedder: &dyn Embedder,
    req: SearchRequest,
) -> Result<SearchResponse, String> {
    let trimmed = req.query.trim();
    if trimmed.is_empty() {
        return Err("query cannot be empty".to_string());
    }
    let req = SearchRequest {
        query: trimmed.to_string(),
        ..req
    };
    let mode = req.mode;
    let hits = retrieve::retrieve(cache, embedder, &req)?;
    let results = assemble::assemble(cache, hits)?;
    Ok(SearchResponse { mode, results })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;

    use super::{run, SearchMode, SearchRequest};

    fn build_cache(files: &[(&str, &str)]) -> (SqliteCache, Arc<dyn Embedder>) {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        (cache, embedder)
    }

    #[test]
    fn semantic_path_end_to_end() {
        let (cache, embedder) = build_cache(&[
            ("Alpha.md", "# Alpha\n\napples and oranges"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let resp = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "apples".to_string(),
                mode: SearchMode::Semantic,
                limit: 10,
                per_note_cap: 2,
            },
        )
        .expect("run");
        assert_eq!(resp.mode, SearchMode::Semantic);
        assert!(!resp.results.is_empty());
        assert!(resp.results[0].note_title == "Alpha" || resp.results[0].note_title == "Bravo");
    }

    #[test]
    fn keyword_path_end_to_end() {
        let (cache, embedder) = build_cache(&[
            ("Alpha.md", "# Alpha\n\noranges"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let resp = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "oranges".to_string(),
                mode: SearchMode::Keyword,
                limit: 10,
                per_note_cap: 2,
            },
        )
        .expect("run");
        assert_eq!(resp.mode, SearchMode::Keyword);
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].note_slug, "alpha");
    }

    #[test]
    fn empty_query_errors() {
        let (cache, embedder) = build_cache(&[("Alpha.md", "# Alpha\n\nbody")]);
        let err = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "   ".to_string(),
                mode: SearchMode::Semantic,
                limit: 10,
                per_note_cap: 2,
            },
        )
        .expect_err("expected empty-query error");
        assert!(err.to_lowercase().contains("empty"));
    }

    #[test]
    fn over_fetch_compensates_for_single_note_flooding() {
        // One note with many distinct chunks (heading-separated). per_note_cap=1 means
        // only one chunk from this note can appear, but limit=3 should still try.
        let body = (0..20)
            .map(|i| format!("# H{i}\n\nsection {i} body text"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let (cache, embedder) = build_cache(&[
            ("Alpha.md", body.as_str()),
            ("Bravo.md", "# Bravo\n\nunrelated"),
        ]);
        let resp = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "section".to_string(),
                mode: SearchMode::Keyword,
                limit: 3,
                per_note_cap: 1,
            },
        )
        .expect("run");
        // With per_note_cap=1, at most 1 chunk from Alpha. We may get 1 from Alpha + 0..1 from Bravo.
        let alpha_count = resp.results.iter().filter(|r| r.note_slug == "alpha").count();
        assert!(alpha_count <= 1);
    }
}
```

- [ ] **Step 2: Run, expect pass**

```bash
cargo test -p hatchdoor --lib search::tests
```

Expected: PASS.

- [ ] **Step 3: Run full suite + clippy**

```bash
cargo test -p hatchdoor
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 4: Commit**

```bash
git add src/search/mod.rs
git commit -m "feat(search): implement run() orchestrator

Validates query, delegates to retrieve + assemble, echoes mode in
the response. Backend pipeline is now complete; transport wiring
follows."
```

---

## Task 9: Wire HTTP `GET /api/search` to `search::run`

**Files**
- Modify: `src/api_types.rs`, `src/handlers/api.rs`

This task changes the HTTP response shape. Frontend will break until Task 11, but Rust will still compile and tests will pass.

- [ ] **Step 1: Replace `SearchQuery` and `SearchResponse` in `src/api_types.rs`**

In `src/api_types.rs`, remove the existing `SearchQuery` (lines 66-71) and `SearchResponse` (lines 73-76) definitions. The `use crate::vault::{..., SearchHit};` import can stay; it's still used by `noteSearch.ts` types via the frontend, but the Rust side no longer needs `SearchHit` here. (Inspect imports: if `SearchHit` is only used by the removed `SearchResponse`, also drop it from the import line.)

Replace with:

```rust
use crate::search::SearchMode;

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub mode: Option<SearchMode>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub per_note_cap: Option<usize>,
}
```

The response shape now comes from `crate::search::SearchResponse` directly; no wrapper type is needed in `api_types.rs`. Update any `use crate::api_types::SearchResponse` callsites to `use crate::search::SearchResponse` (likely just the handler).

- [ ] **Step 2: Rewrite `search_handler` in `src/handlers/api.rs`**

Replace the function (lines 154-175) with:

```rust
pub async fn search_handler(
    Query(query): Query<SearchQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };
    let embedder = state.embedder.as_ref();

    let limit = query.limit.unwrap_or(10).clamp(1, 50);
    let per_note_cap = query.per_note_cap.unwrap_or(2).clamp(1, 10);
    let mode = query.mode.unwrap_or_default();
    let q_len = query.q.len();
    debug!(query_len = q_len, ?mode, limit, per_note_cap, "Executing Phase 2 search");

    let req = crate::search::SearchRequest {
        query: query.q,
        mode,
        limit,
        per_note_cap,
    };
    match crate::search::run(cache.as_ref(), embedder, req) {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => internal_error_response(format!("Search failed: {error}")),
    }
}
```

Notes for the implementer:
- `state.embedder` — confirm the field name in `src/app_state.rs`. If it's wrapped in `Arc<dyn Embedder>` the call site is `state.embedder.as_ref()`. If the field is named differently (e.g. `embedder_arc`), use that. Adjust to match.
- `cache.as_ref()` — if `sqlite_cache(&state)` returns `Arc<SqliteCache>` you need `.as_ref()`; if it returns `&SqliteCache` you don't. Match the existing pattern from `recently_modified_handler`.

- [ ] **Step 3: Build**

```bash
cargo build -p hatchdoor
```

Fix any import errors (Debug derive on the imports, missing `tracing::debug!` etc).

- [ ] **Step 4: Run full test suite + clippy**

```bash
cargo test -p hatchdoor
cargo clippy --all-targets -- -D warnings
```

Expected: PASS. No HTTP integration test exists for the old shape; if any test was asserting against `SearchHit`-shaped JSON in `src/handlers/api.rs` or `tests/`, update it to the new shape or temporarily mark it `#[ignore]` with a comment pointing at this task. **Do not commit ignored tests** — fix them before commit.

- [ ] **Step 5: Commit**

```bash
git add src/api_types.rs src/handlers/api.rs
git commit -m "feat(http): wire /api/search to Phase 2 search pipeline

Replaces note-level FTS results with chunk-level semantic-first
results (mode=semantic|keyword, per_note_cap arg). Frontend
update follows in a separate commit; this commit makes the API
contract live."
```

---

## Task 10: Wire MCP `search_notes` to `search::run`

**Files**
- Modify: `src/mcp/tools.rs`

- [ ] **Step 1: Rewrite `search_notes_tool`**

In `src/mcp/tools.rs`, find `async fn search_notes_tool(...)` (around line 210) and replace it with:

```rust
async fn search_notes_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    let args: SearchNotesArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid search_notes arguments: {error}"))
    })?;
    let query = args.query.trim().to_string();
    if query.is_empty() {
        return Err(JsonRpcFailure::invalid_params(
            "search_notes query cannot be empty",
        ));
    }

    let limit = args.limit.unwrap_or(10).clamp(1, 50);
    let per_note_cap = args.per_note_cap.unwrap_or(2).clamp(1, 10);
    let mode = args.mode.unwrap_or_default();

    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;
    let embedder = state.embedder.as_ref();

    let req = crate::search::SearchRequest {
        query,
        mode,
        limit,
        per_note_cap,
    };
    let response = crate::search::run(cache.as_ref(), embedder, req)
        .map_err(JsonRpcFailure::internal)?;

    Ok(tool_success(serde_json::to_value(&response).map_err(
        |e| JsonRpcFailure::internal(format!("serialize search response: {e}")),
    )?))
}
```

- [ ] **Step 2: Replace `SearchNotesArgs`**

At the bottom of `src/mcp/tools.rs`, find:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchNotesArgs {
    query: String,
    #[serde(default)]
    include_content: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
}
```

Replace with:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchNotesArgs {
    query: String,
    #[serde(default)]
    mode: Option<crate::search::SearchMode>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    per_note_cap: Option<usize>,
}
```

- [ ] **Step 3: Update the tool's JSON schema in `tools_list`**

Find the `"name": "search_notes"` entry in `tools_list` (around lines 94-121). Replace the entire entry with:

```rust
        json!({
            "name": "search_notes",
            "description": "Semantic-first chunk search across the vault. Returns ranked chunks with parent note metadata and the parent note's outbound wikilinks. Use mode=\"keyword\" for exact term/BM25 search when phrasing matters. Use get_note for full note content of a returned slug.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Search query."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["semantic", "keyword"],
                        "default": "semantic",
                        "description": "Retrieval mode. semantic = vector similarity (default). keyword = FTS5 BM25 over chunk content."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10
                    },
                    "per_note_cap": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 2,
                        "description": "Maximum number of chunks returned from any single note."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
```

- [ ] **Step 4: Build + tests + clippy**

```bash
cargo build -p hatchdoor
cargo test -p hatchdoor
cargo clippy --all-targets -- -D warnings
```

Update any MCP-tool integration test in `src/mcp/` or `tests/` that asserts on the old `search_notes` shape. The key contract under test should now be: response wraps `{ "mode": "<mode>", "results": [...] }` inside the MCP `tool_success` envelope.

- [ ] **Step 5: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "feat(mcp): wire search_notes to Phase 2 search pipeline

New inputSchema: mode (semantic|keyword), limit, per_note_cap.
Removes include_content. Response is chunk-level with heading_path,
note metadata, and outbound resolved wikilinks."
```

---

## Task 11: Update frontend to consume the new shape

**Files**
- Modify: `frontend/src/types.ts`, `frontend/src/App.tsx`, `frontend/src/components/SearchDialog.tsx`
- Touch any failing test in `frontend/src/App.*.test.tsx` if their mocked API response shapes change.

- [ ] **Step 1: Update the TypeScript types**

In `frontend/src/types.ts`, find the existing `SearchHit` interface (around line 73 — note `match_kind: string`). Replace it with:

```ts
export type SearchMode = "semantic" | "keyword";

export interface OutboundLink {
  slug: string;
  title: string;
}

export interface SearchResult {
  chunk_id: number;
  note_slug: string;
  note_title: string;
  note_path: string;
  heading_path: string | null;
  content: string;
  score: number;
  outbound_links: OutboundLink[];
}

export interface SearchResponse {
  mode: SearchMode;
  results: SearchResult[];
}
```

If the old `SearchHit` type is referenced elsewhere in `frontend/src/`, replace those references with `SearchResult`. Likely sites: `App.tsx`, `SearchDialog.tsx`, `noteSearch.ts` (if it imports the type).

- [ ] **Step 2: Update the fetch in `App.tsx`**

In `frontend/src/App.tsx` around line 326, replace the `URLSearchParams` block:

```ts
          const params = new URLSearchParams({
            q: query,
            mode: searchIncludeContent ? "keyword" : "semantic",
            limit: "30",
            per_note_cap: "2",
          });
```

The legacy `searchIncludeContent` toggle becomes the semantic/keyword switch. If you want to rename the state for clarity, do so in the same commit (rename `searchIncludeContent` → `searchKeywordMode` with the toggle flipped semantically). Otherwise keep the variable name to minimise diff.

- [ ] **Step 3: Update `SearchDialog.tsx` to render the new shape**

In `frontend/src/components/SearchDialog.tsx`, the existing render loop uses `result.match_kind` (line 83, 90, 105) and `result.slug`. Replace with rendering against `SearchResult`:

- key: `${result.note_slug}-${result.chunk_id}` (chunk_id is unique)
- title: `result.note_title`
- subtitle / path: `result.note_path`
- snippet: `result.content` (truncate visually if needed, e.g. first 240 chars)
- badge: `result.heading_path ?? ""` (show only if non-null) — replaces the old `match_kind` chip
- optional small list of `outbound_links` if you want to surface them (not required for Phase 2 — visible UI for links can land later)

Exact JSX is best chosen by reading the current component; the contract is: every field referenced from `result.*` must come from the new `SearchResult` shape.

- [ ] **Step 4: Update / repair frontend tests**

Run:

```bash
cd frontend
npm test -- --run
```

For each failing test under `frontend/src/App.*.test.tsx` that mocks `/api/search`, update the mocked JSON body to the new shape. Example payload to use in mocks:

```ts
{
  mode: "semantic",
  results: [
    {
      chunk_id: 1,
      note_slug: "alpha",
      note_title: "Alpha",
      note_path: "Alpha.md",
      heading_path: "Intro",
      content: "alpha body…",
      score: 0.9,
      outbound_links: [],
    },
  ],
}
```

If a test was asserting `match_kind` or `content=true` query params verbatim, update it to assert against `mode=semantic` / `mode=keyword`.

- [ ] **Step 5: Final frontend + backend pass**

```bash
cd frontend && npm test -- --run && cd ..
cargo test -p hatchdoor
cargo clippy --all-targets -- -D warnings
```

Expected: all green.

- [ ] **Step 6: Manual smoke test**

```bash
# In one terminal — note the cache must NOT exist or must be v4.
rm -f data/cache/hatchdoor-cache.sqlite3
cargo run --bin hatchdoor
```

In another terminal, after the vault indexes:

```bash
curl -s 'http://localhost:8080/api/search?q=hatchdoor&mode=semantic&limit=5' | jq
curl -s 'http://localhost:8080/api/search?q=hatchdoor&mode=keyword&limit=5' | jq
```

Both should return `{"mode": "...", "results": [...]}` with the new shape. Open `http://localhost:8080/` in a browser and confirm the search UI renders results.

- [ ] **Step 7: Commit**

```bash
git add frontend/
git commit -m "feat(frontend): render Phase 2 chunk-level search results

Replaces SearchHit with SearchResult, swaps the include_content
toggle for an explicit semantic/keyword mode param, and updates
mocked tests to match the new API contract."
```

---

## Task 12: Eval regression check

**Files**
- None modified. This is a verification gate before declaring Phase 2 done.

- [ ] **Step 1: Rebuild a fresh cache**

```bash
rm -f data/cache/hatchdoor-cache.sqlite3
cargo run --bin hatchdoor &  # let it index the vault
# wait until logs show indexing complete
kill %1
```

(Or: invoke `refresh_index` via MCP/HTTP once the server is up, then stop it.)

- [ ] **Step 2: Run the semantic eval**

```bash
cargo run --bin eval -- semantic
```

Expected: Recall@5 (any) ≥ 0.968 and MRR ≥ 0.92 (Phase 1.6 baseline). If numbers regress, something in the retrieval path silently changed — debug before declaring done. The likely culprits are score normalization (does not affect ranking, so should be safe), the over-fetch ceiling (`RAW_K_CEILING=200`), or schema changes that affected chunk content (none expected).

- [ ] **Step 3: Update `docs/hatchdoor-architecture-layers.md`**

Mark Layer 6 status as Implemented (currently "Planned — Phase 1" with a note "Phase 2 will expose..."). Mark Layer 9 status as Implemented with a one-line note pointing at `src/search/`. Add a one-line entry in the Phase table flipping Phase 2 from Planned to Done with a commit-hash link to the merge commit.

Do this in a single commit:

```bash
git add docs/hatchdoor-architecture-layers.md
git commit -m "docs(arch): mark Phase 2 (semantic retrieval + context assembly) done

Layers 6 and 9 are now implemented at runtime via src/search/."
```

- [ ] **Step 4: Final report to user**

Summarize what shipped, what the eval numbers were, and any deviations from the spec. End the plan execution.

---

## Out-of-scope reminders (do not let scope creep in)

- No incremental reindexing — Phase 3.
- No permission layer — Phase 4.
- No hybrid retrieval, no cross-encoder rerank, at runtime. Eval-only code remains in tree.
- No `get_chunk(chunk_id)` tool — full chunk content is returned inline.
- The eval-only `eval semantic|keyword|hybrid|rerank|compare` subcommands keep working unchanged; they call into `SqliteCache::semantic_search` and `fts_search_notes` directly, not `search::run`.
